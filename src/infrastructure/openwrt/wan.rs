use serde_json::{Map, Value, json};

use crate::domain::{
    DiscoveredWan, WanDesired, WanPppoeConfig, WanProtocol, WanPublicState, WanStaticConfig,
    WanStatus,
};

pub fn parse_discovered_wan(value: &Value) -> DiscoveredWan {
    let Some(values) = value.get("values").and_then(Value::as_object) else {
        return DiscoveredWan {
            present: false,
            proto: WanProtocol::None,
            ..DiscoveredWan::default()
        };
    };

    let proto_str = values.get("proto").and_then(Value::as_str).unwrap_or("");
    let proto = match proto_str {
        "dhcp" => WanProtocol::Dhcp,
        "static" => WanProtocol::Static,
        "pppoe" => WanProtocol::Pppoe,
        "extender" => WanProtocol::Extender,
        _ if proto_str.is_empty() => WanProtocol::None,
        _ => WanProtocol::Dhcp,
    };

    let present = proto != WanProtocol::None;
    let device = values
        .get("device")
        .or_else(|| values.get("ifname"))
        .and_then(Value::as_str)
        .map(str::to_owned);

    let custom_mac = values
        .get("macaddr")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let custom_mtu = values
        .get("mtu")
        .and_then(Value::as_u64)
        .and_then(|v| u16::try_from(v).ok());
    let custom_dns = parse_dns_list(values.get("dns"));

    let static_config = if proto == WanProtocol::Static {
        let ip = values
            .get("ipaddr")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        let mask = values
            .get("netmask")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        let gw = values
            .get("gateway")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        Some(WanStaticConfig {
            ip_address: ip,
            netmask: mask,
            gateway: gw,
            dns: custom_dns.clone(),
        })
    } else {
        None
    };

    let pppoe_config = if proto == WanProtocol::Pppoe {
        let user = values
            .get("username")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        let pass = values
            .get("password")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let service = values
            .get("service")
            .and_then(Value::as_str)
            .map(str::to_owned);
        Some(WanPppoeConfig {
            username: user,
            password: pass,
            service_name: service,
        })
    } else {
        None
    };

    DiscoveredWan {
        present,
        device,
        proto,
        custom_mac,
        custom_mtu,
        custom_dns,
        static_config,
        pppoe_config,
    }
}

pub fn build_wan_staging_values(config: &WanDesired) -> Value {
    let mut map = Map::new();

    if !config.present {
        map.insert("proto".into(), Value::String("none".into()));
        return Value::Object(map);
    }

    if let Some(dev) = &config.device {
        map.insert("device".into(), Value::String(dev.clone()));
    }

    match config.proto {
        WanProtocol::Dhcp => {
            map.insert("proto".into(), Value::String("dhcp".into()));
            if !config.custom_dns.is_empty() {
                map.insert("peerdns".into(), Value::String("0".into()));
                map.insert(
                    "dns".into(),
                    Value::Array(
                        config
                            .custom_dns
                            .iter()
                            .cloned()
                            .map(Value::String)
                            .collect(),
                    ),
                );
            }
        }
        WanProtocol::Static => {
            map.insert("proto".into(), Value::String("static".into()));
            if let Some(s) = &config.static_config {
                map.insert("ipaddr".into(), Value::String(s.ip_address.clone()));
                map.insert("netmask".into(), Value::String(s.netmask.clone()));
                map.insert("gateway".into(), Value::String(s.gateway.clone()));
                if !s.dns.is_empty() {
                    map.insert(
                        "dns".into(),
                        Value::Array(s.dns.iter().cloned().map(Value::String).collect()),
                    );
                }
            }
        }
        WanProtocol::Pppoe => {
            map.insert("proto".into(), Value::String("pppoe".into()));
            if let Some(p) = &config.pppoe_config {
                map.insert("username".into(), Value::String(p.username.clone()));
                if let Some(pass) = &p.password {
                    map.insert("password".into(), Value::String(pass.clone()));
                }
                if let Some(svc) = &p.service_name {
                    map.insert("service".into(), Value::String(svc.clone()));
                }
            }
        }
        WanProtocol::Extender => {
            map.insert("proto".into(), Value::String("dhcp".into()));
        }
        WanProtocol::None => {
            map.insert("proto".into(), Value::String("none".into()));
        }
    }

    if let Some(mac) = &config.custom_mac {
        map.insert("macaddr".into(), Value::String(mac.clone()));
    }
    if let Some(mtu) = config.custom_mtu {
        map.insert("mtu".into(), json!(mtu));
    }

    Value::Object(map)
}

pub fn parse_wan_runtime_status(value: &Value) -> WanPublicState {
    let up = value.get("up").and_then(Value::as_bool).unwrap_or(false);
    let pending = value
        .get("pending")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let uptime = value.get("uptime").and_then(Value::as_u64).unwrap_or(0);
    let device = value
        .get("l3_device")
        .or_else(|| value.get("device"))
        .and_then(Value::as_str)
        .map(str::to_owned);

    let proto_str = value.get("proto").and_then(Value::as_str).unwrap_or("");
    let proto = match proto_str {
        "dhcp" => WanProtocol::Dhcp,
        "static" => WanProtocol::Static,
        "pppoe" => WanProtocol::Pppoe,
        "extender" => WanProtocol::Extender,
        _ if proto_str.is_empty() => WanProtocol::None,
        _ => WanProtocol::Dhcp,
    };

    let status = if up {
        WanStatus::Connected
    } else if pending {
        WanStatus::Connecting
    } else if proto == WanProtocol::None {
        WanStatus::NotConfigured
    } else {
        WanStatus::Disconnected
    };

    let (ip_address, netmask) = parse_primary_ipv4(value.get("ipv4-address"));
    let gateway = parse_default_gateway(value.get("route"));
    let dns = parse_dns_list(value.get("dns-server"));

    WanPublicState {
        present: proto != WanProtocol::None,
        proto,
        status,
        device,
        ip_address,
        netmask,
        gateway,
        dns,
        mac_address: None,
        uptime_secs: uptime,
        error_reason: None,
    }
}

fn parse_primary_ipv4(value: Option<&Value>) -> (Option<String>, Option<String>) {
    let Some(Value::Array(arr)) = value else {
        return (None, None);
    };
    let Some(first) = arr.first().and_then(Value::as_object) else {
        return (None, None);
    };

    let ip = first
        .get("address")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let mask = first.get("mask").and_then(Value::as_u64).map(|prefix| {
        let raw = if prefix == 0 {
            0u32
        } else {
            !((1u32 << (32 - prefix)) - 1)
        };
        format!(
            "{}.{}.{}.{}",
            (raw >> 24) & 0xff,
            (raw >> 16) & 0xff,
            (raw >> 8) & 0xff,
            raw & 0xff
        )
    });

    (ip, mask)
}

fn parse_default_gateway(value: Option<&Value>) -> Option<String> {
    let Value::Array(routes) = value? else {
        return None;
    };
    for route in routes {
        let target = route.get("target").and_then(Value::as_str).unwrap_or("");
        let mask = route.get("mask").and_then(Value::as_u64).unwrap_or(32);
        if target == "0.0.0.0" || mask == 0 {
            return route
                .get("nexthop")
                .and_then(Value::as_str)
                .map(str::to_owned);
        }
    }
    None
}

fn parse_dns_list(value: Option<&Value>) -> Vec<String> {
    match value {
        Some(Value::Array(arr)) => arr
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect(),
        Some(Value::String(s)) => s.split_whitespace().map(str::to_owned).collect(),
        _ => Vec::new(),
    }
}
