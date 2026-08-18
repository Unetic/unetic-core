#![allow(clippy::pedantic)]

use std::{
    sync::{Arc, mpsc},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use unetic_core::{
    App, MemoryBackend, RouterBackend, StateStore, Timing,
    backend::FailurePlan,
    errors::ErrorCode,
    model::{Lifecycle, OperationStatus, SetSsidRequest},
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
        .set_ssid(SetSsidRequest {
            ssid: "New Home".into(),
            expected_revision: 1,
            request_id: "request-1".into(),
        })
        .expect("accepted");
    assert_eq!(accepted.status, OperationStatus::Accepted);

    wait_for_idle(&app);
    let state = app.state();
    assert_eq!(state.wifi.ssid, "New Home");
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
fn stale_revision_is_rejected_without_changing_router() {
    let (app, backend) = test_app();
    let error = app
        .set_ssid(SetSsidRequest {
            ssid: "Other".into(),
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

    app.set_ssid(SetSsidRequest {
        ssid: "Broken".into(),
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
    backend.external_set("default_radio0", "Manual");

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
    let request = SetSsidRequest {
        ssid: "One".into(),
        expected_revision: 1,
        request_id: "same-id".into(),
    };
    let first = app.set_ssid(request.clone()).expect("first accepted");
    let second = app.set_ssid(request).expect("duplicate accepted");
    assert_eq!(first.operation_id, second.operation_id);

    wait_for_idle(&app);
    let duplicate = app
        .set_ssid(SetSsidRequest {
            ssid: "One".into(),
            expected_revision: 1,
            request_id: "same-id".into(),
        })
        .expect("finished duplicate");
    assert_eq!(first.operation_id, duplicate.operation_id);
}

#[test]
fn reusing_request_id_for_different_intent_is_rejected() {
    let (app, _) = test_app();
    let request = SetSsidRequest {
        ssid: "One".into(),
        expected_revision: 1,
        request_id: "same-id-different-intent".into(),
    };
    app.set_ssid(request).expect("first accepted");

    let error = app
        .set_ssid(SetSsidRequest {
            ssid: "Two".into(),
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
        "wifi.set_ssid",
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
    use unetic_core::model::{
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
            "Home".into(),
            vec!["default_radio0".into(), "default_radio1".into()],
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
    use unetic_core::model::{
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
        "New".into(),
        vec!["default_radio0".into(), "default_radio1".into()],
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
