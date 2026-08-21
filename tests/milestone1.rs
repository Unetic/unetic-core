#![allow(clippy::pedantic)]

use std::{
    sync::{Arc, mpsc},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use unetic_core::{
    App, MemoryBackend, RouterBackend, StateStore, Timing,
    domain::errors::ErrorCode,
    domain::{Lifecycle, OperationStatus, SetWifiConfigRequest, WifiNetworkConfig},
    infrastructure::backend::FailurePlan,
};

fn test_app() -> (Arc<App>, Arc<MemoryBackend>) {
    let backend = Arc::new(MemoryBackend::new(
        "Home",
        &["default_radio0", "default_radio1"],
    ));
    let (tx, _rx) = mpsc::channel();
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

#[test]
fn successful_change_updates_desired_and_router() {
    let (app, backend) = test_app();
    let accepted = app
        .wifi_set_config(SetWifiConfigRequest {
            ssid: "New Home".into(),
            encryption: "none".into(),
            key: None,
            expected_revision: 1,
            request_id: "request-1".into(),
        })
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
        .wifi_set_config(SetWifiConfigRequest {
            ssid: "SecureHome".into(),
            encryption: "psk2".into(),
            key: Some("supersecret123".into()),
            expected_revision: 1,
            request_id: "request-sec-1".into(),
        })
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
        .wifi_set_config(SetWifiConfigRequest {
            ssid: "SecureHome".into(),
            encryption: "psk2".into(),
            key: None,
            expected_revision: 1,
            request_id: "req-missing-key".into(),
        })
        .expect_err("must reject missing key for non-none encryption");
    assert_eq!(missing_key.code, ErrorCode::InvalidArgument);

    let short_key = app
        .wifi_set_config(SetWifiConfigRequest {
            ssid: "SecureHome".into(),
            encryption: "psk2".into(),
            key: Some("short".into()),
            expected_revision: 1,
            request_id: "req-short-key".into(),
        })
        .expect_err("must reject key shorter than 8 chars");
    assert_eq!(short_key.code, ErrorCode::InvalidArgument);

    let long_key = app
        .wifi_set_config(SetWifiConfigRequest {
            ssid: "SecureHome".into(),
            encryption: "psk2".into(),
            key: Some("a".repeat(64)),
            expected_revision: 1,
            request_id: "req-long-key".into(),
        })
        .expect_err("must reject key longer than 63 chars");
    assert_eq!(long_key.code, ErrorCode::InvalidArgument);

    let valid_8 = app
        .wifi_set_config(SetWifiConfigRequest {
            ssid: "SecureHome".into(),
            encryption: "psk2".into(),
            key: Some("12345678".into()),
            expected_revision: 1,
            request_id: "req-valid-8".into(),
        })
        .expect("accepted 8-char key");
    assert_eq!(valid_8.status, OperationStatus::Accepted);
}

#[test]
fn stale_revision_is_rejected_without_changing_router() {
    let (app, backend) = test_app();
    let error = app
        .wifi_set_config(SetWifiConfigRequest {
            ssid: "Other".into(),
            encryption: "none".into(),
            key: None,
            expected_revision: 99,
            request_id: "request-stale".into(),
        })
        .expect_err("must reject stale revision");

    assert_eq!(error.code, ErrorCode::RevisionConflict);
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

    app.wifi_set_config(SetWifiConfigRequest {
        ssid: "Broken".into(),
        encryption: "none".into(),
        key: None,
        expected_revision: 1,
        request_id: "request-fail".into(),
    })
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
    let request = SetWifiConfigRequest {
        ssid: "One".into(),
        encryption: "none".into(),
        key: None,
        expected_revision: 1,
        request_id: "same-id".into(),
    };
    let first = app
        .wifi_set_config(request.clone())
        .expect("first accepted");
    let second = app.wifi_set_config(request).expect("duplicate accepted");
    assert_eq!(first.operation_id, second.operation_id);

    wait_for_idle(&app);
    let duplicate = app
        .wifi_set_config(SetWifiConfigRequest {
            ssid: "One".into(),
            encryption: "none".into(),
            key: None,
            expected_revision: 1,
            request_id: "same-id".into(),
        })
        .expect("finished duplicate");
    assert_eq!(first.operation_id, duplicate.operation_id);
}

#[test]
fn reusing_request_id_for_different_intent_is_rejected() {
    let (app, _) = test_app();
    let request = SetWifiConfigRequest {
        ssid: "One".into(),
        encryption: "none".into(),
        key: None,
        expected_revision: 1,
        request_id: "same-id-different-intent".into(),
    };
    app.wifi_set_config(request).expect("first accepted");

    let error = app
        .wifi_set_config(SetWifiConfigRequest {
            ssid: "Two".into(),
            encryption: "none".into(),
            key: None,
            expected_revision: 1,
            request_id: "same-id-different-intent".into(),
        })
        .expect_err("different intent must not reuse request id");
    assert_eq!(error.code, ErrorCode::IdempotencyConflict);
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
        r#"{"ssid":"","expected_revision":1,"request_id":"bad"}"#,
    );
    let value: serde_json::Value = serde_json::from_str(&raw).expect("valid JSON envelope");
    assert_eq!(
        value.get("ok").and_then(serde_json::Value::as_bool),
        Some(false)
    );
    assert_eq!(
        value
            .pointer("/error/code")
            .and_then(serde_json::Value::as_str),
        Some("INVALID_ARGUMENT")
    );
    assert!(value.get("state").is_some());
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
            old_ssid: "Home".into(),
            new_ssid: "New".into(),
            old_encryption: "none".into(),
            new_encryption: "none".into(),
            old_key: None,
            new_key: None,
            targets: vec!["default_radio0".into(), "default_radio1".into()],
            phase: OperationStatus::Applying,
        })
        .expect("journal");

    let (tx, _rx) = mpsc::channel();
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
            old_ssid: "Home".into(),
            new_ssid: "New".into(),
            old_encryption: "none".into(),
            new_encryption: "none".into(),
            old_key: None,
            new_key: None,
            targets: vec!["default_radio0".into(), "default_radio1".into()],
            phase: OperationStatus::Confirming,
        })
        .expect("journal");

    let (tx, _rx) = mpsc::channel();
    let app = App::bootstrap(backend, store, tx);
    let last = app.state().last_user_operation.expect("recovered result");
    assert_eq!(last.status, OperationStatus::Succeeded);
    assert_eq!(last.revision, 2);
}

#[test]
fn validation_rejects_invalid_ssids_and_handles_noop() {
    let (app, _) = test_app();

    let empty_err = app
        .wifi_set_config(SetWifiConfigRequest {
            ssid: "".into(),
            encryption: "none".into(),
            key: None,
            expected_revision: 1,
            request_id: "req-empty".into(),
        })
        .expect_err("empty SSID rejected");
    assert_eq!(empty_err.code, ErrorCode::InvalidArgument);

    let too_long_err = app
        .wifi_set_config(SetWifiConfigRequest {
            ssid: "a".repeat(33),
            encryption: "none".into(),
            key: None,
            expected_revision: 1,
            request_id: "req-too-long".into(),
        })
        .expect_err("33-byte SSID rejected");
    assert_eq!(too_long_err.code, ErrorCode::InvalidArgument);

    let nul_err = app
        .wifi_set_config(SetWifiConfigRequest {
            ssid: "Hello\0World".into(),
            encryption: "none".into(),
            key: None,
            expected_revision: 1,
            request_id: "req-nul".into(),
        })
        .expect_err("NUL byte SSID rejected");
    assert_eq!(nul_err.code, ErrorCode::InvalidArgument);

    let valid_32 = app
        .wifi_set_config(SetWifiConfigRequest {
            ssid: "a".repeat(32),
            encryption: "none".into(),
            key: None,
            expected_revision: 1,
            request_id: "req-32".into(),
        })
        .expect("32-byte SSID accepted");
    assert_eq!(valid_32.status, OperationStatus::Accepted);
    assert!(!valid_32.noop);

    wait_for_idle(&app);

    let noop = app
        .wifi_set_config(SetWifiConfigRequest {
            ssid: "a".repeat(32),
            encryption: "none".into(),
            key: None,
            expected_revision: 2,
            request_id: "req-noop".into(),
        })
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

    app.wifi_set_config(SetWifiConfigRequest {
        ssid: "StageFail".into(),
        encryption: "none".into(),
        key: None,
        expected_revision: 1,
        request_id: "req-stage-fail".into(),
    })
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

    app.wifi_set_config(SetWifiConfigRequest {
        ssid: "VerifyFail".into(),
        encryption: "none".into(),
        key: None,
        expected_revision: 1,
        request_id: "req-verify-fail".into(),
    })
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

    app.wifi_set_config(SetWifiConfigRequest {
        ssid: "RollbackFail".into(),
        encryption: "none".into(),
        key: None,
        expected_revision: 1,
        request_id: "req-rb-fail".into(),
    })
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
    let (tx, _rx) = mpsc::channel();
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
        .wifi_set_config(SetWifiConfigRequest {
            ssid: "First".into(),
            encryption: "none".into(),
            key: None,
            expected_revision: 1,
            request_id: "req-first".into(),
        })
        .expect("first accepted");
    assert_eq!(first.status, OperationStatus::Accepted);

    let second_err = app
        .wifi_set_config(SetWifiConfigRequest {
            ssid: "Second".into(),
            encryption: "none".into(),
            key: None,
            expected_revision: 1,
            request_id: "req-second".into(),
        })
        .expect_err("second operation must be rejected with BUSY while first is active");
    assert_eq!(second_err.code, ErrorCode::Busy);

    wait_for_idle(&app);
}
