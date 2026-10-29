use serde::{Deserialize, Serialize};

/// The risk class of a tool, declared by the tool itself at registration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Tier {
    /// Observes state without changing it. Runs without confirmation.
    Read,
    /// Changes state in a way DeskHand can undo. Runs without confirmation,
    /// is logged, and records undo data.
    Reversible,
    /// Changes state in a way DeskHand cannot undo. Always requires confirmation.
    Irreversible,
}
