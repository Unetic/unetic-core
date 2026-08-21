use std::collections::BTreeMap;

use super::MemoryBackend;
use crate::{
    domain::errors::{DomainError, ErrorCode, ErrorStage},
    domain::{
        DiscoveredWan, DiscoveredWifi, WanDesired, WanProtocol, WanPublicState, WanStatus,
        WifiNetworkConfig,
    },
    infrastructure::backend::RouterBackend,
};

impl RouterBackend for MemoryBackend {
    fn discover_primary_wifi(&self) -> Result<DiscoveredWifi, DomainError> {
        let state = self.state.lock().expect("memory backend poisoned");
        let mut configs = state.committed.values();
        let Some(first) = configs.next() else {
            return Err(DomainError::new(
                ErrorCode::AmbiguousWifiConfig,
                ErrorStage::Bootstrap,
                "no AP targets found",
            ));
        };
        if configs
            .any(|c| c.ssid != first.ssid || c.encryption != first.encryption || c.key != first.key)
        {
            return Err(DomainError::new(
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
        })
    }

    fn discover_primary_wan(&self) -> Result<DiscoveredWan, DomainError> {
        self.mem_discover_primary_wan()
    }

    fn create_session(&self) -> Result<String, DomainError> {
        let mut state = self.state.lock().expect("memory backend poisoned");
        let sid = format!("memory-session-{}", state.next_session);
        state.next_session += 1;
        let committed = state.committed.clone();
        state.sessions.insert(sid.clone(), committed);
        let wan_committed = state.wan_committed.clone();
        state.wan_sessions.insert(sid.clone(), wan_committed);
        Ok(sid)
    }

    fn destroy_session(&self, session: &str) -> Result<(), DomainError> {
        let mut state = self.state.lock().expect("memory backend poisoned");
        state.sessions.remove(session);
        state.wan_sessions.remove(session);
        state.rollback_snapshots.remove(session);
        state.wan_rollback_snapshots.remove(session);
        Ok(())
    }

    fn read_wifi_configs(
        &self,
        targets: &[String],
        session: Option<&str>,
    ) -> Result<BTreeMap<String, WifiNetworkConfig>, DomainError> {
        let state = self.state.lock().expect("memory backend poisoned");
        let source = session
            .and_then(|sid| state.sessions.get(sid))
            .unwrap_or(&state.committed);
        targets
            .iter()
            .map(|target| {
                source.get(target).cloned().map_or_else(
                    || {
                        Err(DomainError::new(
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
    ) -> Result<(), DomainError> {
        let mut state = self.state.lock().expect("memory backend poisoned");
        if state.failure.fail_stage {
            return Err(DomainError::new(
                ErrorCode::UciStageFailed,
                ErrorStage::Stage,
                "injected stage failure",
            ));
        }
        let staged = state.sessions.get_mut(session).ok_or_else(|| {
            DomainError::new(
                ErrorCode::RpcdSessionLost,
                ErrorStage::Stage,
                "session not found",
            )
        })?;
        for target in targets {
            if !staged.contains_key(target) {
                return Err(DomainError::new(
                    ErrorCode::TargetMissing,
                    ErrorStage::Stage,
                    format!("missing target {target}"),
                ));
            }
            let mut target_config = config.clone();
            target_config.targets = vec![target.clone()];
            staged.insert(target.clone(), target_config);
        }
        Ok(())
    }

    fn read_wan_config(&self, session: Option<&str>) -> Result<WanDesired, DomainError> {
        self.mem_read_wan_config(session)
    }

    fn stage_wan_config(&self, session: &str, config: &WanDesired) -> Result<(), DomainError> {
        self.mem_stage_wan_config(session, config)
    }

    fn read_wan_runtime_status(&self) -> Result<WanPublicState, DomainError> {
        self.mem_read_wan_runtime_status()
    }

    fn revert_staged(&self, session: &str) -> Result<(), DomainError> {
        let mut state = self.state.lock().expect("memory backend poisoned");
        let committed = state.committed.clone();
        state.sessions.insert(session.to_owned(), committed);
        let wan_committed = state.wan_committed.clone();
        state.wan_sessions.insert(session.to_owned(), wan_committed);
        Ok(())
    }

    fn apply(&self, session: &str, _rollback_timeout_secs: u32) -> Result<(), DomainError> {
        let mut state = self.state.lock().expect("memory backend poisoned");
        if state.failure.fail_apply {
            return Err(DomainError::new(
                ErrorCode::UciApplyFailed,
                ErrorStage::Apply,
                "injected apply failure",
            ));
        }
        let staged = state.sessions.get(session).cloned().ok_or_else(|| {
            DomainError::new(
                ErrorCode::RpcdSessionLost,
                ErrorStage::Apply,
                "session not found",
            )
        })?;
        let snapshot = state.committed.clone();
        state
            .rollback_snapshots
            .insert(session.to_owned(), snapshot);
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
        } else {
            state.wan_runtime.present = false;
            state.wan_runtime.proto = WanProtocol::None;
            state.wan_runtime.status = WanStatus::NotConfigured;
        }
        state.wan_committed = wan_staged;
        Ok(())
    }

    fn confirm(&self, session: &str) -> Result<(), DomainError> {
        let mut state = self.state.lock().expect("memory backend poisoned");
        if state.failure.fail_confirm {
            return Err(DomainError::new(
                ErrorCode::ConfirmFailed,
                ErrorStage::Confirm,
                "injected confirm failure",
            ));
        }
        state.rollback_snapshots.remove(session);
        state.wan_rollback_snapshots.remove(session);
        Ok(())
    }

    fn rollback(&self, session: &str) -> Result<(), DomainError> {
        let mut state = self.state.lock().expect("memory backend poisoned");
        if state.failure.fail_rollback {
            return Err(DomainError::new(
                ErrorCode::RollbackFailed,
                ErrorStage::Rollback,
                "injected rollback failure",
            ));
        }
        if let Some(snapshot) = state.rollback_snapshots.remove(session) {
            state.committed = snapshot.clone();
            state.sessions.insert(session.to_owned(), snapshot);
        }
        if let Some(wan_snapshot) = state.wan_rollback_snapshots.remove(session) {
            if wan_snapshot.present {
                state.wan_runtime.present = true;
                state.wan_runtime.proto = wan_snapshot.proto;
                state.wan_runtime.status = WanStatus::Connected;
            } else {
                state.wan_runtime.present = false;
                state.wan_runtime.proto = WanProtocol::None;
                state.wan_runtime.status = WanStatus::NotConfigured;
            }
            state.wan_committed = wan_snapshot.clone();
            state.wan_sessions.insert(session.to_owned(), wan_snapshot);
        }
        Ok(())
    }

    fn runtime_healthy(&self, _targets: &[String], _ssid: &str) -> Result<bool, DomainError> {
        let state = self.state.lock().expect("memory backend poisoned");
        if state.failure.runtime_unhealthy {
            return Ok(false);
        }
        if state.failure.fail_candidate_verify && !state.rollback_snapshots.is_empty() {
            return Ok(false);
        }
        Ok(true)
    }

    fn reload_wireless_runtime(&self) -> Result<(), DomainError> {
        self.state
            .lock()
            .expect("memory backend poisoned")
            .failure
            .runtime_unhealthy = false;
        Ok(())
    }

    fn read_switch_info(&self) -> Result<crate::domain::switch::SwitchInfo, DomainError> {
        Ok(super::mock::mock_switch_info())
    }

    fn read_system_info(&self) -> Result<crate::domain::system::SystemInfo, DomainError> {
        Ok(super::mock::mock_system_info())
    }

    fn read_devices(&self) -> Result<Vec<crate::domain::device::Device>, DomainError> {
        Ok(super::mock::mock_devices())
    }
}
