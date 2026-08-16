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
