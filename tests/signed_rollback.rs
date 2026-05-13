//! Integration coverage for signed rollback events.
//!
//! `Veh::commit_rollback` appends a new signed `AgentNode` whose
//! `MutationIntent::rollback_target` records the original target.
//! History is never rewritten, every transition stays auditable, and
//! tampering with a historical node breaks its signature.
#![allow(
    clippy::unwrap_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::expect_used
)]

use std::sync::Arc;

use ed25519_dalek::SigningKey;
use rand_core::OsRng;
use rig_compose::AgentManifest;
use rig_veh::{
    AgentArtifact, AllowedScope, CycleInputs, CycleOutcome, InMemoryLineage, LineageStore,
    MutationIntent, NodeStatus, StubEvaluator, Veh, verify_node,
};

fn manifest(name: &str) -> AgentManifest {
    AgentManifest::from_yaml(&format!("name: {name}\ntools: []\n")).unwrap()
}

#[tokio::test]
async fn commit_rollback_appends_signed_node_and_preserves_history() {
    let ledger = Arc::new(InMemoryLineage::new());
    let evaluator = Arc::new(StubEvaluator::new(0.7));
    let key = SigningKey::generate(&mut OsRng);
    let veh = Veh::new(ledger.clone(), evaluator, key).with_strict_improvement(false);

    let root = veh
        .commit_root(
            AgentArtifact::new(manifest("root")),
            AllowedScope::default(),
            "2026-05-21T00:00:00Z",
        )
        .await
        .unwrap();

    let v1 = match veh
        .run_cycle(CycleInputs {
            parent: &root,
            intent: MutationIntent::new("v1"),
            candidate: AgentArtifact::new(manifest("v1")),
            allowed_scope: AllowedScope::default(),
            created_at: "2026-05-21T00:01:00Z".into(),
        })
        .await
        .unwrap()
    {
        CycleOutcome::Promoted { node, .. } => node,
        _ => panic!("expected Promoted"),
    };

    // Append a signed rollback to `root`.
    let rolled = veh
        .commit_rollback(&root.agent_id, "2026-05-21T00:02:00Z")
        .await
        .unwrap();

    // The rollback node is itself signed and verifies.
    verify_node(&rolled).unwrap();
    assert_eq!(rolled.status, NodeStatus::Promoted);
    assert_eq!(rolled.parent_id.as_deref(), Some(v1.agent_id.as_str()));
    assert_eq!(rolled.generation, v1.generation + 1);
    assert_eq!(
        rolled.mutation_intent.rollback_target(),
        Some(root.agent_id.as_str()),
    );
    // The rollback restores the target's artifact bytes.
    assert_eq!(
        rolled.artifact.canonical_bytes().unwrap(),
        root.artifact.canonical_bytes().unwrap(),
    );

    // Head advanced to the rollback node — not back to `root` — so the
    // ledger reflects the additive event.
    assert_eq!(ledger.head().await.unwrap(), Some(rolled.agent_id.clone()));

    // All prior nodes are still present and verify.
    for node in [&root, &v1] {
        let reloaded = ledger.get(&node.agent_id).await.unwrap();
        verify_node(&reloaded).unwrap();
    }

    // DAG export shows all three nodes — no history was rewritten.
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
    assert!(ids.contains(&rolled.agent_id.as_str()));
}

#[tokio::test]
async fn commit_rollback_refuses_unknown_target() {
    let ledger = Arc::new(InMemoryLineage::new());
    let evaluator = Arc::new(StubEvaluator::new(0.7));
    let key = SigningKey::generate(&mut OsRng);
    let veh = Veh::new(ledger.clone(), evaluator, key);

    veh.commit_root(
        AgentArtifact::new(manifest("root")),
        AllowedScope::default(),
        "2026-05-21T00:00:00Z",
    )
    .await
    .unwrap();

    let err = veh
        .commit_rollback("sha256:does-not-exist", "2026-05-21T00:02:00Z")
        .await
        .unwrap_err();
    let msg = format!("{err}");
    assert!(
        msg.contains("does-not-exist") || msg.to_lowercase().contains("not found"),
        "unexpected error: {msg}",
    );
}

#[tokio::test]
async fn tampered_historical_node_fails_signature_verification() {
    let ledger = Arc::new(InMemoryLineage::new());
    let evaluator = Arc::new(StubEvaluator::new(0.7));
    let key = SigningKey::generate(&mut OsRng);
    let veh = Veh::new(ledger.clone(), evaluator, key).with_strict_improvement(false);

    let root = veh
        .commit_root(
            AgentArtifact::new(manifest("root")),
            AllowedScope::default(),
            "2026-05-21T00:00:00Z",
        )
        .await
        .unwrap();

    // Round-trip through JSON, mutate a load-bearing signed field,
    // deserialize back into `AgentNode`, and confirm signature
    // verification rejects the tampered copy.
    let serialised = serde_json::to_string(&root).unwrap();
    let mut tampered: serde_json::Value = serde_json::from_str(&serialised).unwrap();
    tampered
        .get_mut("mutation_intent")
        .and_then(|v| v.as_object_mut())
        .unwrap()
        .insert("goal".into(), serde_json::json!("attacker_inserted"));

    let tampered_node: rig_veh::AgentNode = serde_json::from_value(tampered).unwrap();
    let err = verify_node(&tampered_node).unwrap_err();
    let msg = format!("{err}");
    assert!(
        msg.to_lowercase().contains("signature") || msg.to_lowercase().contains("hash"),
        "tamper detection should surface a signature/hash error, got: {msg}",
    );

    // Sanity: the un-tampered node still verifies.
    verify_node(&root).unwrap();
}
