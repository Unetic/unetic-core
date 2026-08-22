#![allow(clippy::pedantic)]

use std::{
    sync::Arc,
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use unetic_core::{
    App, MemoryBackend, StateStore, Timing,
    domain::{
        DesiredConfig, ErrorCode, Lifecycle, OperationSource, OperationStatus,
        STATE_SCHEMA_VERSION, SetWanRequest, TransactionJournal, TransactionKind, WanDesired,
        WanProtocol, WifiNetworkConfig,
    },
    infrastructure::backend::FailurePlan,
};

const TARGETS: &[&str] = &["default_radio0", "default_radio1"];

fn disabled_wan() -> WanDesired {
    WanDesired {
        present: false,
        proto: WanProtocol::None,
        ..WanDesired::default()
    }
}

fn dhcp_wan() -> WanDesired {
    WanDesired {
        present: true,
        device: Some("eth1".into()),
        proto: WanProtocol::Dhcp,
        ..WanDesired::default()
    }
}

fn wifi() -> WifiNetworkConfig {
    WifiNetworkConfig {
        ssid: "Home".into(),
        encryption: "none".into(),
        key: None,
        targets: TARGETS.iter().map(|target| (*target).to_owned()).collect(),
    }
}

fn test_store(name: &str) -> StateStore {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    StateStore::new(
        std::env::temp_dir().join(format!("unetic-{name}-{}-{suffix}", std::process::id())),
    )
}

fn test_app(backend: Arc<MemoryBackend>, store: StateStore) -> Arc<App> {
    let (tx, _rx) = tokio::sync::broadcast::channel(16);
    App::bootstrap_with_timing(
        backend,
        store,
        tx,
        Timing {
            reconcile_interval: Duration::from_millis(20),
            verify_timeout: Duration::from_millis(100),
            verify_sample_delay: Duration::from_millis(2),
            rollback_verify_timeout: Duration::from_millis(50),
            rpcd_rollback_timeout_secs: 2,
        },
    )
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

fn wait_for_wan_health(app: &App, expected: &str) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        if app.state().health.wan == expected {
            return;
        }
        thread::sleep(Duration::from_millis(5));
    }
    panic!("WAN health did not become {expected}");
}

fn wan_journal(old_wan: WanDesired, new_wan: WanDesired) -> TransactionJournal {
    TransactionJournal {
        schema_version: STATE_SCHEMA_VERSION,
        operation_id: "op-wan-crashed".into(),
        request_id: "req-wan-crashed".into(),
        source: OperationSource::User,
        base_revision: 1,
        target_revision: 2,
        kind: TransactionKind::Wan,
        old_ssid: String::new(),
        new_ssid: String::new(),
        old_encryption: "none".into(),
        new_encryption: "none".into(),
        old_key: None,
        new_key: None,
        old_roaming: Default::default(),
        new_roaming: Default::default(),
        targets: Vec::new(),
        old_wan: Some(old_wan),
        new_wan: Some(new_wan),
        phase: OperationStatus::Applying,
    }
}

#[test]
fn rejects_a_mismatched_staged_wan_before_apply() {
    let backend = Arc::new(MemoryBackend::with_wan("Home", TARGETS, disabled_wan()));
    let app = test_app(Arc::clone(&backend), test_store("wan-stage-mismatch"));
    backend.set_failure_plan(FailurePlan {
        fail_wan_candidate_verify: true,
        ..FailurePlan::default()
    });

    app.set_wan(SetWanRequest {
        expected_revision: 1,
        request_id: "req-stage-mismatch".into(),
        wan: dhcp_wan(),
    })
    .expect("operation accepted");
    wait_for_idle(&app);

    let state = app.state();
    assert_eq!(state.revision, 1);
    assert_eq!(backend.committed_wan(), disabled_wan());
    assert_eq!(
        state
            .last_user_operation
            .expect("failed operation")
            .error
            .expect("stage mismatch")
            .code,
        ErrorCode::UciStageMismatch
    );
}

#[test]
fn confirm_failure_does_not_explicitly_rollback_durable_wan_intent() {
    let backend = Arc::new(MemoryBackend::with_wan("Home", TARGETS, disabled_wan()));
    let app = test_app(Arc::clone(&backend), test_store("wan-confirm-failure"));
    backend.set_failure_plan(FailurePlan {
        fail_confirm: true,
        ..FailurePlan::default()
    });
    let desired = dhcp_wan();

    app.set_wan(SetWanRequest {
        expected_revision: 1,
        request_id: "req-confirm-failure".into(),
        wan: desired.clone(),
    })
    .expect("operation accepted");
    wait_for_idle(&app);

    let state = app.state();
    assert_eq!(state.revision, 2);
    assert_eq!(backend.committed_wan(), desired);
    assert_eq!(state.lifecycle, Lifecycle::Degraded);
    assert_eq!(
        state.last_system_error.expect("uncertain commit").code,
        ErrorCode::CommitUncertain
    );
}

#[test]
fn runtime_read_failure_preserves_last_known_wan_state() {
    let desired = dhcp_wan();
    let backend = Arc::new(MemoryBackend::with_wan("Home", TARGETS, desired.clone()));
    let app = test_app(Arc::clone(&backend), test_store("wan-runtime-read"));
    backend.set_failure_plan(FailurePlan {
        fail_wan_runtime_read: true,
        ..FailurePlan::default()
    });

    app.start_background();
    wait_for_wan_health(&app, "error");
    app.shutdown();

    let state = app.state();
    assert!(state.wan.present);
    assert_eq!(state.wan.proto, WanProtocol::Dhcp);
    assert_eq!(state.wan.device, desired.device);
}

#[test]
fn crash_recovery_restores_old_wan_when_intent_is_not_durable() {
    let old_wan = disabled_wan();
    let new_wan = dhcp_wan();
    let backend = Arc::new(MemoryBackend::with_wan("Home", TARGETS, new_wan.clone()));
    let store = test_store("wan-recovery-old");
    store
        .persist_config(&DesiredConfig::new(wifi(), old_wan.clone()))
        .expect("old desired state");
    store
        .persist_transaction(&wan_journal(old_wan.clone(), new_wan))
        .expect("WAN journal");

    let app = test_app(Arc::clone(&backend), store);

    assert_eq!(backend.committed_wan(), old_wan);
    let last = app
        .state()
        .last_user_operation
        .expect("recovered operation");
    assert_eq!(last.status, OperationStatus::Failed);
    assert_eq!(
        last.error.expect("interruption error").code,
        ErrorCode::OperationInterrupted
    );
}

#[test]
fn crash_recovery_reapplies_durable_wan_intent() {
    let old_wan = disabled_wan();
    let new_wan = dhcp_wan();
    let backend = Arc::new(MemoryBackend::with_wan("Home", TARGETS, old_wan.clone()));
    let store = test_store("wan-recovery-new");
    let mut desired = DesiredConfig::new(wifi(), new_wan.clone());
    desired.revision = 2;
    store.persist_config(&desired).expect("new desired state");
    store
        .persist_transaction(&wan_journal(old_wan, new_wan.clone()))
        .expect("WAN journal");

    let app = test_app(Arc::clone(&backend), store);

    assert_eq!(backend.committed_wan(), new_wan);
    let last = app
        .state()
        .last_user_operation
        .expect("recovered operation");
    assert_eq!(last.status, OperationStatus::Succeeded);
    assert_eq!(last.revision, 2);
}
