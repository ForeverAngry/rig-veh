//! Mutation intent — the declarative *why* behind a candidate agent.
//!
//! Every candidate must be accompanied by a [`MutationIntent`]
//! describing the goal of the mutation, the expected improvement, and
//! any constraints the host wants the gate to enforce. The intent is
//! folded into the artifact hash, so it is immutable once the candidate
//! is committed.

use serde::{Deserialize, Serialize};

/// Declarative description of a proposed mutation.
///
/// The intent is part of the canonical artifact bytes that feed the
/// `agent_id` hash, so any tampering with `goal` or `constraints` after
/// the fact will be detected by [`crate::identity::verify_node`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MutationIntent {
    /// Short human-readable label (e.g. `"improve_recall_at_10"`).
    pub goal: String,
    /// Optional free-form description of *why* this mutation was proposed.
    #[serde(default)]
    pub rationale: String,
    /// Expected improvement on the host's primary metric, in the same
    /// units as [`crate::evaluator::EvalResult::score`].
    #[serde(default)]
    pub expected_improvement: Option<f64>,
    /// Hard constraints the host wants the policy gate to enforce
    /// (e.g. `"no_new_mcp_servers"`, `"max_tokens_lt_8k"`).
    #[serde(default)]
    pub constraints: Vec<String>,
    /// Free-form metadata for host-specific telemetry.
    #[serde(default)]
    pub metadata: serde_json::Value,
}

impl MutationIntent {
    /// Construct a new intent with only the required `goal` field.
    pub fn new(goal: impl Into<String>) -> Self {
        Self {
            goal: goal.into(),
            rationale: String::new(),
            expected_improvement: None,
            constraints: Vec::new(),
            metadata: serde_json::Value::Null,
        }
    }

    /// Set the human-readable rationale.
    pub fn with_rationale(mut self, rationale: impl Into<String>) -> Self {
        self.rationale = rationale.into();
        self
    }

    /// Set the expected scalar improvement.
    pub fn with_expected_improvement(mut self, delta: f64) -> Self {
        self.expected_improvement = Some(delta);
        self
    }

    /// Append a constraint tag.
    pub fn with_constraint(mut self, tag: impl Into<String>) -> Self {
        self.constraints.push(tag.into());
        self
    }
}

/// Hard governance boundaries inherited by all descendants of a node.
///
/// The policy gate cross-references a candidate's `capability_sbom`
/// against the parent's [`AllowedScope`]. Anything outside the scope
/// requires an out-of-band approval (see
/// [`crate::policy::PolicyDecision::RequireApproval`]).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[allow(clippy::derive_partial_eq_without_eq)]
pub struct AllowedScope {
    /// Tool names (or prefixes) descendants may invoke. An empty list
    /// means "inherit parent's manifest verbatim — no additions".
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    /// MCP server names descendants may add. Empty = no new servers.
    #[serde(default)]
    pub allowed_mcp_servers: Vec<String>,
    /// Delegate agents descendants may add. Empty = no new delegates.
    #[serde(default)]
    pub allowed_delegates: Vec<String>,
    /// Maximum generation depth permitted under this scope. `None` =
    /// unbounded.
    #[serde(default)]
    pub max_generation: Option<u32>,
    /// Capability tags that always require human approval (e.g.
    /// `"autonomous_trading"`).
    #[serde(default)]
    pub gated_capabilities: Vec<String>,
}
