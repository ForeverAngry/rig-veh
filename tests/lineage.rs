//! Integration tests for FR-2 (lineage DAG) and FR-6 (rollback).

#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use std::sync::Arc;

use ed25519_dalek::SigningKey;
use rand_core::OsRng;
use rig_compose::AgentManifest;
use rig_veh::{
    AgentArtifact, AllowedScope, CycleInputs, CycleOutcome, InMemoryLineage, LineageStore,
    MutationIntent, StubEvaluator, Veh,
};

fn manifest(name: &str) -> AgentManifest {
    AgentManifest::from_yaml(&format!("name: {name}\ntools: []\n")).unwrap()
}

#[tokio::test]
async fn promotes_three_generations_and_tracks_head() {
    let ledger = Arc::new(InMemoryLineage::new());
    let mut evaluator_scores = [0.5_f64, 0.6, 0.7, 0.8].into_iter();
    // First commit_root sees the parent has no score, so any score promotes.
    let first = evaluator_scores.next().unwrap();
    let evaluator = Arc::new(StubEvaluator::new(first));
    let key = SigningKey::generate(&mut OsRng);
    let veh = Veh::new(ledger.clone(), evaluator, key);

    let root = veh
        .commit_root(
            AgentArtifact::new(manifest("root")),
            AllowedScope::default(),
            "2026-05-11T00:00:00Z",
        )
        .await
        .unwrap();

    assert_eq!(ledger.head().await.unwrap(), Some(root.agent_id.clone()));

    let mut parent = root;
    for (i, score) in evaluator_scores.enumerate() {
        let evaluator = Arc::new(StubEvaluator::new(score));
        let key = SigningKey::generate(&mut OsRng);
        let veh = Veh::new(ledger.clone(), evaluator, key);
        let outcome = veh
            .run_cycle(CycleInputs {
                parent: &parent,
                intent: MutationIntent::new(format!("gen-{}", i + 1)),
                candidate: AgentArtifact::new(manifest(&format!("v{}", i + 1))),
                allowed_scope: AllowedScope::default(),
                created_at: format!("2026-05-11T00:0{}:00Z", i + 1),
            })
            .await
            .unwrap();
        match outcome {
            CycleOutcome::Promoted { node, .. } => {
                assert_eq!(node.generation, parent.generation + 1);
                assert_eq!(node.parent_id.as_deref(), Some(parent.agent_id.as_str()));
                parent = node;
            }
            other => panic!(
                "expected Promoted, got something else: {:?}",
                outcome_kind(&other)
            ),
        }
    }

    assert_eq!(parent.generation, 3);
    assert_eq!(ledger.head().await.unwrap(), Some(parent.agent_id.clone()));

    let dag = ledger.export_dag_json().await.unwrap();
    let nodes = dag.get("nodes").and_then(|v| v.as_array()).unwrap();
    assert_eq!(nodes.len(), 4); // root + 3 candidates
}

#[tokio::test]
async fn strict_improvement_rejects_regression() {
    let ledger = Arc::new(InMemoryLineage::new());
    let evaluator = Arc::new(StubEvaluator::new(0.9));
    let key = SigningKey::generate(&mut OsRng);
    let veh = Veh::new(ledger.clone(), evaluator, key);

    let root = veh
        .commit_root(
            AgentArtifact::new(manifest("root")),
            AllowedScope::default(),
            "2026-05-11T00:00:00Z",
        )
        .await
        .unwrap();

    // Now wire a weaker evaluator and confirm the candidate is rejected.
    let weak = Arc::new(StubEvaluator::new(0.1));
    let key2 = SigningKey::generate(&mut OsRng);
    let veh2 = Veh::new(ledger.clone(), weak, key2);

    // Seed parent with its own score so FR-5 fires.
    let mut seeded_parent = root.clone();
    seeded_parent.eval_results = Some(serde_json::json!({"score": 0.9, "details": null}));

    let outcome = veh2
        .run_cycle(CycleInputs {
            parent: &seeded_parent,
            intent: MutationIntent::new("regression"),
            candidate: AgentArtifact::new(manifest("worse")),
            allowed_scope: AllowedScope::default(),
            created_at: "2026-05-11T00:01:00Z".into(),
        })
        .await
        .unwrap();

    match outcome {
        CycleOutcome::Rejected { node, reason } => {
            assert_eq!(node.status, rig_veh::NodeStatus::Rejected);
            assert!(reason.contains("did not exceed"));
        }
        other => panic!("expected Rejected, got {:?}", outcome_kind(&other)),
    }

    // Head still points at the (real) root since the rejection didn't promote.
    assert_eq!(ledger.head().await.unwrap(), Some(root.agent_id));
}

fn outcome_kind(o: &CycleOutcome) -> &'static str {
    match o {
        CycleOutcome::Promoted { .. } => "Promoted",
        CycleOutcome::Rejected { .. } => "Rejected",
        CycleOutcome::AwaitingApproval(_) => "AwaitingApproval",
    }
}
