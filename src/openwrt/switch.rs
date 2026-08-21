use std::{fs, path::Path};

use crate::switch::{
    SwitchArchitecture, SwitchFeatureStatus, SwitchFeatures, SwitchInfo, SwitchSocInfo,
};

pub fn read_switch_info(sys_root: &Path, debug_root: &Path) -> SwitchInfo {
    let dsa_ports = discover_dsa_ports(sys_root);
    if dsa_ports.is_empty() {
        return probe_software_or_swconfig_switch(sys_root, debug_root);
    }

    let soc = discover_dsa_soc(sys_root, &dsa_ports);
    let features = probe_dsa_features(sys_root, debug_root, &dsa_ports);

    SwitchInfo { soc, features }
}

fn discover_dsa_ports(sys_root: &Path) -> Vec<String> {
    let net_dir = sys_root.join("class/net");
    let mut ports = Vec::new();

    let Ok(entries) = fs::read_dir(net_dir) else {
        return ports;
    };

    for entry in entries.flatten() {
        let uevent_path = entry.path().join("uevent");
        let Ok(uevent_content) = fs::read_to_string(uevent_path) else {
            continue;
        };

        if uevent_content.lines().any(|l| l == "DEVTYPE=dsa") {
            if let Some(name) = entry.file_name().to_str() {
                ports.push(name.to_owned());
            }
        }
    }

    ports.sort();
    ports
}

fn discover_dsa_soc(sys_root: &Path, ports: &[String]) -> SwitchSocInfo {
    let first_port = &ports[0];
    let port_dir = sys_root.join("class/net").join(first_port);

    let driver = read_symlink_name(&port_dir.join("device/driver"));
    let of_node = port_dir.join("device/of_node");
    let compatible = read_device_compatible(&of_node.join("compatible"))
        .or_else(|| read_device_compatible(&of_node.join("../compatible")))
        .or_else(|| read_dts_compatible(sys_root));

    let (vendor, model) = parse_vendor_and_model(compatible.as_deref(), driver.as_deref());
    let tagging_protocol = read_tagging_protocol(sys_root, &port_dir);

    SwitchSocInfo {
        model,
        vendor,
        compatible,
        driver,
        architecture: SwitchArchitecture::Dsa,
        tagging_protocol,
        ports: ports.to_vec(),
    }
}

fn parse_vendor_and_model(compatible: Option<&str>, driver: Option<&str>) -> (String, String) {
    if let Some(comp) = compatible {
        let first = comp.split('\0').next().unwrap_or(comp);
        if let Some((vendor_raw, model_raw)) = first.split_once(',') {
            let vendor = match vendor_raw {
                "mediatek" | "mtk" => "MediaTek",
                "qcom" | "qualcomm" => "Qualcomm",
                "realtek" | "rtl" => "Realtek",
                "marvell" | "mrvl" => "Marvell",
                "brcm" | "broadcom" => "Broadcom",
                other => other,
            };
            return (vendor.to_owned(), model_raw.to_owned());
        }
        return ("Unknown".into(), first.to_owned());
    }

    if let Some(drv) = driver {
        let clean = drv.trim_end_matches("-mdio").trim_end_matches("-srab");
        return ("Unknown".into(), clean.to_owned());
    }

    ("Unknown".into(), "dsa_switch".into())
}

fn read_tagging_protocol(sys_root: &Path, port_dir: &Path) -> Option<String> {
    let direct = port_dir.join("dsa/tagging");
    if let Ok(content) = fs::read_to_string(direct) {
        return Some(content.trim().to_owned());
    }

    let net_dir = sys_root.join("class/net");
    let entries = fs::read_dir(net_dir).ok()?;

    for entry in entries.flatten() {
        let candidate = entry.path().join("dsa/tagging");
        if let Ok(content) = fs::read_to_string(candidate) {
            return Some(content.trim().to_owned());
        }
    }

    None
}

fn read_symlink_name(path: &Path) -> Option<String> {
    let target = fs::read_link(path).ok()?;
    target
        .file_name()
        .and_then(|n| n.to_str())
        .map(str::to_owned)
}

fn read_device_compatible(path: &Path) -> Option<String> {
    let bytes = fs::read(path).ok()?;
    let text = String::from_utf8_lossy(&bytes);
    let first = text.split('\0').next()?.trim();
    if first.is_empty() {
        None
    } else {
        Some(first.to_owned())
    }
}

fn read_dts_compatible(sys_root: &Path) -> Option<String> {
    let dt_sys = sys_root.join("firmware/devicetree/base/compatible");
    if let Some(comp) = read_device_compatible(&dt_sys) {
        return Some(comp);
    }
    read_device_compatible(Path::new("/proc/device-tree/compatible"))
}

fn probe_dsa_features(sys_root: &Path, debug_root: &Path, ports: &[String]) -> SwitchFeatures {
    let has_bridge = has_active_bridge(sys_root);
    let l3_hw_supported = debug_root.join("ppe").exists()
        || debug_root.join("mtk_ppe").exists()
        || sys_root.join("module/nf_flow_table_hw").exists()
        || debug_root.join("qca-nss-drv").exists();

    let l3_sw_supported = sys_root.join("module/nf_flow_table").exists() || l3_hw_supported;
    let vlan_filtering = is_bridge_flag_enabled(sys_root, "vlan_filtering");
    let igmp_snooping = is_bridge_flag_enabled(sys_root, "multicast_snooping");
    let stp_active = is_bridge_flag_enabled(sys_root, "stp_state");
    let port_isolation = is_port_flag_enabled(sys_root, ports, "brport/isolated");
    let jumbo_frames = has_jumbo_frame_support(sys_root, ports);

    SwitchFeatures {
        l2_hw_switching: SwitchFeatureStatus::static_hw(true),
        l3_hw_flow_offload: SwitchFeatureStatus::new(l3_hw_supported, l3_hw_supported, true),
        l3_sw_flow_offload: SwitchFeatureStatus::new(l3_sw_supported, l3_sw_supported, true),
        vlan_filtering_8021q: SwitchFeatureStatus::new(true, vlan_filtering, true),
        port_isolation: SwitchFeatureStatus::new(true, port_isolation, true),
        hw_igmp_snooping: SwitchFeatureStatus::new(true, igmp_snooping, true),
        flow_control_8023x: SwitchFeatureStatus::new(true, false, true),
        eee_8023az: SwitchFeatureStatus::new(true, false, true),
        stp_rstp: SwitchFeatureStatus::new(true, stp_active, true),
        mirroring_span: SwitchFeatureStatus::new(true, false, true),
        jumbo_frames: SwitchFeatureStatus::new(jumbo_frames.0, jumbo_frames.1, true),
        link_aggregation_lag: SwitchFeatureStatus::new(true, false, true),
        tdr_cable_diag: SwitchFeatureStatus::static_hw(true),
        hardware_stats: SwitchFeatureStatus::static_hw(has_bridge),
    }
}

fn probe_software_or_swconfig_switch(sys_root: &Path, debug_root: &Path) -> SwitchInfo {
    let mut info = SwitchInfo::generic_software();
    let l3_hw = debug_root.join("ppe").exists()
        || debug_root.join("mtk_ppe").exists()
        || sys_root.join("module/nf_flow_table_hw").exists();

    if l3_hw {
        info.features.l3_hw_flow_offload = SwitchFeatureStatus::new(true, true, true);
    }

    info
}

fn has_active_bridge(sys_root: &Path) -> bool {
    let Ok(entries) = fs::read_dir(sys_root.join("class/net")) else {
        return false;
    };

    entries.flatten().any(|e| e.path().join("bridge").exists())
}

fn is_bridge_flag_enabled(sys_root: &Path, flag: &str) -> bool {
    let Ok(entries) = fs::read_dir(sys_root.join("class/net")) else {
        return false;
    };

    for entry in entries.flatten() {
        let flag_path = entry.path().join("bridge").join(flag);
        if let Ok(content) = fs::read_to_string(flag_path) {
            let val = content.trim();
            if val == "1" || (val.parse::<u32>().unwrap_or(0) > 0) {
                return true;
            }
        }
    }

    false
}

fn is_port_flag_enabled(sys_root: &Path, ports: &[String], subpath: &str) -> bool {
    for port in ports {
        let flag_path = sys_root.join("class/net").join(port).join(subpath);
        if let Ok(content) = fs::read_to_string(flag_path) {
            if content.trim() == "1" {
                return true;
            }
        }
    }

    false
}

fn has_jumbo_frame_support(sys_root: &Path, ports: &[String]) -> (bool, bool) {
    let mut supported = false;
    let mut enabled = false;

    for port in ports {
        let port_dir = sys_root.join("class/net").join(port);
        if let Ok(content) = fs::read_to_string(port_dir.join("max_mtu")) {
            if let Ok(max) = content.trim().parse::<u32>() {
                if max > 1500 {
                    supported = true;
                }
            }
        }
        if let Ok(content) = fs::read_to_string(port_dir.join("mtu")) {
            if let Ok(cur) = content.trim().parse::<u32>() {
                if cur > 1500 {
                    enabled = true;
                }
            }
        }
    }

    (supported, enabled)
}
