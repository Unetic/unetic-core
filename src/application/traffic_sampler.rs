use std::{collections::HashMap, sync::Arc, time::Duration};

use crate::{
    application::App,
    domain::traffic::{IfaceStats, TrafficState},
    infrastructure::openwrt::traffic::read_iface_counters,
};

pub fn start_traffic_sampler(app: Arc<App>) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(
            crate::domain::TRAFFIC_SAMPLING_INTERVAL_SECS,
        ));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut previous = read_iface_counters();
        let mut sampled_at = tokio::time::Instant::now();

        loop {
            interval.tick().await;
            let current = read_iface_counters();
            let now = tokio::time::Instant::now();
            let ifaces = calculate_rates(&previous, &current, now.duration_since(sampled_at));
            previous = current;
            sampled_at = now;
            app.update_traffic(TrafficState {
                ifaces,
                devices: HashMap::new(),
            });
        }
    });
}

fn calculate_rates(
    previous: &HashMap<String, (u64, u64)>,
    current: &HashMap<String, (u64, u64)>,
    elapsed: Duration,
) -> HashMap<String, IfaceStats> {
    let elapsed_millis = u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX);
    if elapsed_millis == 0 {
        return HashMap::new();
    }

    current
        .iter()
        .filter_map(|(interface, &(rx, tx))| {
            let &(previous_rx, previous_tx) = previous.get(interface)?;
            Some((
                interface.clone(),
                IfaceStats {
                    rx_bps: bytes_per_second(rx.saturating_sub(previous_rx), elapsed_millis),
                    tx_bps: bytes_per_second(tx.saturating_sub(previous_tx), elapsed_millis),
                },
            ))
        })
        .collect()
}

fn bytes_per_second(bytes: u64, elapsed_millis: u64) -> u64 {
    bytes.saturating_mul(1_000) / elapsed_millis
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calculates_bytes_per_second_using_actual_interval() {
        let previous = HashMap::from([("eth0".to_owned(), (1_000, 2_000))]);
        let current = HashMap::from([("eth0".to_owned(), (2_000, 2_500))]);

        let rates = calculate_rates(&previous, &current, Duration::from_millis(500));

        assert_eq!(rates["eth0"].rx_bps, 2_000);
        assert_eq!(rates["eth0"].tx_bps, 1_000);
    }
}
