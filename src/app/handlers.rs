use std::{sync::Arc, thread};

use serde_json::json;

use super::{App, state::validate_ssid};
use crate::{
    errors::{DomainError, ErrorCode, ErrorStage},
    model::{
        Lifecycle, OperationAccepted, OperationSource, OperationStatus, SetSsidRequest,
        SetWanRequest,
    },
    transaction::{self, ChangeContext},
    wan::WanChangeContext,
};

impl App {
    pub fn set_ssid(
        self: &Arc<Self>,
        request: SetSsidRequest,
    ) -> Result<OperationAccepted, DomainError> {
        validate_ssid(&request.ssid)?;
        if request.request_id.trim().is_empty() || request.request_id.len() > 128 {
            return Err(DomainError::new(
                ErrorCode::InvalidArgument,
                ErrorStage::Validate,
                "request_id must be between 1 and 128 bytes",
            ));
        }

        let context = {
            let mut inner = self.inner.lock().expect("app state poisoned");

            if inner.maintenance {
                return Err(DomainError::new(
                    ErrorCode::MaintenanceMode,
                    ErrorStage::Validate,
                    "Unetic is in maintenance mode",
                ));
            }
            if inner.lifecycle != Lifecycle::Ready {
                return Err(DomainError::new(
                    ErrorCode::NotReady,
                    ErrorStage::Validate,
                    format!("core is not ready: {:?}", inner.lifecycle),
                ));
            }

            if let Some(active) = &inner.active_operation {
                if active.request_id.as_deref() == Some(&request.request_id) {
                    if active.requested_ssid != request.ssid {
                        return Err(DomainError::new(
                            ErrorCode::IdempotencyConflict,
                            ErrorStage::Validate,
                            "request_id was already used for a different SSID",
                        )
                        .details(json!({
                            "previous_ssid": active.requested_ssid.clone(),
                            "requested_ssid": request.ssid.clone()
                        })));
                    }
                    return Ok(OperationAccepted {
                        operation_id: active.id.clone(),
                        status: active.status,
                        noop: false,
                    });
                }
                return Err(DomainError::new(
                    ErrorCode::Busy,
                    ErrorStage::Validate,
                    "another configuration operation is active",
                ));
            }

            if let Some(last) = &inner.last_user_operation
                && last.request_id.as_deref() == Some(&request.request_id)
            {
                if last.requested_ssid != request.ssid {
                    return Err(DomainError::new(
                        ErrorCode::IdempotencyConflict,
                        ErrorStage::Validate,
                        "request_id was already used for a different SSID",
                    )
                    .details(json!({
                        "previous_ssid": last.requested_ssid.clone(),
                        "requested_ssid": request.ssid.clone()
                    })));
                }
                return Ok(OperationAccepted {
                    operation_id: last.id.clone(),
                    status: last.status,
                    noop: false,
                });
            }

            if request.expected_revision != inner.config.revision {
                return Err(DomainError::new(
                    ErrorCode::RevisionConflict,
                    ErrorStage::Validate,
                    "configuration changed since this client last synchronized",
                )
                .details(json!({
                    "expected_revision": request.expected_revision,
                    "current_revision": inner.config.revision
                })));
            }

            if inner.config.wifi.primary.ssid == request.ssid {
                return Ok(OperationAccepted {
                    operation_id: self.next_operation_id(),
                    status: OperationStatus::Succeeded,
                    noop: true,
                });
            }

            let context = ChangeContext {
                operation_id: self.next_operation_id(),
                request_id: Some(request.request_id.clone()),
                source: OperationSource::User,
                base_revision: inner.config.revision,
                target_revision: inner.config.revision + 1,
                old_ssid: inner.config.wifi.primary.ssid.clone(),
                new_ssid: request.ssid.clone(),
                targets: inner.config.wifi.primary.targets.clone(),
            };
            inner.active_operation = Some(context.public(OperationStatus::Accepted, None));
            context
        };

        let journal = context.to_journal(OperationStatus::Accepted);
        if let Err(error) = self.store.persist_transaction(&journal) {
            let mut inner = self.inner.lock().expect("app state poisoned");
            inner.active_operation = None;
            return Err(error.with_operation(&context.operation_id, context.request_id.as_deref()));
        }
        self.publish();

        let operation_id = context.operation_id.clone();
        let app = Arc::clone(self);
        let worker_context = context.clone();
        if let Err(spawn_error) = thread::Builder::new()
            .name(format!(
                "unetic-wifi-{}",
                &operation_id[..operation_id.len().min(24)]
            ))
            .spawn(move || transaction::run_change(app, worker_context))
        {
            let error = DomainError::new(
                ErrorCode::Internal,
                ErrorStage::Internal,
                format!("failed to start transaction worker: {spawn_error}"),
            )
            .with_operation(&context.operation_id, context.request_id.as_deref());
            self.complete_failure(&context, error.clone(), false);
            return Err(error);
        }

        Ok(OperationAccepted {
            operation_id,
            status: OperationStatus::Accepted,
            noop: false,
        })
    }

    pub fn set_wan(
        self: &Arc<Self>,
        request: SetWanRequest,
    ) -> Result<OperationAccepted, DomainError> {
        if request.request_id.trim().is_empty() || request.request_id.len() > 128 {
            return Err(DomainError::new(
                ErrorCode::InvalidArgument,
                ErrorStage::Validate,
                "request_id must be between 1 and 128 bytes",
            ));
        }

        crate::wan::validate_wan_desired(&request.wan)?;

        let context = {
            let mut inner = self.inner.lock().expect("app state poisoned");

            if inner.maintenance {
                return Err(DomainError::new(
                    ErrorCode::MaintenanceMode,
                    ErrorStage::Validate,
                    "Unetic is in maintenance mode",
                ));
            }
            if inner.lifecycle != Lifecycle::Ready {
                return Err(DomainError::new(
                    ErrorCode::NotReady,
                    ErrorStage::Validate,
                    format!("core is not ready: {:?}", inner.lifecycle),
                ));
            }

            if let Some(active) = &inner.active_operation {
                if active.request_id.as_deref() == Some(&request.request_id) {
                    return Ok(OperationAccepted {
                        operation_id: active.id.clone(),
                        status: active.status,
                        noop: false,
                    });
                }
                return Err(DomainError::new(
                    ErrorCode::Busy,
                    ErrorStage::Validate,
                    "another configuration operation is active",
                ));
            }

            if let Some(last) = &inner.last_user_operation
                && last.request_id.as_deref() == Some(&request.request_id)
            {
                return Ok(OperationAccepted {
                    operation_id: last.id.clone(),
                    status: last.status,
                    noop: false,
                });
            }

            if request.expected_revision != inner.config.revision {
                return Err(DomainError::new(
                    ErrorCode::RevisionConflict,
                    ErrorStage::Validate,
                    "configuration changed since this client last synchronized",
                )
                .details(json!({
                    "expected_revision": request.expected_revision,
                    "current_revision": inner.config.revision
                })));
            }

            if inner.config.wan == request.wan {
                return Ok(OperationAccepted {
                    operation_id: self.next_operation_id(),
                    status: OperationStatus::Succeeded,
                    noop: true,
                });
            }

            let context = WanChangeContext {
                operation_id: self.next_operation_id(),
                request_id: Some(request.request_id.clone()),
                source: OperationSource::User,
                base_revision: inner.config.revision,
                target_revision: inner.config.revision + 1,
                old_wan: inner.config.wan.clone(),
                new_wan: request.wan.clone(),
            };
            inner.active_operation = Some(context.public(OperationStatus::Accepted, None));
            context
        };

        let journal = context.to_journal(OperationStatus::Accepted);
        if let Err(error) = self.store.persist_transaction(&journal) {
            let mut inner = self.inner.lock().expect("app state poisoned");
            inner.active_operation = None;
            return Err(error.with_operation(&context.operation_id, context.request_id.as_deref()));
        }
        self.publish();

        let operation_id = context.operation_id.clone();
        let app = Arc::clone(self);
        let worker_context = context.clone();
        if let Err(spawn_error) = thread::Builder::new()
            .name(format!(
                "unetic-wan-{}",
                &operation_id[..operation_id.len().min(24)]
            ))
            .spawn(move || crate::wan::run_wan_change(app, worker_context))
        {
            let error = DomainError::new(
                ErrorCode::Internal,
                ErrorStage::Internal,
                format!("failed to start transaction worker: {spawn_error}"),
            )
            .with_operation(&context.operation_id, context.request_id.as_deref());
            self.complete_wan_failure(&context, error.clone(), false);
            return Err(error);
        }

        Ok(OperationAccepted {
            operation_id,
            status: OperationStatus::Accepted,
            noop: false,
        })
    }
}
