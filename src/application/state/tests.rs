use super::*;
use crate::domain::wifi::{MeshBackhaulConfig, RadioChannelConfig};

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
