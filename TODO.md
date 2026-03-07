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

## Phase A-2: 클라이언트 SdpBuilder ✅ (v0.1.6)
- [x] SdpBuilder JS 모듈 개발 (sdp-builder.mjs)
  - [x] buildPublishRemoteSdp(serverConfig) → recvonly fake SDP
  - [x] buildSubscribeRemoteSdp(serverConfig, tracks[]) → sendonly × N fake SDP
  - [x] updateSubscribeRemoteSdp(serverConfig, allTracks) → re-nego용 전체 재조립
  - [x] validateSdp(sdp) → 디버깅용 구조 검증
- [x] SdpBuilder 유닛테스트 74개 전부 통과
- [x] livechat-sdk.js v2.0.0 (2PC/SDP-free 전환)
  - [x] _pubPc + _subPc 2개 PC 관리
  - [x] PUBLISH_TRACKS로 SSRC 서버 등록
  - [x] tracks_update 수신 → subscribe PC re-negotiation
- [x] handler.rs fingerprint 중복 접두어 버그 수정
- [x] 브라우저 E2E 테스트 성공 (양방향 RTP, lost=0, jitter<0.003)

## Phase A-3: PLI 키프레임 요청 ✅ (v0.1.7)
> 원인: 새 구독자 입장 시 VP8 키프레임 없이 P-frame만 수신 → Chrome 디코더 10~20초 대기
> 해결: subscribe SRTP ready 시점에 publisher에게 RTCP PLI 전송

### 서버 (light-livechat)
- [x] RTCP PLI 패킷 생성 함수 (12바이트 고정, RFC 4585)
  - FMT=1, PT=206, SSRC of sender=0, SSRC of media=publisher의 video SSRC
- [x] SrtpContext.encrypt_rtcp() 추가
- [x] PLI를 SRTP 암호화하여 publisher의 publish addr로 전송
- [x] subscribe SRTP ready 이벤트 시점에 PLI 트리거
  - udp.rs: DTLS handshake 완료 → PcType::Subscribe인 경우
  - 해당 room의 모든 다른 참가자(publisher)에게 PLI 전송
- [ ] PUBLISH_TRACKS 수신 시점에도 기존 구독자에게 PLI 전송 (late join) — Phase B에서

### 검증
- [ ] 서버 로그에 [DBG:PLI] 태그로 PLI 전송 확인
- [ ] 브라우저 E2E: B 입장 후 1~2초 이내 비디오 표시 확인
- [ ] 기존 기능 회귀 테스트 (audio relay, 정상 퇴장 등)

## Phase B: 통합 테스트 + 다중 참가자
- [x] 3명 동시 접속 테스트 — 영상+오디오 양방향 확인
- [x] app.js 다중 참가자 비디오 대응 (remoteStreams Map + userId 매핑)
- [x] app.js 다중 참가자 오디오 대응 (참가자별 <audio> 엘리먼트)
- [ ] 참가자 중간 퇴장 → inactive m-line 처리 확인
- [ ] 재입장 → 슬롯 재활용 확인
- [ ] subscribe PC 사전 생성 옵션 검토 (joinRoom 시점)

## Phase C: RTCP + 안정화
- [x] RTCP SR/RR transparent relay — v0.2.2
- [x] NACK 수신 → 서버에서 RTX 재전송 — v0.2.0
- [x] REMB 처리 (대역폭 추정 전달) — v0.2.2
- [x] PLI 클라이언트 발 → 해당 publisher에 전달 — v0.2.2
- [x] mute/unmute 이벤트 처리 (시그널링 + 브로드캐스트) — v0.2.3
- [x] 서버 자체 REMB 생성 (Chrome BWE 대역폭 힌트) — v0.3.4
- [x] RR relay metrics 카운터 버그 수정 — v0.3.4
- [x] transport-wide-cc extmap 제거 (TWCC 구현 전 REMB 모드) — v0.3.4
- [x] SDK jitterBufferDelay delta 계산 전환 — v0.3.4
- [x] TWCC feedback 생성 (서버, REMB 대체) — v0.3.8
- [ ] VP8 키프레임 캐시 (LRU) 검토
  - RTP payload에서 VP8 I-frame 감지 (RFC 7741 descriptor + bit0)
  - publisher별 마지막 키프레임 RTP 패킷 묶음(same timestamp) 캐시
  - 새 구독자 입장 시 캐시된 키프레임 즉시 전달 (PLI 왕복 없이 200ms 이내)
  - seq/timestamp rewrite 필요 — 복잡도 높음, 성능 효과 측정 후 결정

## Phase D: Hardening
- [ ] IDENTIFY token verification (JWT or shared secret)
- [x] Zombie session reaper (last_seen timeout) — v0.2.1
- [x] Heartbeat timeout → disconnect — v0.2.1
- [x] DTLS handshake timeout cleanup (zombie reaper에 통합) — v0.2.1
- [x] Graceful shutdown (drain connections) — v0.2.1
- [x] Structured logging (info/debug/trace 레벨 정리) — v0.2.1

## Phase E: PTT Support
- [x] Room mode field (Conference / PTT) — v0.5.0
- [x] Floor control state machine (Idle → Taken → Idle) — v0.5.0
- [x] FLOOR_REQUEST / FLOOR_RELEASE / FLOOR_PING opcodes — v0.5.0
- [x] Floor indicator broadcast (FLOOR_TAKEN/FLOOR_IDLE/FLOOR_REVOKE) — v0.5.0
- [x] E-0: Floor Timer Task (2초 주기, T2/PING 타임아웃 revoke) — v0.5.1
- [x] E-1: Relay Gate (handle_srtp + relay_publish_rtcp PTT 게이팅) — v0.5.1
- [x] E-2: Audio SSRC Rewriting (PttRewriter, Opus PT=111, 오프셋 연산) — v0.5.1
- [x] E-4: Video SSRC Rewriting (VP8 PT=96, 키프레임 대기, is_vp8_keyframe) — v0.5.1
- [x] E-4: NACK 역매핑 (가상seq→원본seq, RtpCache 조회) — v0.5.1
- [x] E-4: Subscribe RTCP relay 가상SSRC→원본SSRC 변환 — v0.5.1
- [x] PTT 메트릭 7개 카운터 + 어드민 PTT 상태 스냅샷 — v0.5.1
- [ ] E-5: 클라이언트 SDK PTT 지원 (Floor UI + 에스컬레이션 면트 연동)

## Phase W: UDP Worker 멀티코어 분산
- [x] W-1: Fan-out spawn — handle_srtp/relay_publish_rtcp fan-out을 tokio::spawn 분리 — v0.3.5
  - 30인 loss 9.6%→1.3%, 4코어 균등 분산 확인
- [x] W-2: Multi-worker (SO_REUSEPORT) — N개 독립 recv 루프, 커널 4-tuple hash 분배 — v0.3.6
  - 30인 loss 0.1%, CPU 113% / 35인 FAIL (outbound_srtp Mutex 경합)
- [x] W-3: Subscriber Egress Task (LiveKit 패턴) — subscriber별 독립 egress pipeline — v0.3.7
  - 30인 loss 0.000%/15ms, 35인 7.0%, 40인 22.8% (RPi 한계)
- [ ] W-4: recvmmsg batch 수신 (선택적, pps 5만+ 시)

## Phase TV: Telemetry Visibility (v0.3.9)
- [x] 환경 메타데이터 (build_mode, log_level, worker_count, bwe_mode, version)
- [x] Egress encrypt timing (Arc<AtomicU64>, lock-free CAS max)
- [x] Tokio RuntimeMetrics (busy_ratio, alive_tasks, global_queue, budget_yield, io_ready)
- [x] Per-worker 상세 (busy_ratio, poll_count, steal_count, noop_count)
- [x] 어드민 대시보드 표시 (Egress Encrypt, Tokio Runtime, Environment)
- [x] Contract 체크: runtime_busy (85% WARN, 95% FAIL)
- [x] 스냅샷 내보내기 연동

## Phase HP: Hot Path 병목 제거 (v0.3.10)
- [x] handle_srtp fan-out: other_participants() Vec → DashMap iter (0 alloc)
- [x] relay_publish_rtcp: 동일 Vec 할당 제거
- [x] handle_nack_block/subscribe_rtcp: all_participants().find() → find_by_track_ssrc() (0 alloc)
- [x] egress_drop 카운터: try_send 실패 시 silent drop → 카운팅
- [x] 어드민: eg_drop 표시 + 경고 배너

## Phase GM: GlobalMetrics 리팩터링 (v0.4.0) ✅
- [x] AtomicTimingStat 구현 (EgressTimingAtomics 일반화)
- [x] GlobalMetrics 구조체 (Arc 공유, 전체 Atomic)
- [x] ServerMetrics + EgressTimingAtomics + spawn atomics 통합
- [x] UdpTransport &mut self → &self 복귀
- [x] egress task 파라미터 정리 (timing 제거, metrics 통합)
- [x] `src/metrics/` 모듈 분리 (env.rs, tokio_snapshot.rs, mod.rs)
- [x] `src/transport/udp/metrics.rs` 제거 (udp/ = 순수 미디어 코어)

## Benchmark
- [x] sfu-bench v0.1.0 완성 (insight-lens/livechat-bench) — publisher 1 + subscriber N 자동화
- [x] RPi 4B fan-out 한계 테스트 (fo1→499, 13회, loss 0.002%, CPU 69%)
- [x] 벤치마크 리포트 문서화 (doc/BENCHMARK-FANOUT-20260306.md)
- [x] Conference 벤치마크 (5/10/20/25/30인, 25인 PASS, 30인 FAIL)
- [x] W-1 Conference 벤치마크 (25인 0%, 30인 1.3%, 35인 13.4%) — v0.3.5
- [x] W-2 Conference 벤치마크 (30인 0.1%, 35인 17.8%) — v0.3.6
- [x] W-3 Conference 벤치마크 (30인 0%, 35인 7%, 40인 22.8%) — v0.3.7
- [ ] x86 서버 벤치마크 (50인+ 목표)
- [ ] TWCC 전후 벤치마크 비교 (Chrome BWE 반응 확인, v0.3.8 vs v0.3.4)

## Backlog
- [ ] Simulcast / SVC (layer detection, adaptive quality)
- [ ] TURN relay support (for restrictive NATs)
- [ ] Recording (RTP → file)
- [ ] Data channel support
- [ ] Horizontal scaling (multi-node)
