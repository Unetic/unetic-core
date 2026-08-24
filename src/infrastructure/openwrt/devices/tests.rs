use super::*;
use std::collections::HashMap;

#[test]
fn test_parse_dhcp_and_arp() {
    let dhcp_raw = "1724278900 00:11:22:33:44:55 192.168.1.100 phone 01:00\n1724279000 AA:BB:CC:DD:EE:FF 192.168.1.101 * *";
    let arp_raw = "IP address HW type Flags HW address Mask Device\n192.168.1.100 0x1 0x2 00:11:22:33:44:55 * br-lan\n192.168.1.102 0x1 0x0 00:00:00:00:00:00 * br-lan";

    let leases = parse_dhcp_leases(dhcp_raw);
    assert_eq!(leases.len(), 2);
    assert_eq!(leases["00:11:22:33:44:55"].hostname, Some("phone".into()));
    assert_eq!(leases["aa:bb:cc:dd:ee:ff"].hostname, None);

    let arp = parse_arp_table(arp_raw);
    assert_eq!(arp.len(), 1);
    assert_eq!(arp["00:11:22:33:44:55"].ip, "192.168.1.100");
}

#[test]
fn test_merge_and_sort() {
    let dhcp = parse_dhcp_leases(
        "1724278900 00:11:22:33:44:55 192.168.1.100 Alice-Phone *\n1724279000 aa:bb:cc:dd:ee:ff 192.168.1.101 Desktop *",
    );
    let arp = parse_arp_table(
        "192.168.1.100 0x1 0x2 00:11:22:33:44:55 * br-lan\n192.168.1.150 0x1 0x2 11:22:33:44:55:66 * br-lan",
    );
    let mut wireless = HashMap::new();
    wireless.insert(
        "00:11:22:33:44:55".into(),
        WirelessClient {
            interface: "wlan0".into(),
            network: Some("Home".into()),
            signal_dbm: -60,
        },
    );
    let mut mac_to_iface = HashMap::new();
    mac_to_iface.insert("aa:bb:cc:dd:ee:ff".into(), "lan1".into());
    let mut ip6_by_mac = HashMap::new();
    ip6_by_mac.insert("00:11:22:33:44:55".into(), "2001:db8::1".into());

    let extenders: Vec<crate::domain::extender::KnownExtender> = Vec::new();
    let extender_clients = HashMap::new();
    let devices = merge_devices(
        dhcp,
        arp,
        wireless,
        mac_to_iface,
        ip6_by_mac,
        &extenders,
        &extender_clients,
    );
    assert_eq!(devices.len(), 3);
    assert_eq!(devices[0].mac, "00:11:22:33:44:55");
    assert_eq!(devices[0].ip, Some("192.168.1.100".into()));
    assert_eq!(devices[0].ip6, Some("2001:db8::1".into()));
    assert_eq!(
        devices[0].connection,
        crate::domain::device::DeviceConnection::Wireless {
            signal_dbm: -60,
            interface: "wlan0".into(),
            network: Some("Home".into()),
        }
    );
    assert_eq!(devices[1].mac, "aa:bb:cc:dd:ee:ff");
    assert_eq!(
        devices[1].connection,
        crate::domain::device::DeviceConnection::Wired {
            port_id: "lan1".into(),
        }
    );
    assert_eq!(devices[2].mac, "11:22:33:44:55:66");
    assert_eq!(devices[2].hostname, None);
}

#[test]
fn test_dhcp_lease_does_not_make_an_absent_device_online() {
    let devices = merge_devices(
        parse_dhcp_leases("1724278900 aa:bb:cc:dd:ee:ff 192.168.1.101 Desktop *"),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        &[],
        &HashMap::new(),
    );

    assert!(devices.is_empty());
}

#[test]
fn test_extender_telemetry_identifies_client_without_master_fdb_entry() {
    let mut extender_clients = HashMap::new();
    extender_clients.insert(
        "00:11:22:33:44:55".into(),
        vec![crate::domain::extender::ExtenderClient {
            mac: "aa:bb:cc:dd:ee:ff".into(),
            signal_dbm: Some(-56),
            interface: Some("wlan1".into()),
            network: Some("Home".into()),
            port_id: None,
        }],
    );
    let extenders = vec![crate::domain::extender::KnownExtender {
        mac: "00:11:22:33:44:55".into(),
        ip: "192.168.1.2".into(),
        model: "Extender".into(),
        auth_token: "secret".into(),
    }];

    let devices = merge_devices(
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        &extenders,
        &extender_clients,
    );

    assert_eq!(devices.len(), 1);
    assert_eq!(
        devices[0].connection,
        crate::domain::device::DeviceConnection::ViaExtender {
            extender_mac: "00:11:22:33:44:55".into(),
            signal_dbm: Some(-56),
            interface: Some("wlan1".into()),
            network: Some("Home".into()),
            port_id: None,
        }
    );
}
