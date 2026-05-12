//! Integration tests for the parent-selection strategies.

#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use std::sync::Arc;

use ed25519_dalek::SigningKey;
use rand_core::OsRng;
use rig_compose::AgentManifest;
use rig_veh::{
    AgentArtifact, AgentNode, AllowedScope, Best, CycleInputs, CycleOutcome, InMemoryLineage,
    Latest, LineageStore, MutationIntent, ParentSelector, Random, ScoreChildProportional,
    ScoreProportional, StubEvaluator, Veh,
};

fn manifest(name: &str) -> AgentManifest {
    AgentManifest::from_yaml(&format!("name: {name}\ntools: []\n")).unwrap()
}

async fn build_lineage(scores: &[f64]) -> (Arc<InMemoryLineage>, Vec<AgentNode>) {
    let ledger = Arc::new(InMemoryLineage::new());
    let first = scores[0];
    let evaluator = Arc::new(StubEvaluator::new(first));
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
    let mut chain = vec![root.clone()];
    let mut parent = root;
    for (i, score) in scores.iter().skip(1).enumerate() {
        let evaluator = Arc::new(StubEvaluator::new(*score));
        let key = SigningKey::generate(&mut OsRng);
        let veh = Veh::new(ledger.clone(), evaluator, key).with_strict_improvement(false);
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
                parent = node.clone();
                chain.push(node);
            }
            _ => panic!("expected Promoted"),
        }
    }
    (ledger, chain)
}

#[tokio::test]
async fn latest_returns_most_recent_promoted() {
    let (ledger, chain) = build_lineage(&[0.1, 0.5, 0.9]).await;
    let selector = Latest;
    let pick = selector.select(ledger.as_ref()).await.unwrap();
    assert_eq!(pick, chain.last().unwrap().agent_id);
}

#[tokio::test]
async fn best_returns_highest_score() {
    let (ledger, chain) = build_lineage(&[0.1, 0.9, 0.5]).await;
    let selector = Best;
    let pick = selector.select(ledger.as_ref()).await.unwrap();
    // chain index 1 has score 0.9.
    assert_eq!(pick, chain[1].agent_id);
}

#[tokio::test]
async fn random_with_seed_is_deterministic() {
    let (ledger, _chain) = build_lineage(&[0.1, 0.5, 0.9]).await;
    let a = Random::with_seed(42).select(ledger.as_ref()).await.unwrap();
    let b = Random::with_seed(42).select(ledger.as_ref()).await.unwrap();
    assert_eq!(a, b);
}

#[tokio::test]
async fn score_proportional_picks_some_promoted_node() {
    let (ledger, chain) = build_lineage(&[0.1, 0.5, 0.9]).await;
    let selector = ScoreProportional::with_seed(7);
    let pick = selector.select(ledger.as_ref()).await.unwrap();
    assert!(chain.iter().any(|n| n.agent_id == pick));
}

#[tokio::test]
async fn score_child_proportional_picks_some_promoted_node() {
    let (ledger, chain) = build_lineage(&[0.1, 0.5, 0.9]).await;
    let selector = ScoreChildProportional::with_seed(7);
    let pick = selector.select(ledger.as_ref()).await.unwrap();
    assert!(chain.iter().any(|n| n.agent_id == pick));
}

#[tokio::test]
async fn selectors_error_on_empty_ledger() {
    let ledger = Arc::new(InMemoryLineage::new());
    let err = Latest.select(ledger.as_ref()).await.unwrap_err();
    assert!(format!("{err}").contains("no valid parent nodes"));
}

#[tokio::test]
async fn nodes_helper_returns_all_in_insertion_order() {
    let (ledger, chain) = build_lineage(&[0.1, 0.5, 0.9]).await;
    let nodes = ledger.nodes().await.unwrap();
    assert_eq!(nodes.len(), chain.len());
    for (a, b) in nodes.iter().zip(chain.iter()) {
        assert_eq!(a.agent_id, b.agent_id);
    }
}
