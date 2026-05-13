//! Integration coverage for the additive [`EvidenceBundle`] shape.
//!
//! The bundle is hash-neutral: it round-trips through `eval_results`
//! and survives JSON reload + signature verification, so reloaded
//! lineage can answer "why did this mutation pass/fail?" without
//! re-running the evaluator.
#![allow(
    clippy::unwrap_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::expect_used
)]

use std::sync::Arc;

use async_trait::async_trait;
use ed25519_dalek::SigningKey;
use rand_core::OsRng;
use rig_compose::AgentManifest;
use rig_veh::evaluator::{EvalResult, Evaluator};
use rig_veh::{
    AgentArtifact, AllowedScope, CycleInputs, CycleOutcome, EvidenceBundle, InMemoryLineage,
    LineageStore, MutationIntent, OperatorApproval, PolicyVerdict, Result, Veh, verify_node,
};
use serde_json::{Value, json};

fn manifest(name: &str) -> AgentManifest {
    AgentManifest::from_yaml(&format!("name: {name}\ntools: []\n")).unwrap()
}

/// Evaluator that stashes a prebuilt `EvidenceBundle` JSON into
/// `EvalResult.details`. Real callers would build the bundle from the
/// live policy decision + evaluator report; the test bypasses that
/// plumbing so it can assert the bundle survives sign + reload.
struct EvidenceEvaluator {
    score: f64,
    details: Value,
}

#[async_trait]
impl Evaluator for EvidenceEvaluator {
    async fn evaluate(&self, _artifact: &AgentArtifact) -> Result<EvalResult> {
        Ok(EvalResult {
            score: self.score,
            details: self.details.clone(),
        })
    }
}

#[tokio::test]
async fn evidence_bundle_survives_sign_and_reload() {
    let bundle = EvidenceBundle::new(PolicyVerdict::approved("within_scope"))
        .with_evaluator_report(json!({
            "metric": "recall@10",
            "mean": 0.83,
        }))
        .with_context_items(vec![
            json!({"source": "memvid", "score": 0.91, "text": "prior turn"}),
            json!({"source": "resource", "score": 0.42, "text": "behavior pattern"}),
        ])
        .with_operator_approval(OperatorApproval::new("alice", "ship it"));
    let bundle_value = bundle.to_value().unwrap();

    let ledger = Arc::new(InMemoryLineage::new());
    let evaluator = Arc::new(EvidenceEvaluator {
        score: 0.83,
        details: bundle_value.clone(),
    });
    let key = SigningKey::generate(&mut OsRng);
    let veh = Veh::new(ledger.clone(), evaluator, key).with_strict_improvement(false);

    let root = veh
        .commit_root(
            AgentArtifact::new(manifest("root")),
            AllowedScope::default(),
            "2026-05-20T00:00:00Z",
        )
        .await
        .unwrap();

    let outcome = veh
        .run_cycle(CycleInputs {
            parent: &root,
            intent: MutationIntent::new("v1"),
            candidate: AgentArtifact::new(manifest("v1")),
            allowed_scope: AllowedScope::default(),
            created_at: "2026-05-20T00:01:00Z".into(),
        })
        .await
        .unwrap();

    let promoted = match outcome {
        CycleOutcome::Promoted { node, .. } => node,
        _ => panic!("expected Promoted"),
    };

    // Signature still verifies on the freshly-committed node.
    verify_node(&promoted).unwrap();

    // The eval_results carries the full `EvalResult`; the bundle lives
    // under `details` (the same slot the retrieval evaluator uses for
    // `MultiReport`s), and parses back into an `EvidenceBundle` with no
    // data loss.
    let stored = promoted
        .eval_results
        .as_ref()
        .expect("eval_results should be populated");
    let stored_bundle = stored
        .get("details")
        .expect("eval_results.details should hold the bundle");
    let parsed = EvidenceBundle::from_value(stored_bundle).unwrap();
    assert_eq!(parsed, bundle);

    // Reload from the ledger and re-verify: the bundle survives
    // JSON round-trip through the store.
    let reloaded = ledger.get(&promoted.agent_id).await.unwrap();
    verify_node(&reloaded).unwrap();
    let reloaded_bundle = EvidenceBundle::from_value(
        reloaded
            .eval_results
            .as_ref()
            .unwrap()
            .get("details")
            .unwrap(),
    )
    .unwrap();
    assert_eq!(reloaded_bundle, bundle);
}
