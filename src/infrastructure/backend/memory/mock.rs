use crate::{
    domain::device::Device,
    domain::system::{SystemInfo, SystemRuntime, TemperatureReading, TemperatureSource},
};

pub(crate) fn mock_ports_info() -> Vec<crate::domain::ports::PhysicalPort> {
    use crate::domain::ports::{PhysicalPort, PortConnection, PortSpeed, PortType};
    vec![
        PhysicalPort {
            id: "wan".to_string(),
            port_type: PortType::Wan,
            speed: PortSpeed::Speed1000,
            connections: vec![],
        },
        PhysicalPort {
            id: "lan1".to_string(),
            port_type: PortType::Lan,
            speed: PortSpeed::Speed1000,
            connections: vec![],
        },
        PhysicalPort {
            id: "lan2".to_string(),
            port_type: PortType::Lan,
            speed: PortSpeed::Speed1000,
            connections: vec![
                PortConnection {
                    device_id: "device-66778899aabb".to_string(),
                },
                PortConnection {
                    device_id: "device-112233445566".to_string(),
                },
            ],
        },
        PhysicalPort {
            id: "lan3".to_string(),
            port_type: PortType::Lan,
            speed: PortSpeed::NoLink,
            connections: vec![],
        },
        PhysicalPort {
            id: "lan4".to_string(),
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
        cpu_count: 2,
    }
}

pub(crate) fn mock_system_runtime() -> SystemRuntime {
    SystemRuntime {
        uptime_secs: 86400,
        load_average: [0.12, 0.08, 0.05],
        memory_total_kb: 524288,
        memory_available_kb: 412672,
        storage_total_kb: 131072,
        storage_available_kb: 98304,
        temperatures: vec![
            TemperatureReading {
                source: TemperatureSource::Soc,
                temp_celsius: 48.3,
            },
            TemperatureReading {
                source: TemperatureSource::Wifi24,
                temp_celsius: 44.1,
            },
            TemperatureReading {
                source: TemperatureSource::Wifi5,
                temp_celsius: 46.7,
            },
        ],
    }
}

pub(crate) fn mock_devices() -> Vec<Device> {
    vec![
        Device {
            mac: "00:11:22:33:44:55".into(),
            ip: Some("192.168.1.100".into()),
            ip6: Some("2001:db8::1".into()),
            hostname: Some("Alice-Phone".into()),
            connection: crate::domain::device::DeviceConnection::Wireless {
                signal_dbm: -82,
                interface: "wlan0".into(),
                network: Some("Unetic".into()),
            },
        },
        Device {
            mac: "66:77:88:99:aa:bb".into(),
            ip: Some("192.168.1.101".into()),
            ip6: None,
            hostname: Some("Desktop-PC".into()),
            connection: crate::domain::device::DeviceConnection::Wired {
                port_id: "lan1".into(),
            },
        },
        Device {
            mac: "cc:dd:ee:ff:00:11".into(),
            ip: Some("192.168.1.102".into()),
            ip6: None,
            hostname: None,
            connection: crate::domain::device::DeviceConnection::Wireless {
                signal_dbm: -50,
                interface: "wlan1".into(),
                network: Some("Unetic".into()),
            },
        },
    ]
}
