use crate::{
    domain::device::Device,

    domain::system::SystemInfo,
};

pub(crate) fn mock_ports_info() -> Vec<crate::domain::ports::PhysicalPort> {
    use crate::domain::ports::{PhysicalPort, PortConnection, PortSpeed, PortType};
    vec![
        PhysicalPort {
            id: "wan".to_string(),
            name: "wan".to_string(),
            port_type: PortType::Wan,
            speed: PortSpeed::Speed1000,
            connections: vec![],
        },
        PhysicalPort {
            id: "lan1".to_string(),
            name: "lan1".to_string(),
            port_type: PortType::Lan,
            speed: PortSpeed::Speed1000,
            connections: vec![],
        },
        PhysicalPort {
            id: "lan2".to_string(),
            name: "lan2".to_string(),
            port_type: PortType::Lan,
            speed: PortSpeed::Speed1000,
            connections: vec![
                PortConnection {
                    mac: "66:77:88:99:aa:bb".to_string(),
                    ip: Some("192.168.1.101".to_string()),
                    hostname: Some("Desktop-PC".to_string()),
                },
                PortConnection {
                    mac: "11:22:33:44:55:66".to_string(),
                    ip: Some("192.168.1.103".to_string()),
                    hostname: Some("Switch-Downstream".to_string()),
                }
            ],
        },
        PhysicalPort {
            id: "lan3".to_string(),
            name: "lan3".to_string(),
            port_type: PortType::Lan,
            speed: PortSpeed::NoLink,
            connections: vec![],
        },
        PhysicalPort {
            id: "lan4".to_string(),
            name: "lan4".to_string(),
            port_type: PortType::Lan,
            speed: PortSpeed::NoLink,
            connections: vec![],
        },
    ]
}

pub(crate) fn mock_system_info() -> SystemInfo {
    SystemInfo {
        hostname: "OpenWrt".into(),
        model: "MediaTek MT7981B (Filogic 820)".into(),
        board_name: "bananapi,bpi-r3-mini".into(),
        firmware_version: "25.12.5".into(),
        firmware_revision: "r12345-abcdef".into(),
        target: "mediatek/filogic".into(),
        arch: "aarch64_cortex-a53".into(),
        kernel_version: "6.6.86".into(),
        uptime_secs: 86400,
        load_average: [0.12, 0.08, 0.05],
        memory_total_kb: 524288,
        memory_available_kb: 412672,
    }
}

pub(crate) fn mock_devices() -> Vec<Device> {
    vec![
        Device {
            mac: "00:11:22:33:44:55".into(),
            ip: Some("192.168.1.100".into()),
            ip6: Some("2001:db8::1".into()),
            hostname: Some("Alice-Phone".into()),
            connection: crate::domain::device::DeviceConnection::Wireless { signal_pct: 82 },
        },
        Device {
            mac: "66:77:88:99:aa:bb".into(),
            ip: Some("192.168.1.101".into()),
            ip6: None,
            hostname: Some("Desktop-PC".into()),
            connection: crate::domain::device::DeviceConnection::Wired { port_id: 1 },
        },
        Device {
            mac: "cc:dd:ee:ff:00:11".into(),
            ip: Some("192.168.1.102".into()),
            ip6: None,
            hostname: None,
            connection: crate::domain::device::DeviceConnection::Wireless { signal_pct: 50 },
        },
    ]
}

pub(crate) fn mock_dns_config() -> crate::domain::DnsConfig {
    crate::domain::DnsConfig {
        upstream: vec!["1.1.1.1".to_string(), "1.0.0.1".to_string()],
        local_domain: Some("home.local".to_string()),
        dhcp_start: 100,
        dhcp_limit: 150,
        dhcp_lease_hours: 12,
        custom_records: vec![
            crate::domain::DnsRecord {
                id: "nas".to_string(),
                hostname: "nas.home.local".to_string(),
                ip: "192.168.1.10".to_string(),
            }
        ],
    }
}
