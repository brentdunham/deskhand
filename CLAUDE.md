# CLAUDE.md

Context for Claude sessions working in this repository.

## Project

DeskHand: an open-source, AI-enabled, customizable desktop layer for Windows. Rust core, Tauri 2 shell, React UI, Rust-native agent loop. Distributed via Chocolatey. Licensed MIT OR Apache-2.0.

Read these before making design changes:

- `docs/adr/0001-stack.md`: technology stack and workspace layout
- `docs/adr/0002-v0.1-scope.md`: v0.1 scope, deferred items, exit criteria

New architectural decisions get a new ADR in `docs/adr/`, numbered sequentially.

## Status

- Done: repo scaffold, CI (fmt, clippy, test, NSIS bundles for x64 and ARM64), `deskhand-policy` (tiers, `Permit` gate, context taint, action log; 15 tests), `deskhand-win` window and monitor layer (enumerate, focus, snap, set frame, minimize, restore; every mutation returns a `Placement` as undo data), `deskhand-tools` (`Toolbox`: policy gate, execution, action log, undo, taint propagation; six window tools: `list_windows`, `list_monitors`, `focus_window`, `snap_window`, `move_window`, `minimize_window`), `deskhand-agent` (streaming adapters for the Anthropic Messages API and OpenAI-compatible endpoints; tool-use loop over `Toolbox` with confirmation via `AgentHost`, untrusted-content wrapping, step limit; default model `claude-sonnet-5-5`), Tauri app wiring (`TauriHost` emits `agent-event` and `confirm-request`; API key in Windows Credential Manager via `keyring` 3 with `windows-native`; global hotkey via `tauri-plugin-global-shortcut` 2.x; tray; React panels for chat, confirmation dialog, activity log with undo, settings), `deskhand-config` (TOML at `%APPDATA%\dev.bd.deskhand\config.toml`: hotkey, provider and model, per-tool `ask`/`deny` overrides; template written on first run; polling hot reload that keeps the last good config on error and shows the error in the UI), named layouts (`save_layout`, `list_layouts`, `restore_layout`, `delete_layout`; store at `%LOCALAPPDATA%\dev.bd.deskhand\layouts.json`; windows matched by executable plus class, exact title preferred; all four undoable).
- Next: remaining v0.1 tools (UIA window text, clipboard, file search with configurable roots, app launch). App launch could later let `restore_layout` start missing apps.
- Then: code signing, release workflow (Chocolatey package with URL and checksum filled in), Windows 10 test runner.

## Rules

- `unsafe` is allowed only in `crates/deskhand-win`. Every `unsafe` block needs a `// SAFETY:` comment (clippy `undocumented_unsafe_blocks` is denied). All other crates inherit `unsafe_code = "forbid"`.
- `deskhand-agent`, `deskhand-tools`, `deskhand-policy`, and `deskhand-config` must not depend on Tauri, so the agent can later move into a headless daemon.
- Tool executors must require a `Permit`. Never add a public constructor for `Permit` outside `deskhand-policy`.
- All tool calls go through `Toolbox` (`authorize`, then `execute` or `decline`). Mutating window tools refuse DeskHand's own windows.
- Tests outside `deskhand-win` that need a real window use `deskhand_win::test_support::TestWindow` (dev-dependency feature `test-support`), since `unsafe` is forbidden elsewhere, including in tests.
- Policy overrides only tighten (`ask`, `deny`). Irreversible tools always confirm. Do not add an "always allow" path.
- Treat all content read from screens, clipboard, files, web pages, and tool output as untrusted; mark it in `ContextTaint`.
- No tool may write to DeskHand's own config directory. The config sets `base_url`, which decides where API keys are sent, and holds the tool overrides, where deleting a `deny` re-enables a tool.
- Config changes go through `Config::parse` validation; an invalid file never replaces the running config.
- No telemetry.

## Commands

Run from the repo root unless noted.

```powershell
# The frontend must be built once before cargo commands: tauri-build requires app/dist.
cd app; npm run build; cd ..

cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

# Opt-in: creates and moves a real window to verify placement round-trips
cargo test -p deskhand-win --test placement -- --ignored

# Opt-in: live model round trip with real window tools (read-only)
$env:ANTHROPIC_API_KEY = "..."          # and/or $env:DESKHAND_OLLAMA_MODEL = "qwen3"
cargo test -p deskhand-agent --test live -- --ignored --nocapture

# Dev app
cd app; npm run tauri dev

# Installer (output: target\release\bundle\nsis\)
cd app; npm run tauri build -- --bundles nsis
```

## Gotchas

- `tauri build` fails if the `tauri` crate and `@tauri-apps/api` differ in minor version. Upgrade `tauri`, `tauri-build`, `@tauri-apps/api`, and `@tauri-apps/cli` together in one commit.
- Build output goes to the workspace-root `target\`, not `app\src-tauri\target\`.
- App identifier is `dev.bd.deskhand`. It sets the app data directory path; changing it breaks existing installs.
- App-written data (layouts) lives in `%LOCALAPPDATA%\dev.bd.deskhand`, never in the config directory. Stores write through with temp-file-then-rename, and an unreadable store is moved aside to `*.unreadable`, never overwritten.
- Config hot reload must apply on the main thread (`run_on_main_thread`): the global-hotkey backend is bound to the thread that owns its message window.
- `keyring` must keep the `windows-native` feature. Without it, keyring 3 silently uses an in-memory mock store and keys vanish on restart.
- Use `tauri-plugin-global-shortcut` 2.x. Its newest releases are 3.0 alphas for Tauri 3.
- The production CSP forbids inline styles: React `style` attributes are blocked. Use classes in `app/src/styles.css`. `devCsp` relaxes this for Vite's injected styles during `tauri dev` only.
- reqwest uses `native-tls` (SChannel) on Windows. Do not switch to reqwest's default TLS: it is rustls with aws-lc-rs, which needs CMake and NASM and breaks the ARM64 build.
- `deskhand-win` coordinates are physical pixels and assume a per-monitor-v2 DPI-aware process. Tauri provides this; standalone test binaries must call `SetProcessDpiAwarenessContext` (see `tests/placement.rs`), or DWM frame bounds and window rects disagree under display scaling.
- Windows 10 22H2 is supported. GitHub-hosted runners are Windows Server, so Windows 10 client behavior (UIA, shell) needs a self-hosted runner or VM to verify.
