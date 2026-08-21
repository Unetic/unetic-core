use std::{
    collections::BTreeMap,
    fs,
    sync::atomic::Ordering,
    time::{SystemTime, UNIX_EPOCH},
};

use serde_json::json;
use tracing::warn;

use super::{App, Inner};
use crate::{
    errors::{DomainError, ErrorCode, ErrorStage},
    model::{API_VERSION, DriftState, MaintenanceState, PublicState, WifiPublicState, WifiStatus},
};

impl App {
    pub(crate) fn publish(&self) -> PublicState {
        let state = {
            let mut inner = self.inner.lock().expect("app state poisoned");
            inner.event_seq = inner.event_seq.saturating_add(1);
            snapshot(&inner)
        };
        let _ = self.event_tx.send(state.clone());
        state
    }

    pub(crate) fn next_operation_id(&self) -> String {
        let count = self.op_counter.fetch_add(1, Ordering::Relaxed);
        format!("op-{}-{count}", now_ms())
    }

    pub(crate) fn refresh_observed(&self) -> bool {
        let (targets, desired) = {
            let inner = self.inner.lock().expect("app state poisoned");
            (
                inner.config.wifi.primary.targets.clone(),
                inner.config.wifi.primary.ssid.clone(),
            )
        };

        let wan_status = self.backend.read_wan_runtime_status().unwrap_or_default();
        let mut wan_changed = false;
        {
            let mut inner = self.inner.lock().expect("app state poisoned");
            if inner.wan != wan_status {
                inner.wan = wan_status;
                wan_changed = true;
            }
        }

        if targets.is_empty() {
            return wan_changed;
        }

        match self.backend.read_ssids(&targets, None) {
            Ok(observed) => {
                let (runtime, runtime_error) =
                    match self.backend.runtime_healthy(&targets, &desired) {
                        Ok(value) => (value, None),
                        Err(error) => {
                            warn!(%error, "failed to observe wireless runtime");
                            (false, Some(error))
                        }
                    };
                let mut inner = self.inner.lock().expect("app state poisoned");
                let wireless_health = if runtime { "ok" } else { "error" };
                let changed = inner.observed != observed
                    || inner.runtime_healthy != runtime
                    || inner.health.wireless != wireless_health
                    || wan_changed
                    || runtime_error
                        .as_ref()
                        .is_some_and(|error| inner.last_system_error.as_ref() != Some(error));
                inner.observed = observed;
                inner.runtime_healthy = runtime;
                inner.health.wireless = wireless_health.into();
                if let Some(error) = runtime_error {
                    inner.last_system_error = Some(error);
                }
                changed
            }
            Err(error) => {
                warn!(%error, "failed to observe wireless config");
                let mut inner = self.inner.lock().expect("app state poisoned");
                let changed = inner.health.wireless != "error"
                    || wan_changed
                    || inner.last_system_error.as_ref() != Some(&error);
                inner.health.wireless = "error".into();
                inner.last_system_error = Some(error);
                changed
            }
        }
    }
}

pub(crate) fn snapshot(inner: &Inner) -> PublicState {
    let desired = &inner.config.wifi.primary;
    let mut drift_fields: Vec<String> = desired
        .targets
        .iter()
        .filter(|target| inner.observed.get(*target) != Some(&desired.ssid))
        .map(|target| format!("wifi.primary.targets.{target}.ssid"))
        .collect();
    if !inner.runtime_healthy && !desired.targets.is_empty() {
        drift_fields.push("wifi.primary.runtime".into());
    }
    let drifted = !drift_fields.is_empty();

    let wifi_status = if inner.active_operation.is_some() {
        WifiStatus::Applying
    } else if drifted {
        WifiStatus::Drifted
    } else if inner.observed.is_empty() {
        WifiStatus::Unknown
    } else {
        WifiStatus::Synced
    };

    PublicState {
        api_version: API_VERSION,
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
            targets: desired.targets.clone(),
            observed: inner.observed.clone(),
            status: wifi_status,
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
    }
}

pub(crate) fn all_equal(
    observed: &BTreeMap<String, String>,
    targets: &[String],
    expected: &str,
) -> bool {
    !targets.is_empty()
        && targets
            .iter()
            .all(|target| observed.get(target).is_some_and(|ssid| ssid == expected))
}

pub(crate) fn validate_ssid(ssid: &str) -> Result<(), DomainError> {
    if ssid.is_empty() {
        return Err(DomainError::new(
            ErrorCode::InvalidArgument,
            ErrorStage::Validate,
            "SSID must not be empty",
        ));
    }
    if ssid.len() > 32 {
        return Err(DomainError::new(
            ErrorCode::InvalidArgument,
            ErrorStage::Validate,
            "SSID must be at most 32 UTF-8 bytes",
        )
        .details(json!({"bytes": ssid.len()})));
    }
    if ssid.contains('\0') {
        return Err(DomainError::new(
            ErrorCode::InvalidArgument,
            ErrorStage::Validate,
            "SSID must not contain NUL",
        ));
    }
    Ok(())
}

pub(crate) fn generate_id(prefix: &str) -> String {
    if let Ok(value) = fs::read_to_string("/proc/sys/kernel/random/uuid") {
        return format!("{prefix}-{}", value.trim());
    }
    format!("{prefix}-{}-{}", std::process::id(), now_ms())
}

pub(crate) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            duration.as_millis().try_into().unwrap_or(u64::MAX)
        })
}
