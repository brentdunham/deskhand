//! MCP client: connects to configured MCP servers and exposes their tools
//! through the DeskHand [`Toolbox`](deskhand_tools::Toolbox), so the policy
//! gate, confirmations, and action log apply to them like any built-in tool.
//!
//! Policy for remote tools:
//! - A tool annotated `readOnlyHint: true` is [`Tier::Read`]. Anything else
//!   is [`Tier::Irreversible`] and always asks: DeskHand cannot undo another
//!   server's actions. Annotations are hints from a server the user chose to
//!   configure, so they are honored, not trusted as proof.
//! - A tool that may reach outside the machine (`openWorldHint`, which
//!   defaults to true) is taint-sensitive: a read-only fetch can still leak
//!   data through a URL, so once untrusted content is in the conversation it
//!   asks first.
//! - Every result is untrusted ([`Source::ToolOutput`]).
//!
//! [`Tier::Read`]: deskhand_policy::Tier::Read
//! [`Tier::Irreversible`]: deskhand_policy::Tier::Irreversible
//! [`Source::ToolOutput`]: deskhand_policy::Source::ToolOutput

mod tool;

pub use tool::{McpTool, call_timeout, result_to_output, spec_for, tool_name};

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use rmcp::service::RunningService;
use rmcp::{RoleClient, ServiceExt};

/// A connected server session.
pub type Session = RunningService<RoleClient, ()>;

/// How to start a local MCP server over stdio.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StdioServer {
    pub command: String,
    pub args: Vec<String>,
    /// Added to DeskHand's own environment for the child.
    pub env: BTreeMap<String, String>,
}

/// How long a server gets to start and complete the handshake. `npx` and
/// `uvx` can download packages on first run, so this is generous.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(60);

/// Connects to an MCP server over any transport rmcp accepts.
pub async fn connect<T, E, A>(transport: T) -> Result<Arc<Session>, String>
where
    T: rmcp::transport::IntoTransport<RoleClient, E, A> + Send + 'static,
    E: std::error::Error + Send + Sync + 'static,
{
    let session = tokio::time::timeout(CONNECT_TIMEOUT, ().serve(transport))
        .await
        .map_err(|_| format!("no response within {}s", CONNECT_TIMEOUT.as_secs()))?
        .map_err(|e| format!("handshake failed: {e}"))?;
    Ok(Arc::new(session))
}

/// Starts a stdio server as a hidden child process and connects to it.
pub async fn connect_stdio(server: &StdioServer) -> Result<Arc<Session>, String> {
    let program = resolve_command(&server.command)
        .ok_or_else(|| format!("`{}` was not found on PATH", server.command))?;
    let mut cmd = tokio::process::Command::new(program);
    cmd.args(&server.args)
        .envs(&server.env)
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    #[cfg(windows)]
    {
        // A console program started from a GUI app opens a console window
        // unless created without one.
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let transport = rmcp::transport::TokioChildProcess::new(cmd)
        .map_err(|e| format!("could not start `{}`: {e}", server.command))?;
    connect(transport).await
}

/// Finds a command on PATH the way a shell would. On Windows this includes
/// `.cmd` and `.bat` scripts such as `npx.cmd`, which `Command::new("npx")`
/// does not find; Rust runs those through cmd.exe with safe argument quoting.
pub fn resolve_command(command: &str) -> Option<std::path::PathBuf> {
    let path = std::path::Path::new(command);
    if path.components().count() > 1 || path.is_absolute() {
        return path.is_file().then(|| path.to_owned());
    }
    let exts: &[&str] = if cfg!(windows) {
        &["", ".exe", ".cmd", ".bat", ".com"]
    } else {
        &[""]
    };
    let dirs = std::env::var_os("PATH")?;
    std::env::split_paths(&dirs).find_map(|dir| {
        exts.iter().find_map(|ext| {
            let candidate = dir.join(format!("{command}{ext}"));
            candidate.is_file().then_some(candidate)
        })
    })
}

/// Lists a connected server's tools as toolbox tools. Tools whose names
/// cannot be mapped are reported in the second list.
pub async fn tools(
    server: &str,
    session: Arc<Session>,
    runtime: tokio::runtime::Handle,
) -> Result<(Vec<McpTool>, Vec<String>), String> {
    let listed = session
        .list_all_tools()
        .await
        .map_err(|e| format!("tools/list failed: {e}"))?;
    let mut out = Vec::new();
    let mut skipped = Vec::new();
    for t in listed {
        match McpTool::new(server, &t, session.clone(), runtime.clone()) {
            Some(tool) => out.push(tool),
            None => skipped.push(t.name.to_string()),
        }
    }
    Ok((out, skipped))
}
