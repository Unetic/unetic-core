use crate::application::app::App;
use futures_util::stream::StreamExt;
use netlink_packet_core::NetlinkPayload;
use netlink_packet_route::AddressFamily;
use netlink_packet_route::RouteNetlinkMessage;
use netlink_packet_route::neighbour::{NeighbourAddress, NeighbourAttribute};
use netlink_sys::{AsyncSocket, SocketAddr};
use std::sync::Arc;
use tokio::task;

pub fn start_neighbor_listener(app: Arc<App>) {
    task::spawn(async move {
        let (mut connection, _handle, mut messages) = match rtnetlink::new_connection() {
            Ok(c) => c,
            Err(_) => return,
        };

        // Subscribe to both IPv4 and IPv6 neighbor events.
        let addr = SocketAddr::new(0, libc::RTMGRP_NEIGH as u32);
        if connection.socket_mut().socket_mut().bind(&addr).is_err() {
            return;
        }

        tokio::spawn(connection);

        while let Some((message, _)) = messages.next().await {
            if let NetlinkPayload::InnerMessage(RouteNetlinkMessage::NewNeighbour(msg)) =
                message.payload
            {
                let is_ipv4 = msg.header.family == AddressFamily::Inet;
                let is_ipv6 = msg.header.family == AddressFamily::Inet6;
                if !is_ipv4 && !is_ipv6 {
                    continue;
                }

                let mut mac_opt: Option<String> = None;
                let mut ip_opt: Option<String> = None;
                let mut ip6_opt: Option<String> = None;

                for attr in msg.attributes {
                    match attr {
                        NeighbourAttribute::LinkLayerAddress(mac) => {
                            let s = mac
                                .iter()
                                .map(|b| format!("{b:02x}"))
                                .collect::<Vec<_>>()
                                .join(":");
                            mac_opt = Some(s);
                        }
                        NeighbourAttribute::Destination(NeighbourAddress::Inet(ip)) => {
                            ip_opt = Some(ip.to_string());
                        }
                        // Link-local addresses are not useful as forwarding destinations.
                        NeighbourAttribute::Destination(NeighbourAddress::Inet6(ip))
                            if !ip.is_unicast_link_local() =>
                        {
                            ip6_opt = Some(ip.to_string());
                        }
                        _ => {}
                    }
                }

                if let Some(mac) = mac_opt {
                    if ip_opt.is_some() || ip6_opt.is_some() {
                        let app = Arc::clone(&app);
                        tokio::task::spawn_blocking(move || {
                            app.devices_sync_ip(&mac, ip_opt.as_deref(), ip6_opt.as_deref());
                        });
                    }
                }
            }
        }
    });
}
