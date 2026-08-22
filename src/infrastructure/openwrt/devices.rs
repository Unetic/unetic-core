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
    pub device: String,
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
        let device = parts.get(5).copied().unwrap_or("").to_string();
        entries.insert(mac, ArpEntry { ip, device });
    }
    entries
}

pub fn parse_station_dump(stdout: &str) -> HashSet<String> {
    let mut stations = HashSet::new();
    for line in stdout.lines() {
        if let Some(rest) = line.trim().strip_prefix("Station ") {
            let mac = rest
                .split_whitespace()
                .next()
                .unwrap_or("")
                .to_ascii_lowercase();
            if is_valid_mac(&mac) {
                stations.insert(mac);
            }
        }
    }
    stations
}

pub fn parse_iwinfo_assoclist(stdout: &str) -> HashSet<String> {
    let mut stations = HashSet::new();
    for line in stdout.lines() {
        let mac = line
            .split_whitespace()
            .next()
            .unwrap_or("")
            .to_ascii_lowercase();
        if is_valid_mac(&mac) {
            stations.insert(mac);
        }
    }
    stations
}

fn is_valid_mac(mac: &str) -> bool {
    let parts: Vec<&str> = mac.split(':').collect();
    parts.len() == 6
        && parts
            .iter()
            .all(|p| p.len() == 2 && p.chars().all(|c| c.is_ascii_hexdigit()))
}

pub fn get_wireless_macs() -> HashSet<String> {
    let mut stations = HashSet::new();
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
        if let Ok(output) = Command::new("iw")
            .args(["dev", &iface, "station", "dump"])
            .output()
        {
            if output.status.success() {
                stations.extend(parse_station_dump(&String::from_utf8_lossy(&output.stdout)));
            }
        }
        if let Ok(output) = Command::new("iwinfo").args([&iface, "assoclist"]).output() {
            if output.status.success() {
                stations.extend(parse_iwinfo_assoclist(&String::from_utf8_lossy(
                    &output.stdout,
                )));
            }
        }
    }

    stations
}

pub fn merge_devices(
    dhcp_leases: HashMap<String, DhcpLease>,
    arp_entries: HashMap<String, ArpEntry>,
    wireless_macs: HashSet<String>,
) -> Vec<Device> {
    let mut all_macs: HashSet<String> = HashSet::new();
    all_macs.extend(arp_entries.keys().cloned());
    all_macs.extend(dhcp_leases.keys().cloned());

    let mut devices = Vec::new();
    for mac in all_macs {
        let arp = arp_entries.get(&mac);
        let dhcp = dhcp_leases.get(&mac);

        let ip = match (arp, dhcp) {
            (Some(a), _) => a.ip.clone(),
            (None, Some(d)) => d.ip.clone(),
            (None, None) => continue,
        };

        let hostname = dhcp.and_then(|d| d.hostname.clone());
        let is_wireless = wireless_macs.contains(&mac)
            || arp.is_some_and(|a| a.device.starts_with("wlan") || a.device.starts_with("phy"));

        devices.push(Device {
            mac,
            ip,
            hostname,
            connection_type: if is_wireless { "Wireless" } else { "Wired" }.to_owned(),
        });
    }

    devices.sort_by(|a, b| {
        let ip_a = a.ip.parse::<Ipv4Addr>().map_or((255, 255, 255, 255), |ip| {
            let o = ip.octets();
            (o[0], o[1], o[2], o[3])
        });
        let ip_b = b.ip.parse::<Ipv4Addr>().map_or((255, 255, 255, 255), |ip| {
            let o = ip.octets();
            (o[0], o[1], o[2], o[3])
        });
        ip_a.cmp(&ip_b).then_with(|| a.mac.cmp(&b.mac))
    });

    devices
}

pub fn read_devices() -> Result<Vec<Device>, LegacyAppError> {
    let dhcp_content = fs::read_to_string("/tmp/dhcp.leases").unwrap_or_default();
    let arp_content = fs::read_to_string("/proc/net/arp").unwrap_or_default();
    let dhcp_leases = parse_dhcp_leases(&dhcp_content);
    let arp_entries = parse_arp_table(&arp_content);
    let wireless_macs = get_wireless_macs();
    Ok(merge_devices(dhcp_leases, arp_entries, wireless_macs))
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
        let mut wireless = HashSet::new();
        wireless.insert("00:11:22:33:44:55".into());

        let devices = merge_devices(dhcp, arp, wireless);
        assert_eq!(devices.len(), 3);
        assert_eq!(devices[0].mac, "00:11:22:33:44:55");
        assert_eq!(devices[0].connection_type, "Wireless");
        assert_eq!(devices[1].mac, "aa:bb:cc:dd:ee:ff");
        assert_eq!(devices[1].connection_type, "Wired");
        assert_eq!(devices[2].mac, "11:22:33:44:55:66");
        assert_eq!(devices[2].hostname, None);
    }
}
