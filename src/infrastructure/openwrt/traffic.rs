use std::collections::HashMap;
use std::fs;

pub fn read_iface_counters() -> HashMap<String, (u64, u64)> {
    let mut stats = HashMap::new();
    if let Ok(content) = fs::read_to_string("/proc/net/dev") {
        for line in content.lines().skip(2) {
            let mut parts = line.splitn(2, ':');
            if let (Some(iface_part), Some(stats_part)) = (parts.next(), parts.next()) {
                let iface = iface_part.trim();
                if iface == "lo" {
                    continue;
                }
                let stat_vals: Vec<&str> = stats_part.split_whitespace().collect();
                if stat_vals.len() >= 9 {
                    if let (Ok(rx), Ok(tx)) =
                        (stat_vals[0].parse::<u64>(), stat_vals[8].parse::<u64>())
                    {
                        stats.insert(iface.to_string(), (rx, tx));
                    }
                }
            }
        }
    }
    stats
}
