// author: kodeholic (powered by Claude)
//! RTCP/RTP 패킷 파싱 및 조립 헬퍼
//!
//! - NACK 파싱 (RFC 4585 Generic NACK)
//! - Compound RTCP 분류 (NACK vs relay 대상)
//! - RTX 패킷 조립 (RFC 4588)
//! - PLI / REMB 빌더
//! - RTP 헤더 파싱

use crate::config;

// ============================================================================
// RTP Header 파싱
// ============================================================================

pub(crate) struct RtpHeader {
    pub(crate) pt:         u8,
    pub(crate) marker:     bool,
    pub(crate) seq:        u16,
    pub(crate) timestamp:  u32,
    pub(crate) ssrc:       u32,
    pub(crate) header_len: usize,
}

pub(crate) fn parse_rtp_header(buf: &[u8]) -> RtpHeader {
    if buf.len() < config::RTP_HEADER_MIN_SIZE {
        return RtpHeader { pt: 0, marker: false, seq: 0, timestamp: 0, ssrc: 0, header_len: 0 };
    }
    let b1 = buf[1];
    let cc = (buf[0] & 0x0F) as usize;
    RtpHeader {
        pt:         b1 & 0x7F,
        marker:     (b1 & 0x80) != 0,
        seq:        u16::from_be_bytes([buf[2], buf[3]]),
        timestamp:  u32::from_be_bytes([buf[4], buf[5], buf[6], buf[7]]),
        ssrc:       u32::from_be_bytes([buf[8], buf[9], buf[10], buf[11]]),
        header_len: 12 + cc * 4,
    }
}

// ============================================================================
// Utility
// ============================================================================

pub(crate) fn current_ts() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

// ============================================================================
// NACK 파싱 (RFC 4585 Generic NACK)
// ============================================================================

/// NACK FCI (Feedback Control Information) 항목
pub(crate) struct NackItem {
    pub(crate) media_ssrc: u32,
    pub(crate) pid: u16,
    pub(crate) blp: u16,
}

/// RTCP compound 패킷에서 Generic NACK (PT=205, FMT=1) 항목 추출
///
/// RTCP 패킷 구조 (RFC 4585):
///   Byte 0: V=2, P, FMT (5bit)
///   Byte 1: PT (8bit)
///   Bytes 2-3: length (32-bit words - 1)
///   Bytes 4-7: SSRC of sender
///   Bytes 8-11: SSRC of media source
///   Bytes 12+: FCI (pid:16 + blp:16) × N
pub(crate) fn parse_rtcp_nack(buf: &[u8]) -> Vec<NackItem> {
    let mut items = Vec::new();
    let mut offset = 0;

    while offset + 4 <= buf.len() {
        if buf.len() < offset + 4 { break; }

        let fmt = buf[offset] & 0x1F;
        let pt  = buf[offset + 1];
        let length_words = u16::from_be_bytes([buf[offset + 2], buf[offset + 3]]) as usize;
        let pkt_len = (length_words + 1) * 4;

        if pt == config::RTCP_PT_NACK && fmt == config::RTCP_FMT_NACK {
            // Generic NACK
            if offset + 12 <= buf.len() {
                let media_ssrc = u32::from_be_bytes([
                    buf[offset + 8], buf[offset + 9],
                    buf[offset + 10], buf[offset + 11],
                ]);

                // FCI: (pid:16 + blp:16) × N
                let fci_start = offset + 12;
                let fci_end = (offset + pkt_len).min(buf.len());
                let mut fci_off = fci_start;
                while fci_off + 4 <= fci_end {
                    let pid = u16::from_be_bytes([buf[fci_off], buf[fci_off + 1]]);
                    let blp = u16::from_be_bytes([buf[fci_off + 2], buf[fci_off + 3]]);
                    items.push(NackItem { media_ssrc, pid, blp });
                    fci_off += 4;
                }
            }
        }

        offset += pkt_len;
        if pkt_len == 0 { break; } // safety
    }

    items
}

// ============================================================================
// Compound RTCP 파싱 + 분리 (Phase C-2)
// ============================================================================

/// Compound RTCP 파싱 결과
pub(crate) struct CompoundRtcpParsed {
    /// NACK 블록 (PT=205): 서버에서 RTX 처리
    pub(crate) nack_blocks: Vec<Vec<u8>>,
    /// 릴레이 대상 블록 (RR, PLI, REMB 등): publisher로 전달
    pub(crate) relay_blocks: Vec<RtcpBlockRef>,
}

/// Compound 내 개별 RTCP 블록 참조 (offset + length + media_ssrc)
pub(crate) struct RtcpBlockRef {
    pub(crate) offset: usize,
    pub(crate) length: usize,
    /// RTCP 내 media source SSRC (RR/PLI/REMB: bytes 8-11)
    pub(crate) media_ssrc: u32,
}

/// Compound RTCP를 순회하며 NACK / relay 대상으로 분류
///
/// - PT=205 FMT=1 (Generic NACK): nack_blocks로 분리 (서버 처리)
/// - PT=201 (RR), PT=206 FMT=1 (PLI), PT=206 FMT=15 (REMB): relay_blocks로 수집
/// - 그 외: 무시
pub(crate) fn split_compound_rtcp(buf: &[u8]) -> CompoundRtcpParsed {
    let mut result = CompoundRtcpParsed {
        nack_blocks: Vec::new(),
        relay_blocks: Vec::new(),
    };

    let mut offset = 0;
    while offset + 4 <= buf.len() {
        let fmt = buf[offset] & 0x1F;
        let pt  = buf[offset + 1];
        let length_words = u16::from_be_bytes([buf[offset + 2], buf[offset + 3]]) as usize;
        let pkt_len = (length_words + 1) * 4;

        if pkt_len == 0 || offset + pkt_len > buf.len() { break; }

        // media_ssrc: 대부분 RTCP에서 bytes 8-11 (패킷 내 2번째 SSRC)
        let media_ssrc = if offset + 12 <= buf.len() {
            u32::from_be_bytes([
                buf[offset + 8], buf[offset + 9],
                buf[offset + 10], buf[offset + 11],
            ])
        } else {
            0
        };

        if pt == config::RTCP_PT_NACK && fmt == config::RTCP_FMT_NACK {
            // NACK: 별도 복사하여 nack_blocks에 저장
            result.nack_blocks.push(buf[offset..offset + pkt_len].to_vec());
        } else if pt == config::RTCP_PT_RR
            || (pt == config::RTCP_PT_PSFB && fmt == config::RTCP_FMT_PLI)
            || (pt == config::RTCP_PT_PSFB && fmt == config::RTCP_FMT_REMB)
        {
            // 릴레이 대상: RR, PLI, REMB
            result.relay_blocks.push(RtcpBlockRef {
                offset,
                length: pkt_len,
                media_ssrc,
            });
        }
        // 그 외 (SR 등 subscribe에서 오는 것): 무시

        offset += pkt_len;
    }

    result
}

/// 릴레이 대상 RTCP 블록들을 하나의 compound 패킷으로 재조립
pub(crate) fn assemble_compound(blocks: &[&[u8]]) -> Vec<u8> {
    let total: usize = blocks.iter().map(|b| b.len()).sum();
    let mut buf = Vec::with_capacity(total);
    for block in blocks {
        buf.extend_from_slice(block);
    }
    buf
}

/// NACK PID + BLP → 손실 seq 목록 확장
///
/// PID = 첫 번째 손실 seq
/// BLP = 비트마스크, bit i 설정 → (PID + i + 1) 손실
pub(crate) fn expand_nack(pid: u16, blp: u16) -> Vec<u16> {
    let mut seqs = vec![pid];
    for i in 0..16u16 {
        if blp & (1 << i) != 0 {
            seqs.push(pid.wrapping_add(i + 1));
        }
    }
    seqs
}

// ============================================================================
// RTX 패킷 조립 (RFC 4588)
// ============================================================================

/// 원본 RTP plaintext → RTX 패킷 조립
///
/// RTX 패킷 구조:
///   - RTP 헤더: V/P/X/CC 동일, M=0, PT=97(RTX), seq=rtx_seq, ts=원본, SSRC=rtx_ssrc
///   - 페이로드: [원본 seq 2바이트 big-endian] + [원본 RTP 페이로드]
pub(crate) fn build_rtx_packet(original: &[u8], rtx_ssrc: u32, rtx_seq: u16) -> Vec<u8> {
    if original.len() < config::RTP_HEADER_MIN_SIZE {
        return Vec::new();
    }

    let cc = (original[0] & 0x0F) as usize;
    let header_len = 12 + cc * 4;
    if original.len() < header_len {
        return Vec::new();
    }

    let orig_seq = u16::from_be_bytes([original[2], original[3]]);
    let payload = &original[header_len..];

    // RTX 패킷 = header(12+cc*4) + orig_seq(2) + payload
    let mut rtx = Vec::with_capacity(header_len + 2 + payload.len());

    // RTP header 복사 (V/P/X/CC 유지)
    rtx.extend_from_slice(&original[..header_len]);

    // PT → RTX (97), M=0
    rtx[1] = config::RTX_PAYLOAD_TYPE;

    // seq → rtx_seq
    rtx[2..4].copy_from_slice(&rtx_seq.to_be_bytes());

    // SSRC → rtx_ssrc
    rtx[8..12].copy_from_slice(&rtx_ssrc.to_be_bytes());

    // OSN (Original Sequence Number) + 원본 페이로드
    rtx.extend_from_slice(&orig_seq.to_be_bytes());
    rtx.extend_from_slice(payload);

    rtx
}

// ============================================================================
// RTCP PLI builder (RFC 4585, 12 bytes fixed)
// ============================================================================
//
//  0               1               2               3
//  0 1 2 3 4 5 6 7 0 1 2 3 4 5 6 7 0 1 2 3 4 5 6 7 0 1 2 3 4 5 6 7
// +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
// |V=2|P| FMT=1  |   PT=206      |          length=2             |
// +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
// |                  SSRC of packet sender (0)                   |
// +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
// |                  SSRC of media source                        |
// +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+

pub fn build_pli(media_ssrc: u32) -> [u8; 12] {
    let mut buf = [0u8; 12];
    // V=2, P=0, FMT=1 → 0b10_0_00001 = 0x81
    buf[0] = 0x81;
    // PT=206 (PSFB)
    buf[1] = 206;
    // length=2 (in 32-bit words minus 1)
    buf[2] = 0;
    buf[3] = 2;
    // SSRC of sender = 0 (server doesn't have its own SSRC)
    // buf[4..8] already 0
    // SSRC of media source
    buf[8..12].copy_from_slice(&media_ssrc.to_be_bytes());
    buf
}

// ============================================================================
// RTCP REMB builder (draft-alvestrand-rmcat-remb)
// ============================================================================
//
//  0               1               2               3
//  0 1 2 3 4 5 6 7 0 1 2 3 4 5 6 7 0 1 2 3 4 5 6 7 0 1 2 3 4 5 6 7
// +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
// |V=2|P| FMT=15 |   PT=206      |          length=5             |
// +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
// |                  SSRC of packet sender (0)                   |
// +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
// |                  SSRC of media source (0)                    |
// +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
// |  'R' 'E' 'M' 'B'                                            |
// +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
// | Num SSRC=1  | BR Exp  |  BR Mantissa (18 bits)              |
// +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
// |   SSRC feedback (video SSRC)                                 |
// +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+

/// 서버 자체 REMB 패킷 생성 (24바이트 고정, SSRC 1개)
///
/// Chrome의 goog-remb rtcp_fb에 대응. 서버가 publisher에게
/// "이 만큼까지 보내도 된다"는 대역폭 힌트를 제공한다.
/// TWCC 활성화 후 사용하지 않지만, TWCC 불가 시 fallback용으로 보존.
#[allow(dead_code)]
pub(crate) fn build_remb(bitrate_bps: u64, media_ssrc: u32) -> [u8; 24] {
    let mut buf = [0u8; 24];

    // V=2, P=0, FMT=15 → 0b10_0_01111 = 0x8F
    buf[0] = 0x8F;
    // PT=206 (PSFB)
    buf[1] = 206;
    // length=5 (24 bytes / 4 - 1)
    buf[2] = 0;
    buf[3] = 5;
    // SSRC of sender = 0
    // buf[4..8] already 0
    // SSRC of media source = 0
    // buf[8..12] already 0

    // 'R' 'E' 'M' 'B'
    buf[12] = b'R';
    buf[13] = b'E';
    buf[14] = b'M';
    buf[15] = b'B';

    // Num SSRC = 1, BR Exp (6 bits), BR Mantissa (18 bits)
    // bitrate_bps = mantissa * 2^exp
    let (exp, mantissa) = encode_remb_bitrate(bitrate_bps);

    // Byte 16: Num SSRC (1)
    // Byte 16-19: [num_ssrc:8][exp:6][mantissa:18]
    buf[16] = 1; // num SSRC
    buf[17] = (exp << 2) | ((mantissa >> 16) as u8 & 0x03);
    buf[18] = (mantissa >> 8) as u8;
    buf[19] = mantissa as u8;

    // SSRC feedback
    buf[20..24].copy_from_slice(&media_ssrc.to_be_bytes());

    buf
}

/// REMB 비트레이트 인코딩: bps → (exp, mantissa)
/// mantissa * 2^exp = bps, mantissa는 18비트 이하
#[allow(dead_code)]
fn encode_remb_bitrate(bps: u64) -> (u8, u32) {
    let mut exp: u8 = 0;
    let mut mantissa = bps;
    while mantissa > 0x3FFFF { // 18 bits max = 262143
        mantissa >>= 1;
        exp += 1;
    }
    (exp, mantissa as u32)
}
