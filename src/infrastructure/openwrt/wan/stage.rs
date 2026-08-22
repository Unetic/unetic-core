use serde_json::json;

use super::build_wan_staging_values;
use crate::{
    domain::{
        WanDesired,
        errors::{ErrorCode, ErrorStage, LegacyAppError},
    },
    infrastructure::openwrt::rpc,
};

const NETWORK_CONFIG: &str = "network";
const WAN_SECTION: &str = "wan";
const INTERFACE_TYPE: &str = "interface";

pub fn replace_wan_section(session: &str, config: &WanDesired) -> Result<(), LegacyAppError> {
    delete_wan_section(session)?;

    rpc::call_ubus(
        "uci",
        "add",
        json!({
            "config": NETWORK_CONFIG,
            "type": INTERFACE_TYPE,
            "name": WAN_SECTION,
            "values": build_wan_staging_values(config),
            "ubus_rpc_session": session,
        }),
    )
    .map(|_| ())
    .map_err(stage_error)
}

fn delete_wan_section(session: &str) -> Result<(), LegacyAppError> {
    match rpc::call_ubus(
        "uci",
        "delete",
        json!({
            "config": NETWORK_CONFIG,
            "section": WAN_SECTION,
            "ubus_rpc_session": session,
        }),
    ) {
        Ok(_) => Ok(()),
        Err(error) if error.code == ErrorCode::NotFound => Ok(()),
        Err(error) => Err(stage_error(error)),
    }
}

fn stage_error(error: LegacyAppError) -> LegacyAppError {
    LegacyAppError::new(
        ErrorCode::UciStageFailed,
        ErrorStage::Stage,
        format!("failed to replace WAN configuration: {}", error.message),
    )
    .retryable(error.retryable)
}
