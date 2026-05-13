//! VEH lifecycle runner.
//!
//! v0.1 implements the five-state machine from §4 of the spec as a
//! plain `async` function on [`Veh`]. The states map 1:1 to the spec:
//!
//! 1. `INTENT_GENERATION` — caller supplies a [`MutationIntent`].
//! 2. `CANDIDATE_SPAWNING` — caller supplies the candidate
//!    [`AgentArtifact`]; `Veh` computes the diff against the parent.
//! 3. `BENCHMARK_EVALUATION` — `Veh` calls the [`Evaluator`].
//! 4. `POLICY_GATE` — `Veh` calls the [`PolicyGate`]; on
//!    `RequireApproval` the cycle returns
//!    [`CycleOutcome::AwaitingApproval`] and the caller must invoke
//!    [`Veh::resume_with_approval`] with an out-of-band signing key.
//! 5. `COMMIT_OR_DISCARD` — on approval the candidate is signed and
//!    appended; on deny a `Rejected` node is appended as a negative
//!    cache entry.
//!
//! `graph-flow` integration is intentionally not part of v0.1 so the
//! library stays runtime-agnostic. A `graph-flow` feature can wrap
//! these same primitives as `Task` impls in a later release.

use std::sync::Arc;

use ed25519_dalek::SigningKey;
use tracing::{debug, info, warn};

use crate::artifact::AgentArtifact;
use crate::diff::manifest_diff;
use crate::error::{Error, Result};
use crate::evaluator::{EvalResult, Evaluator};
use crate::identity::{CommitInputs, sign_node};
use crate::intent::{AllowedScope, MutationIntent};
use crate::ledger::LineageStore;
use crate::node::{AgentNode, NodeStatus};
use crate::policy::{DefaultPolicyGate, PolicyDecision, PolicyGate};

/// Inputs to one evolutionary cycle.
pub struct CycleInputs<'a> {
    /// Parent node the candidate should descend from.
    pub parent: &'a AgentNode,
    /// Mutation intent that produced the candidate.
    pub intent: MutationIntent,
    /// Candidate artifact under evaluation.
    pub candidate: AgentArtifact,
    /// Scope inherited by descendants of the candidate. Usually the
    /// parent's `allowed_scope` carried forward verbatim.
    pub allowed_scope: AllowedScope,
    /// RFC 3339 timestamp the host wants stamped on the node.
    pub created_at: String,
}

/// State carried across a paused `RequireApproval` cycle.
///
/// Intentionally **not** `Clone`: [`Veh::resume_with_approval`] consumes
/// the value to take sole ownership of the captured candidate. Letting
/// callers clone it would either leak state or — worse — cause
/// `resume_with_approval` to fail at runtime trying to unwrap an
/// `Arc` with multiple owners.
pub struct PendingApproval {
    inner: PendingInner,
}

struct PendingInner {
    parent_id: Option<String>,
    parent_generation: u32,
    intent: MutationIntent,
    candidate: AgentArtifact,
    allowed_scope: AllowedScope,
    created_at: String,
    eval_results: serde_json::Value,
    mutation_diff: String,
    decision_reason: String,
}

impl PendingApproval {
    /// Operator-readable reason the gate paused the cycle.
    pub fn reason(&self) -> &str {
        &self.inner.decision_reason
    }
}

/// Outcome of a single [`Veh::run_cycle`] call.
pub enum CycleOutcome {
    /// Candidate was promoted; the new head is `node.agent_id`.
    Promoted {
        /// The signed node now in the ledger.
        node: AgentNode,
        /// Evaluation result attached to the node.
        eval: EvalResult,
    },
    /// Candidate was rejected by the gate or scored below the parent;
    /// the rejected node was appended as a negative-cache entry.
    Rejected {
        /// The signed (but `Rejected`) node now in the ledger.
        node: AgentNode,
        /// Operator-readable reason.
        reason: String,
    },
    /// Candidate is out-of-scope and needs an out-of-band signature.
    /// The caller resumes via [`Veh::resume_with_approval`].
    AwaitingApproval(PendingApproval),
}

/// Lifecycle runner.
///
/// `Veh` borrows its collaborators behind `Arc` so it can be cloned
/// freely and shared between async tasks. None of its operations hold
/// a lock across an `.await`.
pub struct Veh {
    ledger: Arc<dyn LineageStore>,
    evaluator: Arc<dyn Evaluator>,
    policy: Arc<dyn PolicyGate>,
    signing_key: SigningKey,
    /// If `true`, a candidate whose score is not strictly greater
    /// than the parent is treated as a rejection (FR-5).
    pub require_strict_improvement: bool,
}

impl Veh {
    /// Build a runner with the default [`DefaultPolicyGate`].
    pub fn new(
        ledger: Arc<dyn LineageStore>,
        evaluator: Arc<dyn Evaluator>,
        signing_key: SigningKey,
    ) -> Self {
        Self {
            ledger,
            evaluator,
            policy: Arc::new(DefaultPolicyGate),
            signing_key,
            require_strict_improvement: true,
        }
    }

    /// Override the policy gate.
    pub fn with_policy(mut self, policy: Arc<dyn PolicyGate>) -> Self {
        self.policy = policy;
        self
    }

    /// Toggle the FR-5 strict-improvement rule.
    pub fn with_strict_improvement(mut self, strict: bool) -> Self {
        self.require_strict_improvement = strict;
        self
    }

    /// Sign and append the evolutionary root. Returns the committed
    /// node so callers can record its `agent_id`.
    pub async fn commit_root(
        &self,
        artifact: AgentArtifact,
        allowed_scope: AllowedScope,
        created_at: impl Into<String>,
    ) -> Result<AgentNode> {
        let intent = MutationIntent::new("genesis");
        let created = created_at.into();
        let inputs = CommitInputs {
            parent_id: None,
            generation: 0,
            created_at: &created,
            mutation_intent: &intent,
            mutation_diff: "",
            allowed_scope: &allowed_scope,
            eval_results: None,
            artifact: &artifact,
            status: NodeStatus::Promoted,
            parent_agent_success: true,
            valid_parent: true,
            eval_stage: None,
        };
        let node = sign_node(&inputs, &self.signing_key)?;
        self.ledger.append(node.clone()).await?;
        info!(agent_id = %node.agent_id, "committed root agent");
        Ok(node)
    }

    /// Execute one evolutionary cycle.
    pub async fn run_cycle(&self, inputs: CycleInputs<'_>) -> Result<CycleOutcome> {
        let CycleInputs {
            parent,
            intent,
            candidate,
            allowed_scope,
            created_at,
        } = inputs;

        // Step 1: diff.
        let mutation_diff = manifest_diff(&parent.artifact, &candidate)?;

        // Step 2: evaluation (BENCHMARK_EVALUATION).
        let eval = self.evaluator.evaluate(&candidate).await?;
        debug!(score = eval.score, "candidate evaluated");

        // Step 3: FR-5 strict improvement check.
        let parent_score = parent
            .eval_results
            .as_ref()
            .and_then(|v| v.get("score"))
            .and_then(|v| v.as_f64());
        if self.require_strict_improvement
            && let Some(parent_score) = parent_score
            && eval.score <= parent_score
        {
            let reason = format!(
                "candidate score {} did not exceed parent {}",
                eval.score, parent_score
            );
            warn!(%reason, "rejecting candidate");
            let node = self
                .commit_rejected(
                    parent,
                    &intent,
                    &candidate,
                    &allowed_scope,
                    &created_at,
                    &mutation_diff,
                    &eval,
                )
                .await?;
            return Ok(CycleOutcome::Rejected { node, reason });
        }

        // Step 4: POLICY_GATE.
        let sbom = candidate.capability_sbom();
        let decision = self.policy.evaluate(parent, &intent, &sbom);

        match decision {
            PolicyDecision::Approve => {
                let node = self
                    .commit_promoted(
                        parent,
                        &intent,
                        &candidate,
                        &allowed_scope,
                        &created_at,
                        &mutation_diff,
                        &eval,
                        &self.signing_key,
                    )
                    .await?;
                Ok(CycleOutcome::Promoted { node, eval })
            }
            PolicyDecision::Deny { reason } => {
                warn!(%reason, "policy denied candidate");
                let node = self
                    .commit_rejected(
                        parent,
                        &intent,
                        &candidate,
                        &allowed_scope,
                        &created_at,
                        &mutation_diff,
                        &eval,
                    )
                    .await?;
                Ok(CycleOutcome::Rejected { node, reason })
            }
            PolicyDecision::RequireApproval { reason } => {
                info!(%reason, "candidate requires out-of-band approval");
                let eval_value = serde_json::to_value(&eval)?;
                let pending = PendingApproval {
                    inner: PendingInner {
                        parent_id: Some(parent.agent_id.clone()),
                        parent_generation: parent.generation,
                        intent,
                        candidate,
                        allowed_scope,
                        created_at,
                        eval_results: eval_value,
                        mutation_diff,
                        decision_reason: reason,
                    },
                };
                Ok(CycleOutcome::AwaitingApproval(pending))
            }
        }
    }

    /// Resume a paused cycle by supplying an out-of-band approval key.
    ///
    /// The approval key may be the same key the runner was constructed
    /// with (single-signer mode) or a distinct out-of-band key — the
    /// API already accepts either. The signed node records *that* key
    /// as its signer, so the audit trail can prove which authority
    /// approved the out-of-scope mutation.
    pub async fn resume_with_approval(
        &self,
        pending: PendingApproval,
        approval_key: &SigningKey,
    ) -> Result<AgentNode> {
        let inner = pending.inner;

        let eval_results = Some(&inner.eval_results);
        let parent_id_owned = inner.parent_id;
        let inputs = CommitInputs {
            parent_id: parent_id_owned.as_ref(),
            generation: inner.parent_generation + 1,
            created_at: &inner.created_at,
            mutation_intent: &inner.intent,
            mutation_diff: &inner.mutation_diff,
            allowed_scope: &inner.allowed_scope,
            eval_results,
            artifact: &inner.candidate,
            status: NodeStatus::Promoted,
            parent_agent_success: true,
            valid_parent: true,
            eval_stage: None,
        };
        let node = sign_node(&inputs, approval_key)?;
        self.ledger.append(node.clone()).await?;
        info!(agent_id = %node.agent_id, "promoted candidate after approval");
        Ok(node)
    }

    /// Revert the head pointer to a previous generation (FR-6).
    ///
    /// The target node must exist, must verify against its recorded
    /// signature, and must have status [`NodeStatus::Promoted`].
    /// Pointing `head` at a `Rejected` node would violate the
    /// "head = most recently promoted" invariant the rest of the
    /// pipeline relies on.
    pub async fn rollback_to(&self, agent_id: &str) -> Result<()> {
        let target = self.ledger.get(&agent_id.to_string()).await?;
        crate::identity::verify_node(&target)?;
        if !matches!(target.status, NodeStatus::Promoted) {
            return Err(Error::PolicyDenied(format!(
                "cannot roll back to non-promoted node {agent_id}"
            )));
        }
        self.ledger.set_head(&agent_id.to_string()).await
    }

    /// Append a signed rollback event that reverts the active head to
    /// `target_id`'s artifact while preserving history.
    ///
    /// Unlike [`Veh::rollback_to`] (which only moves the head pointer),
    /// this method emits a brand-new signed [`AgentNode`] whose parent
    /// is the current head and whose [`MutationIntent`] records the
    /// rollback target via [`MutationIntent::rollback`]. The new node's
    /// artifact mirrors `target_id`'s artifact, so callers can roll
    /// forward, roll back, and keep an auditable signed trail of every
    /// transition.
    ///
    /// The target must verify and must be `Promoted`. The new node is
    /// signed with the runner's signing key.
    pub async fn commit_rollback(
        &self,
        target_id: &str,
        created_at: impl Into<String>,
    ) -> Result<AgentNode> {
        let target = self.ledger.get(&target_id.to_string()).await?;
        crate::identity::verify_node(&target)?;
        if !matches!(target.status, NodeStatus::Promoted) {
            return Err(Error::PolicyDenied(format!(
                "cannot roll back to non-promoted node {target_id}"
            )));
        }

        let parent_id = self
            .ledger
            .head()
            .await?
            .ok_or_else(|| Error::Ledger("ledger has no head; nothing to roll back".into()))?;
        let parent = self.ledger.get(&parent_id).await?;

        let intent = MutationIntent::rollback(target_id);
        let allowed_scope = target.allowed_scope.clone();
        let created = created_at.into();
        let inputs = CommitInputs {
            parent_id: Some(&parent.agent_id),
            generation: parent.generation + 1,
            created_at: &created,
            mutation_intent: &intent,
            mutation_diff: "",
            allowed_scope: &allowed_scope,
            eval_results: None,
            artifact: &target.artifact,
            status: NodeStatus::Promoted,
            parent_agent_success: true,
            valid_parent: true,
            eval_stage: None,
        };
        let node = sign_node(&inputs, &self.signing_key)?;
        self.ledger.append(node.clone()).await?;
        Ok(node)
    }

    #[allow(clippy::too_many_arguments)]
    async fn commit_promoted(
        &self,
        parent: &AgentNode,
        intent: &MutationIntent,
        candidate: &AgentArtifact,
        allowed_scope: &AllowedScope,
        created_at: &str,
        mutation_diff: &str,
        eval: &EvalResult,
        signing_key: &SigningKey,
    ) -> Result<AgentNode> {
        let eval_value = serde_json::to_value(eval)?;
        let parent_id = parent.agent_id.clone();
        let inputs = CommitInputs {
            parent_id: Some(&parent_id),
            generation: parent.generation + 1,
            created_at,
            mutation_intent: intent,
            mutation_diff,
            allowed_scope,
            eval_results: Some(&eval_value),
            artifact: candidate,
            status: NodeStatus::Promoted,
            parent_agent_success: true,
            valid_parent: true,
            eval_stage: None,
        };
        let node = sign_node(&inputs, signing_key)?;
        self.ledger.append(node.clone()).await?;
        Ok(node)
    }

    #[allow(clippy::too_many_arguments)]
    async fn commit_rejected(
        &self,
        parent: &AgentNode,
        intent: &MutationIntent,
        candidate: &AgentArtifact,
        allowed_scope: &AllowedScope,
        created_at: &str,
        mutation_diff: &str,
        eval: &EvalResult,
    ) -> Result<AgentNode> {
        let eval_value = serde_json::to_value(eval)?;
        let parent_id = parent.agent_id.clone();
        let inputs = CommitInputs {
            parent_id: Some(&parent_id),
            generation: parent.generation + 1,
            created_at,
            mutation_intent: intent,
            mutation_diff,
            allowed_scope,
            eval_results: Some(&eval_value),
            artifact: candidate,
            status: NodeStatus::Rejected,
            parent_agent_success: true,
            valid_parent: false,
            eval_stage: None,
        };
        let node = sign_node(&inputs, &self.signing_key)?;
        self.ledger.append(node.clone()).await?;
        Ok(node)
    }
}
