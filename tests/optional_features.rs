//! Integration coverage for optional `rig-veh` feature surfaces.

#![cfg(any(feature = "jsonl-ledger", feature = "dot-export", feature = "rag"))]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::sync::Arc;

use ed25519_dalek::SigningKey;
use rand_core::OsRng;
use rig_compose::AgentManifest;
use rig_veh::{
    AgentArtifact, AllowedScope, CycleInputs, CycleOutcome, Evaluator, InMemoryLineage,
    LineageStore, MutationIntent, StubEvaluator, Veh,
};

fn manifest(name: &str) -> AgentManifest {
    AgentManifest::from_yaml(&format!("name: {name}\ntools: []\n")).unwrap()
}

#[cfg(feature = "jsonl-ledger")]
#[tokio::test]
async fn jsonl_lineage_replays_existing_nodes_and_head() {
    use rig_veh::JsonlLineage;

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("lineage.jsonl");
    let ledger = Arc::new(JsonlLineage::open(&path).unwrap());
    let evaluator = Arc::new(StubEvaluator::new(0.7));
    let key = SigningKey::generate(&mut OsRng);
    let veh = Veh::new(ledger.clone(), evaluator, key).with_strict_improvement(false);

    let root = veh
        .commit_root(
            AgentArtifact::new(manifest("root")),
            AllowedScope::default(),
            "2026-05-12T00:00:00Z",
        )
        .await
        .unwrap();
    let outcome = veh
        .run_cycle(CycleInputs {
            parent: &root,
            intent: MutationIntent::new("persisted_candidate"),
            candidate: AgentArtifact::new(manifest("v1")),
            allowed_scope: AllowedScope::default(),
            created_at: "2026-05-12T00:01:00Z".into(),
        })
        .await
        .unwrap();
    let promoted = match outcome {
        CycleOutcome::Promoted { node, .. } => node,
        _ => panic!("expected promoted candidate"),
    };

    let replayed = JsonlLineage::open(&path).unwrap();
    assert_eq!(
        replayed.head().await.unwrap(),
        Some(promoted.agent_id.clone())
    );
    assert_eq!(
        replayed.get(&root.agent_id).await.unwrap().agent_id,
        root.agent_id
    );
    assert_eq!(
        replayed.get(&promoted.agent_id).await.unwrap().agent_id,
        promoted.agent_id
    );
}

#[cfg(feature = "dot-export")]
#[tokio::test]
async fn dot_export_contains_nodes_and_parent_edge() {
    let ledger = Arc::new(InMemoryLineage::new());
    let evaluator = Arc::new(StubEvaluator::new(0.8));
    let key = SigningKey::generate(&mut OsRng);
    let veh = Veh::new(ledger.clone(), evaluator, key).with_strict_improvement(false);

    let root = veh
        .commit_root(
            AgentArtifact::new(manifest("root")),
            AllowedScope::default(),
            "2026-05-12T00:00:00Z",
        )
        .await
        .unwrap();
    let outcome = veh
        .run_cycle(CycleInputs {
            parent: &root,
            intent: MutationIntent::new("dot_candidate"),
            candidate: AgentArtifact::new(manifest("v1")),
            allowed_scope: AllowedScope::default(),
            created_at: "2026-05-12T00:01:00Z".into(),
        })
        .await
        .unwrap();
    let promoted = match outcome {
        CycleOutcome::Promoted { node, .. } => node,
        _ => panic!("expected promoted candidate"),
    };

    let dot = rig_veh::export_dot(ledger.as_ref()).await.unwrap();
    assert!(dot.contains("digraph lineage"));
    assert!(dot.contains(&root.agent_id));
    assert!(dot.contains(&promoted.agent_id));
    assert!(dot.contains(&format!(
        "\"{}\" -> \"{}\"",
        root.agent_id, promoted.agent_id
    )));
}

#[cfg(feature = "rag")]
#[tokio::test]
async fn retrieval_evaluator_projects_primary_metric() {
    use futures::FutureExt as _;
    use rig_evals_rag::{MetricReport, MultiReport};
    use rig_veh::{ReportFactory, RetrievalEvaluator};

    let factory: ReportFactory = Arc::new(|_artifact| {
        async move {
            Ok(MultiReport::new(vec![MetricReport::from_per_query(
                "recall@10".to_string(),
                vec![("q1".to_string(), 1.0), ("q2".to_string(), 0.5)],
            )]))
        }
        .boxed()
    });
    let evaluator = RetrievalEvaluator::new("recall@10", factory);

    let eval = evaluator
        .evaluate(&AgentArtifact::new(manifest("candidate")))
        .await
        .unwrap();
    assert_eq!(eval.score, 0.75);
    assert_eq!(eval.details["metrics"][0]["metric"], "recall@10");
}

#[cfg(feature = "rag")]
#[tokio::test]
async fn retrieval_evaluator_reports_missing_primary_metric() {
    use futures::FutureExt as _;
    use rig_evals_rag::{MetricReport, MultiReport};
    use rig_veh::{ReportFactory, RetrievalEvaluator};

    let factory: ReportFactory = Arc::new(|_artifact| {
        async move {
            Ok(MultiReport::new(vec![MetricReport::from_per_query(
                "ndcg@10".to_string(),
                vec![("q1".to_string(), 1.0)],
            )]))
        }
        .boxed()
    });
    let evaluator = RetrievalEvaluator::new("recall@10", factory);

    let err = evaluator
        .evaluate(&AgentArtifact::new(manifest("candidate")))
        .await
        .unwrap_err();
    assert!(matches!(err, rig_veh::Error::Evaluator(_)));
}
