use std::sync::{Arc, mpsc};
use std::time::Duration;

use deskhand_policy::{Permit, Source, Tier, ToolSpec};
use deskhand_tools::{Tool, ToolError, ToolOutput};
use rmcp::model::{CallToolRequestParams, CallToolResult, ContentBlock};
use serde_json::{Value, json};

use crate::Session;

/// Longest a single tool call may take.
pub fn call_timeout() -> Duration {
    Duration::from_secs(120)
}

/// Anthropic and OpenAI accept tool names matching `^[a-zA-Z0-9_-]{1,64}$`.
const MAX_NAME: usize = 64;

/// `server__tool`, restricted to the characters model APIs accept. Returns
/// `None` if nothing usable remains or the name would be too long.
pub fn tool_name(server: &str, tool: &str) -> Option<String> {
    let clean = |s: &str| -> String {
        s.chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                    c
                } else {
                    '_'
                }
            })
            .collect()
    };
    let name = format!("{}__{}", clean(server), clean(tool));
    let valid = !server.is_empty() && !tool.is_empty() && name.len() <= MAX_NAME;
    valid.then_some(name)
}

/// The policy attributes for a remote tool; see the crate docs.
pub fn spec_for(name: &str, tool: &rmcp::model::Tool) -> ToolSpec {
    let hints = tool.annotations.as_ref();
    let read_only = hints.and_then(|a| a.read_only_hint).unwrap_or(false);
    let open_world = hints.and_then(|a| a.open_world_hint).unwrap_or(true);
    let spec = ToolSpec::new(
        name,
        if read_only {
            Tier::Read
        } else {
            Tier::Irreversible
        },
    );
    if open_world {
        spec.taint_sensitive()
    } else {
        spec
    }
}

/// Flattens a tool result for the model: text blocks joined, structured
/// content included, other block kinds noted rather than dropped silently.
pub fn result_to_output(result: CallToolResult) -> Result<ToolOutput, ToolError> {
    let mut text = Vec::new();
    let mut omitted = Vec::new();
    for block in &result.content {
        match block {
            ContentBlock::Text(t) => text.push(t.text.clone()),
            ContentBlock::Image(_) => omitted.push("image"),
            ContentBlock::Audio(_) => omitted.push("audio"),
            ContentBlock::Resource(_) => omitted.push("embedded resource"),
            ContentBlock::ResourceLink(_) => omitted.push("resource link"),
            _ => omitted.push("other content"),
        }
    }
    let joined = text.join("\n");
    if result.is_error.unwrap_or(false) {
        return Err(ToolError::Failed(if joined.is_empty() {
            "the MCP server reported an error".into()
        } else {
            joined
        }));
    }
    let mut content = json!({ "text": joined });
    if let Some(structured) = result.structured_content {
        content["structured"] = structured;
    }
    if !omitted.is_empty() {
        content["omitted"] = json!(omitted);
    }
    Ok(ToolOutput::new(content).untrusted(Source::ToolOutput))
}

/// A remote tool registered in the toolbox.
pub struct McpTool {
    name: String,
    remote_name: String,
    description: String,
    schema: Value,
    spec: ToolSpec,
    session: Arc<Session>,
    runtime: tokio::runtime::Handle,
}

impl McpTool {
    /// `None` if the tool's name cannot be mapped to one model APIs accept.
    pub fn new(
        server: &str,
        tool: &rmcp::model::Tool,
        session: Arc<Session>,
        runtime: tokio::runtime::Handle,
    ) -> Option<Self> {
        let name = tool_name(server, &tool.name)?;
        let description = format!(
            "[MCP server `{server}`] {}",
            tool.description.as_deref().unwrap_or("(no description)")
        );
        Some(Self {
            spec: spec_for(&name, tool),
            name,
            remote_name: tool.name.to_string(),
            description,
            schema: Value::Object((*tool.input_schema).clone()),
            session,
            runtime,
        })
    }

    pub fn name(&self) -> &str {
        &self.name
    }
}

impl Tool for McpTool {
    fn spec(&self) -> ToolSpec {
        self.spec.clone()
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn input_schema(&self) -> Value {
        self.schema.clone()
    }

    fn execute(&self, _: &Permit, args: &Value) -> Result<ToolOutput, ToolError> {
        let arguments = match args {
            Value::Object(map) => map.clone(),
            Value::Null => serde_json::Map::new(),
            _ => {
                return Err(ToolError::InvalidArgs("arguments must be an object".into()));
            }
        };
        let params = CallToolRequestParams::new(self.remote_name.clone()).with_arguments(arguments);
        let session = self.session.clone();
        // Tool execution is synchronous; run the call on the MCP runtime and
        // wait here. Safe from any thread, including other runtimes' workers.
        let (tx, rx) = mpsc::channel();
        self.runtime.spawn(async move {
            let _ = tx.send(session.call_tool(params).await);
        });
        let result = rx
            .recv_timeout(call_timeout())
            .map_err(|_| ToolError::Failed("the MCP server did not answer in time".into()))?
            .map_err(|e| ToolError::Failed(format!("MCP call failed: {e}")))?;
        result_to_output(result)
    }
}
