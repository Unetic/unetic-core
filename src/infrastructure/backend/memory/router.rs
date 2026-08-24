use std::collections::BTreeMap;

use super::MemoryBackend;
use crate::{
    domain::errors::{ErrorCode, ErrorStage, LegacyAppError},
    domain::{
        AppliedRoamingConfig, DiscoveredWan, DiscoveredWifi, RoamingConfig, RoamingRuntime,
        RoamingRuntimeStatus, WanDesired, WanProtocol, WanPublicState, WanStatus,
        WifiNetworkConfig,
    },
    infrastructure::backend::RouterBackend,
};

impl RouterBackend for MemoryBackend {
    fn discover_primary_wifi(&self) -> Result<DiscoveredWifi, LegacyAppError> {
        let state = self.state.lock().expect("memory backend poisoned");
        let mut configs = state.committed.values();
        let Some(first) = configs.next() else {
            return Err(LegacyAppError::new(
                ErrorCode::AmbiguousWifiConfig,
                ErrorStage::Bootstrap,
                "no AP targets found",
            ));
        };
        if configs
            .any(|c| c.ssid != first.ssid || c.encryption != first.encryption || c.key != first.key)
        {
            return Err(LegacyAppError::new(
                ErrorCode::AmbiguousWifiConfig,
                ErrorStage::Bootstrap,
                "managed APs do not share one Wi-Fi configuration",
            ));
        }
        Ok(DiscoveredWifi {
            ssid: first.ssid.clone(),
            encryption: first.encryption.clone(),
            key: first.key.clone(),
            targets: state.committed.keys().cloned().collect(),
            backhaul: None,
            radio_channels: Vec::new(),
        })
    }

    fn discover_primary_wan(&self) -> Result<DiscoveredWan, LegacyAppError> {
        self.mem_discover_primary_wan()
    }

    fn create_session(&self) -> Result<String, LegacyAppError> {
        let mut state = self.state.lock().expect("memory backend poisoned");
        let sid = format!("memory-session-{}", state.next_session);
        state.next_session += 1;
        let committed = state.committed.clone();
        state.sessions.insert(sid.clone(), committed);
        let roaming = state.roaming_committed;
        state.roaming_sessions.insert(sid.clone(), roaming);
        let wan_committed = state.wan_committed.clone();
        state.wan_sessions.insert(sid.clone(), wan_committed);
        Ok(sid)
    }

    fn destroy_session(&self, session: &str) -> Result<(), LegacyAppError> {
        let mut state = self.state.lock().expect("memory backend poisoned");
        state.sessions.remove(session);
        state.wan_sessions.remove(session);
        state.rollback_snapshots.remove(session);
        state.roaming_sessions.remove(session);
        state.roaming_rollback_snapshots.remove(session);
        state.wan_rollback_snapshots.remove(session);
        Ok(())
    }

    fn read_wifi_configs(
        &self,
        targets: &[String],
        session: Option<&str>,
    ) -> Result<BTreeMap<String, WifiNetworkConfig>, LegacyAppError> {
        let state = self.state.lock().expect("memory backend poisoned");
        let source = session
            .and_then(|sid| state.sessions.get(sid))
            .unwrap_or(&state.committed);
        targets
            .iter()
            .map(|target| {
                source.get(target).cloned().map_or_else(
                    || {
                        Err(LegacyAppError::new(
                            ErrorCode::TargetMissing,
                            ErrorStage::Verify,
                            format!("missing target {target}"),
                        ))
                    },
                    |cfg| Ok((target.clone(), cfg)),
                )
            })
            .collect()
    }

    fn stage_wifi_config(
        &self,
        session: &str,
        targets: &[String],
        config: &WifiNetworkConfig,
        roaming: RoamingConfig,
        _is_extender: bool,
    ) -> Result<(), LegacyAppError> {
        let mut state = self.state.lock().expect("memory backend poisoned");
        if state.failure.fail_stage {
            return Err(LegacyAppError::new(
                ErrorCode::UciStageFailed,
                ErrorStage::Stage,
                "injected stage failure",
            ));
        }
        let staged = state.sessions.get_mut(session).ok_or_else(|| {
            LegacyAppError::new(
                ErrorCode::RpcdSessionLost,
                ErrorStage::Stage,
                "session not found",
            )
        })?;
        for target in targets {
            if !staged.contains_key(target) {
                return Err(LegacyAppError::new(
                    ErrorCode::TargetMissing,
                    ErrorStage::Stage,
                    format!("missing target {target}"),
                ));
            }
            let mut target_config = config.clone();
            target_config.targets = vec![target.clone()];
            staged.insert(target.clone(), target_config);
        }
        state.roaming_sessions.insert(session.to_owned(), roaming);
        Ok(())
    }

    fn read_roaming_config(
        &self,
        targets: &[String],
        session: Option<&str>,
    ) -> Result<AppliedRoamingConfig, LegacyAppError> {
        let state = self.state.lock().expect("memory backend poisoned");
        let roaming = session
            .and_then(|id| state.roaming_sessions.get(id))
            .copied()
            .unwrap_or(state.roaming_committed);
        let configs = session
            .and_then(|id| state.sessions.get(id))
            .unwrap_or(&state.committed);
        let first = targets
            .first()
            .and_then(|target| configs.get(target))
            .ok_or_else(|| {
                LegacyAppError::new(
                    ErrorCode::TargetMissing,
                    ErrorStage::Verify,
                    "managed Wi-Fi target is missing",
                )
            })?;

        Ok(crate::domain::compile_applied_roaming(
            roaming,
            &first.ssid,
            &first.encryption,
            targets,
        ))
    }

    fn read_roaming_runtime(
        &self,
        targets: &[String],
        _ssid: &str,
        _roaming: RoamingConfig,
    ) -> RoamingRuntime {
        RoamingRuntime {
            available: true,
            local_bss: targets.len().try_into().unwrap_or(u32::MAX),
            remote_bss: 0,
            status: RoamingRuntimeStatus::Ready,
            error: None,
        }
    }

    fn read_wan_config(&self, session: Option<&str>) -> Result<WanDesired, LegacyAppError> {
        self.mem_read_wan_config(session)
    }

    fn stage_wan_config(&self, session: &str, config: &WanDesired) -> Result<(), LegacyAppError> {
        self.mem_stage_wan_config(session, config)
    }

    fn read_wan_runtime_status(&self) -> Result<WanPublicState, LegacyAppError> {
        self.mem_read_wan_runtime_status()
    }

    fn revert_staged(&self, session: &str) -> Result<(), LegacyAppError> {
        let mut state = self.state.lock().expect("memory backend poisoned");
        let committed = state.committed.clone();
        state.sessions.insert(session.to_owned(), committed);
        let roaming = state.roaming_committed;
        state.roaming_sessions.insert(session.to_owned(), roaming);
        let wan_committed = state.wan_committed.clone();
        state.wan_sessions.insert(session.to_owned(), wan_committed);
        Ok(())
    }

    fn apply(&self, session: &str, _rollback_timeout_secs: u32) -> Result<(), LegacyAppError> {
        let mut state = self.state.lock().expect("memory backend poisoned");
        if state.failure.fail_apply {
            return Err(LegacyAppError::new(
                ErrorCode::UciApplyFailed,
                ErrorStage::Apply,
                "injected apply failure",
            ));
        }
        let staged = state.sessions.get(session).cloned().ok_or_else(|| {
            LegacyAppError::new(
                ErrorCode::RpcdSessionLost,
                ErrorStage::Apply,
                "session not found",
            )
        })?;
        let snapshot = state.committed.clone();
        state
            .rollback_snapshots
            .insert(session.to_owned(), snapshot);
        let roaming_snapshot = state.roaming_committed;
        state
            .roaming_rollback_snapshots
            .insert(session.to_owned(), roaming_snapshot);
        state.roaming_committed = state
            .roaming_sessions
            .get(session)
            .copied()
            .unwrap_or_default();
        state.committed = staged;

        let wan_staged = state
            .wan_sessions
            .get(session)
            .cloned()
            .unwrap_or_else(|| state.wan_committed.clone());
        let wan_snapshot = state.wan_committed.clone();
        state
            .wan_rollback_snapshots
            .insert(session.to_owned(), wan_snapshot);
        if wan_staged.present {
            state.wan_runtime.present = true;
            state.wan_runtime.proto = wan_staged.proto;
            state.wan_runtime.status = WanStatus::Connected;
            state.wan_runtime.qos = wan_staged.qos.clone();
        } else {
            state.wan_runtime.present = false;
            state.wan_runtime.proto = WanProtocol::None;
            state.wan_runtime.status = WanStatus::NotConfigured;
            state.wan_runtime.qos = None;
        }
        state.wan_committed = wan_staged;
        Ok(())
    }

    fn confirm(&self, session: &str) -> Result<(), LegacyAppError> {
        let mut state = self.state.lock().expect("memory backend poisoned");
        if state.failure.fail_confirm {
            return Err(LegacyAppError::new(
                ErrorCode::ConfirmFailed,
                ErrorStage::Confirm,
                "injected confirm failure",
            ));
        }
        state.rollback_snapshots.remove(session);
        state.roaming_rollback_snapshots.remove(session);
        state.wan_rollback_snapshots.remove(session);
        Ok(())
    }

    fn rollback(&self, session: &str) -> Result<(), LegacyAppError> {
        let mut state = self.state.lock().expect("memory backend poisoned");
        if state.failure.fail_rollback {
            return Err(LegacyAppError::new(
                ErrorCode::RollbackFailed,
                ErrorStage::Rollback,
                "injected rollback failure",
            ));
        }
        if let Some(snapshot) = state.rollback_snapshots.remove(session) {
            state.committed = snapshot.clone();
            state.sessions.insert(session.to_owned(), snapshot);
        }
        if let Some(snapshot) = state.roaming_rollback_snapshots.remove(session) {
            state.roaming_committed = snapshot;
            state.roaming_sessions.insert(session.to_owned(), snapshot);
        }
        if let Some(wan_snapshot) = state.wan_rollback_snapshots.remove(session) {
            if wan_snapshot.present {
                state.wan_runtime.present = true;
                state.wan_runtime.proto = wan_snapshot.proto;
                state.wan_runtime.status = WanStatus::Connected;
                state.wan_runtime.qos = wan_snapshot.qos.clone();
            } else {
                state.wan_runtime.present = false;
                state.wan_runtime.proto = WanProtocol::None;
                state.wan_runtime.status = WanStatus::NotConfigured;
                state.wan_runtime.qos = None;
            }
            state.wan_committed = wan_snapshot.clone();
            state.wan_sessions.insert(session.to_owned(), wan_snapshot);
        }
        Ok(())
    }

    fn runtime_healthy(&self, _targets: &[String], _ssid: &str) -> Result<bool, LegacyAppError> {
        let state = self.state.lock().expect("memory backend poisoned");
        if state.failure.runtime_unhealthy {
            return Ok(false);
        }
        if state.failure.fail_candidate_verify && !state.rollback_snapshots.is_empty() {
            return Ok(false);
        }
        Ok(true)
    }

    fn reload_wireless_runtime(&self) -> Result<(), LegacyAppError> {
        self.state
            .lock()
            .expect("memory backend poisoned")
            .failure
            .runtime_unhealthy = false;
        Ok(())
    }

    fn ports_list(&self) -> Result<Vec<crate::domain::ports::PhysicalPort>, LegacyAppError> {
        Ok(super::mock::mock_ports_info())
    }

    fn read_traffic_counters(
        &self,
    ) -> Result<crate::domain::traffic::TrafficCounters, LegacyAppError> {
        Ok(self
            .state
            .lock()
            .expect("memory backend poisoned")
            .traffic_counters)
    }

    fn read_switch_state(&self) -> Result<crate::domain::ports::SwitchState, LegacyAppError> {
        Ok(self
            .state
            .lock()
            .expect("memory backend poisoned")
            .switch_state)
    }

    fn set_hw_offload(
        &self,
        enabled: bool,
    ) -> Result<crate::domain::ports::SwitchState, LegacyAppError> {
        let mut state = self.state.lock().expect("memory backend poisoned");
        if enabled && !state.switch_state.hw_offload.available {
            return Err(LegacyAppError::new(
                ErrorCode::InvalidArgument,
                ErrorStage::Validate,
                "hardware flow offload is unavailable",
            ));
        }
        if state.failure.fail_switch_reload {
            return Err(LegacyAppError::new(
                ErrorCode::UciApplyFailed,
                ErrorStage::Apply,
                "injected firewall reload failure",
            ));
        }
        state.switch_state.hw_offload.enabled = enabled;
        Ok(state.switch_state)
    }

    fn read_system_info(&self) -> Result<crate::domain::system::SystemInfo, LegacyAppError> {
        Ok(super::mock::mock_system_info())
    }

    fn read_system_runtime(&self) -> Result<crate::domain::system::SystemRuntime, LegacyAppError> {
        Ok(super::mock::mock_system_runtime())
    }

    fn read_devices(
        &self,
        _extenders: &[crate::domain::extender::KnownExtender],
        _extender_clients: &std::collections::HashMap<
            String,
            Vec<crate::domain::extender::ExtenderClient>,
        >,
    ) -> Result<Vec<crate::domain::device::Device>, LegacyAppError> {
        Ok(super::mock::mock_devices())
    }

    fn write_static_lease(
        &self,
        _mac: &str,
        _ip: &str,
        _hostname: Option<&str>,
    ) -> Result<(), LegacyAppError> {
        Ok(())
    }
    fn delete_static_lease(&self, _mac: &str) -> Result<(), LegacyAppError> {
        Ok(())
    }
    fn sync_port_forwards(
        &self,
        _registered_devices: &[crate::domain::device::RegisteredDevice],
        _current_devices: &[crate::domain::device::Device],
    ) -> Result<(), LegacyAppError> {
        Ok(())
    }
    fn read_dns_config(&self) -> Result<crate::domain::DnsConfig, LegacyAppError> {
        Ok(crate::domain::DnsConfig::default())
    }
    fn write_dns_config(&self, _cfg: &crate::domain::DnsConfig) -> Result<(), LegacyAppError> {
        Ok(())
    }
}
