use std::process::Command;

use crate::domain::{
    WanQos,
    errors::LegacyAppError,
};

pub fn parse_sqm_config(raw_output: &str) -> Option<WanQos> {
    let mut enabled = false;
    let mut download_kbps = None;
    let mut upload_kbps = None;

    for line in raw_output.lines() {
        let Some((key, raw_value)) = line.split_once('=') else {
            continue;
        };
        let value = raw_value.trim_matches('\'');
        match key {
            "sqm.wan.enabled" => enabled = value == "1" || value == "true",
            "sqm.wan.download" => download_kbps = value.parse::<u32>().ok(),
            "sqm.wan.upload" => upload_kbps = value.parse::<u32>().ok(),
            _ => {}
        }
    }

    if !enabled && download_kbps.is_none() && upload_kbps.is_none() {
        return None;
    }

    Some(WanQos {
        enabled,
        download_kbps,
        upload_kbps,
    })
}

pub fn read_sqm_config() -> Option<WanQos> {
    let output = Command::new("uci")
        .args(["show", "sqm"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    parse_sqm_config(&String::from_utf8_lossy(&output.stdout))
}

pub fn write_sqm_config(device: Option<&str>, qos: &Option<WanQos>) -> Result<(), LegacyAppError> {
    let Some(qos) = qos else {
        let _ = Command::new("uci").args(["-q", "delete", "sqm.wan"]).output();
        let _ = Command::new("uci").args(["commit", "sqm"]).output();
        let _ = Command::new("/etc/init.d/sqm").args(["stop"]).output();
        return Ok(());
    };

    let dev = device.unwrap_or("eth1");
    let enabled_str = if qos.enabled { "1" } else { "0" };

    let _ = Command::new("uci").args(["set", "sqm.wan=queue"]).output();
    let _ = Command::new("uci").args(["set", &format!("sqm.wan.enabled={enabled_str}")]).output();
    let _ = Command::new("uci").args(["set", &format!("sqm.wan.interface={dev}")]).output();
    let _ = Command::new("uci").args(["set", "sqm.wan.qdisc=cake"]).output();
    let _ = Command::new("uci").args(["set", "sqm.wan.script=piece_of_cake.qos"]).output();

    if let Some(dl) = qos.download_kbps {
        let _ = Command::new("uci").args(["set", &format!("sqm.wan.download={dl}")]).output();
    }
    if let Some(ul) = qos.upload_kbps {
        let _ = Command::new("uci").args(["set", &format!("sqm.wan.upload={ul}")]).output();
    }

    let _ = Command::new("uci").args(["commit", "sqm"]).output();

    if qos.enabled {
        let _ = Command::new("/etc/init.d/sqm").args(["restart"]).output();
    } else {
        let _ = Command::new("/etc/init.d/sqm").args(["stop"]).output();
    }

    Ok(())
}
