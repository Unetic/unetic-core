use std::process::{Command, Output};

use crate::domain::{
    device::{Device, RegisteredDevice},
    errors::{ErrorCode, ErrorStage, LegacyAppError},
};

pub fn write_static_lease(
    mac: &str,
    ip: &str,
    hostname: Option<&str>,
) -> Result<(), LegacyAppError> {
    let section = lease_section(mac);
    uci_set(&format!("dhcp.{section}=host"))?;
    uci_set(&format!("dhcp.{section}.mac={mac}"))?;
    uci_set(&format!("dhcp.{section}.ip={ip}"))?;
    if let Some(hostname) = hostname {
        uci_set(&format!("dhcp.{section}.name={hostname}"))?;
    }
    commit("dhcp")?;
    run("reload_config", &[])?;
    Ok(())
}

pub fn delete_static_lease(mac: &str) -> Result<(), LegacyAppError> {
    let section = format!("dhcp.{}", lease_section(mac));
    uci_delete_if_present(&section)?;
    commit("dhcp")?;
    run("reload_config", &[])?;
    Ok(())
}

pub fn sync_port_forwards(
    registered_devices: &[RegisteredDevice],
    current_devices: &[Device],
) -> Result<(), LegacyAppError> {
    let managed_sections = managed_forward_sections()?;
    let has_desired_rules = registered_devices
        .iter()
        .any(|device| !device.port_forwards.is_empty());
    if managed_sections.is_empty() && !has_desired_rules {
        return Ok(());
    }

    for section in managed_sections {
        uci_delete_if_present(&section)?;
    }

    for registered in registered_devices {
        let destination = current_devices
            .iter()
            .find(|device| device.mac.eq_ignore_ascii_case(&registered.mac))
            .and_then(|device| device.ip.as_deref().or(device.ip6.as_deref()));
        let Some(destination) = destination else {
            continue;
        };

        for rule in &registered.port_forwards {
            write_port_forward(rule, destination)?;
        }
    }

    commit("firewall")?;
    run("fw4", &["reload"])?;
    Ok(())
}

fn write_port_forward(
    rule: &crate::domain::device::PortForward,
    destination: &str,
) -> Result<(), LegacyAppError> {
    let section = format!("firewall.pf_{}", rule.id);
    for value in [
        format!("{section}=redirect"),
        format!("{section}.target=DNAT"),
        format!("{section}.src=wan"),
        format!("{section}.dest=lan"),
        format!("{section}.reflection=1"),
        format!("{section}.proto={}", rule.protocol.uci_value()),
        format!("{section}.src_dport={}", rule.external_port),
        format!("{section}.dest_port={}", rule.internal_port),
        format!("{section}.dest_ip={destination}"),
        format!("{section}.name={}", rule.id),
    ] {
        uci_set(&value)?;
    }
    Ok(())
}

fn managed_forward_sections() -> Result<Vec<String>, LegacyAppError> {
    let output = run("uci", &["show", "firewall"])?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(stdout
        .lines()
        .filter_map(|line| {
            let (section, kind) = line.split_once('=')?;
            (section.starts_with("firewall.pf_") && kind.trim_matches('\'') == "redirect")
                .then(|| section.to_owned())
        })
        .collect())
}

fn lease_section(mac: &str) -> String {
    format!("unetic_{}", mac.replace(':', ""))
}

fn uci_set(value: &str) -> Result<(), LegacyAppError> {
    run("uci", &["set", value]).map(|_| ())
}

fn uci_delete_if_present(section: &str) -> Result<(), LegacyAppError> {
    let output = Command::new("uci")
        .args(["-q", "delete", section])
        .output()
        .map_err(command_error)?;
    if output.status.success() || output.status.code() == Some(1) {
        Ok(())
    } else {
        Err(failed_command("uci", &output))
    }
}

fn commit(config: &str) -> Result<(), LegacyAppError> {
    run("uci", &["commit", config]).map(|_| ())
}

fn run(program: &str, args: &[&str]) -> Result<Output, LegacyAppError> {
    let output = Command::new(program)
        .args(args)
        .output()
        .map_err(command_error)?;
    if output.status.success() {
        Ok(output)
    } else {
        Err(failed_command(program, &output))
    }
}

fn command_error(error: std::io::Error) -> LegacyAppError {
    LegacyAppError::new(
        ErrorCode::UciApplyFailed,
        ErrorStage::Apply,
        error.to_string(),
    )
}

fn failed_command(program: &str, output: &Output) -> LegacyAppError {
    let stderr = String::from_utf8_lossy(&output.stderr);
    LegacyAppError::new(
        ErrorCode::UciApplyFailed,
        ErrorStage::Apply,
        format!("{program} failed: {}", stderr.trim()),
    )
}
