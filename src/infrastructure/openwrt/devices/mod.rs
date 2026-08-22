mod catalog;

use std::{collections::HashMap, fs, process::Command};

use crate::{domain::device::Device, domain::errors::LegacyAppError};
pub use catalog::{merge_devices, parse_arp_table, parse_dhcp_leases};

pub fn get_wireless_clients() -> HashMap<String, i32> {
    let mut stations = HashMap::new();
    let mut ifaces = Vec::new();

    if let Ok(output) = Command::new("iw").arg("dev").output() {
        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            for line in stdout.lines() {
                if let Some(rest) = line.trim().strip_prefix("Interface ") {
                    let iface = rest.trim();
                    if !iface.is_empty() {
                        ifaces.push(iface.to_owned());
                    }
                }
            }
        }
    }

    if ifaces.is_empty() {
        if let Ok(entries) = fs::read_dir("/sys/class/net") {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.join("wireless").exists() || path.join("phy80211").exists() {
                    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                        ifaces.push(name.to_owned());
                    }
                }
            }
        }
    }

    if ifaces.is_empty() {
        ifaces.push("wlan0".into());
        ifaces.push("wlan1".into());
    }

    for iface in ifaces {
        if let Ok(output) = Command::new("ubus")
            .args(["call", &format!("hostapd.{}", iface), "get_clients"])
            .output()
        {
            if output.status.success() {
                if let Ok(json) = serde_json::from_slice::<serde_json::Value>(&output.stdout) {
                    if let Some(clients) = json.get("clients").and_then(|c| c.as_object()) {
                        for (mac, info) in clients {
                            if let Some(signal) = info.get("signal").and_then(|s| s.as_i64()) {
                                stations.insert(mac.to_ascii_lowercase(), signal as i32);
                            }
                        }
                    }
                }
            }
        }
    }

    stations
}

pub fn read_devices(
    extenders: &[crate::domain::extender::KnownExtender],
    extender_clients: &HashMap<String, Vec<crate::domain::extender::ExtenderClient>>,
) -> Result<Vec<Device>, LegacyAppError> {
    let dhcp_content = fs::read_to_string("/tmp/dhcp.leases").unwrap_or_default();
    let arp_content = fs::read_to_string("/proc/net/arp").unwrap_or_default();
    let dhcp_leases = parse_dhcp_leases(&dhcp_content);
    let arp_entries = parse_arp_table(&arp_content);
    let wireless_clients = get_wireless_clients();

    let mut mac_to_iface: HashMap<String, String> = HashMap::new();
    if let Ok(output) = Command::new("bridge")
        .args(["fdb", "show", "dev", "br-lan"])
        .output()
    {
        if output.status.success() {
            if let Ok(fdb_str) = String::from_utf8(output.stdout) {
                for line in fdb_str.lines() {
                    let parts: Vec<&str> = line.split_whitespace().collect();
                    if parts.len() >= 3 && parts[1] == "dev" {
                        let mac = parts[0].to_lowercase();
                        let iface = parts[2].to_string();
                        mac_to_iface.insert(mac, iface);
                    }
                }
            }
        }
    }

    // Read global IPv6 addresses from the kernel NDP table.
    // Format: ip6_addr dev_index state flags iface
    let mut ip6_by_mac: HashMap<String, String> = HashMap::new();
    if let Ok(output) = Command::new("ip").args(["-6", "neigh", "show"]).output() {
        if output.status.success() {
            if let Ok(s) = String::from_utf8(output.stdout) {
                for line in s.lines() {
                    // Example: "2001:db8::1 dev br-lan lladdr aa:bb:cc:dd:ee:ff REACHABLE"
                    let parts: Vec<&str> = line.split_whitespace().collect();
                    if parts.len() >= 5 {
                        let ip6 = parts[0];
                        // Skip link-local.
                        if ip6.starts_with("fe80") {
                            continue;
                        }
                        // lladdr is preceded by the keyword "lladdr".
                        if let Some(pos) = parts.iter().position(|&p| p == "lladdr") {
                            if let Some(&mac) = parts.get(pos + 1) {
                                ip6_by_mac.insert(mac.to_lowercase(), ip6.to_string());
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(merge_devices(
        dhcp_leases,
        arp_entries,
        wireless_clients,
        mac_to_iface,
        ip6_by_mac,
        extenders,
        extender_clients,
    ))
}

#[cfg(test)]
mod tests;
