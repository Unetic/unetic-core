use std::{collections::BTreeMap, process::Command};

use crate::domain::{
    DnsConfig, DnsRecord,
    errors::{ErrorCode, ErrorStage, LegacyAppError},
};

pub fn read_dns_config() -> Result<DnsConfig, LegacyAppError> {
    let output = run_uci(&["show", "dhcp"])?;
    Ok(parse_dns_config(&String::from_utf8_lossy(&output.stdout)))
}

pub fn write_dns_config(config: &DnsConfig) -> Result<(), LegacyAppError> {
    write_dnsmasq_options(config)?;
    replace_custom_records(config)?;
    run_uci(&["commit", "dhcp"])?;
    run_command("reload_config", &[])?;
    Ok(())
}

fn parse_dns_config(raw: &str) -> DnsConfig {
    let mut config = DnsConfig::default();
    let mut records = BTreeMap::<String, DnsRecord>::new();

    for line in raw.lines() {
        let Some((key, raw_value)) = line.split_once('=') else {
            continue;
        };
        let value = raw_value.trim_matches('\'');
        match key {
            "dhcp.@dnsmasq[0].server" => {
                config
                    .upstream
                    .extend(value.split_whitespace().map(str::to_owned));
            }
            "dhcp.@dnsmasq[0].local" => {
                let domain = value.trim_matches('/');
                if !domain.is_empty() {
                    config.local_domain = Some(domain.to_owned());
                }
            }
            "dhcp.lan.start" => update_number(value, &mut config.dhcp_start),
            "dhcp.lan.limit" => update_number(value, &mut config.dhcp_limit),
            "dhcp.lan.leasetime" => {
                update_number(value.trim_end_matches('h'), &mut config.dhcp_lease_hours);
            }
            _ => parse_record_line(key, value, &mut records),
        }
    }

    config.custom_records = records
        .into_values()
        .filter(|record| !record.hostname.is_empty() && !record.ip.is_empty())
        .collect();
    config
}

fn parse_record_line(key: &str, value: &str, records: &mut BTreeMap<String, DnsRecord>) {
    let Some(rest) = key.strip_prefix("dhcp.record_") else {
        return;
    };
    let (id, field) = rest
        .split_once('.')
        .map_or((rest, None), |(id, field)| (id, Some(field)));
    if id.is_empty() {
        return;
    }

    if field.is_none() && value != "domain" {
        return;
    }
    let record = records.entry(id.to_owned()).or_insert_with(|| DnsRecord {
        id: id.to_owned(),
        hostname: String::new(),
        ip: String::new(),
    });
    match field {
        Some("name") => record.hostname = value.to_owned(),
        Some("ip") => record.ip = value.to_owned(),
        _ => {}
    }
}

fn update_number(value: &str, target: &mut u32) {
    if let Ok(parsed) = value.parse() {
        *target = parsed;
    }
}

fn write_dnsmasq_options(config: &DnsConfig) -> Result<(), LegacyAppError> {
    if config.upstream.is_empty() {
        delete_if_present("dhcp.@dnsmasq[0].server")?;
    } else {
        uci_set(&format!(
            "dhcp.@dnsmasq[0].server={}",
            config.upstream.join(" ")
        ))?;
    }
    uci_set(&format!("dhcp.lan.start={}", config.dhcp_start))?;
    uci_set(&format!("dhcp.lan.limit={}", config.dhcp_limit))?;
    uci_set(&format!("dhcp.lan.leasetime={}h", config.dhcp_lease_hours))?;

    if let Some(domain) = &config.local_domain {
        uci_set(&format!("dhcp.@dnsmasq[0].local=/{domain}/"))?;
        uci_set(&format!("dhcp.@dnsmasq[0].domain={domain}"))?;
    } else {
        delete_if_present("dhcp.@dnsmasq[0].local")?;
        delete_if_present("dhcp.@dnsmasq[0].domain")?;
    }
    Ok(())
}

fn replace_custom_records(config: &DnsConfig) -> Result<(), LegacyAppError> {
    let output = run_uci(&["show", "dhcp"])?;
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let Some((section, kind)) = line.split_once('=') else {
            continue;
        };
        if section.starts_with("dhcp.record_") && kind.trim_matches('\'') == "domain" {
            delete_if_present(section)?;
        }
    }

    for record in &config.custom_records {
        let section = format!("dhcp.record_{}", record.id);
        uci_set(&format!("{section}=domain"))?;
        uci_set(&format!("{section}.name={}", record.hostname))?;
        uci_set(&format!("{section}.ip={}", record.ip))?;
    }
    Ok(())
}

fn uci_set(value: &str) -> Result<(), LegacyAppError> {
    run_uci(&["set", value]).map(|_| ())
}

fn delete_if_present(key: &str) -> Result<(), LegacyAppError> {
    let output = Command::new("uci")
        .args(["-q", "delete", key])
        .output()
        .map_err(command_error)?;
    if output.status.success() || output.status.code() == Some(1) {
        Ok(())
    } else {
        Err(command_failed("uci", &output))
    }
}

fn run_uci(args: &[&str]) -> Result<std::process::Output, LegacyAppError> {
    run_command("uci", args)
}

fn run_command(program: &str, args: &[&str]) -> Result<std::process::Output, LegacyAppError> {
    let output = Command::new(program)
        .args(args)
        .output()
        .map_err(command_error)?;
    if output.status.success() {
        Ok(output)
    } else {
        Err(command_failed(program, &output))
    }
}

fn command_error(error: std::io::Error) -> LegacyAppError {
    LegacyAppError::new(
        ErrorCode::UciApplyFailed,
        ErrorStage::Apply,
        error.to_string(),
    )
}

fn command_failed(program: &str, output: &std::process::Output) -> LegacyAppError {
    LegacyAppError::new(
        ErrorCode::UciApplyFailed,
        ErrorStage::Apply,
        format!(
            "{program} failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::parse_dns_config;

    #[test]
    fn parses_named_domain_records() {
        let raw = "dhcp.record_printer=domain\ndhcp.record_printer.name='printer.lan'\ndhcp.record_printer.ip='192.168.1.20'\n";
        let config = parse_dns_config(raw);

        assert_eq!(config.custom_records.len(), 1);
        assert_eq!(config.custom_records[0].id, "printer");
        assert_eq!(config.custom_records[0].hostname, "printer.lan");
        assert_eq!(config.custom_records[0].ip, "192.168.1.20");
    }
}
