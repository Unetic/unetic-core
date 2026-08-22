use tracing::error;

use crate::{
    application::app::{App, Inner},
    application::state::now_ms,
    application::transaction::ChangeContext,
    domain::errors::{ErrorCode, ErrorStage, LegacyAppError},
    domain::{LastOperation, Lifecycle, OperationIntent, OperationSource, OperationStatus},
};

impl App {
    pub(crate) fn set_operation_status(
        &self,
        context: &ChangeContext,
        status: OperationStatus,
        error: Option<LegacyAppError>,
    ) -> Result<(), LegacyAppError> {
        self.set_operation_status_with_kind(&context.operation_id, "wifi.set_config", status, error)
    }

    pub(crate) fn set_operation_status_with_kind(
        &self,
        operation_id: &str,
        _kind: &str,
        status: OperationStatus,
        error: Option<LegacyAppError>,
    ) -> Result<(), LegacyAppError> {
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

    pub(crate) fn persist_new_desired(
        &self,
        context: &ChangeContext,
    ) -> Result<(), LegacyAppError> {
        let mut config = {
            let inner = self.inner.lock().expect("app state poisoned");
            inner.config.clone()
        };
        config.revision = context.target_revision;
        config.wifi.primary = context.new_wifi.clone();
        config.wifi.roaming = context.new_roaming;
        if context.backhaul.is_some() {
            config.wifi.backhaul = context.backhaul.clone();
        }
        if !context.radio_channels.is_empty() {
            config.wifi.radio_channels = context.radio_channels.clone();
        }
        self.store.persist_config(&config)?;
        {
            let mut inner = self.inner.lock().expect("app state poisoned");
            inner.config = config;
        }
        Ok(())
    }

    pub(crate) fn complete_success(&self, context: &ChangeContext) -> Result<(), LegacyAppError> {
        let revision = self
            .inner
            .lock()
            .expect("app state poisoned")
            .config
            .revision;
        let last = make_wifi_last_op(context, OperationStatus::Succeeded, revision, None);

        let mut store_error = None;
        if context.source == OperationSource::User
            && let Err(error) = self.store.clear_transaction()
        {
            error!(%error, "configuration was committed but transaction journal cleanup failed");
            store_error = Some(error);
        }

        {
            let mut inner = self.inner.lock().expect("app state poisoned");
            apply_wifi_success_state(&mut inner, context, last, store_error);
        }
        self.publish();
        Ok(())
    }

    pub(crate) fn complete_failure(
        &self,
        context: &ChangeContext,
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
        let last = make_wifi_last_op(context, status, revision, Some(error.clone()));

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
            apply_wifi_failure_state(
                &mut inner,
                context,
                last,
                error,
                rollback_failed,
                store_failed,
            );
        }
        self.refresh_observed();
        self.publish();
    }

    pub(crate) fn mark_commit_uncertain(&self, context: &ChangeContext, error: LegacyAppError) {
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
        let last = make_wifi_last_op(
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

    pub(crate) fn mark_degraded(&self, error: LegacyAppError) {
        error!(%error, "core entered degraded mode");
        {
            let mut inner = self.inner.lock().expect("app state poisoned");
            inner.lifecycle = Lifecycle::Degraded;
            inner.health.core = "error".into();
            inner.last_system_error = Some(error);
        }
        self.publish();
    }

    pub(crate) fn record_recovered_operation(
        &self,
        journal: &crate::domain::TransactionJournal,
        status: OperationStatus,
        revision: u64,
        error: Option<LegacyAppError>,
    ) {
        if journal.source != OperationSource::User {
            return;
        }

        let last = LastOperation {
            id: journal.operation_id.clone(),
            request_id: Some(journal.request_id.clone()),
            source: OperationSource::User,
            kind: "wifi.set_config".into(),
            status,
            revision,
            requested_ssid: journal.new_ssid.clone(),
            intent: Some(OperationIntent::Wifi {
                ssid: journal.new_ssid.clone(),
                encryption: journal.new_encryption.clone(),
                key: journal.new_key.clone(),
                roaming: journal.new_roaming,
            }),
            error,
            finished_at_ms: now_ms(),
        };
        self.inner
            .lock()
            .expect("app state poisoned")
            .last_user_operation = Some(last);
    }
}

fn make_wifi_last_op(
    context: &ChangeContext,
    status: OperationStatus,
    revision: u64,
    error: Option<LegacyAppError>,
) -> LastOperation {
    LastOperation {
        id: context.operation_id.clone(),
        request_id: context.request_id.clone(),
        source: context.source,
        kind: "wifi.set_config".into(),
        status,
        revision,
        requested_ssid: context.new_wifi.ssid.clone(),
        intent: Some(OperationIntent::Wifi {
            ssid: context.new_wifi.ssid.clone(),
            encryption: context.new_wifi.encryption.clone(),
            key: context.new_wifi.key.clone(),
            roaming: context.new_roaming,
        }),
        error,
        finished_at_ms: now_ms(),
    }
}

fn apply_wifi_success_state(
    inner: &mut Inner,
    context: &ChangeContext,
    last: LastOperation,
    store_error: Option<LegacyAppError>,
) {
    if context.source == OperationSource::User {
        inner.last_user_operation = Some(last);
    }
    inner.active_operation = None;
    inner.observed_configs = context
        .targets
        .iter()
        .map(|t| {
            let mut cfg = context.new_wifi.clone();
            cfg.targets = vec![t.clone()];
            (t.clone(), cfg)
        })
        .collect();
    inner.observed_roaming = Some(crate::domain::compile_applied_roaming(
        context.new_roaming,
        &context.new_wifi.ssid,
        &context.new_wifi.encryption,
        &context.targets,
    ));
    inner.roaming_runtime = crate::domain::RoamingRuntime {
        available: true,
        local_bss: context.targets.len().try_into().unwrap_or(u32::MAX),
        remote_bss: inner.roaming_runtime.remote_bss,
        status: crate::domain::RoamingRuntimeStatus::Ready,
        error: None,
    };
    inner.runtime_healthy = true;
    inner.repair_failures = 0;
    inner.health.wireless = "ok".into();

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

fn apply_wifi_failure_state(
    inner: &mut Inner,
    context: &ChangeContext,
    last: LastOperation,
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
    if rollback_failed || store_failed || inner.repair_failures >= 3 {
        inner.lifecycle = Lifecycle::Degraded;
        inner.health.core = "error".into();
    }
    inner.health.wireless = "error".into();
}
