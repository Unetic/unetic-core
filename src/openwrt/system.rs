use std::fs;

use crate::system::SystemInfo;

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
        uptime_secs: parse_uptime(),
        load_average: parse_load_average(),
        memory_total_kb: parse_meminfo_field("MemTotal"),
        memory_available_kb: parse_meminfo_field("MemAvailable"),
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

fn parse_meminfo_field(field: &str) -> u64 {
    let content = fs::read_to_string("/proc/meminfo").unwrap_or_default();
    for line in content.lines() {
        let Some(rest) = line.strip_prefix(field) else {
            continue;
        };
        let Some(rest) = rest.strip_prefix(':') else {
            continue;
        };
        return rest
            .split_whitespace()
            .next()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
    }
    0
}
