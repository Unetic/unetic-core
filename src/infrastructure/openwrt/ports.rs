use std::collections::HashMap;
use std::fs;
use std::process::Command;

use crate::domain::device::Device;
use crate::domain::ports::{PhysicalPort, PortConnection, PortSpeed, PortType};

fn parse_speed(speed_str: &str) -> PortSpeed {
    match speed_str.trim() {
        "10" => PortSpeed::Speed10,
        "100" => PortSpeed::Speed100,
        "1000" => PortSpeed::Speed1000,
        "2500" => PortSpeed::Speed2500,
        "5000" => PortSpeed::Speed5000,
        "10000" => PortSpeed::Speed10000,
        _ => PortSpeed::NoLink,
    }
}

pub(crate) fn ports_list(devices: &[Device]) -> Vec<PhysicalPort> {
    let mut ports = Vec::new();

    let mut mac_to_iface: HashMap<String, String> = HashMap::new();
    if let Ok(output) = Command::new("bridge").args(["fdb", "show"]).output() {
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

    let mut iface_to_connections: HashMap<String, Vec<PortConnection>> = HashMap::new();
    for device in devices {
        let mac = device.mac.to_lowercase();
        if let Some(iface) = mac_to_iface.get(&mac) {
            let conn = PortConnection {
                mac,
                ip: device.ip.clone(),
                hostname: device.hostname.clone(),
            };
            iface_to_connections
                .entry(iface.clone())
                .or_default()
                .push(conn);
        }
    }

    let mut lan_ifaces = Vec::new();
    if let Ok(entries) = fs::read_dir("/sys/class/net/br-lan/brif") {
        for entry in entries.flatten() {
            if let Ok(file_name) = entry.file_name().into_string() {
                if !file_name.starts_with("wlan") {
                    lan_ifaces.push(file_name);
                }
            }
        }
    }

    for iface in lan_ifaces {
        let mut speed = PortSpeed::NoLink;
        if let Ok(operstate) = fs::read_to_string(format!("/sys/class/net/{}/operstate", iface)) {
            if operstate.trim() == "up" {
                if let Ok(speed_str) = fs::read_to_string(format!("/sys/class/net/{}/speed", iface)) {
                    speed = parse_speed(&speed_str);
                }
            }
        }
        let connections = iface_to_connections.remove(&iface).unwrap_or_default();
        ports.push(PhysicalPort {
            id: iface.clone(),
            name: iface.clone(),
            port_type: PortType::Lan,
            speed,
            connections,
        });
    }

    let possible_wan = vec!["eth0", "wan"];
    let mut wan_iface = None;
    for w in possible_wan {
        if fs::metadata(format!("/sys/class/net/{}", w)).is_ok() {
            wan_iface = Some(w.to_string());
            break;
        }
    }

    if let Some(wan) = wan_iface {
        let mut speed = PortSpeed::NoLink;
        if let Ok(operstate) = fs::read_to_string(format!("/sys/class/net/{}/operstate", wan)) {
            if operstate.trim() == "up" {
                if let Ok(speed_str) = fs::read_to_string(format!("/sys/class/net/{}/speed", wan)) {
                    speed = parse_speed(&speed_str);
                }
            }
        }
        let connections = iface_to_connections.remove(&wan).unwrap_or_default();
        ports.push(PhysicalPort {
            id: wan.clone(),
            name: wan.clone(),
            port_type: PortType::Wan,
            speed,
            connections,
        });
    }

    ports
}
