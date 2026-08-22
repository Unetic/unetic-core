use std::{
    collections::{HashMap, HashSet},
    fs,
    net::Ipv4Addr,
    process::Command,
};

use crate::{domain::device::Device, domain::errors::LegacyAppError};

#[derive(Debug, Clone)]
pub struct DhcpLease {
    pub ip: String,
    pub hostname: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ArpEntry {
    pub ip: String,
}

pub fn parse_dhcp_leases(content: &str) -> HashMap<String, DhcpLease> {
    let mut leases = HashMap::new();
    for line in content.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 3 {
            continue;
        }

        let mac = parts[1].to_ascii_lowercase();
        let ip = parts[2].to_string();
        let hostname = parts.get(3).and_then(|h| {
            let h = h.trim();
            if h.is_empty() || h == "*" || h == "-" {
                None
            } else {
                Some(h.to_owned())
            }
        });

        leases.insert(mac, DhcpLease { ip, hostname });
    }
    leases
}

pub fn parse_arp_table(content: &str) -> HashMap<String, ArpEntry> {
    let mut entries = HashMap::new();
    for line in content.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 4 || parts[0].eq_ignore_ascii_case("ip") || parts[2] == "0x0" {
            continue;
        }

        let mac = parts[3].to_ascii_lowercase();
        if mac == "00:00:00:00:00:00" || mac == "<incomplete>" || mac.is_empty() {
            continue;
        }

        let ip = parts[0].to_string();
        entries.insert(mac, ArpEntry { ip });
    }
    entries
}

fn is_valid_mac(mac: &str) -> bool {
    let parts: Vec<&str> = mac.split(':').collect();
    parts.len() == 6
        && parts
            .iter()
            .all(|p| p.len() == 2 && p.chars().all(|c| c.is_ascii_hexdigit()))
}

pub fn calculate_distance_m(signal_dbm: i32) -> f32 {
    10_f32.powf((signal_dbm.abs() as f32 - 40.0) / 20.0)
}

pub fn get_wireless_clients() -> HashMap<String, (i32, f32)> {
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
                                let signal_dbm = signal as i32;
                                let distance_m = calculate_distance_m(signal_dbm);
                                stations.insert(mac.to_ascii_lowercase(), (signal_dbm, distance_m));
                            }
                        }
                    }
                }
            }
        }
    }

    stations
}

pub fn merge_devices(
    dhcp_leases: HashMap<String, DhcpLease>,
    arp_entries: HashMap<String, ArpEntry>,
    wireless_clients: HashMap<String, (i32, f32)>,
    mac_to_iface: HashMap<String, String>,
    ip6_by_mac: HashMap<String, String>,
    extenders: &[crate::domain::extender::KnownExtender],
    extender_clients: &HashMap<String, Vec<crate::domain::extender::ExtenderClient>>,
) -> Vec<Device> {
    let mut all_macs: HashSet<String> = HashSet::new();
    all_macs.extend(arp_entries.keys().cloned());
    all_macs.extend(dhcp_leases.keys().cloned());
    all_macs.extend(ip6_by_mac.keys().cloned());

    let mut iface_to_extender: HashMap<String, String> = HashMap::new();
    for ext in extenders {
        let ext_mac = ext.mac.to_lowercase();
        if let Some(iface) = mac_to_iface.get(&ext_mac) {
            iface_to_extender.insert(iface.clone(), ext_mac);
        }
    }

    let mut devices = Vec::new();
    for mac in all_macs {
        let arp = arp_entries.get(&mac);
        let dhcp = dhcp_leases.get(&mac);

        let ip = arp.map(|a| a.ip.clone()).or_else(|| dhcp.map(|d| d.ip.clone()));
        let ip6 = ip6_by_mac.get(&mac).cloned();

        if ip.is_none() && ip6.is_none() {
            continue;
        }

        let hostname = dhcp.and_then(|d| d.hostname.clone());

        let is_extender = extenders.iter().any(|e| e.mac.eq_ignore_ascii_case(&mac));
        let connection = if !is_extender && mac_to_iface.get(&mac).and_then(|i| iface_to_extender.get(i)).is_some() {
            let extender_mac = iface_to_extender[mac_to_iface.get(&mac).unwrap()].clone();
            
            let client = extender_clients.values().flatten().find(|c| c.mac.eq_ignore_ascii_case(&mac));
            let signal_dbm = client.map(|c| c.signal_dbm);
            let distance_m = client.and_then(|c| c.distance_m);

            crate::domain::device::DeviceConnection::ViaExtender { extender_mac, signal_dbm, distance_m }
        } else if let Some(&(signal_dbm, distance_m)) = wireless_clients.get(&mac) {
            crate::domain::device::DeviceConnection::Wireless { signal_dbm, distance_m }
        } else if let Some(iface) = mac_to_iface.get(&mac) {
            let port_id = iface.chars().filter(|c| c.is_ascii_digit()).collect::<String>().parse().unwrap_or(0);
            crate::domain::device::DeviceConnection::Wired { port_id }
        } else {
            crate::domain::device::DeviceConnection::Unknown
        };

        devices.push(Device { mac, ip, ip6, hostname, connection });
    }

    // Sort by IPv4 first, fall back to MAC for stable ordering.
    devices.sort_by(|a, b| {
        let ip_a = a.ip.as_deref().and_then(|s| s.parse::<Ipv4Addr>().ok())
            .map_or([255u8; 4], |ip| ip.octets());
        let ip_b = b.ip.as_deref().and_then(|s| s.parse::<Ipv4Addr>().ok())
            .map_or([255u8; 4], |ip| ip.octets());
        ip_a.cmp(&ip_b).then_with(|| a.mac.cmp(&b.mac))
    });

    devices
}

pub fn read_devices(
    extenders: &[crate::domain::extender::KnownExtender],
    extender_clients: &HashMap<String, Vec<crate::domain::extender::ExtenderClient>>
) -> Result<Vec<Device>, LegacyAppError> {
    let dhcp_content = fs::read_to_string("/tmp/dhcp.leases").unwrap_or_default();
    let arp_content = fs::read_to_string("/proc/net/arp").unwrap_or_default();
    let dhcp_leases = parse_dhcp_leases(&dhcp_content);
    let arp_entries = parse_arp_table(&arp_content);
    let wireless_clients = get_wireless_clients();

    let mut mac_to_iface: HashMap<String, String> = HashMap::new();
    if let Ok(output) = Command::new("bridge").args(["fdb", "show", "dev", "br-lan"]).output() {
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

    Ok(merge_devices(dhcp_leases, arp_entries, wireless_clients, mac_to_iface, ip6_by_mac, extenders, extender_clients))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_dhcp_and_arp() {
        let dhcp_raw = "1724278900 00:11:22:33:44:55 192.168.1.100 phone 01:00\n1724279000 AA:BB:CC:DD:EE:FF 192.168.1.101 * *";
        let arp_raw = "IP address HW type Flags HW address Mask Device\n192.168.1.100 0x1 0x2 00:11:22:33:44:55 * br-lan\n192.168.1.102 0x1 0x0 00:00:00:00:00:00 * br-lan";

        let leases = parse_dhcp_leases(dhcp_raw);
        assert_eq!(leases.len(), 2);
        assert_eq!(leases["00:11:22:33:44:55"].hostname, Some("phone".into()));
        assert_eq!(leases["aa:bb:cc:dd:ee:ff"].hostname, None);

        let arp = parse_arp_table(arp_raw);
        assert_eq!(arp.len(), 1);
        assert_eq!(arp["00:11:22:33:44:55"].ip, "192.168.1.100");
    }

    #[test]
    fn test_merge_and_sort() {
        let dhcp = parse_dhcp_leases(
            "1724278900 00:11:22:33:44:55 192.168.1.100 Alice-Phone *\n1724279000 aa:bb:cc:dd:ee:ff 192.168.1.101 Desktop *",
        );
        let arp = parse_arp_table(
            "192.168.1.100 0x1 0x2 00:11:22:33:44:55 * br-lan\n192.168.1.150 0x1 0x2 11:22:33:44:55:66 * br-lan",
        );
        let mut wireless = HashMap::new();
        wireless.insert("00:11:22:33:44:55".into(), (-60, 10.0));
        let mut mac_to_iface = HashMap::new();
        mac_to_iface.insert("aa:bb:cc:dd:ee:ff".into(), "lan1".into());
        let mut ip6_by_mac = HashMap::new();
        ip6_by_mac.insert("00:11:22:33:44:55".into(), "2001:db8::1".into());

        let extenders: Vec<crate::domain::extender::KnownExtender> = Vec::new();
        let extender_clients = HashMap::new();
        let devices = merge_devices(dhcp, arp, wireless, mac_to_iface, ip6_by_mac, &extenders, &extender_clients);
        assert_eq!(devices.len(), 3);
        assert_eq!(devices[0].mac, "00:11:22:33:44:55");
        assert_eq!(devices[0].ip, Some("192.168.1.100".into()));
        assert_eq!(devices[0].ip6, Some("2001:db8::1".into()));
        assert_eq!(devices[0].connection, crate::domain::device::DeviceConnection::Wireless { signal_dbm: -60, distance_m: 10.0 });
        assert_eq!(devices[1].mac, "aa:bb:cc:dd:ee:ff");
        assert_eq!(devices[1].connection, crate::domain::device::DeviceConnection::Wired { port_id: 1 });
        assert_eq!(devices[2].mac, "11:22:33:44:55:66");
        assert_eq!(devices[2].hostname, None);
    }
}
