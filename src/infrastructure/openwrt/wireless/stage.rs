use crate::domain::{
    errors::{ErrorCode, ErrorStage, LegacyAppError},
    wifi::WifiNetworkConfig,
};
use crate::infrastructure::openwrt::rpc::call_ubus;
use serde_json::{Value, json};

pub fn stage_wifi_config(
    session: &str,
    targets: &[String],
    config: &WifiNetworkConfig,
    is_extender: bool,
) -> Result<(), LegacyAppError> {
    for target in targets {
        let mut values = serde_json::Map::new();
        values.insert("ssid".into(), json!(config.ssid));
        values.insert("encryption".into(), json!(config.encryption));

        // Inject 802.11r/k/v options
        values.insert("ieee80211k".into(), json!("1"));
        values.insert("ieee80211v".into(), json!("1"));
        values.insert("bss_transition".into(), json!("1"));
        values.insert("wnm_sleep_mode".into(), json!("1"));
        values.insert("ieee80211r".into(), json!("1"));
        values.insert("ft_over_ds".into(), json!("1"));
        values.insert("ft_psk_generate_local".into(), json!("1"));

        let md = format!(
            "{:04x}",
            config
                .ssid
                .bytes()
                .fold(0u16, |acc, b| acc.wrapping_add(b as u16))
        );
        values.insert("mobility_domain".into(), json!(md));

        if config.encryption != "none"
            && let Some(key) = &config.key
        {
            values.insert("key".into(), json!(key));
        }

        call_ubus(
            "uci",
            "set",
            json!({
                "config": "wireless",
                "section": target,
                "values": values,
                "ubus_rpc_session": session
            }),
        )
        .map(|_| ())
        .map_err(|error| {
            LegacyAppError::new(ErrorCode::UciStageFailed, ErrorStage::Stage, error.message)
                .retryable(error.retryable)
        })?;

        if config.encryption == "none" || config.key.is_none() {
            let _ = call_ubus(
                "uci",
                "delete",
                json!({
                    "config": "wireless",
                    "section": target,
                    "option": "key",
                    "ubus_rpc_session": session
                }),
            );
        }
    }

    if is_extender && !targets.is_empty() {
        let mut backhaul_values = serde_json::Map::new();
        backhaul_values.insert("device".into(), json!(targets[0]));
        backhaul_values.insert("mode".into(), json!("sta"));
        backhaul_values.insert("network".into(), json!("lan"));
        backhaul_values.insert("wds".into(), json!("1"));
        backhaul_values.insert("ssid".into(), json!(config.ssid));
        backhaul_values.insert("encryption".into(), json!(config.encryption));
        if config.encryption != "none" {
            if let Some(key) = &config.key {
                backhaul_values.insert("key".into(), json!(key));
            }
        }

        let update_res = call_ubus(
            "uci",
            "set",
            json!({
                "config": "wireless",
                "section": "mesh_backhaul",
                "values": backhaul_values.clone(),
                "ubus_rpc_session": session
            }),
        );

        if update_res.is_err() {
            call_ubus(
                "uci",
                "set",
                json!({
                    "config": "wireless",
                    "section": "mesh_backhaul",
                    "type": "wifi-iface",
                    "values": backhaul_values,
                    "ubus_rpc_session": session
                }),
            )
            .map(|_| ())
            .map_err(|error| {
                LegacyAppError::new(ErrorCode::UciStageFailed, ErrorStage::Stage, error.message)
                    .retryable(error.retryable)
            })?;
        }

        crate::infrastructure::openwrt::network::stage_stp(session)?;
    }

    Ok(())
}

pub fn check_runtime_healthy(targets: &[String], ssid: &str) -> Result<bool, LegacyAppError> {
    let status = call_ubus("network.wireless", "status", json!({}))?;
    let Some(radios) = status.as_object() else {
        return Ok(false);
    };

    for target in targets {
        let matched = radios
            .values()
            .filter_map(|r| r.get("interfaces").and_then(Value::as_array))
            .flatten()
            .any(|iface| runtime_target_matches(iface, target, ssid));

        if !matched {
            return Ok(false);
        }
    }

    Ok(true)
}

pub fn reload_wireless() -> Result<(), LegacyAppError> {
    call_ubus("network.wireless", "down", json!({}))
        .and_then(|_| call_ubus("network.wireless", "up", json!({})))
        .map(|_| ())
        .map_err(|error| {
            LegacyAppError::new(
                ErrorCode::ReconcileFailed,
                ErrorStage::Reconcile,
                format!("wireless runtime reload failed: {}", error.message),
            )
            .retryable(true)
        })
}

fn runtime_target_matches(value: &Value, target: &str, ssid: &str) -> bool {
    let Some(map) = value.as_object() else {
        return false;
    };
    if map.get("section").and_then(Value::as_str) != Some(target) {
        return false;
    }
    let configured_ssid = map
        .get("config")
        .and_then(Value::as_object)
        .and_then(|c| c.get("ssid"))
        .and_then(Value::as_str);
    let data_ssid = map
        .get("data")
        .and_then(Value::as_object)
        .and_then(|d| d.get("ssid"))
        .and_then(Value::as_str);
    let is_up = map.get("up").is_some_and(is_truthy);

    is_up && (configured_ssid == Some(ssid) || data_ssid == Some(ssid))
}

pub(crate) fn is_truthy(value: &Value) -> bool {
    match value {
        Value::Bool(value) => *value,
        Value::Number(value) => value.as_i64().is_some_and(|value| value != 0),
        Value::String(value) => {
            let normalized = value.trim().to_ascii_lowercase();
            matches!(normalized.as_str(), "1" | "true" | "on" | "yes" | "enabled")
        }
        _ => false,
    }
}
