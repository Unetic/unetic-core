use serde_json::json;
use crate::domain::errors::{LegacyAppError, ErrorCode, ErrorStage};
use super::rpc::call_ubus;

pub fn enable_stp() -> Result<(), LegacyAppError> {
    call_ubus(
        "uci",
        "set",
        json!({
            "config": "network",
            "section": "lan",
            "values": {
                "stp": "1"
            }
        })
    ).map_err(|error| LegacyAppError::new(ErrorCode::UciStageFailed, ErrorStage::Stage, error.message))?;

    call_ubus(
        "uci",
        "commit",
        json!({
            "config": "network"
        })
    ).map_err(|error| LegacyAppError::new(ErrorCode::ConfirmFailed, ErrorStage::Confirm, error.message))?;

    Ok(())
}
