use std::collections::BTreeMap;

use crate::{
    domain::errors::LegacyAppError,
    domain::{DiscoveredWan, DiscoveredWifi, WanDesired, WanPublicState, WifiNetworkConfig},
};

pub mod memory;
pub use memory::{FailurePlan, MemoryBackend};

pub trait RouterBackend: Send + Sync {
    fn discover_primary_wifi(&self) -> Result<DiscoveredWifi, LegacyAppError>;
    fn discover_primary_wan(&self) -> Result<DiscoveredWan, LegacyAppError>;
    fn create_session(&self) -> Result<String, LegacyAppError>;
    fn destroy_session(&self, session: &str) -> Result<(), LegacyAppError>;
    fn read_wifi_configs(
        &self,
        targets: &[String],
        session: Option<&str>,
    ) -> Result<BTreeMap<String, WifiNetworkConfig>, LegacyAppError>;
    fn stage_wifi_config(
        &self,
        session: &str,
        targets: &[String],
        config: &WifiNetworkConfig,
        is_extender: bool,
    ) -> Result<(), LegacyAppError>;
    fn read_wan_config(&self, session: Option<&str>) -> Result<WanDesired, LegacyAppError>;
    fn stage_wan_config(&self, session: &str, config: &WanDesired) -> Result<(), LegacyAppError>;
    fn read_wan_runtime_status(&self) -> Result<WanPublicState, LegacyAppError>;
    fn revert_staged(&self, session: &str) -> Result<(), LegacyAppError>;
    fn apply(&self, session: &str, rollback_timeout_secs: u32) -> Result<(), LegacyAppError>;
    fn confirm(&self, session: &str) -> Result<(), LegacyAppError>;
    fn rollback(&self, session: &str) -> Result<(), LegacyAppError>;
    fn runtime_healthy(&self, targets: &[String], ssid: &str) -> Result<bool, LegacyAppError>;
    fn reload_wireless_runtime(&self) -> Result<(), LegacyAppError>;
    fn ports_list(&self) -> Result<Vec<crate::domain::ports::PhysicalPort>, LegacyAppError>;
    fn read_system_info(&self) -> Result<crate::domain::system::SystemInfo, LegacyAppError>;
    fn read_devices(
        &self,
        extenders: &[crate::domain::extender::KnownExtender],
        extender_clients: &std::collections::HashMap<String, Vec<crate::domain::extender::ExtenderClient>>
    ) -> Result<Vec<crate::domain::device::Device>, LegacyAppError>;
    fn write_static_lease(&self, mac: &str, ip: &str, hostname: Option<&str>) -> Result<(), LegacyAppError>;
    fn delete_static_lease(&self, mac: &str) -> Result<(), LegacyAppError>;
    fn sync_port_forwards(&self, registered_devices: &[crate::domain::device::RegisteredDevice], current_devices: &[crate::domain::device::Device]) -> Result<(), LegacyAppError>;
    fn read_dns_config(&self) -> Result<crate::domain::DnsConfig, LegacyAppError>;
    fn write_dns_config(&self, cfg: &crate::domain::DnsConfig) -> Result<(), LegacyAppError>;
    fn write_ddns_config(&self, cfg: &crate::domain::DdnsConfig) -> Result<(), LegacyAppError>;

    fn read_ssids(
        &self,
        targets: &[String],
        session: Option<&str>,
    ) -> Result<BTreeMap<String, String>, LegacyAppError> {
        let configs = self.read_wifi_configs(targets, session)?;
        Ok(configs
            .into_iter()
            .map(|(target, config)| (target, config.ssid))
            .collect())
    }
}

pub struct SessionGuard<'a> {
    pub id: String,
    backend: &'a dyn RouterBackend,
}

impl<'a> SessionGuard<'a> {
    pub fn new(backend: &'a dyn RouterBackend) -> Result<Self, LegacyAppError> {
        let id = backend.create_session()?;
        Ok(Self { id, backend })
    }
}

impl Drop for SessionGuard<'_> {
    fn drop(&mut self) {
        let _ = self.backend.destroy_session(&self.id);
    }
}
