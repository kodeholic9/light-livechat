# TODO — Implementation Roadmap

## Phase 0: STUN / ICE-Lite ✅
- [x] STUN message parsing (hand-rolled, RFC 8489)
- [x] STUN binding response generation (XOR-MAPPED-ADDRESS, MESSAGE-INTEGRITY, FINGERPRINT)
- [x] ICE credential verification (ufrag:pwd matching)
- [x] USE-CANDIDATE handling
- [x] Single UDP socket listener with demux dispatch
- [x] Unit tests for STUN parsing

## Phase 1: DTLS + SRTP Modules ✅
- [x] Server certificate generation (self-signed, per-instance)
- [x] SHA-256 fingerprint (for SDP answer)
- [x] DemuxConn adapter (Conn trait → mpsc channel bridge)
- [x] DTLS passive handshake function (dtls 0.17.1)
- [x] SRTP key derivation layout (RFC 5764 §4.2, 60 bytes)
- [x] SRTP context: encrypt_rtp, decrypt_rtp, decrypt_rtcp
- [x] SRTP roundtrip unit test
- [x] ServerCert in AppState

## Phase 1.5: Room + Participant + Signaling ✅
- [x] Participant: ICE ufrag/pwd, latched address, SRTP contexts, tracks
- [x] Room: 3-index DashMap (user_id, ufrag, addr) — all O(1)
- [x] RoomHub: reverse indices (ufrag → room_id, addr → room_id)
- [x] STUN latch with NAT rebinding support
- [x] IDENTIFY handler (token → user_id)
- [x] ROOM_CREATE handler
- [x] ROOM_JOIN handler (ICE credentials, DTLS fingerprint, member list)
- [x] ROOM_LEAVE handler + disconnect cleanup
- [x] Participant join/leave broadcast (ROOM_EVENT)
- [x] MESSAGE relay (MESSAGE_EVENT)
- [x] ICE_CANDIDATE (trickle — acknowledged, ignored in ICE-Lite)

## Phase 2: UDP ↔ RoomHub Integration ✅
- [x] STUN handler uses RoomHub.latch_by_ufrag() (per-participant ICE credentials)
- [x] MESSAGE-INTEGRITY verification with participant's ice_pwd
- [x] USE-CANDIDATE → trigger DTLS handshake (10s timeout, async spawn)
- [x] DTLS handshake complete → export_srtp_keys() → install on Participant
- [x] SRTP hot path: find_by_addr() O(1) → decrypt → fan-out → encrypt → send
- [x] RTCP detection (RFC 5761 PT demux), decrypt for logging, no relay
- [x] DemuxConn channel switched to Bytes
- [x] DTLSConn keepalive recv loop
- [x] Stale DTLS session cleanup
- [x] AppState shared between WS handlers and UDP transport

## Phase 3: SDP Negotiation ✅ (v0.1.4)
- [x] SDP Offer parsing + Answer generation
- [x] Browser integration test (3-person grid)
- [x] Debug logging + video rendering fix

## Phase A-1: 2PC / SDP-free Architecture ✅ (v0.1.5)
- [x] PcType enum (Publish / Subscribe)
- [x] MediaSession 구조체 (ufrag, ice_pwd, address, srtp per session)
- [x] Participant: publish + subscribe MediaSession 소유
- [x] Room: by_ufrag/by_addr → (Participant, PcType) 매핑
- [x] RoomHub: 참가자당 2개 ufrag 역인덱스
- [x] Signaling: server_config JSON 응답 (SDP 제거)
- [x] PUBLISH_TRACKS 핸들러 (클라이언트 트랙 SSRC 등록)
- [x] TRACKS_UPDATE 이벤트 (subscribe re-nego 트리거)
- [x] UDP: PcType 식별, publish 수신 → subscribe 전송
- [x] transport/sdp.rs 삭제
- [x] Router 제거 (sockaddr 기반 직접 릴레이)

## Phase A-2: 클라이언트 SdpBuilder
- [ ] SdpBuilder JS 모듈 개발
  - [ ] buildPublishRemoteSdp(serverConfig) → recvonly fake SDP
  - [ ] buildSubscribeRemoteSdp(serverConfig, tracks[]) → sendonly × N fake SDP
  - [ ] updateSubscribeRemoteSdp(기존 SDP, 추가/제거 트랙)
- [ ] SdpBuilder 유닛테스트 (서버 없이 순수 입출력 검증)
- [ ] 클라이언트 PC 관리 (publish PC + subscribe PC 생성/연결)
- [ ] tracks_update 수신 → subscribe PC re-negotiation

## Phase B: 통합 테스트
- [ ] 1:1 audio 통화 확인
- [ ] 1:1 video 통화 확인
- [ ] 3명 conference (audio + video)
- [ ] 참가자 입퇴장 시 subscribe re-negotiation 확인

## Phase C: RTCP + 안정화
- [ ] RTCP 릴레이 (SR/RR transparent relay)
- [ ] NACK 처리 (수신 → 송신자에게 전달)
- [ ] PLI 생성/전달 (새 참가자 입장 시 키프레임 요청)
- [ ] REMB 처리 (대역폭 추정 전달)
- [ ] mute/unmute 이벤트 처리 (포워딩 중단/재개)

## Phase D: Hardening
- [ ] IDENTIFY token verification (JWT or shared secret)
- [ ] Zombie session reaper (last_seen timeout)
- [ ] Heartbeat timeout → disconnect
- [ ] DTLS handshake timeout cleanup
- [ ] Graceful shutdown (drain connections)
- [ ] Structured logging & metrics

## Phase E: PTT Support
- [ ] Room mode field (Conference / PTT)
- [ ] Floor control state machine (Idle → Taken → Idle)
- [ ] FLOOR_REQUEST / FLOOR_RELEASE opcodes
- [ ] Relay gate: only floor holder's media forwarded in PTT mode
- [ ] Floor indicator broadcast

## Backlog
- [ ] Simulcast / SVC (layer detection, adaptive quality)
- [ ] TURN relay support (for restrictive NATs)
- [ ] Recording (RTP → file)
- [ ] Data channel support
- [ ] Horizontal scaling (multi-node)
