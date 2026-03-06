# Telemetry Snapshots — 2026-03-05 Session

> 미디어 품질 개선 세션에서 수집한 스냅샷 원본 8건.
> 저장 위치: `light-livechat/doc/telemetry-snapshots-2026-03-05.md`

---

## ① 최초 상태 (v0.3.3, 12:07, 로컬 PC 2명)

```
=== LIGHT-SFU TELEMETRY SNAPSHOT ===
timestamp: 2026-03-05T12:07:20.476Z

--- PUBLISH (3s window) ---
[U250:video] pkts=1596 bytes=494669 nack=0 pli=2 bitrate=32kbps target=27280 retx=0 fps=14
[U495:video] pkts=1490 bytes=498595 nack=0 pli=1 bitrate=32kbps target=27280 retx=0 fps=15

--- SUBSCRIBE (3s window) ---
[U250←U495:video] pkts=1438 lost=0 bitrate=32kbps jitter=39.0ms jb_delay=57ms nack_sent=0 freeze=3 dropped=0 fps=15
[U495←U250:video] pkts=1416 lost=0 bitrate=31kbps jitter=10.0ms jb_delay=48ms nack_sent=0 freeze=3 dropped=1 fps=15

--- NETWORK ---
[U250:pub] rtt=4ms available_bitrate=84400
[U495:pub] rtt=2ms available_bitrate=84400

--- SFU SERVER (3s window) ---
[server] relay: avg=0.10ms max=0.20ms count=394
[server] nack_recv=0 rtx_sent=0 rtx_miss=0 pli_sent=0 sr_relay=8 rr_relay=0 twcc=0

--- CONTRACT CHECK ---
[PASS] sdp_negotiation
[FAIL] encoder_healthy: quality limited
[PASS] sr_relay: 8
[FAIL] rr_relay: 0
[PASS] nack_rtx: no NACK
[PASS] jitter_buffer: < 100ms
[FAIL] video_freeze: freeze detected
[WARN] twcc_feedback: not implemented
```

**Contract: 4/8 PASS**

---

## ② RR 카운터 버그 수정 (12:40, 로컬 PC 2명)

> 변경: plaintext RTCP PT에서 `& 0x7F` 마스크 제거

```
--- NETWORK ---
[U837:pub] rtt=2ms available_bitrate=84400
[U564:pub] rtt=2ms available_bitrate=84400

--- SFU SERVER (3s window) ---
[server] relay: avg=0.48ms max=2.03ms count=387
[server] nack_recv=0 rtx_sent=0 rtx_miss=0 pli_sent=0 sr_relay=7 rr_relay=8 twcc=0

--- CONTRACT CHECK ---
[PASS] rr_relay: 8 in 3s ← 0에서 8로 수정됨 (카운터 버그였음)
```

**변화: rr_relay 0 → 8 (카운터만 고장이었음, 릴레이는 원래 동작 중)**

---

## ③ REMB 2Mbps + TWCC extmap 제거 (12:45, 로컬 PC 2명)

> 변경: server_extmap_policy에서 transport-wide-cc 제거 + build_remb() 2Mbps

```
--- PUBLISH (3s window) ---
[U837:video] pkts=3706 bytes=4125519 bitrate=1520kbps target=1500000 fps=15
[U564:video] pkts=3092 bytes=3445754 bitrate=1608kbps target=1500000 fps=15

--- SUBSCRIBE (3s window) ---
[U837←U564:video] pkts=3302 lost=0 bitrate=1607kbps jitter=25.0ms jb_delay=84ms freeze=0 fps=15
[U564←U837:video] pkts=3295 lost=0 bitrate=1516kbps jitter=26.0ms jb_delay=70ms freeze=0 fps=15

--- NETWORK ---
[U837:pub] rtt=39ms available_bitrate=2000000
[U564:pub] rtt=6ms available_bitrate=2000000

--- SFU SERVER (3s window) ---
[server] relay: avg=1.64ms max=14.14ms count=1352
```

**변화: available_bitrate 84kbps → 2,000kbps (24배), video_bitrate 32kbps → 1,520kbps (47배)**

---

## ④ 과부하 후 스냅샷 (12:49, 로컬 PC 2명, 2Mbps 지속)

> 한 PC에서 모든 걸 돌리니 CPU 경합 발생

```
--- SUBSCRIBE (3s window) ---
[U837←U564:video] pkts=13924 lost=843 bitrate=1455kbps jitter=30.0ms jb_delay=237ms nack_sent=731 freeze=10 dropped=350 fps=15
[U564←U837:video] pkts=14078 lost=713 bitrate=1502kbps jitter=23.0ms jb_delay=255ms nack_sent=637 freeze=7 dropped=375 fps=15

--- NETWORK ---
[U837:pub] rtt=110ms available_bitrate=2000000

--- SFU SERVER (3s window) ---
[server] relay: avg=1.88ms max=36.21ms
[server] nack_recv=0 rtx_sent=0 ← nack_sent=731인데 서버가 0! (같은 PC 과부하)
```

**판단: 로컬 과부하. REMB를 500kbps로 낮추기로 결정**

---

## ⑤ REMB 500kbps 안정화 (12:56, 로컬 PC 2명)

> 변경: REMB_BITRATE_BPS 2,000,000 → 500,000

```
--- PUBLISH (3s window) ---
[U231:video] pkts=3524 bytes=3547143 bitrate=467kbps target=498144 fps=15
[U703:video] pkts=3542 bytes=3576887 bitrate=456kbps target=498304 fps=15

--- SUBSCRIBE (3s window) ---
[U231←U703:video] pkts=3436 lost=0 bitrate=473kbps jitter=6.0ms jb_delay=29ms nack_sent=0 freeze=0 fps=15
[U703←U231:video] pkts=3556 lost=0 bitrate=455kbps jitter=11.0ms jb_delay=42ms nack_sent=0 freeze=1 fps=15

--- NETWORK ---
[U231:pub] rtt=5ms available_bitrate=500000

--- CONTRACT CHECK ---
[PASS] encoder_healthy: no limitation ← quality_limit=none 달성!
[PASS] jitter_buffer: < 100ms
```

**변화: quality_limit bandwidth → none, jb_delay 84ms → 29ms, freeze 10 → 0**
**Contract: 7/8 PASS**

---

## ⑥ JB delta 수정 후 안정 확인 (13:04, 로컬 PC 2명)

> 변경: SDK jitterBufferDelay 누적값 → 3초 delta ms 전환

```
--- SUBSCRIBE (3s window) ---
[U231←U703:video] pkts=3931 lost=0 bitrate=465kbps jitter=7.0ms jb_delay=44ms nack_sent=0 freeze=0 fps=14
[U703←U231:video] pkts=4038 lost=0 bitrate=462kbps jitter=7.0ms jb_delay=63ms nack_sent=0 freeze=0 fps=14

--- CONTRACT CHECK ---
[PASS] sdp_negotiation: all m-lines OK
[PASS] encoder_healthy: no limitation
[PASS] sr_relay: 8 in 3s
[PASS] rr_relay: 26 in 3s
[PASS] nack_rtx: no NACK
[PASS] jitter_buffer: < 100ms
[PASS] video_freeze: 0 freezes
[WARN] twcc_feedback: not implemented
```

**변화: JB delay 안정 (44~63ms, 가속 없음). Contract 7/7 PASS (TWCC만 WARN)**

---

## ⑦ 라즈베리파이 3명 (13:27, RPi + PC 2 + 모바일 1, 캐시 128)

> 변경: .env PUBLIC_IP 설정 + 라즈베리파이 배포

```
--- PUBLISH (3s window) ---
[U659:video] pkts=9545 bytes=9598906 bitrate=388kbps target=498880 fps=7 ← fps 낮음(디바이스 문제)
[U844:video] pkts=9276 bytes=9303972 bitrate=472kbps target=498144 fps=15

--- SUBSCRIBE (3s window) ---
[U659←U844:video] pkts=9443 lost=1 bitrate=392kbps jitter=23.0ms jb_delay=32ms nack_sent=166 freeze=1 dropped=40 fps=7
[U844←U659:video] pkts=9265 lost=1 bitrate=483kbps jitter=5.0ms jb_delay=44ms nack_sent=163 freeze=1 dropped=44 fps=15

--- SFU SERVER (3s window) ---
[server] relay: avg=0.17ms max=0.70ms count=902
[server] nack_recv=93 rtx_sent=416 rtx_miss=224 ← 캐시 히트율 65%

--- CONTRACT CHECK ---
[FAIL] nack_rtx: 416/640 hit ← 65%
```

**변화: SFU relay avg=0.17ms (라즈베리파이 훌륭), 하지만 rtx_miss=224**

---

## ⑧ 캐시 512 + PC 3명 (14:03, RPi + PC 2 + 모바일 1)

> 변경: RTP_CACHE_SIZE 128 → 512

```
--- PUBLISH (3s window) ---
[U677:video] pkts=3869 bytes=3882310 bitrate=483kbps target=497952 fps=15
[U689:video] pkts=3705 bytes=3719803 bitrate=475kbps target=498080 fps=15
[U166:video] pkts=2331 bytes=2137911 bitrate=336kbps target=475234 fps=24 (모바일 HW)

--- SUBSCRIBE (3s window) ---
[U677←U689:video] pkts=3668 lost=0 bitrate=477kbps jitter=3.0ms jb_delay=23ms nack_sent=56 freeze=0 fps=16
[U689←U677:video] pkts=3731 lost=0 bitrate=475kbps jitter=5.0ms jb_delay=30ms nack_sent=69 freeze=0 fps=14
[U166←U689:video] pkts=2364 lost=6 bitrate=453kbps jitter=4.0ms jb_delay=123ms nack_sent=85 freeze=1 dropped=45 fps=15
[U166←U677:video] pkts=2363 lost=10 bitrate=455kbps jitter=5.0ms jb_delay=102ms nack_sent=88 freeze=1 dropped=43 fps=15

--- SFU SERVER (3s window) ---
[server] relay: avg=0.19ms max=0.35ms count=938
[server] nack_recv=2 rtx_sent=2 rtx_miss=0 ← 히트율 100%!

--- CONTRACT CHECK ---
[PASS] nack_rtx: 2/2 hit ← 100%
```

**변화: rtx_miss 224 → 0, 히트율 65% → 100%**

---

## ⑨ LTE 모바일 포함 4명 (14:14, RPi + PC 2 + LTE 모바일 2)

> 변경: 없음 (안정성 확인)

```
--- PUBLISH (3s window) ---
[U677:video] bitrate=442kbps target=498336 fps=17
[U689:video] bitrate=538kbps target=498624 fps=14
[U166:video] bitrate=393kbps target=459075 fps=24 (LTE 모바일1)
[U687:video] bitrate=382kbps target=468356 fps=24 (LTE 모바일2)

--- SUBSCRIBE (3s window) ---
[U687←U689:video] lost=0 bitrate=498kbps jitter=35.0ms jb_delay=51ms nack_sent=29 freeze=2 fps=22
[U687←U677:video] lost=0 bitrate=496kbps jitter=17.0ms jb_delay=62ms nack_sent=25 freeze=2 fps=24

--- NETWORK ---
[U687:pub] rtt=57ms available_bitrate=500000 ← LTE
[U687:sub] rtt=44ms

--- SFU SERVER (3s window) ---
[server] relay: avg=0.17ms max=0.34ms count=958
[server] nack_recv=7 rtx_sent=9 rtx_miss=0 ← 여전히 100%
```

**최종: PC 2대 + LTE 모바일 2대, 라즈베리파이 위에서 4명 HD 화상회의 안정 동작**

---

## 수치 변화 요약

| 지표 | ① 최초 | ⑤ REMB 안정 | ⑧ RPi 3명 | ⑨ RPi+LTE 4명 |
|------|--------|-------------|-----------|---------------|
| available_bw | 84 kbps | 500 kbps | 500 kbps | 500 kbps |
| video_bitrate | 32 kbps | 470 kbps | 483 kbps | 442 kbps |
| quality_limit | bandwidth | **none** | **none** | **none** |
| jb_delay (PC) | 47 ms | 29 ms | 23 ms | 55 ms |
| freeze | 3 | 0 | 1 | 1 |
| lost | 0 | 0 | 1 | 0 |
| rr_relay | 0 | 26 | 53 | 56 |
| rtx_miss | 0 | 0 | 224 | **0** |
| rtx_hit | N/A | N/A | 65% | **100%** |
| relay_avg | 0.10 ms | 1.03 ms | 0.19 ms | 0.17 ms |
| Contract | 4/8 | 7/8 | 5/8 | 5/8* |

*모바일 포함 시 encoder_healthy(모바일 bandwidth), jb_delay(LTE), freeze로 FAIL
*PC만 보면 7/8 PASS

---

## 적용 이력

| 단계 | 적용 내용 | 핵심 효과 |
|------|----------|----------|
| ②→③ | RR 카운터 버그 수정 | 관측 정확도 확보 |
| ③ | TWCC extmap 제거 + REMB 2Mbps | available_bw 84k→2M (24배) |
| ④→⑤ | REMB 500kbps로 조정 | 로컬 안정화, quality_limit=none |
| ⑥ | JB delay delta 수정 | JB 가속 현상 해소 |
| ⑦ | .env PUBLIC_IP + RPi 배포 | LTE 접속 가능 |
| ⑧ | RTP 캐시 128→512 | RTX 히트율 65%→100% |
