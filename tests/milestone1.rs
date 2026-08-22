#![allow(clippy::pedantic)]

use std::{
    sync::Arc,
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use unetic_core::{
    App, MemoryBackend, RouterBackend, StateStore, Timing,
    domain::errors::ErrorCode,
    domain::{
        Lifecycle, OperationStatus, RoamingConfig, RoamingMode, RoamingSensitivity,
        SetWifiConfigRequest, WifiNetworkConfig,
    },
    infrastructure::backend::FailurePlan,
};

fn test_app() -> (Arc<App>, Arc<MemoryBackend>) {
    let backend = Arc::new(MemoryBackend::new(
        "Home",
        &["default_radio0", "default_radio1"],
    ));
    let (tx, _rx) = tokio::sync::broadcast::channel(16);
    let timing = Timing {
        reconcile_interval: Duration::from_millis(20),
        verify_timeout: Duration::from_millis(100),
        verify_sample_delay: Duration::from_millis(2),
        rollback_verify_timeout: Duration::from_millis(50),
        rpcd_rollback_timeout_secs: 2,
    };
    let root = std::env::temp_dir().join(format!(
        "unetic-test-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    let app = App::bootstrap_with_timing(backend.clone(), StateStore::new(root), tx, timing);
    (app, backend)
}

fn wait_for_idle(app: &App) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        if app.state().active_operation.is_none() {
            return;
        }
        thread::sleep(Duration::from_millis(5));
    }
    panic!("operation did not finish");
}

fn wifi_req(
    ssid: &str,
    encryption: &str,
    key: Option<&str>,
    expected_revision: u64,
    request_id: &str,
) -> SetWifiConfigRequest {
    SetWifiConfigRequest::new(
        ssid,
        encryption,
        key.map(Into::into),
        expected_revision,
        request_id,
    )
}

#[test]
fn successful_change_updates_desired_and_router() {
    let (app, backend) = test_app();
    let accepted = app
        .wifi_set_config(wifi_req("New Home", "none", None, 1, "request-1"))
        .expect("accepted");
    assert_eq!(accepted.status, OperationStatus::Accepted);

    wait_for_idle(&app);
    let state = app.state();
    assert_eq!(state.wifi.ssid, "New Home");
    assert_eq!(state.wifi.encryption, "none");
    assert_eq!(state.revision, 2);
    assert_eq!(
        state
            .last_user_operation
            .as_ref()
            .expect("last operation")
            .status,
        OperationStatus::Succeeded
    );
    assert!(
        backend
            .committed_ssids()
            .values()
            .all(|ssid| ssid == "New Home")
    );
}

#[test]
fn successful_wifi_config_with_encryption_and_password() {
    let (app, backend) = test_app();
    let accepted = app
        .wifi_set_config(wifi_req(
            "SecureHome",
            "psk2",
            Some("supersecret123"),
            1,
            "request-sec-1",
        ))
        .expect("accepted");
    assert_eq!(accepted.status, OperationStatus::Accepted);

    wait_for_idle(&app);
    let state = app.state();
    assert_eq!(state.wifi.ssid, "SecureHome");
    assert_eq!(state.wifi.encryption, "psk2");
    assert_eq!(state.wifi.key.as_deref(), Some("supersecret123"));
    assert_eq!(state.revision, 2);

    let committed = backend.committed_configs();
    for config in committed.values() {
        assert_eq!(config.ssid, "SecureHome");
        assert_eq!(config.encryption, "psk2");
        assert_eq!(config.key.as_deref(), Some("supersecret123"));
    }
}

#[test]
fn validation_encryption_requires_valid_key() {
    let (app, _) = test_app();

    let missing_key = app
        .wifi_set_config(wifi_req("SecureHome", "psk2", None, 1, "req-missing-key"))
        .expect_err("must reject missing key for non-none encryption");
    assert_eq!(
        missing_key,
        unetic_core::application::app::handlers::WifiSetError::InvalidWifiConfig
    );

    let short_key = app
        .wifi_set_config(wifi_req(
            "SecureHome",
            "psk2",
            Some("short"),
            1,
            "req-short-key",
        ))
        .expect_err("must reject key shorter than 8 chars");
    assert_eq!(
        short_key,
        unetic_core::application::app::handlers::WifiSetError::InvalidWifiConfig
    );

    let long_key = app
        .wifi_set_config(SetWifiConfigRequest::new(
            "SecureHome",
            "psk2",
            Some("a".repeat(64)),
            1,
            "req-long-key",
        ))
        .expect_err("must reject key longer than 63 chars");
    assert_eq!(
        long_key,
        unetic_core::application::app::handlers::WifiSetError::InvalidWifiConfig
    );

    let valid_8 = app
        .wifi_set_config(wifi_req(
            "SecureHome",
            "psk2",
            Some("12345678"),
            1,
            "req-valid-8",
        ))
        .expect("accepted 8-char key");
    assert_eq!(valid_8.status, OperationStatus::Accepted);
}

#[test]
fn stale_revision_is_rejected_without_changing_router() {
    let (app, backend) = test_app();
    let error = app
        .wifi_set_config(wifi_req("Other", "none", None, 99, "request-stale"))
        .expect_err("must reject stale revision");

    assert_eq!(
        error,
        unetic_core::application::app::handlers::WifiSetError::ApplyFailed
    );
    assert!(
        backend
            .committed_ssids()
            .values()
            .all(|ssid| ssid == "Home")
    );
}

#[test]
fn apply_failure_returns_authoritative_old_state() {
    let (app, backend) = test_app();
    backend.set_failure_plan(FailurePlan {
        fail_apply: true,
        ..FailurePlan::default()
    });

    app.wifi_set_config(wifi_req("Broken", "none", None, 1, "request-fail"))
        .expect("accepted");

    wait_for_idle(&app);
    let state = app.state();
    assert_eq!(state.wifi.ssid, "Home");
    assert_eq!(state.revision, 1);
    assert_eq!(
        state
            .last_user_operation
            .as_ref()
            .expect("last operation")
            .status,
        OperationStatus::Failed
    );
    assert!(
        backend
            .committed_ssids()
            .values()
            .all(|ssid| ssid == "Home")
    );
}

#[test]
fn maintenance_suppresses_repair_then_exit_repairs_drift() {
    let (app, backend) = test_app();
    app.start_background();
    app.maintenance_enter(Some("test".into()))
        .expect("enter maintenance");
    backend.external_set_ssid("default_radio0", "Manual");

    thread::sleep(Duration::from_millis(80));
    assert_eq!(
        backend
            .committed_ssids()
            .get("default_radio0")
            .map(String::as_str),
        Some("Manual")
    );
    assert_eq!(app.state().lifecycle, Lifecycle::Maintenance);

    let state = app.maintenance_exit().expect("exit maintenance");
    assert!(state.maintenance.enabled);
    assert!(state.maintenance.exiting);

    let deadline = Instant::now() + Duration::from_secs(1);
    while Instant::now() < deadline {
        let state = app.state();
        if state.lifecycle == Lifecycle::Ready
            && !state.maintenance.enabled
            && backend
                .committed_ssids()
                .values()
                .all(|ssid| ssid == "Home")
        {
            app.shutdown();
            return;
        }
        thread::sleep(Duration::from_millis(10));
    }
    app.shutdown();
    panic!("maintenance exit did not reconcile");
}

#[test]
fn same_request_id_is_idempotent_while_active_or_finished() {
    let (app, _) = test_app();
    let request = wifi_req("One", "none", None, 1, "same-id");
    let first = app
        .wifi_set_config(request.clone())
        .expect("first accepted");
    let second = app.wifi_set_config(request).expect("duplicate accepted");
    assert_eq!(first.operation_id, second.operation_id);

    wait_for_idle(&app);
    let duplicate = app
        .wifi_set_config(wifi_req("One", "none", None, 1, "same-id"))
        .expect("finished duplicate");
    assert_eq!(first.operation_id, duplicate.operation_id);
}

#[test]
fn reusing_request_id_for_different_intent_is_rejected() {
    let (app, _) = test_app();
    let request = wifi_req("One", "none", None, 1, "same-id-different-intent");
    app.wifi_set_config(request).expect("first accepted");

    let error = app
        .wifi_set_config(wifi_req("Two", "none", None, 1, "same-id-different-intent"))
        .expect_err("different intent must not reuse request id");
    assert_eq!(
        error,
        unetic_core::application::app::handlers::WifiSetError::ApplyFailed
    );
}

#[test]
fn reusing_request_id_with_same_ssid_and_different_key_is_rejected() {
    let (app, _) = test_app();
    app.wifi_set_config(wifi_req(
        "Home",
        "psk2",
        Some("first-password"),
        1,
        "same-id-different-key",
    ))
    .expect("first accepted");

    let error = app
        .wifi_set_config(wifi_req(
            "Home",
            "psk2",
            Some("second-password"),
            1,
            "same-id-different-key",
        ))
        .expect_err("different key must not reuse request id");

    assert_eq!(
        error,
        unetic_core::application::app::handlers::WifiSetError::ApplyFailed
    );
}

#[test]
fn noop_request_id_cannot_be_reused_for_different_intent() {
    let (app, _) = test_app();
    let first = app
        .wifi_set_config(wifi_req("Home", "none", None, 1, "noop-id"))
        .expect("noop accepted");
    assert!(first.noop);

    let error = app
        .wifi_set_config(wifi_req("Other", "none", None, 1, "noop-id"))
        .expect_err("noop request ID remains reserved");

    assert_eq!(
        error,
        unetic_core::application::app::handlers::WifiSetError::ApplyFailed
    );
}

#[test]
fn old_wifi_request_preserves_current_roaming_profile() {
    let (app, backend) = test_app();
    let roaming = RoamingConfig {
        mode: RoamingMode::Aggressive,
        sensitivity: RoamingSensitivity::High,
    };
    let mut first = wifi_req("Home", "none", None, 1, "roaming-profile");
    first.roaming = Some(roaming);
    app.wifi_set_config(first)
        .expect("roaming profile accepted");
    wait_for_idle(&app);

    app.wifi_set_config(wifi_req("New Home", "none", None, 2, "old-client"))
        .expect("old client request accepted");
    wait_for_idle(&app);

    assert_eq!(app.state().wifi.roaming, roaming);
    assert_eq!(backend.committed_roaming(), roaming);
}

#[test]
fn roaming_policy_drift_is_reconciled_without_revision_change() {
    let (app, backend) = test_app();
    app.start_background();
    backend.external_set_roaming(RoamingConfig {
        mode: RoamingMode::Aggressive,
        sensitivity: RoamingSensitivity::High,
    });

    let deadline = Instant::now() + Duration::from_secs(1);
    while Instant::now() < deadline {
        if backend.committed_roaming() == RoamingConfig::default()
            && app.state().revision == 1
            && !app.state().drift.detected
        {
            app.shutdown();
            return;
        }
        thread::sleep(Duration::from_millis(10));
    }
    app.shutdown();
    panic!("roaming drift was not reconciled");
}

#[test]
fn runtime_only_drift_is_repaired_without_revision_change() {
    let (app, backend) = test_app();
    app.start_background();
    backend.set_failure_plan(FailurePlan {
        runtime_unhealthy: true,
        ..FailurePlan::default()
    });
    assert!(
        !backend
            .runtime_healthy(&["default_radio0".into()], "Home")
            .expect("runtime state")
    );

    let deadline = Instant::now() + Duration::from_secs(1);
    while Instant::now() < deadline {
        let state = app.state();
        if backend
            .runtime_healthy(&["default_radio0".into()], "Home")
            .expect("runtime state")
            && state.health.wireless == "ok"
            && state.revision == 1
        {
            app.shutdown();
            return;
        }
        thread::sleep(Duration::from_millis(10));
    }
    app.shutdown();
    panic!("runtime-only drift was not repaired");
}

#[test]
fn api_keeps_domain_errors_in_structured_success_payload() {
    let (app, _) = test_app();
    let raw = unetic_core::api::dispatch(
        &app,
        "wifi.set_config",
        r#"{"idempotence_token":"xyz","ssid":"","expected_revision":1,"request_id":"bad"}"#,
    );
    let value: serde_json::Value = serde_json::from_str(&raw).expect("valid JSON envelope");
    assert_eq!(
        value.get("error").and_then(serde_json::Value::as_u64),
        Some(1)
    );
}

#[test]
fn crash_recovery_reports_interrupted_uncommitted_user_operation() {
    use unetic_core::domain::{
        DesiredConfig, OperationSource, STATE_SCHEMA_VERSION, TransactionJournal,
    };

    let backend = Arc::new(MemoryBackend::new(
        "Home",
        &["default_radio0", "default_radio1"],
    ));
    let root = std::env::temp_dir().join(format!(
        "unetic-recovery-old-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    let store = StateStore::new(root);
    store
        .persist_config(&DesiredConfig::new(
            WifiNetworkConfig {
                ssid: "Home".into(),
                encryption: "none".into(),
                key: None,
                targets: vec!["default_radio0".into(), "default_radio1".into()],
            },
            unetic_core::domain::WanDesired::default(),
        ))
        .expect("desired state");
    store
        .persist_transaction(&TransactionJournal {
            schema_version: STATE_SCHEMA_VERSION,
            operation_id: "op-crashed".into(),
            request_id: "req-crashed".into(),
            source: OperationSource::User,
            base_revision: 1,
            target_revision: 2,
            kind: Default::default(),
            old_ssid: "Home".into(),
            new_ssid: "New".into(),
            old_encryption: "none".into(),
            new_encryption: "none".into(),
            old_key: None,
            new_key: None,
            old_roaming: Default::default(),
            new_roaming: Default::default(),
            targets: vec!["default_radio0".into(), "default_radio1".into()],
            old_wan: None,
            new_wan: None,
            phase: OperationStatus::Applying,
        })
        .expect("journal");

    let (tx, _rx) = tokio::sync::broadcast::channel(16);
    let app = App::bootstrap(backend, store, tx);
    let last = app.state().last_user_operation.expect("recovered result");
    assert_eq!(last.status, OperationStatus::Failed);
    assert_eq!(
        last.error.expect("interruption error").code,
        ErrorCode::OperationInterrupted
    );
}

#[test]
fn crash_recovery_finishes_durable_user_intent() {
    use unetic_core::domain::{
        DesiredConfig, OperationSource, STATE_SCHEMA_VERSION, TransactionJournal,
    };

    let backend = Arc::new(MemoryBackend::new(
        "New",
        &["default_radio0", "default_radio1"],
    ));
    let root = std::env::temp_dir().join(format!(
        "unetic-recovery-new-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    let store = StateStore::new(root);
    let mut desired = DesiredConfig::new(
        WifiNetworkConfig {
            ssid: "New".into(),
            encryption: "none".into(),
            key: None,
            targets: vec!["default_radio0".into(), "default_radio1".into()],
        },
        unetic_core::domain::WanDesired::default(),
    );
    desired.revision = 2;
    store.persist_config(&desired).expect("desired state");
    store
        .persist_transaction(&TransactionJournal {
            schema_version: STATE_SCHEMA_VERSION,
            operation_id: "op-durable".into(),
            request_id: "req-durable".into(),
            source: OperationSource::User,
            base_revision: 1,
            target_revision: 2,
            kind: Default::default(),
            old_ssid: "Home".into(),
            new_ssid: "New".into(),
            old_encryption: "none".into(),
            new_encryption: "none".into(),
            old_key: None,
            new_key: None,
            old_roaming: Default::default(),
            new_roaming: Default::default(),
            targets: vec!["default_radio0".into(), "default_radio1".into()],
            old_wan: None,
            new_wan: None,
            phase: OperationStatus::Confirming,
        })
        .expect("journal");

    let (tx, _rx) = tokio::sync::broadcast::channel(16);
    let app = App::bootstrap(backend, store, tx);
    let last = app.state().last_user_operation.expect("recovered result");
    assert_eq!(last.status, OperationStatus::Succeeded);
    assert_eq!(last.revision, 2);
}

#[test]
fn validation_rejects_invalid_ssids_and_handles_noop() {
    let (app, _) = test_app();

    let empty_err = app
        .wifi_set_config(wifi_req("", "none", None, 1, "req-empty"))
        .expect_err("empty SSID rejected");
    assert_eq!(
        empty_err,
        unetic_core::application::app::handlers::WifiSetError::InvalidWifiConfig
    );

    let too_long_err = app
        .wifi_set_config(SetWifiConfigRequest::new(
            "a".repeat(33),
            "none",
            None,
            1,
            "req-too-long",
        ))
        .expect_err("33-byte SSID rejected");
    assert_eq!(
        too_long_err,
        unetic_core::application::app::handlers::WifiSetError::InvalidWifiConfig
    );

    let nul_err = app
        .wifi_set_config(wifi_req("Hello\0World", "none", None, 1, "req-nul"))
        .expect_err("NUL byte SSID rejected");
    assert_eq!(
        nul_err,
        unetic_core::application::app::handlers::WifiSetError::InvalidWifiConfig
    );

    let valid_32 = app
        .wifi_set_config(SetWifiConfigRequest::new(
            "a".repeat(32),
            "none",
            None,
            1,
            "req-32",
        ))
        .expect("32-byte SSID accepted");
    assert_eq!(valid_32.status, OperationStatus::Accepted);
    assert!(!valid_32.noop);

    wait_for_idle(&app);

    let noop = app
        .wifi_set_config(SetWifiConfigRequest::new(
            "a".repeat(32),
            "none",
            None,
            2,
            "req-noop",
        ))
        .expect("same SSID is accepted as no-op");
    assert_eq!(noop.status, OperationStatus::Succeeded);
    assert!(noop.noop);
    assert_eq!(app.state().revision, 2);
}

#[test]
fn stage_failure_leaves_desired_and_router_unchanged() {
    let (app, backend) = test_app();
    backend.set_failure_plan(FailurePlan {
        fail_stage: true,
        ..FailurePlan::default()
    });

    app.wifi_set_config(wifi_req("StageFail", "none", None, 1, "req-stage-fail"))
        .expect("accepted");

    wait_for_idle(&app);
    let state = app.state();
    assert_eq!(state.wifi.ssid, "Home");
    assert_eq!(state.revision, 1);
    assert_eq!(
        state
            .last_user_operation
            .as_ref()
            .expect("last operation")
            .status,
        OperationStatus::Failed
    );
    assert!(
        backend
            .committed_ssids()
            .values()
            .all(|ssid| ssid == "Home")
    );
}

#[test]
fn verify_failure_rolls_back_to_old_state() {
    let (app, backend) = test_app();
    backend.set_failure_plan(FailurePlan {
        fail_candidate_verify: true,
        ..FailurePlan::default()
    });

    app.wifi_set_config(wifi_req("VerifyFail", "none", None, 1, "req-verify-fail"))
        .expect("accepted");

    wait_for_idle(&app);
    let state = app.state();
    assert_eq!(state.wifi.ssid, "Home");
    assert_eq!(state.revision, 1);
    assert_eq!(
        state
            .last_user_operation
            .as_ref()
            .expect("last operation")
            .status,
        OperationStatus::Failed
    );
    assert!(
        backend
            .committed_ssids()
            .values()
            .all(|ssid| ssid == "Home")
    );
}

#[test]
fn rollback_failure_transitions_to_degraded() {
    let (app, backend) = test_app();
    backend.set_failure_plan(FailurePlan {
        runtime_unhealthy: true,
        fail_rollback: true,
        ..FailurePlan::default()
    });

    app.wifi_set_config(wifi_req("RollbackFail", "none", None, 1, "req-rb-fail"))
        .expect("accepted");

    wait_for_idle(&app);
    let state = app.state();
    assert_eq!(state.lifecycle, Lifecycle::Degraded);
    assert_eq!(
        state
            .last_user_operation
            .as_ref()
            .expect("last operation")
            .status,
        OperationStatus::RollbackFailed
    );
}

#[test]
fn bootstrap_without_targets_sets_lifecycle_needs_setup() {
    let backend = Arc::new(MemoryBackend::new("Home", &[]));
    let (tx, _rx) = tokio::sync::broadcast::channel(16);
    let root = std::env::temp_dir().join(format!(
        "unetic-test-bootstrap-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    let app = App::bootstrap(backend, StateStore::new(root), tx);
    assert_eq!(app.state().lifecycle, Lifecycle::NeedsSetup);
}

#[test]
fn concurrency_rejects_second_concurrent_operation_with_busy() {
    let (app, _) = test_app();
    let first = app
        .wifi_set_config(wifi_req("First", "none", None, 1, "req-first"))
        .expect("first accepted");
    assert_eq!(first.status, OperationStatus::Accepted);

    let second_err = app
        .wifi_set_config(wifi_req("Second", "none", None, 1, "req-second"))
        .expect_err("second operation must be rejected with BUSY while first is active");
    assert_eq!(
        second_err,
        unetic_core::application::app::handlers::WifiSetError::NotReady
    );

    wait_for_idle(&app);
}
