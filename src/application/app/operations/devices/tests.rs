use std::sync::Arc;

use super::*;
use crate::{MemoryBackend, StateStore};

fn app() -> Arc<App> {
    let backend = Arc::new(MemoryBackend::new("Home", &["radio0"]));
    let (events, _) = tokio::sync::broadcast::channel(16);
    let state_dir = std::env::temp_dir().join(format!("unetic-devices-{}", uuid::Uuid::new_v4()));
    App::bootstrap(backend, StateStore::new(state_dir), events)
}

fn device() -> RegisteredDevice {
    RegisteredDevice {
        uuid: "phone_1".to_owned(),
        mac: "00:11:22:33:44:55".to_owned(),
        name: "Phone".to_owned(),
        is_static_ip: false,
        port_forwards: Vec::new(),
    }
}

#[test]
fn register_device_updates_state_without_deadlocking() {
    let app = app();
    let revision = app.state().revision;

    app.register_device(device()).expect("device registered");

    let state = app.state();
    assert_eq!(state.revision, revision + 1);
    assert_eq!(state.registered_devices, vec![device()]);
}

#[test]
fn update_rejects_uuid_changes() {
    let app = app();
    app.register_device(device()).expect("device registered");
    let mut changed = device();
    changed.uuid = "different".to_owned();

    let error = app
        .update_device("phone_1", changed)
        .expect_err("UUID change rejected");

    assert_eq!(error.code, ErrorCode::InvalidArgument);
}

#[test]
fn update_rejects_mac_changes() {
    let app = app();
    app.register_device(device()).expect("device registered");
    let mut changed = device();
    changed.mac = "66:77:88:99:aa:bb".to_owned();

    let error = app
        .update_device("phone_1", changed)
        .expect_err("MAC change rejected");

    assert_eq!(error.code, ErrorCode::InvalidArgument);
}

#[test]
fn remove_missing_port_forward_does_not_change_revision() {
    let app = app();
    app.register_device(device()).expect("device registered");
    let revision = app.state().revision;

    let error = app
        .remove_port_forward("phone_1", "missing")
        .expect_err("missing rule rejected");

    assert_eq!(error.code, ErrorCode::NotFound);
    assert_eq!(app.state().revision, revision);
}
