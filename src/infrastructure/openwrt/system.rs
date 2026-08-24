use std::fs;

use nix::sys::statvfs::statvfs;

use crate::domain::system::{SystemInfo, SystemRuntime};

pub fn local_device_id() -> Option<String> {
    std::env::var("UNETIC_DEVICE_ID")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| read_trimmed("/sys/class/net/br-lan/address"))
}

pub fn read_system_info() -> SystemInfo {
    SystemInfo {
        hostname: read_trimmed("/proc/sys/kernel/hostname").unwrap_or_default(),
        model: read_trimmed("/tmp/sysinfo/model").unwrap_or_else(|| "Generic".into()),
        board_name: read_trimmed("/tmp/sysinfo/board_name").unwrap_or_default(),
        firmware_version: read_release_field("DISTRIB_RELEASE").unwrap_or_default(),
        firmware_revision: read_release_field("DISTRIB_REVISION").unwrap_or_default(),
        target: read_release_field("DISTRIB_TARGET").unwrap_or_default(),
        arch: read_release_field("DISTRIB_ARCH").unwrap_or_default(),
        kernel_version: parse_kernel_version(),
        cpu_count: parse_cpu_count(),
    }
}

pub fn read_system_runtime(reader: &super::temperature::TemperatureReader) -> SystemRuntime {
    let (memory_total_kb, memory_available_kb) = parse_meminfo();
    let (storage_total_kb, storage_available_kb) = read_storage();

    SystemRuntime {
        uptime_secs: parse_uptime(),
        load_average: parse_load_average(),
        memory_total_kb,
        memory_available_kb,
        storage_total_kb,
        storage_available_kb,
        temperatures: reader.read(),
    }
}

fn read_trimmed(path: &str) -> Option<String> {
    fs::read_to_string(path)
        .ok()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
}

fn read_release_field(field: &str) -> Option<String> {
    let content = fs::read_to_string("/etc/openwrt_release").ok()?;
    for line in content.lines() {
        let Some(rest) = line.strip_prefix(field) else {
            continue;
        };
        let Some(value) = rest.strip_prefix('=') else {
            continue;
        };
        return Some(value.trim_matches('\'').trim_matches('"').to_owned());
    }
    None
}

fn parse_kernel_version() -> String {
    fs::read_to_string("/proc/version")
        .ok()
        .and_then(|s| s.split_whitespace().nth(2).map(str::to_owned))
        .unwrap_or_default()
}

fn parse_cpu_count() -> u32 {
    let count = fs::read_to_string("/proc/cpuinfo")
        .ok()
        .map(|content| {
            content
                .lines()
                .filter(|line| line.starts_with("processor"))
                .count()
        })
        .unwrap_or(0);

    u32::try_from(count).unwrap_or(u32::MAX).max(1)
}

#[cfg(test)]
mod tests {
    use super::parse_cpu_count;

    #[test]
    fn cpu_count_is_at_least_one() {
        assert!(parse_cpu_count() >= 1);
    }
}

fn parse_uptime() -> u64 {
    fs::read_to_string("/proc/uptime")
        .ok()
        .and_then(|s| s.split_whitespace().next()?.parse::<f64>().ok())
        .map(|v| v as u64)
        .unwrap_or(0)
}

fn parse_load_average() -> [f32; 3] {
    let content = fs::read_to_string("/proc/loadavg").unwrap_or_default();
    let mut parts = content.split_whitespace();
    let a = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0.0);
    let b = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0.0);
    let c = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0.0);
    [a, b, c]
}

fn parse_meminfo() -> (u64, u64) {
    let content = fs::read_to_string("/proc/meminfo").unwrap_or_default();
    let mut total = 0;
    let mut available = 0;

    for line in content.lines() {
        if let Some(value) = parse_kb_field(line, "MemTotal") {
            total = value;
        } else if let Some(value) = parse_kb_field(line, "MemAvailable") {
            available = value;
        }
    }

    (total, available)
}

fn parse_kb_field(line: &str, field: &str) -> Option<u64> {
    line.strip_prefix(field)?
        .strip_prefix(':')?
        .split_whitespace()
        .next()?
        .parse()
        .ok()
}

fn read_storage() -> (u64, u64) {
    let Ok(stats) = statvfs("/") else {
        return (0, 0);
    };
    let block_size = stats.fragment_size();
    let total_kb = stats.blocks().saturating_mul(block_size) / 1024;
    let available_kb = stats.blocks_available().saturating_mul(block_size) / 1024;
    (total_kb, available_kb)
}
