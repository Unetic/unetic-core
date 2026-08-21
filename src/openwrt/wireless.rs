use std::collections::BTreeMap;

use serde_json::{Value, json};

use super::rpc::{call_ubus, uci_get_config};
use crate::{
    errors::{DomainError, ErrorCode, ErrorStage},
    model::DiscoveredWifi,
};

pub fn discover_primary_wifi() -> Result<DiscoveredWifi, DomainError> {
    let response = uci_get_config("wireless", None, None, None)?;
    let values = response
        .get("values")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            DomainError::new(
                ErrorCode::AmbiguousWifiConfig,
                ErrorStage::Bootstrap,
                "wireless UCI response has no values table",
            )
        })?;

    let mut candidates = Vec::new();
    for (name, section) in values {
        let Some(section) = section.as_object() else {
            continue;
        };
        if section.get(".type").and_then(Value::as_str) != Some("wifi-iface")
            || section.get("mode").and_then(Value::as_str) != Some("ap")
            || section.get("disabled").is_some_and(is_truthy)
        {
            continue;
        }

        let belongs_to_lan = section.get("network").is_some_and(|network| match network {
            Value::String(value) => value.split_ascii_whitespace().any(|part| part == "lan"),
            Value::Array(values) => values.iter().any(|value| value.as_str() == Some("lan")),
            _ => false,
        });
        if !belongs_to_lan {
            continue;
        }

        let Some(ssid) = section.get("ssid").and_then(Value::as_str) else {
            continue;
        };
        if ssid.is_empty() {
            continue;
        }
        candidates.push((name.clone(), ssid.to_owned()));
    }

    if candidates.is_empty() {
        return Err(DomainError::new(
            ErrorCode::AmbiguousWifiConfig,
            ErrorStage::Bootstrap,
            "no LAN AP wifi-iface sections found",
        ));
    }

    let first = candidates[0].1.clone();
    if candidates.iter().any(|(_, ssid)| ssid != &first) {
        return Err(DomainError::new(
            ErrorCode::AmbiguousWifiConfig,
            ErrorStage::Bootstrap,
            "LAN AP wifi-iface sections use different SSIDs",
        ));
    }

    Ok(DiscoveredWifi {
        ssid: first,
        targets: candidates.into_iter().map(|(name, _)| name).collect(),
    })
}

pub fn read_ssids(
    targets: &[String],
    session: Option<&str>,
) -> Result<BTreeMap<String, String>, DomainError> {
    let mut result = BTreeMap::new();
    for target in targets {
        let response = uci_get_config("wireless", Some(target), Some("ssid"), session)?;
        let ssid = response
            .get("value")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                DomainError::new(
                    ErrorCode::TargetMissing,
                    ErrorStage::Verify,
                    format!("target {target} has no SSID option"),
                )
            })?;
        result.insert(target.clone(), ssid.to_owned());
    }
    Ok(result)
}

pub fn stage_ssid(session: &str, targets: &[String], ssid: &str) -> Result<(), DomainError> {
    for target in targets {
        call_ubus(
            "uci",
            "set",
            json!({
                "config": "wireless",
                "section": target,
                "values": {"ssid": ssid},
                "ubus_rpc_session": session
            }),
        )
        .map(|_| ())
        .map_err(|error| {
            DomainError::new(ErrorCode::UciStageFailed, ErrorStage::Stage, error.message)
                .retryable(error.retryable)
        })?;
    }
    Ok(())
}

pub fn check_runtime_healthy(targets: &[String], ssid: &str) -> Result<bool, DomainError> {
    let status = call_ubus("network.wireless", "status", json!({}))?;
    let Some(radios) = status.as_object() else {
        return Ok(false);
    };

    for target in targets {
        let mut matched = false;
        for radio in radios.values() {
            let Some(interfaces) = radio.get("interfaces").and_then(Value::as_array) else {
                continue;
            };
            if interfaces
                .iter()
                .any(|iface| runtime_target_matches(iface, target, ssid))
            {
                matched = true;
                break;
            }
        }
        if !matched {
            return Ok(false);
        }
    }

    Ok(true)
}

pub fn reload_wireless() -> Result<(), DomainError> {
    call_ubus("network.wireless", "down", json!({}))
        .and_then(|_| call_ubus("network.wireless", "up", json!({})))
        .map(|_| ())
        .map_err(|error| {
            DomainError::new(
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

fn is_truthy(value: &Value) -> bool {
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
