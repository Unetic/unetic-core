use serde_json::Value;
use std::collections::BTreeMap;



use super::rpc::uci_get_config;
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

pub mod stage;
pub use stage::*;
