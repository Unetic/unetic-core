use crate::application::app::App;
use crate::domain::errors::{ErrorCode, LegacyAppError};
use crate::domain::{DnsConfig, DnsRecord};
use serde_json::{Value, json};
use std::sync::Arc;

#[repr(u32)]
pub enum DnsError {
    InvalidUpstreamIp = 1,
    InvalidHostname = 2,
    InvalidDhcpRange = 3,
    DuplicateRecord = 4,
    RecordNotFound = 5,
    UciApplyFailed = 6,
}

fn map_error(e: LegacyAppError) -> u32 {
    if e.code == ErrorCode::UciApplyFailed {
        return DnsError::UciApplyFailed as u32;
    }
    if e.code == ErrorCode::NotFound {
        return DnsError::RecordNotFound as u32;
    }
    if e.message.contains("upstream IP") {
        return DnsError::InvalidUpstreamIp as u32;
    }
    if e.message.contains("DHCP range") {
        return DnsError::InvalidDhcpRange as u32;
    }
    if e.message.contains("hostname") {
        return DnsError::InvalidHostname as u32;
    }
    if e.message.contains("Duplicate") {
        return DnsError::DuplicateRecord as u32;
    }
    DnsError::InvalidUpstreamIp as u32
}

pub fn dispatch(app: &Arc<App>, method: &str, request: Value) -> Result<Value, u32> {
    match method {
        "dns.get" => {
            let state = app.state();
            Ok(json!(state.dns))
        }
        "dns.set" => {
            let cfg: DnsConfig = serde_json::from_value(request).map_err(|_| 1u32)?;
            app.dns_set(cfg).map(|_| json!({})).map_err(map_error)
        }
        "dns.record.add" => {
            let record: DnsRecord = serde_json::from_value(request).map_err(|_| 1u32)?;
            app.dns_add_record(record)
                .map(|_| json!({}))
                .map_err(map_error)
        }
        "dns.record.remove" => {
            #[derive(serde::Deserialize)]
            struct RmReq {
                id: String,
            }
            let req: RmReq = serde_json::from_value(request).map_err(|_| 1u32)?;
            app.dns_remove_record(&req.id)
                .map(|_| json!({}))
                .map_err(map_error)
        }
        _ => Err(1u32),
    }
}
