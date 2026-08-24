use std::{
    fs,
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

use crate::domain::traffic::{
    TrafficBytes, TrafficCharts, TrafficCounters, TrafficPoint, TrafficSource, TrafficState,
    push_point, rate,
};

const HISTORY_PATH: &str = "/tmp/unetic/traffic-history.json";
type ChartPoints = fn(&mut TrafficCharts) -> &mut Vec<TrafficPoint>;
type TrafficBucket = (u64, ChartPoints);

const BUCKETS: [TrafficBucket; 5] = [
    (1_000, |charts| &mut charts.one_minute),
    (15_000, |charts| &mut charts.fifteen_minutes),
    (60_000, |charts| &mut charts.one_hour),
    (1_440_000, |charts| &mut charts.twenty_four_hours),
    (10_080_000, |charts| &mut charts.seven_days),
];

#[derive(Debug, Clone, Copy, Default)]
struct BucketAccumulator {
    bytes: TrafficBytes,
    elapsed_ms: u64,
}

#[derive(Debug)]
pub(crate) struct TrafficSampler {
    previous: Option<TrafficCounters>,
    sampled_at: Option<std::time::Instant>,
    wan_buckets: [BucketAccumulator; 5],
    lan_buckets: [BucketAccumulator; 5],
    all_buckets: [BucketAccumulator; 5],
}

impl TrafficSampler {
    pub(crate) fn new() -> Self {
        Self {
            previous: None,
            sampled_at: None,
            wan_buckets: [BucketAccumulator::default(); 5],
            lan_buckets: [BucketAccumulator::default(); 5],
            all_buckets: [BucketAccumulator::default(); 5],
        }
    }

    pub(crate) fn sample(
        &mut self,
        current: TrafficCounters,
        hw_offload_enabled: bool,
        state: &mut TrafficState,
    ) -> bool {
        let now = std::time::Instant::now();
        let Some(previous) = self.previous.replace(current) else {
            self.sampled_at = Some(now);
            state.lan.hw_offload_enabled = hw_offload_enabled;
            return false;
        };
        let elapsed_ms = self
            .sampled_at
            .replace(now)
            .map(|at| u64::try_from(now.duration_since(at).as_millis()).unwrap_or(u64::MAX))
            .unwrap_or(0);
        if elapsed_ms == 0 {
            return false;
        }
        let wan = delta(previous.wan, current.wan);
        let lan = delta(previous.lan, current.lan);
        let all = wan + lan;
        let at_ms = now_ms();
        update_source(
            &mut state.wan,
            wan,
            elapsed_ms,
            at_ms,
            &mut self.wan_buckets,
        );
        update_source(
            &mut state.lan.source,
            lan,
            elapsed_ms,
            at_ms,
            &mut self.lan_buckets,
        );
        update_source(
            &mut state.all,
            all,
            elapsed_ms,
            at_ms,
            &mut self.all_buckets,
        );
        state.lan.hw_offload_enabled = hw_offload_enabled;
        true
    }
}

fn delta(previous: TrafficBytes, current: TrafficBytes) -> TrafficBytes {
    TrafficBytes {
        rx: current.rx.saturating_sub(previous.rx),
        tx: current.tx.saturating_sub(previous.tx),
    }
}

fn update_source(
    source: &mut TrafficSource,
    bytes: TrafficBytes,
    elapsed_ms: u64,
    at_ms: u64,
    buckets: &mut [BucketAccumulator; 5],
) {
    source.realtime = rate(bytes, elapsed_ms);
    for (index, (bucket_ms, points)) in BUCKETS.iter().enumerate() {
        let bucket = &mut buckets[index];
        bucket.bytes = bucket.bytes + bytes;
        bucket.elapsed_ms = bucket.elapsed_ms.saturating_add(elapsed_ms);
        if bucket.elapsed_ms >= *bucket_ms {
            let point_rate = rate(bucket.bytes, bucket.elapsed_ms);
            push_point(
                points(&mut source.charts),
                TrafficPoint {
                    at_ms,
                    rx_kbps: point_rate.rx_kbps,
                    tx_kbps: point_rate.tx_kbps,
                },
            );
            *bucket = BucketAccumulator::default();
        }
    }
}

pub(crate) fn load_history() -> TrafficState {
    fs::read(HISTORY_PATH)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

pub(crate) fn save_history(state: &TrafficState) {
    let path = Path::new(HISTORY_PATH);
    let Some(parent) = path.parent() else {
        return;
    };
    if fs::create_dir_all(parent).is_err() {
        return;
    }
    let Ok(bytes) = serde_json::to_vec(state) else {
        return;
    };
    let temporary = path.with_extension("json.tmp");
    if fs::write(&temporary, bytes).is_ok() {
        let _ = fs::rename(temporary, path);
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| u64::try_from(duration.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_is_derived_from_wan_and_lan_deltas() {
        let mut sampler = TrafficSampler::new();
        let mut state = TrafficState::default();
        sampler.sample(
            TrafficCounters {
                wan: TrafficBytes {
                    rx: 1_000,
                    tx: 2_000,
                },
                lan: TrafficBytes {
                    rx: 3_000,
                    tx: 4_000,
                },
            },
            false,
            &mut state,
        );
        sampler.sampled_at = Some(std::time::Instant::now() - std::time::Duration::from_secs(1));
        sampler.sample(
            TrafficCounters {
                wan: TrafficBytes {
                    rx: 2_024,
                    tx: 3_024,
                },
                lan: TrafficBytes {
                    rx: 5_048,
                    tx: 6_048,
                },
            },
            true,
            &mut state,
        );
        assert_eq!(state.wan.realtime.rx_kbps, 1);
        assert_eq!(state.lan.source.realtime.rx_kbps, 2);
        assert_eq!(state.all.realtime.rx_kbps, 3);
        assert!(state.lan.hw_offload_enabled);
    }
}
