#![allow(clippy::pedantic)]

use std::{
    sync::Arc,
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use serde_json::json;
use unetic_core::{
    App, MemoryBackend, StateStore, Timing, api,
    domain::{OperationStatus, SetWanRequest, WanDesired, WanProtocol, WanStatus},
    openwrt::{build_wan_staging_values, parse_discovered_wan, parse_wan_runtime_status},
};

fn test_app() -> Arc<App> {
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
        "unetic-test-wan-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    App::bootstrap_with_timing(backend, StateStore::new(root), tx, timing)
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
fn test_extender_protocol_serialization() {
    let proto = WanProtocol::Extender;
    let json_val = serde_json::to_value(proto).expect("serialize");
    assert_eq!(json_val, json!("extender"));

    let parsed: WanProtocol = serde_json::from_value(json!("extender")).expect("deserialize");
    assert_eq!(parsed, WanProtocol::Extender);
}

#[test]
fn test_openwrt_parse_discovered_wan_extender() {
    let raw = json!({
        "values": {
            "proto": "extender",
            "device": "eth1",
            "macaddr": "00:11:22:33:44:55",
            "mtu": 1500
        }
    });

    let discovered = parse_discovered_wan(&raw);
    assert!(discovered.present);
    assert_eq!(discovered.proto, WanProtocol::Extender);
    assert_eq!(discovered.device.as_deref(), Some("eth1"));
    assert_eq!(discovered.custom_mac.as_deref(), Some("00:11:22:33:44:55"));
    assert_eq!(discovered.custom_mtu, Some(1500));
}

#[test]
fn test_openwrt_build_wan_staging_values_extender() {
    let desired = WanDesired {
        present: true,
        device: Some("eth1".into()),
        proto: WanProtocol::Extender,
        custom_mac: Some("aa:bb:cc:dd:ee:ff".into()),
        custom_mtu: Some(1400),
        ..WanDesired::default()
    };

    let values = build_wan_staging_values(&desired);
    assert_eq!(
        values.get("proto").and_then(serde_json::Value::as_str),
        Some("dhcp")
    );
    assert_eq!(
        values.get("device").and_then(serde_json::Value::as_str),
        Some("eth1")
    );
    assert_eq!(
        values.get("macaddr").and_then(serde_json::Value::as_str),
        Some("aa:bb:cc:dd:ee:ff")
    );
    assert_eq!(
        values.get("mtu").and_then(serde_json::Value::as_u64),
        Some(1400)
    );
}

#[test]
fn test_openwrt_parse_wan_runtime_status_extender() {
    let raw = json!({
        "proto": "extender",
        "up": true,
        "device": "eth1",
        "uptime": 120
    });

    let status = parse_wan_runtime_status(&raw);
    assert!(status.present);
    assert_eq!(status.proto, WanProtocol::Extender);
    assert_eq!(status.status, WanStatus::Connected);
    assert_eq!(status.uptime_secs, 120);
}

#[test]
fn test_api_set_wan_extender_success() {
    let app = test_app();
    let accepted = app
        .set_wan(SetWanRequest {
            expected_revision: 1,
            request_id: "req-wan-extender-1".into(),
            wan: WanDesired {
                present: true,
                device: Some("eth1".into()),
                proto: WanProtocol::Extender,
                ..WanDesired::default()
            },
        })
        .expect("accepted");
    assert_eq!(accepted.status, OperationStatus::Accepted);

    wait_for_idle(&app);
    let state = app.state();
    assert_eq!(state.revision, 2);
    assert_eq!(
        state
            .last_user_operation
            .as_ref()
            .expect("last operation")
            .status,
        OperationStatus::Succeeded
    );
}

#[test]
fn test_wan_request_id_rejects_different_configuration() {
    let app = test_app();
    app.set_wan(SetWanRequest {
        expected_revision: 1,
        request_id: "same-wan-request".into(),
        wan: WanDesired {
            present: true,
            proto: WanProtocol::Dhcp,
            ..WanDesired::default()
        },
    })
    .expect("first accepted");

    let error = app
        .set_wan(SetWanRequest {
            expected_revision: 1,
            request_id: "same-wan-request".into(),
            wan: WanDesired {
                present: true,
                proto: WanProtocol::Extender,
                ..WanDesired::default()
            },
        })
        .expect_err("different WAN intent rejected");

    assert_eq!(
        error.code,
        unetic_core::domain::errors::ErrorCode::IdempotencyConflict
    );
}

#[test]
fn test_api_dispatch_wan_set_extender() {
    let app = test_app();
    let payload = serde_json::json!({
        "idempotence_token": "xyz",
        "request_id": "req-1",
        "expected_revision": 1,
        "wan": {
            "present": true,
            "proto": "extender"
        }
    })
    .to_string();

    let response = api::dispatch(&app, "wan.set", &payload);
    let val: serde_json::Value = serde_json::from_str(&response).expect("valid json response");
    assert_eq!(
        val.get("error").and_then(serde_json::Value::as_u64),
        Some(0)
    );

    wait_for_idle(&app);
    let state = app.state();
    assert_eq!(state.revision, 2);
}

#[test]
fn test_api_dispatch_wan_set_qos_master_success() {
    let app = test_app();
    let payload = serde_json::json!({
        "idempotence_token": "xyz",
        "request_id": "req-qos-1",
        "expected_revision": 1,
        "wan": {
            "present": true,
            "proto": "dhcp",
            "qos": {
                "enabled": true,
                "download_kbps": 100000,
                "upload_kbps": 20000
            }
        }
    })
    .to_string();

    let response = api::dispatch(&app, "wan.set", &payload);
    let val: serde_json::Value = serde_json::from_str(&response).expect("valid json response");
    assert_eq!(
        val.get("error").and_then(serde_json::Value::as_u64),
        Some(0)
    );

    wait_for_idle(&app);
    let state = app.state();
    assert_eq!(state.revision, 2);
    assert_eq!(state.wan.qos, Some(unetic_core::domain::WanQos {
        enabled: true,
        download_kbps: Some(100000),
        upload_kbps: Some(20000),
    }));
}

#[test]
fn test_api_dispatch_wan_set_qos_extender_rejected() {
    let app = test_app();
    let payload = serde_json::json!({
        "idempotence_token": "xyz",
        "request_id": "req-qos-2",
        "expected_revision": 1,
        "wan": {
            "present": true,
            "proto": "extender",
            "qos": {
                "enabled": true,
                "download_kbps": 100000,
                "upload_kbps": 20000
            }
        }
    })
    .to_string();

    let response = api::dispatch(&app, "wan.set", &payload);
    let val: serde_json::Value = serde_json::from_str(&response).expect("valid json response");
    assert_ne!(
        val.get("error").and_then(serde_json::Value::as_u64),
        Some(0)
    );
}
