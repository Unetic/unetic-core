use crate::{
    device::Device,
    switch::{SwitchArchitecture, SwitchFeatureStatus, SwitchFeatures, SwitchInfo, SwitchSocInfo},
    system::SystemInfo,
};

pub(crate) fn mock_switch_info() -> SwitchInfo {
    SwitchInfo {
        soc: SwitchSocInfo {
            model: "mt7531".into(),
            vendor: "MediaTek".into(),
            compatible: Some("mediatek,mt7531".into()),
            driver: Some("mt7530-mdio".into()),
            architecture: SwitchArchitecture::Dsa,
            tagging_protocol: Some("mtk".into()),
            ports: vec![
                "lan1".into(),
                "lan2".into(),
                "lan3".into(),
                "lan4".into(),
                "wan".into(),
            ],
        },
        features: SwitchFeatures {
            l2_hw_switching: SwitchFeatureStatus::static_hw(true),
            l3_hw_flow_offload: SwitchFeatureStatus::new(true, true, true),
            l3_sw_flow_offload: SwitchFeatureStatus::new(true, true, true),
            vlan_filtering_8021q: SwitchFeatureStatus::new(true, false, true),
            port_isolation: SwitchFeatureStatus::new(true, false, true),
            hw_igmp_snooping: SwitchFeatureStatus::new(true, true, true),
            flow_control_8023x: SwitchFeatureStatus::new(true, false, true),
            eee_8023az: SwitchFeatureStatus::new(true, false, true),
            stp_rstp: SwitchFeatureStatus::new(true, false, true),
            mirroring_span: SwitchFeatureStatus::new(true, false, true),
            jumbo_frames: SwitchFeatureStatus::new(true, false, true),
            link_aggregation_lag: SwitchFeatureStatus::new(true, false, true),
            tdr_cable_diag: SwitchFeatureStatus::static_hw(true),
            hardware_stats: SwitchFeatureStatus::static_hw(true),
        },
    }
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
            ip: "192.168.1.100".into(),
            hostname: Some("Alice-Phone".into()),
            connection_type: "Wireless".into(),
        },
        Device {
            mac: "66:77:88:99:aa:bb".into(),
            ip: "192.168.1.101".into(),
            hostname: Some("Desktop-PC".into()),
            connection_type: "Wired".into(),
        },
        Device {
            mac: "cc:dd:ee:ff:00:11".into(),
            ip: "192.168.1.102".into(),
            hostname: None,
            connection_type: "Wireless".into(),
        },
    ]
}
