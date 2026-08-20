use std::{collections::BTreeMap, path::Path};

use serde_json::{Map, Value, json};

use crate::{
    backend::RouterBackend,
    errors::{DomainError, ErrorCode, ErrorStage},
    model::{DiscoveredWan, DiscoveredWifi, WanDesired, WanPublicState},
};

pub struct OpenWrtBackend;

impl OpenWrtBackend {
    pub fn new() -> Result<Self, DomainError> {
        Ok(Self)
    }

    fn call(&self, object: &str, method: &str, request: Value) -> Result<Value, DomainError> {
        let payload = serde_json::to_string(&request).map_err(|error| {
            DomainError::new(
                ErrorCode::Internal,
                ErrorStage::Transport,
                format!("failed to encode ubus request: {error}"),
            )
        })?;

        let socket = Path::new("/var/run/ubus/ubus.sock");
        let mut connection = ubus::Connection::connect(socket).map_err(|error| {
            DomainError::new(
                ErrorCode::UbusUnavailable,
                ErrorStage::Transport,
                format!("failed to connect to ubus: {error}"),
            )
            .retryable(true)
        })?;

        let response = connection.call(object, method, &payload).map_err(|error| {
            let code = if object == "session" {
                ErrorCode::RpcdSessionLost
            } else {
                ErrorCode::UbusUnavailable
            };
            DomainError::new(
                code,
                ErrorStage::Transport,
                format!("ubus {object}.{method} failed: {error}"),
            )
            .retryable(true)
        })?;

        serde_json::from_str(&response).map_err(|error| {
            DomainError::new(
                ErrorCode::UbusUnavailable,
                ErrorStage::Transport,
                format!("invalid JSON reply from {object}.{method}: {error}"),
            )
        })
    }

    fn new_session(&self) -> Result<String, DomainError> {
        let response = self.call("session", "create", json!({"timeout": 300}))?;
        let sid = response
            .get("ubus_rpc_session")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                DomainError::new(
                    ErrorCode::RpcdSessionLost,
                    ErrorStage::Transport,
                    "rpcd session.create did not return ubus_rpc_session",
                )
            })?
            .to_owned();

        self.call(
            "session",
            "grant",
            json!({
                "ubus_rpc_session": sid,
                "scope": "uci",
                "objects": [
                    ["wireless", "read"],
                    ["wireless", "write"],
                    ["network", "read"],
                    ["network", "write"]
                ]
            }),
        )?;
        Ok(sid)
    }

    fn uci_get(
        &self,
        section: Option<&str>,
        option: Option<&str>,
        session: Option<&str>,
    ) -> Result<Value, DomainError> {
        self.uci_get_config("wireless", section, option, session)
    }

    fn uci_get_config(
        &self,
        config: &str,
        section: Option<&str>,
        option: Option<&str>,
        session: Option<&str>,
    ) -> Result<Value, DomainError> {
        let mut request = Map::new();
        request.insert("config".into(), Value::String(config.into()));
        if let Some(section) = section {
            request.insert("section".into(), Value::String(section.into()));
        }
        if let Some(option) = option {
            request.insert("option".into(), Value::String(option.into()));
        }
        if let Some(session) = session {
            request.insert("ubus_rpc_session".into(), Value::String(session.to_owned()));
        }
        self.call("uci", "get", Value::Object(request))
            .map_err(|error| {
                DomainError::new(ErrorCode::UciReadFailed, ErrorStage::Verify, error.message)
                    .retryable(error.retryable)
            })
    }

    fn set_for_session(&self, session: &str, section: &str, ssid: &str) -> Result<(), DomainError> {
        self.call(
            "uci",
            "set",
            json!({
                "config": "wireless",
                "section": section,
                "values": {"ssid": ssid},
                "ubus_rpc_session": session
            }),
        )
        .map(|_| ())
        .map_err(|error| {
            DomainError::new(ErrorCode::UciStageFailed, ErrorStage::Stage, error.message)
                .retryable(error.retryable)
        })
    }
}

impl RouterBackend for OpenWrtBackend {
    fn discover_primary_wifi(&self) -> Result<DiscoveredWifi, DomainError> {
        let response = self.uci_get(None, None, None)?;
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

    fn create_session(&self) -> Result<String, DomainError> {
        self.new_session()
    }

    fn read_ssids(
        &self,
        targets: &[String],
        session: Option<&str>,
    ) -> Result<BTreeMap<String, String>, DomainError> {
        let mut result = BTreeMap::new();
        for target in targets {
            let response = self.uci_get(Some(target), Some("ssid"), session)?;
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

    fn stage_ssid(&self, session: &str, targets: &[String], ssid: &str) -> Result<(), DomainError> {
        for target in targets {
            self.set_for_session(session, target, ssid)?;
        }
        Ok(())
    }

    fn discover_primary_wan(&self) -> Result<DiscoveredWan, DomainError> {
        let response = match self.uci_get_config("network", Some("wan"), None, None) {
            Ok(res) => res,
            Err(error) if error.code == ErrorCode::UciReadFailed => {
                return Ok(DiscoveredWan {
                    present: false,
                    proto: crate::model::WanProtocol::None,
                    ..DiscoveredWan::default()
                });
            }
            Err(error) => return Err(error),
        };
        Ok(super::wan::parse_discovered_wan(&response))
    }

    fn read_wan_config(&self, session: Option<&str>) -> Result<WanDesired, DomainError> {
        let response = match self.uci_get_config("network", Some("wan"), None, session) {
            Ok(res) => res,
            Err(error) if error.code == ErrorCode::UciReadFailed => {
                return Ok(WanDesired::default());
            }
            Err(error) => return Err(error),
        };
        Ok(super::wan::parse_discovered_wan(&response).to_desired())
    }

    fn stage_wan_config(&self, session: &str, config: &WanDesired) -> Result<(), DomainError> {
        let values = super::wan::build_wan_staging_values(config);
        self.call(
            "uci",
            "set",
            json!({
                "config": "network",
                "section": "wan",
                "values": values,
                "ubus_rpc_session": session
            }),
        )
        .map(|_| ())
        .map_err(|error| {
            DomainError::new(ErrorCode::UciStageFailed, ErrorStage::Stage, error.message)
                .retryable(error.retryable)
        })
    }

    fn read_wan_runtime_status(&self) -> Result<WanPublicState, DomainError> {
        let response = match self.call("network.interface.wan", "status", json!({})) {
            Ok(res) => res,
            Err(_) => {
                return Ok(WanPublicState {
                    present: false,
                    proto: crate::model::WanProtocol::None,
                    status: crate::model::WanStatus::NotConfigured,
                    ..Default::default()
                });
            }
        };
        Ok(super::wan::parse_wan_runtime_status(&response))
    }

    fn revert_staged(&self, session: &str) -> Result<(), DomainError> {
        let _ = self.call(
            "uci",
            "revert",
            json!({"config": "network", "ubus_rpc_session": session}),
        );
        self.call(
            "uci",
            "revert",
            json!({"config": "wireless", "ubus_rpc_session": session}),
        )
        .map(|_| ())
        .map_err(|error| {
            DomainError::new(
                ErrorCode::UciStageFailed,
                ErrorStage::Rollback,
                error.message,
            )
        })
    }

    fn apply(&self, session: &str, rollback_timeout_secs: u32) -> Result<(), DomainError> {
        self.call(
            "uci",
            "apply",
            json!({
                "rollback": true,
                "timeout": rollback_timeout_secs,
                "ubus_rpc_session": session
            }),
        )
        .map(|_| ())
        .map_err(|error| {
            DomainError::new(ErrorCode::UciApplyFailed, ErrorStage::Apply, error.message)
                .retryable(true)
        })
    }

    fn confirm(&self, session: &str) -> Result<(), DomainError> {
        self.call("uci", "confirm", json!({"ubus_rpc_session": session}))
            .map(|_| ())
            .map_err(|error| {
                DomainError::new(ErrorCode::ConfirmFailed, ErrorStage::Confirm, error.message)
                    .retryable(true)
            })
    }

    fn rollback(&self, session: &str) -> Result<(), DomainError> {
        self.call("uci", "rollback", json!({"ubus_rpc_session": session}))
            .map(|_| ())
            .map_err(|error| {
                DomainError::new(
                    ErrorCode::RollbackFailed,
                    ErrorStage::Rollback,
                    error.message,
                )
                .retryable(true)
            })
    }

    fn runtime_healthy(&self, targets: &[String], ssid: &str) -> Result<bool, DomainError> {
        let response = self.call("network.wireless", "status", json!({}))?;
        if contains_true_key(&response, "pending") {
            return Ok(false);
        }

        Ok(targets
            .iter()
            .all(|target| runtime_target_matches(&response, target, ssid)))
    }

    fn reload_wireless_runtime(&self) -> Result<(), DomainError> {
        self.call("network.wireless", "down", json!({}))
            .and_then(|_| self.call("network.wireless", "up", json!({})))
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
}

fn runtime_target_matches(value: &Value, target: &str, ssid: &str) -> bool {
    match value {
        Value::Object(map) => {
            if map.get("section").and_then(Value::as_str) == Some(target) {
                let configured_ssid = map
                    .get("config")
                    .and_then(Value::as_object)
                    .and_then(|config| config.get("ssid"))
                    .and_then(Value::as_str)
                    .or_else(|| map.get("ssid").and_then(Value::as_str));
                return configured_ssid == Some(ssid);
            }
            map.values()
                .any(|child| runtime_target_matches(child, target, ssid))
        }
        Value::Array(values) => values
            .iter()
            .any(|child| runtime_target_matches(child, target, ssid)),
        _ => false,
    }
}

fn is_truthy(value: &Value) -> bool {
    value.as_bool() == Some(true)
        || value.as_u64() == Some(1)
        || value.as_str().is_some_and(|value| value == "1")
}

fn contains_true_key(value: &Value, key: &str) -> bool {
    match value {
        Value::Object(map) => map.iter().any(|(name, child)| {
            (name == key && child.as_bool() == Some(true)) || contains_true_key(child, key)
        }),
        Value::Array(values) => values.iter().any(|child| contains_true_key(child, key)),
        _ => false,
    }
}
