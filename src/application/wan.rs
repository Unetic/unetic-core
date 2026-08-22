use std::{sync::Arc, thread, time::Instant};

use tracing::{error, info, warn};

use crate::{
    application::app::App,
    domain::errors::{LegacyAppError, ErrorCode, ErrorStage},
    domain::{
        OperationSource, OperationStatus, PublicOperation, STATE_SCHEMA_VERSION,
        TransactionJournal, WanDesired, WanStatus,
    },
};

#[derive(Debug, Clone)]
pub struct WanChangeContext {
    pub operation_id: String,
    pub request_id: Option<String>,
    pub source: OperationSource,
    pub base_revision: u64,
    pub target_revision: u64,
    pub old_wan: WanDesired,
    pub new_wan: WanDesired,
}

impl WanChangeContext {
    #[must_use]
    pub fn public(&self, status: OperationStatus, error: Option<LegacyAppError>) -> PublicOperation {
        PublicOperation {
            id: self.operation_id.clone(),
            request_id: self.request_id.clone(),
            source: self.source,
            kind: "wan.set_config".into(),
            status,
            requested_ssid: String::new(),
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
            old_ssid: String::new(),
            new_ssid: String::new(),
            old_encryption: "none".into(),
            new_encryption: "none".into(),
            old_key: None,
            new_key: None,
            targets: Vec::new(),
            phase,
        }
    }
}

pub fn run_wan_change(app: Arc<App>, context: WanChangeContext) {
    let span = tracing::info_span!(
        "wan_operation",
        operation_id = %context.operation_id,
        request_id = ?context.request_id,
        source = ?context.source,
    );
    let _entered = span.enter();

    if let Err(error) = execute_wan(&app, &context) {
        error!(%error, "wan configuration operation failed unexpectedly");
        app.complete_wan_failure(&context, error, false);
    }
}

fn execute_wan(app: &Arc<App>, context: &WanChangeContext) -> Result<(), LegacyAppError> {
    let session = crate::infrastructure::backend::SessionGuard::new(app.backend.as_ref()).map_err(
        |error| error.with_operation(&context.operation_id, context.request_id.as_deref()),
    )?;

    if let Err(error) = stage_and_apply_wan(app, context, &session.id) {
        let _ = app.backend.rollback(&session.id);
        let _ = app.backend.revert_staged(&session.id);
        app.complete_wan_failure(context, attach(error, context), false);
        return Ok(());
    }

    if let Err(error) = verify_wan_ready(app, context) {
        let _ = app.backend.rollback(&session.id);
        app.complete_wan_failure(context, attach(error, context), false);
        return Ok(());
    }

    if let Err(error) = persist_and_confirm_wan(app, context, &session.id) {
        let _ = app.backend.rollback(&session.id);
        if context.source == OperationSource::User && error.code == ErrorCode::ConfirmFailed {
            app.mark_wan_commit_uncertain(context, error);
            return Ok(());
        }
        app.complete_wan_failure(context, attach(error, context), false);
        return Ok(());
    }

    info!("WAN configuration confirmed successfully");
    app.complete_wan_success(context)?;
    Ok(())
}

fn stage_and_apply_wan(
    app: &Arc<App>,
    context: &WanChangeContext,
    session_id: &str,
) -> Result<(), LegacyAppError> {
    app.set_operation_status_with_kind(
        &context.operation_id,
        "wan.set_config",
        OperationStatus::Staging,
        None,
    )?;
    app.backend.stage_wan_config(session_id, &context.new_wan)?;

    app.set_operation_status_with_kind(
        &context.operation_id,
        "wan.set_config",
        OperationStatus::Applying,
        None,
    )?;
    app.backend
        .apply(session_id, app.timing.rpcd_rollback_timeout_secs)
}

fn verify_wan_ready(app: &Arc<App>, context: &WanChangeContext) -> Result<(), LegacyAppError> {
    app.set_operation_status_with_kind(
        &context.operation_id,
        "wan.set_config",
        OperationStatus::Verifying,
        None,
    )?;

    if !context.new_wan.present {
        return Ok(());
    }

    let deadline = Instant::now() + app.timing.verify_timeout;
    while Instant::now() < deadline {
        if let Ok(st) = app.backend.read_wan_runtime_status() {
            if matches!(st.status, WanStatus::Connected | WanStatus::Connecting) {
                return Ok(());
            }
        }
        thread::sleep(app.timing.verify_sample_delay);
    }

    warn!("WAN verification timed out; rolling back");
    Err(LegacyAppError::new(
        ErrorCode::VerifyTimeout,
        ErrorStage::Verify,
        "WAN interface did not become ready",
    ))
}

fn persist_and_confirm_wan(
    app: &Arc<App>,
    context: &WanChangeContext,
    session_id: &str,
) -> Result<(), LegacyAppError> {
    if context.source == OperationSource::User {
        app.set_operation_status_with_kind(
            &context.operation_id,
            "wan.set_config",
            OperationStatus::Persisting,
            None,
        )?;
        app.persist_new_desired_wan(context)?;
    }

    app.set_operation_status_with_kind(
        &context.operation_id,
        "wan.set_config",
        OperationStatus::Confirming,
        None,
    )?;
    app.backend.confirm(session_id)
}

fn attach(error: LegacyAppError, context: &WanChangeContext) -> LegacyAppError {
    error.with_operation(&context.operation_id, context.request_id.as_deref())
}

pub fn force_wan_state_sync(
    app: &App,
    desired: &WanDesired,
    _source: OperationSource,
    _base_revision: u64,
) -> Result<(), LegacyAppError> {
    let session = crate::infrastructure::backend::SessionGuard::new(app.backend.as_ref())?;
    if let Err(error) = app.backend.stage_wan_config(&session.id, desired) {
        let _ = app.backend.revert_staged(&session.id);
        return Err(error);
    }
    if let Err(error) = app
        .backend
        .apply(&session.id, app.timing.rpcd_rollback_timeout_secs)
    {
        let _ = app.backend.rollback(&session.id);
        let _ = app.backend.revert_staged(&session.id);
        return Err(error);
    }
    app.backend.confirm(&session.id)?;
    Ok(())
}

pub mod validation;
pub use validation::*;
