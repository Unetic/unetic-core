use std::{fs, process::Command};

use crate::domain::{
    errors::{ErrorCode, ErrorStage, LegacyAppError},
    traffic::{TrafficBytes, TrafficCounters},
};

const NFT_FAMILY: &str = "bridge";
const NFT_TABLE: &str = "unetic_traffic";

pub fn read_traffic_counters(
    wan_interface: Option<&str>,
) -> Result<TrafficCounters, LegacyAppError> {
    reconcile_lan_counters()?;
    Ok(TrafficCounters {
        wan: wan_interface.map(read_interface).unwrap_or_default(),
        lan: TrafficBytes {
            rx: read_counter("lan_rx")?,
            tx: read_counter("lan_tx")?,
        },
    })
}

fn reconcile_lan_counters() -> Result<(), LegacyAppError> {
    let members = lan_members();
    let expected = members.join(", ");
    let current = run_nft(["list", "set", NFT_FAMILY, NFT_TABLE, "lan_members"], false)?;
    if current
        .as_deref()
        .is_some_and(|rules| rules.contains(&expected))
    {
        return Ok(());
    }
    let _ = run_nft(["delete", "table", NFT_FAMILY, NFT_TABLE], false)?;
    run_nft(["add", "table", NFT_FAMILY, NFT_TABLE], true)?;
    run_nft(
        [
            "add",
            "set",
            NFT_FAMILY,
            NFT_TABLE,
            "lan_members",
            "{",
            "type",
            "ifname",
            ";",
            "}",
        ],
        true,
    )?;
    if !members.is_empty() {
        let elements = format!("{{ {} }}", members.join(", "));
        run_nft(
            [
                "add",
                "element",
                NFT_FAMILY,
                NFT_TABLE,
                "lan_members",
                &elements,
            ],
            true,
        )?;
    }
    run_nft(
        [
            "add", "chain", NFT_FAMILY, NFT_TABLE, "forward", "{", "type", "filter", "hook",
            "forward", "priority", "0", ";", "policy", "accept", ";", "}",
        ],
        true,
    )?;
    run_nft(["add", "counter", NFT_FAMILY, NFT_TABLE, "lan_rx"], true)?;
    run_nft(["add", "counter", NFT_FAMILY, NFT_TABLE, "lan_tx"], true)?;
    run_nft(
        [
            "add",
            "rule",
            NFT_FAMILY,
            NFT_TABLE,
            "forward",
            "iifname",
            "@lan_members",
            "oifname",
            "@lan_members",
            "counter",
            "name",
            "lan_rx",
        ],
        true,
    )?;
    run_nft(
        [
            "add",
            "rule",
            NFT_FAMILY,
            NFT_TABLE,
            "forward",
            "oifname",
            "@lan_members",
            "iifname",
            "@lan_members",
            "counter",
            "name",
            "lan_tx",
        ],
        true,
    )?;
    Ok(())
}

fn lan_members() -> Vec<String> {
    let Ok(entries) = fs::read_dir("/sys/class/net/br-lan/brif") else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|entry| entry.file_name().into_string().ok())
        .collect()
}

fn read_interface(interface: &str) -> TrafficBytes {
    let Ok(content) = fs::read_to_string("/proc/net/dev") else {
        return TrafficBytes::default();
    };
    let Some(line) = content
        .lines()
        .find(|line| line.trim_start().starts_with(&format!("{interface}:")))
    else {
        return TrafficBytes::default();
    };
    let Some((_, stats)) = line.split_once(':') else {
        return TrafficBytes::default();
    };
    let values: Vec<_> = stats.split_whitespace().collect();
    TrafficBytes {
        rx: values
            .first()
            .and_then(|value| value.parse().ok())
            .unwrap_or(0),
        tx: values
            .get(8)
            .and_then(|value| value.parse().ok())
            .unwrap_or(0),
    }
}

fn read_counter(name: &str) -> Result<u64, LegacyAppError> {
    let output =
        run_nft(["list", "counter", NFT_FAMILY, NFT_TABLE, name], true)?.unwrap_or_default();
    output
        .split_whitespace()
        .collect::<Vec<_>>()
        .windows(2)
        .find_map(|pair| (pair[0] == "bytes").then(|| pair[1].parse().ok()).flatten())
        .ok_or_else(|| {
            LegacyAppError::new(
                ErrorCode::UbusUnavailable,
                ErrorStage::Verify,
                format!("nft counter {name} did not report bytes"),
            )
        })
}

fn run_nft<const N: usize>(
    arguments: [&str; N],
    required: bool,
) -> Result<Option<String>, LegacyAppError> {
    let output = Command::new("nft").args(arguments).output();
    let Ok(output) = output else {
        return if required {
            Err(LegacyAppError::new(
                ErrorCode::UbusUnavailable,
                ErrorStage::Transport,
                "nft is unavailable",
            ))
        } else {
            Ok(None)
        };
    };
    if !output.status.success() {
        return if required {
            Err(LegacyAppError::new(
                ErrorCode::UbusUnavailable,
                ErrorStage::Transport,
                String::from_utf8_lossy(&output.stderr),
            ))
        } else {
            Ok(None)
        };
    }
    Ok(Some(String::from_utf8_lossy(&output.stdout).into_owned()))
}
