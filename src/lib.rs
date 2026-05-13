//! # rig-veh
//!
//! Verifiable Evolutionary Hyperagent — **Git for cognition**.
//!
//! `rig-veh` adds a cryptographic, policy-gated evolution loop on top
//! of the `rig` ecosystem:
//!
//! - **Agent Node** — an immutable wrapper around
//!   [`rig_compose::AgentManifest`] hashed with SHA-256 and signed
//!   with Ed25519 ([`identity`], [`node`]).
//! - **Lineage DAG** — append-only ledger of every promoted and
//!   rejected agent ([`ledger`]).
//! - **Mutation Intent** — every candidate must declare its `why`
//!   ([`intent`]).
//! - **Policy Gate** — deterministic, LLM-free firewall against
//!   out-of-scope capability expansion ([`policy`]).
//! - **Evaluation Sandbox** — generic [`evaluator::Evaluator`] trait;
//!   a `rig-evals-rag` adapter ships behind feature `rag`.
//! - **Lifecycle runner** — [`graph::Veh`] drives the five-state
//!   spec lifecycle including a `WaitForInput`-style pause for SR-1
//!   multi-sig approval.
//!
//! ## Example
//!
//! ```no_run
//! use std::sync::Arc;
//! use ed25519_dalek::SigningKey;
//! use rand_core::OsRng;
//! use rig_compose::AgentManifest;
//! use rig_veh::{
//!     AgentArtifact, AllowedScope, CycleInputs, InMemoryLineage, MutationIntent,
//!     StubEvaluator, Veh,
//! };
//!
//! # async fn run() -> rig_veh::Result<()> {
//! let ledger = Arc::new(InMemoryLineage::new());
//! let evaluator = Arc::new(StubEvaluator::new(0.8));
//! let key = SigningKey::generate(&mut OsRng);
//! let veh = Veh::new(ledger.clone(), evaluator, key);
//!
//! let manifest = AgentManifest::from_yaml("name: root\ntools: []\n").map_err(|e| {
//!     rig_veh::Error::Canonical(e.to_string())
//! })?;
//! let root = veh
//!     .commit_root(AgentArtifact::new(manifest), AllowedScope::default(), "2026-05-11T00:00:00Z")
//!     .await?;
//!
//! let candidate_manifest =
//!     AgentManifest::from_yaml("name: v2\ntools: []\n").map_err(|e| {
//!         rig_veh::Error::Canonical(e.to_string())
//!     })?;
//! let _outcome = veh
//!     .run_cycle(CycleInputs {
//!         parent: &root,
//!         intent: MutationIntent::new("tighten_preamble"),
//!         candidate: AgentArtifact::new(candidate_manifest),
//!         allowed_scope: AllowedScope::default(),
//!         created_at: "2026-05-11T00:01:00Z".into(),
//!     })
//!     .await?;
//! # Ok(()) }
//! ```

#![deny(rust_2018_idioms)]
#![forbid(unsafe_code)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

pub mod artifact;
pub mod diff;
pub mod ensemble;
pub mod error;
pub mod evaluator;
pub mod evidence;
pub mod evolution;
pub mod graph;
pub mod identity;
pub mod intent;
pub mod ledger;
pub mod mutator;
pub mod node;
pub mod policy;
pub mod sandbox;
pub mod selector;

#[cfg(feature = "rag")]
pub mod rag_evaluator;

pub use artifact::{AgentArtifact, canonical_json};
pub use diff::manifest_diff;
pub use ensemble::{BestByMetric, EnsembleSelector};
pub use error::{Error, Result};
pub use evaluator::{
    CompositeEvaluator, EvalResult, EvalStage, Evaluator, StagedResult, StubEvaluator,
};
pub use evidence::{EVIDENCE_BUNDLE_VERSION, EvidenceBundle, OperatorApproval, PolicyVerdict};
pub use evolution::EvolutionDriver;
pub use graph::{CycleInputs, CycleOutcome, PendingApproval, Veh};
pub use identity::{CommitInputs, compute_agent_id, decode_verifying_key, sign_node, verify_node};
pub use intent::{AllowedScope, MutationIntent};
pub use ledger::{InMemoryLineage, LineageStore};
pub use mutator::{MutationContext, Mutator, StaticMutator};
pub use node::{AgentId, AgentNode, NodeStatus};
pub use policy::{DefaultPolicyGate, PolicyDecision, PolicyGate};
pub use sandbox::{NoopSandbox, Sandbox};
pub use selector::{
    Best, Latest, ParentSelector, Random, ScoreChildProportional, ScoreProportional,
};

#[cfg(feature = "jsonl-ledger")]
pub use ledger::JsonlLineage;

#[cfg(feature = "dot-export")]
pub use ledger::export_dot;

#[cfg(feature = "rag")]
pub use rag_evaluator::{ReportFactory, RetrievalEvaluator};
