use std::{
    sync::{Arc, atomic::Ordering},
    thread,
    time::{Duration, Instant},
};

use tracing::warn;

use super::{App, state::all_equal};
use crate::{
    errors::{DomainError, ErrorCode, ErrorStage},
    model::{Lifecycle, OperationSource, OperationStatus},
    transaction::{self, ChangeContext},
};

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
            self.reconcile_step();
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

    fn reconcile_step(self: &Arc<Self>) {
        self.reconcile_once();
    }

    fn reconcile_once(self: &Arc<Self>) {
        let observation_changed = self.refresh_observed();
        let observed_wan = self.backend.read_wan_config(None).ok();

        enum Repair {
            Config(ChangeContext),
            WanConfig(crate::transaction::WanChangeContext),
            Runtime { targets: Vec<String>, ssid: String },
            None { publish: bool },
        }

        let repair = {
            let mut inner = self.inner.lock().expect("app state poisoned");
            let config_drift = !all_equal(
                &inner.observed,
                &inner.config.wifi.primary.targets,
                &inner.config.wifi.primary.ssid,
            );
            let wan_drift = observed_wan.as_ref().is_some_and(|w| w != &inner.config.wan);
            let runtime_drift = !inner.runtime_healthy;

            let should_skip = inner.maintenance
                || (inner.lifecycle == Lifecycle::Degraded && inner.repair_failures >= 3)
                || inner.active_operation.is_some()
                || inner.config.wifi.primary.targets.is_empty()
                || matches!(inner.lifecycle, Lifecycle::NeedsSetup | Lifecycle::Booting);

            if should_skip {
                drop(inner);
                if observation_changed {
                    self.publish();
                }
                return;
            }

            if !config_drift && !wan_drift && !runtime_drift {
                let mut publish = observation_changed;
                if inner.repair_failures > 0 {
                    inner.repair_failures = 0;
                    if inner.lifecycle == Lifecycle::Degraded {
                        inner.lifecycle = Lifecycle::Ready;
                    }
                    inner.health.core = "ok".into();
                    inner.last_system_error = None;
                    publish = true;
                } else if inner.lifecycle == Lifecycle::Ready && inner.last_system_error.is_some() {
                    inner.last_system_error = None;
                    publish = true;
                }
                Repair::None { publish }
            } else if config_drift {
                let context = ChangeContext {
                    operation_id: self.next_operation_id(),
                    request_id: None,
                    source: OperationSource::Reconcile,
                    base_revision: inner.config.revision,
                    target_revision: inner.config.revision,
                    old_ssid: inner.config.wifi.primary.ssid.clone(),
                    new_ssid: inner.config.wifi.primary.ssid.clone(),
                    targets: inner.config.wifi.primary.targets.clone(),
                };
                inner.active_operation = Some(context.public(OperationStatus::Accepted, None));
                Repair::Config(context)
            } else if wan_drift {
                let context = crate::transaction::WanChangeContext {
                    operation_id: self.next_operation_id(),
                    request_id: None,
                    source: OperationSource::Reconcile,
                    base_revision: inner.config.revision,
                    target_revision: inner.config.revision,
                    old_wan: inner.config.wan.clone(),
                    new_wan: inner.config.wan.clone(),
                };
                inner.active_operation = Some(context.public(OperationStatus::Accepted, None));
                Repair::WanConfig(context)
            } else {
                let context = ChangeContext {
                    operation_id: self.next_operation_id(),
                    request_id: None,
                    source: OperationSource::Reconcile,
                    base_revision: inner.config.revision,
                    target_revision: inner.config.revision,
                    old_ssid: inner.config.wifi.primary.ssid.clone(),
                    new_ssid: inner.config.wifi.primary.ssid.clone(),
                    targets: inner.config.wifi.primary.targets.clone(),
                };
                inner.active_operation = Some(context.public(OperationStatus::Accepted, None));
                Repair::Runtime {
                    targets: inner.config.wifi.primary.targets.clone(),
                    ssid: inner.config.wifi.primary.ssid.clone(),
                }
            }
        };

        match repair {
            Repair::Config(context) => {
                self.publish();
                let app = Arc::clone(self);
                if thread::Builder::new()
                    .name("unetic-reconcile".into())
                    .spawn(move || transaction::run_change(app, context))
                    .is_err()
                {
                    let mut inner = self.inner.lock().expect("app state poisoned");
                    inner.active_operation = None;
                    inner.repair_failures = inner.repair_failures.saturating_add(1);
                    let error = DomainError::new(
                        ErrorCode::ReconcileFailed,
                        ErrorStage::Reconcile,
                        "failed to start reconciliation worker",
                    )
                    .retryable(true);
                    inner.last_system_error = Some(error);
                    if inner.repair_failures >= 3 {
                        inner.lifecycle = Lifecycle::Degraded;
                        inner.health.core = "error".into();
                    }
                    drop(inner);
                    self.publish();
                }
            }
            Repair::WanConfig(context) => {
                self.publish();
                let app = Arc::clone(self);
                if thread::Builder::new()
                    .name("unetic-reconcile-wan".into())
                    .spawn(move || crate::wan::execute_wan(&app, &context))
                    .is_err()
                {
                    let mut inner = self.inner.lock().expect("app state poisoned");
                    inner.active_operation = None;
                    inner.repair_failures = inner.repair_failures.saturating_add(1);
                    let error = DomainError::new(
                        ErrorCode::ReconcileFailed,
                        ErrorStage::Reconcile,
                        "failed to start WAN reconciliation worker",
                    )
                    .retryable(true);
                    inner.last_system_error = Some(error);
                    if inner.repair_failures >= 3 {
                        inner.lifecycle = Lifecycle::Degraded;
                        inner.health.core = "error".into();
                    }
                    drop(inner);
                    self.publish();
                }
            }
            Repair::Runtime { targets, ssid } => {
                let app = Arc::clone(self);
                if thread::Builder::new()
                    .name("unetic-repair".into())
                    .spawn(move || {
                        if let Err(error) = app.repair_runtime(&targets, &ssid) {
                            warn!(%error, "runtime-only wireless repair failed");
                            let mut inner = app.inner.lock().expect("app state poisoned");
                            inner.active_operation = None;
                            inner.repair_failures = inner.repair_failures.saturating_add(1);
                            inner.last_system_error = Some(error.clone());
                            if inner.repair_failures >= 3 {
                                inner.lifecycle = Lifecycle::Degraded;
                                inner.health.core = "error".into();
                            }
                            inner.health.wireless = "error".into();
                            drop(inner);
                            app.publish();
                        } else {
                            let mut inner = app.inner.lock().expect("app state poisoned");
                            inner.active_operation = None;
                            inner.repair_failures = 0;
                            if inner.lifecycle == Lifecycle::Degraded {
                                inner.lifecycle = Lifecycle::Ready;
                            }
                            inner.health.core = "ok".into();
                            inner.health.wireless = "ok".into();
                            inner.runtime_healthy = true;
                            inner.last_system_error = None;
                            drop(inner);
                            app.publish();
                        }
                    })
                    .is_err()
                {
                    let mut inner = self.inner.lock().expect("app state poisoned");
                    inner.active_operation = None;
                    drop(inner);
                }
            }
            Repair::None { publish } => {
                if publish {
                    self.publish();
                }
            }
        }
    }

    fn repair_runtime(&self, targets: &[String], ssid: &str) -> Result<(), DomainError> {
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
        Err(DomainError::new(
            ErrorCode::ReconcileFailed,
            ErrorStage::Reconcile,
            "wireless runtime did not recover after reload",
        )
        .retryable(true))
    }
}
