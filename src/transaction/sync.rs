use std::{thread, time::Instant};

use crate::{
    app::App,
    errors::{DomainError, ErrorCode, ErrorStage},
    model::OperationSource,
};

pub fn verify(
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

    let session = crate::backend::SessionGuard::new(app.backend.as_ref())?;
    if let Err(error) = app.backend.stage_ssid(&session.id, targets, ssid) {
        let _ = app.backend.revert_staged(&session.id);
        return Err(error);
    }

    let staged = match app.backend.read_ssids(targets, Some(&session.id)) {
        Ok(staged) => staged,
        Err(error) => {
            let _ = app.backend.revert_staged(&session.id);
            return Err(error);
        }
    };
    if targets
        .iter()
        .any(|target| staged.get(target).is_none_or(|value| value != ssid))
    {
        let _ = app.backend.revert_staged(&session.id);
        return Err(DomainError::new(
            ErrorCode::UciStageMismatch,
            ErrorStage::Reconcile,
            "recovery stage did not match desired state",
        ));
    }

    if let Err(error) = app
        .backend
        .apply(&session.id, app.timing.rpcd_rollback_timeout_secs)
    {
        let _ = app.backend.rollback(&session.id);
        let _ = app.backend.revert_staged(&session.id);
        return Err(error);
    }
    if let Err(error) = verify(app, targets, ssid, app.timing.verify_timeout) {
        let _ = app.backend.rollback(&session.id);
        return Err(error);
    }
    app.backend.confirm(&session.id)?;
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
