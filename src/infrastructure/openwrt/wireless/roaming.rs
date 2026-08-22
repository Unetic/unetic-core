use std::collections::BTreeMap;

use serde_json::Value;

use crate::{
    domain::{
        AccessPointRoamingPolicy, AppliedRoamingConfig, UsteerPolicy,
        errors::{ErrorCode, ErrorStage, LegacyAppError},
    },
    infrastructure::openwrt::rpc::uci_get_config,
};

pub fn read_roaming_config(
    targets: &[String],
    session: Option<&str>,
) -> Result<AppliedRoamingConfig, LegacyAppError> {
    verify_single_owned_section(session)?;
    let response = uci_get_config("usteer", Some("unetic"), None, session)?;
    let values = values_table(&response, "usteer.unetic")?;
    let ssid_list = string_list(values, "ssid_list")?;
    let policy = parse_usteer_policy(values)?;
    let access_points = targets
        .iter()
        .map(|target| {
            let response = uci_get_config("wireless", Some(target), None, session)?;
            let values = values_table(&response, target)?;
            Ok((target.clone(), parse_ap_policy(values)))
        })
        .collect::<Result<BTreeMap<_, _>, LegacyAppError>>()?;

    Ok(AppliedRoamingConfig {
        enabled: boolean(values.get("enabled")),
        network: values
            .get("network")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        local_mode: boolean(values.get("local_mode")),
        ssid_list,
        policy,
        access_points,
    })
}

fn verify_single_owned_section(session: Option<&str>) -> Result<(), LegacyAppError> {
    let response = uci_get_config("usteer", None, None, session)?;
    let sections = values_table(&response, "usteer")?;
    let names: Vec<&str> = sections
        .iter()
        .filter(|(_, values)| values.get(".type").and_then(Value::as_str) == Some("usteer"))
        .map(|(name, _)| name.as_str())
        .collect();
    if names == ["unetic"] {
        return Ok(());
    }

    Err(LegacyAppError::new(
        ErrorCode::UciReadFailed,
        ErrorStage::Verify,
        "usteer must contain exactly one Unetic-owned section",
    ))
}

fn parse_usteer_policy(
    values: &serde_json::Map<String, Value>,
) -> Result<UsteerPolicy, LegacyAppError> {
    Ok(UsteerPolicy {
        aggressiveness: integer(values, "aggressiveness")? as u8,
        roam_scan_snr: integer(values, "roam_scan_snr")?,
        roam_trigger_snr: integer(values, "roam_trigger_snr")?,
        signal_diff_threshold: integer(values, "signal_diff_threshold")?,
        roam_scan_tries: integer(values, "roam_scan_tries")? as u8,
        roam_scan_interval: integer(values, "roam_scan_interval")? as u32,
        roam_scan_timeout: integer(values, "roam_scan_timeout")? as u32,
        roam_trigger_interval: integer(values, "roam_trigger_interval")? as u32,
        roam_kick_delay: integer(values, "roam_kick_delay")? as u32,
        steer_reject_timeout: integer(values, "steer_reject_timeout")? as u32,
        max_neighbor_reports: integer(values, "max_neighbor_reports")? as u8,
        assoc_steering: boolean(values.get("assoc_steering")),
        probe_steering: boolean(values.get("probe_steering")),
        load_kick_enabled: boolean(values.get("load_kick_enabled")),
        min_connect_snr: integer(values, "min_connect_snr")?,
        min_snr: integer(values, "min_snr")?,
        band_steering_interval: integer(values, "band_steering_interval")? as u32,
        band_steering_min_snr: integer(values, "band_steering_min_snr")?,
        band_steering_signal_threshold: integer(values, "band_steering_signal_threshold")?,
    })
}

fn parse_ap_policy(values: &serde_json::Map<String, Value>) -> AccessPointRoamingPolicy {
    let ieee80211r = boolean(values.get("ieee80211r"));
    AccessPointRoamingPolicy {
        ieee80211k: boolean(values.get("ieee80211k")),
        rrm_neighbor_report: boolean(values.get("rrm_neighbor_report")),
        rrm_beacon_report: boolean(values.get("rrm_beacon_report")),
        bss_transition: boolean(values.get("bss_transition")),
        ieee80211r,
        ft_over_ds: boolean(values.get("ft_over_ds")),
        ft_psk_generate_local: boolean(values.get("ft_psk_generate_local")),
        mobility_domain: ieee80211r
            .then(|| {
                values
                    .get("mobility_domain")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
            .flatten(),
    }
}

fn values_table<'a>(
    response: &'a Value,
    name: &str,
) -> Result<&'a serde_json::Map<String, Value>, LegacyAppError> {
    response
        .get("values")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            LegacyAppError::new(
                ErrorCode::UciReadFailed,
                ErrorStage::Verify,
                format!("{name} has no UCI values table"),
            )
        })
}

fn integer(values: &serde_json::Map<String, Value>, name: &str) -> Result<i32, LegacyAppError> {
    values
        .get(name)
        .and_then(|value| value.as_i64().or_else(|| value.as_str()?.parse().ok()))
        .and_then(|value| value.try_into().ok())
        .ok_or_else(|| {
            LegacyAppError::new(
                ErrorCode::UciReadFailed,
                ErrorStage::Verify,
                format!("usteer.unetic has no valid {name} option"),
            )
        })
}

fn string_list(
    values: &serde_json::Map<String, Value>,
    name: &str,
) -> Result<Vec<String>, LegacyAppError> {
    match values.get(name) {
        Some(Value::Array(values)) => Ok(values
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect()),
        Some(Value::String(value)) => Ok(vec![value.clone()]),
        _ => Err(LegacyAppError::new(
            ErrorCode::UciReadFailed,
            ErrorStage::Verify,
            format!("usteer.unetic has no valid {name} option"),
        )),
    }
}

fn boolean(value: Option<&Value>) -> bool {
    value.is_some_and(super::is_truthy)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_boolean_uci_forms() {
        assert!(boolean(Some(&Value::String("1".into()))));
        assert!(boolean(Some(&Value::Bool(true))));
        assert!(!boolean(Some(&Value::String("0".into()))));
        assert!(!boolean(None));
    }

    #[test]
    fn parses_string_and_array_lists() {
        let mut values = serde_json::Map::new();
        values.insert("ssid_list".into(), serde_json::json!(["Home"]));
        assert_eq!(string_list(&values, "ssid_list").unwrap(), ["Home"]);
        values.insert("ssid_list".into(), Value::String("Home".into()));
        assert_eq!(string_list(&values, "ssid_list").unwrap(), ["Home"]);
    }
}
