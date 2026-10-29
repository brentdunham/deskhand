//! Policy engine: permission tiers, the action log, and untrusted-content tagging.
//!
//! Every tool invocation passes through [`Policy::authorize`] before it executes.
//! Tool executors accept only a [`Permit`], which this crate alone can create, so
//! an action cannot run without a policy decision.
//!
//! Invariants:
//! - A tool that is not registered is always denied.
//! - A [`Tier::Irreversible`] tool always requires user confirmation. No override
//!   or configuration can remove that requirement.
//! - User overrides can only tighten a tool's policy ([`Override::Ask`],
//!   [`Override::Deny`]), never loosen it.
//! - While untrusted content is in the model context, taint-sensitive tools
//!   require confirmation regardless of tier.

mod log;
mod policy;
mod taint;
mod tier;

pub use log::{ActionEntry, ActionId, ActionLog, Gate, LogError, Outcome};
pub use policy::{
    Authorization, ConfirmReason, Decision, DenyReason, Override, PendingConfirmation, Permit,
    Policy, PolicyError, ToolSpec,
};
pub use taint::{ContextTaint, Source};
pub use tier::Tier;
