use crate::domain::{WanDesired, WanProtocol};

pub(crate) fn normalize_wan_desired(mut wan: WanDesired) -> WanDesired {
    if !wan.present || wan.proto == WanProtocol::None {
        return WanDesired {
            present: false,
            proto: WanProtocol::None,
            ..WanDesired::default()
        };
    }

    match wan.proto {
        WanProtocol::Dhcp | WanProtocol::Extender => {
            wan.static_config = None;
            wan.pppoe_config = None;
        }
        WanProtocol::Static => {
            wan.custom_dns = wan
                .static_config
                .as_ref()
                .map_or_else(Vec::new, |config| config.dns.clone());
            wan.pppoe_config = None;
        }
        WanProtocol::Pppoe => {
            wan.static_config = None;
        }
        WanProtocol::None => unreachable!("disabled WAN returned above"),
    }

    wan
}

pub(crate) fn wan_config_matches(observed: &WanDesired, expected: &WanDesired) -> bool {
    let mut observed = normalize_wan_desired(observed.clone());
    let expected = normalize_wan_desired(expected.clone());

    if expected.proto == WanProtocol::Extender && observed.proto == WanProtocol::Dhcp {
        observed.proto = WanProtocol::Extender;
    }

    observed == expected
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_wan_drops_stale_protocol_settings() {
        let normalized = normalize_wan_desired(WanDesired {
            present: false,
            device: Some("eth1".into()),
            proto: WanProtocol::Dhcp,
            custom_dns: vec!["1.1.1.1".into()],
            ..WanDesired::default()
        });

        assert_eq!(
            normalized,
            WanDesired {
                present: false,
                proto: WanProtocol::None,
                ..WanDesired::default()
            }
        );
    }

    #[test]
    fn extender_matches_its_dhcp_uci_representation() {
        let extender = WanDesired {
            present: true,
            proto: WanProtocol::Extender,
            ..WanDesired::default()
        };
        let dhcp = WanDesired {
            present: true,
            proto: WanProtocol::Dhcp,
            ..WanDesired::default()
        };

        assert!(wan_config_matches(&dhcp, &extender));
    }
}
