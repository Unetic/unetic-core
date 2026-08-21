use std::collections::BTreeMap;

use serde_json::{Value, json};

use super::rpc::{call_ubus, uci_get_config};
use crate::{
    errors::{DomainError, ErrorCode, ErrorStage},
    model::{DiscoveredWifi, WifiNetworkConfig},
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
        let encryption = section
            .get("encryption")
            .and_then(Value::as_str)
            .unwrap_or("none")
            .to_owned();
        let key = section
            .get("key")
            .and_then(Value::as_str)
            .map(str::to_owned);
        candidates.push((name.clone(), ssid.to_owned(), encryption, key));
    }

    if candidates.is_empty() {
        return Err(DomainError::new(
            ErrorCode::AmbiguousWifiConfig,
            ErrorStage::Bootstrap,
            "no LAN AP wifi-iface sections found",
        ));
    }

    let first_ssid = candidates[0].1.clone();
    let first_enc = candidates[0].2.clone();
    let first_key = candidates[0].3.clone();
    if candidates
        .iter()
        .any(|(_, ssid, enc, key)| ssid != &first_ssid || enc != &first_enc || key != &first_key)
    {
        return Err(DomainError::new(
            ErrorCode::AmbiguousWifiConfig,
            ErrorStage::Bootstrap,
            "LAN AP wifi-iface sections use different wireless settings",
        ));
    }

    Ok(DiscoveredWifi {
        ssid: first_ssid,
        encryption: first_enc,
        key: first_key,
        targets: candidates.into_iter().map(|(name, _, _, _)| name).collect(),
    })
}

pub fn read_wifi_configs(
    targets: &[String],
    session: Option<&str>,
) -> Result<BTreeMap<String, WifiNetworkConfig>, DomainError> {
    let mut result = BTreeMap::new();
    for target in targets {
        let response = uci_get_config("wireless", Some(target), None, session)?;
        let values = response
            .get("values")
            .and_then(Value::as_object)
            .ok_or_else(|| {
                DomainError::new(
                    ErrorCode::TargetMissing,
                    ErrorStage::Verify,
                    format!("target {target} has no UCI values table"),
                )
            })?;
        let ssid = values
            .get("ssid")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                DomainError::new(
                    ErrorCode::TargetMissing,
                    ErrorStage::Verify,
                    format!("target {target} has no SSID option"),
                )
            })?
            .to_owned();
        let encryption = values
            .get("encryption")
            .and_then(Value::as_str)
            .unwrap_or("none")
            .to_owned();
        let key = values
            .get("key")
            .and_then(Value::as_str)
            .map(str::to_owned);

        result.insert(
            target.clone(),
            WifiNetworkConfig {
                ssid,
                encryption,
                key,
                targets: vec![target.clone()],
            },
        );
    }
    Ok(result)
}



pub fn stage_wifi_config(
    session: &str,
    targets: &[String],
    config: &WifiNetworkConfig,
) -> Result<(), DomainError> {
    for target in targets {
        let mut values = serde_json::Map::new();
        values.insert("ssid".into(), json!(config.ssid));
        values.insert("encryption".into(), json!(config.encryption));
        if config.encryption != "none" {
            if let Some(key) = &config.key {
                values.insert("key".into(), json!(key));
            }
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
            DomainError::new(ErrorCode::UciStageFailed, ErrorStage::Stage, error.message)
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
