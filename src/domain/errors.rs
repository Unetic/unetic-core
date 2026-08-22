use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    NotFound,
    InvalidArgument,
    IdempotencyConflict,
    RevisionConflict,
    MaintenanceMode,
    Busy,
    NotReady,
    OperationInterrupted,
    AmbiguousWifiConfig,
    TargetMissing,
    UbusUnavailable,
    RpcdSessionLost,
    UciReadFailed,
    UciStageFailed,
    UciStageMismatch,
    UciApplyFailed,
    VerifyTimeout,
    VerifyMismatch,
    StateStoreFailed,
    ConfirmFailed,
    CommitUncertain,
    RollbackFailed,
    ReconcileFailed,
    StateCorrupt,
    Internal,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ErrorStage {
    Bootstrap,
    Validate,
    Journal,
    Stage,
    Apply,
    Verify,
    Persist,
    Confirm,
    Rollback,
    Reconcile,
    Transport,
    Internal,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LegacyAppError {
    pub code: ErrorCode,
    pub message: String,
    pub stage: ErrorStage,
    pub operation_id: Option<String>,
    pub request_id: Option<String>,
    pub retryable: bool,
    pub details: Value,
}

impl LegacyAppError {
    #[must_use]
    pub fn new(code: ErrorCode, stage: ErrorStage, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            stage,
            operation_id: None,
            request_id: None,
            retryable: false,
            details: json!({}),
        }
    }

    #[must_use]
    pub fn retryable(mut self, retryable: bool) -> Self {
        self.retryable = retryable;
        self
    }

    #[must_use]
    pub fn details(mut self, details: Value) -> Self {
        self.details = details;
        self
    }

    #[must_use]
    pub fn with_operation(mut self, operation_id: &str, request_id: Option<&str>) -> Self {
        self.operation_id = Some(operation_id.to_owned());
        self.request_id = request_id.map(str::to_owned);
        self
    }

    #[must_use]
    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Internal, ErrorStage::Internal, message)
    }
}

impl fmt::Display for LegacyAppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}: {}", self.code, self.message)
    }
}

impl std::error::Error for LegacyAppError {}
