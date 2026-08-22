use tracing::error;

use crate::{
    application::app::{App, Inner},
    application::state::now_ms,
    application::wan::WanChangeContext,
    domain::errors::{LegacyAppError, ErrorCode, ErrorStage},
    domain::{LastOperation, Lifecycle, OperationSource, OperationStatus},
};

impl App {
    pub(crate) fn persist_new_desired_wan(
        &self,
        context: &WanChangeContext,
    ) -> Result<(), LegacyAppError> {
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
    ) -> Result<(), LegacyAppError> {
        let revision = self
            .inner
            .lock()
            .expect("app state poisoned")
            .config
            .revision;
        let last = make_wan_last_op(context, OperationStatus::Succeeded, revision, None);

        let mut store_error = None;
        if context.source == OperationSource::User
            && let Err(error) = self.store.clear_transaction()
        {
            error!(%error, "configuration was committed but transaction journal cleanup failed");
            store_error = Some(error);
        }

        let wan_status = self.backend.read_wan_runtime_status().unwrap_or_default();
        {
            let mut inner = self.inner.lock().expect("app state poisoned");
            apply_wan_success_state(&mut inner, context, last, wan_status, store_error);
        }
        self.publish();
        Ok(())
    }

    pub(crate) fn complete_wan_failure(
        &self,
        context: &WanChangeContext,
        error: LegacyAppError,
        rollback_failed: bool,
    ) {
        let error = error.with_operation(&context.operation_id, context.request_id.as_deref());
        let revision = self
            .inner
            .lock()
            .expect("app state poisoned")
            .config
            .revision;
        let status = if rollback_failed {
            OperationStatus::RollbackFailed
        } else {
            OperationStatus::Failed
        };
        let last = make_wan_last_op(context, status, revision, Some(error.clone()));

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
            apply_wan_failure_state(
                &mut inner,
                context,
                last,
                wan_status,
                error,
                rollback_failed,
                store_failed,
            );
        }
        self.refresh_observed();
        self.publish();
    }

    pub(crate) fn mark_wan_commit_uncertain(&self, context: &WanChangeContext, error: LegacyAppError) {
        let uncertain = LegacyAppError::new(
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
        let last = make_wan_last_op(
            context,
            OperationStatus::Failed,
            revision,
            Some(uncertain.clone()),
        );

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

fn make_wan_last_op(
    context: &WanChangeContext,
    status: OperationStatus,
    revision: u64,
    error: Option<LegacyAppError>,
) -> LastOperation {
    LastOperation {
        id: context.operation_id.clone(),
        request_id: context.request_id.clone(),
        source: context.source,
        kind: "wan.set_config".into(),
        status,
        revision,
        requested_ssid: String::new(),
        error,
        finished_at_ms: now_ms(),
    }
}

fn apply_wan_success_state(
    inner: &mut Inner,
    context: &WanChangeContext,
    last: LastOperation,
    wan_status: crate::domain::WanPublicState,
    store_error: Option<LegacyAppError>,
) {
    if context.source == OperationSource::User {
        inner.last_user_operation = Some(last);
    }
    inner.active_operation = None;
    inner.wan = wan_status;
    inner.repair_failures = 0;
    inner.health.wan = "ok".into();

    if let Some(err) = store_error {
        inner.lifecycle = Lifecycle::Degraded;
        inner.health.core = "error".into();
        inner.last_system_error = Some(err);
    } else {
        if !inner.maintenance {
            inner.lifecycle = Lifecycle::Ready;
        }
        inner.health.core = "ok".into();
        if context.source != OperationSource::User {
            inner.last_system_error = None;
        }
    }
}

fn apply_wan_failure_state(
    inner: &mut Inner,
    context: &WanChangeContext,
    last: LastOperation,
    wan_status: crate::domain::WanPublicState,
    error: LegacyAppError,
    rollback_failed: bool,
    store_failed: bool,
) {
    if context.source == OperationSource::User {
        inner.last_user_operation = Some(last);
        if store_failed {
            inner.last_system_error = Some(LegacyAppError::new(
                ErrorCode::StateStoreFailed,
                ErrorStage::Persist,
                "failed to clear transaction journal",
            ));
        }
    } else {
        inner.repair_failures = inner.repair_failures.saturating_add(1);
        inner.last_system_error = Some(error);
    }

    inner.active_operation = None;
    inner.wan = wan_status;
    if rollback_failed || store_failed || inner.repair_failures >= 3 {
        inner.lifecycle = Lifecycle::Degraded;
        inner.health.core = "error".into();
    }
    inner.health.wan = "error".into();
}
