use std::process::Command;

use serde::{Deserialize, Serialize};

use crate::{
    application::app::App,
    domain::errors::{DomainError, ErrorCode, ErrorStage},
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PingRequest {
    pub host: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PingResult {
    pub output: String,
}

pub fn execute_ping(host: &str) -> Result<PingResult, DomainError> {
    let host = host.trim();
    if host.is_empty() {
        return Err(DomainError::new(
            ErrorCode::InvalidArgument,
            ErrorStage::Validate,
            "host must not be empty",
        ));
    }

    let output = Command::new("ping")
        .args(["-c", "4", host])
        .output()
        .map_err(|error| {
            DomainError::new(
                ErrorCode::Internal,
                ErrorStage::Transport,
                format!("failed to execute ping command: {error}"),
            )
        })?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let output_str = if stdout.is_empty() {
        stderr.into_owned()
    } else if stderr.is_empty() {
        stdout.into_owned()
    } else {
        format!("{stdout}{stderr}")
    };

    Ok(PingResult { output: output_str })
}

impl App {
    pub fn ping(&self, host: &str) -> Result<PingResult, DomainError> {
        execute_ping(host)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, mpsc};

    use super::*;
    use crate::{MemoryBackend, StateStore};

    #[test]
    fn test_execute_ping_loopback() {
        let result = execute_ping("127.0.0.1").expect("ping 127.0.0.1 should succeed");
        assert!(
            result.output.contains("bytes from 127.0.0.1") || result.output.contains("127.0.0.1"),
            "ping output did not contain expected content: {}",
            result.output
        );
    }

    #[test]
    fn test_execute_ping_empty_host() {
        let err = execute_ping("   ").expect_err("empty host must fail");
        assert_eq!(err.code, ErrorCode::InvalidArgument);
    }

    #[test]
    fn test_api_dispatch_ping_success() {
        let backend = Arc::new(MemoryBackend::new("Home", &["radio0"]));
        let (tx, _rx) = mpsc::channel();
        let store = StateStore::new(std::env::temp_dir().join("unetic-test-ping-api"));
        let app = App::bootstrap(backend, store, tx);

        let response_str =
            crate::presentation::api::dispatch(&app, "tools.ping", r#"{"host":"127.0.0.1"}"#);
        let val: serde_json::Value = serde_json::from_str(&response_str).expect("valid json");
        assert_eq!(val.get("ok").and_then(|v| v.as_bool()), Some(true));
        assert!(val.pointer("/result/output").is_some());
    }

    #[test]
    fn test_api_dispatch_ping_invalid_argument() {
        let backend = Arc::new(MemoryBackend::new("Home", &["radio0"]));
        let (tx, _rx) = mpsc::channel();
        let store = StateStore::new(std::env::temp_dir().join("unetic-test-ping-invalid"));
        let app = App::bootstrap(backend, store, tx);

        let response_str = crate::presentation::api::dispatch(&app, "tools.ping", r#"{"host":""}"#);
        let val: serde_json::Value = serde_json::from_str(&response_str).expect("valid json");
        assert_eq!(val.get("ok").and_then(|v| v.as_bool()), Some(false));
        assert_eq!(
            val.pointer("/error/code").and_then(|v| v.as_str()),
            Some("INVALID_ARGUMENT")
        );
    }
}
