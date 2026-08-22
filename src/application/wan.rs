use std::{sync::Arc, thread, time::Instant};

use tracing::{error, info, warn};

use crate::{
    application::app::App,
    domain::errors::{ErrorCode, ErrorStage, LegacyAppError},
    domain::{OperationSource, OperationStatus, WanDesired, WanStatus},
};

mod context;
pub use context::WanChangeContext;

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

    if let Err(error) = stage_and_verify_wan(app, context, &session.id) {
        let error = attach_cleanup_error(error, app.backend.revert_staged(&session.id));
        app.complete_wan_failure(context, attach(error, context), false);
        return Ok(());
    }

    if let Err(error) = apply_and_verify_wan(app, context, &session.id) {
        rollback_to_old_wan(app, context, &session.id, error);
        return Ok(());
    }

    if let Err(error) = persist_and_confirm_wan(app, context, &session.id) {
        if error.code == ErrorCode::ConfirmFailed {
            app.mark_wan_commit_uncertain(context, error);
            return Ok(());
        }
        rollback_to_old_wan(app, context, &session.id, error);
        return Ok(());
    }

    info!("WAN configuration confirmed successfully");
    app.complete_wan_success(context)?;
    Ok(())
}

fn stage_and_verify_wan(
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
    stage_wan_candidate(app, session_id, &context.new_wan)
}

fn apply_and_verify_wan(
    app: &Arc<App>,
    context: &WanChangeContext,
    session_id: &str,
) -> Result<(), LegacyAppError> {
    app.set_operation_status_with_kind(
        &context.operation_id,
        "wan.set_config",
        OperationStatus::Applying,
        None,
    )?;
    app.backend
        .apply(session_id, app.timing.rpcd_rollback_timeout_secs)?;

    verify_wan_ready(app, context)
}

fn verify_wan_ready(app: &Arc<App>, context: &WanChangeContext) -> Result<(), LegacyAppError> {
    app.set_operation_status_with_kind(
        &context.operation_id,
        "wan.set_config",
        OperationStatus::Verifying,
        None,
    )?;

    let result = verify_wan_configuration(app, &context.new_wan, app.timing.verify_timeout);
    if result.is_err() {
        warn!("WAN verification timed out; rolling back");
    }
    result
}

fn rollback_to_old_wan(
    app: &Arc<App>,
    context: &WanChangeContext,
    session_id: &str,
    original_error: LegacyAppError,
) {
    warn!(%original_error, "rolling WAN configuration back");
    let _ = app.set_operation_status_with_kind(
        &context.operation_id,
        "wan.set_config",
        OperationStatus::RollingBack,
        Some(original_error.clone()),
    );

    if let Err(error) = app.backend.rollback(session_id) {
        let error = rollback_error(&original_error, "rollback failed", &error);
        app.complete_wan_failure(context, attach(error, context), true);
        return;
    }

    if let Err(error) =
        verify_wan_configuration(app, &context.old_wan, app.timing.rollback_verify_timeout)
    {
        let error = rollback_error(&original_error, "rollback verification failed", &error);
        app.complete_wan_failure(context, attach(error, context), true);
        return;
    }

    app.complete_wan_failure(context, attach(original_error, context), false);
}

fn rollback_error(
    original: &LegacyAppError,
    action: &str,
    rollback: &LegacyAppError,
) -> LegacyAppError {
    LegacyAppError::new(
        ErrorCode::RollbackFailed,
        ErrorStage::Rollback,
        format!("{}; {action}: {}", original.message, rollback.message),
    )
}

fn attach_cleanup_error(
    original: LegacyAppError,
    cleanup: Result<(), LegacyAppError>,
) -> LegacyAppError {
    let Err(cleanup) = cleanup else {
        return original;
    };
    LegacyAppError::new(
        original.code,
        original.stage,
        format!(
            "{}; staged cleanup failed: {}",
            original.message, cleanup.message
        ),
    )
    .retryable(original.retryable || cleanup.retryable)
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
    if let Err(error) = stage_wan_candidate(app, &session.id, desired) {
        let error = attach_cleanup_error(error, app.backend.revert_staged(&session.id));
        return Err(error);
    }
    if let Err(error) = app
        .backend
        .apply(&session.id, app.timing.rpcd_rollback_timeout_secs)
    {
        if let Err(rollback) = app.backend.rollback(&session.id) {
            return Err(rollback_error(&error, "rollback failed", &rollback));
        }
        return Err(attach_cleanup_error(
            error,
            app.backend.revert_staged(&session.id),
        ));
    }
    if let Err(error) = verify_wan_configuration(app, desired, app.timing.verify_timeout) {
        return match app.backend.rollback(&session.id) {
            Ok(()) => Err(error),
            Err(rollback) => Err(rollback_error(&error, "rollback failed", &rollback)),
        };
    }
    app.backend.confirm(&session.id)?;
    Ok(())
}

fn stage_wan_candidate(
    app: &App,
    session_id: &str,
    desired: &WanDesired,
) -> Result<(), LegacyAppError> {
    app.backend.stage_wan_config(session_id, desired)?;
    let staged = app.backend.read_wan_config(Some(session_id))?;
    if wan_config_matches(&staged, desired) {
        return Ok(());
    }
    Err(LegacyAppError::new(
        ErrorCode::UciStageMismatch,
        ErrorStage::Stage,
        "staged WAN configuration does not match desired state",
    ))
}

fn verify_wan_configuration(
    app: &App,
    expected: &WanDesired,
    timeout: std::time::Duration,
) -> Result<(), LegacyAppError> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        let config_matches = app
            .backend
            .read_wan_config(None)
            .is_ok_and(|wan| wan_config_matches(&wan, expected));
        if config_matches && !expected.present {
            return Ok(());
        }
        if config_matches
            && app
                .backend
                .read_wan_runtime_status()
                .is_ok_and(|status| status.status == WanStatus::Connected)
        {
            return Ok(());
        }
        thread::sleep(app.timing.verify_sample_delay);
    }

    Err(LegacyAppError::new(
        ErrorCode::VerifyTimeout,
        ErrorStage::Verify,
        "WAN configuration did not converge before timeout",
    )
    .retryable(true))
}

mod normalization;
pub mod validation;
pub(crate) use normalization::{normalize_wan_desired, wan_config_matches};
pub use validation::*;
