use crate::domain::{
    MAX_WAN_QOS_KBPS, WanProtocol, WanQos,
    errors::{ErrorCode, ErrorStage, LegacyAppError},
};

pub(super) fn validate_qos(
    qos: &WanQos,
    wan_present: bool,
    proto: WanProtocol,
) -> Result<(), LegacyAppError> {
    if !wan_present || proto == WanProtocol::None {
        return invalid_qos("QoS cannot be configured when WAN is disabled");
    }
    if proto == WanProtocol::Extender {
        return invalid_qos("QoS cannot be configured on WAN in extender mode (master only)");
    }
    if !qos.enabled {
        return Ok(());
    }
    if qos.download_kbps.is_none() && qos.upload_kbps.is_none() {
        return invalid_qos(
            "At least one bandwidth limit (download or upload) must be specified when QoS is enabled",
        );
    }

    validate_limit(qos.download_kbps, "download")?;
    validate_limit(qos.upload_kbps, "upload")
}

fn validate_limit(limit: Option<u32>, direction: &str) -> Result<(), LegacyAppError> {
    let Some(value) = limit else {
        return Ok(());
    };
    if (1..=MAX_WAN_QOS_KBPS).contains(&value) {
        return Ok(());
    }

    invalid_qos(format!(
        "QoS {direction} bandwidth limit must be between 1 and {MAX_WAN_QOS_KBPS} kbps"
    ))
}

fn invalid_qos(message: impl Into<String>) -> Result<(), LegacyAppError> {
    Err(LegacyAppError::new(
        ErrorCode::InvalidArgument,
        ErrorStage::Validate,
        message,
    ))
}
