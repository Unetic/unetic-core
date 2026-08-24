use std::{
    sync::{Arc, atomic::Ordering},
    thread,
    time::{Duration, Instant},
};

use tracing::warn;

use super::{App, Inner, StateTopic};
use crate::application::state::all_equal_config;
use crate::{
    application::transaction::ChangeContext,
    application::wan::WanChangeContext,
    domain::errors::{ErrorCode, ErrorStage, LegacyAppError},
    domain::{Lifecycle, OperationSource, OperationStatus},
};

enum Repair {
    Config(Box<ChangeContext>),
    WanConfig(Box<WanChangeContext>),
    Runtime { targets: Vec<String>, ssid: String },
}

impl App {
    pub fn start_background(self: &Arc<Self>) {
        let app = Arc::clone(self);
        thread::Builder::new()
            .name("unetic-reconcile".into())
            .spawn(move || app.reconcile_loop())
            .expect("failed to spawn reconcile thread");
    }

    fn reconcile_loop(self: &Arc<Self>) {
        let interval = self.timing.reconcile_interval;
        let mut next_tick = Instant::now();

        while !self.shutdown.load(Ordering::Relaxed) {
            self.reconcile_once();
            next_tick += interval;
            let now = Instant::now();
            if next_tick > now {
                thread::sleep(next_tick - now);
            } else {
                next_tick = now;
                thread::sleep(Duration::from_millis(50));
            }
        }
    }

    fn reconcile_once(self: &Arc<Self>) {
        let observation_changed = self.refresh_observed();
        let observed_wan = self.backend.read_wan_config(None).ok();

        let (repair, should_publish) = {
            let mut inner = self.inner.lock().expect("app state poisoned");
            if should_skip_reconcile(&inner) {
                (None, observation_changed)
            } else {
                evaluate_reconcile_state(
                    &mut inner,
                    self,
                    observed_wan.as_ref(),
                    observation_changed,
                )
            }
        };

        if let Some(repair) = repair {
            self.publish(StateTopic::Reconciliation);
            self.spawn_repair_task(repair);
        } else if should_publish {
            self.publish(StateTopic::Reconciliation);
        }
    }

    pub fn sync_registered_devices(&self) -> Result<(), LegacyAppError> {
        let (registered_devices, extenders, extender_clients) = {
            let inner = self.inner.lock().expect("app state poisoned");
            (
                inner.config.registered_devices.clone(),
                inner.config.extenders.clone(),
                inner.extender_clients.clone(),
            )
        };
        let devices = self.backend.read_devices(&extenders, &extender_clients)?;
        self.backend
            .sync_port_forwards(&registered_devices, &devices)
    }

    fn spawn_repair_task(self: &Arc<Self>, repair: Repair) {
        let app = Arc::clone(self);
        match repair {
            Repair::Config(context) => {
                if thread::Builder::new()
                    .name("unetic-reconcile".into())
                    .spawn(move || crate::application::transaction::run_change(app, *context))
                    .is_err()
                {
                    self.handle_reconcile_spawn_error("failed to start reconciliation worker");
                }
            }
            Repair::WanConfig(context) => {
                if thread::Builder::new()
                    .name("unetic-reconcile-wan".into())
                    .spawn(move || crate::application::wan::run_wan_change(app, *context))
                    .is_err()
                {
                    self.handle_reconcile_spawn_error("failed to start WAN reconciliation worker");
                }
            }
            Repair::Runtime { targets, ssid } => {
                let _ = thread::Builder::new()
                    .name("unetic-repair".into())
                    .spawn(move || app.run_runtime_repair(targets, ssid));
            }
        }
    }

    fn handle_reconcile_spawn_error(&self, message: &'static str) {
        let mut inner = self.inner.lock().expect("app state poisoned");
        inner.active_operation = None;
        inner.repair_failures = inner.repair_failures.saturating_add(1);
        let error = LegacyAppError::new(ErrorCode::ReconcileFailed, ErrorStage::Reconcile, message)
            .retryable(true);
        inner.last_system_error = Some(error);
        if inner.repair_failures >= 3 {
            inner.lifecycle = Lifecycle::Degraded;
            inner.health.core = "error".into();
        }
        drop(inner);
        self.publish(StateTopic::Reconciliation);
    }

    fn run_runtime_repair(&self, targets: Vec<String>, ssid: String) {
        let outcome = self.repair_runtime(&targets, &ssid);
        let mut inner = self.inner.lock().expect("app state poisoned");
        inner.active_operation = None;

        if let Err(error) = outcome {
            warn!(%error, "runtime-only wireless repair failed");
            inner.repair_failures = inner.repair_failures.saturating_add(1);
            inner.last_system_error = Some(error);
            if inner.repair_failures >= 3 {
                inner.lifecycle = Lifecycle::Degraded;
                inner.health.core = "error".into();
            }
            inner.health.wireless = "error".into();
        } else {
            inner.repair_failures = 0;
            if inner.lifecycle == Lifecycle::Degraded {
                inner.lifecycle = Lifecycle::Ready;
            }
            inner.health.core = "ok".into();
            inner.health.wireless = "ok".into();
            inner.runtime_healthy = true;
            inner.last_system_error = None;
        }

        drop(inner);
        self.publish(StateTopic::Reconciliation);
    }

    fn repair_runtime(&self, targets: &[String], ssid: &str) -> Result<(), LegacyAppError> {
        self.backend.reload_wireless_runtime()?;
        let deadline = Instant::now() + self.timing.rollback_verify_timeout;
        let mut successful_samples = 0_u8;
        while Instant::now() < deadline {
            match self.backend.runtime_healthy(targets, ssid) {
                Ok(true) => {
                    successful_samples = successful_samples.saturating_add(1);
                    if successful_samples >= 2 {
                        self.refresh_observed();
                        return Ok(());
                    }
                }
                Ok(false) | Err(_) => successful_samples = 0,
            }
            thread::sleep(self.timing.verify_sample_delay);
        }
        Err(LegacyAppError::new(
            ErrorCode::ReconcileFailed,
            ErrorStage::Reconcile,
            "wireless runtime did not recover after reload",
        )
        .retryable(true))
    }
}

fn should_skip_reconcile(inner: &Inner) -> bool {
    inner.maintenance
        || (inner.lifecycle == Lifecycle::Degraded && inner.repair_failures >= 3)
        || inner.active_operation.is_some()
        || inner.config.wifi.primary.targets.is_empty()
        || matches!(inner.lifecycle, Lifecycle::NeedsSetup | Lifecycle::Booting)
}

fn evaluate_reconcile_state(
    inner: &mut Inner,
    app: &App,
    observed_wan: Option<&crate::domain::WanDesired>,
    observation_changed: bool,
) -> (Option<Repair>, bool) {
    let config_drift = !all_equal_config(
        &inner.observed_configs,
        &inner.config.wifi.primary.targets,
        &inner.config.wifi.primary,
    ) || inner.observed_roaming.as_ref()
        != Some(&crate::domain::compile_applied_roaming(
            inner.config.wifi.roaming,
            &inner.config.wifi.primary.ssid,
            &inner.config.wifi.primary.encryption,
            &inner.config.wifi.primary.targets,
        ));
    let wan_drift = observed_wan
        .is_some_and(|wan| !crate::application::wan::wan_config_matches(wan, &inner.config.wan));
    let runtime_drift = !inner.runtime_healthy;

    if !config_drift && !wan_drift && !runtime_drift {
        let publish = clear_recovered_errors(inner) || observation_changed;
        (None, publish)
    } else if config_drift {
        let context = build_wifi_reconcile_context(inner, app.next_operation_id());
        inner.active_operation = Some(context.public(OperationStatus::Accepted, None));
        (Some(Repair::Config(Box::new(context))), false)
    } else if wan_drift {
        let context = build_wan_reconcile_context(inner, app.next_operation_id());
        inner.active_operation = Some(context.public(OperationStatus::Accepted, None));
        (Some(Repair::WanConfig(Box::new(context))), false)
    } else {
        let targets = inner.config.wifi.primary.targets.clone();
        let ssid = inner.config.wifi.primary.ssid.clone();
        let context = build_wifi_reconcile_context(inner, app.next_operation_id());
        inner.active_operation = Some(context.public(OperationStatus::Accepted, None));
        (Some(Repair::Runtime { targets, ssid }), false)
    }
}

fn clear_recovered_errors(inner: &mut Inner) -> bool {
    if inner.repair_failures > 0 {
        inner.repair_failures = 0;
        if inner.lifecycle == Lifecycle::Degraded {
            inner.lifecycle = Lifecycle::Ready;
        }
        inner.health.core = "ok".into();
        inner.last_system_error = None;
        true
    } else if inner.lifecycle == Lifecycle::Ready && inner.last_system_error.is_some() {
        inner.last_system_error = None;
        true
    } else {
        false
    }
}

fn build_wifi_reconcile_context(inner: &Inner, op_id: String) -> ChangeContext {
    ChangeContext {
        operation_id: op_id,
        request_id: None,
        source: OperationSource::Reconcile,
        base_revision: inner.config.revision,
        target_revision: inner.config.revision,
        old_wifi: inner.config.wifi.primary.clone(),
        new_wifi: inner.config.wifi.primary.clone(),
        old_roaming: inner.config.wifi.roaming,
        new_roaming: inner.config.wifi.roaming,
        targets: inner.config.wifi.primary.targets.clone(),
        backhaul: inner.config.wifi.backhaul.clone(),
        radio_channels: inner.config.wifi.radio_channels.clone(),
    }
}

fn build_wan_reconcile_context(inner: &Inner, op_id: String) -> WanChangeContext {
    WanChangeContext {
        operation_id: op_id,
        request_id: None,
        source: OperationSource::Reconcile,
        base_revision: inner.config.revision,
        target_revision: inner.config.revision,
        old_wan: inner.config.wan.clone(),
        new_wan: inner.config.wan.clone(),
    }
}
