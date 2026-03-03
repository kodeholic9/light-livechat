// author: kodeholic (powered by Claude)
//! UDP media transport — single port, demux dispatch, RoomHub integration
//!
//! Packet flow:
//!   recv_from(addr)
//!     → classify (RFC 5764 first-byte)
//!     → STUN : RoomHub.latch_by_ufrag() → Binding Response → trigger DTLS
//!     → DTLS : DtlsSessionMap.inject() or start new handshake
//!     → SRTP : RoomHub.find_by_addr() → decrypt → relay → encrypt → send

use bytes::{Bytes, BytesMut};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::UdpSocket;
use tracing::{debug, error, info, trace, warn};

use webrtc_util::conn::Conn;

use crate::config;
use crate::room::room::RoomHub;
use crate::transport::demux::{self, PacketType};
use crate::transport::demux_conn::{DemuxConn, DtlsPacketTx};
use crate::transport::dtls::{self, ServerCert};
use crate::transport::stun;

// ============================================================================
// DTLS Session Map (addr → packet channel)
// ============================================================================

struct DtlsSessionMap {
    sessions: HashMap<SocketAddr, DtlsPacketTx>,
}

impl DtlsSessionMap {
    fn new() -> Self {
        Self { sessions: HashMap::new() }
    }

    fn insert(&mut self, addr: SocketAddr, tx: DtlsPacketTx) {
        self.sessions.insert(addr, tx);
    }

    /// Inject packet into existing session. Returns false if no session.
    async fn inject(&self, addr: &SocketAddr, data: Bytes) -> bool {
        if let Some(tx) = self.sessions.get(addr) {
            tx.send(data).await.is_ok()
        } else {
            false
        }
    }

    fn has(&self, addr: &SocketAddr) -> bool {
        self.sessions.contains_key(addr)
    }

    /// Periodically clean up sessions whose tx channel is closed
    fn remove_stale(&mut self) {
        self.sessions.retain(|addr, tx| {
            if tx.is_closed() {
                debug!("stale DTLS session removed addr={}", addr);
                false
            } else {
                true
            }
        });
    }
}

// ============================================================================
// UdpTransport
// ============================================================================

pub struct UdpTransport {
    pub socket:   Arc<UdpSocket>,
    room_hub:     Arc<RoomHub>,
    cert:         Arc<ServerCert>,
    dtls_map:     DtlsSessionMap,
    /// Counter for periodic stale session cleanup
    pkt_count:    u64,
}

impl UdpTransport {
    pub async fn bind(
        room_hub: Arc<RoomHub>,
        cert:     Arc<ServerCert>,
    ) -> std::io::Result<Self> {
        let addr = SocketAddr::from(([0, 0, 0, 0], config::UDP_PORT));
        let socket = UdpSocket::bind(addr).await?;
        info!("UDP transport bound on {}", addr);

        Ok(Self {
            socket: Arc::new(socket),
            room_hub,
            cert,
            dtls_map: DtlsSessionMap::new(),
            pkt_count: 0,
        })
    }

    /// Main receive loop — runs forever
    pub async fn run(mut self) {
        let mut buf = BytesMut::zeroed(config::UDP_RECV_BUF_SIZE);

        loop {
            let (len, remote) = match self.socket.recv_from(&mut buf).await {
                Ok(r) => r,
                Err(e) => { error!("UDP recv error: {e}"); continue; }
            };

            let data = Bytes::copy_from_slice(&buf[..len]);

            match demux::classify(&data) {
                PacketType::Stun => self.handle_stun(&data, remote).await,
                PacketType::Dtls => self.handle_dtls(data, remote).await,
                PacketType::Srtp => self.handle_srtp(&data, remote).await,
                PacketType::Unknown => {
                    trace!("unknown packet from {} byte0=0x{:02X}", remote, data[0]);
                }
            }

            // Periodic cleanup (every ~1000 packets)
            self.pkt_count += 1;
            if self.pkt_count % 1000 == 0 {
                self.dtls_map.remove_stale();
            }
        }
    }

    // ========================================================================
    // STUN — cold path (ICE connectivity check)
    // ========================================================================

    async fn handle_stun(&mut self, buf: &[u8], remote: SocketAddr) {
        // Parse STUN message
        let msg = match stun::parse(buf) {
            Some(m) => m,
            None => { trace!("STUN parse failed from {}", remote); return; }
        };

        // Only handle Binding Requests
        if msg.msg_type != stun::BINDING_REQUEST {
            trace!("non-binding STUN from {} type=0x{:04X}", remote, msg.msg_type);
            return;
        }

        // USERNAME = "server_ufrag:client_ufrag"
        let username = match msg.username() {
            Some(u) => u,
            None => { debug!("STUN without USERNAME from {}", remote); return; }
        };
        let server_ufrag = match username.split(':').next() {
            Some(s) => s,
            None => { debug!("invalid STUN USERNAME format: {}", username); return; }
        };

        // Latch via RoomHub → registers addr reverse index
        let (participant, _room) = match self.room_hub.latch_by_ufrag(server_ufrag, remote) {
            Some(r) => r,
            None => { debug!("unknown ufrag={} from {}", server_ufrag, remote); return; }
        };

        participant.touch(current_ts());

        // Verify MESSAGE-INTEGRITY with participant's ice_pwd
        let integrity_key = stun::ice_integrity_key(&participant.ice_pwd);
        if !stun::verify_message_integrity(&msg, &integrity_key) {
            warn!("STUN MESSAGE-INTEGRITY mismatch user={}", participant.user_id);
            return;
        }

        // Build and send Binding Success Response
        let response = stun::build_binding_response(
            &msg.transaction_id,
            remote,
            &integrity_key,
        );
        if let Err(e) = self.socket.send_to(&response, remote).await {
            error!("STUN response send failed: {e}");
        }

        // USE-CANDIDATE → trigger DTLS handshake (if not already running)
        if msg.has_use_candidate() && !self.dtls_map.has(&remote) {
            debug!("USE-CANDIDATE user={} → starting DTLS", participant.user_id);
            self.start_dtls_handshake(remote, participant).await;
        }
    }

    // ========================================================================
    // DTLS — handshake path
    // ========================================================================

    async fn handle_dtls(&mut self, data: Bytes, remote: SocketAddr) {
        // Try injecting into existing session first
        if self.dtls_map.inject(&remote, data.clone()).await {
            return;
        }

        // No session yet — check if participant is latched
        let (participant, _room) = match self.room_hub.find_by_addr(&remote) {
            Some(r) => r,
            None => {
                debug!("DTLS from unlatched addr={}, dropping", remote);
                return;
            }
        };

        // Start new session + inject the first packet
        debug!("DTLS new session user={} addr={}", participant.user_id, remote);
        self.start_dtls_handshake(remote, participant).await;
        self.dtls_map.inject(&remote, data).await;
    }

    async fn start_dtls_handshake(
        &mut self,
        remote: SocketAddr,
        participant: Arc<crate::room::participant::Participant>,
    ) {
        let (adapter, tx) = DemuxConn::new(Arc::clone(&self.socket), remote);
        self.dtls_map.insert(remote, tx);

        let cert = Arc::clone(&self.cert);

        tokio::spawn(async move {
            let config = dtls::server_config(&cert);
            let conn: Arc<dyn webrtc_util::conn::Conn + Send + Sync> = Arc::new(adapter);

            let timeout = tokio::time::Duration::from_secs(10);
            let result = tokio::time::timeout(timeout, dtls::accept_dtls(conn, config)).await;

            match result {
                Ok(Ok(dtls_conn)) => {
                    match dtls::export_srtp_keys(&dtls_conn).await {
                        Ok(keys) => {
                            participant.install_srtp_keys(
                                &keys.client_key,
                                &keys.client_salt,
                                &keys.server_key,
                                &keys.server_salt,
                            );
                            info!("DTLS+SRTP ready user={} addr={}", participant.user_id, remote);
                        }
                        Err(e) => {
                            error!("SRTP key export failed user={}: {e}", participant.user_id);
                        }
                    }

                    // Keep DTLSConn alive — recv loop until connection ends
                    let mut keepalive_buf = vec![0u8; 1500];
                    loop {
                        match dtls_conn.recv(&mut keepalive_buf).await {
                            Ok(0) | Err(_) => break,
                            Ok(_) => {} // application data (unused in SFU)
                        }
                    }
                }
                Ok(Err(e)) => {
                    warn!("DTLS handshake failed user={}: {e}", participant.user_id);
                }
                Err(_) => {
                    warn!("DTLS handshake timeout (10s) user={} addr={}", participant.user_id, remote);
                }
            }

            debug!("DTLS session ended user={} addr={}", participant.user_id, remote);
            // tx drops here → remove_stale() will clean up
        });
    }

    // ========================================================================
    // SRTP — hot path (media relay)
    // ========================================================================

    async fn handle_srtp(&self, buf: &[u8], remote: SocketAddr) {
        // O(1) lookup: addr → participant + room
        let (sender, room) = match self.room_hub.find_by_addr(&remote) {
            Some(r) => r,
            None => { trace!("SRTP from unknown addr={}", remote); return; }
        };

        sender.touch(current_ts());

        if !sender.is_media_ready() {
            trace!("SRTP before DTLS complete user={}, dropping", sender.user_id);
            return;
        }

        // Detect RTCP vs RTP (RFC 5761 demux: PT 72-79 = RTCP)
        let is_rtcp = buf.get(1)
            .map(|b| { let pt = b & 0x7F; (72..=79).contains(&pt) })
            .unwrap_or(false);

        if is_rtcp {
            let mut ctx = sender.inbound_srtp.lock().unwrap();
            match ctx.decrypt_rtcp(buf) {
                Ok(_) => trace!("SRTCP from user={}", sender.user_id),
                Err(e) => trace!("SRTCP decrypt err user={}: {e}", sender.user_id),
            }
            return; // don't relay RTCP
        }

        // Decrypt SRTP → plaintext RTP
        let plaintext = {
            let mut ctx = sender.inbound_srtp.lock().unwrap();
            match ctx.decrypt_rtp(buf) {
                Ok(p) => p,
                Err(e) => {
                    warn!("SRTP decrypt failed user={}: {e}", sender.user_id);
                    return;
                }
            }
        };

        // Fan-out: relay to all other media-ready participants in the room
        let targets = room.other_participants(&sender.user_id);
        for target in &targets {
            if !target.is_media_ready() { continue; }

            let addr = match target.get_address() {
                Some(a) => a,
                None => continue,
            };

            let encrypted = {
                let mut ctx = target.outbound_srtp.lock().unwrap();
                match ctx.encrypt_rtp(&plaintext) {
                    Ok(p) => p,
                    Err(e) => {
                        warn!("SRTP encrypt failed → user={}: {e}", target.user_id);
                        continue;
                    }
                }
            };

            if let Err(e) = self.socket.send_to(&encrypted, addr).await {
                warn!("UDP send failed → user={}: {e}", target.user_id);
            }
        }
    }
}

// ============================================================================
// Utility
// ============================================================================

fn current_ts() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
