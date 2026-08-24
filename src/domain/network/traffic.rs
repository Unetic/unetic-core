use serde::{Deserialize, Serialize};

pub const TRAFFIC_HISTORY_CAPACITY: usize = 60;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct TrafficBytes {
    pub rx: u64,
    pub tx: u64,
}

impl std::ops::Add for TrafficBytes {
    type Output = Self;

    fn add(self, other: Self) -> Self {
        Self {
            rx: self.rx.saturating_add(other.rx),
            tx: self.tx.saturating_add(other.tx),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct TrafficCounters {
    pub wan: TrafficBytes,
    pub lan: TrafficBytes,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct TrafficRate {
    pub rx_kbps: u64,
    pub tx_kbps: u64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct TrafficPoint {
    pub at_ms: u64,
    pub rx_kbps: u64,
    pub tx_kbps: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct TrafficCharts {
    #[serde(rename = "1m")]
    pub one_minute: Vec<TrafficPoint>,
    #[serde(rename = "15m")]
    pub fifteen_minutes: Vec<TrafficPoint>,
    #[serde(rename = "1h")]
    pub one_hour: Vec<TrafficPoint>,
    #[serde(rename = "24h")]
    pub twenty_four_hours: Vec<TrafficPoint>,
    #[serde(rename = "7d")]
    pub seven_days: Vec<TrafficPoint>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct TrafficSource {
    pub realtime: TrafficRate,
    pub charts: TrafficCharts,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct LanTrafficSource {
    pub hw_offload_enabled: bool,
    #[serde(flatten)]
    pub source: TrafficSource,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct TrafficState {
    pub wan: TrafficSource,
    pub lan: LanTrafficSource,
    pub all: TrafficSource,
}

pub fn rate(delta: TrafficBytes, elapsed_ms: u64) -> TrafficRate {
    if elapsed_ms == 0 {
        return TrafficRate::default();
    }
    TrafficRate {
        rx_kbps: delta.rx.saturating_mul(1_000) / elapsed_ms / 1_024,
        tx_kbps: delta.tx.saturating_mul(1_000) / elapsed_ms / 1_024,
    }
}

pub fn push_point(points: &mut Vec<TrafficPoint>, point: TrafficPoint) {
    if points.len() == TRAFFIC_HISTORY_CAPACITY {
        points.remove(0);
    }
    points.push(point);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rate_uses_elapsed_time_and_kibibytes() {
        assert_eq!(
            rate(
                TrafficBytes {
                    rx: 2_048,
                    tx: 1_024
                },
                500
            ),
            TrafficRate {
                rx_kbps: 4,
                tx_kbps: 2
            }
        );
    }

    #[test]
    fn chart_capacity_is_sixty() {
        let mut points = Vec::new();
        for at_ms in 0..61 {
            push_point(
                &mut points,
                TrafficPoint {
                    at_ms,
                    ..Default::default()
                },
            );
        }
        assert_eq!(points.len(), TRAFFIC_HISTORY_CAPACITY);
        assert_eq!(points[0].at_ms, 1);
    }
}
