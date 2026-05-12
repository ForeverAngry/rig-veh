//! Lineage node — the metadata record stored in the ledger DAG.
//!
//! Matches the schema in §5 of the VEH specification exactly. The
//! `agent_id` is the cryptographic primary key; `parent_id` links a
//! node to its immediate ancestor (omitted for the root). The
//! `signature` field binds the canonical artifact bytes to an
//! authorised signing key — see [`crate::identity`] for the verifier.

use serde::{Deserialize, Serialize};

use crate::artifact::AgentArtifact;
use crate::evaluator::EvalStage;
use crate::intent::{AllowedScope, MutationIntent};

/// Hex-encoded SHA-256 agent identifier.
pub type AgentId = String;

/// One node in the lineage DAG.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentNode {
    /// Cryptographic primary key (`SHA256(parent_id || ts || manifest || creator_sig)`).
    pub agent_id: AgentId,
    /// Immediate ancestor. `None` only for the evolutionary root.
    #[serde(default)]
    pub parent_id: Option<AgentId>,
    /// Depth in the evolutionary tree; the root is `0`.
    pub generation: u32,
    /// RFC 3339 timestamp at which the node was committed to the ledger.
    pub created_at: String,
    /// Mutation intent that produced this node. Required for non-root
    /// nodes; a synthetic "genesis" intent is acceptable for the root.
    pub mutation_intent: MutationIntent,
    /// Stable, sorted SBOM derived from the artifact's manifest.
    pub capability_sbom: Vec<String>,
    /// Evaluator output. `None` is permitted only for the root.
    #[serde(default)]
    pub eval_results: Option<serde_json::Value>,
    /// Unified-diff string of the manifest YAML relative to the parent.
    /// Empty for the root.
    #[serde(default)]
    pub mutation_diff: String,
    /// Governance scope inherited by descendants.
    pub allowed_scope: AllowedScope,
    /// The artifact this node refers to.
    pub artifact: AgentArtifact,
    /// Hex-encoded Ed25519 public key of the signer.
    pub signer_public_key: String,
    /// Hex-encoded Ed25519 signature over the canonical artifact bytes
    /// concatenated with the canonical metadata bytes.
    pub signature: String,
    /// marks a node that was discarded by the policy gate or the
    /// evaluator. Discarded nodes are still appended so the system
    /// remembers not to re-propose the same mutation.
    #[serde(default)]
    pub status: NodeStatus,
    /// Whether the mutation resulted in a valid, evaluable artifact.
    #[serde(default)]
    pub parent_agent_success: bool,
    /// Whether this node is eligible to be selected as a parent by selectors.
    #[serde(default)]
    pub valid_parent: bool,
    /// Which stage promoted this candidate in a staged evaluation.
    #[serde(default)]
    pub eval_stage: Option<EvalStage>,
}

/// Lifecycle status of a lineage node.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum NodeStatus {
    /// Promoted candidate: passed evaluation + policy gate, signed.
    #[default]
    Promoted,
    /// Candidate generated but rejected. Body is preserved as a
    /// negative-cache entry; `signature` is still over the canonical
    /// bytes so the negative cache itself is tamper-evident.
    Rejected,
}
