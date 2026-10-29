# CLAUDE.md

Context for Claude sessions working in this repository.

## Project

DeskHand: an open-source, AI-enabled, customizable desktop layer for Windows. Rust core, Tauri 2 shell, React UI, Rust-native agent loop. Distributed via Chocolatey. Licensed MIT OR Apache-2.0.

Read these before making design changes:

- `docs/adr/0001-stack.md`: technology stack and workspace layout
- `docs/adr/0002-v0.1-scope.md`: v0.1 scope, deferred items, exit criteria

New architectural decisions get a new ADR in `docs/adr/`, numbered sequentially.

## Status

- Done: repo scaffold, CI (fmt, clippy, test, NSIS bundles for x64 and ARM64), `deskhand-policy` (tiers, `Permit` gate, context taint, action log; 15 tests).
- Next: window tools in `deskhand-win`. Enumerate windows and monitors; focus, move, resize, snap, and minimize, each returning the undo data needed to restore the prior state.
- Then: `deskhand-tools` (tool definitions; executors take a `deskhand_policy::Permit` by value), `deskhand-agent` (provider adapters, tool-use loop), UI panels.

## Rules

- `unsafe` is allowed only in `crates/deskhand-win`. Every `unsafe` block needs a `// SAFETY:` comment (clippy `undocumented_unsafe_blocks` is denied). All other crates inherit `unsafe_code = "forbid"`.
- `deskhand-agent`, `deskhand-tools`, `deskhand-policy`, and `deskhand-config` must not depend on Tauri, so the agent can later move into a headless daemon.
- Tool executors must require a `Permit`. Never add a public constructor for `Permit` outside `deskhand-policy`.
- Policy overrides only tighten (`ask`, `deny`). Irreversible tools always confirm. Do not add an "always allow" path.
- Treat all content read from screens, clipboard, files, web pages, and tool output as untrusted; mark it in `ContextTaint`.
- No telemetry.

## Commands

Run from the repo root unless noted.

```powershell
# The frontend must be built once before cargo commands: tauri-build requires app/dist.
cd app; npm run build; cd ..

cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

# Dev app
cd app; npm run tauri dev

# Installer (output: target\release\bundle\nsis\)
cd app; npm run tauri build -- --bundles nsis
```

## Gotchas

- `tauri build` fails if the `tauri` crate and `@tauri-apps/api` differ in minor version. Upgrade `tauri`, `tauri-build`, `@tauri-apps/api`, and `@tauri-apps/cli` together in one commit.
- Build output goes to the workspace-root `target\`, not `app\src-tauri\target\`.
- App identifier is `dev.bd.deskhand`. It sets the app data directory path; changing it breaks existing installs.
- Windows 10 22H2 is supported. GitHub-hosted runners are Windows Server, so Windows 10 client behavior (UIA, shell) needs a self-hosted runner or VM to verify.
