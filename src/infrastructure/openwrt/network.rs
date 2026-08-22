use super::rpc::call_ubus;
use crate::domain::errors::{ErrorCode, ErrorStage, LegacyAppError};
use serde_json::json;

pub fn default_gateway() -> Option<String> {
    let output = std::process::Command::new("ip")
        .args(["route", "show", "default"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8(output.stdout).ok()?;
    let fields: Vec<&str> = stdout.lines().next()?.split_whitespace().collect();
    let gateway_index = fields.iter().position(|field| *field == "via")? + 1;
    fields
        .get(gateway_index)
        .map(|gateway| (*gateway).to_owned())
}

pub fn is_wireless_uplink() -> bool {
    let Ok(output) = std::process::Command::new("ip")
        .args(["route", "show", "default"])
        .output()
    else {
        return false;
    };
    if !output.status.success() {
        return false;
    }
    let Ok(stdout) = String::from_utf8(output.stdout) else {
        return false;
    };
    let Some(first_line) = stdout.lines().next() else {
        return false;
    };
    let fields: Vec<&str> = first_line.split_whitespace().collect();
    let Some(dev_index) = fields
        .iter()
        .position(|field| *field == "dev")
        .map(|idx| idx + 1)
    else {
        return false;
    };
    let Some(dev) = fields.get(dev_index) else {
        return false;
    };
    dev.starts_with("wlan")
        || dev.starts_with("mesh")
        || dev.starts_with("phy")
        || std::path::Path::new(&format!("/sys/class/net/{}/wireless", dev)).exists()
}

pub fn stage_stp(session: &str) -> Result<(), LegacyAppError> {
    call_ubus(
        "uci",
        "set",
        json!({
            "config": "network",
            "section": "lan",
            "values": {
                "stp": "1"
            },
            "ubus_rpc_session": session
        }),
    )
    .map_err(|error| {
        LegacyAppError::new(ErrorCode::UciStageFailed, ErrorStage::Stage, error.message)
    })?;

    Ok(())
}
