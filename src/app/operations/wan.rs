use tracing::error;

use crate::{
    app::{App, state::now_ms},
    errors::{DomainError, ErrorCode, ErrorStage},
    model::{LastOperation, Lifecycle, OperationSource, OperationStatus},
    wan::WanChangeContext,
};

impl App {
    pub(crate) fn persist_new_desired_wan(
        &self,
        context: &WanChangeContext,
    ) -> Result<(), DomainError> {
        let mut config = {
            let inner = self.inner.lock().expect("app state poisoned");
            inner.config.clone()
        };
        config.revision = context.target_revision;
        config.wan = context.new_wan.clone();

        self.store.persist_config(&config)?;
        {
            let mut inner = self.inner.lock().expect("app state poisoned");
            inner.config = config;
        }
        Ok(())
    }

    pub(crate) fn complete_wan_success(
        &self,
        context: &WanChangeContext,
    ) -> Result<(), DomainError> {
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
            kind: "wan.set_config".into(),
            status: OperationStatus::Succeeded,
            revision,
            requested_ssid: String::new(),
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

        let wan_status = self.backend.read_wan_runtime_status().unwrap_or_default();
        {
            let mut inner = self.inner.lock().expect("app state poisoned");
            if context.source == OperationSource::User {
                inner.last_user_operation = Some(last);
            }
            inner.active_operation = None;
            inner.wan = wan_status;
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
            inner.health.wan = "ok".into();
        }
        self.publish();
        Ok(())
    }

    pub(crate) fn complete_wan_failure(
        &self,
        context: &WanChangeContext,
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
            kind: "wan.set_config".into(),
            status: if rollback_failed {
                OperationStatus::RollbackFailed
            } else {
                OperationStatus::Failed
            },
            revision,
            requested_ssid: String::new(),
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

        let wan_status = self.backend.read_wan_runtime_status().unwrap_or_default();
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
            inner.wan = wan_status;
            if rollback_failed || store_failed || inner.repair_failures >= 3 {
                inner.lifecycle = Lifecycle::Degraded;
                inner.health.core = "error".into();
            }
            inner.health.wan = "error".into();
        }
        self.refresh_observed();
        self.publish();
    }

    pub(crate) fn mark_wan_commit_uncertain(&self, context: &WanChangeContext, error: DomainError) {
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
            kind: "wan.set_config".into(),
            status: OperationStatus::Failed,
            revision,
            requested_ssid: String::new(),
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
}
