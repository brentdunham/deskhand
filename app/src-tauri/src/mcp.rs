//! MCP servers: connects configured servers in the background, registers
//! their tools in the toolbox, and tracks per-server status for the UI.

use std::collections::BTreeMap;
use std::sync::Arc;

use deskhand_config::{McpConfig, McpServerConfig};
use deskhand_mcp::{McpTool, Session, StdioServer, connect_stdio, tools};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::{AppState, CONFIG_EVENT, lock};

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ServerStatus {
    Disabled,
    Connecting,
    Ready {
        tools: usize,
        /// Tools whose names could not be mapped or collided.
        skipped: Vec<String>,
    },
    Failed {
        error: String,
    },
}

struct Slot {
    config: McpServerConfig,
    status: ServerStatus,
    /// Held so the server stays connected; tools hold clones.
    _session: Option<Arc<Session>>,
    /// Matches the connection attempt that may fill this slot.
    generation: u64,
}

pub struct McpState {
    /// Dedicated runtime for MCP I/O. `Option` only so `Drop` can shut it
    /// down without blocking.
    runtime: Option<tokio::runtime::Runtime>,
    servers: BTreeMap<String, Slot>,
    generation: u64,
}

impl McpState {
    pub fn new() -> Self {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("deskhand-mcp")
            .enable_all()
            .build()
            .expect("failed to start the MCP runtime");
        Self {
            runtime: Some(runtime),
            servers: BTreeMap::new(),
            generation: 0,
        }
    }

    fn runtime(&self) -> &tokio::runtime::Runtime {
        self.runtime.as_ref().expect("runtime lives until drop")
    }
}

impl Drop for McpState {
    fn drop(&mut self) {
        // Sessions must drop inside the runtime; then stop it without
        // blocking on in-flight server I/O.
        if let Some(rt) = self.runtime.take() {
            {
                let _enter = rt.enter();
                self.servers.clear();
            }
            rt.shutdown_background();
        }
    }
}

#[derive(Debug, Serialize)]
pub struct ServerView {
    name: String,
    command: String,
    #[serde(flatten)]
    status: ServerStatus,
}

pub fn statuses(state: &AppState) -> Vec<ServerView> {
    lock(&state.mcp)
        .servers
        .iter()
        .map(|(name, slot)| ServerView {
            name: name.clone(),
            command: slot.config.command.clone(),
            status: slot.status.clone(),
        })
        .collect()
}

/// Tool names from a server start with this; names are validated in the
/// config to need no sanitizing.
pub fn tool_prefix(server: &str) -> String {
    format!("{server}__")
}

fn remove_tools(state: &AppState, server: &str) {
    let mut toolbox = lock(&state.toolbox);
    for name in toolbox.names_with_prefix(&tool_prefix(server)) {
        toolbox.unregister(&name);
    }
}

/// Applies `[mcp]` from a loaded config: disconnects servers that were
/// removed, changed, or disabled, and connects new or changed ones in the
/// background. Unchanged servers keep their connection.
pub fn apply(app: &AppHandle, config: &McpConfig) {
    let state = app.state::<AppState>();
    let mut guard = lock(&state.mcp);
    let mcp = &mut *guard;
    let runtime = mcp.runtime().handle().clone();
    // Sessions and tools hold child processes and tasks; drop them in the
    // runtime's context.
    let _enter = runtime.enter();

    let stale: Vec<String> = mcp
        .servers
        .iter()
        .filter(|(name, slot)| config.servers.get(*name) != Some(&slot.config))
        .map(|(name, _)| name.clone())
        .collect();
    for name in &stale {
        mcp.servers.remove(name);
        remove_tools(&state, name);
    }

    for (name, cfg) in &config.servers {
        if mcp.servers.contains_key(name) {
            continue;
        }
        mcp.generation += 1;
        let generation = mcp.generation;
        let status = if cfg.enabled {
            ServerStatus::Connecting
        } else {
            ServerStatus::Disabled
        };
        mcp.servers.insert(
            name.clone(),
            Slot {
                config: cfg.clone(),
                status,
                _session: None,
                generation,
            },
        );
        if !cfg.enabled {
            continue;
        }
        let spec = StdioServer {
            command: cfg.command.clone(),
            args: cfg.args.clone(),
            env: cfg.env.clone(),
        };
        let (app, name, handle) = (app.clone(), name.clone(), runtime.clone());
        runtime.spawn(async move {
            let result = async {
                let session = connect_stdio(&spec).await?;
                let (list, skipped) = tools(&name, session.clone(), handle).await?;
                Ok::<_, String>((session, list, skipped))
            }
            .await;
            finish(&app, &name, generation, result);
        });
    }
    drop(guard);
    let _ = app.emit(CONFIG_EVENT, ());
}

type Connected = (Arc<Session>, Vec<McpTool>, Vec<String>);

/// Records a connection attempt's outcome, unless a newer config replaced
/// the server meanwhile; then the result is dropped, disconnecting it.
fn finish(app: &AppHandle, name: &str, generation: u64, result: Result<Connected, String>) {
    let state = app.state::<AppState>();
    let mut mcp = lock(&state.mcp);
    let Some(slot) = mcp
        .servers
        .get_mut(name)
        .filter(|s| s.generation == generation)
    else {
        return;
    };
    match result {
        Ok((session, list, mut skipped)) => {
            let mut toolbox = lock(&state.toolbox);
            for stale in toolbox.names_with_prefix(&tool_prefix(name)) {
                toolbox.unregister(&stale);
            }
            let mut count = 0;
            for tool in list {
                let tool_name = tool.name().to_owned();
                match toolbox.register(tool) {
                    Ok(()) => count += 1,
                    Err(_) => skipped.push(tool_name),
                }
            }
            slot._session = Some(session);
            slot.status = ServerStatus::Ready {
                tools: count,
                skipped,
            };
        }
        Err(error) => slot.status = ServerStatus::Failed { error },
    }
    drop(mcp);
    let _ = app.emit(CONFIG_EVENT, ());
}
