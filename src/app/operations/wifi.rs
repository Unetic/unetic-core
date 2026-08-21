use tracing::error;

use crate::{
    app::{App, state::now_ms},
    errors::{DomainError, ErrorCode, ErrorStage},
    model::{LastOperation, Lifecycle, OperationSource, OperationStatus},
    transaction::ChangeContext,
};

impl App {

    pub(crate) fn set_operation_status(
        &self,
        context: &ChangeContext,
        status: OperationStatus,
        error: Option<DomainError>,
    ) -> Result<(), DomainError> {
        self.set_operation_status_with_kind(&context.operation_id, "wifi.set_ssid", status, error)
    }

    pub(crate) fn set_operation_status_with_kind(
        &self,
        operation_id: &str,
        _kind: &str,
        status: OperationStatus,
        error: Option<DomainError>,
    ) -> Result<(), DomainError> {
        {
            let mut inner = self.inner.lock().expect("app state poisoned");
            if let Some(active) = &mut inner.active_operation
                && active.id == operation_id
            {
                active.status = status;
                active.error = error;
            }
        }
        self.publish();
        Ok(())
    }

    pub(crate) fn persist_new_desired(&self, context: &ChangeContext) -> Result<(), DomainError> {
        let mut config = {
            let inner = self.inner.lock().expect("app state poisoned");
            inner.config.clone()
        };
        config.revision = context.target_revision;
        config.wifi.primary.ssid.clone_from(&context.new_ssid);
        self.store.persist_config(&config)?;
        {
            let mut inner = self.inner.lock().expect("app state poisoned");
            inner.config = config;
        }
        Ok(())
    }

    pub(crate) fn complete_success(&self, context: &ChangeContext) -> Result<(), DomainError> {
        let revision = self
            .inner
            .lock()
            .expect("app state poisoned")
            .config
            .revision;
        let last = LastOperation {
            id: context.operation_id.clone(),
            request_id: context.request_id.clone(),
            source: context.source,
            kind: "wifi.set_ssid".into(),
            status: OperationStatus::Succeeded,
            revision,
            requested_ssid: context.new_ssid.clone(),
            error: None,
            finished_at_ms: now_ms(),
        };

        let mut completion_store_error = None;
        if context.source == OperationSource::User
            && let Err(error) = self.store.clear_transaction()
        {
            error!(%error, "configuration was committed but transaction journal cleanup failed");
            completion_store_error = Some(error);
        }

        {
            let mut inner = self.inner.lock().expect("app state poisoned");
            if context.source == OperationSource::User {
                inner.last_user_operation = Some(last);
            }
            inner.active_operation = None;
            inner.observed = context
                .targets
                .iter()
                .map(|target| (target.clone(), context.new_ssid.clone()))
                .collect();
            inner.runtime_healthy = true;
            inner.repair_failures = 0;
            if let Some(store_error) = completion_store_error {
                inner.lifecycle = Lifecycle::Degraded;
                inner.health.core = "error".into();
                inner.last_system_error = Some(store_error);
            } else {
                if !inner.maintenance {
                    inner.lifecycle = Lifecycle::Ready;
                }
                inner.health.core = "ok".into();
                if context.source != OperationSource::User {
                    inner.last_system_error = None;
                }
            }
            inner.health.wireless = "ok".into();
        }
        self.publish();
        Ok(())
    }

    pub(crate) fn complete_failure(
        &self,
        context: &ChangeContext,
        error: DomainError,
        rollback_failed: bool,
    ) {
        let error = error.with_operation(&context.operation_id, context.request_id.as_deref());
        let revision = self
            .inner
            .lock()
            .expect("app state poisoned")
            .config
            .revision;
        let last = LastOperation {
            id: context.operation_id.clone(),
            request_id: context.request_id.clone(),
            source: context.source,
            kind: "wifi.set_ssid".into(),
            status: if rollback_failed {
                OperationStatus::RollbackFailed
            } else {
                OperationStatus::Failed
            },
            revision,
            requested_ssid: context.new_ssid.clone(),
            error: Some(error.clone()),
            finished_at_ms: now_ms(),
        };

        let mut store_failed = false;
        if context.source == OperationSource::User
            && !rollback_failed
            && let Err(store_error) = self.store.clear_transaction()
        {
            error!(%store_error, "failed to clear transaction journal");
            store_failed = true;
        }

        {
            let mut inner = self.inner.lock().expect("app state poisoned");
            if context.source == OperationSource::User {
                inner.last_user_operation = Some(last);
                if store_failed {
                    inner.last_system_error = Some(DomainError::new(
                        ErrorCode::StateStoreFailed,
                        ErrorStage::Persist,
                        "failed to clear transaction journal",
                    ));
                }
            } else {
                inner.repair_failures = inner.repair_failures.saturating_add(1);
                inner.last_system_error = Some(error.clone());
            }
            inner.active_operation = None;
            if rollback_failed || store_failed || inner.repair_failures >= 3 {
                inner.lifecycle = Lifecycle::Degraded;
                inner.health.core = "error".into();
            }
            inner.health.wireless = "error".into();
        }
        self.refresh_observed();
        self.publish();
    }

    pub(crate) fn mark_commit_uncertain(&self, context: &ChangeContext, error: DomainError) {
        let uncertain = DomainError::new(
            ErrorCode::CommitUncertain,
            ErrorStage::Confirm,
            format!(
                "desired state is durable, but UCI confirm failed: {}",
                error.message
            ),
        )
        .retryable(true)
        .with_operation(&context.operation_id, context.request_id.as_deref());

        let revision = self
            .inner
            .lock()
            .expect("app state poisoned")
            .config
            .revision;
        let last = LastOperation {
            id: context.operation_id.clone(),
            request_id: context.request_id.clone(),
            source: context.source,
            kind: "wifi.set_ssid".into(),
            status: OperationStatus::Failed,
            revision,
            requested_ssid: context.new_ssid.clone(),
            error: Some(uncertain.clone()),
            finished_at_ms: now_ms(),
        };

        {
            let mut inner = self.inner.lock().expect("app state poisoned");
            if context.source == OperationSource::User {
                inner.last_user_operation = Some(last);
            }
            inner.active_operation = None;
            inner.lifecycle = Lifecycle::Degraded;
            inner.health.core = "error".into();
            inner.last_system_error = Some(uncertain);
        }
        self.publish();
    }

    pub(crate) fn mark_degraded(&self, error: DomainError) {
        error!(%error, "core entered degraded mode");
        {
            let mut inner = self.inner.lock().expect("app state poisoned");
            inner.lifecycle = Lifecycle::Degraded;
            inner.health.core = "error".into();
            inner.last_system_error = Some(error);
        }
        self.publish();
    }
}
