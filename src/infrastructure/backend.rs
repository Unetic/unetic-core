use std::collections::BTreeMap;

use crate::{
    domain::errors::DomainError,
    domain::{DiscoveredWan, DiscoveredWifi, WanDesired, WanPublicState, WifiNetworkConfig},
};

pub mod memory;
pub use memory::{FailurePlan, MemoryBackend};

pub trait RouterBackend: Send + Sync {
    fn discover_primary_wifi(&self) -> Result<DiscoveredWifi, DomainError>;
    fn discover_primary_wan(&self) -> Result<DiscoveredWan, DomainError>;
    fn create_session(&self) -> Result<String, DomainError>;
    fn destroy_session(&self, session: &str) -> Result<(), DomainError>;
    fn read_wifi_configs(
        &self,
        targets: &[String],
        session: Option<&str>,
    ) -> Result<BTreeMap<String, WifiNetworkConfig>, DomainError>;
    fn stage_wifi_config(
        &self,
        session: &str,
        targets: &[String],
        config: &WifiNetworkConfig,
    ) -> Result<(), DomainError>;
    fn read_wan_config(&self, session: Option<&str>) -> Result<WanDesired, DomainError>;
    fn stage_wan_config(&self, session: &str, config: &WanDesired) -> Result<(), DomainError>;
    fn read_wan_runtime_status(&self) -> Result<WanPublicState, DomainError>;
    fn revert_staged(&self, session: &str) -> Result<(), DomainError>;
    fn apply(&self, session: &str, rollback_timeout_secs: u32) -> Result<(), DomainError>;
    fn confirm(&self, session: &str) -> Result<(), DomainError>;
    fn rollback(&self, session: &str) -> Result<(), DomainError>;
    fn runtime_healthy(&self, targets: &[String], ssid: &str) -> Result<bool, DomainError>;
    fn reload_wireless_runtime(&self) -> Result<(), DomainError>;
    fn read_switch_info(&self) -> Result<crate::domain::switch::SwitchInfo, DomainError>;
    fn read_system_info(&self) -> Result<crate::domain::system::SystemInfo, DomainError>;
    fn read_devices(&self) -> Result<Vec<crate::domain::device::Device>, DomainError>;

    fn read_ssids(
        &self,
        targets: &[String],
        session: Option<&str>,
    ) -> Result<BTreeMap<String, String>, DomainError> {
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
    pub fn new(backend: &'a dyn RouterBackend) -> Result<Self, DomainError> {
        let id = backend.create_session()?;
        Ok(Self { id, backend })
    }
}

impl Drop for SessionGuard<'_> {
    fn drop(&mut self) {
        let _ = self.backend.destroy_session(&self.id);
    }
}
