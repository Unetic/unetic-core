use serde_json::Value;

use crate::{
    domain::{RoamingConfig, RoamingRuntime, RoamingRuntimeStatus, UsteerPolicy},
    infrastructure::openwrt::rpc::call_ubus,
};

pub fn read(targets: &[String], ssid: &str, profile: RoamingConfig) -> RoamingRuntime {
    let config = match call_ubus("usteer", "get_config", serde_json::json!({})) {
        Ok(value) => value,
        Err(error) => {
            return unavailable(format!("usteer.get_config unavailable: {}", error.message));
        }
    };
    let local = match call_ubus("usteer", "local_info", serde_json::json!({})) {
        Ok(value) => count_bss(&value, ssid),
        Err(error) => {
            return unavailable(format!("usteer.local_info unavailable: {}", error.message));
        }
    };
    let remote = match call_ubus("usteer", "remote_info", serde_json::json!({})) {
        Ok(value) => count_bss(&value, ssid),
        Err(error) => {
            return unavailable(format!("usteer.remote_info unavailable: {}", error.message));
        }
    };

    let expected = crate::domain::compile_usteer_policy(profile, targets.len());
    if !runtime_policy_matches(&config, &expected, ssid) {
        return degraded(
            local,
            remote,
            "usteer runtime policy does not match desired state",
        );
    }
    if local < targets.len() as u32 {
        return degraded(
            local,
            remote,
            format!("usteer sees {local} of {} managed local BSS", targets.len()),
        );
    }

    RoamingRuntime {
        available: true,
        local_bss: local,
        remote_bss: remote,
        status: RoamingRuntimeStatus::Ready,
        error: None,
    }
}

fn runtime_policy_matches(value: &Value, expected: &UsteerPolicy, ssid: &str) -> bool {
    let Some(values) = value.as_object() else {
        return false;
    };
    let integers = [
        ("aggressiveness", i64::from(expected.aggressiveness)),
        ("roam_scan_snr", i64::from(expected.roam_scan_snr)),
        ("roam_trigger_snr", i64::from(expected.roam_trigger_snr)),
        (
            "signal_diff_threshold",
            i64::from(expected.signal_diff_threshold),
        ),
        ("roam_scan_tries", i64::from(expected.roam_scan_tries)),
        ("roam_scan_interval", i64::from(expected.roam_scan_interval)),
        ("roam_scan_timeout", i64::from(expected.roam_scan_timeout)),
        (
            "roam_trigger_interval",
            i64::from(expected.roam_trigger_interval),
        ),
        ("roam_kick_delay", i64::from(expected.roam_kick_delay)),
        (
            "steer_reject_timeout",
            i64::from(expected.steer_reject_timeout),
        ),
        (
            "max_neighbor_reports",
            i64::from(expected.max_neighbor_reports),
        ),
        ("min_connect_snr", i64::from(expected.min_connect_snr)),
        ("min_snr", i64::from(expected.min_snr)),
        (
            "band_steering_interval",
            i64::from(expected.band_steering_interval),
        ),
        (
            "band_steering_min_snr",
            i64::from(expected.band_steering_min_snr),
        ),
    ];

    integers
        .iter()
        .all(|(name, expected)| integer(values.get(*name)) == Some(*expected))
        && !truthy(values.get("assoc_steering"))
        && !truthy(values.get("load_kick_enabled"))
        && string_list_contains(values.get("ssid_list"), ssid)
}

fn integer(value: Option<&Value>) -> Option<i64> {
    value.and_then(|value| value.as_i64().or_else(|| value.as_str()?.parse().ok()))
}

fn truthy(value: Option<&Value>) -> bool {
    value.is_some_and(super::is_truthy)
}

fn string_list_contains(value: Option<&Value>, expected: &str) -> bool {
    match value {
        Some(Value::Array(values)) => values.iter().any(|value| value.as_str() == Some(expected)),
        Some(Value::String(value)) => value == expected,
        _ => false,
    }
}

fn count_bss(value: &Value, ssid: &str) -> u32 {
    value.as_object().map_or(0, |entries| {
        entries
            .values()
            .filter(|node| node.get("ssid").and_then(Value::as_str) == Some(ssid))
            .count()
            .try_into()
            .unwrap_or(u32::MAX)
    })
}

fn unavailable(error: String) -> RoamingRuntime {
    RoamingRuntime {
        available: false,
        local_bss: 0,
        remote_bss: 0,
        status: RoamingRuntimeStatus::Degraded,
        error: Some(error),
    }
}

fn degraded(local_bss: u32, remote_bss: u32, error: impl Into<String>) -> RoamingRuntime {
    RoamingRuntime {
        available: true,
        local_bss,
        remote_bss,
        status: RoamingRuntimeStatus::Degraded,
        error: Some(error.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_only_bss_for_the_managed_ssid() {
        let value = serde_json::json!({
            "phy0-ap0": {"ssid": "Home"},
            "phy1-ap0": {"ssid": "Home"},
            "phy0-ap1": {"ssid": "Guest"}
        });

        assert_eq!(count_bss(&value, "Home"), 2);
    }
}
