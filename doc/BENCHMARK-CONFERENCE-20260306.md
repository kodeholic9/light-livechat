# Light-SFU Conference Benchmark Report

> 2026-03-06 — Raspberry Pi 4B 대상 N인 회의실 시뮬레이션 한계 테스트

## 1. 테스트 개요

### 목적
light-livechat SFU 서버의 실제 화상회의 시나리오 처리 한계를 측정한다.
N명의 참가자가 모두 publish + subscribe를 동시 수행하는 "회의실" 시뮬레이션으로,
총 N×(N-1) 스트림이 서버를 경유한다.

### Fan-out 테스트와의 차이

| | Fan-out (1→N) | Conference (N↔N) |
|---|---|---|
| **구조** | 1 publisher → N subscriber | N participants (all pub + all sub) |
| **입력** | 30 pps (고정) | N × 30 pps (N에 비례) |
| **출력** | N × 30 pps | N × (N-1) × 30 pps (N² 증가) |
| **decrypt** | 1회/프레임 | N회/프레임 |
| **encrypt** | N회/프레임 | N×(N-1)회/프레임 |
| **핵심 차이** | 출력만 증가 | 입출력 모두 N²으로 증가 |

### 테스트 환경

| 항목 | 사양 |
|------|------|
| **서버** | Raspberry Pi 4B (4-core ARM Cortex-A72 @ 1.5GHz, 4GB RAM, GbE) |
| **서버 SW** | light-livechat v0.3.4 (Rust + Tokio + Axum, REMB 모드) |
| **벤치 클라이언트** | Windows PC (sfu-bench v0.2.0, conference mode) |
| **네트워크** | 동일 LAN, 유선 Gigabit Ethernet |
| **서버 설정** | LOG_LEVEL=info, PUBLIC_IP=192.168.0.29, ROOM_CAPACITY=1000 |

### 벤치마크 도구 (sfu-bench v0.2.0 conference mode)

각 참가자가 실제 WebRTC 미디어 파이프라인을 완전히 재현한다:

```
Participant[i]
├── Publish PC:    WS signaling → STUN → DTLS → SRTP outbound → fake RTP send @ 30fps
└── Subscribe PC:  WS signaling → STUN → DTLS → SRTP inbound → receive from N-1 others
```

- 참가자별 고유 SSRC 할당 (90000 + i)
- RTP payload에 `sender_id(2B) + send_timestamp(8B)` 삽입
- Subscriber가 SSRC로 sender 식별 → per-sender loss/latency 개별 추적
- 순차 셋업 (DTLS handshake 폭주 방지) → 전원 PUBLISH_TRACKS → 동시 send/recv 시작

---

## 2. 테스트 결과

### 2.1 결과 요약표

```
┌──────────┬────┬─────────┬────────┬─────────┬────────┬───────┬─────────┬─────────┬──────┬──────┐
│ label    │  N │ streams │ in pps │ out pps │   lost │ loss% │ avg(ms) │ p95(ms) │ CPU% │ 판정 │
├──────────┼────┼─────────┼────────┼─────────┼────────┼───────┼─────────┼─────────┼──────┼──────┤
│ conf-5p  │  5 │      20 │    150 │     600 │      0 │ 0.000 │    3.37 │    6.25 │    7 │ PASS │
│ conf-10p │ 10 │      90 │    300 │   2,700 │      0 │ 0.000 │    5.79 │    9.84 │   22 │ PASS │
│ conf-20p │ 20 │     380 │    600 │  11,362 │      0 │ 0.000 │   10.57 │   19.13 │   52 │ PASS │
│ conf-25p │ 25 │     600 │    750 │  17,948 │     45 │ 0.004 │   15.35 │   26.95 │   80 │ PASS │
│ conf-30p │ 30 │     870 │    900 │  23,488 │ 150075 │ 9.595 │   67.45 │   85.98 │  121 │ FAIL │
└──────────┴────┴─────────┴────────┴─────────┴────────┴───────┴─────────┴─────────┴──────┴──────┘
```

### 2.2 판정 기준

| 등급 | loss | latency avg | 의미 |
|------|------|-------------|------|
| **PASS** | < 0.1% | < 100ms | 실시간 통화 운영 가능 |
| **WARN** | 0.1% ~ 1% | 100~150ms | 주의 필요 |
| **FAIL** | ≥ 1% | ≥ 150ms | 한계 초과 |

---

## 3. 상세 분석

### 3.1 스트림 수 vs CPU (N² 스케일링)

```
N:          5 →   10 →    20 →    25 →    30
streams:   20 →   90 →   380 →   600 →   870
out pps:  600 → 2700 → 11362 → 17948 → 23488
CPU%:       7 →   22 →    52 →    80 →   121
```

- 5→20인: CPU가 스트림 수에 비례하여 선형 증가
- 25인(CPU 80%): 한계 접근 — loss 0.004% 첫 등장
- 30인(CPU 121%): 단일 코어 포화로 급격한 성능 붕괴

### 3.2 Latency 추세

```
N:          5 →   10 →    20 →    25 →    30
avg(ms): 3.37 → 5.79 → 10.57 → 15.35 → 67.45
p95(ms): 6.25 → 9.84 → 19.13 → 26.95 → 85.98
```

- 5→25인: latency가 완만하게 증가 (sub-linear)
- 30인: avg 67ms로 급등 (25인 대비 ×4.4) — CPU 포화에 의한 큐잉 지연

### 3.3 30인 붕괴 원인 — 단일 코어 병목

```
conf-25p (PASS):              conf-30p (FAIL):
  Cpu0:  7.8% us                Cpu0: 73.1% us + 18.5% sy = 91.6%
  Cpu1:  0.2%                   Cpu1:  0.2%
  Cpu2: 25.8%                   Cpu2:  5.2%
  Cpu3: 25.1%                   Cpu3:  0.0%
```

25인에서는 Tokio가 Cpu2/Cpu3에 분산했으나, 30인에서는 UDP hot loop가
Cpu0 한 코어에 집중되어 91.6%에 도달. 나머지 3코어는 유휴 상태.

**원인**: `recv → decrypt → fan-out(encrypt × 29) → send × 29` 루프가
단일 async task로 실행되어 Tokio work-stealing이 분산하지 못함.

**개선 가능성**: fan-out 루프를 per-publisher별로 spawn하면 멀티코어 분산 가능.
이론적으로 4코어 활용 시 30인 이상도 PASS 가능.

### 3.4 참가자 공평성

모든 PASS 라운드에서 참가자 간 latency 편차가 매우 작음:

| N | avg 최소 | avg 최대 | 편차 |
|---|----------|----------|------|
| 5 | 3.20ms | 3.53ms | 0.33ms |
| 10 | 5.48ms | 6.12ms | 0.64ms |
| 20 | 10.14ms | 11.33ms | 1.19ms |
| 25 | 14.89ms | 16.01ms | 1.12ms |

N이 증가해도 참가자 간 공평성이 유지됨. fan-out 루프에서 발생하는
순서 편차가 참가자 수에 의해 평균화되기 때문.

### 3.5 패킷 손실 분석

| N | lost | 패턴 |
|---|------|------|
| 5 | 0 | — |
| 10 | 0 | — |
| 20 | 0 | — |
| 25 | 45 | 25명 중 22명에서 1~3패킷씩 (네트워크 단발성) |
| 30 | 150,075 | 전원 균등 ~5000패킷 (서버 CPU 포화) |

25인의 loss는 단발성 (0.004%), 30인의 loss는 구조적 (9.6%).
25→30 사이에 명확한 cliff가 존재.

### 3.6 메모리 사용량

```
N:          5 →   10 →   20 →   25 →   30
RES(MB):    — →   16 →   24 →   23 →   24
sessions:  10 →   20 →   40 →   50 →   60
```

- 참가자당 2 세션 (pub + sub) × N = 2N 세션
- 60 세션에 24MB — 메모리는 병목 아님

---

## 4. Fan-out 결과와의 비교

동일 서버(RPi 4B)에서 측정한 fan-out과 conference를 비슷한 출력 pps 기준으로 비교:

```
                     │ fan-out 100 │ conf-10p  │ 비고
─────────────────────┼─────────────┼───────────┼──────────────
streams              │     100     │      90   │ 비슷한 규모
input pps            │      30     │     300   │ conf 10×
output pps           │    3,000    │   2,700   │ 비슷
decrypt/frame        │       1     │      10   │ conf 10×
encrypt total        │     100     │      90   │ 비슷
CPU%                 │      22     │      22   │ 동일
avg latency          │   7.25ms    │   5.79ms  │ conf가 낮음
```

출력 pps가 비슷할 때 CPU도 동일하지만, conference의 latency가 낮은 이유:
- fan-out 100: 1패킷을 100번 encrypt하는 긴 루프 (last subscriber 대기 큼)
- conf 10P: 10패킷을 각 9번 encrypt하는 짧은 루프 10개 (per-subscriber 대기 짧음)

---

## 5. 결론

### 5.1 핵심 수치

| 항목 | 결과 |
|------|------|
| **최대 PASS (loss < 0.1%)** | 25인 회의 (600 스트림) |
| **권장 운영 규모 (CPU < 60%)** | 20인 회의 (380 스트림) |
| **최대 출력 (PASS 기준)** | 17,948 pps / 172 Mbps |
| **CPU @ 25인** | 80% |
| **Avg latency @ 20인** | 10.57 ms |
| **Avg latency @ 25인** | 15.35 ms |
| **Loss @ 25인** | 0.004% |
| **FAIL 경계** | 30인 (CPU 121%, loss 9.6%) |

### 5.2 성능 특성

1. **N² 스케일링에도 선형 CPU 증가**: 5→20인에서 CPU가 스트림 수에 비례
2. **참가자 공평성**: 전 구간에서 참가자 간 latency 편차 < 1.2ms
3. **명확한 cliff**: 25인(PASS) → 30인(FAIL), 단일 코어 포화가 원인
4. **메모리 무관**: 60 세션에 24MB, GC 없는 Rust 특성

### 5.3 한계 및 개선 방향

**현재 한계**: UDP hot loop가 단일 async task → 특정 코어 편중 → 30인에서 포화

**개선 방향**:
1. **Fan-out 루프 병렬화**: per-publisher fan-out을 별도 tokio::spawn → 4코어 분산
2. **encrypt batch**: 동일 plaintext를 복수 subscriber에게 보낼 때 encrypt를 batch 처리
3. **TWCC 구현**: 현재 REMB 모드, TWCC로 전환 시 대역폭 적응 개선

**개선 시 예상**: 4코어 균등 분산 시 현재 25인 한계 → **40~50인** 가능

### 5.4 운영 권고

| 규모 | CPU 예상 | 권고 |
|------|----------|------|
| 5인 이하 | < 10% | 라즈베리파이 충분 |
| 10인 | ~22% | 라즈베리파이 권장 |
| 20인 | ~52% | 라즈베리파이 가능 (설계 목표) |
| 25인 | ~80% | 한계 접근, 모니터링 필요 |
| 30인+ | 100%+ | x86 서버 또는 코어 분산 최적화 필요 |

---

## 6. 실행 명령어 기록

```bash
# 서버 설정: LOG_LEVEL=info, ROOM_CAPACITY=1000

# 5인 회의 (20 스트림)
sfu-bench --mode conference --participants 5 --duration 60 --fps 30 \
          --server 192.168.0.29 --ws-port 1974 --udp-port 19740 --label conf-5p

# 10인 회의 (90 스트림)
sfu-bench --mode conference --participants 10 --duration 60 --fps 30 \
          --server 192.168.0.29 --ws-port 1974 --udp-port 19740 --label conf-10p

# 20인 회의 (380 스트림)
sfu-bench --mode conference --participants 20 --duration 60 --fps 30 \
          --server 192.168.0.29 --ws-port 1974 --udp-port 19740 --label conf-20p

# 25인 회의 (600 스트림)
sfu-bench --mode conference --participants 25 --duration 60 --fps 30 \
          --server 192.168.0.29 --ws-port 1974 --udp-port 19740 --label conf-25p

# 30인 회의 (870 스트림) — FAIL
sfu-bench --mode conference --participants 30 --duration 60 --fps 30 \
          --server 192.168.0.29 --ws-port 1974 --udp-port 19740 --label conf-30p
```

---

*author: kodeholic (powered by Claude)*
*sfu-bench v0.2.0 / light-livechat v0.3.4*
