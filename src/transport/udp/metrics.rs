// author: kodeholic (powered by Claude)
//! Server metrics — B구간 계측 (timing accumulators + RTCP counters)

/// 3초 주기 집계를 위한 타이밍 어퀴뮬레이터
pub(crate) struct TimingStat {
    pub(crate) sum_us: u64,
    pub(crate) count:  u64,
    pub(crate) min_us: u64,
    pub(crate) max_us: u64,
}

impl TimingStat {
    pub(crate) fn new() -> Self {
        Self { sum_us: 0, count: 0, min_us: u64::MAX, max_us: 0 }
    }

    pub(crate) fn record(&mut self, us: u64) {
        self.sum_us += us;
        self.count += 1;
        if us < self.min_us { self.min_us = us; }
        if us > self.max_us { self.max_us = us; }
    }

    pub(crate) fn avg(&self) -> u64 {
        if self.count == 0 { 0 } else { self.sum_us / self.count }
    }

    #[allow(dead_code)]
    pub(crate) fn p95_approx(&self) -> u64 {
        // 정확한 p95는 histogram 필요. 단순 근사: max * 0.95 또는 avg + (max-avg)*0.8
        // 여기서는 max를 그대로 노출 (p95 대신 max 사용)
        self.max_us
    }

    pub(crate) fn to_json(&self) -> serde_json::Value {
        if self.count == 0 {
            return serde_json::json!(null);
        }
        serde_json::json!({
            "avg_us": self.avg(),
            "min_us": if self.min_us == u64::MAX { 0 } else { self.min_us },
            "max_us": self.max_us,
            "count":  self.count,
        })
    }

    pub(crate) fn reset(&mut self) {
        self.sum_us = 0;
        self.count = 0;
        self.min_us = u64::MAX;
        self.max_us = 0;
    }
}

pub(crate) struct ServerMetrics {
    // B-1: relay total (decrypt ~ last send_to)
    pub(crate) relay:          TimingStat,
    // B-2: SRTP decrypt
    pub(crate) decrypt:        TimingStat,
    // B-3: SRTP encrypt (per target)
    pub(crate) encrypt:        TimingStat,
    // B-4: Mutex lock wait
    pub(crate) lock_wait:      TimingStat,
    // B-5: fan-out count per relay
    pub(crate) fan_out_sum:    u64,
    pub(crate) fan_out_count:  u64,
    pub(crate) fan_out_min:    u32,
    pub(crate) fan_out_max:    u32,
    // B-6, B-7: encrypt/decrypt failures
    pub(crate) encrypt_fail:   u64,
    pub(crate) decrypt_fail:   u64,
    // B-8~14: RTCP counters
    pub(crate) nack_received:  u64,
    pub(crate) rtx_sent:       u64,
    pub(crate) rtx_cache_miss: u64,
    pub(crate) pli_sent:       u64,
    pub(crate) sr_relayed:     u64,
    pub(crate) rr_relayed:     u64,
    pub(crate) twcc_sent:      u64,
    pub(crate) twcc_recorded:   u64,
    // subscribe RTCP 진단 카운터
    pub(crate) sub_rtcp_received: u64,
    pub(crate) sub_rtcp_not_rtcp: u64,
    pub(crate) sub_rtcp_decrypted: u64,
}

impl ServerMetrics {
    pub(crate) fn new() -> Self {
        Self {
            relay:          TimingStat::new(),
            decrypt:        TimingStat::new(),
            encrypt:        TimingStat::new(),
            lock_wait:      TimingStat::new(),
            fan_out_sum:    0,
            fan_out_count:  0,
            fan_out_min:    u32::MAX,
            fan_out_max:    0,
            encrypt_fail:   0,
            decrypt_fail:   0,
            nack_received:  0,
            rtx_sent:       0,
            rtx_cache_miss: 0,
            pli_sent:       0,
            sr_relayed:     0,
            rr_relayed:     0,
            twcc_sent:      0,
            twcc_recorded:   0,
            sub_rtcp_received: 0,
            sub_rtcp_not_rtcp: 0,
            sub_rtcp_decrypted: 0,
        }
    }

    #[allow(dead_code)]
    pub(crate) fn record_fan_out(&mut self, count: u32) {
        self.fan_out_sum += count as u64;
        self.fan_out_count += 1;
        if count < self.fan_out_min { self.fan_out_min = count; }
        if count > self.fan_out_max { self.fan_out_max = count; }
    }

    pub(crate) fn to_json(&self) -> serde_json::Value {
        let fan_out_avg = if self.fan_out_count == 0 { 0.0 }
            else { self.fan_out_sum as f64 / self.fan_out_count as f64 };
        serde_json::json!({
            "type": "server_metrics",
            "relay":          self.relay.to_json(),
            "decrypt":        self.decrypt.to_json(),
            "encrypt":        self.encrypt.to_json(),
            "lock_wait":      self.lock_wait.to_json(),
            "fan_out": {
                "avg": format!("{:.1}", fan_out_avg),
                "min": if self.fan_out_min == u32::MAX { 0 } else { self.fan_out_min },
                "max": self.fan_out_max,
            },
            "encrypt_fail":   self.encrypt_fail,
            "decrypt_fail":   self.decrypt_fail,
            "nack_received":  self.nack_received,
            "rtx_sent":       self.rtx_sent,
            "rtx_cache_miss": self.rtx_cache_miss,
            "pli_sent":       self.pli_sent,
            "sr_relayed":     self.sr_relayed,
            "rr_relayed":     self.rr_relayed,
            "twcc_sent":      self.twcc_sent,
            "twcc_recorded":   self.twcc_recorded,
            "sub_rtcp_received": self.sub_rtcp_received,
            "sub_rtcp_not_rtcp": self.sub_rtcp_not_rtcp,
            "sub_rtcp_decrypted": self.sub_rtcp_decrypted,
        })
    }

    pub(crate) fn reset(&mut self) {
        self.relay.reset();
        self.decrypt.reset();
        self.encrypt.reset();
        self.lock_wait.reset();
        self.fan_out_sum = 0;
        self.fan_out_count = 0;
        self.fan_out_min = u32::MAX;
        self.fan_out_max = 0;
        self.encrypt_fail = 0;
        self.decrypt_fail = 0;
        self.nack_received = 0;
        self.rtx_sent = 0;
        self.rtx_cache_miss = 0;
        self.pli_sent = 0;
        self.sr_relayed = 0;
        self.rr_relayed = 0;
        self.twcc_sent = 0;
        self.twcc_recorded = 0;
        self.sub_rtcp_received = 0;
        self.sub_rtcp_not_rtcp = 0;
        self.sub_rtcp_decrypted = 0;
    }
}
