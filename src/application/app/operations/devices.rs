use crate::{
    application::app::App,
    domain::{
        device::{PortForward, RegisteredDevice},
        errors::{ErrorCode, ErrorStage, LegacyAppError},
    },
};

impl App {
    pub fn register_device(&self, device: RegisteredDevice) -> Result<(), LegacyAppError> {
        let mut inner = self.inner.lock().unwrap();
        inner.config.registered_devices.retain(|d| d.uuid != device.uuid);
        inner.config.registered_devices.push(device.clone());
        self.store.persist_config(&inner.config)?;
        self.publish();
        Ok(())
    }

    pub fn update_device(&self, uuid: &str, device: RegisteredDevice) -> Result<(), LegacyAppError> {
        let mut inner = self.inner.lock().unwrap();
        if let Some(pos) = inner.config.registered_devices.iter().position(|d| d.uuid == uuid) {
            inner.config.registered_devices[pos] = device;
            self.store.persist_config(&inner.config)?;
            self.publish();
            Ok(())
        } else {
            Err(LegacyAppError::new(ErrorCode::NotFound, ErrorStage::Validate, "Device not found"))
        }
    }

    pub fn delete_device(&self, uuid: &str) -> Result<(), LegacyAppError> {
        let (is_static_ip, port_forwards) = {
            let mut inner = self.inner.lock().unwrap();
            let mut is_static_ip = false;
            let mut port_forwards = Vec::new();
            if let Some(device) = inner.config.registered_devices.iter().find(|d| d.uuid == uuid) {
                is_static_ip = device.is_static_ip;
                port_forwards = device.port_forwards.clone();
            }
            inner.config.registered_devices.retain(|d| d.uuid != uuid);
            self.store.persist_config(&inner.config)?;
            self.publish();
            (is_static_ip, port_forwards)
        };
        
        if is_static_ip {
            let _ = std::process::Command::new("uci")
                .args(["delete", &format!("dhcp.host_{}", uuid)])
                .output();
        }
        for pf in &port_forwards {
            let _ = std::process::Command::new("uci")
                .args(["delete", &format!("firewall.pf_{}", pf.id)])
                .output();
        }
        
        if is_static_ip || !port_forwards.is_empty() {
            let _ = std::process::Command::new("uci")
                .args(["commit"])
                .output();
            let _ = std::process::Command::new("reload_config")
                .output();
            if !port_forwards.is_empty() {
                let _ = std::process::Command::new("fw4")
                    .arg("reload")
                    .output();
            }
        }
        
        Ok(())
    }

    pub fn add_port_forward(&self, uuid: &str, pf: PortForward) -> Result<(), LegacyAppError> {
        let valid_protocols = ["tcp", "udp", "tcp udp", "tcp,udp"];
        if !valid_protocols.contains(&pf.protocol.to_lowercase().as_str()) {
            return Err(LegacyAppError::new(ErrorCode::InvalidArgument, ErrorStage::Validate, "Invalid protocol"));
        }
        if pf.external_port == 0 || pf.internal_port == 0 || pf.external_port > 65535 || pf.internal_port > 65535 {
            return Err(LegacyAppError::new(ErrorCode::InvalidArgument, ErrorStage::Validate, "Invalid port"));
        }

        let success = {
            let mut inner = self.inner.lock().unwrap();
            if let Some(device) = inner.config.registered_devices.iter_mut().find(|d| d.uuid == uuid) {
                device.port_forwards.retain(|p| p.id != pf.id);
                device.port_forwards.push(pf.clone());
                self.store.persist_config(&inner.config)?;
                self.publish();
                true
            } else {
                false
            }
        };

        if success {
            // Create port forward rule in UCI
            let section = format!("pf_{}", pf.id);
            let _ = std::process::Command::new("uci")
                .args(["set", &format!("firewall.{}=redirect", section)])
                .output();
            let _ = std::process::Command::new("uci")
                .args(["set", &format!("firewall.{}.target=DNAT", section)])
                .output();
            let _ = std::process::Command::new("uci")
                .args(["set", &format!("firewall.{}.src=wan", section)])
                .output();
            let _ = std::process::Command::new("uci")
                .args(["set", &format!("firewall.{}.dest=lan", section)])
                .output();
            let _ = std::process::Command::new("uci")
                .args(["set", &format!("firewall.{}.proto={}", section, pf.protocol)])
                .output();
            let _ = std::process::Command::new("uci")
                .args(["set", &format!("firewall.{}.src_dport={}", section, pf.external_port)])
                .output();
            let _ = std::process::Command::new("uci")
                .args(["set", &format!("firewall.{}.dest_port={}", section, pf.internal_port)])
                .output();
            let _ = std::process::Command::new("uci")
                .args(["set", &format!("firewall.{}.name={}", section, pf.id)])
                .output();
            let _ = std::process::Command::new("uci")
                .args(["commit", "firewall"])
                .output();
            Ok(())
        } else {
            Err(LegacyAppError::new(ErrorCode::NotFound, ErrorStage::Validate, "Device not found"))
        }
    }

    pub fn remove_port_forward(&self, uuid: &str, pf_id: &str) -> Result<(), LegacyAppError> {
        let success = {
            let mut inner = self.inner.lock().unwrap();
            if let Some(device) = inner.config.registered_devices.iter_mut().find(|d| d.uuid == uuid) {
                device.port_forwards.retain(|p| p.id != pf_id);
                self.store.persist_config(&inner.config)?;
                self.publish();
                true
            } else {
                false
            }
        };

        if success {
            // Delete port forward from UCI
            let _ = std::process::Command::new("uci")
                .args(["delete", &format!("firewall.pf_{}", pf_id)])
                .output();
            let _ = std::process::Command::new("uci")
                .args(["commit", "firewall"])
                .output();
            let _ = std::process::Command::new("fw4")
                .arg("reload")
                .output();
                
            Ok(())
        } else {
            Err(LegacyAppError::new(ErrorCode::NotFound, ErrorStage::Validate, "Device not found"))
        }
    }

    pub fn devices_sync_ip(&self, mac: &str, ip: Option<&str>, ip6: Option<&str>) {
        let port_forwards = {
            let inner = self.inner.lock().unwrap();
            let mac_lower = mac.to_lowercase();
            inner.config.registered_devices.iter()
                .find(|d| d.mac.to_lowercase() == mac_lower)
                .map(|d| d.port_forwards.clone())
                .unwrap_or_default()
        };

        if port_forwards.is_empty() {
            return;
        }

        // Use the most-specific address available for dest_ip.
        // IPv4 is preferred because most port-forward rules are v4.
        // If the device is v6-only, use ip6.
        let dest = ip.or(ip6);
        if let Some(addr) = dest {
            for pf in &port_forwards {
                let section = format!("pf_{}", pf.id);
                let _ = std::process::Command::new("uci")
                    .args(["set", &format!("firewall.{}.dest_ip={}", section, addr)])
                    .output();
            }
            let _ = std::process::Command::new("uci")
                .args(["commit", "firewall"])
                .output();
            let _ = std::process::Command::new("fw4")
                .arg("reload")
                .output();
        }
    }
}
