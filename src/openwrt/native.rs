use std::{collections::BTreeMap, path::Path};

use serde_json::json;

use super::{rpc, switch, wan, wireless};
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
}

impl RouterBackend for OpenWrtBackend {
    fn discover_primary_wifi(&self) -> Result<DiscoveredWifi, DomainError> {
        wireless::discover_primary_wifi()
    }

    fn create_session(&self) -> Result<String, DomainError> {
        crate::openwrt::rpc::create_rpcd_session()
    }

    fn destroy_session(&self, session: &str) -> Result<(), DomainError> {
        crate::openwrt::rpc::destroy_rpcd_session(session)
    }

    fn read_ssids(
        &self,
        targets: &[String],
        session: Option<&str>,
    ) -> Result<BTreeMap<String, String>, DomainError> {
        wireless::read_ssids(targets, session)
    }

    fn stage_ssid(&self, session: &str, targets: &[String], ssid: &str) -> Result<(), DomainError> {
        wireless::stage_ssid(session, targets, ssid)
    }

    fn discover_primary_wan(&self) -> Result<DiscoveredWan, DomainError> {
        match rpc::uci_get_config("network", Some("wan"), None, None) {
            Ok(res) => Ok(wan::parse_discovered_wan(&res)),
            Err(error) if error.code == ErrorCode::UciReadFailed => Ok(DiscoveredWan {
                present: false,
                proto: crate::model::WanProtocol::None,
                ..DiscoveredWan::default()
            }),
            Err(error) => Err(error),
        }
    }

    fn read_wan_config(&self, session: Option<&str>) -> Result<WanDesired, DomainError> {
        match rpc::uci_get_config("network", Some("wan"), None, session) {
            Ok(res) => Ok(wan::parse_discovered_wan(&res).to_desired()),
            Err(error) if error.code == ErrorCode::UciReadFailed => Ok(WanDesired::default()),
            Err(error) => Err(error),
        }
    }

    fn stage_wan_config(&self, session: &str, config: &WanDesired) -> Result<(), DomainError> {
        let values = wan::build_wan_staging_values(config);
        rpc::call_ubus(
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
        let response = match rpc::call_ubus("network.interface.wan", "status", json!({})) {
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
        Ok(wan::parse_wan_runtime_status(&response))
    }

    fn revert_staged(&self, session: &str) -> Result<(), DomainError> {
        let _ = rpc::call_ubus(
            "uci",
            "revert",
            json!({"config": "network", "ubus_rpc_session": session}),
        );
        rpc::call_ubus(
            "uci",
            "revert",
            json!({"config": "wireless", "ubus_rpc_session": session}),
        )
        .map(|_| ())
        .map_err(|error| {
            DomainError::new(
                ErrorCode::UciStageFailed,
                ErrorStage::Rollback,
                format!("failed to revert staged UCI changes: {}", error.message),
            )
            .retryable(error.retryable)
        })
    }

    fn apply(&self, session: &str, rollback_timeout_secs: u32) -> Result<(), DomainError> {
        rpc::call_ubus(
            "uci",
            "apply",
            json!({
                "ubus_rpc_session": session,
                "timeout": rollback_timeout_secs,
                "rollback": true
            }),
        )
        .map(|_| ())
        .map_err(|error| {
            DomainError::new(
                ErrorCode::UciApplyFailed,
                ErrorStage::Apply,
                format!("failed to apply staged UCI changes: {}", error.message),
            )
            .retryable(error.retryable)
        })
    }

    fn confirm(&self, session: &str) -> Result<(), DomainError> {
        rpc::call_ubus("uci", "confirm", json!({"ubus_rpc_session": session}))
            .map(|_| ())
            .map_err(|error| {
                DomainError::new(
                    ErrorCode::ConfirmFailed,
                    ErrorStage::Confirm,
                    format!("failed to confirm applied UCI changes: {}", error.message),
                )
                .retryable(error.retryable)
            })
    }

    fn rollback(&self, session: &str) -> Result<(), DomainError> {
        rpc::call_ubus("uci", "rollback", json!({"ubus_rpc_session": session}))
            .map(|_| ())
            .map_err(|error| {
                DomainError::new(
                    ErrorCode::RollbackFailed,
                    ErrorStage::Rollback,
                    format!("failed to roll back applied UCI changes: {}", error.message),
                )
                .retryable(error.retryable)
            })
    }

    fn runtime_healthy(&self, targets: &[String], ssid: &str) -> Result<bool, DomainError> {
        wireless::check_runtime_healthy(targets, ssid)
    }

    fn reload_wireless_runtime(&self) -> Result<(), DomainError> {
        wireless::reload_wireless()
    }

    fn read_switch_info(&self) -> Result<crate::switch::SwitchInfo, DomainError> {
        let sys_root = Path::new("/sys");
        let debug_root = Path::new("/sys/kernel/debug");
        Ok(switch::read_switch_info(sys_root, debug_root))
    }

    fn read_system_info(&self) -> Result<crate::system::SystemInfo, DomainError> {
        Ok(super::system::read_system_info())
    }
}
