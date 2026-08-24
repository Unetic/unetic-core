use std::collections::HashMap;
use std::fs;
use std::process::Command;

use crate::domain::{device::Device, device_inventory::DeviceRuntime};
use crate::domain::{
    errors::{ErrorCode, ErrorStage, LegacyAppError},
    ports::{HardwareOffload, PhysicalPort, PortConnection, PortSpeed, PortType, SwitchState},
};

pub(crate) fn read_switch_state() -> Result<SwitchState, LegacyAppError> {
    let software = uci_get("flow_offloading")?;
    let hardware = uci_get("flow_offloading_hw")?;
    Ok(SwitchState {
        hw_offload: HardwareOffload {
            available: hardware.is_some(),
            enabled: software.as_deref() == Some("1") && hardware.as_deref() == Some("1"),
        },
    })
}

pub(crate) fn set_hw_offload(enabled: bool) -> Result<SwitchState, LegacyAppError> {
    let previous = read_switch_state()?;
    let previous_software = uci_get("flow_offloading")?.as_deref() == Some("1");
    if enabled && !previous.hw_offload.available {
        return Err(LegacyAppError::new(
            ErrorCode::InvalidArgument,
            ErrorStage::Validate,
            "hardware flow offload is unavailable",
        ));
    }
    if previous.hw_offload.enabled == enabled {
        return Ok(previous);
    }
    if enabled {
        set_uci("flow_offloading", "1")?;
    }
    set_uci("flow_offloading_hw", if enabled { "1" } else { "0" })?;
    if let Err(error) = reload_firewall().and_then(|()| verify_hw_offload(enabled)) {
        let _ = restore_switch_state(previous_software, previous.hw_offload.enabled);
        return Err(error);
    }
    read_switch_state()
}

fn verify_hw_offload(enabled: bool) -> Result<(), LegacyAppError> {
    if read_switch_state()?.hw_offload.enabled == enabled {
        return Ok(());
    }
    Err(LegacyAppError::new(
        ErrorCode::VerifyMismatch,
        ErrorStage::Verify,
        "firewall did not apply hardware flow offload",
    ))
}

fn restore_switch_state(
    software_enabled: bool,
    hardware_enabled: bool,
) -> Result<(), LegacyAppError> {
    set_uci("flow_offloading", if software_enabled { "1" } else { "0" })?;
    set_uci(
        "flow_offloading_hw",
        if hardware_enabled { "1" } else { "0" },
    )?;
    reload_firewall()
}

fn uci_get(option: &str) -> Result<Option<String>, LegacyAppError> {
    let assignment = format!("firewall.@defaults[0].{option}");
    let output = Command::new("uci")
        .args(["-q", "get", &assignment])
        .output()
        .map_err(|error| {
            LegacyAppError::new(
                ErrorCode::UciReadFailed,
                ErrorStage::Verify,
                error.to_string(),
            )
        })?;
    if !output.status.success() {
        return Ok(None);
    }
    Ok(Some(
        String::from_utf8_lossy(&output.stdout).trim().to_owned(),
    ))
}

fn set_uci(option: &str, value: &str) -> Result<(), LegacyAppError> {
    let assignment = format!("firewall.@defaults[0].{option}={value}");
    run_command("uci", ["set", &assignment])?;
    run_command("uci", ["commit", "firewall"])
}

fn reload_firewall() -> Result<(), LegacyAppError> {
    run_command("/etc/init.d/firewall", ["reload"])
}

fn run_command<const N: usize>(program: &str, args: [&str; N]) -> Result<(), LegacyAppError> {
    let output = Command::new(program).args(args).output().map_err(|error| {
        LegacyAppError::new(
            ErrorCode::UciApplyFailed,
            ErrorStage::Apply,
            error.to_string(),
        )
    })?;
    if output.status.success() {
        return Ok(());
    }
    Err(LegacyAppError::new(
        ErrorCode::UciApplyFailed,
        ErrorStage::Apply,
        String::from_utf8_lossy(&output.stderr),
    ))
}

fn parse_speed(speed_str: &str) -> PortSpeed {
    match speed_str.trim() {
        "10" => PortSpeed::Speed10,
        "100" => PortSpeed::Speed100,
        "1000" => PortSpeed::Speed1000,
        "2500" => PortSpeed::Speed2500,
        "5000" => PortSpeed::Speed5000,
        "10000" => PortSpeed::Speed10000,
        _ => PortSpeed::NoLink,
    }
}

pub(crate) fn ports_list(devices: &[Device], wan_interface: Option<&str>) -> Vec<PhysicalPort> {
    let mut ports = Vec::new();

    let mut mac_to_iface: HashMap<String, String> = HashMap::new();
    if let Ok(output) = Command::new("bridge").args(["fdb", "show"]).output() {
        if output.status.success() {
            if let Ok(fdb_str) = String::from_utf8(output.stdout) {
                for line in fdb_str.lines() {
                    let parts: Vec<&str> = line.split_whitespace().collect();
                    if parts.len() >= 3 && parts[1] == "dev" {
                        let mac = parts[0].to_lowercase();
                        let iface = parts[2].to_string();
                        mac_to_iface.insert(mac, iface);
                    }
                }
            }
        }
    }

    let mut iface_to_connections: HashMap<String, Vec<PortConnection>> = HashMap::new();
    for device in devices {
        let mac = device.mac.to_lowercase();
        if let Some(iface) = mac_to_iface.get(&mac) {
            let conn = PortConnection {
                device_id: DeviceRuntime::id_for_mac(&mac),
            };
            iface_to_connections
                .entry(iface.clone())
                .or_default()
                .push(conn);
        }
    }

    let mut lan_ifaces = Vec::new();
    if let Ok(entries) = fs::read_dir("/sys/class/net/br-lan/brif") {
        for entry in entries.flatten() {
            if let Ok(file_name) = entry.file_name().into_string() {
                if !file_name.starts_with("wlan") {
                    lan_ifaces.push(file_name);
                }
            }
        }
    }

    for iface in lan_ifaces {
        let mut speed = PortSpeed::NoLink;
        if let Ok(operstate) = fs::read_to_string(format!("/sys/class/net/{}/operstate", iface)) {
            if operstate.trim() == "up" {
                if let Ok(speed_str) = fs::read_to_string(format!("/sys/class/net/{}/speed", iface))
                {
                    speed = parse_speed(&speed_str);
                }
            }
        }
        let connections = iface_to_connections.remove(&iface).unwrap_or_default();
        ports.push(PhysicalPort {
            id: iface.clone(),
            port_type: PortType::Lan,
            speed,
            connections,
        });
    }

    let wan_iface = wan_interface
        .filter(|interface| fs::metadata(format!("/sys/class/net/{interface}")).is_ok())
        .map(str::to_owned)
        .or_else(|| existing_interface("wan"))
        .or_else(|| existing_interface("eth0"));

    if let Some(wan) = wan_iface {
        let mut speed = PortSpeed::NoLink;
        if let Ok(operstate) = fs::read_to_string(format!("/sys/class/net/{}/operstate", wan)) {
            if operstate.trim() == "up" {
                if let Ok(speed_str) = fs::read_to_string(format!("/sys/class/net/{}/speed", wan)) {
                    speed = parse_speed(&speed_str);
                }
            }
        }
        let connections = iface_to_connections.remove(&wan).unwrap_or_default();
        ports.push(PhysicalPort {
            id: wan.clone(),
            port_type: PortType::Wan,
            speed,
            connections,
        });
    }

    ports
}

fn existing_interface(interface: &str) -> Option<String> {
    fs::metadata(format!("/sys/class/net/{interface}"))
        .is_ok()
        .then(|| interface.to_owned())
}
