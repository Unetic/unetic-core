use std::sync::Arc;

use tracing::{error, info, warn};

use crate::{
    app::App,
    errors::{DomainError, ErrorCode, ErrorStage},
    model::{
        OperationSource, OperationStatus, PublicOperation, STATE_SCHEMA_VERSION,
        TransactionJournal, WifiNetworkConfig,
    },
};

#[derive(Debug, Clone)]
pub struct ChangeContext {
    pub operation_id: String,
    pub request_id: Option<String>,
    pub source: OperationSource,
    pub base_revision: u64,
    pub target_revision: u64,
    pub old_wifi: WifiNetworkConfig,
    pub new_wifi: WifiNetworkConfig,
    pub targets: Vec<String>,
}

impl ChangeContext {
    #[must_use]
    pub fn public(&self, status: OperationStatus, error: Option<DomainError>) -> PublicOperation {
        PublicOperation {
            id: self.operation_id.clone(),
            request_id: self.request_id.clone(),
            source: self.source,
            kind: "wifi.set_config".into(),
            status,
            requested_ssid: self.new_wifi.ssid.clone(),
            error,
        }
    }

    #[must_use]
    pub fn to_journal(&self, phase: OperationStatus) -> TransactionJournal {
        TransactionJournal {
            schema_version: STATE_SCHEMA_VERSION,
            operation_id: self.operation_id.clone(),
            request_id: self.request_id.clone().unwrap_or_default(),
            source: self.source,
            base_revision: self.base_revision,
            target_revision: self.target_revision,
            old_ssid: self.old_wifi.ssid.clone(),
            new_ssid: self.new_wifi.ssid.clone(),
            old_encryption: self.old_wifi.encryption.clone(),
            new_encryption: self.new_wifi.encryption.clone(),
            old_key: self.old_wifi.key.clone(),
            new_key: self.new_wifi.key.clone(),
            targets: self.targets.clone(),
            phase,
        }
    }
}

pub fn run_change(app: Arc<App>, context: ChangeContext) {
    let span = tracing::info_span!(
        "configuration_operation",
        operation_id = %context.operation_id,
        request_id = ?context.request_id,
        source = ?context.source,
        old_ssid = %context.old_wifi.ssid,
        new_ssid = %context.new_wifi.ssid,
    );
    let _entered = span.enter();

    if let Err(error) = execute(&app, &context) {
        error!(%error, "configuration operation failed unexpectedly");
        app.complete_failure(&context, error, false);
    }
}

fn execute(app: &Arc<App>, context: &ChangeContext) -> Result<(), DomainError> {
    let session = crate::backend::SessionGuard::new(app.backend.as_ref()).map_err(|error| {
        error.with_operation(&context.operation_id, context.request_id.as_deref())
    })?;

    app.set_operation_status(context, OperationStatus::Staging, None)?;
    if let Err(error) = app
        .backend
        .stage_wifi_config(&session.id, &context.targets, &context.new_wifi)
    {
        let _ = app.backend.revert_staged(&session.id);
        app.complete_failure(context, attach(error, context), false);
        return Ok(());
    }

    let staged = match app.backend.read_wifi_configs(&context.targets, Some(&session.id)) {
        Ok(staged) => staged,
        Err(error) => {
            let _ = app.backend.revert_staged(&session.id);
            app.complete_failure(context, attach(error, context), false);
            return Ok(());
        }
    };

    if context.targets.iter().any(|target| {
        staged.get(target).is_none_or(|cfg| {
            cfg.ssid != context.new_wifi.ssid
                || cfg.encryption != context.new_wifi.encryption
                || cfg.key != context.new_wifi.key
        })
    }) {
        let _ = app.backend.revert_staged(&session.id);
        app.complete_failure(
            context,
            DomainError::new(
                ErrorCode::UciStageMismatch,
                ErrorStage::Stage,
                "staged UCI values do not match requested Wi-Fi configuration",
            )
            .with_operation(&context.operation_id, context.request_id.as_deref()),
            false,
        );
        return Ok(());
    }

    app.set_operation_status(context, OperationStatus::Applying, None)?;
    if let Err(error) = app
        .backend
        .apply(&session.id, app.timing.rpcd_rollback_timeout_secs)
    {
        let _ = app.backend.rollback(&session.id);
        let _ = app.backend.revert_staged(&session.id);
        app.complete_failure(context, attach(error, context), false);
        return Ok(());
    }

    app.set_operation_status(context, OperationStatus::Verifying, None)?;
    if let Err(error) = verify(
        app,
        &context.targets,
        &context.new_wifi,
        app.timing.verify_timeout,
    ) {
        rollback_to_old(app, context, &session.id, error);
        return Ok(());
    }

    if context.source == OperationSource::User {
        app.set_operation_status(context, OperationStatus::Persisting, None)?;
        if let Err(error) = app.persist_new_desired(context) {
            rollback_to_old(app, context, &session.id, attach(error, context));
            return Ok(());
        }
    }

    app.set_operation_status(context, OperationStatus::Confirming, None)?;
    if let Err(error) = app.backend.confirm(&session.id) {
        if context.source == OperationSource::User {
            app.mark_commit_uncertain(context, error);
            return Ok(());
        }
        app.complete_failure(context, attach(error, context), false);
        return Ok(());
    }

    info!("configuration operation confirmed");
    app.complete_success(context)?;
    Ok(())
}

fn rollback_to_old(
    app: &Arc<App>,
    context: &ChangeContext,
    session: &str,
    original_error: DomainError,
) {
    warn!(%original_error, "rolling configuration back");
    let _ = app.set_operation_status(
        context,
        OperationStatus::RollingBack,
        Some(original_error.clone()),
    );

    if let Err(rollback_error) = app.backend.rollback(session) {
        app.complete_failure(
            context,
            DomainError::new(
                ErrorCode::RollbackFailed,
                ErrorStage::Rollback,
                format!(
                    "{}; rollback failed: {}",
                    original_error.message, rollback_error.message
                ),
            )
            .with_operation(&context.operation_id, context.request_id.as_deref()),
            true,
        );
        return;
    }

    let rollback_result = verify(
        app,
        &context.targets,
        &context.old_wifi,
        app.timing.rollback_verify_timeout,
    );
    if let Err(error) = rollback_result {
        app.complete_failure(
            context,
            DomainError::new(
                ErrorCode::RollbackFailed,
                ErrorStage::Rollback,
                format!(
                    "{}; rollback verification failed: {}",
                    original_error.message, error.message
                ),
            )
            .with_operation(&context.operation_id, context.request_id.as_deref()),
            true,
        );
        return;
    }

    app.complete_failure(context, original_error, false);
}

pub mod sync;
use sync::verify;
pub use sync::{force_state_sync, run_recovery_sync};

fn attach(error: DomainError, context: &ChangeContext) -> DomainError {
    error.with_operation(&context.operation_id, context.request_id.as_deref())
}
