mod catalog;

use std::{collections::HashMap, fs, process::Command};

use crate::{domain::device::Device, domain::errors::LegacyAppError};
pub use catalog::{merge_devices, parse_arp_table, parse_dhcp_leases};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WirelessClient {
    pub interface: String,
    pub network: Option<String>,
    pub signal_dbm: i32,
}

pub fn get_wireless_clients() -> HashMap<String, WirelessClient> {
    let mut stations = HashMap::new();
    for (interface, network) in wireless_interfaces() {
        let object = format!("hostapd.{interface}");
        let Ok(reply) = crate::infrastructure::openwrt::rpc::call_ubus(
            &object,
            "get_clients",
            serde_json::json!({}),
        ) else {
            continue;
        };
        let Some(clients) = reply.get("clients").and_then(|clients| clients.as_object()) else {
            continue;
        };
        for (mac, info) in clients {
            let Some(signal_dbm) = info.get("signal").and_then(|signal| signal.as_i64()) else {
                continue;
            };
            stations.insert(
                mac.to_ascii_lowercase(),
                WirelessClient {
                    interface: interface.clone(),
                    network: network.clone(),
                    signal_dbm: signal_dbm as i32,
                },
            );
        }
    }
    stations
}

fn wireless_interfaces() -> Vec<(String, Option<String>)> {
    let mut interfaces = Vec::new();
    if let Ok(reply) = crate::infrastructure::openwrt::rpc::call_ubus(
        "network.wireless",
        "status",
        serde_json::json!({}),
    ) {
        if let Some(radios) = reply.as_object() {
            for radio in radios.values() {
                let Some(entries) = radio.get("interfaces").and_then(|value| value.as_array()) else {
                    continue;
                };
                for entry in entries {
                    let Some(interface) = entry.get("ifname").and_then(|value| value.as_str()) else {
                        continue;
                    };
                    let network = entry
                        .get("config")
                        .and_then(|config| config.get("ssid"))
                        .and_then(|ssid| ssid.as_str())
                        .map(str::to_owned);
                    interfaces.push((interface.to_owned(), network));
                }
            }
        }
    }

    if interfaces.is_empty() && let Ok(entries) = fs::read_dir("/sys/class/net") {
        for entry in entries.flatten() {
            let path = entry.path();
            if !(path.join("wireless").exists() || path.join("phy80211").exists()) {
                continue;
            }
            if let Some(name) = path.file_name().and_then(|name| name.to_str()) {
                interfaces.push((name.to_owned(), None));
            }
        }
    }
    interfaces
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
