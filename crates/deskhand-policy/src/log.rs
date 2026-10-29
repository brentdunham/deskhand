use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::policy::{DenyReason, PendingConfirmation, Permit};
use crate::tier::Tier;

pub type ActionId = u64;

/// How a call passed (or failed to pass) the policy gate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum Gate {
    Allowed,
    Confirmed,
    /// The user rejected a confirmation request.
    Declined,
    Denied {
        reason: DenyReason,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum Outcome {
    Pending,
    Succeeded,
    Failed {
        error: String,
    },
    Undone,
    /// Declined or denied at the gate; nothing ran.
    NotExecuted,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActionEntry {
    pub id: ActionId,
    pub at_unix_ms: u64,
    pub tool: String,
    /// `None` for calls to unregistered tools.
    pub tier: Option<Tier>,
    pub args: Value,
    pub gate: Gate,
    pub outcome: Outcome,
    /// Data the executor needs to reverse a successful reversible action.
    pub undo: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LogError {
    UnknownAction(ActionId),
    InvalidTransition { id: ActionId, from: Outcome },
    NotUndoable(ActionId),
}

impl fmt::Display for LogError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownAction(id) => write!(f, "no action with id {id}"),
            Self::InvalidTransition { id, from } => {
                write!(f, "action {id} cannot transition from {from:?}")
            }
            Self::NotUndoable(id) => write!(f, "action {id} has no undo data"),
        }
    }
}

impl std::error::Error for LogError {}

/// Append-only record of every tool call and its gate result.
///
/// In-memory for now; persistence (SQLite) wraps this type later.
/// Entries are never removed, only transitioned:
/// `Pending -> Succeeded | Failed`, `Succeeded -> Undone`.
#[derive(Debug, Default)]
pub struct ActionLog {
    entries: Vec<ActionEntry>,
}

impl ActionLog {
    pub fn new() -> Self {
        Self::default()
    }

    /// Records a permitted call about to execute.
    pub fn begin(&mut self, permit: &Permit, args: Value) -> ActionId {
        let gate = if permit.was_confirmed() {
            Gate::Confirmed
        } else {
            Gate::Allowed
        };
        self.push(
            permit.tool(),
            Some(permit.tier()),
            args,
            gate,
            Outcome::Pending,
        )
    }

    /// Records a confirmation the user rejected. Consumes the pending call.
    pub fn record_declined(&mut self, pending: PendingConfirmation, args: Value) -> ActionId {
        self.push(
            pending.tool(),
            Some(pending.tier()),
            args,
            Gate::Declined,
            Outcome::NotExecuted,
        )
    }

    /// Records a call the policy denied.
    pub fn record_denied(
        &mut self,
        tool: &str,
        tier: Option<Tier>,
        reason: DenyReason,
        args: Value,
    ) -> ActionId {
        self.push(
            tool,
            tier,
            args,
            Gate::Denied { reason },
            Outcome::NotExecuted,
        )
    }

    pub fn succeed(&mut self, id: ActionId, undo: Option<Value>) -> Result<(), LogError> {
        let entry = self.pending_mut(id)?;
        entry.outcome = Outcome::Succeeded;
        entry.undo = undo;
        Ok(())
    }

    pub fn fail(&mut self, id: ActionId, error: impl Into<String>) -> Result<(), LogError> {
        let entry = self.pending_mut(id)?;
        entry.outcome = Outcome::Failed {
            error: error.into(),
        };
        Ok(())
    }

    /// Marks a succeeded action as undone. The executor performs the reversal
    /// using the entry's undo data before calling this.
    pub fn mark_undone(&mut self, id: ActionId) -> Result<(), LogError> {
        let entry = self.entry_mut(id)?;
        if entry.outcome != Outcome::Succeeded {
            return Err(LogError::InvalidTransition {
                id,
                from: entry.outcome.clone(),
            });
        }
        if entry.undo.is_none() {
            return Err(LogError::NotUndoable(id));
        }
        entry.outcome = Outcome::Undone;
        Ok(())
    }

    pub fn get(&self, id: ActionId) -> Option<&ActionEntry> {
        self.index(id).map(|i| &self.entries[i])
    }

    pub fn entries(&self) -> &[ActionEntry] {
        &self.entries
    }

    fn push(
        &mut self,
        tool: &str,
        tier: Option<Tier>,
        args: Value,
        gate: Gate,
        outcome: Outcome,
    ) -> ActionId {
        let id = self.entries.last().map_or(1, |e| e.id + 1);
        self.entries.push(ActionEntry {
            id,
            at_unix_ms: now_unix_ms(),
            tool: tool.to_owned(),
            tier,
            args,
            gate,
            outcome,
            undo: None,
        });
        id
    }

    // Ids are assigned sequentially from 1 and never removed, so lookup is O(1).
    fn index(&self, id: ActionId) -> Option<usize> {
        let i = usize::try_from(id.checked_sub(1)?).ok()?;
        (i < self.entries.len()).then_some(i)
    }

    fn entry_mut(&mut self, id: ActionId) -> Result<&mut ActionEntry, LogError> {
        let i = self.index(id).ok_or(LogError::UnknownAction(id))?;
        Ok(&mut self.entries[i])
    }

    fn pending_mut(&mut self, id: ActionId) -> Result<&mut ActionEntry, LogError> {
        let entry = self.entry_mut(id)?;
        if entry.outcome != Outcome::Pending {
            return Err(LogError::InvalidTransition {
                id,
                from: entry.outcome.clone(),
            });
        }
        Ok(entry)
    }
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Authorization, ContextTaint, Policy, ToolSpec};
    use serde_json::json;

    fn policy() -> Policy {
        let mut p = Policy::new();
        p.register(ToolSpec::new("snap_window", Tier::Reversible))
            .unwrap();
        p.register(ToolSpec::new("empty_recycle_bin", Tier::Irreversible))
            .unwrap();
        p
    }

    fn permit(p: &Policy, tool: &str) -> Permit {
        match p.authorize(tool, &ContextTaint::new()) {
            Authorization::Permitted(permit) => permit,
            Authorization::NeedsConfirmation(pending) => pending.approve(),
            Authorization::Denied(r) => panic!("denied: {r:?}"),
        }
    }

    #[test]
    fn records_allowed_call_lifecycle_and_undo() {
        let p = policy();
        let mut log = ActionLog::new();
        let id = log.begin(&permit(&p, "snap_window"), json!({ "hwnd": 1 }));
        assert_eq!(log.get(id).unwrap().gate, Gate::Allowed);
        assert_eq!(log.get(id).unwrap().outcome, Outcome::Pending);

        log.succeed(id, Some(json!({ "rect": [0, 0, 800, 600] })))
            .unwrap();
        log.mark_undone(id).unwrap();
        assert_eq!(log.get(id).unwrap().outcome, Outcome::Undone);
    }

    #[test]
    fn confirmed_call_is_logged_as_confirmed() {
        let p = policy();
        let mut log = ActionLog::new();
        let id = log.begin(&permit(&p, "empty_recycle_bin"), json!({}));
        assert_eq!(log.get(id).unwrap().gate, Gate::Confirmed);
    }

    #[test]
    fn declined_and_denied_are_recorded_as_not_executed() {
        let p = policy();
        let mut log = ActionLog::new();

        let Authorization::NeedsConfirmation(pending) =
            p.authorize("empty_recycle_bin", &ContextTaint::new())
        else {
            panic!("expected confirmation");
        };
        let declined = log.record_declined(pending, json!({}));
        let denied = log.record_denied("format_disk", None, DenyReason::UnknownTool, json!({}));

        assert_eq!(log.get(declined).unwrap().gate, Gate::Declined);
        assert_eq!(log.get(declined).unwrap().outcome, Outcome::NotExecuted);
        assert_eq!(log.get(denied).unwrap().outcome, Outcome::NotExecuted);
        assert_eq!(log.get(denied).unwrap().tier, None);
    }

    #[test]
    fn rejects_invalid_transitions() {
        let p = policy();
        let mut log = ActionLog::new();
        let id = log.begin(&permit(&p, "snap_window"), json!({}));

        assert_eq!(
            log.mark_undone(id),
            Err(LogError::InvalidTransition {
                id,
                from: Outcome::Pending
            })
        );
        log.fail(id, "window closed").unwrap();
        assert!(matches!(
            log.succeed(id, None),
            Err(LogError::InvalidTransition { .. })
        ));
        assert_eq!(log.fail(99, "x"), Err(LogError::UnknownAction(99)));
        assert_eq!(log.get(0), None);
    }

    #[test]
    fn undo_requires_undo_data() {
        let p = policy();
        let mut log = ActionLog::new();
        let id = log.begin(&permit(&p, "snap_window"), json!({}));
        log.succeed(id, None).unwrap();
        assert_eq!(log.mark_undone(id), Err(LogError::NotUndoable(id)));
    }

    #[test]
    fn entries_serialize_to_json() {
        let p = policy();
        let mut log = ActionLog::new();
        let id = log.begin(&permit(&p, "snap_window"), json!({ "hwnd": 1 }));
        log.fail(id, "boom").unwrap();
        let v = serde_json::to_value(log.get(id).unwrap()).unwrap();
        assert_eq!(v["tier"], "reversible");
        assert_eq!(v["gate"]["kind"], "allowed");
        assert_eq!(v["outcome"]["kind"], "failed");
        assert_eq!(v["outcome"]["error"], "boom");
    }
}
