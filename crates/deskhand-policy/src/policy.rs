use std::collections::HashMap;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::taint::{ContextTaint, Source};
use crate::tier::Tier;

/// A tool's declared policy attributes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolSpec {
    name: String,
    tier: Tier,
    taint_sensitive: bool,
}

impl ToolSpec {
    pub fn new(name: impl Into<String>, tier: Tier) -> Self {
        Self {
            name: name.into(),
            tier,
            taint_sensitive: false,
        }
    }

    /// Marks the tool as able to move data out of the machine or act on the
    /// user's behalf (opening URLs, launching apps, writing the clipboard,
    /// moving files). Such tools require confirmation while the context is tainted.
    pub fn taint_sensitive(mut self) -> Self {
        self.taint_sensitive = true;
        self
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn tier(&self) -> Tier {
        self.tier
    }

    pub fn is_taint_sensitive(&self) -> bool {
        self.taint_sensitive
    }
}

/// A user-set override. Overrides can only tighten policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Override {
    /// Require confirmation even where the tier would allow the tool.
    Ask,
    /// Never run the tool.
    Deny,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum ConfirmReason {
    Irreversible,
    UserOverride,
    TaintedContext { sources: Vec<Source> },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DenyReason {
    UnknownTool,
    UserOverride,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Allow,
    Confirm(ConfirmReason),
    Deny(DenyReason),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicyError {
    DuplicateTool(String),
}

impl fmt::Display for PolicyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateTool(name) => write!(f, "tool `{name}` is already registered"),
        }
    }
}

impl std::error::Error for PolicyError {}

/// Authority to execute one tool call. Created only by this crate, consumed by
/// the executor. Not `Clone`: one permit, one execution.
#[derive(Debug)]
pub struct Permit {
    tool: String,
    tier: Tier,
    confirmed: bool,
}

impl Permit {
    pub fn tool(&self) -> &str {
        &self.tool
    }

    pub fn tier(&self) -> Tier {
        self.tier
    }

    /// Whether the user explicitly approved this call.
    pub fn was_confirmed(&self) -> bool {
        self.confirmed
    }
}

/// A call that is waiting on the user. The agent loop must route this to the
/// UI and call [`approve`](Self::approve) only on explicit user approval.
#[derive(Debug)]
pub struct PendingConfirmation {
    tool: String,
    tier: Tier,
    reason: ConfirmReason,
}

impl PendingConfirmation {
    pub fn tool(&self) -> &str {
        &self.tool
    }

    pub fn tier(&self) -> Tier {
        self.tier
    }

    pub fn reason(&self) -> &ConfirmReason {
        &self.reason
    }

    /// Converts the pending call into a permit. Call only in response to an
    /// explicit user action in the UI.
    pub fn approve(self) -> Permit {
        Permit {
            tool: self.tool,
            tier: self.tier,
            confirmed: true,
        }
    }
}

#[derive(Debug)]
pub enum Authorization {
    Permitted(Permit),
    NeedsConfirmation(PendingConfirmation),
    Denied(DenyReason),
}

#[derive(Debug, Default)]
pub struct Policy {
    tools: HashMap<String, ToolSpec>,
    overrides: HashMap<String, Override>,
}

impl Policy {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, spec: ToolSpec) -> Result<(), PolicyError> {
        if self.tools.contains_key(spec.name()) {
            return Err(PolicyError::DuplicateTool(spec.name));
        }
        self.tools.insert(spec.name.clone(), spec);
        Ok(())
    }

    pub fn spec(&self, tool: &str) -> Option<&ToolSpec> {
        self.tools.get(tool)
    }

    /// Removes a tool. Calls to it are denied as unknown from then on.
    /// Overrides that name it are kept, as for not-yet-registered tools.
    pub fn unregister(&mut self, tool: &str) -> bool {
        self.tools.remove(tool).is_some()
    }

    /// Sets or clears (`None`) a user override. Overrides for unregistered tools
    /// are kept, so config can name tools that a plugin registers later.
    pub fn set_override(&mut self, tool: impl Into<String>, value: Option<Override>) {
        let tool = tool.into();
        match value {
            Some(v) => {
                self.overrides.insert(tool, v);
            }
            None => {
                self.overrides.remove(&tool);
            }
        }
    }

    /// Replaces every user override, e.g. after the config file reloads.
    /// Tools missing from `overrides` return to their default policy.
    pub fn replace_overrides(&mut self, overrides: impl IntoIterator<Item = (String, Override)>) {
        self.overrides = overrides.into_iter().collect();
    }

    /// Evaluates a tool call without producing a permit. Rules, in order:
    /// unknown tool, Deny override, Irreversible tier, Ask override, tainted
    /// context for taint-sensitive tools, then allow.
    pub fn decide(&self, tool: &str, taint: &ContextTaint) -> Decision {
        let Some(spec) = self.tools.get(tool) else {
            return Decision::Deny(DenyReason::UnknownTool);
        };
        let ov = self.overrides.get(tool).copied();

        if ov == Some(Override::Deny) {
            return Decision::Deny(DenyReason::UserOverride);
        }
        if spec.tier == Tier::Irreversible {
            return Decision::Confirm(ConfirmReason::Irreversible);
        }
        if ov == Some(Override::Ask) {
            return Decision::Confirm(ConfirmReason::UserOverride);
        }
        if spec.taint_sensitive && taint.is_tainted() {
            return Decision::Confirm(ConfirmReason::TaintedContext {
                sources: taint.sources().collect(),
            });
        }
        Decision::Allow
    }

    /// Evaluates a tool call and returns the permit or pending confirmation.
    pub fn authorize(&self, tool: &str, taint: &ContextTaint) -> Authorization {
        match self.decide(tool, taint) {
            Decision::Deny(reason) => Authorization::Denied(reason),
            Decision::Allow => {
                let spec = &self.tools[tool];
                Authorization::Permitted(Permit {
                    tool: spec.name.clone(),
                    tier: spec.tier,
                    confirmed: false,
                })
            }
            Decision::Confirm(reason) => {
                let spec = &self.tools[tool];
                Authorization::NeedsConfirmation(PendingConfirmation {
                    tool: spec.name.clone(),
                    tier: spec.tier,
                    reason,
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy() -> Policy {
        let mut p = Policy::new();
        p.register(ToolSpec::new("list_windows", Tier::Read))
            .unwrap();
        p.register(ToolSpec::new("snap_window", Tier::Reversible))
            .unwrap();
        p.register(ToolSpec::new("open_url", Tier::Reversible).taint_sensitive())
            .unwrap();
        p.register(ToolSpec::new("empty_recycle_bin", Tier::Irreversible))
            .unwrap();
        p
    }

    fn tainted() -> ContextTaint {
        let mut t = ContextTaint::new();
        t.ingest(Source::Screen);
        t
    }

    #[test]
    fn unknown_tool_is_denied() {
        let p = policy();
        assert_eq!(
            p.decide("format_disk", &ContextTaint::new()),
            Decision::Deny(DenyReason::UnknownTool)
        );
    }

    #[test]
    fn duplicate_registration_fails() {
        let mut p = policy();
        assert_eq!(
            p.register(ToolSpec::new("list_windows", Tier::Read)),
            Err(PolicyError::DuplicateTool("list_windows".into()))
        );
    }

    #[test]
    fn read_and_reversible_allowed_by_default() {
        let p = policy();
        let clean = ContextTaint::new();
        assert_eq!(p.decide("list_windows", &clean), Decision::Allow);
        assert_eq!(p.decide("snap_window", &clean), Decision::Allow);
        assert_eq!(p.decide("open_url", &clean), Decision::Allow);
    }

    #[test]
    fn irreversible_always_confirms_under_every_condition() {
        let mut p = policy();
        for taint in [ContextTaint::new(), tainted()] {
            for ov in [None, Some(Override::Ask)] {
                p.set_override("empty_recycle_bin", ov);
                assert!(matches!(
                    p.authorize("empty_recycle_bin", &taint),
                    Authorization::NeedsConfirmation(_)
                ));
            }
        }
    }

    #[test]
    fn deny_override_beats_everything() {
        let mut p = policy();
        for tool in [
            "list_windows",
            "snap_window",
            "open_url",
            "empty_recycle_bin",
        ] {
            p.set_override(tool, Some(Override::Deny));
            assert_eq!(
                p.decide(tool, &ContextTaint::new()),
                Decision::Deny(DenyReason::UserOverride)
            );
        }
    }

    #[test]
    fn ask_override_requires_confirmation() {
        let mut p = policy();
        p.set_override("snap_window", Some(Override::Ask));
        assert_eq!(
            p.decide("snap_window", &ContextTaint::new()),
            Decision::Confirm(ConfirmReason::UserOverride)
        );
        p.set_override("snap_window", None);
        assert_eq!(
            p.decide("snap_window", &ContextTaint::new()),
            Decision::Allow
        );
    }

    #[test]
    fn replace_overrides_drops_previous_ones() {
        let mut p = policy();
        p.set_override("snap_window", Some(Override::Ask));
        p.replace_overrides([("list_windows".to_owned(), Override::Deny)]);
        let clean = ContextTaint::new();
        assert_eq!(p.decide("snap_window", &clean), Decision::Allow);
        assert_eq!(
            p.decide("list_windows", &clean),
            Decision::Deny(DenyReason::UserOverride)
        );
    }

    #[test]
    fn taint_gates_only_sensitive_tools() {
        let p = policy();
        let t = tainted();
        assert_eq!(p.decide("snap_window", &t), Decision::Allow);
        assert_eq!(
            p.decide("open_url", &t),
            Decision::Confirm(ConfirmReason::TaintedContext {
                sources: vec![Source::Screen]
            })
        );
    }

    #[test]
    fn approval_produces_confirmed_permit() {
        let p = policy();
        let Authorization::NeedsConfirmation(pending) =
            p.authorize("empty_recycle_bin", &ContextTaint::new())
        else {
            panic!("expected confirmation");
        };
        let permit = pending.approve();
        assert!(permit.was_confirmed());
        assert_eq!(permit.tier(), Tier::Irreversible);
    }

    #[test]
    fn allowed_permit_is_unconfirmed() {
        let p = policy();
        let Authorization::Permitted(permit) = p.authorize("list_windows", &ContextTaint::new())
        else {
            panic!("expected permit");
        };
        assert!(!permit.was_confirmed());
    }
}
