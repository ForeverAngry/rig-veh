//! Additive evidence bundle attached to [`AgentNode`] metadata.
//!
//! `EvidenceBundle` captures *why* a mutation passed or failed in a single
//! reload-friendly record. It is intentionally additive: the canonical
//! signing payload still only sees the bundle through the existing
//! `eval_results` JSON field, so attaching a bundle does not change hash
//! semantics or the on-disk shape of [`AgentNode`].
//!
//! The bundle holds, in order:
//!
//! - [`PolicyVerdict`] — the gate's decision and reason code,
//! - `evaluator_report` — the raw report JSON the evaluator emitted,
//! - `context_items` — the context items that fed the decision (kept as
//!   opaque JSON values so this crate does not pull in `rig-compose`'s
//!   `ContextItem` type),
//! - `operator_approvals` — out-of-band approvals collected for SR-1.
//!
//! ```
//! use rig_veh::evidence::{EvidenceBundle, OperatorApproval, PolicyVerdict};
//! use serde_json::json;
//!
//! let bundle = EvidenceBundle::new(PolicyVerdict::approved("within_scope"))
//!     .with_evaluator_report(json!({"recall@10": 0.83}))
//!     .with_context_items(vec![json!({"source": "memvid", "score": 0.91})])
//!     .with_operator_approval(OperatorApproval::new("alice", "ship it"));
//!
//! let value = bundle.to_value().unwrap();
//! let parsed = EvidenceBundle::from_value(&value).unwrap();
//! assert_eq!(bundle, parsed);
//! ```

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{Error, Result};
use crate::policy::PolicyDecision;

/// Current evidence-bundle schema version. Bumping is hash-neutral
/// because the bundle is carried inside an opaque `eval_results` value.
pub const EVIDENCE_BUNDLE_VERSION: u32 = 1;

/// Stable verdict shape derived from [`PolicyDecision`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum PolicyVerdict {
    /// Candidate was approved by the gate.
    Approved {
        /// Short machine-readable reason code.
        reason: String,
    },
    /// Candidate was denied by the gate.
    Denied {
        /// Short machine-readable reason code.
        reason: String,
    },
    /// Candidate is parked awaiting an out-of-band approval.
    RequiresApproval {
        /// Short machine-readable reason code.
        reason: String,
    },
}

impl PolicyVerdict {
    /// Approved verdict with a reason code.
    #[must_use]
    pub fn approved(reason: impl Into<String>) -> Self {
        Self::Approved {
            reason: reason.into(),
        }
    }

    /// Denied verdict with a reason code.
    #[must_use]
    pub fn denied(reason: impl Into<String>) -> Self {
        Self::Denied {
            reason: reason.into(),
        }
    }

    /// Requires-approval verdict with a reason code.
    #[must_use]
    pub fn requires_approval(reason: impl Into<String>) -> Self {
        Self::RequiresApproval {
            reason: reason.into(),
        }
    }
}

impl From<&PolicyDecision> for PolicyVerdict {
    fn from(decision: &PolicyDecision) -> Self {
        match decision {
            PolicyDecision::Approve => Self::approved("approved"),
            PolicyDecision::Deny { reason } => Self::denied(reason.clone()),
            PolicyDecision::RequireApproval { reason } => Self::requires_approval(reason.clone()),
        }
    }
}

impl From<PolicyDecision> for PolicyVerdict {
    fn from(decision: PolicyDecision) -> Self {
        (&decision).into()
    }
}

/// Out-of-band approval captured for an SR-1 pause.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperatorApproval {
    /// Operator identifier (e.g. email, KID).
    pub operator: String,
    /// Free-form note recorded with the approval.
    pub note: String,
}

impl OperatorApproval {
    /// Create a new operator approval.
    #[must_use]
    pub fn new(operator: impl Into<String>, note: impl Into<String>) -> Self {
        Self {
            operator: operator.into(),
            note: note.into(),
        }
    }
}

/// Additive evidence record attached to an [`AgentNode`] via
/// `eval_results`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceBundle {
    /// Schema version. Always [`EVIDENCE_BUNDLE_VERSION`] for newly built
    /// bundles. Older bundles loaded from disk keep their original value.
    pub version: u32,
    /// Policy gate verdict.
    pub policy: PolicyVerdict,
    /// Raw evaluator report (opaque JSON; typically a
    /// `rig_evals_rag::MultiReport` or `ReportDiff`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evaluator_report: Option<Value>,
    /// Context items selected for the decision. Kept as opaque JSON so
    /// `rig-veh` does not pull in `rig-compose`'s `ContextItem` type.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub context_items: Vec<Value>,
    /// Out-of-band approvals collected for SR-1 pauses.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub operator_approvals: Vec<OperatorApproval>,
}

impl EvidenceBundle {
    /// Start a new bundle from a policy verdict.
    #[must_use]
    pub fn new(policy: PolicyVerdict) -> Self {
        Self {
            version: EVIDENCE_BUNDLE_VERSION,
            policy,
            evaluator_report: None,
            context_items: Vec::new(),
            operator_approvals: Vec::new(),
        }
    }

    /// Attach the evaluator's raw report JSON.
    #[must_use]
    pub fn with_evaluator_report(mut self, report: Value) -> Self {
        self.evaluator_report = Some(report);
        self
    }

    /// Attach the selected context items.
    #[must_use]
    pub fn with_context_items(mut self, items: Vec<Value>) -> Self {
        self.context_items = items;
        self
    }

    /// Append an operator approval.
    #[must_use]
    pub fn with_operator_approval(mut self, approval: OperatorApproval) -> Self {
        self.operator_approvals.push(approval);
        self
    }

    /// Render the bundle as a canonical JSON `Value` suitable for
    /// stashing in [`crate::node::AgentNode::eval_results`].
    ///
    /// # Errors
    ///
    /// Returns [`Error`] when serde fails (should not happen for the
    /// in-crate types).
    pub fn to_value(&self) -> Result<Value> {
        serde_json::to_value(self).map_err(Error::from)
    }

    /// Parse an evidence bundle from a JSON value (typically
    /// [`crate::node::AgentNode::eval_results`]).
    ///
    /// # Errors
    ///
    /// Returns [`Error`] when the value does not deserialize into an
    /// [`EvidenceBundle`].
    pub fn from_value(value: &Value) -> Result<Self> {
        serde_json::from_value(value.clone()).map_err(Error::from)
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::expect_used
)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn approved_bundle_round_trips_through_json() {
        let bundle = EvidenceBundle::new(PolicyVerdict::approved("within_scope"))
            .with_evaluator_report(json!({"recall@10": 0.83}))
            .with_context_items(vec![json!({"source": "memvid", "score": 0.91})])
            .with_operator_approval(OperatorApproval::new("alice", "ship it"));

        let value = bundle.to_value().unwrap();
        let parsed = EvidenceBundle::from_value(&value).unwrap();
        assert_eq!(parsed, bundle);
        assert_eq!(parsed.version, EVIDENCE_BUNDLE_VERSION);
    }

    #[test]
    fn policy_decision_maps_to_verdict() {
        let deny = PolicyDecision::Deny {
            reason: "cap_out_of_scope".into(),
        };
        let bundle = EvidenceBundle::new(PolicyVerdict::from(&deny));
        match bundle.policy {
            PolicyVerdict::Denied { reason } => assert_eq!(reason, "cap_out_of_scope"),
            other => panic!("expected Denied, got {other:?}"),
        }

        let pending = PolicyDecision::RequireApproval {
            reason: "scope_widened".into(),
        };
        let verdict: PolicyVerdict = pending.into();
        assert_eq!(verdict, PolicyVerdict::requires_approval("scope_widened"));
    }

    #[test]
    fn minimal_bundle_omits_empty_fields_from_json() {
        let bundle = EvidenceBundle::new(PolicyVerdict::approved("ok"));
        let json = serde_json::to_string(&bundle).unwrap();
        assert!(!json.contains("evaluator_report"));
        assert!(!json.contains("context_items"));
        assert!(!json.contains("operator_approvals"));
    }
}
