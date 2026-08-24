use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::device::Device;

pub const DEVICE_HISTORY_RETENTION_MS: u64 = 30 * 24 * 60 * 60 * 1_000;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DeviceRuntime {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub registered: bool,
    #[serde(flatten)]
    pub device: Device,
    pub online: bool,
    pub last_seen_ms: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct DeviceInventory {
    #[serde(default)]
    devices: BTreeMap<String, DeviceRuntime>,
}

impl DeviceInventory {
    pub fn replace_snapshot(
        &mut self,
        devices: Vec<Device>,
        registered_macs: &[String],
        now_ms: u64,
    ) -> bool {
        let registered_macs = registered_macs
            .iter()
            .map(|mac| mac.to_ascii_lowercase())
            .collect::<BTreeSet<_>>();
        let mut changed = false;
        let mut observed = BTreeMap::new();

        for runtime in self.devices.values_mut() {
            if runtime.id.is_empty() {
                runtime.id = DeviceRuntime::id_for_mac(&runtime.device.mac);
                changed = true;
            }
        }

        for mut device in devices {
            device.mac = device.mac.to_ascii_lowercase();
            let runtime = match self.devices.get(&device.mac) {
                Some(existing) if existing.online && existing.device == device => existing.clone(),
                _ => DeviceRuntime {
                    id: DeviceRuntime::id_for_mac(&device.mac),
                    registered: false,
                    device,
                    online: true,
                    last_seen_ms: now_ms,
                },
            };
            observed.insert(runtime.device.mac.clone(), runtime);
        }

        for (mac, runtime) in &observed {
            if self.devices.get(mac) != Some(runtime) {
                changed = true;
            }
        }

        for (mac, existing) in &mut self.devices {
            if observed.contains_key(mac) || !existing.online {
                continue;
            }
            existing.online = false;
            changed = true;
        }

        self.devices.extend(observed);

        for mac in &registered_macs {
            if self.devices.contains_key(mac) {
                continue;
            }
            self.devices.insert(
                mac.clone(),
                DeviceRuntime {
                    id: DeviceRuntime::id_for_mac(mac),
                    registered: true,
                    device: Device {
                        mac: mac.clone(),
                        ip: None,
                        ip6: None,
                        hostname: None,
                        connection: super::device::DeviceConnection::Unknown,
                    },
                    online: false,
                    last_seen_ms: 0,
                },
            );
            changed = true;
        }

        for runtime in self.devices.values_mut() {
            let registered = registered_macs.contains(&runtime.device.mac);
            if runtime.registered != registered {
                runtime.registered = registered;
                changed = true;
            }
        }

        self.prune(now_ms) || changed
    }

    pub fn devices(&self) -> Vec<DeviceRuntime> {
        self.devices.values().cloned().collect()
    }

    fn prune(&mut self, now_ms: u64) -> bool {
        let before = self.devices.len();
        self.devices.retain(|_, runtime| {
            runtime.registered || runtime.online
                || now_ms.saturating_sub(runtime.last_seen_ms) <= DEVICE_HISTORY_RETENTION_MS
        });
        self.devices.len() != before
    }
}

impl DeviceRuntime {
    #[must_use]
    pub fn id_for_mac(mac: &str) -> String {
        format!("device-{}", mac.to_ascii_lowercase().replace(':', ""))
    }

    pub fn mac_from_id(id: &str) -> Option<String> {
        let suffix = id.strip_prefix("device-")?;
        if suffix.len() != 12 || !suffix.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return None;
        }
        Some(format!(
            "{}:{}:{}:{}:{}:{}",
            &suffix[0..2],
            &suffix[2..4],
            &suffix[4..6],
            &suffix[6..8],
            &suffix[8..10],
            &suffix[10..12],
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::device::{Device, DeviceConnection};

    fn device(mac: &str) -> Device {
        Device::new(
            mac,
            Some("192.168.1.2".into()),
            None,
            Some("phone".into()),
            DeviceConnection::Wireless {
                signal_dbm: -55,
                interface: "wlan0".into(),
                network: Some("Home".into()),
            },
        )
    }

    #[test]
    fn preserves_offline_devices_until_retention_expires() {
        let mut inventory = DeviceInventory::default();
        assert!(inventory.replace_snapshot(vec![device("AA:BB:CC:DD:EE:FF")], &[], 10));
        assert!(inventory.replace_snapshot(Vec::new(), &[], 20));

        let devices = inventory.devices();
        assert_eq!(devices.len(), 1);
        assert!(!devices[0].online);
        assert!(!devices[0].registered);
        assert_eq!(devices[0].last_seen_ms, 10);
        assert_eq!(devices[0].id, "device-aabbccddeeff");

        assert!(inventory.replace_snapshot(
            Vec::new(),
            &[],
            10 + DEVICE_HISTORY_RETENTION_MS + 1,
        ));
        assert!(inventory.devices().is_empty());
    }

    #[test]
    fn does_not_change_a_steady_online_snapshot() {
        let mut inventory = DeviceInventory::default();
        assert!(inventory.replace_snapshot(vec![device("AA:BB:CC:DD:EE:FF")], &[], 10));
        assert!(!inventory.replace_snapshot(
            vec![device("AA:BB:CC:DD:EE:FF")],
            &[],
            20,
        ));
        assert_eq!(inventory.devices()[0].last_seen_ms, 10);
    }

    #[test]
    fn preserves_a_registered_device_that_has_never_been_seen() {
        let mut inventory = DeviceInventory::default();
        let registered = vec!["AA:BB:CC:DD:EE:FF".into()];
        assert!(inventory.replace_snapshot(Vec::new(), &registered, 10));

        let device = inventory.devices().pop().expect("registered device");
        assert!(!device.online);
        assert!(device.registered);
        assert_eq!(device.last_seen_ms, 0);
        assert_eq!(device.id, "device-aabbccddeeff");
    }

    #[test]
    fn keeps_the_same_id_after_a_device_reappears() {
        let mut inventory = DeviceInventory::default();
        inventory.replace_snapshot(vec![device("AA:BB:CC:DD:EE:FF")], &[], 10);
        inventory.replace_snapshot(Vec::new(), &[], 20);
        inventory.replace_snapshot(vec![device("AA:BB:CC:DD:EE:FF")], &[], 30);

        assert_eq!(inventory.devices()[0].id, "device-aabbccddeeff");
    }

    #[test]
    fn resolves_a_device_id_back_to_its_mac() {
        assert_eq!(
            DeviceRuntime::mac_from_id("device-aabbccddeeff"),
            Some("aa:bb:cc:dd:ee:ff".into())
        );
        assert_eq!(DeviceRuntime::mac_from_id("device-not-a-mac"), None);
    }
}
