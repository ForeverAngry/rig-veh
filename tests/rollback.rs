//! Integration tests for FR-6 (rollback preserves signatures).

#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use std::sync::Arc;

use ed25519_dalek::SigningKey;
use rand_core::OsRng;
use rig_compose::AgentManifest;
use rig_veh::{
    AgentArtifact, AllowedScope, CycleInputs, CycleOutcome, InMemoryLineage, LineageStore,
    MutationIntent, StubEvaluator, Veh, verify_node,
};

fn manifest(name: &str) -> AgentManifest {
    AgentManifest::from_yaml(&format!("name: {name}\ntools: []\n")).unwrap()
}

#[tokio::test]
async fn rollback_reverts_head_and_signatures_still_verify() {
    let ledger = Arc::new(InMemoryLineage::new());
    let evaluator = Arc::new(StubEvaluator::new(0.5));
    let key = SigningKey::generate(&mut OsRng);
    let veh = Veh::new(ledger.clone(), evaluator, key).with_strict_improvement(false);

    let root = veh
        .commit_root(
            AgentArtifact::new(manifest("root")),
            AllowedScope::default(),
            "2026-05-11T00:00:00Z",
        )
        .await
        .unwrap();

    let outcome = veh
        .run_cycle(CycleInputs {
            parent: &root,
            intent: MutationIntent::new("v1"),
            candidate: AgentArtifact::new(manifest("v1")),
            allowed_scope: AllowedScope::default(),
            created_at: "2026-05-11T00:01:00Z".into(),
        })
        .await
        .unwrap();
    let v1 = match outcome {
        CycleOutcome::Promoted { node, .. } => node,
        _ => panic!("expected Promoted"),
    };

    assert_eq!(ledger.head().await.unwrap(), Some(v1.agent_id.clone()));

    // Roll the head back to the root.
    veh.rollback_to(&root.agent_id).await.unwrap();
    assert_eq!(ledger.head().await.unwrap(), Some(root.agent_id.clone()));

    // Both nodes still verify cryptographically.
    verify_node(&root).unwrap();
    verify_node(&v1).unwrap();

    // v1 is still present in the DAG export — rollback is non-destructive.
    let dag = ledger.export_dag_json().await.unwrap();
    let ids: Vec<&str> = dag
        .get("nodes")
        .and_then(|v| v.as_array())
        .unwrap()
        .iter()
        .filter_map(|n| n.get("agent_id").and_then(|v| v.as_str()))
        .collect();
    assert!(ids.contains(&root.agent_id.as_str()));
    assert!(ids.contains(&v1.agent_id.as_str()));
}

#[tokio::test]
async fn rollback_to_unknown_id_fails() {
    let ledger = Arc::new(InMemoryLineage::new());
    let evaluator = Arc::new(StubEvaluator::new(0.5));
    let key = SigningKey::generate(&mut OsRng);
    let veh = Veh::new(ledger.clone(), evaluator, key);

    let err = veh.rollback_to("does-not-exist").await.unwrap_err();
    assert!(matches!(err, rig_veh::Error::NotFound(_)));
}

#[tokio::test]
async fn rollback_to_rejected_node_is_refused() {
    // Regression: `rollback_to` must refuse to move `head` to a node
    // whose status is `Rejected`. Otherwise the "head = most recently
    // promoted" invariant breaks and downstream readers think a
    // discarded candidate is the live agent.
    let ledger = Arc::new(InMemoryLineage::new());
    let evaluator = Arc::new(StubEvaluator::new(0.5));
    let key = SigningKey::generate(&mut OsRng);
    let veh = Veh::new(ledger.clone(), evaluator, key); // strict improvement on

    let root = veh
        .commit_root(
            AgentArtifact::new(manifest("root")),
            AllowedScope::default(),
            "2026-05-11T00:00:00Z",
        )
        .await
        .unwrap();

    // First promote v1 so we have a parent with recorded eval_results
    // (commit_root records none, so the strict-improvement check is
    // bypassed against the root).
    let outcome = veh
        .run_cycle(CycleInputs {
            parent: &root,
            intent: MutationIntent::new("v1"),
            candidate: AgentArtifact::new(manifest("v1")),
            allowed_scope: AllowedScope::default(),
            created_at: "2026-05-11T00:01:00Z".into(),
        })
        .await
        .unwrap();
    let v1 = match outcome {
        CycleOutcome::Promoted { node, .. } => node,
        _ => panic!("expected Promoted"),
    };

    // Score equal to parent v1 → rejected under strict-improvement.
    let outcome = veh
        .run_cycle(CycleInputs {
            parent: &v1,
            intent: MutationIntent::new("flat"),
            candidate: AgentArtifact::new(manifest("flat")),
            allowed_scope: AllowedScope::default(),
            created_at: "2026-05-11T00:02:00Z".into(),
        })
        .await
        .unwrap();
    let rejected = match outcome {
        CycleOutcome::Rejected { node, .. } => node,
        _ => panic!("expected Rejected"),
    };

    let err = veh.rollback_to(&rejected.agent_id).await.unwrap_err();
    assert!(matches!(err, rig_veh::Error::PolicyDenied(_)));
    // Head still points at v1 (the most recently promoted node).
    assert_eq!(ledger.head().await.unwrap(), Some(v1.agent_id.clone()));
}
