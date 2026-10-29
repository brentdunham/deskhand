use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

/// Where untrusted content entered the model context.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    /// Text read from another application's UI (UI Automation).
    Screen,
    Clipboard,
    File,
    Web,
    /// Output of a tool or external MCP server.
    ToolOutput,
}

/// Tracks untrusted content present in the model context.
///
/// Taint is scoped to the context, not the turn: content read earlier stays in
/// the conversation history and can still steer the model on later turns.
/// Clear it only when the context itself is cleared.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ContextTaint {
    sources: BTreeSet<Source>,
}

impl ContextTaint {
    pub fn new() -> Self {
        Self::default()
    }

    /// Records that content from `source` was added to the model context.
    pub fn ingest(&mut self, source: Source) {
        self.sources.insert(source);
    }

    pub fn is_tainted(&self) -> bool {
        !self.sources.is_empty()
    }

    pub fn sources(&self) -> impl Iterator<Item = Source> + '_ {
        self.sources.iter().copied()
    }

    /// Call only when the model context is cleared (new conversation).
    pub fn clear(&mut self) {
        self.sources.clear();
    }
}
