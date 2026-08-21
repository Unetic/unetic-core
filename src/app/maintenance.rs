use std::{sync::Arc, thread};

use super::App;
use crate::{
    errors::{DomainError, ErrorCode, ErrorStage},
    model::{Lifecycle, MaintenanceState, PublicState},
};

impl App {
    pub fn maintenance_get(&self) -> MaintenanceState {
        self.state().maintenance
    }

    pub fn maintenance_enter(&self, reason: Option<String>) -> Result<PublicState, DomainError> {
        {
            let mut inner = self.inner.lock().expect("app state poisoned");
            if inner.active_operation.is_some() || inner.maintenance_exiting {
                return Err(DomainError::new(
                    ErrorCode::Busy,
                    ErrorStage::Validate,
                    "cannot enter maintenance while another transition is active",
                ));
            }

            inner.maintenance = true;
            inner.maintenance_exiting = false;
            inner.maintenance_reason = reason;
            inner.lifecycle = Lifecycle::Maintenance;
        }

        Ok(self.publish())
    }

    pub fn maintenance_exit(self: &Arc<Self>) -> Result<PublicState, DomainError> {
        {
            let mut inner = self.inner.lock().expect("app state poisoned");
            if !inner.maintenance {
                return Ok(self.state());
            }
            if inner.active_operation.is_some() || inner.maintenance_exiting {
                return Err(DomainError::new(
                    ErrorCode::Busy,
                    ErrorStage::Validate,
                    "maintenance exit is already running or blocked by another transition",
                ));
            }
            inner.maintenance_exiting = true;
        }
        self.publish();

        let app = Arc::clone(self);
        if let Err(error) = thread::Builder::new()
            .name("unetic-maint-exit".into())
            .spawn(move || run_maintenance_exit(app))
        {
            let mut inner = self.inner.lock().expect("app state poisoned");
            inner.maintenance_exiting = false;
            let domain_error = DomainError::new(
                ErrorCode::Internal,
                ErrorStage::Reconcile,
                format!("failed to start maintenance exit worker: {error}"),
            );
            inner.last_system_error = Some(domain_error.clone());
            inner.lifecycle = Lifecycle::Degraded;
            drop(inner);
            self.publish();
            return Err(domain_error);
        }

        Ok(self.state())
    }
}

fn run_maintenance_exit(app: Arc<App>) {
    let (targets, desired, base_revision) = {
        let inner = app.inner.lock().expect("app state poisoned");
        (
            inner.config.wifi.primary.targets.clone(),
            inner.config.wifi.primary.clone(),
            inner.config.revision,
        )
    };

    let sync_result = if targets.is_empty() {
        Ok(())
    } else {
        crate::transaction::force_state_sync(
            &app,
            &targets,
            &desired,
            crate::model::OperationSource::Reconcile,
        )
    };

    let wan_config = {
        let inner = app.inner.lock().expect("app state poisoned");
        inner.config.wan.clone()
    };

    let wan_sync_result = crate::wan::force_wan_state_sync(
        &app,
        &wan_config,
        crate::model::OperationSource::Reconcile,
        base_revision,
    );

    let mut inner = app.inner.lock().expect("app state poisoned");
    inner.maintenance_exiting = false;

    match (sync_result, wan_sync_result) {
        (Ok(()), Ok(())) => {
            inner.maintenance = false;
            inner.maintenance_reason = None;
            inner.lifecycle = Lifecycle::Ready;
            inner.repair_failures = 0;
            inner.health.core = "ok".into();
            inner.health.wireless = "ok".into();
            inner.health.wan = "ok".into();
            inner.last_system_error = None;
        }
        (Err(error), _) | (_, Err(error)) => {
            inner.maintenance = true;
            inner.lifecycle = Lifecycle::Degraded;
            inner.health.core = "error".into();
            inner.last_system_error = Some(error);
        }
    }
    drop(inner);
    app.refresh_observed();
    app.publish();
}
