use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use crate::domain::system::{TemperatureReading, TemperatureSource};

pub struct TemperatureReader {
    wifi_sources: HashMap<String, TemperatureSource>,
}

impl TemperatureReader {
    pub fn new() -> Self {
        Self {
            wifi_sources: detect_wifi_sources(),
        }
    }

    pub fn read(&self) -> Vec<TemperatureReading> {
        let mut readings = Vec::new();

        self.read_hwmon(&mut readings);
        self.read_thermal_zones(&mut readings);
        readings.sort_by_key(|reading| reading.source.sort_order());
        readings
    }

    fn read_hwmon(&self, readings: &mut Vec<TemperatureReading>) {
        for directory in sorted_entries("/sys/class/hwmon") {
            let name = read_text(directory.join("name")).unwrap_or_default();
            let device = fs::canonicalize(directory.join("device"))
                .ok()
                .map_or_else(String::new, |path| path.to_string_lossy().into_owned());

            for input in sorted_entries(&directory) {
                let Some(file_name) = input.file_name().and_then(|name| name.to_str()) else {
                    continue;
                };
                let Some(channel) = file_name.strip_suffix("_input") else {
                    continue;
                };
                if !channel.starts_with("temp") || sensor_faulted(&directory, channel) {
                    continue;
                }

                let label =
                    read_text(directory.join(format!("{channel}_label"))).unwrap_or_default();
                let metadata = format!("{name} {label} {device}");
                self.add_reading(readings, &metadata, &input);
            }
        }
    }

    fn read_thermal_zones(&self, readings: &mut Vec<TemperatureReading>) {
        for directory in sorted_entries("/sys/class/thermal") {
            let Some(name) = directory.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            if !name.starts_with("thermal_zone") {
                continue;
            }

            let zone_type = read_text(directory.join("type")).unwrap_or_default();
            let device = fs::canonicalize(directory.join("device"))
                .ok()
                .map_or_else(String::new, |path| path.to_string_lossy().into_owned());
            let metadata = format!("{zone_type} {device}");
            self.add_reading(readings, &metadata, &directory.join("temp"));
        }
    }

    fn add_reading(&self, readings: &mut Vec<TemperatureReading>, metadata: &str, input: &Path) {
        let Some(source) = self.classify_source(metadata) else {
            return;
        };
        if readings.iter().any(|reading| reading.source == source) {
            return;
        }
        let Some(temp_celsius) = read_temperature(input) else {
            return;
        };

        readings.push(TemperatureReading {
            source,
            temp_celsius,
        });
    }

    fn classify_source(&self, metadata: &str) -> Option<TemperatureSource> {
        let value = metadata.to_ascii_lowercase();

        if contains_any(&value, &["wifi24", "wifi_24", "2.4ghz", "2.4g"]) {
            return Some(TemperatureSource::Wifi24);
        }
        if contains_any(&value, &["wifi5", "wifi_5", "5ghz"]) {
            return Some(TemperatureSource::Wifi5);
        }
        for (phy, source) in &self.wifi_sources {
            if value.contains(phy) {
                return Some(*source);
            }
        }

        if contains_any(&value, &["sfp", "transceiver", "optical"]) {
            return Some(TemperatureSource::Sfp);
        }
        if contains_any(&value, &["nvme", "ssd"]) {
            return Some(TemperatureSource::Ssd);
        }
        if contains_any(
            &value,
            &[
                "modem", "wwan", "qmi", "lte", "quectel", "fibocom", "sierra",
            ],
        ) {
            return Some(TemperatureSource::Modem);
        }
        if contains_any(&value, &["poe", "pse"]) {
            return Some(TemperatureSource::Poe);
        }
        if contains_any(&value, &["switch", "dsa", "mv88", "rtl83", "rtl93", "qca8"]) {
            return Some(TemperatureSource::Switch);
        }
        if contains_any(&value, &["pcb", "board", "ambient", "chassis"]) {
            return Some(TemperatureSource::Pcb);
        }
        if contains_any(
            &value,
            &[
                "soc", "cpu", "package", "die", "coretemp", "k10temp", "thermal", "mediatek",
                "mtk", "ipq", "rockchip", "sunxi", "bcm",
            ],
        ) {
            return Some(TemperatureSource::Soc);
        }

        None
    }
}

fn detect_wifi_sources() -> HashMap<String, TemperatureSource> {
    let mut sources = HashMap::new();

    for path in sorted_entries("/sys/class/ieee80211") {
        let Some(phy) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if let Some(source) = detect_wifi_source(phy) {
            sources.insert(phy.to_ascii_lowercase(), source);
        }
    }

    sources
}

fn detect_wifi_source(phy: &str) -> Option<TemperatureSource> {
    let output = Command::new("iw")
        .args(["phy", phy, "info"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }

    let text = String::from_utf8_lossy(&output.stdout);
    let has_24 = text
        .lines()
        .any(|line| line_has_frequency(line, 2400, 2500));
    let has_5 = text
        .lines()
        .any(|line| line_has_frequency(line, 4900, 5900));

    match (has_24, has_5) {
        (true, false) => Some(TemperatureSource::Wifi24),
        (false, true) => Some(TemperatureSource::Wifi5),
        _ => None,
    }
}

fn line_has_frequency(line: &str, minimum: u32, maximum: u32) -> bool {
    line.split(|character: char| !character.is_ascii_digit())
        .filter_map(|part| part.parse::<u32>().ok())
        .any(|frequency| (minimum..=maximum).contains(&frequency))
}

fn read_temperature(path: &Path) -> Option<f32> {
    let millidegrees = read_text(path)?.parse::<i64>().ok()?;
    let celsius = millidegrees as f32 / 1000.0;
    (-50.0..=250.0).contains(&celsius).then_some(celsius)
}

fn sensor_faulted(directory: &Path, channel: &str) -> bool {
    read_text(directory.join(format!("{channel}_fault"))).as_deref() == Some("1")
}

fn sorted_entries(path: impl AsRef<Path>) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(path) else {
        return Vec::new();
    };
    let mut paths: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .collect();
    paths.sort();
    paths
}

fn read_text(path: impl AsRef<Path>) -> Option<String> {
    fs::read_to_string(path)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn contains_any(value: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| value.contains(needle))
}
