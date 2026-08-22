use std::process::Command;
use crate::domain::{DnsConfig, DnsRecord};
use crate::domain::errors::{LegacyAppError, ErrorCode, ErrorStage};

pub fn read_dns_config() -> DnsConfig {
    let mut config = DnsConfig::default();
    let output = Command::new("uci")
        .arg("show")
        .arg("dhcp")
        .output()
        .expect("failed to execute uci show dhcp");

    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            if line.starts_with("dhcp.@dnsmasq[0].server=") {
                if let Some(servers_str) = line.split('=').nth(1) {
                    let servers_str = servers_str.trim_matches('\'');
                    config.upstream = servers_str.split(' ').map(|s| s.to_string()).collect();
                }
            } else if line.starts_with("dhcp.@dnsmasq[0].local=") {
                if let Some(val) = line.split('=').nth(1) {
                    let val = val.trim_matches('\'').trim_start_matches('/').trim_end_matches('/');
                    if !val.is_empty() {
                        config.local_domain = Some(val.to_string());
                    }
                }
            } else if line.starts_with("dhcp.lan.start=") {
                if let Some(val) = line.split('=').nth(1) {
                    if let Ok(v) = val.trim_matches('\'').parse::<u32>() {
                        config.dhcp_start = v;
                    }
                }
            } else if line.starts_with("dhcp.lan.limit=") {
                if let Some(val) = line.split('=').nth(1) {
                    if let Ok(v) = val.trim_matches('\'').parse::<u32>() {
                        config.dhcp_limit = v;
                    }
                }
            } else if line.starts_with("dhcp.lan.leasetime=") {
                if let Some(val) = line.split('=').nth(1) {
                    let val = val.trim_matches('\'').trim_end_matches('h');
                    if let Ok(v) = val.parse::<u32>() {
                        config.dhcp_lease_hours = v;
                    }
                }
            }
        }
        
        // Custom records... a bit trickier to parse from `uci show dhcp` if we want full fidelity.
        // We look for dhcp.record_id=domain
        // dhcp.record_id.name=hostname
        // dhcp.record_id.ip=ip
        // For simplicity, we can parse them in a second pass.
        let mut records: std::collections::HashMap<String, DnsRecord> = std::collections::HashMap::new();
        for line in stdout.lines() {
            if line.starts_with("dhcp.") && line.contains("=domain") && !line.contains('.') {
                // wait, format is dhcp.record_id=domain
                if let Some(id_part) = line.split('=').next() {
                    let id = id_part.trim_start_matches("dhcp.");
                    if !records.contains_key(id) {
                        records.insert(id.to_string(), DnsRecord { id: id.to_string(), hostname: String::new(), ip: String::new() });
                    }
                }
            }
        }
        for line in stdout.lines() {
            for (id, record) in records.iter_mut() {
                if line.starts_with(&format!("dhcp.{}.name=", id)) {
                    if let Some(val) = line.split('=').nth(1) {
                        record.hostname = val.trim_matches('\'').to_string();
                    }
                } else if line.starts_with(&format!("dhcp.{}.ip=", id)) {
                    if let Some(val) = line.split('=').nth(1) {
                        record.ip = val.trim_matches('\'').to_string();
                    }
                }
            }
        }
        config.custom_records = records.into_values().collect();
    }
    
    config
}

pub fn write_dns_config(cfg: &DnsConfig) -> Result<(), LegacyAppError> {
    let mut cmds: Vec<Vec<String>> = vec![];
    
    if cfg.upstream.is_empty() {
        cmds.push(vec!["delete".to_string(), "dhcp.@dnsmasq[0].server".to_string()]);
    } else {
        cmds.push(vec!["set".to_string(), format!("dhcp.@dnsmasq[0].server={}", cfg.upstream.join(" "))]);
    }
    
    cmds.push(vec!["set".to_string(), format!("dhcp.lan.start={}", cfg.dhcp_start)]);
    cmds.push(vec!["set".to_string(), format!("dhcp.lan.limit={}", cfg.dhcp_limit)]);
    cmds.push(vec!["set".to_string(), format!("dhcp.lan.leasetime={}h", cfg.dhcp_lease_hours)]);
    
    if let Some(local) = &cfg.local_domain {
        cmds.push(vec!["set".to_string(), format!("dhcp.@dnsmasq[0].local=/{}/", local)]);
        cmds.push(vec!["set".to_string(), format!("dhcp.@dnsmasq[0].domain={}", local)]);
    } else {
        cmds.push(vec!["delete".to_string(), "dhcp.@dnsmasq[0].local".to_string()]);
        cmds.push(vec!["delete".to_string(), "dhcp.@dnsmasq[0].domain".to_string()]);
    }
    
    // Custom records
    // First, clear existing records.
    let output = Command::new("uci")
        .arg("show")
        .arg("dhcp")
        .output()
        .map_err(|e| LegacyAppError::new(ErrorCode::UciApplyFailed, ErrorStage::Apply, e.to_string()))?;
    
    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        for line in stdout.lines() {
            if line.starts_with("dhcp.record_") && line.contains("=domain") {
                if let Some(id_part) = line.split('=').next() {
                    cmds.push(vec!["delete".to_string(), id_part.to_string()]);
                }
            }
        }
    }
    
    for record in &cfg.custom_records {
        let section = format!("dhcp.record_{}", record.id);
        cmds.push(vec!["set".to_string(), format!("{}=domain", section)]);
        cmds.push(vec!["set".to_string(), format!("{}.name={}", section, record.hostname)]);
        cmds.push(vec!["set".to_string(), format!("{}.ip={}", section, record.ip)]);
    }
    
    for args in cmds {
        let mut cmd = Command::new("uci");
        cmd.args(&args);
        // ignore errors on delete if doesn't exist
        let _ = cmd.output();
    }
    
    let res = Command::new("uci")
        .arg("commit")
        .arg("dhcp")
        .output()
        .map_err(|e| LegacyAppError::new(ErrorCode::UciApplyFailed, ErrorStage::Apply, e.to_string()))?;
        
    if !res.status.success() {
        return Err(LegacyAppError::new(ErrorCode::UciApplyFailed, ErrorStage::Apply, "uci commit dhcp failed".to_string()));
    }
    
    let _ = Command::new("reload_config").output();
    
    Ok(())
}
