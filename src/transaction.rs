use std::{sync::Arc, thread, time::Instant};

use tracing::{error, info, warn};

use crate::{
    app::App,
    errors::{DomainError, ErrorCode, ErrorStage},
    model::{
        OperationSource, OperationStatus, PublicOperation, STATE_SCHEMA_VERSION, TransactionJournal,
    },
};

#[derive(Debug, Clone)]
pub struct ChangeContext {
    pub operation_id: String,
    pub request_id: Option<String>,
    pub source: OperationSource,
    pub base_revision: u64,
    pub target_revision: u64,
    pub old_ssid: String,
    pub new_ssid: String,
    pub targets: Vec<String>,
}

impl ChangeContext {
    #[must_use]
    pub fn public(&self, status: OperationStatus, error: Option<DomainError>) -> PublicOperation {
        PublicOperation {
            id: self.operation_id.clone(),
            request_id: self.request_id.clone(),
            source: self.source,
            kind: "wifi.set_ssid".into(),
            status,
            requested_ssid: self.new_ssid.clone(),
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
            old_ssid: self.old_ssid.clone(),
            new_ssid: self.new_ssid.clone(),
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
        old_ssid = %context.old_ssid,
        new_ssid = %context.new_ssid,
    );
    let _entered = span.enter();

    if let Err(error) = execute(&app, &context) {
        error!(%error, "configuration operation failed unexpectedly");
        app.complete_failure(&context, error, false);
    }
}

fn execute(app: &Arc<App>, context: &ChangeContext) -> Result<(), DomainError> {
    let session = app.ensure_session().map_err(|error| {
        error.with_operation(&context.operation_id, context.request_id.as_deref())
    })?;

    app.set_operation_status(context, OperationStatus::Staging, None)?;
    if let Err(error) = app
        .backend
        .stage_ssid(&session, &context.targets, &context.new_ssid)
    {
        let _ = app.backend.revert_staged(&session);
        app.complete_failure(context, attach(error, context), false);
        return Ok(());
    }

    let staged = match app.backend.read_ssids(&context.targets, Some(&session)) {
        Ok(staged) => staged,
        Err(error) => {
            let _ = app.backend.revert_staged(&session);
            app.complete_failure(context, attach(error, context), false);
            return Ok(());
        }
    };

    if context
        .targets
        .iter()
        .any(|target| staged.get(target) != Some(&context.new_ssid))
    {
        let _ = app.backend.revert_staged(&session);
        app.complete_failure(
            context,
            DomainError::new(
                ErrorCode::UciStageMismatch,
                ErrorStage::Stage,
                "staged UCI values do not match requested SSID",
            )
            .with_operation(&context.operation_id, context.request_id.as_deref()),
            false,
        );
        return Ok(());
    }

    app.set_operation_status(context, OperationStatus::Applying, None)?;
    if let Err(error) = app
        .backend
        .apply(&session, app.timing.rpcd_rollback_timeout_secs)
    {
        // An apply transport failure can be ambiguous: rpcd may have already
        // committed and armed its rollback timer before the reply was lost.
        // Attempt both rollback and staged-delta cleanup before reporting failure.
        let _ = app.backend.rollback(&session);
        let _ = app.backend.revert_staged(&session);
        app.complete_failure(context, attach(error, context), false);
        return Ok(());
    }

    app.set_operation_status(context, OperationStatus::Verifying, None)?;
    if let Err(error) = verify(
        app,
        &context.targets,
        &context.new_ssid,
        app.timing.verify_timeout,
    ) {
        rollback_to_old(app, context, &session, error);
        return Ok(());
    }

    if context.source == OperationSource::User {
        app.set_operation_status(context, OperationStatus::Persisting, None)?;
        if let Err(error) = app.persist_new_desired(context) {
            rollback_to_old(app, context, &session, attach(error, context));
            return Ok(());
        }
    }

    app.set_operation_status(context, OperationStatus::Confirming, None)?;
    if let Err(error) = app.backend.confirm(&session) {
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
        &context.old_ssid,
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

fn verify(
    app: &App,
    targets: &[String],
    ssid: &str,
    timeout: std::time::Duration,
) -> Result<(), DomainError> {
    let deadline = Instant::now() + timeout;
    let mut successful_samples = 0_u8;
    let mut last_reason = String::from("wireless state not converged");

    while Instant::now() < deadline {
        match app.backend.read_ssids(targets, None) {
            Ok(observed)
                if targets
                    .iter()
                    .all(|target| observed.get(target).is_some_and(|value| value == ssid)) =>
            {
                match app.backend.runtime_healthy(targets, ssid) {
                    Ok(true) => {
                        successful_samples += 1;
                        if successful_samples >= 2 {
                            return Ok(());
                        }
                    }
                    Ok(false) => {
                        successful_samples = 0;
                        last_reason = "wireless runtime is not healthy".into();
                    }
                    Err(error) => {
                        successful_samples = 0;
                        last_reason = error.message;
                    }
                }
            }
            Ok(_) => {
                successful_samples = 0;
                last_reason = "committed UCI does not match candidate SSID".into();
            }
            Err(error) => {
                successful_samples = 0;
                last_reason = error.message;
            }
        }

        thread::sleep(app.timing.verify_sample_delay);
    }

    Err(DomainError::new(
        ErrorCode::VerifyTimeout,
        ErrorStage::Verify,
        format!("SSID did not converge before timeout: {last_reason}"),
    )
    .retryable(true))
}

pub fn force_state_sync(
    app: &App,
    targets: &[String],
    ssid: &str,
    _source: OperationSource,
) -> Result<(), DomainError> {
    if targets.is_empty() {
        return Err(DomainError::new(
            ErrorCode::TargetMissing,
            ErrorStage::Reconcile,
            "cannot reconcile Wi-Fi without managed targets",
        ));
    }

    let session = app.ensure_session()?;
    if let Err(error) = app.backend.stage_ssid(&session, targets, ssid) {
        let _ = app.backend.revert_staged(&session);
        return Err(error);
    }

    let staged = match app.backend.read_ssids(targets, Some(&session)) {
        Ok(staged) => staged,
        Err(error) => {
            let _ = app.backend.revert_staged(&session);
            return Err(error);
        }
    };
    if targets
        .iter()
        .any(|target| staged.get(target).is_none_or(|value| value != ssid))
    {
        let _ = app.backend.revert_staged(&session);
        return Err(DomainError::new(
            ErrorCode::UciStageMismatch,
            ErrorStage::Reconcile,
            "recovery stage did not match desired state",
        ));
    }

    if let Err(error) = app
        .backend
        .apply(&session, app.timing.rpcd_rollback_timeout_secs)
    {
        let _ = app.backend.rollback(&session);
        let _ = app.backend.revert_staged(&session);
        return Err(error);
    }
    if let Err(error) = verify(app, targets, ssid, app.timing.verify_timeout) {
        let _ = app.backend.rollback(&session);
        return Err(error);
    }
    app.backend.confirm(&session)?;
    Ok(())
}

pub fn run_recovery_sync(app: &App, source: OperationSource) -> Result<(), DomainError> {
    let (targets, ssid) = {
        let inner = app.inner.lock().expect("app state poisoned");
        (
            inner.config.wifi.primary.targets.clone(),
            inner.config.wifi.primary.ssid.clone(),
        )
    };
    force_state_sync(app, &targets, &ssid, source)
}

fn attach(error: DomainError, context: &ChangeContext) -> DomainError {
    error.with_operation(&context.operation_id, context.request_id.as_deref())
}
