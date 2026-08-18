# Unetic Core

OpenWrt control-plane daemon for Unetic.

Repository: <https://github.com/Unetic/unetic-core>

## Development

```sh
nix develop
cargo fmt --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-targets --all-features
cargo build --locked --release --all-features
```

Normal pushes and pull requests run only these CI checks. They do not create APKs or GitHub Releases.

## Release

A semantic tag `vX.Y.Z` must match the `Cargo.toml` package version. The release workflow runs CI first and, only after it passes, cross-builds the Rust binary for every OpenWrt target declared by `Unetic/packages/config/targets.json`.

The component GitHub Release contains target-specific binaries and `SHA256SUMS`. It does **not** build the final APK.

Production APK construction and signing are owned by `Unetic/packages`. A packages release with the same tag downloads the released core binary, compiles the small OpenWrt C bridge against the selected SDK, wraps both into an APK, signs the repository and publishes it.

Do not add Cargo compilation to the `Unetic/packages` APK Makefile.
