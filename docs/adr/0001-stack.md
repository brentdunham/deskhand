# ADR 0001: v0.1 Technology Stack

- Status: Accepted
- Date: 2026-10-29

## Context

DeskHand is an open-source, AI-enabled overlay for the Windows desktop, distributed via Chocolatey. Constraints:

- No project-operated backend. Users bring their own model keys or run local models.
- Windows-first. Overlay on top of Explorer for v0.1; shell-replacement mode is a later option and must not require a rewrite.
- Every agent action passes through a permission policy and is recorded in a user-reviewable action log.
- Release binaries must be code-signed.

## Decision

| Layer | Choice |
|---|---|
| Core | Rust, `windows` crate, Tokio |
| UI | Tauri 2 (WebView2) |
| Frontend | TypeScript + React (Vite) |
| Agent loop | Rust-native, in-process |
| Model providers | Anthropic Messages API adapter; OpenAI-compatible adapter (OpenAI, OpenRouter, Ollama, LM Studio, llama.cpp server) |
| Tools and extensibility | MCP via `rmcp`: DeskHand tools exposed as an MCP server; external MCP servers consumed as plugins |
| Storage | SQLite (`rusqlite`): action log, memory, routines |
| Config | TOML: layout, hotkeys, routines |
| Secrets | Windows Credential Manager via `keyring` |
| Packaging | Tauri NSIS installer (per-machine), wrapped by a Chocolatey package; winget and Scoop later from the same installer |
| App identifier | `dev.bd.deskhand` |
| Signing | SignPath Foundation or Azure Trusted Signing |
| CI | GitHub Actions on windows-latest; x64 and ARM64 targets |
| Supported OS | Windows 11; Windows 10 22H2 |
| License | MIT OR Apache-2.0 (dual) |

### Process model

Single process for v0.1. The Tauri app hosts the UI, agent loop, tools, and policy engine on a Tokio runtime. Agent, tool, and policy crates do not depend on Tauri, so a headless daemon can be split out later for shell-replacement mode.

### Workspace layout

```
deskhand/
  crates/
    deskhand-win      Win32, UIA, window management, hotkeys (all unsafe code)
    deskhand-policy   permission tiers, action log, untrusted-content tagging
    deskhand-agent    provider adapters, tool-use loop
    deskhand-tools    tool definitions, MCP server and client
    deskhand-config   TOML schema, validation, hot reload
  app/
    src/              React UI
    src-tauri/        Tauri application crate
  packaging/choco/
  docs/adr/
```

## Alternatives Considered

- **WinUI 3 instead of Tauri 2.** More native look, but Rust support is poor, forcing a C# UI and a two-language core.
- **Python sidecar (LangGraph) instead of a Rust-native agent.** Would allow reuse of existing Python agent code, but adds a bundled Python runtime, antivirus false positives on frozen executables, and slower cold start. The required orchestration is a tool-use loop plus a policy gate, which does not need a graph framework.
- **Svelte or Solid instead of React.** Lighter runtime; React chosen for contributor familiarity.
- **GPL-3.0 instead of MIT/Apache-2.0.** Would force forks to stay open, but limits embedding and reuse of the crates.
- **Windows 11 only.** Simpler testing and guaranteed WebView2, but excludes a large Windows 10 install base.
- **MSI instead of NSIS.** Tauri's MSI (WiX) path does not support ARM64 targets.

## Consequences

- One language (Rust) for all native, agent, and tool code; TypeScript only in the UI.
- `unsafe_code = "forbid"` applies to every crate except `deskhand-win`, where each `unsafe` block requires a `SAFETY` comment.
- No bundled local models. DeskHand detects a running Ollama or LM Studio endpoint instead.
- Tauri's built-in updater is disabled. Chocolatey (and later winget and Scoop) owns upgrades.
- No telemetry by default.
- The per-machine installer requires elevation. This matches how Chocolatey installs; direct-download users see a UAC prompt.
- The app identifier determines the per-user data directory. Changing it after testers install requires a data migration.
- Windows 10 22H2 support requires the installer to bootstrap the WebView2 Evergreen Runtime when it is absent, and CI must smoke-test on both Windows 10 and Windows 11. Windows 10 is past end of support, so its continued support is reviewed at each minor release.
- Dual licensing follows the Rust ecosystem convention: `license = "MIT OR Apache-2.0"` in every crate, `LICENSE-MIT` and `LICENSE-APACHE` at the repo root, and inbound contributions accepted under the same dual terms.
