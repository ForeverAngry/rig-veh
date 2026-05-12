//! Integration tests for FR-4 (policy gate) and SR-1 (out-of-band approval).

#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use std::sync::Arc;

use ed25519_dalek::SigningKey;
use rand_core::OsRng;
use rig_compose::AgentManifest;
use rig_veh::{
    AgentArtifact, AllowedScope, CycleInputs, CycleOutcome, InMemoryLineage, LineageStore,
    MutationIntent, StubEvaluator, Veh, verify_node,
};

fn manifest_with_tool(name: &str, tool: &str) -> AgentManifest {
    let yaml = format!("name: {name}\ntools:\n  - kind: local\n    name: {tool}\n");
    AgentManifest::from_yaml(&yaml).unwrap()
}

#[tokio::test]
async fn out_of_scope_tool_pauses_then_resumes_with_approval() {
    let ledger = Arc::new(InMemoryLineage::new());
    let evaluator = Arc::new(StubEvaluator::new(0.7));
    let runner_key = SigningKey::generate(&mut OsRng);
    let veh = Veh::new(ledger.clone(), evaluator, runner_key).with_strict_improvement(false);

    // Parent ships tool "a" and only allows tool "a".
    let scope = AllowedScope {
        allowed_tools: vec!["a".into()],
        ..AllowedScope::default()
    };
    let root = veh
        .commit_root(
            AgentArtifact::new(manifest_with_tool("root", "a")),
            scope.clone(),
            "2026-05-11T00:00:00Z",
        )
        .await
        .unwrap();

    // Candidate adds tool "b" — out of scope.
    let candidate = AgentArtifact::new(manifest_with_tool("v1", "b"));
    let outcome = veh
        .run_cycle(CycleInputs {
            parent: &root,
            intent: MutationIntent::new("add_tool_b"),
            candidate,
            allowed_scope: scope,
            created_at: "2026-05-11T00:01:00Z".into(),
        })
        .await
        .unwrap();

    let pending = match outcome {
        CycleOutcome::AwaitingApproval(p) => p,
        other => panic!("expected AwaitingApproval, got {:?}", kind(&other)),
    };
    assert!(!pending.reason().is_empty());

    // Head must NOT have advanced.
    assert_eq!(ledger.head().await.unwrap(), Some(root.agent_id.clone()));

    // Resume with a distinct approval key (multisig story).
    let approval_key = SigningKey::generate(&mut OsRng);
    let promoted = veh
        .resume_with_approval(pending, &approval_key)
        .await
        .unwrap();

    // Promoted node verifies under the approver's public key.
    verify_node(&promoted).unwrap();
    assert_eq!(promoted.generation, root.generation + 1);
    assert_eq!(ledger.head().await.unwrap(), Some(promoted.agent_id));
}

#[tokio::test]
async fn in_scope_addition_promotes_directly() {
    let ledger = Arc::new(InMemoryLineage::new());
    let evaluator = Arc::new(StubEvaluator::new(0.9));
    let key = SigningKey::generate(&mut OsRng);
    let veh = Veh::new(ledger.clone(), evaluator, key).with_strict_improvement(false);

    let scope = AllowedScope {
        allowed_tools: vec!["a".into(), "b".into()],
        ..AllowedScope::default()
    };
    let root = veh
        .commit_root(
            AgentArtifact::new(manifest_with_tool("root", "a")),
            scope.clone(),
            "2026-05-11T00:00:00Z",
        )
        .await
        .unwrap();

    let outcome = veh
        .run_cycle(CycleInputs {
            parent: &root,
            intent: MutationIntent::new("add_in_scope_tool"),
            candidate: AgentArtifact::new(manifest_with_tool("v1", "b")),
            allowed_scope: scope,
            created_at: "2026-05-11T00:01:00Z".into(),
        })
        .await
        .unwrap();

    assert!(matches!(outcome, CycleOutcome::Promoted { .. }));
}

fn kind(o: &CycleOutcome) -> &'static str {
    match o {
        CycleOutcome::Promoted { .. } => "Promoted",
        CycleOutcome::Rejected { .. } => "Rejected",
        CycleOutcome::AwaitingApproval(_) => "AwaitingApproval",
    }
}
