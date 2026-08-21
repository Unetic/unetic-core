use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SwitchArchitecture {
    Dsa,
    Swconfig,
    Software,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SwitchSocInfo {
    pub model: String,
    pub vendor: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compatible: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub driver: Option<String>,
    pub architecture: SwitchArchitecture,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tagging_protocol: Option<String>,
    pub ports: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SwitchFeatureStatus {
    pub supported: bool,
    pub enabled: bool,
    pub controllable: bool,
}

impl SwitchFeatureStatus {
    #[must_use]
    pub const fn new(supported: bool, enabled: bool, controllable: bool) -> Self {
        Self {
            supported,
            enabled: supported && enabled,
            controllable,
        }
    }

    #[must_use]
    pub const fn unsupported(controllable: bool) -> Self {
        Self {
            supported: false,
            enabled: false,
            controllable,
        }
    }

    #[must_use]
    pub const fn static_hw(supported: bool) -> Self {
        Self {
            supported,
            enabled: supported,
            controllable: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SwitchFeatures {
    pub l2_hw_switching: SwitchFeatureStatus,
    pub l3_hw_flow_offload: SwitchFeatureStatus,
    pub l3_sw_flow_offload: SwitchFeatureStatus,
    pub vlan_filtering_8021q: SwitchFeatureStatus,
    pub port_isolation: SwitchFeatureStatus,
    pub hw_igmp_snooping: SwitchFeatureStatus,
    pub flow_control_8023x: SwitchFeatureStatus,
    pub eee_8023az: SwitchFeatureStatus,
    pub stp_rstp: SwitchFeatureStatus,
    pub mirroring_span: SwitchFeatureStatus,
    pub jumbo_frames: SwitchFeatureStatus,
    pub link_aggregation_lag: SwitchFeatureStatus,
    pub tdr_cable_diag: SwitchFeatureStatus,
    pub hardware_stats: SwitchFeatureStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SwitchInfo {
    pub soc: SwitchSocInfo,
    pub features: SwitchFeatures,
}

impl SwitchInfo {
    #[must_use]
    pub fn generic_software() -> Self {
        Self {
            soc: SwitchSocInfo {
                model: "generic_software".into(),
                vendor: "Linux".into(),
                compatible: None,
                driver: None,
                architecture: SwitchArchitecture::Software,
                tagging_protocol: None,
                ports: Vec::new(),
            },
            features: SwitchFeatures {
                l2_hw_switching: SwitchFeatureStatus::static_hw(false),
                l3_hw_flow_offload: SwitchFeatureStatus::unsupported(true),
                l3_sw_flow_offload: SwitchFeatureStatus::new(true, false, true),
                vlan_filtering_8021q: SwitchFeatureStatus::new(true, false, true),
                port_isolation: SwitchFeatureStatus::new(true, false, true),
                hw_igmp_snooping: SwitchFeatureStatus::new(true, true, true),
                flow_control_8023x: SwitchFeatureStatus::unsupported(true),
                eee_8023az: SwitchFeatureStatus::unsupported(true),
                stp_rstp: SwitchFeatureStatus::new(true, false, true),
                mirroring_span: SwitchFeatureStatus::new(true, false, true),
                jumbo_frames: SwitchFeatureStatus::new(true, false, true),
                link_aggregation_lag: SwitchFeatureStatus::new(true, false, true),
                tdr_cable_diag: SwitchFeatureStatus::static_hw(false),
                hardware_stats: SwitchFeatureStatus::static_hw(false),
            },
        }
    }
}
