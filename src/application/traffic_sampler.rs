use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use crate::application::App;
use crate::domain::traffic::{IfaceStats, TrafficState};
use crate::infrastructure::openwrt::traffic::read_iface_counters;

pub fn start_traffic_sampler(app: Arc<App>) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(1));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut prev: HashMap<String, (u64, u64)> = HashMap::new();
        loop {
            interval.tick().await;
            let current = read_iface_counters();
            let mut ifaces = HashMap::new();
            for (iface, (rx, tx)) in &current {
                if let Some(&(prev_rx, prev_tx)) = prev.get(iface) {
                    ifaces.insert(iface.clone(), IfaceStats {
                        rx_bps: rx.saturating_sub(prev_rx),
                        tx_bps: tx.saturating_sub(prev_tx),
                    });
                }
            }
            prev = current;
            let traffic = TrafficState { ifaces, devices: HashMap::new() };
            app.update_traffic(traffic);
        }
    });
}
