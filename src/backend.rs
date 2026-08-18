use std::{
    collections::{BTreeMap, HashMap},
    sync::Mutex,
};

use crate::{
    errors::{DomainError, ErrorCode, ErrorStage},
    model::DiscoveredWifi,
};

pub trait RouterBackend: Send + Sync {
    fn discover_primary_wifi(&self) -> Result<DiscoveredWifi, DomainError>;
    fn create_session(&self) -> Result<String, DomainError>;
    fn read_ssids(
        &self,
        targets: &[String],
        session: Option<&str>,
    ) -> Result<BTreeMap<String, String>, DomainError>;
    fn stage_ssid(&self, session: &str, targets: &[String], ssid: &str) -> Result<(), DomainError>;
    fn revert_staged(&self, session: &str) -> Result<(), DomainError>;
    fn apply(&self, session: &str, rollback_timeout_secs: u32) -> Result<(), DomainError>;
    fn confirm(&self, session: &str) -> Result<(), DomainError>;
    fn rollback(&self, session: &str) -> Result<(), DomainError>;
    fn runtime_healthy(&self, targets: &[String], ssid: &str) -> Result<bool, DomainError>;
    fn reload_wireless_runtime(&self) -> Result<(), DomainError>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct FailurePlan {
    pub fail_stage: bool,
    pub fail_apply: bool,
    pub fail_confirm: bool,
    pub fail_rollback: bool,
    pub runtime_unhealthy: bool,
}

#[derive(Debug)]
struct MemoryState {
    committed: BTreeMap<String, String>,
    sessions: HashMap<String, BTreeMap<String, String>>,
    rollback_snapshots: HashMap<String, BTreeMap<String, String>>,
    next_session: u64,
    failure: FailurePlan,
}

#[derive(Debug)]
pub struct MemoryBackend {
    state: Mutex<MemoryState>,
}

impl MemoryBackend {
    #[must_use]
    pub fn new(ssid: &str, targets: &[&str]) -> Self {
        let committed = targets
            .iter()
            .map(|target| ((*target).to_owned(), ssid.to_owned()))
            .collect();
        Self {
            state: Mutex::new(MemoryState {
                committed,
                sessions: HashMap::new(),
                rollback_snapshots: HashMap::new(),
                next_session: 1,
                failure: FailurePlan::default(),
            }),
        }
    }

    pub fn set_failure_plan(&self, failure: FailurePlan) {
        self.state.lock().expect("memory backend poisoned").failure = failure;
    }

    #[must_use]
    pub fn committed_ssids(&self) -> BTreeMap<String, String> {
        self.state
            .lock()
            .expect("memory backend poisoned")
            .committed
            .clone()
    }

    pub fn external_set(&self, target: &str, ssid: &str) {
        self.state
            .lock()
            .expect("memory backend poisoned")
            .committed
            .insert(target.to_owned(), ssid.to_owned());
    }
}

impl RouterBackend for MemoryBackend {
    fn discover_primary_wifi(&self) -> Result<DiscoveredWifi, DomainError> {
        let state = self.state.lock().expect("memory backend poisoned");
        let mut ssids = state.committed.values();
        let Some(first) = ssids.next() else {
            return Err(DomainError::new(
                ErrorCode::AmbiguousWifiConfig,
                ErrorStage::Bootstrap,
                "no AP targets found",
            ));
        };
        if ssids.any(|ssid| ssid != first) {
            return Err(DomainError::new(
                ErrorCode::AmbiguousWifiConfig,
                ErrorStage::Bootstrap,
                "managed APs do not share one SSID",
            ));
        }
        Ok(DiscoveredWifi {
            ssid: first.clone(),
            targets: state.committed.keys().cloned().collect(),
        })
    }

    fn create_session(&self) -> Result<String, DomainError> {
        let mut state = self.state.lock().expect("memory backend poisoned");
        let sid = format!("memory-session-{}", state.next_session);
        state.next_session += 1;
        let committed = state.committed.clone();
        state.sessions.insert(sid.clone(), committed);
        Ok(sid)
    }

    fn read_ssids(
        &self,
        targets: &[String],
        session: Option<&str>,
    ) -> Result<BTreeMap<String, String>, DomainError> {
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
                    |ssid| Ok((target.clone(), ssid)),
                )
            })
            .collect()
    }

    fn stage_ssid(&self, session: &str, targets: &[String], ssid: &str) -> Result<(), DomainError> {
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
            staged.insert(target.clone(), ssid.to_owned());
        }
        Ok(())
    }

    fn revert_staged(&self, session: &str) -> Result<(), DomainError> {
        let mut state = self.state.lock().expect("memory backend poisoned");
        let committed = state.committed.clone();
        state.sessions.insert(session.to_owned(), committed);
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
        Ok(())
    }

    fn runtime_healthy(&self, _targets: &[String], _ssid: &str) -> Result<bool, DomainError> {
        Ok(!self
            .state
            .lock()
            .expect("memory backend poisoned")
            .failure
            .runtime_unhealthy)
    }

    fn reload_wireless_runtime(&self) -> Result<(), DomainError> {
        self.state
            .lock()
            .expect("memory backend poisoned")
            .failure
            .runtime_unhealthy = false;
        Ok(())
    }
}
