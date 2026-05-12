//! Open-ended evolution driver.
//!
//! [`EvolutionDriver`] wires the four runtime-agnostic primitives —
//! [`ParentSelector`], [`Mutator`], [`Sandbox`], and [`Veh`] — into a
//! single generation step. It mirrors the responsibilities of
//! HyperAgents'
//! [`generate_loop.py`](https://github.com/facebookresearch/HyperAgents/blob/main/generate_loop.py)
//! without committing the host to a specific runtime: Docker, Wasm,
//! and in-process executors all satisfy [`Sandbox`].
//!
//! ## Pipeline
//!
//! Each call to [`EvolutionDriver::run_generation`] runs:
//!
//! 1. `selector.select(ledger)` — pick the parent;
//! 2. `mutator.propose(parent, intent, context)` — propose a child
//!    artifact;
//! 3. `sandbox.enforce(candidate, scope)` — assert isolation;
//! 4. `veh.run_cycle(parent, intent, candidate, scope, created_at)` —
//!    evaluate, policy-gate, sign, and append;
//! 5. `sandbox.teardown()` — release sandbox resources, even on
//!    rejection.
//!
//! ## Example
//!
//! ```no_run
//! use std::sync::Arc;
//! use ed25519_dalek::SigningKey;
//! use rand_core::OsRng;
//! use rig_compose::AgentManifest;
//! use rig_veh::{
//!     AgentArtifact, AllowedScope, EvolutionDriver, InMemoryLineage, Latest,
//!     MutationContext, MutationIntent, NoopSandbox, StaticMutator, StubEvaluator, Veh,
//! };
//!
//! # async fn run() -> rig_veh::Result<()> {
//! let ledger = Arc::new(InMemoryLineage::new());
//! let evaluator = Arc::new(StubEvaluator::new(0.8));
//! let key = SigningKey::generate(&mut OsRng);
//! let veh = Veh::new(ledger.clone(), evaluator, key).with_strict_improvement(false);
//!
//! let root_manifest = AgentManifest::from_yaml("name: root\ntools: []\n")
//!     .map_err(|e| rig_veh::Error::Canonical(e.to_string()))?;
//! veh.commit_root(
//!     AgentArtifact::new(root_manifest),
//!     AllowedScope::default(),
//!     "2026-05-11T00:00:00Z",
//! ).await?;
//!
//! let candidate_manifest = AgentManifest::from_yaml("name: v2\ntools: []\n")
//!     .map_err(|e| rig_veh::Error::Canonical(e.to_string()))?;
//! let driver = EvolutionDriver::new(
//!     veh,
//!     ledger.clone(),
//!     Arc::new(Latest),
//!     Arc::new(StaticMutator::new(AgentArtifact::new(candidate_manifest))),
//!     Arc::new(NoopSandbox),
//! );
//!
//! let _outcome = driver
//!     .run_generation(
//!         MutationIntent::new("tighten_preamble"),
//!         AllowedScope::default(),
//!         MutationContext::new().with_iterations_left(2),
//!         "2026-05-11T00:01:00Z",
//!     )
//!     .await?;
//! # Ok(()) }
//! ```

use std::sync::Arc;

use crate::error::Result;
use crate::graph::{CycleInputs, CycleOutcome, Veh};
use crate::intent::{AllowedScope, MutationIntent};
use crate::ledger::LineageStore;
use crate::mutator::{MutationContext, Mutator};
use crate::sandbox::Sandbox;
use crate::selector::ParentSelector;

/// Wires the open-ended evolution pipeline.
pub struct EvolutionDriver {
    veh: Veh,
    ledger: Arc<dyn LineageStore>,
    selector: Arc<dyn ParentSelector>,
    mutator: Arc<dyn Mutator>,
    sandbox: Arc<dyn Sandbox>,
}

impl EvolutionDriver {
    /// Build a driver from its collaborators. `ledger` must be the
    /// same store the [`Veh`] was constructed with; the driver uses
    /// it to resolve the parent the selector chose.
    pub fn new(
        veh: Veh,
        ledger: Arc<dyn LineageStore>,
        selector: Arc<dyn ParentSelector>,
        mutator: Arc<dyn Mutator>,
        sandbox: Arc<dyn Sandbox>,
    ) -> Self {
        Self {
            veh,
            ledger,
            selector,
            mutator,
            sandbox,
        }
    }

    /// Borrow the underlying [`Veh`] for operations not covered by
    /// the driver (root commit, rollback, etc.).
    pub fn veh(&self) -> &Veh {
        &self.veh
    }

    /// Run one generation of the evolution loop. See the module-level
    /// docs for the exact step sequence.
    pub async fn run_generation(
        &self,
        intent: MutationIntent,
        allowed_scope: AllowedScope,
        context: MutationContext,
        created_at: impl Into<String>,
    ) -> Result<CycleOutcome> {
        let parent_id = self.selector.select(self.ledger.as_ref()).await?;
        let parent = self.ledger.get(&parent_id).await?;
        let candidate = self.mutator.propose(&parent, &intent, &context).await?;
        self.sandbox.enforce(&candidate, &allowed_scope).await?;
        let outcome = self
            .veh
            .run_cycle(CycleInputs {
                parent: &parent,
                intent,
                candidate,
                allowed_scope,
                created_at: created_at.into(),
            })
            .await;
        // Always tear down the sandbox even if the cycle failed; bubble
        // up the first error.
        let teardown = self.sandbox.teardown().await;
        let result = outcome?;
        teardown?;
        Ok(result)
    }
}
