use std::{net::Ipv4Addr, sync::Arc, thread, time::Instant};

use tracing::{error, info, warn};

use crate::{
    app::App,
    errors::{DomainError, ErrorCode, ErrorStage},
    model::{
        OperationSource, OperationStatus, PublicOperation, STATE_SCHEMA_VERSION, SetWanRequest,
        TransactionJournal, WanDesired, WanProtocol, WanStatus,
    },
};

#[derive(Debug, Clone)]
pub struct WanChangeContext {
    pub operation_id: String,
    pub request_id: Option<String>,
    pub source: OperationSource,
    pub base_revision: u64,
    pub target_revision: u64,
    pub old_wan: WanDesired,
    pub new_wan: WanDesired,
}

impl WanChangeContext {
    #[must_use]
    pub fn public(&self, status: OperationStatus, error: Option<DomainError>) -> PublicOperation {
        PublicOperation {
            id: self.operation_id.clone(),
            request_id: self.request_id.clone(),
            source: self.source,
            kind: "wan.set_config".into(),
            status,
            requested_ssid: String::new(),
            error,
        }
    }

    #[must_use]
    pub fn to_journal(&self, phase: OperationStatus) -> TransactionJournal {
        TransactionJournal {
            schema_version: STATE_SCHEMA_VERSION,
            operation_id: self.operation_id.clone(),
            request_id: self.request_id.clone().unwrap_or_default(),
            source: self.source,
            base_revision: self.base_revision,
            target_revision: self.target_revision,
            old_ssid: String::new(),
            new_ssid: String::new(),
            targets: Vec::new(),
            phase,
        }
    }
}

pub fn run_wan_change(app: Arc<App>, context: WanChangeContext) {
    let span = tracing::info_span!(
        "wan_operation",
        operation_id = %context.operation_id,
        request_id = ?context.request_id,
        source = ?context.source,
    );
    let _entered = span.enter();

    if let Err(error) = execute_wan(&app, &context) {
        error!(%error, "wan configuration operation failed unexpectedly");
        app.complete_wan_failure(&context, error, false);
    }
}

fn execute_wan(app: &Arc<App>, context: &WanChangeContext) -> Result<(), DomainError> {
    let session = app.ensure_session().map_err(|error| {
        error.with_operation(&context.operation_id, context.request_id.as_deref())
    })?;

    app.set_operation_status_with_kind(
        &context.operation_id,
        "wan.set_config",
        OperationStatus::Staging,
        None,
    )?;
    if let Err(error) = app.backend.stage_wan_config(&session, &context.new_wan) {
        let _ = app.backend.revert_staged(&session);
        app.complete_wan_failure(context, attach(error, context), false);
        return Ok(());
    }

    app.set_operation_status_with_kind(
        &context.operation_id,
        "wan.set_config",
        OperationStatus::Applying,
        None,
    )?;
    if let Err(error) = app
        .backend
        .apply(&session, app.timing.rpcd_rollback_timeout_secs)
    {
        let _ = app.backend.rollback(&session);
        let _ = app.backend.revert_staged(&session);
        app.complete_wan_failure(context, attach(error, context), false);
        return Ok(());
    }

    app.set_operation_status_with_kind(
        &context.operation_id,
        "wan.set_config",
        OperationStatus::Verifying,
        None,
    )?;
    if context.new_wan.present {
        let deadline = Instant::now() + app.timing.verify_timeout;
        let mut ok = false;
        while Instant::now() < deadline {
            if let Ok(st) = app.backend.read_wan_runtime_status()
                && (st.status == WanStatus::Connected || st.status == WanStatus::Connecting)
            {
                ok = true;
                break;
            }
            thread::sleep(app.timing.verify_sample_delay);
        }
        if !ok {
            warn!("WAN verification timed out; rolling back");
            let _ = app.backend.rollback(&session);
            app.complete_wan_failure(
                context,
                DomainError::new(
                    ErrorCode::VerifyTimeout,
                    ErrorStage::Verify,
                    "WAN interface did not become ready",
                ),
                false,
            );
            return Ok(());
        }
    }

    if context.source == OperationSource::User {
        app.set_operation_status_with_kind(
            &context.operation_id,
            "wan.set_config",
            OperationStatus::Persisting,
            None,
        )?;
        if let Err(error) = app.persist_new_desired_wan(context) {
            let _ = app.backend.rollback(&session);
            app.complete_wan_failure(context, attach(error, context), false);
            return Ok(());
        }
    }

    app.set_operation_status_with_kind(
        &context.operation_id,
        "wan.set_config",
        OperationStatus::Confirming,
        None,
    )?;
    if let Err(error) = app.backend.confirm(&session) {
        if context.source == OperationSource::User {
            app.mark_wan_commit_uncertain(context, error);
            return Ok(());
        }
        app.complete_wan_failure(context, attach(error, context), false);
        return Ok(());
    }

    info!("WAN configuration confirmed successfully");
    app.complete_wan_success(context)?;
    Ok(())
}

fn attach(error: DomainError, context: &WanChangeContext) -> DomainError {
    error.with_operation(&context.operation_id, context.request_id.as_deref())
}

pub fn validate_wan_request(request: &SetWanRequest) -> Result<(), DomainError> {
    if request.request_id.trim().is_empty() || request.request_id.len() > 128 {
        return Err(DomainError::new(
            ErrorCode::InvalidArgument,
            ErrorStage::Validate,
            "request_id must be between 1 and 128 characters",
        ));
    }
    validate_wan_desired(&request.wan)
}

pub fn validate_wan_desired(desired: &WanDesired) -> Result<(), DomainError> {
    if !desired.present {
        return Ok(());
    }

    match desired.proto {
        WanProtocol::None => {
            return Err(DomainError::new(
                ErrorCode::InvalidArgument,
                ErrorStage::Validate,
                "WAN protocol must be specified when WAN is present",
            ));
        }
        WanProtocol::Dhcp => {}
        WanProtocol::Static => {
            let Some(static_cfg) = &desired.static_config else {
                return Err(DomainError::new(
                    ErrorCode::InvalidArgument,
                    ErrorStage::Validate,
                    "static_config is required when WAN protocol is static",
                ));
            };
            validate_static_config(static_cfg)?;
        }
        WanProtocol::Pppoe => {
            let Some(pppoe_cfg) = &desired.pppoe_config else {
                return Err(DomainError::new(
                    ErrorCode::InvalidArgument,
                    ErrorStage::Validate,
                    "pppoe_config is required when WAN protocol is pppoe",
                ));
            };
            validate_pppoe_config(pppoe_cfg)?;
        }
    }

    if let Some(mac) = &desired.custom_mac {
        validate_mac_address(mac)?;
    }

    if let Some(mtu) = desired.custom_mtu {
        validate_mtu(mtu, desired.proto)?;
    }

    for dns in &desired.custom_dns {
        validate_ipv4(dns, "custom DNS")?;
    }

    Ok(())
}

fn validate_static_config(config: &crate::model::WanStaticConfig) -> Result<(), DomainError> {
    let ip = validate_ipv4(&config.ip_address, "IP address")?;
    let mask = validate_netmask(&config.netmask)?;
    let gw = validate_ipv4(&config.gateway, "gateway")?;

    if (u32::from(ip) & u32::from(mask)) != (u32::from(gw) & u32::from(mask)) {
        return Err(DomainError::new(
            ErrorCode::InvalidArgument,
            ErrorStage::Validate,
            "gateway IP is outside the specified subnet",
        ));
    }

    for dns in &config.dns {
        validate_ipv4(dns, "static DNS")?;
    }

    Ok(())
}

fn validate_pppoe_config(config: &crate::model::WanPppoeConfig) -> Result<(), DomainError> {
    if config.username.trim().is_empty() || config.username.len() > 128 {
        return Err(DomainError::new(
            ErrorCode::InvalidArgument,
            ErrorStage::Validate,
            "PPPoE username must be between 1 and 128 characters",
        ));
    }

    if let Some(pass) = &config.password
        && pass.len() > 128
    {
        return Err(DomainError::new(
            ErrorCode::InvalidArgument,
            ErrorStage::Validate,
            "PPPoE password must be at most 128 characters",
        ));
    }

    Ok(())
}

fn validate_ipv4(value: &str, field_name: &str) -> Result<Ipv4Addr, DomainError> {
    let ip: Ipv4Addr = value.parse().map_err(|_| {
        DomainError::new(
            ErrorCode::InvalidArgument,
            ErrorStage::Validate,
            format!("invalid IPv4 format for {field_name}: '{value}'"),
        )
    })?;

    if ip.is_unspecified() || ip.is_loopback() || ip.is_multicast() {
        return Err(DomainError::new(
            ErrorCode::InvalidArgument,
            ErrorStage::Validate,
            format!("{field_name} cannot be unspecified, loopback, or multicast"),
        ));
    }

    Ok(ip)
}

fn validate_netmask(value: &str) -> Result<Ipv4Addr, DomainError> {
    let mask: Ipv4Addr = value.parse().map_err(|_| {
        DomainError::new(
            ErrorCode::InvalidArgument,
            ErrorStage::Validate,
            format!("invalid netmask format: '{value}'"),
        )
    })?;

    let raw = u32::from(mask);
    let inverted = !raw;
    if (inverted.wrapping_add(1) & inverted) != 0 || raw == 0 {
        return Err(DomainError::new(
            ErrorCode::InvalidArgument,
            ErrorStage::Validate,
            format!("netmask '{value}' is not contiguous"),
        ));
    }

    Ok(mask)
}

fn validate_mac_address(mac: &str) -> Result<(), DomainError> {
    let parts: Vec<&str> = mac.split(':').collect();
    if parts.len() != 6 {
        return Err(DomainError::new(
            ErrorCode::InvalidArgument,
            ErrorStage::Validate,
            "MAC address must have 6 octets separated by colons",
        ));
    }

    for part in parts {
        if part.len() != 2 || !part.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(DomainError::new(
                ErrorCode::InvalidArgument,
                ErrorStage::Validate,
                format!("invalid MAC octet: '{part}'"),
            ));
        }
    }

    if mac == "00:00:00:00:00:00" || mac.eq_ignore_ascii_case("ff:ff:ff:ff:ff:ff") {
        return Err(DomainError::new(
            ErrorCode::InvalidArgument,
            ErrorStage::Validate,
            "MAC address cannot be all zeros or broadcast",
        ));
    }

    Ok(())
}

fn validate_mtu(mtu: u16, proto: WanProtocol) -> Result<(), DomainError> {
    let max = if proto == WanProtocol::Pppoe {
        1492
    } else {
        1500
    };
    if !(576..=max).contains(&mtu) {
        return Err(DomainError::new(
            ErrorCode::InvalidArgument,
            ErrorStage::Validate,
            format!("MTU must be between 576 and {max} for {proto:?}"),
        ));
    }
    Ok(())
}
