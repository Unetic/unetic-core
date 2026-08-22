use serde_json::{Value, json};

use crate::{
    domain::{
        UsteerPolicy,
        errors::{ErrorCode, ErrorStage, LegacyAppError},
    },
    infrastructure::openwrt::rpc::{call_ubus, uci_get_config},
};

pub fn stage_usteer_config(
    session: &str,
    ssid: &str,
    policy: &UsteerPolicy,
) -> Result<(), LegacyAppError> {
    delete_existing_sections(session)?;
    call_ubus(
        "uci",
        "set",
        json!({
            "config": "usteer",
            "section": "unetic",
            "type": "usteer",
            "values": build_usteer_values(ssid, policy),
            "ubus_rpc_session": session
        }),
    )
    .map(|_| ())
    .map_err(stage_error)
}

pub(crate) fn build_usteer_values(
    ssid: &str,
    policy: &UsteerPolicy,
) -> serde_json::Map<String, Value> {
    let mut values = serde_json::Map::new();
    insert_section_values(&mut values, ssid, policy);
    insert_roaming_values(&mut values, policy);
    insert_steering_values(&mut values, policy);
    values
}

fn insert_section_values(
    values: &mut serde_json::Map<String, Value>,
    ssid: &str,
    policy: &UsteerPolicy,
) {
    insert(values, "enabled", 1);
    values.insert("network".into(), json!("lan"));
    insert(values, "local_mode", 0);
    values.insert("ssid_list".into(), json!([ssid]));
    insert(values, "aggressiveness", policy.aggressiveness);
}

fn insert_roaming_values(values: &mut serde_json::Map<String, Value>, policy: &UsteerPolicy) {
    insert(values, "roam_scan_snr", policy.roam_scan_snr);
    insert(values, "roam_trigger_snr", policy.roam_trigger_snr);
    insert(
        values,
        "signal_diff_threshold",
        policy.signal_diff_threshold,
    );
    insert(values, "roam_scan_tries", policy.roam_scan_tries);
    insert(values, "roam_scan_interval", policy.roam_scan_interval);
    insert(values, "roam_scan_timeout", policy.roam_scan_timeout);
    insert(
        values,
        "roam_trigger_interval",
        policy.roam_trigger_interval,
    );
    insert(values, "roam_kick_delay", policy.roam_kick_delay);
    insert(values, "steer_reject_timeout", policy.steer_reject_timeout);
    insert(values, "max_neighbor_reports", policy.max_neighbor_reports);
}

fn insert_steering_values(values: &mut serde_json::Map<String, Value>, policy: &UsteerPolicy) {
    insert(values, "assoc_steering", 0);
    insert(values, "probe_steering", 0);
    insert(values, "load_kick_enabled", 0);
    insert(values, "min_connect_snr", 0);
    insert(values, "min_snr", 0);
    insert(
        values,
        "band_steering_interval",
        policy.band_steering_interval,
    );
    insert(
        values,
        "band_steering_min_snr",
        policy.band_steering_min_snr,
    );
    insert(
        values,
        "band_steering_signal_threshold",
        policy.band_steering_signal_threshold,
    );
}

fn delete_existing_sections(session: &str) -> Result<(), LegacyAppError> {
    let response = match uci_get_config("usteer", None, None, Some(session)) {
        Ok(response) => response,
        Err(error) if error.code == ErrorCode::UciReadFailed => return Ok(()),
        Err(error) => return Err(stage_error(error)),
    };
    let Some(sections) = response.get("values").and_then(Value::as_object) else {
        return Ok(());
    };

    for (section, values) in sections {
        if values.get(".type").and_then(Value::as_str) != Some("usteer") {
            continue;
        }
        call_ubus(
            "uci",
            "delete",
            json!({
                "config": "usteer",
                "section": section,
                "ubus_rpc_session": session
            }),
        )
        .map_err(stage_error)?;
    }
    Ok(())
}

fn insert(values: &mut serde_json::Map<String, Value>, name: &str, value: impl ToString) {
    values.insert(name.into(), json!(value.to_string()));
}

fn stage_error(error: LegacyAppError) -> LegacyAppError {
    LegacyAppError::new(ErrorCode::UciStageFailed, ErrorStage::Stage, error.message)
        .retryable(error.retryable)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_safe_common_policy() {
        let policy = crate::domain::compile_usteer_policy(Default::default(), 2);
        let values = build_usteer_values("Home", &policy);

        assert_eq!(values["ssid_list"], json!(["Home"]));
        assert_eq!(values["assoc_steering"], "0");
        assert_eq!(values["probe_steering"], "0");
        assert_eq!(values["load_kick_enabled"], "0");
        assert_eq!(values["max_neighbor_reports"], "8");
    }
}
