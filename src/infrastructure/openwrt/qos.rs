use serde_json::{Map, Value, json};

use super::rpc;
use crate::domain::{
    WanQos,
    errors::{ErrorCode, ErrorStage, LegacyAppError},
};

const SQM_CONFIG: &str = "sqm";
const SQM_SECTION: &str = "wan";
const SQM_QUEUE_TYPE: &str = "queue";
const SQM_QDISC: &str = "cake";
const SQM_SCRIPT: &str = "piece_of_cake.qos";

#[derive(Debug, PartialEq, Eq)]
struct SqmSection {
    section_type: Option<String>,
    interface: Option<String>,
    qdisc: Option<String>,
    script: Option<String>,
    qos: WanQos,
}

pub fn parse_sqm_config(value: &Value) -> Option<WanQos> {
    parse_sqm_section(value).map(|section| section.qos)
}

fn parse_sqm_section(value: &Value) -> Option<SqmSection> {
    let values = value.get("values").and_then(Value::as_object)?;
    let enabled = parse_bool(values.get("enabled"));
    let download_kbps = parse_bandwidth(values.get("download"));
    let upload_kbps = parse_bandwidth(values.get("upload"));

    Some(SqmSection {
        section_type: string_value(values.get(".type")),
        interface: string_value(values.get("interface")),
        qdisc: string_value(values.get("qdisc")),
        script: string_value(values.get("script")),
        qos: WanQos {
            enabled,
            download_kbps,
            upload_kbps,
        },
    })
}

pub fn read_sqm_config(session: Option<&str>) -> Result<Option<WanQos>, LegacyAppError> {
    match rpc::uci_get_config(SQM_CONFIG, Some(SQM_SECTION), None, session) {
        Ok(value) => Ok(parse_sqm_config(&value)),
        Err(error) if error.code == ErrorCode::UciReadFailed => Ok(None),
        Err(error) => Err(error),
    }
}

pub fn stage_sqm_config(
    session: &str,
    interface: Option<&str>,
    qos: Option<&WanQos>,
) -> Result<(), LegacyAppError> {
    delete_sqm_section(session)?;
    let Some(qos) = qos else {
        return Ok(());
    };
    let interface = required_interface(interface)?;

    rpc::call_ubus("uci", "add", upsert_request(session, interface, qos)).map_err(|error| {
        LegacyAppError::new(
            ErrorCode::UciStageFailed,
            ErrorStage::Stage,
            format!("failed to stage SQM configuration: {}", error.message),
        )
        .retryable(error.retryable)
    })?;

    let staged = rpc::uci_get_config(SQM_CONFIG, Some(SQM_SECTION), None, Some(session))?;
    let expected = SqmSection {
        section_type: Some(SQM_QUEUE_TYPE.into()),
        interface: Some(interface.into()),
        qdisc: Some(SQM_QDISC.into()),
        script: Some(SQM_SCRIPT.into()),
        qos: qos.clone(),
    };
    if parse_sqm_section(&staged).as_ref() == Some(&expected) {
        Ok(())
    } else {
        Err(LegacyAppError::new(
            ErrorCode::UciStageMismatch,
            ErrorStage::Stage,
            "staged SQM configuration does not match requested QoS settings",
        ))
    }
}

fn delete_sqm_section(session: &str) -> Result<(), LegacyAppError> {
    match rpc::call_ubus("uci", "delete", delete_request(session)) {
        Ok(_) => Ok(()),
        Err(error) if error.code == ErrorCode::NotFound => Ok(()),
        Err(error) => Err(LegacyAppError::new(
            ErrorCode::UciStageFailed,
            ErrorStage::Stage,
            format!("failed to replace SQM configuration: {}", error.message),
        )
        .retryable(error.retryable)),
    }
}

fn upsert_request(session: &str, interface: &str, qos: &WanQos) -> Value {
    let mut values = Map::new();
    values.insert("enabled".into(), bool_string(qos.enabled));
    values.insert("interface".into(), Value::String(interface.into()));
    values.insert("qdisc".into(), Value::String(SQM_QDISC.into()));
    values.insert("script".into(), Value::String(SQM_SCRIPT.into()));
    values.insert("download".into(), bandwidth_string(qos.download_kbps));
    values.insert("upload".into(), bandwidth_string(qos.upload_kbps));

    json!({
        "config": SQM_CONFIG,
        "type": SQM_QUEUE_TYPE,
        "name": SQM_SECTION,
        "values": values,
        "ubus_rpc_session": session,
    })
}

fn delete_request(session: &str) -> Value {
    json!({
        "config": SQM_CONFIG,
        "section": SQM_SECTION,
        "ubus_rpc_session": session,
    })
}

fn required_interface(interface: Option<&str>) -> Result<&str, LegacyAppError> {
    interface.ok_or_else(|| {
        LegacyAppError::new(
            ErrorCode::InvalidArgument,
            ErrorStage::Stage,
            "cannot configure QoS without a WAN interface",
        )
    })
}

fn parse_bool(value: Option<&Value>) -> bool {
    matches!(value.and_then(Value::as_str), Some("1" | "true"))
        || value.and_then(Value::as_u64) == Some(1)
        || value.and_then(Value::as_bool) == Some(true)
}

fn parse_bandwidth(value: Option<&Value>) -> Option<u32> {
    let bandwidth = value
        .and_then(|value| {
            value
                .as_u64()
                .or_else(|| value.as_str()?.parse::<u64>().ok())
        })
        .and_then(|value| u32::try_from(value).ok())?;
    (bandwidth > 0).then_some(bandwidth)
}

fn string_value(value: Option<&Value>) -> Option<String> {
    value.and_then(Value::as_str).map(str::to_owned)
}

fn bool_string(value: bool) -> Value {
    Value::String(if value { "1" } else { "0" }.into())
}

fn bandwidth_string(value: Option<u32>) -> Value {
    Value::String(value.unwrap_or(0).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_uci_values_and_treats_zero_as_unlimited() {
        let qos = parse_sqm_config(&json!({
            "values": {
                "enabled": "1",
                ".type": "queue",
                "interface": "wan-device",
                "qdisc": "cake",
                "script": "piece_of_cake.qos",
                "download": "100000",
                "upload": "0"
            }
        }))
        .expect("SQM section");

        assert!(qos.enabled);
        assert_eq!(qos.download_kbps, Some(100_000));
        assert_eq!(qos.upload_kbps, None);
    }

    #[test]
    fn upsert_request_replaces_optional_limits_with_zero() {
        let request = upsert_request(
            "session",
            "wan-device",
            &WanQos {
                enabled: true,
                download_kbps: None,
                upload_kbps: Some(20_000),
            },
        );

        assert_eq!(request["type"], "queue");
        assert_eq!(request["name"], "wan");
        assert_eq!(request["values"]["interface"], "wan-device");
        assert_eq!(request["values"]["download"], "0");
        assert_eq!(request["values"]["upload"], "20000");
    }
}
