use std::collections::BTreeMap;

use serde_json::json;

use super::{rpc, wan, wireless};
use crate::{
    domain::errors::{ErrorCode, ErrorStage, LegacyAppError},
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
        is_extender: bool,
    ) -> Result<(), LegacyAppError> {
        wireless::stage_wifi_config(session, targets, config, is_extender)
    }

    fn discover_primary_wan(&self) -> Result<DiscoveredWan, LegacyAppError> {
        let mut wan = match rpc::uci_get_config("network", Some("wan"), None, None) {
            Ok(res) => wan::parse_discovered_wan(&res),
            Err(error) if error.code == ErrorCode::UciReadFailed => DiscoveredWan {
                present: false,
                proto: crate::domain::WanProtocol::None,
                ..DiscoveredWan::default()
            },
            Err(error) => return Err(error),
        };
        wan.qos = super::qos::read_sqm_config();
        Ok(wan)
    }

    fn read_wan_config(&self, session: Option<&str>) -> Result<WanDesired, LegacyAppError> {
        let mut wan = match rpc::uci_get_config("network", Some("wan"), None, session) {
            Ok(res) => wan::parse_discovered_wan(&res).to_desired(),
            Err(error) if error.code == ErrorCode::UciReadFailed => WanDesired::default(),
            Err(error) => return Err(error),
        };
        wan.qos = super::qos::read_sqm_config();
        Ok(wan)
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
        })?;

        if config.proto != crate::domain::WanProtocol::Extender {
            super::qos::write_sqm_config(config.device.as_deref(), &config.qos)?;
        } else {
            super::qos::write_sqm_config(None, &None)?;
        }

        Ok(())
    }

    fn read_wan_runtime_status(&self) -> Result<WanPublicState, LegacyAppError> {
        let mut status = match rpc::call_ubus("network.interface.wan", "status", json!({})) {
            Ok(res) => wan::parse_wan_runtime_status(&res),
            Err(_) => {
                return Ok(WanPublicState {
                    present: false,
                    proto: crate::domain::WanProtocol::None,
                    status: crate::domain::WanStatus::NotConfigured,
                    ..Default::default()
                });
            }
        };
        status.qos = super::qos::read_sqm_config();
        Ok(status)
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
        let empty_extenders: Vec<crate::domain::extender::KnownExtender> = Vec::new();
        let empty_clients = std::collections::HashMap::new();
        let devices = self
            .read_devices(&empty_extenders, &empty_clients)
            .unwrap_or_default();
        let wan = self.read_wan_runtime_status()?.device;
        Ok(super::ports::ports_list(&devices, wan.as_deref()))
    }

    fn read_system_info(&self) -> Result<crate::domain::system::SystemInfo, LegacyAppError> {
        Ok(super::system::read_system_info())
    }

    fn read_devices(
        &self,
        extenders: &[crate::domain::extender::KnownExtender],
        extender_clients: &std::collections::HashMap<
            String,
            Vec<crate::domain::extender::ExtenderClient>,
        >,
    ) -> Result<Vec<crate::domain::device::Device>, LegacyAppError> {
        super::devices::read_devices(extenders, extender_clients)
    }

    fn write_static_lease(
        &self,
        mac: &str,
        ip: &str,
        hostname: Option<&str>,
    ) -> Result<(), LegacyAppError> {
        super::device_config::write_static_lease(mac, ip, hostname)
    }
    fn delete_static_lease(&self, mac: &str) -> Result<(), LegacyAppError> {
        super::device_config::delete_static_lease(mac)
    }
    fn sync_port_forwards(
        &self,
        registered_devices: &[crate::domain::device::RegisteredDevice],
        current_devices: &[crate::domain::device::Device],
    ) -> Result<(), LegacyAppError> {
        super::device_config::sync_port_forwards(registered_devices, current_devices)
    }
    fn read_dns_config(&self) -> Result<crate::domain::DnsConfig, LegacyAppError> {
        super::dns::read_dns_config()
    }
    fn write_dns_config(&self, cfg: &crate::domain::DnsConfig) -> Result<(), LegacyAppError> {
        super::dns::write_dns_config(cfg)
    }
}
