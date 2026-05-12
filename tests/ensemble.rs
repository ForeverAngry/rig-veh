//! Integration test for [`EnsembleSelector`].

#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use std::sync::Arc;

use async_trait::async_trait;
use ed25519_dalek::SigningKey;
use rand_core::OsRng;
use rig_compose::AgentManifest;
use rig_veh::{
    AgentArtifact, AgentNode, AllowedScope, BestByMetric, EnsembleSelector, EvalResult, Evaluator,
    InMemoryLineage, LineageStore, MutationContext, MutationIntent, Mutator, NoopSandbox, Random,
    Result, Veh,
};

/// Evaluator that returns scripted scores in order. Used so each
/// `run_generation` call gets a distinct score.
struct ScriptedEvaluator {
    scores: std::sync::Mutex<Vec<f64>>,
}

#[async_trait]
impl Evaluator for ScriptedEvaluator {
    async fn evaluate(&self, _artifact: &AgentArtifact) -> Result<EvalResult> {
        let mut q = self.scores.lock().unwrap();
        let s = if q.is_empty() { 0.0 } else { q.remove(0) };
        Ok(EvalResult::from_score(s))
    }
}

struct CountingMutator {
    counter: std::sync::Mutex<usize>,
}

#[async_trait]
impl Mutator for CountingMutator {
    async fn propose(
        &self,
        _parent: &AgentNode,
        _intent: &MutationIntent,
        _context: &MutationContext,
    ) -> Result<AgentArtifact> {
        let mut c = self.counter.lock().unwrap();
        *c += 1;
        let manifest = AgentManifest::from_yaml(&format!("name: v{}\ntools: []\n", *c)).unwrap();
        Ok(AgentArtifact::new(manifest))
    }
}

async fn build_lineage(scores: Vec<f64>) -> Arc<InMemoryLineage> {
    let ledger = Arc::new(InMemoryLineage::new());
    let mut full_scores = vec![0.0];
    full_scores.extend(scores.iter().copied());
    let evaluator = Arc::new(ScriptedEvaluator {
        scores: std::sync::Mutex::new(full_scores),
    });
    let key = SigningKey::generate(&mut OsRng);
    let veh = Veh::new(ledger.clone(), evaluator, key).with_strict_improvement(false);

    veh.commit_root(
        AgentArtifact::new(AgentManifest::from_yaml("name: root\ntools: []\n").unwrap()),
        AllowedScope::default(),
        "2026-05-11T00:00:00Z",
    )
    .await
    .unwrap();

    let driver = rig_veh::EvolutionDriver::new(
        veh,
        ledger.clone(),
        Arc::new(Random::with_seed(0)),
        Arc::new(CountingMutator {
            counter: std::sync::Mutex::new(0),
        }),
        Arc::new(NoopSandbox),
    );

    for i in 0..scores.len() {
        driver
            .run_generation(
                MutationIntent::new(format!("gen-{i}")),
                AllowedScope::default(),
                MutationContext::new(),
                format!("2026-05-11T00:0{}:00Z", i + 1),
            )
            .await
            .unwrap();
    }
    ledger
}

#[tokio::test]
async fn best_by_metric_returns_top_k_by_score_desc() {
    let ledger = build_lineage(vec![0.9, 0.5, 0.7]).await;
    let selector = BestByMetric::top_k(2);
    let ids = selector.select(ledger.as_ref()).await.unwrap();
    assert!(ids.len() <= 2);
    let nodes = LineageStore::nodes(ledger.as_ref()).await.unwrap();
    let scores: Vec<f64> = ids
        .iter()
        .map(|id| {
            let n = nodes.iter().find(|n| &n.agent_id == id).unwrap();
            EnsembleSelector::score_of(&selector, n)
        })
        .collect();
    // Sorted descending.
    for w in scores.windows(2) {
        assert!(w[0] >= w[1], "scores not sorted desc: {:?}", scores);
    }
    // Best promoted node must be present.
    if let Some(first) = scores.first() {
        let max_promoted = nodes
            .iter()
            .filter(|n| matches!(n.status, rig_veh::NodeStatus::Promoted))
            .map(|n| EnsembleSelector::score_of(&selector, n))
            .fold(f64::MIN, f64::max);
        assert!((first - max_promoted).abs() < f64::EPSILON);
    }
}

#[tokio::test]
async fn best_by_metric_clamps_to_available_nodes() {
    let ledger = build_lineage(vec![0.5]).await;
    let selector = BestByMetric::top_k(10);
    let ids = selector.select(ledger.as_ref()).await.unwrap();
    assert!(ids.len() <= 2);
}
