use std::{
    collections::{HashMap, HashSet},
    net::Ipv4Addr,
};

use crate::domain::{
    device::{Device, DeviceConnection},
    extender::{ExtenderClient, KnownExtender},
};

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
        if !is_valid_mac(&mac) {
            continue;
        }
        let ip = parts[2].to_string();
        let hostname = parts.get(3).and_then(|hostname| match hostname.trim() {
            "" | "*" | "-" => None,
            value => Some(value.to_owned()),
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
        if mac == "00:00:00:00:00:00" || !is_valid_mac(&mac) {
            continue;
        }

        entries.insert(
            mac,
            ArpEntry {
                ip: parts[0].into(),
            },
        );
    }
    entries
}

fn is_valid_mac(mac: &str) -> bool {
    let parts: Vec<&str> = mac.split(':').collect();
    parts.len() == 6
        && parts
            .iter()
            .all(|part| part.len() == 2 && part.chars().all(|c| c.is_ascii_hexdigit()))
}

pub fn merge_devices(
    dhcp_leases: HashMap<String, DhcpLease>,
    arp_entries: HashMap<String, ArpEntry>,
    wireless_clients: HashMap<String, i32>,
    mac_to_iface: HashMap<String, String>,
    ip6_by_mac: HashMap<String, String>,
    extenders: &[KnownExtender],
    extender_clients: &HashMap<String, Vec<ExtenderClient>>,
) -> Vec<Device> {
    let mut all_macs: HashSet<String> = HashSet::new();
    all_macs.extend(arp_entries.keys().cloned());
    all_macs.extend(dhcp_leases.keys().cloned());
    all_macs.extend(ip6_by_mac.keys().cloned());

    let iface_to_extender: HashMap<String, String> = extenders
        .iter()
        .filter_map(|extender| {
            let mac = extender.mac.to_lowercase();
            mac_to_iface.get(&mac).map(|iface| (iface.clone(), mac))
        })
        .collect();

    let mut devices: Vec<Device> = all_macs
        .into_iter()
        .filter_map(|mac| {
            let arp = arp_entries.get(&mac);
            let dhcp = dhcp_leases.get(&mac);
            let ip = arp
                .map(|entry| entry.ip.clone())
                .or_else(|| dhcp.map(|lease| lease.ip.clone()));
            let ip6 = ip6_by_mac.get(&mac).cloned();
            if ip.is_none() && ip6.is_none() {
                return None;
            }

            let hostname = dhcp.and_then(|lease| lease.hostname.clone());
            let connection = device_connection(
                &mac,
                &wireless_clients,
                &mac_to_iface,
                &iface_to_extender,
                extenders,
                extender_clients,
            );
            Some(Device {
                mac,
                ip,
                ip6,
                hostname,
                connection,
            })
        })
        .collect();

    devices.sort_by(|a, b| {
        ipv4_sort_key(a.ip.as_deref())
            .cmp(&ipv4_sort_key(b.ip.as_deref()))
            .then_with(|| a.mac.cmp(&b.mac))
    });
    devices
}

fn device_connection(
    mac: &str,
    wireless_clients: &HashMap<String, i32>,
    mac_to_iface: &HashMap<String, String>,
    iface_to_extender: &HashMap<String, String>,
    extenders: &[KnownExtender],
    extender_clients: &HashMap<String, Vec<ExtenderClient>>,
) -> DeviceConnection {
    let extender_mac = mac_to_iface
        .get(mac)
        .and_then(|iface| iface_to_extender.get(iface));
    let is_extender = extenders
        .iter()
        .any(|extender| extender.mac.eq_ignore_ascii_case(mac));

    if !is_extender && let Some(extender_mac) = extender_mac {
        let client = extender_clients
            .values()
            .flatten()
            .find(|client| client.mac.eq_ignore_ascii_case(mac));
        return DeviceConnection::ViaExtender {
            extender_mac: extender_mac.clone(),
            signal_dbm: client.map(|client| client.signal_dbm),
        };
    }

    if let Some(&signal_dbm) = wireless_clients.get(mac) {
        return DeviceConnection::Wireless { signal_dbm };
    }

    if let Some(iface) = mac_to_iface.get(mac) {
        let port_id = iface
            .chars()
            .filter(|c| c.is_ascii_digit())
            .collect::<String>()
            .parse()
            .unwrap_or(0);
        return DeviceConnection::Wired { port_id };
    }

    DeviceConnection::Unknown
}

fn ipv4_sort_key(ip: Option<&str>) -> [u8; 4] {
    ip.and_then(|value| value.parse::<Ipv4Addr>().ok())
        .map_or([u8::MAX; 4], |address| address.octets())
}
