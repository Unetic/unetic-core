use std::{thread, time::Instant};

use crate::{
    app::App,
    errors::{DomainError, ErrorCode, ErrorStage},
    model::{OperationSource, WifiNetworkConfig},
};

pub fn verify(
    app: &App,
    targets: &[String],
    expected: &WifiNetworkConfig,
    timeout: std::time::Duration,
) -> Result<(), DomainError> {
    let deadline = Instant::now() + timeout;
    let mut successful_samples = 0_u8;
    let mut last_reason = String::from("wireless state not converged");

    while Instant::now() < deadline {
        match app.backend.read_wifi_configs(targets, None) {
            Ok(observed)
                if targets.iter().all(|target| {
                    observed.get(target).is_some_and(|cfg| {
                        cfg.ssid == expected.ssid
                            && cfg.encryption == expected.encryption
                            && cfg.key == expected.key
                    })
                }) =>
            {
                match app.backend.runtime_healthy(targets, &expected.ssid) {
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
                last_reason = "committed UCI does not match candidate Wi-Fi configuration".into();
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
        format!("Wi-Fi configuration did not converge before timeout: {last_reason}"),
    )
    .retryable(true))
}

pub fn force_state_sync(
    app: &App,
    targets: &[String],
    config: &WifiNetworkConfig,
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
    if let Err(error) = app.backend.stage_wifi_config(&session.id, targets, config) {
        let _ = app.backend.revert_staged(&session.id);
        return Err(error);
    }

    let staged = match app.backend.read_wifi_configs(targets, Some(&session.id)) {
        Ok(staged) => staged,
        Err(error) => {
            let _ = app.backend.revert_staged(&session.id);
            return Err(error);
        }
    };
    if targets.iter().any(|target| {
        staged.get(target).is_none_or(|cfg| {
            cfg.ssid != config.ssid
                || cfg.encryption != config.encryption
                || cfg.key != config.key
        })
    }) {
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
    if let Err(error) = verify(app, targets, config, app.timing.verify_timeout) {
        let _ = app.backend.rollback(&session.id);
        return Err(error);
    }
    app.backend.confirm(&session.id)?;
    Ok(())
}

pub fn run_recovery_sync(app: &App, source: OperationSource) -> Result<(), DomainError> {
    let (targets, wifi) = {
        let inner = app.inner.lock().expect("app state poisoned");
        (
            inner.config.wifi.primary.targets.clone(),
            inner.config.wifi.primary.clone(),
        )
    };
    force_state_sync(app, &targets, &wifi, source)
}
