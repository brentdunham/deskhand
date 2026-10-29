# DeskHand

An open-source, AI-enabled, customizable desktop layer for Windows. DeskHand runs an agent you control on top of your desktop: bring your own model key or use a local model, and every action the agent takes is permission-gated and logged.

**Status:** pre-alpha. Not ready for use.

## Requirements

- Windows 11, or Windows 10 22H2
- [Rust](https://rustup.rs) via rustup (the toolchain in `rust-toolchain.toml` installs automatically)
- Node.js LTS
- Visual Studio 2022 Build Tools with the "Desktop development with C++" workload. For ARM64 builds, also install the MSVC ARM64 build tools component.
- WebView2 Runtime (preinstalled on Windows 11)

## Development

```powershell
cd app
npm install
npm run tauri dev
```

Checks run in CI:

```powershell
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Build an installer:

```powershell
cd app
npm run tauri build -- --bundles nsis
```

## Layout

| Path | Contents |
|---|---|
| `crates/deskhand-win` | Win32, UI Automation, window management, hotkeys. The only crate permitted to use `unsafe`. |
| `crates/deskhand-policy` | Permission tiers, action log, untrusted-content tagging |
| `crates/deskhand-agent` | Model provider adapters and the tool-use loop |
| `crates/deskhand-tools` | Tool definitions, MCP server and client |
| `crates/deskhand-config` | TOML configuration schema, validation, hot reload |
| `app/src` | React UI |
| `app/src-tauri` | Tauri application crate |
| `packaging/choco` | Chocolatey package |
| `docs/adr` | Architecture decision records |

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in this project by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without any additional terms or conditions.
