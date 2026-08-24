use std::{
    collections::{HashMap, HashSet},
    net::Ipv4Addr,
};

use crate::domain::{
    device::{Device, DeviceConnection},
    extender::{ExtenderClient, KnownExtender},
};

use super::WirelessClient;

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
    wireless_clients: HashMap<String, WirelessClient>,
    mac_to_iface: HashMap<String, String>,
    ip6_by_mac: HashMap<String, String>,
    extenders: &[KnownExtender],
    extender_clients: &HashMap<String, Vec<ExtenderClient>>,
) -> Vec<Device> {
    let mut all_macs: HashSet<String> = HashSet::new();
    all_macs.extend(arp_entries.keys().cloned());
    all_macs.extend(ip6_by_mac.keys().cloned());
    all_macs.extend(mac_to_iface.keys().cloned());
    all_macs.extend(wireless_clients.keys().cloned());
    all_macs.extend(
        extender_clients
            .values()
            .flatten()
            .map(|client| client.mac.to_ascii_lowercase()),
    );

    let mut devices: Vec<Device> = all_macs
        .into_iter()
        .map(|mac| {
            let arp = arp_entries.get(&mac);
            let dhcp = dhcp_leases.get(&mac);
            let ip = arp
                .map(|entry| entry.ip.clone())
                .or_else(|| dhcp.map(|lease| lease.ip.clone()));
            let ip6 = ip6_by_mac.get(&mac).cloned();
            let hostname = dhcp.and_then(|lease| lease.hostname.clone());
            let connection = device_connection(
                &mac,
                &wireless_clients,
                &mac_to_iface,
                extenders,
                extender_clients,
            );
            Device {
                mac,
                ip,
                ip6,
                hostname,
                connection,
            }
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
    wireless_clients: &HashMap<String, WirelessClient>,
    mac_to_iface: &HashMap<String, String>,
    extenders: &[KnownExtender],
    extender_clients: &HashMap<String, Vec<ExtenderClient>>,
) -> DeviceConnection {
    let is_extender = extenders
        .iter()
        .any(|extender| extender.mac.eq_ignore_ascii_case(mac));

    if !is_extender
        && let Some((extender_mac, client)) = extender_clients.iter().find_map(|(extender_mac, clients)| {
            clients
                .iter()
                .find(|client| client.mac.eq_ignore_ascii_case(mac))
                .map(|client| (extender_mac, client))
        })
    {
        return DeviceConnection::ViaExtender {
            extender_mac: extender_mac.clone(),
            signal_dbm: client.signal_dbm,
            interface: client.interface.clone(),
            network: client.network.clone(),
            port_id: client.port_id.clone(),
        };
    }

    if let Some(client) = wireless_clients.get(mac) {
        return DeviceConnection::Wireless {
            signal_dbm: client.signal_dbm,
            interface: client.interface.clone(),
            network: client.network.clone(),
        };
    }

    if let Some(iface) = mac_to_iface.get(mac) {
        return DeviceConnection::Wired {
            port_id: iface.clone(),
        };
    }

    DeviceConnection::Unknown
}

fn ipv4_sort_key(ip: Option<&str>) -> [u8; 4] {
    ip.and_then(|value| value.parse::<Ipv4Addr>().ok())
        .map_or([u8::MAX; 4], |address| address.octets())
}
