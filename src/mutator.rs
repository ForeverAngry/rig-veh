//! Candidate-artifact generation.
//!
//! Today [`Veh::run_cycle`](crate::Veh::run_cycle) accepts an
//! already-built [`AgentArtifact`] candidate. Meta's HyperAgents
//! reference implementation goes one step further: a `MetaAgent`
//! *produces* the next candidate by editing the parent
//! ([meta_agent.py](https://github.com/facebookresearch/HyperAgents/blob/main/meta_agent.py)).
//!
//! [`Mutator`] is the VEH-side seam for that capability. It is
//! intentionally LLM-agnostic; hosts wire their preferred provider
//! (rig agent, external service, deterministic template, etc.). The
//! trait is read-only with respect to the ledger and never mutates
//! [`AgentNode`] state directly — the host pipes the result back
//! through [`Veh::run_cycle`](crate::Veh::run_cycle) for signing and
//! policy review.

use async_trait::async_trait;

use crate::artifact::AgentArtifact;
use crate::error::Result;
use crate::intent::MutationIntent;
use crate::node::AgentNode;

/// Optional advisory context the driver hands to a [`Mutator`] when
/// proposing a candidate. Mirrors HyperAgents'
/// `iterations_left` signal passed to the meta agent
/// ([generate_loop.py](https://github.com/facebookresearch/HyperAgents/blob/main/generate_loop.py)),
/// generalised so hosts can pass a wall-clock budget or arbitrary
/// metadata.
///
/// All fields are optional — implementations should treat `None` as
/// "no constraint" and degrade gracefully.
#[derive(Debug, Clone, Default)]
pub struct MutationContext {
    /// Remaining generations the host expects to run after this one.
    /// `None` means open-ended.
    pub iterations_left: Option<u32>,
    /// Free-form metadata for host-specific signals (e.g. budget,
    /// deadline, target metric). Mutators may ignore it.
    pub metadata: serde_json::Value,
}

impl MutationContext {
    /// Build an empty context.
    pub fn new() -> Self {
        Self::default()
    }

    /// Builder-style helper.
    pub fn with_iterations_left(mut self, iterations: u32) -> Self {
        self.iterations_left = Some(iterations);
        self
    }

    /// Attach arbitrary host metadata.
    pub fn with_metadata(mut self, metadata: serde_json::Value) -> Self {
        self.metadata = metadata;
        self
    }
}

/// Produces candidate artifacts for the evolution loop.
#[async_trait]
pub trait Mutator: Send + Sync {
    /// Propose a child artifact for `parent` that satisfies `intent`.
    ///
    /// `context` carries host-level advisories (iterations left,
    /// budget metadata). Implementations may ignore it.
    ///
    /// Implementations must be deterministic given the same inputs
    /// *if* they want signature-stable mutations across replays.
    async fn propose(
        &self,
        parent: &AgentNode,
        intent: &MutationIntent,
        context: &MutationContext,
    ) -> Result<AgentArtifact>;
}

/// Deterministic mutator that returns a caller-supplied artifact
/// regardless of parent/intent. Useful for tests and for hosts that
/// generate candidates out-of-band and just want a [`Mutator`] to
/// plug into a generic evolution driver.
#[derive(Clone)]
pub struct StaticMutator {
    artifact: AgentArtifact,
}

impl StaticMutator {
    /// Construct a mutator that always proposes `artifact`.
    pub fn new(artifact: AgentArtifact) -> Self {
        Self { artifact }
    }
}

#[async_trait]
impl Mutator for StaticMutator {
    async fn propose(
        &self,
        _parent: &AgentNode,
        _intent: &MutationIntent,
        _context: &MutationContext,
    ) -> Result<AgentArtifact> {
        Ok(self.artifact.clone())
    }
}
