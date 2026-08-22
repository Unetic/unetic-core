use crate::{
    application::app::App,
    domain::{
        device::{PortForward, RegisteredDevice},
        errors::{ErrorCode, ErrorStage, LegacyAppError},
    },
};

impl App {
    pub fn register_device(&self, device: RegisteredDevice) -> Result<(), LegacyAppError> {
        validate_device(&device)?;
        if self
            .state()
            .registered_devices
            .iter()
            .any(|registered| registered.mac.eq_ignore_ascii_case(&device.mac))
        {
            return Ok(());
        }

        let static_ip = self.static_ip_for(&device, true)?;

        self.update_registered_devices(|devices| {
            devices.retain(|registered| registered.uuid != device.uuid);
            devices.push(device.clone());
            Ok(())
        })?;
        self.publish();
        if let Some(ip) = static_ip {
            self.backend
                .write_static_lease(&device.mac, &ip, Some(&device.name))?;
        }
        self.sync_registered_devices()
    }

    pub fn update_device(
        &self,
        uuid: &str,
        device: RegisteredDevice,
    ) -> Result<(), LegacyAppError> {
        validate_identifier(uuid, "device UUID")?;
        validate_device(&device)?;
        if device.uuid != uuid {
            return Err(invalid_argument("device UUID cannot be changed"));
        }

        let previous = self
            .state()
            .registered_devices
            .into_iter()
            .find(|registered| registered.uuid == uuid)
            .ok_or_else(|| not_found("device"))?;
        if !previous.mac.eq_ignore_ascii_case(&device.mac) {
            return Err(invalid_argument("device MAC address cannot be changed"));
        }
        let static_ip = self.static_ip_for(&device, !previous.is_static_ip)?;

        self.update_registered_devices(|devices| {
            let registered = devices
                .iter_mut()
                .find(|registered| registered.uuid == uuid)
                .ok_or_else(|| not_found("device"))?;
            *registered = device.clone();
            Ok(())
        })?;
        self.publish();
        if previous.is_static_ip && !device.is_static_ip {
            self.backend.delete_static_lease(&device.mac)?;
        } else if let Some(ip) = static_ip {
            self.backend
                .write_static_lease(&device.mac, &ip, Some(&device.name))?;
        }
        self.sync_registered_devices()
    }

    pub fn delete_device(&self, uuid: &str) -> Result<(), LegacyAppError> {
        validate_identifier(uuid, "device UUID")?;

        let removed = self.update_registered_devices(|devices| {
            let index = devices
                .iter()
                .position(|device| device.uuid == uuid)
                .ok_or_else(|| not_found("device"))?;
            Ok(devices.remove(index))
        })?;

        self.publish();
        if removed.is_static_ip {
            self.backend.delete_static_lease(&removed.mac)?;
        }
        self.sync_registered_devices()
    }

    pub fn add_port_forward(
        &self,
        uuid: &str,
        mut rule: PortForward,
    ) -> Result<(), LegacyAppError> {
        validate_identifier(uuid, "device UUID")?;
        validate_port_forward(&rule)?;
        rule.protocol = normalize_protocol(&rule.protocol)?.to_owned();

        self.update_registered_devices(|devices| {
            let device = devices
                .iter_mut()
                .find(|device| device.uuid == uuid)
                .ok_or_else(|| not_found("device"))?;
            device.port_forwards.retain(|current| current.id != rule.id);
            device.port_forwards.push(rule);
            Ok(())
        })?;
        self.publish();
        self.sync_registered_devices()
    }

    pub fn remove_port_forward(&self, uuid: &str, rule_id: &str) -> Result<(), LegacyAppError> {
        validate_identifier(uuid, "device UUID")?;
        validate_identifier(rule_id, "port-forward ID")?;

        self.update_registered_devices(|devices| {
            let device = devices
                .iter_mut()
                .find(|device| device.uuid == uuid)
                .ok_or_else(|| not_found("device"))?;
            let original_len = device.port_forwards.len();
            device.port_forwards.retain(|rule| rule.id != rule_id);
            if device.port_forwards.len() == original_len {
                return Err(not_found("port-forward rule"));
            }
            Ok(())
        })?;
        self.publish();
        self.sync_registered_devices()
    }

    pub fn devices_sync_ip(&self, mac: &str, ip: Option<&str>, ip6: Option<&str>) {
        let registered = {
            let inner = self.inner.lock().expect("app state poisoned");
            inner
                .config
                .registered_devices
                .iter()
                .find(|device| device.mac.eq_ignore_ascii_case(mac))
                .cloned()
        };
        let Some(device) = registered else {
            return;
        };

        if device.is_static_ip
            && let Some(address) = ip
        {
            if let Err(error) =
                self.backend
                    .write_static_lease(&device.mac, address, Some(&device.name))
            {
                tracing::warn!(%error, mac, "failed to synchronize static lease");
            }
        }

        if ip.is_some() || ip6.is_some() {
            if let Err(error) = self.sync_registered_devices() {
                tracing::warn!(%error, mac, "failed to synchronize port forwards");
            }
        }
    }

    fn update_registered_devices<T>(
        &self,
        update: impl FnOnce(&mut Vec<RegisteredDevice>) -> Result<T, LegacyAppError>,
    ) -> Result<T, LegacyAppError> {
        let mut inner = self.inner.lock().expect("app state poisoned");
        let mut config = inner.config.clone();
        let result = update(&mut config.registered_devices)?;
        config.revision = config.revision.saturating_add(1);
        self.store.persist_config(&config)?;
        inner.config = config;
        Ok(result)
    }

    fn static_ip_for(
        &self,
        device: &RegisteredDevice,
        required: bool,
    ) -> Result<Option<String>, LegacyAppError> {
        if !device.is_static_ip {
            return Ok(None);
        }

        let ip = self
            .devices_list()?
            .into_iter()
            .find(|current| current.mac.eq_ignore_ascii_case(&device.mac))
            .and_then(|current| current.ip);
        if required && ip.is_none() {
            return Err(invalid_argument(
                "device needs an IPv4 address for a static lease",
            ));
        }
        Ok(ip)
    }
}

fn validate_device(device: &RegisteredDevice) -> Result<(), LegacyAppError> {
    validate_identifier(&device.uuid, "device UUID")?;
    if !is_valid_mac(&device.mac) {
        return Err(invalid_argument("invalid device MAC address"));
    }
    if device.name.trim().is_empty() || device.name.len() > 64 {
        return Err(invalid_argument(
            "device name must be between 1 and 64 bytes",
        ));
    }
    for rule in &device.port_forwards {
        validate_port_forward(rule)?;
    }
    Ok(())
}

fn validate_port_forward(rule: &PortForward) -> Result<(), LegacyAppError> {
    validate_identifier(&rule.id, "port-forward ID")?;
    if !(1..=65_535).contains(&rule.external_port) || !(1..=65_535).contains(&rule.internal_port) {
        return Err(invalid_argument("port must be between 1 and 65535"));
    }
    normalize_protocol(&rule.protocol)?;
    Ok(())
}

fn normalize_protocol(protocol: &str) -> Result<&'static str, LegacyAppError> {
    match protocol.to_ascii_lowercase().replace(',', " ").as_str() {
        "tcp" => Ok("tcp"),
        "udp" => Ok("udp"),
        "tcp udp" | "udp tcp" | "both" => Ok("tcp udp"),
        _ => Err(invalid_argument("protocol must be tcp, udp, or both")),
    }
}

fn validate_identifier(value: &str, label: &str) -> Result<(), LegacyAppError> {
    let valid = !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'));
    if valid {
        Ok(())
    } else {
        Err(invalid_argument(format!(
            "{label} must contain only letters, digits, '-' or '_'"
        )))
    }
}

fn is_valid_mac(mac: &str) -> bool {
    let mut octets = mac.split(':');
    (0..6).all(|_| {
        octets.next().is_some_and(|octet| {
            octet.len() == 2 && octet.bytes().all(|byte| byte.is_ascii_hexdigit())
        })
    }) && octets.next().is_none()
}

fn invalid_argument(message: impl Into<String>) -> LegacyAppError {
    LegacyAppError::new(ErrorCode::InvalidArgument, ErrorStage::Validate, message)
}

fn not_found(entity: &str) -> LegacyAppError {
    LegacyAppError::new(
        ErrorCode::NotFound,
        ErrorStage::Validate,
        format!("{entity} not found"),
    )
}

#[cfg(test)]
mod tests;
