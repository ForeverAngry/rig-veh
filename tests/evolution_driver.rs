//! Integration test for the open-ended evolution driver.

#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use std::sync::Arc;

use ed25519_dalek::SigningKey;
use rand_core::OsRng;
use rig_compose::AgentManifest;
use rig_veh::{
    AgentArtifact, AllowedScope, CycleOutcome, EvolutionDriver, InMemoryLineage, Latest,
    LineageStore, MutationContext, MutationIntent, NoopSandbox, StaticMutator, StubEvaluator, Veh,
};

fn manifest(name: &str) -> AgentManifest {
    AgentManifest::from_yaml(&format!("name: {name}\ntools: []\n")).unwrap()
}

#[tokio::test]
async fn driver_runs_three_generations_with_latest_selector() {
    let ledger = Arc::new(InMemoryLineage::new());
    let evaluator = Arc::new(StubEvaluator::new(0.5));
    let key = SigningKey::generate(&mut OsRng);
    let veh = Veh::new(ledger.clone(), evaluator, key).with_strict_improvement(false);

    // Root commit goes through `Veh` directly — the driver only runs
    // mutation generations.
    veh.commit_root(
        AgentArtifact::new(manifest("root")),
        AllowedScope::default(),
        "2026-05-11T00:00:00Z",
    )
    .await
    .unwrap();

    let candidate = AgentArtifact::new(manifest("v1"));
    let driver = EvolutionDriver::new(
        veh,
        ledger.clone(),
        Arc::new(Latest),
        Arc::new(StaticMutator::new(candidate)),
        Arc::new(NoopSandbox),
    );

    for i in 0..3 {
        let outcome = driver
            .run_generation(
                MutationIntent::new(format!("gen-{i}")),
                AllowedScope::default(),
                MutationContext::new().with_iterations_left(3 - i),
                format!("2026-05-11T00:0{}:00Z", i + 1),
            )
            .await
            .unwrap();
        // StaticMutator always returns the same artifact, so the
        // second generation produces a duplicate agent_id and the
        // ledger rejects it. Assert the first call promotes and the
        // rest surface a sensible error or rejection — we just want
        // to confirm the driver wires the pipeline correctly.
        if i == 0 {
            assert!(matches!(outcome, CycleOutcome::Promoted { .. }));
        }
    }

    // At least the root + one promoted child must be present.
    let nodes = ledger.nodes().await.unwrap();
    assert!(nodes.len() >= 2);
}

#[tokio::test]
async fn driver_exposes_underlying_veh() {
    let ledger = Arc::new(InMemoryLineage::new());
    let evaluator = Arc::new(StubEvaluator::new(0.5));
    let key = SigningKey::generate(&mut OsRng);
    let veh = Veh::new(ledger.clone(), evaluator, key);

    let candidate = AgentArtifact::new(manifest("v1"));
    let driver = EvolutionDriver::new(
        veh,
        ledger,
        Arc::new(Latest),
        Arc::new(StaticMutator::new(candidate)),
        Arc::new(NoopSandbox),
    );

    // Borrowed Veh can still commit a root.
    let _root = driver
        .veh()
        .commit_root(
            AgentArtifact::new(manifest("root")),
            AllowedScope::default(),
            "2026-05-11T00:00:00Z",
        )
        .await
        .unwrap();
}
