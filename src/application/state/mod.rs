use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    sync::atomic::Ordering,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use serde_json::json;
use tracing::warn;

use crate::application::app::{App, Inner, PendingStateUpdate, StateTopic, StateUpdateBuffer};
use crate::{
    domain::errors::{ErrorCode, ErrorStage, LegacyAppError},
    domain::{
        DriftState, MaintenanceState, PublicState, WifiNetworkConfig, WifiPublicState, WifiStatus,
    },
};

const STATE_PUBLISH_INTERVAL_MILLIS: u64 = 1_000;
pub(crate) const STATE_PUBLISH_INTERVAL: Duration =
    Duration::from_millis(STATE_PUBLISH_INTERVAL_MILLIS);
const EARLY_FLUSH_MIN_INTERVAL: Duration = Duration::from_millis(STATE_PUBLISH_INTERVAL_MILLIS / 2);

impl App {
    pub(crate) fn publish(&self, topic: StateTopic) -> PublicState {
        let state = self.state();
        self.queue_state_update(topic, state.clone(), true);
        state
    }

    pub(crate) fn publish_system_runtime(&self) {
        self.queue_state_update(StateTopic::SystemRuntime, self.state(), false);
    }

    pub(crate) fn flush_state_update(&self) {
        let mut updates = self.state_updates.lock().expect("state updates poisoned");
        self.send_pending_state(&mut updates);
    }

    fn can_flush_early(&self, last_sent_at: &Option<Instant>) -> bool {
        last_sent_at.is_none_or(|sent_at| sent_at.elapsed() >= EARLY_FLUSH_MIN_INTERVAL)
    }

    pub(crate) fn queue_state_update(
        &self,
        topic: StateTopic,
        state: PublicState,
        allow_early_flush: bool,
    ) {
        let mut updates = self.state_updates.lock().expect("state updates poisoned");
        let repeats_pending_topic = updates
            .pending
            .as_ref()
            .is_some_and(|pending| pending.topics.contains(&topic));

        if allow_early_flush && repeats_pending_topic && self.can_flush_early(&updates.last_sent_at)
        {
            self.send_pending_state(&mut updates);
        }

        match updates.pending.as_mut() {
            Some(pending) => {
                pending.state = state;
                pending.topics.insert(topic);
            }
            None => {
                updates.pending = Some(PendingStateUpdate {
                    state,
                    topics: BTreeSet::from([topic]),
                });
            }
        }
    }

    fn send_pending_state(&self, updates: &mut StateUpdateBuffer) {
        let Some(mut pending) = updates.pending.take() else {
            return;
        };

        pending.state.event_seq = {
            let mut inner = self.inner.lock().expect("app state poisoned");
            inner.event_seq = inner.event_seq.saturating_add(1);
            inner.event_seq
        };
        let _ = self.event_tx.send(pending.state);
        updates.last_sent_at = Some(Instant::now());
    }

    pub(crate) fn next_operation_id(&self) -> String {
        let count = self.op_counter.fetch_add(1, Ordering::Relaxed);
        format!("op-{}-{count}", now_ms())
    }

    pub(crate) fn refresh_observed(&self) -> bool {
        let (targets, desired, roaming) = {
            let inner = self.inner.lock().expect("app state poisoned");
            (
                inner.config.wifi.primary.targets.clone(),
                inner.config.wifi.primary.clone(),
                inner.config.wifi.roaming,
            )
        };

        let wan_observation = self.backend.read_wan_runtime_status();
        let wan_changed = {
            let mut inner = self.inner.lock().expect("app state poisoned");
            match wan_observation {
                Ok(wan_status) => {
                    let changed = inner.wan != wan_status || inner.health.wan != "ok";
                    inner.wan = wan_status;
                    inner.health.wan = "ok".into();
                    changed
                }
                Err(error) => {
                    warn!(%error, "failed to observe WAN runtime");
                    let changed = inner.health.wan != "error"
                        || inner.last_system_error.as_ref() != Some(&error);
                    inner.health.wan = "error".into();
                    inner.last_system_error = Some(error);
                    changed
                }
            }
        };

        if targets.is_empty() {
            return wan_changed;
        }

        match self.backend.read_wifi_configs(&targets, None) {
            Ok(observed_configs) => {
                let observed_roaming = self.backend.read_roaming_config(&targets, None).ok();
                let roaming_runtime =
                    self.backend
                        .read_roaming_runtime(&targets, &desired.ssid, roaming);
                let (runtime, runtime_error) =
                    match self.backend.runtime_healthy(&targets, &desired.ssid) {
                        Ok(value) => (value, None),
                        Err(error) => {
                            warn!(%error, "failed to observe wireless runtime");
                            (false, Some(error))
                        }
                    };
                let mut inner = self.inner.lock().expect("app state poisoned");
                let wireless_health = if runtime { "ok" } else { "error" };
                let changed = inner.observed_configs != observed_configs
                    || inner.runtime_healthy != runtime
                    || inner.observed_roaming != observed_roaming
                    || inner.roaming_runtime != roaming_runtime
                    || inner.health.wireless != wireless_health
                    || wan_changed
                    || runtime_error
                        .as_ref()
                        .is_some_and(|error| inner.last_system_error.as_ref() != Some(error));
                inner.observed_configs = observed_configs;
                inner.observed_roaming = observed_roaming;
                inner.roaming_runtime = roaming_runtime;
                inner.runtime_healthy = runtime;
                inner.health.wireless = wireless_health.into();
                if let Some(error) = runtime_error {
                    inner.last_system_error = Some(error);
                }
                changed
            }
            Err(error) => {
                warn!(%error, "failed to observe wireless config");
                let roaming_runtime =
                    self.backend
                        .read_roaming_runtime(&targets, &desired.ssid, roaming);
                let mut inner = self.inner.lock().expect("app state poisoned");
                let changed = inner.health.wireless != "error"
                    || wan_changed
                    || inner.last_system_error.as_ref() != Some(&error);
                inner.health.wireless = "error".into();
                inner.observed_roaming = None;
                inner.roaming_runtime = roaming_runtime;
                inner.last_system_error = Some(error);
                changed
            }
        }
    }
}

pub(crate) fn snapshot(inner: &Inner) -> PublicState {
    let desired = &inner.config.wifi.primary;
    let mut drift_fields: Vec<String> = Vec::new();
    for target in &desired.targets {
        if let Some(obs) = inner.observed_configs.get(target) {
            if obs.ssid != desired.ssid {
                drift_fields.push(format!("wifi.primary.targets.{target}.ssid"));
            }
            if obs.encryption != desired.encryption {
                drift_fields.push(format!("wifi.primary.targets.{target}.encryption"));
            }
            if obs.key != desired.key {
                drift_fields.push(format!("wifi.primary.targets.{target}.key"));
            }
        } else {
            drift_fields.push(format!("wifi.primary.targets.{target}.ssid"));
        }
    }
    if !inner.runtime_healthy && !desired.targets.is_empty() {
        drift_fields.push("wifi.primary.runtime".into());
    }
    let expected_roaming = crate::domain::compile_applied_roaming(
        inner.config.wifi.roaming,
        &desired.ssid,
        &desired.encryption,
        &desired.targets,
    );
    if !desired.targets.is_empty() && inner.observed_roaming.as_ref() != Some(&expected_roaming) {
        drift_fields.push("wifi.roaming.policy".into());
    }
    let drifted = !drift_fields.is_empty();

    let wifi_status = if inner.active_operation.is_some() {
        WifiStatus::Applying
    } else if drifted {
        WifiStatus::Drifted
    } else if inner.observed_configs.is_empty() {
        WifiStatus::Unknown
    } else {
        WifiStatus::Synced
    };

    PublicState {
        core_version: env!("CARGO_PKG_VERSION").into(),
        boot_id: inner.boot_id.clone(),
        event_seq: inner.event_seq,
        revision: inner.config.revision,
        lifecycle: inner.lifecycle,
        maintenance: MaintenanceState {
            enabled: inner.maintenance,
            exiting: inner.maintenance_exiting,
            reason: inner.maintenance_reason.clone(),
        },
        wifi: WifiPublicState {
            ssid: desired.ssid.clone(),
            encryption: desired.encryption.clone(),
            key: desired.key.clone(),
            targets: desired.targets.clone(),
            observed: inner
                .observed_configs
                .iter()
                .map(|(t, c)| (t.clone(), c.ssid.clone()))
                .collect(),
            status: wifi_status,
            roaming: inner.config.wifi.roaming,
            roaming_runtime: inner.roaming_runtime.clone(),
            backhaul: inner.config.wifi.backhaul.clone(),
            radio_channels: inner.config.wifi.radio_channels.clone(),
        },
        wan: inner.wan.clone(),
        active_operation: inner.active_operation.clone(),
        last_user_operation: inner.last_user_operation.clone(),
        last_system_error: inner.last_system_error.clone(),
        drift: DriftState {
            detected: drifted,
            fields: drift_fields,
        },
        health: inner.health.clone(),
        system: crate::domain::system::SystemState {
            info: inner.system_info.clone(),
            runtime: inner.system_runtime.clone(),
        },
        registered_devices: inner.config.registered_devices.clone(),
        devices: inner.devices.devices(),
        dns: inner.config.dns.clone(),
        traffic: inner.traffic.clone(),
        ddns_config: inner.config.ddns.clone(),
        ddns_status: inner.ddns_status.clone(),
        extenders: inner
            .config
            .extenders
            .iter()
            .map(crate::domain::extender::PublicExtender::from)
            .collect(),
        extender_ports: inner.extender_ports.clone(),
        pending_extenders: inner.pending_extenders.clone(),
        extender_pairing_status: inner.extender_pairing_status.clone(),
        extender_clients: inner.extender_clients.clone(),
        latest_scans: inner.latest_scans.clone(),
    }
}

pub fn all_equal_config(
    observed: &BTreeMap<String, WifiNetworkConfig>,
    targets: &[String],
    expected: &WifiNetworkConfig,
) -> bool {
    !targets.is_empty()
        && targets.iter().all(|target| {
            observed.get(target).is_some_and(|cfg| {
                cfg.ssid == expected.ssid
                    && cfg.encryption == expected.encryption
                    && cfg.key == expected.key
            })
        })
}

pub fn validate_ssid(ssid: &str) -> Result<(), LegacyAppError> {
    if ssid.is_empty() {
        return Err(LegacyAppError::new(
            ErrorCode::InvalidArgument,
            ErrorStage::Validate,
            "SSID must not be empty",
        ));
    }
    if ssid.len() > 32 {
        return Err(LegacyAppError::new(
            ErrorCode::InvalidArgument,
            ErrorStage::Validate,
            "SSID must be at most 32 UTF-8 bytes",
        )
        .details(json!({"bytes": ssid.len()})));
    }
    if ssid.contains('\0') {
        return Err(LegacyAppError::new(
            ErrorCode::InvalidArgument,
            ErrorStage::Validate,
            "SSID must not contain NUL",
        ));
    }
    Ok(())
}

pub fn validate_wifi_config(
    ssid: &str,
    encryption: &str,
    key: Option<&str>,
) -> Result<(), LegacyAppError> {
    validate_ssid(ssid)?;
    if encryption != "none" {
        let Some(key) = key else {
            return Err(LegacyAppError::new(
                ErrorCode::InvalidArgument,
                ErrorStage::Validate,
                "key must be provided when encryption is not 'none'",
            ));
        };
        if key.len() < 8 || key.len() > 63 {
            return Err(LegacyAppError::new(
                ErrorCode::InvalidArgument,
                ErrorStage::Validate,
                "key must be between 8 and 63 characters long",
            ));
        }
        if key.contains('\0') {
            return Err(LegacyAppError::new(
                ErrorCode::InvalidArgument,
                ErrorStage::Validate,
                "key must not contain NUL",
            ));
        }
    }
    Ok(())
}

pub fn validate_mesh_backhaul_config(
    backhaul: &crate::domain::wifi::MeshBackhaulConfig,
    available_targets: &[String],
    radio_channels: &[crate::domain::wifi::RadioChannelConfig],
) -> Result<(), LegacyAppError> {
    if !backhaul.enabled {
        return Ok(());
    }

    if available_targets.len() < 2 {
        return Err(LegacyAppError::new(
            ErrorCode::InvalidArgument,
            ErrorStage::Validate,
            "Dual-radio hardware (at least 2 radios) is required for dedicated wireless backhaul",
        ));
    }

    if backhaul.backhaul_target == backhaul.client_target {
        return Err(LegacyAppError::new(
            ErrorCode::InvalidArgument,
            ErrorStage::Validate,
            "Backhaul radio chip and Client access radio chip must be different",
        ));
    }

    if !available_targets.contains(&backhaul.backhaul_target) {
        return Err(LegacyAppError::new(
            ErrorCode::InvalidArgument,
            ErrorStage::Validate,
            format!(
                "Backhaul target radio '{}' does not exist in available radios",
                backhaul.backhaul_target
            ),
        ));
    }

    if !available_targets.contains(&backhaul.client_target) {
        return Err(LegacyAppError::new(
            ErrorCode::InvalidArgument,
            ErrorStage::Validate,
            format!(
                "Client target radio '{}' does not exist in available radios",
                backhaul.client_target
            ),
        ));
    }

    let b_chan = radio_channels
        .iter()
        .find(|rc| rc.target == backhaul.backhaul_target)
        .map(|rc| rc.channel);
    let c_chan = radio_channels
        .iter()
        .find(|rc| rc.target == backhaul.client_target)
        .map(|rc| rc.channel);

    if let (Some(b), Some(c)) = (b_chan, c_chan) {
        if b > 0 && c > 0 && b == c {
            return Err(LegacyAppError::new(
                ErrorCode::InvalidArgument,
                ErrorStage::Validate,
                format!(
                    "Backhaul radio ({}) and Client radio ({}) must operate on different channels",
                    backhaul.backhaul_target, backhaul.client_target
                ),
            ));
        }
    }

    Ok(())
}

pub fn generate_id(prefix: &str) -> String {
    if let Ok(value) = fs::read_to_string("/proc/sys/kernel/random/uuid") {
        return format!("{prefix}-{}", value.trim());
    }
    format!("{prefix}-{}-{}", std::process::id(), now_ms())
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            duration.as_millis().try_into().unwrap_or(u64::MAX)
        })
}

#[cfg(test)]
mod tests;
