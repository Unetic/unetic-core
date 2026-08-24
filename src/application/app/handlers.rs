use std::{sync::Arc, thread};

use serde_json::json;

use super::{App, Inner, StateTopic};
use crate::application::state::{now_ms, validate_wifi_config};
#[repr(u32)]
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub enum WifiSetError {
    Success = 0,
    InvalidWifiConfig = 1,
    ApplyFailed = 2,
    NotReady = 3,
}

use crate::{
    application::transaction::ChangeContext,
    domain::errors::{ErrorCode, ErrorStage, LegacyAppError},
    domain::{
        LastOperation, Lifecycle, OperationAccepted, OperationIntent, OperationSource,
        OperationStatus, SetWifiConfigRequest, WifiNetworkConfig,
    },
};

impl App {
    pub fn set_wifi_config(
        self: &Arc<Self>,
        request: SetWifiConfigRequest,
    ) -> Result<OperationAccepted, WifiSetError> {
        validate_wifi_config(&request.ssid, &request.encryption, request.key.as_deref())?;
        validate_request_id(&request.request_id)?;

        let (context, noop_result) = {
            let mut inner = self.inner.lock().expect("app state poisoned");
            check_app_ready(&inner)?;

            if let Some(backhaul) = &request.backhaul {
                crate::application::state::validate_mesh_backhaul_config(
                    backhaul,
                    &inner.config.wifi.primary.targets,
                    &request.radio_channels,
                )?;
            }

            let roaming = request.roaming.unwrap_or(inner.config.wifi.roaming);
            if let Some(accepted) = check_idempotency(&inner, &request, roaming)? {
                return Ok(accepted);
            }

            check_revision(inner.config.revision, request.expected_revision)?;

            if is_wifi_noop(&inner.config.wifi, &request, roaming) {
                let accepted = OperationAccepted {
                    operation_id: self.next_operation_id(),
                    status: OperationStatus::Succeeded,
                    noop: true,
                };
                inner.last_user_operation = Some(noop_last_operation(
                    &accepted,
                    inner.config.revision,
                    &request,
                    roaming,
                ));
                (None, Some(accepted))
            } else {
                let context =
                    build_wifi_change_context(&inner, self.next_operation_id(), &request, roaming);
                inner.active_operation = Some(context.public(OperationStatus::Accepted, None));
                (Some(context), None)
            }
        };

        if let Some(accepted) = noop_result {
            self.publish(StateTopic::Operation);
            return Ok(accepted);
        }

        let context = context.expect("context must be present if not noop");
        self.persist_and_spawn_wifi_change(context)
            .map_err(|_| WifiSetError::ApplyFailed)
    }

    fn persist_and_spawn_wifi_change(
        self: &Arc<Self>,
        context: ChangeContext,
    ) -> Result<OperationAccepted, WifiSetError> {
        let journal = context.to_journal(OperationStatus::Accepted);
        if let Err(error) = self.store.persist_transaction(&journal) {
            let mut inner = self.inner.lock().expect("app state poisoned");
            inner.active_operation = None;
            return Err(error
                .with_operation(&context.operation_id, context.request_id.as_deref())
                .into());
        }
            self.publish(StateTopic::Operation);

        let operation_id = context.operation_id.clone();
        let app = Arc::clone(self);
        let worker_context = context.clone();
        let thread_name = format!(
            "unetic-wifi-{}",
            &operation_id[..operation_id.len().min(24)]
        );

        if let Err(_spawn_error) = thread::Builder::new()
            .name(thread_name)
            .spawn(move || crate::application::transaction::run_change(app, worker_context))
        {
            let error = WifiSetError::ApplyFailed;
            self.complete_failure(
                &context,
                crate::domain::errors::LegacyAppError::new(
                    crate::domain::errors::ErrorCode::Internal,
                    crate::domain::errors::ErrorStage::Apply,
                    "ApplyFailed",
                ),
                false,
            );
            return Err(error);
        }

        Ok(OperationAccepted {
            operation_id,
            status: OperationStatus::Accepted,
            noop: false,
        })
    }

    pub fn wifi_set_config(
        self: &Arc<Self>,
        request: SetWifiConfigRequest,
    ) -> Result<OperationAccepted, WifiSetError> {
        self.set_wifi_config(request)
    }

    pub fn set_ssid(
        self: &Arc<Self>,
        request: SetWifiConfigRequest,
    ) -> Result<OperationAccepted, WifiSetError> {
        self.set_wifi_config(request)
    }
}

fn validate_request_id(request_id: &str) -> Result<(), LegacyAppError> {
    if request_id.trim().is_empty() || request_id.len() > 128 {
        return Err(LegacyAppError::new(
            ErrorCode::InvalidArgument,
            ErrorStage::Validate,
            "request_id must be between 1 and 128 bytes",
        ));
    }
    Ok(())
}

fn check_app_ready(inner: &Inner) -> Result<(), LegacyAppError> {
    if inner.maintenance {
        return Err(LegacyAppError::new(
            ErrorCode::MaintenanceMode,
            ErrorStage::Validate,
            "Unetic is in maintenance mode",
        ));
    }
    if inner.lifecycle != Lifecycle::Ready {
        return Err(LegacyAppError::new(
            ErrorCode::NotReady,
            ErrorStage::Validate,
            format!("core is not ready: {:?}", inner.lifecycle),
        ));
    }
    Ok(())
}

fn check_idempotency(
    inner: &Inner,
    request: &SetWifiConfigRequest,
    roaming: crate::domain::RoamingConfig,
) -> Result<Option<OperationAccepted>, LegacyAppError> {
    let intent = wifi_intent(request, roaming);
    if let Some(active) = &inner.active_operation {
        if active.request_id.as_deref() == Some(request.request_id.as_str()) {
            if active.intent.as_ref() != Some(&intent) {
                return Err(idempotency_conflict(&active.requested_ssid, &request.ssid));
            }
            return Ok(Some(OperationAccepted {
                operation_id: active.id.clone(),
                status: active.status,
                noop: false,
            }));
        }
        return Err(LegacyAppError::new(
            ErrorCode::Busy,
            ErrorStage::Validate,
            "another configuration operation is active",
        ));
    }

    if let Some(last) = &inner.last_user_operation
        && last.request_id.as_deref() == Some(request.request_id.as_str())
    {
        if last.intent.as_ref() != Some(&intent) {
            return Err(idempotency_conflict(&last.requested_ssid, &request.ssid));
        }
        return Ok(Some(OperationAccepted {
            operation_id: last.id.clone(),
            status: last.status,
            noop: false,
        }));
    }

    Ok(None)
}

fn wifi_intent(
    request: &SetWifiConfigRequest,
    roaming: crate::domain::RoamingConfig,
) -> OperationIntent {
    let key = if request.encryption == "none" {
        None
    } else {
        request.key.clone()
    };
    OperationIntent::Wifi {
        ssid: request.ssid.clone(),
        encryption: request.encryption.clone(),
        key,
        roaming,
    }
}

fn noop_last_operation(
    accepted: &OperationAccepted,
    revision: u64,
    request: &SetWifiConfigRequest,
    roaming: crate::domain::RoamingConfig,
) -> LastOperation {
    LastOperation {
        id: accepted.operation_id.clone(),
        request_id: Some(request.request_id.clone()),
        source: OperationSource::User,
        kind: "wifi.set_config".to_owned(),
        status: OperationStatus::Succeeded,
        revision,
        requested_ssid: request.ssid.clone(),
        intent: Some(wifi_intent(request, roaming)),
        error: None,
        finished_at_ms: now_ms(),
    }
}

fn idempotency_conflict(previous_ssid: &str, requested_ssid: &str) -> LegacyAppError {
    LegacyAppError::new(
        ErrorCode::IdempotencyConflict,
        ErrorStage::Validate,
        "request_id was already used for a different SSID",
    )
    .details(json!({
        "previous_ssid": previous_ssid,
        "requested_ssid": requested_ssid
    }))
}

fn check_revision(current_revision: u64, expected_revision: u64) -> Result<(), LegacyAppError> {
    if expected_revision != current_revision {
        return Err(LegacyAppError::new(
            ErrorCode::RevisionConflict,
            ErrorStage::Validate,
            "configuration changed since this client last synchronized",
        )
        .details(json!({
            "expected_revision": expected_revision,
            "current_revision": current_revision
        })));
    }
    Ok(())
}

fn is_wifi_noop(
    current: &crate::domain::wifi::WifiDesired,
    request: &SetWifiConfigRequest,
    roaming: crate::domain::RoamingConfig,
) -> bool {
    let normalized_key = if request.encryption == "none" {
        None
    } else {
        request.key.clone()
    };
    current.primary.ssid == request.ssid
        && current.primary.encryption == request.encryption
        && current.primary.key == normalized_key
        && current.roaming == roaming
        && current.backhaul == request.backhaul
        && current.radio_channels == request.radio_channels
}

fn build_wifi_change_context(
    inner: &Inner,
    operation_id: String,
    request: &SetWifiConfigRequest,
    roaming: crate::domain::RoamingConfig,
) -> ChangeContext {
    let normalized_key = if request.encryption == "none" {
        None
    } else {
        request.key.clone()
    };
    let new_wifi = WifiNetworkConfig {
        ssid: request.ssid.clone(),
        encryption: request.encryption.clone(),
        key: normalized_key,
        targets: inner.config.wifi.primary.targets.clone(),
    };

    ChangeContext {
        operation_id,
        request_id: Some(request.request_id.clone()),
        source: OperationSource::User,
        base_revision: inner.config.revision,
        target_revision: inner.config.revision + 1,
        old_wifi: inner.config.wifi.primary.clone(),
        new_wifi,
        old_roaming: inner.config.wifi.roaming,
        new_roaming: roaming,
        targets: inner.config.wifi.primary.targets.clone(),
        backhaul: request.backhaul.clone(),
        radio_channels: request.radio_channels.clone(),
    }
}

impl From<crate::domain::errors::LegacyAppError> for WifiSetError {
    fn from(err: crate::domain::errors::LegacyAppError) -> Self {
        match err.code {
            crate::domain::errors::ErrorCode::Busy
            | crate::domain::errors::ErrorCode::NotReady
            | crate::domain::errors::ErrorCode::MaintenanceMode => WifiSetError::NotReady,
            crate::domain::errors::ErrorCode::RevisionConflict
            | crate::domain::errors::ErrorCode::IdempotencyConflict => WifiSetError::ApplyFailed,
            _ => WifiSetError::InvalidWifiConfig,
        }
    }
}
