use std::collections::BTreeMap;

use serde_json::{Value, json};

use super::rpc::{call_ubus, uci_get_config};
use crate::{
    domain::errors::{LegacyAppError, ErrorCode, ErrorStage},
    domain::{DiscoveredWifi, WifiNetworkConfig},
};

type WifiCandidate = (String, String, String, Option<String>);

pub fn discover_primary_wifi() -> Result<DiscoveredWifi, LegacyAppError> {
    let response = uci_get_config("wireless", None, None, None)?;
    let values = response
        .get("values")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            LegacyAppError::new(
                ErrorCode::AmbiguousWifiConfig,
                ErrorStage::Bootstrap,
                "wireless UCI response has no values table",
            )
        })?;

    let candidates: Vec<WifiCandidate> = values
        .iter()
        .filter_map(|(name, section)| parse_lan_ap_candidate(name, section))
        .collect();

    if candidates.is_empty() {
        return Err(LegacyAppError::new(
            ErrorCode::AmbiguousWifiConfig,
            ErrorStage::Bootstrap,
            "no LAN AP wifi-iface sections found",
        ));
    }

    let first = &candidates[0];
    let differs = candidates
        .iter()
        .any(|(_, ssid, enc, key)| ssid != &first.1 || enc != &first.2 || key != &first.3);

    if differs {
        return Err(LegacyAppError::new(
            ErrorCode::AmbiguousWifiConfig,
            ErrorStage::Bootstrap,
            "LAN AP wifi-iface sections use different wireless settings",
        ));
    }

    Ok(DiscoveredWifi {
        ssid: first.1.clone(),
        encryption: first.2.clone(),
        key: first.3.clone(),
        targets: candidates.into_iter().map(|(name, _, _, _)| name).collect(),
    })
}

fn parse_lan_ap_candidate(name: &str, section: &Value) -> Option<WifiCandidate> {
    let section = section.as_object()?;
    if section.get(".type").and_then(Value::as_str) != Some("wifi-iface")
        || section.get("mode").and_then(Value::as_str) != Some("ap")
        || section.get("disabled").is_some_and(is_truthy)
    {
        return None;
    }

    let belongs_to_lan = section.get("network").is_some_and(|network| match network {
        Value::String(value) => value.split_ascii_whitespace().any(|part| part == "lan"),
        Value::Array(values) => values.iter().any(|value| value.as_str() == Some("lan")),
        _ => false,
    });
    if !belongs_to_lan {
        return None;
    }

    let ssid = section.get("ssid").and_then(Value::as_str)?;
    if ssid.is_empty() {
        return None;
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

    Some((name.to_owned(), ssid.to_owned(), encryption, key))
}

pub fn read_wifi_configs(
    targets: &[String],
    session: Option<&str>,
) -> Result<BTreeMap<String, WifiNetworkConfig>, LegacyAppError> {
    let mut result = BTreeMap::new();
    for target in targets {
        let config = read_target_wifi_config(target, session)?;
        result.insert(target.clone(), config);
    }
    Ok(result)
}

fn read_target_wifi_config(
    target: &str,
    session: Option<&str>,
) -> Result<WifiNetworkConfig, LegacyAppError> {
    let response = uci_get_config("wireless", Some(target), None, session)?;
    let values = response
        .get("values")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            LegacyAppError::new(
                ErrorCode::TargetMissing,
                ErrorStage::Verify,
                format!("target {target} has no UCI values table"),
            )
        })?;

    let ssid = values
        .get("ssid")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            LegacyAppError::new(
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
    let key = values.get("key").and_then(Value::as_str).map(str::to_owned);

    Ok(WifiNetworkConfig {
        ssid,
        encryption,
        key,
        targets: vec![target.to_owned()],
    })
}

pub fn stage_wifi_config(
    session: &str,
    targets: &[String],
    config: &WifiNetworkConfig,
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
        
        let md = format!("{:04x}", config.ssid.bytes().fold(0u16, |acc, b| acc.wrapping_add(b as u16)));
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
