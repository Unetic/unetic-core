use std::collections::BTreeMap;

use serde_json::json;

use super::{rpc, wan, wireless};
use crate::{
    domain::errors::{ErrorCode, ErrorStage, LegacyAppError},
    domain::{
        AppliedRoamingConfig, DiscoveredWan, DiscoveredWifi, RoamingConfig, RoamingRuntime,
        WanDesired, WanPublicState, WifiNetworkConfig,
    },
    infrastructure::backend::RouterBackend,
};

pub struct OpenWrtBackend {
    temperature_reader: super::temperature::TemperatureReader,
}

impl OpenWrtBackend {
    pub fn new() -> Result<Self, LegacyAppError> {
        Ok(Self {
            temperature_reader: super::temperature::TemperatureReader::new(),
        })
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
        roaming: RoamingConfig,
        is_extender: bool,
    ) -> Result<(), LegacyAppError> {
        wireless::stage_wifi_config(session, targets, config, roaming, is_extender)
    }

    fn read_roaming_config(
        &self,
        targets: &[String],
        session: Option<&str>,
    ) -> Result<AppliedRoamingConfig, LegacyAppError> {
        wireless::roaming::read_roaming_config(targets, session)
    }

    fn read_roaming_runtime(
        &self,
        targets: &[String],
        ssid: &str,
        roaming: RoamingConfig,
    ) -> RoamingRuntime {
        wireless::usteer_runtime::read(targets, ssid, roaming)
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
        wan.qos = super::qos::read_sqm_config(None)?;
        Ok(wan)
    }

    fn read_wan_config(&self, session: Option<&str>) -> Result<WanDesired, LegacyAppError> {
        let mut wan = match rpc::uci_get_config("network", Some("wan"), None, session) {
            Ok(res) => wan::parse_discovered_wan(&res).to_desired(),
            Err(error) if error.code == ErrorCode::UciReadFailed => WanDesired {
                present: false,
                proto: crate::domain::WanProtocol::None,
                ..WanDesired::default()
            },
            Err(error) => return Err(error),
        };
        wan.qos = super::qos::read_sqm_config(session)?;
        Ok(wan)
    }

    fn stage_wan_config(&self, session: &str, config: &WanDesired) -> Result<(), LegacyAppError> {
        let qos = if config.proto == crate::domain::WanProtocol::Extender {
            None
        } else {
            config.qos.as_ref()
        };
        let interface = self.resolve_sqm_interface(session, config, qos)?;

        wan::replace_wan_section(session, config)?;
        super::qos::stage_sqm_config(session, interface.as_deref(), qos)?;

        Ok(())
    }

    fn read_wan_runtime_status(&self) -> Result<WanPublicState, LegacyAppError> {
        let mut status = match rpc::call_ubus("network.interface.wan", "status", json!({})) {
            Ok(res) => wan::parse_wan_runtime_status(&res),
            Err(error) if error.code == ErrorCode::NotFound => WanPublicState {
                present: false,
                proto: crate::domain::WanProtocol::None,
                status: crate::domain::WanStatus::NotConfigured,
                ..Default::default()
            },
            Err(error) => return Err(error),
        };
        status.qos = super::qos::read_sqm_config(None)?;
        Ok(status)
    }

    fn revert_staged(&self, session: &str) -> Result<(), LegacyAppError> {
        let mut failures = Vec::new();
        let mut retryable = false;
        for config in ["network", "sqm", "wireless", "usteer"] {
            if let Err(error) = rpc::call_ubus(
                "uci",
                "revert",
                json!({"config": config, "ubus_rpc_session": session}),
            ) {
                retryable |= error.retryable;
                failures.push(format!("{config}: {}", error.message));
            }
        }

        if failures.is_empty() {
            return Ok(());
        }
        Err(LegacyAppError::new(
            ErrorCode::UciStageFailed,
            ErrorStage::Rollback,
            format!(
                "failed to revert staged UCI changes: {}",
                failures.join("; ")
            ),
        )
        .retryable(retryable))
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

    fn read_traffic_counters(
        &self,
    ) -> Result<crate::domain::traffic::TrafficCounters, LegacyAppError> {
        super::traffic::read_traffic_counters(self.read_wan_runtime_status()?.device.as_deref())
    }

    fn read_switch_state(&self) -> Result<crate::domain::ports::SwitchState, LegacyAppError> {
        super::ports::read_switch_state()
    }

    fn set_hw_offload(
        &self,
        enabled: bool,
    ) -> Result<crate::domain::ports::SwitchState, LegacyAppError> {
        super::ports::set_hw_offload(enabled)
    }

    fn read_system_info(&self) -> Result<crate::domain::system::SystemInfo, LegacyAppError> {
        Ok(super::system::read_system_info())
    }

    fn read_system_runtime(&self) -> Result<crate::domain::system::SystemRuntime, LegacyAppError> {
        Ok(super::system::read_system_runtime(&self.temperature_reader))
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

impl OpenWrtBackend {
    fn resolve_sqm_interface(
        &self,
        session: &str,
        config: &crate::domain::WanDesired,
        qos: Option<&crate::domain::WanQos>,
    ) -> Result<Option<String>, LegacyAppError> {
        if qos.is_none() {
            return Ok(None);
        }
        if config.proto == crate::domain::WanProtocol::Pppoe {
            return Ok(Some("pppoe-wan".into()));
        }
        if let Some(device) = &config.device {
            return Ok(Some(device.clone()));
        }

        if let Ok(staged) = rpc::uci_get_config("network", Some("wan"), None, Some(session)) {
            let device = wan::parse_discovered_wan(&staged).device;
            if device.is_some() {
                return Ok(device);
            }
        }

        let status = rpc::call_ubus("network.interface.wan", "status", json!({}))?;
        let device = status
            .get("l3_device")
            .or_else(|| status.get("device"))
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| {
                LegacyAppError::new(
                    ErrorCode::InvalidArgument,
                    ErrorStage::Stage,
                    "cannot enable QoS before the WAN device is known",
                )
            })?;
        Ok(Some(device))
    }
}
