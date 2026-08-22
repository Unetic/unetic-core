use serde_json::{Value, json};

use crate::{
    domain::{
        AccessPointRoamingPolicy,
        errors::{ErrorCode, LegacyAppError},
    },
    infrastructure::openwrt::rpc::call_ubus,
};

pub fn build_values(policy: &AccessPointRoamingPolicy) -> serde_json::Map<String, Value> {
    let mut values = serde_json::Map::new();
    values.insert("ieee80211k".into(), json!("1"));
    values.insert("rrm_neighbor_report".into(), json!("1"));
    values.insert("rrm_beacon_report".into(), json!("1"));
    values.insert("bss_transition".into(), json!("1"));
    values.insert(
        "ieee80211r".into(),
        json!(if policy.ieee80211r { "1" } else { "0" }),
    );

    if policy.ieee80211r {
        values.insert("ft_over_ds".into(), json!("0"));
        values.insert(
            "ft_psk_generate_local".into(),
            json!(if policy.ft_psk_generate_local {
                "1"
            } else {
                "0"
            }),
        );
        values.insert(
            "mobility_domain".into(),
            json!(policy.mobility_domain.as_deref().unwrap_or("4f57")),
        );
    }

    values
}

const CONFLICTING_OPTIONS: [&str; 5] = [
    "ieee80211v",
    "wnm_sleep_mode",
    "ft_over_ds",
    "ft_psk_generate_local",
    "mobility_domain",
];

pub fn delete_conflicting_options(session: &str, target: &str) -> Result<(), LegacyAppError> {
    for option in CONFLICTING_OPTIONS {
        match call_ubus(
            "uci",
            "delete",
            json!({
                "config": "wireless",
                "section": target,
                "option": option,
                "ubus_rpc_session": session
            }),
        ) {
            Ok(_) => {}
            Err(error) if error.code == ErrorCode::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_open_network_without_fast_transition() {
        let values = build_values(&crate::domain::compile_ap_policy("none"));

        assert_eq!(values["ieee80211k"], "1");
        assert_eq!(values["rrm_neighbor_report"], "1");
        assert_eq!(values["rrm_beacon_report"], "1");
        assert_eq!(values["bss_transition"], "1");
        assert_eq!(values["ieee80211r"], "0");
        assert!(!values.contains_key("mobility_domain"));
    }

    #[test]
    fn builds_fast_transition_for_wpa2_sae_and_mixed() {
        let wpa2 = build_values(&crate::domain::compile_ap_policy("psk2"));
        assert_eq!(wpa2["ieee80211r"], "1");
        assert_eq!(wpa2["ft_over_ds"], "0");
        assert_eq!(wpa2["ft_psk_generate_local"], "1");
        assert_eq!(wpa2["mobility_domain"], "4f57");

        for encryption in ["sae", "sae-mixed"] {
            let values = build_values(&crate::domain::compile_ap_policy(encryption));
            assert_eq!(values["ieee80211r"], "1");
            assert_eq!(values["ft_over_ds"], "0");
            assert_eq!(values["ft_psk_generate_local"], "0");
        }
    }

    #[test]
    fn identifies_legacy_and_security_conflicting_options() {
        assert_eq!(
            CONFLICTING_OPTIONS,
            [
                "ieee80211v",
                "wnm_sleep_mode",
                "ft_over_ds",
                "ft_psk_generate_local",
                "mobility_domain",
            ]
        );
    }
}
