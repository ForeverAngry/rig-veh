//! Deterministic policy gate.
//!
//! The gate is intentionally LLM-free: it compares a candidate's
//! capability SBOM and intent against the parent node's
//! [`AllowedScope`] and produces a [`PolicyDecision`]. The
//! [`crate::graph`] state machine consumes the decision and routes
//! either to commit (`Approve`), reject (`Deny`), or pause for
//! out-of-band approval (`RequireApproval`).

use std::collections::HashSet;

use crate::intent::{AllowedScope, MutationIntent};
use crate::node::AgentNode;

/// Outcome of a [`PolicyGate::evaluate`] call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicyDecision {
    /// Candidate is within scope and may be promoted.
    Approve,
    /// Candidate violates a hard constraint and must be discarded.
    Deny {
        /// Operator-readable reason.
        reason: String,
    },
    /// Candidate is outside the auto-approve scope and needs an
    /// out-of-band signature (SR-1).
    RequireApproval {
        /// Operator-readable reason.
        reason: String,
    },
}

/// Pluggable policy gate. Implementations must be deterministic and
/// side-effect free; the gate runs on every cycle and must add less
/// than 50ms of latency (NFR-3).
pub trait PolicyGate: Send + Sync {
    /// Evaluate `candidate_sbom` + `intent` against `parent.allowed_scope`.
    fn evaluate(
        &self,
        parent: &AgentNode,
        intent: &MutationIntent,
        candidate_sbom: &[String],
    ) -> PolicyDecision;
}

/// Default rule engine. Enforces the AllowedScope fields documented on
/// [`crate::intent::AllowedScope`].
#[derive(Debug, Default, Clone)]
pub struct DefaultPolicyGate;

impl PolicyGate for DefaultPolicyGate {
    fn evaluate(
        &self,
        parent: &AgentNode,
        intent: &MutationIntent,
        candidate_sbom: &[String],
    ) -> PolicyDecision {
        let scope = &parent.allowed_scope;

        // FR / SR-1: enforce generation depth cap.
        if let Some(max) = scope.max_generation
            && parent.generation + 1 > max
        {
            return PolicyDecision::Deny {
                reason: format!(
                    "max_generation {} exceeded (parent gen {})",
                    max, parent.generation
                ),
            };
        }

        // Detect new capabilities (anything in candidate not in parent).
        let parent_sbom: HashSet<&str> =
            parent.capability_sbom.iter().map(|s| s.as_str()).collect();
        let added: Vec<&str> = candidate_sbom
            .iter()
            .map(|s| s.as_str())
            .filter(|s| !parent_sbom.contains(s))
            .collect();

        // Cross-check additions against the explicit allow-lists.
        for cap in &added {
            if !capability_in_scope(cap, scope) {
                return PolicyDecision::RequireApproval {
                    reason: format!("capability {cap} outside allowed_scope"),
                };
            }
        }

        // Gated capabilities always require approval.
        for gated in &scope.gated_capabilities {
            if candidate_sbom.iter().any(|c| c == gated)
                || intent.constraints.iter().any(|c| c == gated)
            {
                return PolicyDecision::RequireApproval {
                    reason: format!("gated capability {gated} present"),
                };
            }
        }

        PolicyDecision::Approve
    }
}

fn capability_in_scope(capability: &str, scope: &AllowedScope) -> bool {
    let (kind, name) = match capability.split_once(':') {
        Some(parts) => parts,
        None => return false,
    };
    let list = match kind {
        "tool" => &scope.allowed_tools,
        "mcp" => &scope.allowed_mcp_servers,
        "delegate" => &scope.allowed_delegates,
        _ => return false,
    };
    list.iter().any(|allowed| name == allowed)
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
    use crate::artifact::AgentArtifact;
    use crate::node::NodeStatus;
    use rig_compose::AgentManifest;

    fn parent_with_scope(scope: AllowedScope, sbom: Vec<String>) -> AgentNode {
        let manifest = AgentManifest::from_yaml("name: root\ntools: []\n").unwrap();
        AgentNode {
            agent_id: "root".into(),
            parent_id: None,
            generation: 0,
            created_at: "2026-05-11T00:00:00Z".into(),
            mutation_intent: MutationIntent::new("genesis"),
            capability_sbom: sbom,
            eval_results: None,
            mutation_diff: String::new(),
            allowed_scope: scope,
            artifact: AgentArtifact::new(manifest),
            signer_public_key: String::new(),
            signature: String::new(),
            status: NodeStatus::Promoted,
            parent_agent_success: true,
            valid_parent: true,
            eval_stage: None,
        }
    }

    #[test]
    fn unchanged_sbom_is_approved() {
        let parent = parent_with_scope(AllowedScope::default(), vec!["tool:a".into()]);
        let gate = DefaultPolicyGate;
        assert_eq!(
            gate.evaluate(&parent, &MutationIntent::new("x"), &["tool:a".into()]),
            PolicyDecision::Approve
        );
    }

    #[test]
    fn out_of_scope_tool_requires_approval() {
        let scope = AllowedScope {
            allowed_tools: vec!["a".into()],
            ..Default::default()
        };
        let parent = parent_with_scope(scope, vec!["tool:a".into()]);
        let gate = DefaultPolicyGate;
        let decision = gate.evaluate(
            &parent,
            &MutationIntent::new("x"),
            &["tool:a".into(), "tool:b".into()],
        );
        assert!(matches!(decision, PolicyDecision::RequireApproval { .. }));
    }

    #[test]
    fn in_scope_addition_is_approved() {
        let scope = AllowedScope {
            allowed_tools: vec!["a".into(), "b".into()],
            ..Default::default()
        };
        let parent = parent_with_scope(scope, vec!["tool:a".into()]);
        let gate = DefaultPolicyGate;
        assert_eq!(
            gate.evaluate(
                &parent,
                &MutationIntent::new("x"),
                &["tool:a".into(), "tool:b".into()]
            ),
            PolicyDecision::Approve
        );
    }

    #[test]
    fn generation_cap_denies() {
        let scope = AllowedScope {
            max_generation: Some(0),
            ..Default::default()
        };
        let parent = parent_with_scope(scope, vec![]);
        let gate = DefaultPolicyGate;
        let decision = gate.evaluate(&parent, &MutationIntent::new("x"), &[]);
        assert!(matches!(decision, PolicyDecision::Deny { .. }));
    }
}
