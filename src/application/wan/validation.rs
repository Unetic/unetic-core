use std::net::Ipv4Addr;

use crate::{
    domain::errors::{ErrorCode, ErrorStage, LegacyAppError},
    domain::{SetWanRequest, WanDesired, WanProtocol},
};

pub fn validate_wan_request(request: &SetWanRequest) -> Result<(), LegacyAppError> {
    if request.request_id.trim().is_empty() || request.request_id.len() > 128 {
        return Err(LegacyAppError::new(
            ErrorCode::InvalidArgument,
            ErrorStage::Validate,
            "request_id must be between 1 and 128 characters",
        ));
    }
    validate_wan_desired(&request.wan)
}

pub fn validate_wan_desired(desired: &WanDesired) -> Result<(), LegacyAppError> {
    if !desired.present {
        return Ok(());
    }

    match desired.proto {
        WanProtocol::None => {
            return Err(LegacyAppError::new(
                ErrorCode::InvalidArgument,
                ErrorStage::Validate,
                "WAN protocol must be specified when WAN is present",
            ));
        }
        WanProtocol::Dhcp | WanProtocol::Extender => {}
        WanProtocol::Static => {
            let Some(static_cfg) = &desired.static_config else {
                return Err(LegacyAppError::new(
                    ErrorCode::InvalidArgument,
                    ErrorStage::Validate,
                    "static_config is required when WAN protocol is static",
                ));
            };
            validate_static_config(static_cfg)?;
        }
        WanProtocol::Pppoe => {
            let Some(pppoe_cfg) = &desired.pppoe_config else {
                return Err(LegacyAppError::new(
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

fn validate_static_config(config: &crate::domain::WanStaticConfig) -> Result<(), LegacyAppError> {
    let ip = validate_ipv4(&config.ip_address, "IP address")?;
    let mask = validate_netmask(&config.netmask)?;
    let gw = validate_ipv4(&config.gateway, "gateway")?;

    if (u32::from(ip) & u32::from(mask)) != (u32::from(gw) & u32::from(mask)) {
        return Err(LegacyAppError::new(
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

fn validate_pppoe_config(config: &crate::domain::WanPppoeConfig) -> Result<(), LegacyAppError> {
    if config.username.trim().is_empty() || config.username.len() > 128 {
        return Err(LegacyAppError::new(
            ErrorCode::InvalidArgument,
            ErrorStage::Validate,
            "PPPoE username must be between 1 and 128 characters",
        ));
    }

    if let Some(pass) = &config.password
        && pass.len() > 128
    {
        return Err(LegacyAppError::new(
            ErrorCode::InvalidArgument,
            ErrorStage::Validate,
            "PPPoE password must be at most 128 characters",
        ));
    }

    Ok(())
}

fn validate_ipv4(value: &str, field_name: &str) -> Result<Ipv4Addr, LegacyAppError> {
    let ip: Ipv4Addr = value.parse().map_err(|_| {
        LegacyAppError::new(
            ErrorCode::InvalidArgument,
            ErrorStage::Validate,
            format!("invalid IPv4 format for {field_name}: '{value}'"),
        )
    })?;

    if ip.is_unspecified() || ip.is_loopback() || ip.is_multicast() {
        return Err(LegacyAppError::new(
            ErrorCode::InvalidArgument,
            ErrorStage::Validate,
            format!("{field_name} cannot be unspecified, loopback, or multicast"),
        ));
    }

    Ok(ip)
}

fn validate_netmask(value: &str) -> Result<Ipv4Addr, LegacyAppError> {
    let mask: Ipv4Addr = value.parse().map_err(|_| {
        LegacyAppError::new(
            ErrorCode::InvalidArgument,
            ErrorStage::Validate,
            format!("invalid netmask format: '{value}'"),
        )
    })?;

    let raw = u32::from(mask);
    let inverted = !raw;
    if (inverted.wrapping_add(1) & inverted) != 0 || raw == 0 {
        return Err(LegacyAppError::new(
            ErrorCode::InvalidArgument,
            ErrorStage::Validate,
            format!("netmask '{value}' is not contiguous"),
        ));
    }

    Ok(mask)
}

fn validate_mac_address(mac: &str) -> Result<(), LegacyAppError> {
    let parts: Vec<&str> = mac.split(':').collect();
    if parts.len() != 6 {
        return Err(LegacyAppError::new(
            ErrorCode::InvalidArgument,
            ErrorStage::Validate,
            "MAC address must have 6 octets separated by colons",
        ));
    }

    for part in parts {
        if part.len() != 2 || !part.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(LegacyAppError::new(
                ErrorCode::InvalidArgument,
                ErrorStage::Validate,
                format!("invalid MAC octet: '{part}'"),
            ));
        }
    }

    if mac == "00:00:00:00:00:00" || mac.eq_ignore_ascii_case("ff:ff:ff:ff:ff:ff") {
        return Err(LegacyAppError::new(
            ErrorCode::InvalidArgument,
            ErrorStage::Validate,
            "MAC address cannot be all zeros or broadcast",
        ));
    }

    Ok(())
}

fn validate_mtu(mtu: u16, proto: WanProtocol) -> Result<(), LegacyAppError> {
    let max = if proto == WanProtocol::Pppoe {
        1492
    } else {
        1500
    };
    if !(576..=max).contains(&mtu) {
        return Err(LegacyAppError::new(
            ErrorCode::InvalidArgument,
            ErrorStage::Validate,
            format!("MTU must be between 576 and {max} for {proto:?}"),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::WanDesired;

    #[test]
    fn test_validate_extender_protocol() {
        let desired = WanDesired {
            present: true,
            proto: WanProtocol::Extender,
            ..Default::default()
        };
        assert!(validate_wan_desired(&desired).is_ok());
    }

    #[test]
    fn test_validate_extender_protocol_mtu() {
        let desired_ok = WanDesired {
            present: true,
            proto: WanProtocol::Extender,
            custom_mtu: Some(1500),
            ..Default::default()
        };
        assert!(validate_wan_desired(&desired_ok).is_ok());

        let desired_err = WanDesired {
            present: true,
            proto: WanProtocol::Extender,
            custom_mtu: Some(1501),
            ..Default::default()
        };
        assert!(validate_wan_desired(&desired_err).is_err());
    }
}
