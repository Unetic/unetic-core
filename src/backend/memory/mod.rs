use std::{
    collections::{BTreeMap, HashMap},
    sync::Mutex,
};

use crate::{
    backend::RouterBackend,
    errors::{DomainError, ErrorCode, ErrorStage},
    model::{DiscoveredWan, DiscoveredWifi, WanDesired, WanProtocol, WanPublicState, WanStatus},
};

mod wan;

#[derive(Debug, Clone, Copy, Default)]
pub struct FailurePlan {
    pub fail_stage: bool,
    pub fail_apply: bool,
    pub fail_confirm: bool,
    pub fail_rollback: bool,
    pub fail_candidate_verify: bool,
    pub runtime_unhealthy: bool,
}

#[derive(Debug)]
pub(crate) struct MemoryState {
    pub(crate) committed: BTreeMap<String, String>,
    pub(crate) wan_committed: WanDesired,
    pub(crate) sessions: HashMap<String, BTreeMap<String, String>>,
    pub(crate) wan_sessions: HashMap<String, WanDesired>,
    pub(crate) rollback_snapshots: HashMap<String, BTreeMap<String, String>>,
    pub(crate) wan_rollback_snapshots: HashMap<String, WanDesired>,
    pub(crate) wan_runtime: WanPublicState,
    pub(crate) next_session: u64,
    pub(crate) failure: FailurePlan,
}

#[derive(Debug)]
pub struct MemoryBackend {
    pub(crate) state: Mutex<MemoryState>,
}

impl MemoryBackend {
    #[must_use]
    pub fn new(ssid: &str, targets: &[&str]) -> Self {
        Self::with_wan(
            ssid,
            targets,
            WanDesired {
                present: true,
                proto: WanProtocol::Dhcp,
                device: Some("eth1".into()),
                custom_mac: None,
                custom_mtu: None,
                custom_dns: Vec::new(),
                static_config: None,
                pppoe_config: None,
            },
        )
    }

    #[must_use]
    pub fn with_wan(ssid: &str, targets: &[&str], wan: WanDesired) -> Self {
        let committed = targets
            .iter()
            .map(|target| ((*target).to_owned(), ssid.to_owned()))
            .collect();
        let wan_runtime = WanPublicState {
            present: wan.present,
            proto: wan.proto,
            status: if wan.present {
                WanStatus::Connected
            } else {
                WanStatus::NotConfigured
            },
            device: wan.device.clone(),
            ip_address: if wan.present {
                Some("203.0.113.10".into())
            } else {
                None
            },
            netmask: if wan.present {
                Some("255.255.255.0".into())
            } else {
                None
            },
            gateway: if wan.present {
                Some("203.0.113.1".into())
            } else {
                None
            },
            dns: if wan.present {
                vec!["1.1.1.1".into(), "1.0.0.1".into()]
            } else {
                Vec::new()
            },
            mac_address: Some("00:11:22:33:44:55".into()),
            uptime_secs: 1200,
            error_reason: None,
        };
        Self {
            state: Mutex::new(MemoryState {
                committed,
                wan_committed: wan,
                sessions: HashMap::new(),
                wan_sessions: HashMap::new(),
                rollback_snapshots: HashMap::new(),
                wan_rollback_snapshots: HashMap::new(),
                wan_runtime,
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

    fn read_switch_info(&self) -> Result<crate::switch::SwitchInfo, DomainError> {
        Ok(crate::switch::SwitchInfo {
            soc: crate::switch::SwitchSocInfo {
                model: "mt7531".into(),
                vendor: "MediaTek".into(),
                compatible: Some("mediatek,mt7531".into()),
                driver: Some("mt7530-mdio".into()),
                architecture: crate::switch::SwitchArchitecture::Dsa,
                tagging_protocol: Some("mtk".into()),
                ports: vec![
                    "lan1".into(),
                    "lan2".into(),
                    "lan3".into(),
                    "lan4".into(),
                    "wan".into(),
                ],
            },
            features: crate::switch::SwitchFeatures {
                l2_hw_switching: crate::switch::SwitchFeatureStatus::static_hw(true),
                l3_hw_flow_offload: crate::switch::SwitchFeatureStatus::new(true, true, true),
                l3_sw_flow_offload: crate::switch::SwitchFeatureStatus::new(true, true, true),
                vlan_filtering_8021q: crate::switch::SwitchFeatureStatus::new(true, false, true),
                port_isolation: crate::switch::SwitchFeatureStatus::new(true, false, true),
                hw_igmp_snooping: crate::switch::SwitchFeatureStatus::new(true, true, true),
                flow_control_8023x: crate::switch::SwitchFeatureStatus::new(true, false, true),
                eee_8023az: crate::switch::SwitchFeatureStatus::new(true, false, true),
                stp_rstp: crate::switch::SwitchFeatureStatus::new(true, false, true),
                mirroring_span: crate::switch::SwitchFeatureStatus::new(true, false, true),
                jumbo_frames: crate::switch::SwitchFeatureStatus::new(true, false, true),
                link_aggregation_lag: crate::switch::SwitchFeatureStatus::new(true, false, true),
                tdr_cable_diag: crate::switch::SwitchFeatureStatus::static_hw(true),
                hardware_stats: crate::switch::SwitchFeatureStatus::static_hw(true),
            },
        })
    }
}
