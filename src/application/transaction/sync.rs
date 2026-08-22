use std::{thread, time::Instant};

use crate::{
    application::app::App,
    domain::errors::{ErrorCode, ErrorStage, LegacyAppError},
    domain::{OperationSource, RoamingConfig, RoamingRuntimeStatus, WifiNetworkConfig},
};

pub fn verify(
    app: &App,
    targets: &[String],
    expected: &WifiNetworkConfig,
    roaming: RoamingConfig,
    timeout: std::time::Duration,
) -> Result<(), LegacyAppError> {
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
                let expected_roaming = crate::domain::compile_applied_roaming(
                    roaming,
                    &expected.ssid,
                    &expected.encryption,
                    targets,
                );
                let config_ready = app
                    .backend
                    .read_roaming_config(targets, None)
                    .is_ok_and(|observed| observed == expected_roaming);
                let roaming_runtime =
                    app.backend
                        .read_roaming_runtime(targets, &expected.ssid, roaming);
                match app.backend.runtime_healthy(targets, &expected.ssid) {
                    Ok(true)
                        if config_ready
                            && roaming_runtime.status == RoamingRuntimeStatus::Ready =>
                    {
                        successful_samples += 1;
                        if successful_samples >= 2 {
                            return Ok(());
                        }
                    }
                    Ok(false) => {
                        successful_samples = 0;
                        last_reason = "wireless runtime is not healthy".into();
                    }
                    Ok(true) => {
                        successful_samples = 0;
                        last_reason = roaming_runtime.error.unwrap_or_else(|| {
                            "wireless or usteer roaming state is not converged".into()
                        });
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

    Err(LegacyAppError::new(
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
    roaming: RoamingConfig,
    _source: OperationSource,
) -> Result<(), LegacyAppError> {
    if targets.is_empty() {
        return Err(LegacyAppError::new(
            ErrorCode::TargetMissing,
            ErrorStage::Reconcile,
            "cannot reconcile Wi-Fi without managed targets",
        ));
    }

    let session = crate::infrastructure::backend::SessionGuard::new(app.backend.as_ref())?;
    let is_extender = {
        let inner = app.inner.lock().unwrap();
        inner.config.wan.proto == crate::domain::WanProtocol::Extender
    };
    if let Err(error) =
        app.backend
            .stage_wifi_config(&session.id, targets, config, roaming, is_extender)
    {
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
            cfg.ssid != config.ssid || cfg.encryption != config.encryption || cfg.key != config.key
        })
    }) {
        let _ = app.backend.revert_staged(&session.id);
        return Err(LegacyAppError::new(
            ErrorCode::UciStageMismatch,
            ErrorStage::Reconcile,
            "recovery stage did not match desired state",
        ));
    }
    let expected_roaming =
        crate::domain::compile_applied_roaming(roaming, &config.ssid, &config.encryption, targets);
    if app
        .backend
        .read_roaming_config(targets, Some(&session.id))?
        != expected_roaming
    {
        let _ = app.backend.revert_staged(&session.id);
        return Err(LegacyAppError::new(
            ErrorCode::UciStageMismatch,
            ErrorStage::Reconcile,
            "recovery roaming stage did not match desired state",
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
    if let Err(error) = verify(app, targets, config, roaming, app.timing.verify_timeout) {
        let _ = app.backend.rollback(&session.id);
        return Err(error);
    }
    app.backend.confirm(&session.id)?;
    Ok(())
}

pub fn run_recovery_sync(app: &App, source: OperationSource) -> Result<(), LegacyAppError> {
    let (targets, wifi, roaming) = {
        let inner = app.inner.lock().expect("app state poisoned");
        (
            inner.config.wifi.primary.targets.clone(),
            inner.config.wifi.primary.clone(),
            inner.config.wifi.roaming,
        )
    };
    force_state_sync(app, &targets, &wifi, roaming, source)
}
