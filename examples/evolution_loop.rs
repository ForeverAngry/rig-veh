//! End-to-end open-ended evolution loop using [`EvolutionDriver`].
//!
//! Demonstrates how to wire a [`ParentSelector`], [`Mutator`], and
//! [`Sandbox`] together to run a multi-generation search. The
//! `StaticMutator` here is a placeholder — real hosts plug an
//! LLM-backed mutator that edits the parent manifest.
//!
//! ```sh
//! cargo run --example evolution_loop
//! ```

use std::sync::Arc;

use ed25519_dalek::SigningKey;
use rand_core::OsRng;
use rig_compose::AgentManifest;
use rig_veh::{
    AgentArtifact, AllowedScope, CycleOutcome, EvolutionDriver, InMemoryLineage, LineageStore,
    MutationContext, MutationIntent, NoopSandbox, ScoreChildProportional, StaticMutator,
    StubEvaluator, Veh,
};

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info,rig_veh=debug")),
        )
        .init();

    let ledger = Arc::new(InMemoryLineage::new());
    let evaluator = Arc::new(StubEvaluator::new(0.7));
    let key = SigningKey::generate(&mut OsRng);
    let veh = Veh::new(ledger.clone(), evaluator, key).with_strict_improvement(false);

    let root = AgentArtifact::new(AgentManifest::from_yaml("name: root\ntools: []\n")?);
    veh.commit_root(root, AllowedScope::default(), "2026-05-11T00:00:00Z")
        .await?;

    // Production hosts replace `StaticMutator` with an LLM-backed
    // implementation that edits the parent manifest.
    let candidate = AgentArtifact::new(AgentManifest::from_yaml("name: v1\ntools: []\n")?);
    let driver = EvolutionDriver::new(
        veh,
        ledger.clone(),
        Arc::new(ScoreChildProportional::with_seed(7)),
        Arc::new(StaticMutator::new(candidate)),
        Arc::new(NoopSandbox),
    );

    let outcome = driver
        .run_generation(
            MutationIntent::new("explore_branch"),
            AllowedScope::default(),
            MutationContext::new().with_iterations_left(1),
            "2026-05-11T00:01:00Z",
        )
        .await?;

    match outcome {
        CycleOutcome::Promoted { node, eval } => {
            tracing::info!(
                agent = %node.agent_id,
                generation = node.generation,
                score = eval.score,
                "promoted candidate"
            );
        }
        CycleOutcome::Rejected { node, reason } => {
            tracing::warn!(agent = %node.agent_id, %reason, "candidate rejected");
        }
        CycleOutcome::AwaitingApproval(pending) => {
            tracing::warn!(reason = pending.reason(), "candidate awaiting approval");
        }
    }

    let nodes = ledger.nodes().await?;
    tracing::info!(count = nodes.len(), "ledger size");

    Ok(())
}
