# Unetic Core

The control plane behind [Unetic](https://github.com/Unetic).

`unetic-core` is a lightweight Rust daemon for OpenWrt. It owns Unetic's domain logic,
maintains desired state and exposes the management API through OpenWrt's `ubus`.

## Responsibilities

- transactional configuration apply and rollback;
- desired-state reconciliation and drift repair;
- maintenance mode;
- structured operation and error state;
- ubus API and state notifications;
- integration with OpenWrt configuration and runtime services.

OpenWrt remains responsible for the networking stack itself: netifd, hostapd,
dnsmasq, firewall4, nftables and the kernel.

```text
Unetic Web / CLI
       │
       ▼
      ubus
       │
       ▼
  unetic-core
       │
       ├── UCI / rpcd
       ├── netifd
       └── wireless
```

## Development

With Nix:

```sh
nix develop
```

Checks:

```sh
cargo fmt --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-targets --all-features
```

## OpenWrt

OpenWrt runtime assets used by the distribution recipe live under `packaging/openwrt/`.

Production APKs are built and signed centrally by
[`Unetic/packages`](https://github.com/Unetic/packages).

For local development, an unsigned APK can be installed manually:

```sh
scp unetic-core-*.apk root@router:/tmp/
ssh root@router 'apk --allow-untrusted add /tmp/unetic-core-*.apk'
```

Use `--allow-untrusted` only for local development artifacts.

## License

GPL-2.0-only.
