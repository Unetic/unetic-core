use super::*;
use crate::application::app::StateTopic;
use crate::domain::wifi::{MeshBackhaulConfig, RadioChannelConfig};
use crate::{App, MemoryBackend, StateStore};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

#[test]
fn coalesces_multiple_state_changes_into_one_event() {
    let backend = Arc::new(MemoryBackend::new("Home", &["radio0"]));
    let (event_tx, mut event_rx) = tokio::sync::broadcast::channel(8);
    let store = StateStore::new(
        std::env::temp_dir().join(format!("unetic-state-publisher-{}", generate_id("test"))),
    );
    let app = App::bootstrap(backend, store, event_tx);

    app.flush_state_update();
    let initial = event_rx.try_recv().expect("initial state event");

    app.publish(StateTopic::Wifi);
    app.publish(StateTopic::Wifi);
    assert!(event_rx.try_recv().is_err());

    app.flush_state_update();
    let coalesced = event_rx.try_recv().expect("coalesced state event");
    assert_eq!(coalesced.event_seq, initial.event_seq + 1);
    assert!(event_rx.try_recv().is_err());
}

#[test]
fn state_includes_system_info() {
    let backend = Arc::new(MemoryBackend::new("Home", &["radio0"]));
    let (event_tx, _event_rx) = tokio::sync::broadcast::channel(8);
    let store = StateStore::new(
        std::env::temp_dir().join(format!("unetic-system-info-state-{}", generate_id("test"))),
    );
    let app = App::bootstrap(backend, store, event_tx);

    let state = app.state();
    assert_eq!(state.system.info.hostname, "OpenWrt");
    assert_eq!(state.system.info.model, "MediaTek MT7981B (Filogic 820)");
    assert_eq!(state.system.info.cpu_count, 2);
}

#[test]
fn flushes_pending_state_early_when_another_update_arrives() {
    let backend = Arc::new(MemoryBackend::new("Home", &["radio0"]));
    let (event_tx, mut event_rx) = tokio::sync::broadcast::channel(8);
    let store = StateStore::new(
        std::env::temp_dir().join(format!("unetic-early-state-flush-{}", generate_id("test"))),
    );
    let app = App::bootstrap(backend, store, event_tx);

    app.flush_state_update();
    let initial = event_rx.try_recv().expect("initial state event");

    app.publish(StateTopic::Wifi);
    app.state_updates
        .lock()
        .expect("state updates poisoned")
        .last_sent_at = Some(Instant::now() - Duration::from_secs(1));
    app.publish(StateTopic::Wifi);

    let early = event_rx.try_recv().expect("early state event");
    assert_eq!(early.event_seq, initial.event_seq + 1);

    app.flush_state_update();
    let pending = event_rx.try_recv().expect("pending state event");
    assert_eq!(pending.event_seq, early.event_seq + 1);
}

#[test]
fn system_runtime_waits_for_the_regular_flush() {
    let backend = Arc::new(MemoryBackend::new("Home", &["radio0"]));
    let (event_tx, mut event_rx) = tokio::sync::broadcast::channel(8);
    let store = StateStore::new(
        std::env::temp_dir().join(format!("unetic-runtime-flush-{}", generate_id("test"))),
    );
    let app = App::bootstrap(backend, store, event_tx);

    app.flush_state_update();
    let _ = event_rx.try_recv();
    app.publish(StateTopic::Wifi);
    app.state_updates
        .lock()
        .expect("state updates poisoned")
        .last_sent_at = Some(Instant::now() - Duration::from_secs(1));
    app.publish_system_runtime();

    assert!(event_rx.try_recv().is_err());
    app.flush_state_update();
    assert!(event_rx.try_recv().is_ok());
}

#[test]
fn test_validate_backhaul_disabled_succeeds() {
    let backhaul = MeshBackhaulConfig {
        enabled: false,
        backhaul_target: "radio0".into(),
        client_target: "radio0".into(),
        hidden: true,
    };
    let targets = vec!["radio0".into()];
    assert!(validate_mesh_backhaul_config(&backhaul, &targets, &[]).is_ok());
}

#[test]
fn test_validate_backhaul_rejects_single_radio() {
    let backhaul = MeshBackhaulConfig {
        enabled: true,
        backhaul_target: "radio0".into(),
        client_target: "radio1".into(),
        hidden: true,
    };
    let targets = vec!["radio0".into()];
    let err = validate_mesh_backhaul_config(&backhaul, &targets, &[]).unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidArgument);
    assert!(err.message.contains("Dual-radio hardware"));
}

#[test]
fn test_validate_backhaul_rejects_same_target() {
    let backhaul = MeshBackhaulConfig {
        enabled: true,
        backhaul_target: "radio0".into(),
        client_target: "radio0".into(),
        hidden: true,
    };
    let targets = vec!["radio0".into(), "radio1".into()];
    let err = validate_mesh_backhaul_config(&backhaul, &targets, &[]).unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidArgument);
    assert!(err.message.contains("must be different"));
}

#[test]
fn test_validate_backhaul_rejects_same_channel() {
    let backhaul = MeshBackhaulConfig {
        enabled: true,
        backhaul_target: "radio1".into(),
        client_target: "radio0".into(),
        hidden: true,
    };
    let targets = vec!["radio0".into(), "radio1".into()];
    let channels = vec![
        RadioChannelConfig {
            target: "radio0".into(),
            channel: 6,
            band: Some("2.4g".into()),
        },
        RadioChannelConfig {
            target: "radio1".into(),
            channel: 6,
            band: Some("2.4g".into()),
        },
    ];
    let err = validate_mesh_backhaul_config(&backhaul, &targets, &channels).unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidArgument);
    assert!(err.message.contains("different channels"));
}

#[test]
fn test_validate_backhaul_success_with_different_channels() {
    let backhaul = MeshBackhaulConfig {
        enabled: true,
        backhaul_target: "radio1".into(),
        client_target: "radio0".into(),
        hidden: true,
    };
    let targets = vec!["radio0".into(), "radio1".into()];
    let channels = vec![
        RadioChannelConfig {
            target: "radio0".into(),
            channel: 6,
            band: Some("2.4g".into()),
        },
        RadioChannelConfig {
            target: "radio1".into(),
            channel: 36,
            band: Some("5g".into()),
        },
    ];
    assert!(validate_mesh_backhaul_config(&backhaul, &targets, &channels).is_ok());
}
