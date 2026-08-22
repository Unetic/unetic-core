use crate::application::app::App;
use crate::application::subscription::SubscribeError;
use serde_json::{Value, json};
use std::sync::Arc;

pub fn dispatch(app: &Arc<App>, method: &str, request: Value) -> Result<Value, u32> {
    match method {
        "state.subscribe.create" => parse_ttl(&request)
            .and_then(|ttl_mins| app.subscriptions.create(ttl_mins))
            .map(|id| json!({ "subscription_id": id }))
            .map_err(|error| error as u32),
        "state.subscribe.continue" => {
            let sub_id = request
                .get("subscription_id")
                .and_then(Value::as_str)
                .unwrap_or("");
            parse_ttl(&request)
                .and_then(|ttl_mins| app.subscriptions.continue_sub(sub_id, ttl_mins))
                .map(|_| json!({}))
                .map_err(|e| e as u32)
        }
        "state.subscribe.cancel" => {
            let sub_id = request
                .get("subscription_id")
                .and_then(Value::as_str)
                .unwrap_or("");
            app.subscriptions
                .cancel(sub_id)
                .map(|_| json!({}))
                .map_err(|e| e as u32)
        }
        _ => Err(1),
    }
}

fn parse_ttl(request: &Value) -> Result<u32, SubscribeError> {
    let Some(value) = request.get("ttl_mins") else {
        return Ok(5);
    };
    value
        .as_u64()
        .and_then(|ttl| u32::try_from(ttl).ok())
        .ok_or(SubscribeError::InvalidTtl)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_non_numeric_and_overflowing_ttl() {
        assert!(matches!(
            parse_ttl(&json!({"ttl_mins": "5"})),
            Err(SubscribeError::InvalidTtl)
        ));
        assert!(matches!(
            parse_ttl(&json!({"ttl_mins": u64::from(u32::MAX) + 1})),
            Err(SubscribeError::InvalidTtl)
        ));
    }

    #[test]
    fn defaults_only_when_ttl_is_absent() {
        assert_eq!(parse_ttl(&json!({})).expect("default TTL"), 5);
    }
}
