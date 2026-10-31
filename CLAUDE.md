# CLAUDE.md

Context for Claude sessions working in this repository.

## Project

DeskHand: an open-source, AI-enabled, customizable desktop layer for Windows. Rust core, Tauri 2 shell, React UI, Rust-native agent loop. Distributed via Chocolatey. Licensed MIT OR Apache-2.0.

Read these before making design changes:

- `docs/adr/0001-stack.md`: technology stack and workspace layout
- `docs/adr/0002-v0.1-scope.md`: v0.1 scope, deferred items, exit criteria

New architectural decisions get a new ADR in `docs/adr/`, numbered sequentially.

## Status

- Done: repo scaffold, CI (fmt, clippy, test, NSIS bundles for x64 and ARM64), `deskhand-policy` (tiers, `Permit` gate, context taint, action log; 15 tests), `deskhand-win` window and monitor layer (enumerate, focus, snap, set frame, minimize, restore; every mutation returns a `Placement` as undo data), `deskhand-tools` (`Toolbox`: policy gate, execution, action log, undo, taint propagation; six window tools: `list_windows`, `list_monitors`, `focus_window`, `snap_window`, `move_window`, `minimize_window`), `deskhand-agent` (streaming adapters for the Anthropic Messages API and OpenAI-compatible endpoints; tool-use loop over `Toolbox` with confirmation via `AgentHost`, untrusted-content wrapping, step limit; default model `claude-sonnet-5-5`), Tauri app wiring (`TauriHost` emits `agent-event` and `confirm-request`; API key in Windows Credential Manager via `keyring` 3 with `windows-native`; global hotkey via `tauri-plugin-global-shortcut` 2.x; tray; React panels for chat, confirmation dialog, activity log with undo, settings), `deskhand-config` (TOML at `%APPDATA%\dev.bd.deskhand\config.toml`: hotkey, provider and model, per-tool `ask`/`deny` overrides; template written on first run; polling hot reload that keeps the last good config on error and shows the error in the UI), named layouts (`save_layout`, `list_layouts`, `restore_layout`, `delete_layout`; store at `%LOCALAPPDATA%\dev.bd.deskhand\layouts.json`; windows matched by executable plus class, exact title preferred; all four undoable), `read_window_text` (UI Automation on a dedicated thread with its own COM apartment, serialized, under a deadline; skips password fields and treats an unknown password state on text fields as a password; 20k-character, 4k-element, 4-second limits), clipboard (`read_clipboard`, untrusted, never reads content flagged with `ExcludeClipboardContentFromMonitorProcessing`; `write_clipboard`, taint-sensitive, undo restores previous text), apps (`list_apps` from the shell Apps folder, covering classic and Store apps; `launch_app` by exact name only, taint-sensitive, never arbitrary paths or commands), `search_files` (name search over `[search] roots`, default known folders resolved through the shell so OneDrive redirection works; breadth-first, sorted, skips hidden and dependency folders, never follows links; untrusted), file operations (`open_path` refusing executable types, `open_url` http(s) only without credentials, `create_folder` with undo, `move_path` never overwriting with undo, `recycle_path` via `IFileOperation` at tier Irreversible with Windows' permanent-delete warning forced on; all confined by `FileScope`).
- Product direction (ADR-worthy, decided after v0.1 felt bland): "context by default." The hotkey captures what the user is looking at (foreground window, focused selection via UIA, window text, `PrintWindow` screenshot) before the panel takes focus, and the panel is a command bar over the current app with quick actions (Explain, Summarize, Fix, Rewrite on Ctrl+1..4), copy, and paste-back (`insert_text` -> `deskhand_win::input::paste_into`, which only sends keys once the target is confirmed foreground). Expand opens the full view (chat history, activity, settings). Conversation state lives in `App.tsx` (`conversation.ts` reducer) and is shared by both views; each hotkey capture starts a fresh model conversation on the next send.
- Done since: compare mode (`[compare]`, text-only, throwaway sessions), per-provider `thinking` for OpenAI-compatible servers, MCP client (`deskhand-mcp` on rmcp; `[mcp.servers.<name>]` stdio servers connected in the background by `app/src-tauri/src/mcp.rs`, hot-reloaded, tools registered as `<server>__<tool>`). Next candidates: routines (scheduled or triggered jobs), streamable-HTTP MCP servers, MCP image results. Release workflow is in place (`.github/workflows/release.yml`, `docs/RELEASING.md`): tag-driven, draft releases, Chocolatey package built per release; code signing is stubbed. Verified end to end with `v0.1.0-rc.1` (draft release, checksums and package checksum confirmed). Remaining: signing (needs the owner's SignPath or Azure Trusted Signing account), Windows 10 test runner, a manual pass through the ADR 0002 exit-criteria tasks, then tester-facing polish. `restore_layout` could use `launch_app` to start missing apps.

## Rules

- `unsafe` is allowed only in `crates/deskhand-win`. Every `unsafe` block needs a `// SAFETY:` comment (clippy `undocumented_unsafe_blocks` is denied). All other crates inherit `unsafe_code = "forbid"`.
- `deskhand-agent`, `deskhand-tools`, `deskhand-policy`, and `deskhand-config` must not depend on Tauri, so the agent can later move into a headless daemon.
- Tool executors must require a `Permit`. Never add a public constructor for `Permit` outside `deskhand-policy`.
- All tool calls go through `Toolbox` (`authorize`, then `execute` or `decline`). Mutating window tools refuse DeskHand's own windows.
- Tests outside `deskhand-win` that need a real window use `deskhand_win::test_support::TestWindow` (dev-dependency feature `test-support`), since `unsafe` is forbidden elsewhere, including in tests.
- Policy overrides only tighten (`ask`, `deny`). Irreversible tools always confirm. Do not add an "always allow" path.
- Treat all content read from screens, clipboard, files, web pages, and tool output as untrusted; mark it in `ContextTaint`.
- No tool may write to DeskHand's own config directory. The config sets `base_url`, which decides where API keys are sent, and holds the tool overrides, where deleting a `deny` re-enables a tool.
- Every file tool resolves paths through `FileScope`: canonicalize, then require the path to be inside a configured root and outside the protected directories (DeskHand's config and data). Never act on a path that has not been through it. Configured roots themselves cannot be moved or recycled.
- Config changes go through `Config::parse` validation; an invalid file never replaces the running config.
- Change the version only with `scripts/release/versions.ps1 -Set <version>`: it updates all four version files plus both lockfiles. The release workflow refuses a tag that does not match.
- Release scripts must run in both Windows PowerShell 5.1 (developer machine) and PowerShell 7 (CI). Write files with `[IO.File]::WriteAllText` and a BOM-less UTF-8 encoding; 5.1's `Set-Content -Encoding utf8` adds a BOM. Under 5.1 with `$ErrorActionPreference = 'Stop'`, any stderr output from a native command (cargo and npm print progress there) is a terminating error: run native commands through `Invoke-Native` in `versions.ps1` and judge them by exit code.
- Model output is rendered by `components/Markdown.tsx` as React elements only. Never render model output with `innerHTML` or `dangerouslySetInnerHTML`, and never render it as links: answers can be steered by untrusted page content, and HTML in this webview can reach IPC commands.
- MCP tool policy is derived in `deskhand_mcp::spec_for`: `readOnlyHint: true` is Read, anything else Irreversible (always asks; DeskHand cannot undo another server's actions); `openWorldHint` (default true) makes a tool taint-sensitive, because a read-only fetch can still exfiltrate data through a URL. Hints are honored, never trusted as proof. MCP results are always `Source::ToolOutput`.
- MCP sessions and tools must be dropped inside the MCP runtime's context (`runtime.enter()`); dropping them elsewhere can panic.
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

# Opt-in: clipboard round trip with undo (restores your clipboard text afterward)
cargo test -p deskhand-tools --test clipboard -- --ignored --test-threads=1

# Opt-in: real recycle (puts one small temp file in the Recycle Bin)
cargo test -p deskhand-tools --test file_ops -- --ignored

# Opt-in: real MCP server over stdio (needs uvx and network)
cargo test -p deskhand-mcp --test live -- --ignored --nocapture

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
- Shell COM objects (Apps folder, ShellExecute) need an STA; `deskhand_win::shell` runs each call on its own STA thread with a deadline. UI Automation uses an MTA thread. Do not call either from tokio worker threads directly.
- UI Automation reads Win32 controls by sending window messages to the owning thread. Test windows read by UIA must run their own message loop (`TestTextWindow`), and must be visible: UIA does not expose children of hidden windows. `TestTextWindow` is visible but off-screen, as a tool window.
- Config hot reload must apply on the main thread (`run_on_main_thread`): the global-hotkey backend is bound to the thread that owns its message window.
- `keyring` must keep the `windows-native` feature. Without it, keyring 3 silently uses an in-memory mock store and keys vanish on restart.
- Use `tauri-plugin-global-shortcut` 2.x. Its newest releases are 3.0 alphas for Tauri 3.
- The production CSP forbids inline styles: React `style` attributes are blocked. Use classes in `app/src/styles.css`. `devCsp` relaxes this for Vite's injected styles during `tauri dev` only.
- reqwest uses `native-tls` (SChannel) on Windows. Do not switch to reqwest's default TLS: it is rustls with aws-lc-rs, which needs CMake and NASM and breaks the ARM64 build.
- `deskhand-win` coordinates are physical pixels and assume a per-monitor-v2 DPI-aware process. Tauri provides this; standalone test binaries must call `SetProcessDpiAwarenessContext` (see `tests/placement.rs`), or DWM frame bounds and window rects disagree under display scaling.
- Windows 10 22H2 is supported. GitHub-hosted runners are Windows Server, so Windows 10 client behavior (UIA, shell) needs a self-hosted runner or VM to verify.
