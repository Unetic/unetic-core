# Unetic Core

The OpenWrt control-plane daemon for Unetic. It will own domain logic and expose
it to local clients over ubus.

Repository: <https://github.com/Unetic/unetic-core>

## Development

```sh
nix develop
cargo test
cargo clippy --all-targets --all-features -- -D warnings
```

## OpenWrt package

CI smoke-tests `unetic-core` with the pinned OpenWrt 25.12.5 x86/64 SDK. Builds
for a router must select its actual OpenWrt SDK target. The version comes from
`Cargo.toml`; `openwrt/Makefile` only carries the packaging revision. Tagged
releases use semantic version tags such as `v0.1.0` and attach the APK.

Install a downloaded development artifact with:

```sh
scp unetic-core-*.apk root@router:/tmp/
ssh root@router 'apk --allow-untrusted add /tmp/unetic-core-*.apk && rm -f /tmp/unetic-core-*.apk'
```

`--allow-untrusted` is only for local artifacts. A future public feed will be
signed and installed through the normal `apk update && apk add unetic-core`
path.
