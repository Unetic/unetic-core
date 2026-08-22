use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RoamingMode {
    #[default]
    Soft,
    Aggressive,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RoamingSensitivity {
    Low,
    #[default]
    Balanced,
    High,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct RoamingConfig {
    #[serde(default)]
    pub mode: RoamingMode,
    #[serde(default)]
    pub sensitivity: RoamingSensitivity,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UsteerPolicy {
    pub aggressiveness: u8,
    pub roam_scan_snr: i32,
    pub roam_trigger_snr: i32,
    pub signal_diff_threshold: i32,
    pub roam_scan_tries: u8,
    pub roam_scan_interval: u32,
    pub roam_scan_timeout: u32,
    pub roam_trigger_interval: u32,
    pub roam_kick_delay: u32,
    pub steer_reject_timeout: u32,
    pub max_neighbor_reports: u8,
    pub assoc_steering: bool,
    pub probe_steering: bool,
    pub load_kick_enabled: bool,
    pub min_connect_snr: i32,
    pub min_snr: i32,
    pub band_steering_interval: u32,
    pub band_steering_min_snr: i32,
    pub band_steering_signal_threshold: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AccessPointRoamingPolicy {
    pub ieee80211k: bool,
    pub rrm_neighbor_report: bool,
    pub rrm_beacon_report: bool,
    pub bss_transition: bool,
    pub ieee80211r: bool,
    pub ft_over_ds: bool,
    pub ft_psk_generate_local: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mobility_domain: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AppliedRoamingConfig {
    pub enabled: bool,
    pub network: String,
    pub local_mode: bool,
    pub ssid_list: Vec<String>,
    pub policy: UsteerPolicy,
    pub access_points: BTreeMap<String, AccessPointRoamingPolicy>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RoamingRuntimeStatus {
    Ready,
    Degraded,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RoamingRuntime {
    pub available: bool,
    pub local_bss: u32,
    pub remote_bss: u32,
    pub status: RoamingRuntimeStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl Default for RoamingRuntime {
    fn default() -> Self {
        Self {
            available: false,
            local_bss: 0,
            remote_bss: 0,
            status: RoamingRuntimeStatus::Degraded,
            error: Some("usteer runtime has not been observed".into()),
        }
    }
}

#[must_use]
pub fn compile_usteer_policy(config: RoamingConfig, suitable_bss: usize) -> UsteerPolicy {
    let (roam_snr, signal_diff_threshold) = match config.sensitivity {
        RoamingSensitivity::Low => (-78, 12),
        RoamingSensitivity::Balanced => (-72, 8),
        RoamingSensitivity::High => (-67, 5),
    };
    let (aggressiveness, roam_kick_delay) = match config.mode {
        RoamingMode::Soft => (1, 0),
        RoamingMode::Aggressive => (3, 15_000),
    };

    UsteerPolicy {
        aggressiveness,
        roam_scan_snr: roam_snr,
        roam_trigger_snr: roam_snr,
        signal_diff_threshold,
        roam_scan_tries: 3,
        roam_scan_interval: 10_000,
        roam_scan_timeout: 60_000,
        roam_trigger_interval: 60_000,
        roam_kick_delay,
        steer_reject_timeout: 120_000,
        max_neighbor_reports: 8,
        assoc_steering: false,
        probe_steering: false,
        load_kick_enabled: false,
        min_connect_snr: 0,
        min_snr: 0,
        band_steering_interval: if suitable_bss > 1 { 30_000 } else { 0 },
        band_steering_min_snr: -60,
        band_steering_signal_threshold: 5,
    }
}

#[must_use]
pub fn compile_ap_policy(encryption: &str) -> AccessPointRoamingPolicy {
    let fast_transition = encryption != "none";
    let local_psk = fast_transition && !encryption.contains("sae");

    AccessPointRoamingPolicy {
        ieee80211k: true,
        rrm_neighbor_report: true,
        rrm_beacon_report: true,
        bss_transition: true,
        ieee80211r: fast_transition,
        ft_over_ds: false,
        ft_psk_generate_local: local_psk,
        mobility_domain: fast_transition.then(|| "4f57".into()),
    }
}

#[must_use]
pub fn compile_applied_roaming(
    profile: RoamingConfig,
    ssid: &str,
    encryption: &str,
    targets: &[String],
) -> AppliedRoamingConfig {
    let access_point = compile_ap_policy(encryption);
    AppliedRoamingConfig {
        enabled: true,
        network: "lan".into(),
        local_mode: false,
        ssid_list: vec![ssid.to_owned()],
        policy: compile_usteer_policy(profile, targets.len()),
        access_points: targets
            .iter()
            .map(|target| (target.clone(), access_point.clone()))
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compiles_all_user_profiles() {
        let cases = [
            (RoamingSensitivity::Low, -78, 12),
            (RoamingSensitivity::Balanced, -72, 8),
            (RoamingSensitivity::High, -67, 5),
        ];

        for mode in [RoamingMode::Soft, RoamingMode::Aggressive] {
            for (sensitivity, snr, difference) in cases {
                let policy = compile_usteer_policy(RoamingConfig { mode, sensitivity }, 2);

                assert_eq!(policy.roam_trigger_snr, snr);
                assert_eq!(policy.signal_diff_threshold, difference);
                assert_eq!(
                    policy.aggressiveness,
                    if mode == RoamingMode::Soft { 1 } else { 3 }
                );
                assert_eq!(
                    policy.roam_kick_delay,
                    if mode == RoamingMode::Soft { 0 } else { 15_000 }
                );
                assert_eq!(policy.band_steering_interval, 30_000);
            }
        }
    }

    #[test]
    fn disables_band_steering_without_an_alternative_bss() {
        let policy = compile_usteer_policy(RoamingConfig::default(), 1);

        assert_eq!(policy.band_steering_interval, 0);
    }

    #[test]
    fn compiles_fast_transition_by_encryption() {
        let open = compile_ap_policy("none");
        assert!(!open.ieee80211r);
        assert_eq!(open.mobility_domain, None);

        let wpa2 = compile_ap_policy("psk2");
        assert!(wpa2.ieee80211r);
        assert!(!wpa2.ft_over_ds);
        assert!(wpa2.ft_psk_generate_local);
        assert_eq!(wpa2.mobility_domain.as_deref(), Some("4f57"));

        for encryption in ["sae", "sae-mixed"] {
            let policy = compile_ap_policy(encryption);
            assert!(policy.ieee80211r);
            assert!(!policy.ft_over_ds);
            assert!(!policy.ft_psk_generate_local);
        }
    }
}
