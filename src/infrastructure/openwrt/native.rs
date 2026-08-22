use std::collections::BTreeMap;

use serde_json::json;

use super::{devices, rpc, wan, wireless};
use crate::{
    domain::errors::{LegacyAppError, ErrorCode, ErrorStage},
    domain::{DiscoveredWan, DiscoveredWifi, WanDesired, WanPublicState, WifiNetworkConfig},
    infrastructure::backend::RouterBackend,
};

pub struct OpenWrtBackend;

impl OpenWrtBackend {
    pub fn new() -> Result<Self, LegacyAppError> {
        Ok(Self)
    }
}

impl RouterBackend for OpenWrtBackend {
    fn discover_primary_wifi(&self) -> Result<DiscoveredWifi, LegacyAppError> {
        wireless::discover_primary_wifi()
    }

    fn create_session(&self) -> Result<String, LegacyAppError> {
        crate::infrastructure::openwrt::rpc::create_rpcd_session()
    }

    fn destroy_session(&self, session: &str) -> Result<(), LegacyAppError> {
        crate::infrastructure::openwrt::rpc::destroy_rpcd_session(session)
    }

    fn read_wifi_configs(
        &self,
        targets: &[String],
        session: Option<&str>,
    ) -> Result<BTreeMap<String, WifiNetworkConfig>, LegacyAppError> {
        wireless::read_wifi_configs(targets, session)
    }

    fn stage_wifi_config(
        &self,
        session: &str,
        targets: &[String],
        config: &WifiNetworkConfig,
    ) -> Result<(), LegacyAppError> {
        wireless::stage_wifi_config(session, targets, config)
    }

    fn discover_primary_wan(&self) -> Result<DiscoveredWan, LegacyAppError> {
        match rpc::uci_get_config("network", Some("wan"), None, None) {
            Ok(res) => Ok(wan::parse_discovered_wan(&res)),
            Err(error) if error.code == ErrorCode::UciReadFailed => Ok(DiscoveredWan {
                present: false,
                proto: crate::domain::WanProtocol::None,
                ..DiscoveredWan::default()
            }),
            Err(error) => Err(error),
        }
    }

    fn read_wan_config(&self, session: Option<&str>) -> Result<WanDesired, LegacyAppError> {
        match rpc::uci_get_config("network", Some("wan"), None, session) {
            Ok(res) => Ok(wan::parse_discovered_wan(&res).to_desired()),
            Err(error) if error.code == ErrorCode::UciReadFailed => Ok(WanDesired::default()),
            Err(error) => Err(error),
        }
    }

    fn stage_wan_config(&self, session: &str, config: &WanDesired) -> Result<(), LegacyAppError> {
        let values = crate::infrastructure::openwrt::wan::build_wan_staging_values(config);
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
            LegacyAppError::new(ErrorCode::UciStageFailed, ErrorStage::Stage, error.message)
                .retryable(error.retryable)
        })
    }

    fn read_wan_runtime_status(&self) -> Result<WanPublicState, LegacyAppError> {
        let response = match rpc::call_ubus("network.interface.wan", "status", json!({})) {
            Ok(res) => res,
            Err(_) => {
                return Ok(WanPublicState {
                    present: false,
                    proto: crate::domain::WanProtocol::None,
                    status: crate::domain::WanStatus::NotConfigured,
                    ..Default::default()
                });
            }
        };
        Ok(wan::parse_wan_runtime_status(&response))
    }

    fn revert_staged(&self, session: &str) -> Result<(), LegacyAppError> {
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
            LegacyAppError::new(
                ErrorCode::UciStageFailed,
                ErrorStage::Rollback,
                format!("failed to revert staged UCI changes: {}", error.message),
            )
            .retryable(error.retryable)
        })
    }

    fn apply(&self, session: &str, rollback_timeout_secs: u32) -> Result<(), LegacyAppError> {
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
            LegacyAppError::new(
                ErrorCode::UciApplyFailed,
                ErrorStage::Apply,
                format!("failed to apply staged UCI changes: {}", error.message),
            )
            .retryable(error.retryable)
        })
    }

    fn confirm(&self, session: &str) -> Result<(), LegacyAppError> {
        rpc::call_ubus("uci", "confirm", json!({"ubus_rpc_session": session}))
            .map(|_| ())
            .map_err(|error| {
                LegacyAppError::new(
                    ErrorCode::ConfirmFailed,
                    ErrorStage::Confirm,
                    format!("failed to confirm applied UCI changes: {}", error.message),
                )
                .retryable(error.retryable)
            })
    }

    fn rollback(&self, session: &str) -> Result<(), LegacyAppError> {
        rpc::call_ubus("uci", "rollback", json!({"ubus_rpc_session": session}))
            .map(|_| ())
            .map_err(|error| {
                LegacyAppError::new(
                    ErrorCode::RollbackFailed,
                    ErrorStage::Rollback,
                    format!("failed to roll back applied UCI changes: {}", error.message),
                )
                .retryable(error.retryable)
            })
    }

    fn runtime_healthy(&self, targets: &[String], ssid: &str) -> Result<bool, LegacyAppError> {
        wireless::check_runtime_healthy(targets, ssid)
    }

    fn reload_wireless_runtime(&self) -> Result<(), LegacyAppError> {
        wireless::reload_wireless()
    }

    fn ports_list(&self) -> Result<Vec<crate::domain::ports::PhysicalPort>, LegacyAppError> {
        let devices = self.read_devices().unwrap_or_default();
        Ok(super::ports::ports_list(&devices))
    }

    fn read_system_info(&self) -> Result<crate::domain::system::SystemInfo, LegacyAppError> {
        Ok(super::system::read_system_info())
    }

    fn read_devices(&self) -> Result<Vec<crate::domain::device::Device>, LegacyAppError> {
        devices::read_devices()
    }

    fn write_static_lease(&self, _mac: &str, _ip: &str, _hostname: Option<&str>) -> Result<(), LegacyAppError> {
        Ok(())
    }
    fn delete_static_lease(&self, _mac: &str) -> Result<(), LegacyAppError> {
        Ok(())
    }
    fn sync_port_forwards(&self, _registered_devices: &[crate::domain::device::RegisteredDevice], _current_devices: &[crate::domain::device::Device]) -> Result<(), LegacyAppError> {
        Ok(())
    }
    fn read_dns_config(&self) -> Result<crate::domain::DnsConfig, LegacyAppError> {
        Ok(super::dns::read_dns_config())
    }
    fn write_dns_config(&self, cfg: &crate::domain::DnsConfig) -> Result<(), LegacyAppError> {
        super::dns::write_dns_config(cfg)
    }
    fn write_ddns_config(&self, _cfg: &crate::domain::DdnsConfig) -> Result<(), crate::domain::errors::LegacyAppError> { Ok(()) }
}
