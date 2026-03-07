// author: kodeholic (powered by Claude)
//! TWCC (Transport-Wide Congestion Control) — 도착 시간 기록 + feedback 생성
//!
//! Phase TW-1: RTP header extension 파싱 + TwccRecorder
//! Phase TW-2: TWCC feedback RTCP 빌더
//!
//! 동작 원리:
//!   1. Publisher Chrome이 RTP 패킷마다 transport-wide seq# 헤더 확장을 삽입
//!   2. SFU가 publish RTP 수신 시 twcc_seq 추출 + 도착 시간(Instant) 기록
//!   3. 주기적으로(~100ms) TWCC feedback RTCP를 publisher에게 전송
//!   4. Chrome GCC가 패킷 간 도착 시간 변화(delay gradient) 분석 → 비트레이트 자율 결정
//!
//! RTP Header Extension (one-byte form, RFC 5285):
//!   0xBEDE magic (2 bytes) + length in words (2 bytes)
//!   각 element: [ID:4bit | L:4bit] + data(L+1 bytes)
//!   ID=0 → padding, ID=15 → terminator
//!   twcc extmap ID=6 → data = twcc_seq (2 bytes, big-endian)
//!
//! TWCC Feedback 패킷 (draft-holmer-rmcat-transport-wide-cc-extensions):
//!   RTCP header: V=2, FMT=15, PT=205
//!   base_seq(16) + packet_status_count(16)
//!   reference_time(24, signed, ×64ms) + fb_pkt_count(8)
//!   packet_chunk(s) — 2-bit status vector (7 symbols/chunk)
//!   recv_delta(s) — small(1byte, unsigned, ×250µs) or large(2bytes, signed, ×250µs)

use std::time::Instant;
use tracing::debug;

use crate::config;

// ============================================================================
// RTP Header Extension 파싱 — TWCC seq# 추출
// ============================================================================

/// RTP 패킷의 헤더 확장에서 transport-wide sequence number 추출.
///
/// # Arguments
/// * `buf` — RTP plaintext (헤더 + 페이로드)
/// * `extmap_id` — 서버 extmap 정책에서 지정한 twcc extension ID (기본 6)
///
/// # Returns
/// * `Some(twcc_seq)` — twcc_seq 16bit 값
/// * `None` — 확장 없음 또는 해당 extmap_id 없음
pub fn parse_twcc_seq(buf: &[u8], extmap_id: u8) -> Option<u16> {
    if buf.len() < config::RTP_HEADER_MIN_SIZE {
        return None;
    }

    // X bit (bit 4 of byte 0): 확장 헤더 존재 여부
    let has_extension = (buf[0] & 0x10) != 0;
    if !has_extension {
        return None;
    }

    // CC (CSRC count): byte 0의 하위 4비트
    let cc = (buf[0] & 0x0F) as usize;
    let ext_offset = 12 + cc * 4; // 고정 헤더(12) + CSRC(cc*4) 뒤

    // 확장 헤더: profile(2) + length(2) + data(length*4)
    if buf.len() < ext_offset + 4 {
        return None;
    }

    let profile = u16::from_be_bytes([buf[ext_offset], buf[ext_offset + 1]]);
    let ext_len_words = u16::from_be_bytes([buf[ext_offset + 2], buf[ext_offset + 3]]) as usize;

    // One-byte header form only (0xBEDE)
    // Two-byte form (0x1000) 은 Chrome에서 거의 사용 안 함
    if profile != 0xBEDE {
        return None;
    }

    let ext_data_start = ext_offset + 4;
    let ext_data_end = ext_data_start + ext_len_words * 4;
    if buf.len() < ext_data_end {
        return None;
    }

    // One-byte header elements 순회
    let mut pos = ext_data_start;
    while pos < ext_data_end {
        let byte = buf[pos];

        // ID=0 → padding byte, skip
        if byte == 0 {
            pos += 1;
            continue;
        }

        let id = (byte >> 4) & 0x0F;
        let len = (byte & 0x0F) as usize + 1; // L+1 bytes of data

        // ID=15 → terminator (RFC 5285)
        if id == 15 {
            break;
        }

        pos += 1; // ID/L 바이트 넘어감

        if pos + len > ext_data_end {
            break; // malformed
        }

        if id == extmap_id && len >= 2 {
            let twcc_seq = u16::from_be_bytes([buf[pos], buf[pos + 1]]);
            return Some(twcc_seq);
        }

        pos += len;
    }

    None
}

// ============================================================================
// TwccRecorder — 도착 시간 링버퍼 기록
// ============================================================================

/// Publisher별 TWCC 도착 시간 기록기.
/// publish RTP에서 twcc_seq를 추출하여 도착 Instant를 링버퍼에 저장한다.
/// 주기적으로 build_feedback()으로 TWCC feedback RTCP를 생성한다.
pub struct TwccRecorder {
    /// twcc_seq % capacity → 도착 시간
    arrivals: Vec<Option<ArrivalEntry>>,
    capacity: usize,
    /// feedback 미발송 구간 시작 seq (다음 feedback의 base_seq)
    pub pending_base_seq: u16,
    /// 지금까지 기록된 최대 twcc_seq
    pub max_seq: u16,
    /// 기록 시작 여부 (첫 패킷 수신 전 false)
    pub started: bool,
    /// 총 기록된 패킷 수
    pub count: u64,
    /// 시간 기준점 (첫 패킷 수신 시각, reference_time 계산용)
    pub base_time: Option<Instant>,
    /// feedback 패킷 카운터 (Chrome이 순서 확인용으로 사용)
    pub fb_pkt_count: u8,
}

/// 개별 도착 기록
#[derive(Clone, Copy)]
pub struct ArrivalEntry {
    pub twcc_seq: u16,
    pub arrival: Instant,
}

impl TwccRecorder {
    pub fn new() -> Self {
        let mut arrivals = Vec::with_capacity(config::TWCC_RECORDER_CAPACITY);
        arrivals.resize_with(config::TWCC_RECORDER_CAPACITY, || None);
        Self {
            arrivals,
            capacity: config::TWCC_RECORDER_CAPACITY,
            pending_base_seq: 0,
            max_seq: 0,
            started: false,
            count: 0,
            base_time: None,
            fb_pkt_count: 0,
        }
    }

    /// twcc_seq에 대한 도착 시간 기록
    pub fn record(&mut self, twcc_seq: u16, arrival: Instant) {
        let idx = (twcc_seq as usize) % self.capacity;
        self.arrivals[idx] = Some(ArrivalEntry { twcc_seq, arrival });
        self.count += 1;

        if !self.started {
            self.started = true;
            self.pending_base_seq = twcc_seq;
            self.max_seq = twcc_seq;
            self.base_time = Some(arrival);
        } else {
            // wrapping-aware max 비교: seq 차이가 32768 미만이면 더 큰 것
            let diff = twcc_seq.wrapping_sub(self.max_seq);
            if diff > 0 && diff < 0x8000 {
                self.max_seq = twcc_seq;
            }
        }
    }

    /// 특정 twcc_seq의 도착 정보 조회 (seq 검증 포함)
    pub fn get(&self, twcc_seq: u16) -> Option<&ArrivalEntry> {
        let idx = (twcc_seq as usize) % self.capacity;
        self.arrivals[idx].as_ref().filter(|e| e.twcc_seq == twcc_seq)
    }

    /// Feedback 생성 후 pending_base_seq 전진
    pub fn advance_base(&mut self, new_base: u16) {
        self.pending_base_seq = new_base;
    }

    /// pending 범위의 패킷 수 (feedback 생성 판단용)
    pub fn pending_count(&self) -> u16 {
        if !self.started {
            return 0;
        }
        self.max_seq.wrapping_sub(self.pending_base_seq).wrapping_add(1)
    }
}

impl Default for TwccRecorder {
    fn default() -> Self { Self::new() }
}

// ============================================================================
// TWCC Feedback 빌더 (Phase TW-2)
// ============================================================================
//
// TWCC Feedback RTCP 패킷 구조 (draft-holmer-rmcat-transport-wide-cc-extensions):
//
//  0                   1                   2                   3
//  0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
// +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
// |V=2|P| FMT=15  |    PT=205     |           length              |
// +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
// |                  SSRC of packet sender (SFU)                  |
// +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
// |                  SSRC of media source (publisher)             |
// +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
// |   base sequence number (16)   |  packet status count (16)     |
// +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
// |         reference time (24, signed)        | fb pkt count (8) |
// +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
// |        packet chunk           |        packet chunk           |
// +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
// |  recv delta   |  recv delta   |  recv delta   |  recv delta   |
// +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
// |      ...      | zero padding  |
// +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
//
// Packet status (2 bits):
//   0 = not received
//   1 = received, small delta (unsigned 1 byte, 0~63.75ms in 250µs units)
//   2 = received, large or negative delta (signed 2 bytes, ×250µs)
//   3 = reserved
//
// Chunk encoding: 2-bit status vector (type bit=1, symbol_size bit=1)
//   bit 0 = 1 (status vector chunk)
//   bit 1 = 1 (2-bit symbols)
//   bits 2-15 = 7 symbols × 2 bits
//
// recv_delta: 순서는 status에서 received (1 or 2)인 패킷 순

/// 2-bit packet status
#[derive(Clone, Copy, PartialEq, Debug)]
#[repr(u8)]
enum PacketStatus {
    NotReceived = 0,
    SmallDelta  = 1,
    LargeDelta  = 2,
}

/// TWCC feedback RTCP 패킷 생성.
///
/// recorder의 pending 구간 [pending_base_seq, max_seq]을 읽어
/// Chrome GCC가 이해하는 TWCC feedback 패킷을 조립한다.
///
/// # Returns
/// * `Some(Vec<u8>)` — RTCP plaintext (SRTCP 암호화 전)
/// * `None` — pending 패킷 없음 또는 수신된 패킷 없음
pub fn build_twcc_feedback(
    recorder: &mut TwccRecorder,
    media_ssrc: u32,
) -> Option<Vec<u8>> {
    if !recorder.started || recorder.pending_count() == 0 {
        return None;
    }

    let base_time = recorder.base_time?;
    let base_seq = recorder.pending_base_seq;
    let pkt_status_count = recorder.pending_count();

    // 안전 가드: 너무 큰 범위 방지 (8192 = 링버퍼 크기)
    if pkt_status_count > config::TWCC_RECORDER_CAPACITY as u16 {
        // 범위 초과 — base를 max_seq 근처로 강제 전진
        recorder.advance_base(recorder.max_seq.wrapping_sub(100));
        return None;
    }

    // ================================================================
    // 1단계: 도착 정보 수집 + status/delta 결정
    // ================================================================

    let mut statuses = Vec::with_capacity(pkt_status_count as usize);
    let mut deltas: Vec<i64> = Vec::new(); // 250µs 단위, received 패킷만
    let mut first_arrival: Option<Instant> = None;
    let mut prev_arrival: Option<Instant> = None;

    for i in 0..pkt_status_count {
        let seq = base_seq.wrapping_add(i);
        match recorder.get(seq) {
            Some(entry) => {
                let arrival = entry.arrival;

                if first_arrival.is_none() {
                    first_arrival = Some(arrival);
                }

                // delta 계산 (이전 received 패킷 대비)
                let delta_250us = match prev_arrival {
                    Some(prev) => {
                        // Instant 간 차이 (음수 가능하도록 signed)
                        if arrival >= prev {
                            arrival.duration_since(prev).as_micros() as i64 / 250
                        } else {
                            -(prev.duration_since(arrival).as_micros() as i64 / 250)
                        }
                    }
                    None => {
                        // 첫 received 패킷: reference_time과의 차이
                        // reference_time을 first_arrival 기준으로 잡으므로,
                        // reference_time_250us = floor(arrival_offset / 256) * 256
                        // first_delta = arrival_offset - reference_time_250us
                        // → 항상 [0, 255] 범위 (small delta 보장)
                        let offset_us = arrival.duration_since(base_time).as_micros() as i64;
                        let offset_250us = offset_us / 250;
                        let ref_250us = (offset_250us / 256) * 256;
                        offset_250us - ref_250us
                    }
                };

                // status 결정
                let status = if delta_250us >= 0 && delta_250us <= 255 {
                    PacketStatus::SmallDelta
                } else if delta_250us >= -8192 && delta_250us <= 8191 {
                    PacketStatus::LargeDelta
                } else {
                    // 범위 초과 → large delta로 클램핑
                    PacketStatus::LargeDelta
                };

                statuses.push(status);
                deltas.push(delta_250us);
                prev_arrival = Some(arrival);
            }
            None => {
                statuses.push(PacketStatus::NotReceived);
            }
        }
    }

    // received 패킷이 하나도 없으면 feedback 불필요
    let first_arrival = first_arrival?;

    // ================================================================
    // 2단계: reference_time 계산 (24-bit signed, ×64ms = ×256 in 250µs units)
    // ================================================================

    let first_offset_us = first_arrival.duration_since(base_time).as_micros() as i64;
    let first_offset_250us = first_offset_us / 250;
    // reference_time = floor(first_offset / 256) — 이렇게 하면 첫 delta가 [0,255]
    let reference_time_raw = first_offset_250us / 256;
    // 24-bit signed 으로 truncate (wrap around)
    let reference_time_24 = (reference_time_raw as i32) & 0x00FF_FFFF;

    // ================================================================
    // 진단 로그 — feedback 패킷 핵심 파라미터
    // ================================================================
    {
        let received = statuses.iter().filter(|s| **s != PacketStatus::NotReceived).count();
        let lost = statuses.iter().filter(|s| **s == PacketStatus::NotReceived).count();
        let small = statuses.iter().filter(|s| **s == PacketStatus::SmallDelta).count();
        let large = statuses.iter().filter(|s| **s == PacketStatus::LargeDelta).count();
        let (d_min, d_max, d_first) = if deltas.is_empty() {
            (0i64, 0i64, 0i64)
        } else {
            (*deltas.iter().min().unwrap(), *deltas.iter().max().unwrap(), deltas[0])
        };
        let neg_count = deltas.iter().filter(|d| **d < 0).count();

        debug!(
            "[TWCC:FB] ssrc=0x{:08X} fb#{} base_seq={} count={} recv={} lost={} \
             small={} large={} neg={} delta_first={} delta_min={} delta_max={} \
             ref_time={} (raw={}) base_elapsed_ms={}",
            media_ssrc, recorder.fb_pkt_count, base_seq, pkt_status_count,
            received, lost, small, large, neg_count,
            d_first, d_min, d_max,
            reference_time_24, reference_time_raw,
            first_offset_us / 1000,
        );
    }

    // ================================================================
    // 3단계: Packet chunk 인코딩 (2-bit status vector, 7 symbols/chunk)
    // ================================================================

    let chunks = encode_status_chunks(&statuses);

    // ================================================================
    // 4단계: recv_delta 인코딩
    // ================================================================

    let delta_bytes = encode_recv_deltas(&statuses, &deltas);

    // ================================================================
    // 5단계: RTCP 패킷 조립
    // ================================================================

    let fb_pkt_count = recorder.fb_pkt_count;
    recorder.fb_pkt_count = recorder.fb_pkt_count.wrapping_add(1);

    // 전체 크기 계산
    // 헤더: 4(RTCP header) + 4(sender SSRC) + 4(media SSRC)
    //        + 4(base_seq + status_count) + 4(ref_time + fb_count)
    //      = 20 bytes 고정
    // chunks: chunks.len() * 2 bytes
    // deltas: delta_bytes.len() bytes
    // padding: 4-byte alignment

    let payload_len = 8 + chunks.len() * 2 + delta_bytes.len(); // base_seq~끝
    let body_len = 12 + payload_len; // sender_ssrc + media_ssrc + payload
    let padded_body = (body_len + 3) & !3; // 4-byte align
    let need_padding = padded_body > body_len;
    let total_len = 4 + padded_body; // RTCP header(4) + body

    let mut buf = Vec::with_capacity(total_len);

    // --- RTCP Header ---
    // V=2, P=padding?, FMT=15 → byte0 = 0b10_P_01111
    let byte0 = if need_padding { 0xAF } else { 0x8F }; // 0xAF = V=2,P=1,FMT=15
    buf.push(byte0);
    buf.push(config::RTCP_PT_RTPFB); // PT=205
    let length_words = ((padded_body / 4) as u16).to_be_bytes();
    buf.extend_from_slice(&length_words);

    // --- Sender SSRC (SFU = 1) ---
    buf.extend_from_slice(&1u32.to_be_bytes());

    // --- Media source SSRC (publisher video) ---
    buf.extend_from_slice(&media_ssrc.to_be_bytes());

    // --- Base sequence number + packet status count ---
    buf.extend_from_slice(&base_seq.to_be_bytes());
    buf.extend_from_slice(&pkt_status_count.to_be_bytes());

    // --- Reference time (24-bit signed) + fb_pkt_count (8-bit) ---
    buf.push(((reference_time_24 >> 16) & 0xFF) as u8);
    buf.push(((reference_time_24 >> 8) & 0xFF) as u8);
    buf.push((reference_time_24 & 0xFF) as u8);
    buf.push(fb_pkt_count);

    // --- Packet chunks ---
    for chunk in &chunks {
        buf.extend_from_slice(&chunk.to_be_bytes());
    }

    // --- Recv deltas ---
    buf.extend_from_slice(&delta_bytes);

    // --- Padding (4-byte alignment) ---
    while buf.len() < total_len {
        buf.push(0);
    }
    // P=1일 때 마지막 바이트 = padding 크기
    if need_padding {
        let pad_count = total_len - (4 + body_len);
        if pad_count > 0 {
            *buf.last_mut().unwrap() = pad_count as u8;
        }
    }

    // pending_base 전진 (다음 feedback은 여기서부터)
    recorder.advance_base(base_seq.wrapping_add(pkt_status_count));

    debug!("[TWCC:FB] pkt_size={} chunks={} delta_bytes={}",
        buf.len(), chunks.len(), delta_bytes.len());

    Some(buf)
}

/// 2-bit status vector chunk 인코딩.
///
/// 각 chunk = 16비트:
///   bit 15 = 1 (status vector type)
///   bit 14 = 1 (2-bit symbols)
///   bits 13-0 = 7 symbols × 2 bits
///
/// 마지막 chunk는 남는 자리를 0(not received)으로 패딩.
fn encode_status_chunks(statuses: &[PacketStatus]) -> Vec<u16> {
    let mut chunks = Vec::new();
    let mut i = 0;

    while i < statuses.len() {
        // 2-bit status vector: type=1, symbol_size=1, 7 symbols
        let mut chunk: u16 = 0xC000; // bits 15-14 = 11

        for slot in 0..7 {
            let status = if i + slot < statuses.len() {
                statuses[i + slot]
            } else {
                PacketStatus::NotReceived // 패딩
            };
            let shift = 12 - slot * 2; // bit 13-12, 11-10, 9-8, 7-6, 5-4, 3-2, 1-0
            chunk |= (status as u16) << shift;
        }

        chunks.push(chunk);
        i += 7;
    }

    chunks
}

/// recv_delta 바이트열 인코딩.
///
/// statuses에서 received (SmallDelta/LargeDelta)인 패킷만 순서대로 delta를 인코딩.
/// SmallDelta → unsigned 1 byte (0~255, ×250µs = 0~63.75ms)
/// LargeDelta → signed 2 bytes (×250µs)
fn encode_recv_deltas(statuses: &[PacketStatus], deltas: &[i64]) -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut delta_idx = 0;

    for status in statuses {
        match status {
            PacketStatus::SmallDelta => {
                if delta_idx < deltas.len() {
                    let d = deltas[delta_idx].clamp(0, 255) as u8;
                    bytes.push(d);
                    delta_idx += 1;
                }
            }
            PacketStatus::LargeDelta => {
                if delta_idx < deltas.len() {
                    let d = deltas[delta_idx].clamp(-8192, 8191) as i16;
                    bytes.extend_from_slice(&d.to_be_bytes());
                    delta_idx += 1;
                }
            }
            PacketStatus::NotReceived => {
                // delta 없음
            }
        }
    }

    bytes
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    // === parse_twcc_seq 테스트 ===

    /// 최소 RTP 패킷 (확장 없음) → None
    #[test]
    fn parse_twcc_seq_no_extension() {
        let rtp = [
            0x80, 96, 0x00, 0x01,
            0x00, 0x00, 0x00, 0xA0,
            0x00, 0x00, 0x10, 0x00,
            0xDE, 0xAD,
        ];
        assert_eq!(parse_twcc_seq(&rtp, 6), None);
    }

    /// X=1 + one-byte form (0xBEDE) + ID=6(twcc) → Some(twcc_seq)
    #[test]
    fn parse_twcc_seq_one_byte_form() {
        let rtp = vec![
            0x90, 96, 0x00, 0x01,
            0x00, 0x00, 0x00, 0xA0,
            0x00, 0x00, 0x10, 0x00,
            0xBE, 0xDE,
            0x00, 0x01,
            0x61, 0x00, 0xFF,
            0x00,
        ];
        assert_eq!(parse_twcc_seq(&rtp, 6), Some(0x00FF));
    }

    /// 여러 extension element 중에서 twcc(ID=6) 찾기
    #[test]
    fn parse_twcc_seq_multiple_extensions() {
        let rtp = vec![
            0x90, 96, 0x00, 0x02,
            0x00, 0x00, 0x00, 0xA0,
            0x00, 0x00, 0x10, 0x00,
            0xBE, 0xDE,
            0x00, 0x03,
            0x10, 0x00,
            0x40, 0x7F,
            0x61, 0x01, 0x23,
            0x00, 0x00, 0x00, 0x00, 0x00,
        ];
        assert_eq!(parse_twcc_seq(&rtp, 6), Some(0x0123));
    }

    /// 다른 extmap_id로 찾기 → None
    #[test]
    fn parse_twcc_seq_wrong_id() {
        let rtp = vec![
            0x90, 96, 0x00, 0x01,
            0x00, 0x00, 0x00, 0xA0,
            0x00, 0x00, 0x10, 0x00,
            0xBE, 0xDE,
            0x00, 0x01,
            0x61, 0x00, 0xFF, 0x00,
        ];
        assert_eq!(parse_twcc_seq(&rtp, 3), None);
    }

    /// Two-byte form (0x1000) → None (미지원)
    #[test]
    fn parse_twcc_seq_two_byte_form_unsupported() {
        let rtp = vec![
            0x90, 96, 0x00, 0x01,
            0x00, 0x00, 0x00, 0xA0,
            0x00, 0x00, 0x10, 0x00,
            0x10, 0x00,
            0x00, 0x01,
            0x06, 0x02, 0x00, 0xFF,
        ];
        assert_eq!(parse_twcc_seq(&rtp, 6), None);
    }

    // === TwccRecorder 테스트 ===

    #[test]
    fn recorder_basic() {
        let mut rec = TwccRecorder::new();
        assert!(!rec.started);
        assert_eq!(rec.pending_count(), 0);

        let t0 = Instant::now();
        rec.record(100, t0);
        assert!(rec.started);
        assert_eq!(rec.pending_base_seq, 100);
        assert_eq!(rec.max_seq, 100);
        assert_eq!(rec.count, 1);
        assert_eq!(rec.pending_count(), 1);
        assert!(rec.base_time.is_some());

        rec.record(101, t0);
        rec.record(102, t0);
        assert_eq!(rec.max_seq, 102);
        assert_eq!(rec.pending_count(), 3);

        assert!(rec.get(100).is_some());
        assert!(rec.get(101).is_some());
        assert!(rec.get(99).is_none());

        rec.advance_base(103);
        assert_eq!(rec.pending_base_seq, 103);
        assert_eq!(rec.pending_count(), 0);
    }

    #[test]
    fn recorder_seq_wrapping() {
        let mut rec = TwccRecorder::new();
        let t0 = Instant::now();

        rec.record(65534, t0);
        rec.record(65535, t0);
        rec.record(0, t0);
        rec.record(1, t0);
        assert_eq!(rec.max_seq, 1);
    }

    #[test]
    fn recorder_overwrite() {
        let mut rec = TwccRecorder::new();
        let t0 = Instant::now();
        let t1 = t0 + Duration::from_millis(100);
        let cap = config::TWCC_RECORDER_CAPACITY as u16;

        rec.record(42, t0);
        rec.record(42 + cap, t1);

        assert!(rec.get(42).is_none());
        assert!(rec.get(42 + cap).is_some());
    }

    // === encode_status_chunks 테스트 ===

    #[test]
    fn chunks_single_full() {
        // 7 symbols 정확히 1 chunk
        let statuses = vec![
            PacketStatus::SmallDelta,  // 01
            PacketStatus::SmallDelta,  // 01
            PacketStatus::NotReceived, // 00
            PacketStatus::SmallDelta,  // 01
            PacketStatus::LargeDelta,  // 10
            PacketStatus::NotReceived, // 00
            PacketStatus::SmallDelta,  // 01
        ];
        let chunks = encode_status_chunks(&statuses);
        assert_eq!(chunks.len(), 1);
        // 1_1_01_01_00_01_10_00_01 = 0b_1101_0100_0110_0001 = 0xD461
        assert_eq!(chunks[0], 0xD461);
    }

    #[test]
    fn chunks_padding() {
        // 3 symbols → 1 chunk, 나머지 4칸 = NotReceived(0)
        let statuses = vec![
            PacketStatus::SmallDelta,  // 01
            PacketStatus::SmallDelta,  // 01
            PacketStatus::SmallDelta,  // 01
        ];
        let chunks = encode_status_chunks(&statuses);
        assert_eq!(chunks.len(), 1);
        // 11_01_01_01_00_00_00_00 = 0b_1101_0101_0000_0000 = 0xD500
        assert_eq!(chunks[0], 0xD500);
    }

    #[test]
    fn chunks_two_chunks() {
        // 8 symbols → 2 chunks (7 + 1)
        let statuses = vec![
            PacketStatus::SmallDelta; 8
        ];
        let chunks = encode_status_chunks(&statuses);
        assert_eq!(chunks.len(), 2);
        // chunk 0: 11_01_01_01_01_01_01_01 = 0xD555
        assert_eq!(chunks[0], 0xD555);
        // chunk 1: 11_01_00_00_00_00_00_00 = 0xD000
        assert_eq!(chunks[1], 0xD000);
    }

    // === encode_recv_deltas 테스트 ===

    #[test]
    fn deltas_small_only() {
        let statuses = vec![
            PacketStatus::SmallDelta,
            PacketStatus::NotReceived,
            PacketStatus::SmallDelta,
        ];
        let deltas = vec![10i64, 20]; // 2 received
        let bytes = encode_recv_deltas(&statuses, &deltas);
        assert_eq!(bytes, vec![10, 20]);
    }

    #[test]
    fn deltas_mixed() {
        let statuses = vec![
            PacketStatus::SmallDelta,
            PacketStatus::LargeDelta,
        ];
        let deltas = vec![100i64, 300]; // 300 > 255 → large delta
        let bytes = encode_recv_deltas(&statuses, &deltas);
        // small: [100], large: [0x01, 0x2C] (300 as i16 big-endian)
        assert_eq!(bytes, vec![100, 0x01, 0x2C]);
    }

    #[test]
    fn deltas_negative() {
        let statuses = vec![PacketStatus::LargeDelta];
        let deltas = vec![-100i64];
        let bytes = encode_recv_deltas(&statuses, &deltas);
        // -100 as i16 = 0xFF9C
        assert_eq!(bytes, vec![0xFF, 0x9C]);
    }

    // === build_twcc_feedback 통합 테스트 ===

    #[test]
    fn feedback_empty_recorder() {
        let mut rec = TwccRecorder::new();
        assert!(build_twcc_feedback(&mut rec, 0x1000).is_none());
    }

    #[test]
    fn feedback_single_packet() {
        let mut rec = TwccRecorder::new();
        let t0 = Instant::now();
        rec.record(100, t0);

        let pkt = build_twcc_feedback(&mut rec, 0x1000).unwrap();

        // 기본 구조 검증
        assert!(pkt.len() >= 20); // 최소 헤더 크기
        assert_eq!(pkt[1], 205); // PT=205

        // sender SSRC = 1
        assert_eq!(u32::from_be_bytes([pkt[4], pkt[5], pkt[6], pkt[7]]), 1);
        // media SSRC = 0x1000
        assert_eq!(u32::from_be_bytes([pkt[8], pkt[9], pkt[10], pkt[11]]), 0x1000);

        // base_seq = 100
        assert_eq!(u16::from_be_bytes([pkt[12], pkt[13]]), 100);
        // packet_status_count = 1
        assert_eq!(u16::from_be_bytes([pkt[14], pkt[15]]), 1);

        // fb_pkt_count = 0 (first feedback)
        assert_eq!(pkt[19], 0);

        // 4-byte aligned
        assert_eq!(pkt.len() % 4, 0);

        // pending_base 전진 확인
        assert_eq!(rec.pending_base_seq, 101);
        assert_eq!(rec.fb_pkt_count, 1);
    }

    #[test]
    fn feedback_multiple_with_loss() {
        let mut rec = TwccRecorder::new();
        let t0 = Instant::now();

        // seq 100, 101(lost), 102 — 1ms 간격
        rec.record(100, t0);
        // 101은 기록하지 않음 (lost)
        rec.record(102, t0 + Duration::from_millis(2));

        let pkt = build_twcc_feedback(&mut rec, 0x2000).unwrap();

        // base_seq = 100, count = 3
        assert_eq!(u16::from_be_bytes([pkt[12], pkt[13]]), 100);
        assert_eq!(u16::from_be_bytes([pkt[14], pkt[15]]), 3);

        // chunk: 3 statuses → 1 chunk
        // status[0]=SmallDelta(1), status[1]=NotReceived(0), status[2]=SmallDelta/LargeDelta
        let chunk = u16::from_be_bytes([pkt[20], pkt[21]]);
        assert_eq!(chunk & 0xC000, 0xC000); // type=1, symbol_size=1

        // 4-byte aligned
        assert_eq!(pkt.len() % 4, 0);
    }

    #[test]
    fn feedback_fb_pkt_count_increments() {
        let mut rec = TwccRecorder::new();
        let t0 = Instant::now();

        rec.record(10, t0);
        let pkt1 = build_twcc_feedback(&mut rec, 0x1000).unwrap();
        assert_eq!(pkt1[19], 0); // first

        rec.record(11, t0 + Duration::from_millis(1));
        let pkt2 = build_twcc_feedback(&mut rec, 0x1000).unwrap();
        assert_eq!(pkt2[19], 1); // second
    }

    #[test]
    fn feedback_rtcp_length_field_valid() {
        let mut rec = TwccRecorder::new();
        let t0 = Instant::now();

        for i in 0u16..20 {
            rec.record(i, t0 + Duration::from_millis(i as u64));
        }

        let pkt = build_twcc_feedback(&mut rec, 0x3000).unwrap();

        // RTCP length field = (total_bytes / 4) - 1
        let length_words = u16::from_be_bytes([pkt[2], pkt[3]]) as usize;
        assert_eq!((length_words + 1) * 4, pkt.len());
    }
}
