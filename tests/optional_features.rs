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
    LineageStore, MutationIntent, Veh,
};

#[cfg(any(feature = "jsonl-ledger", feature = "dot-export"))]
use rig_veh::StubEvaluator;

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

/// UAT-06 end-to-end: a `RetrievalEvaluator` backed by deterministic
/// `rig-evals-rag` reports drives the full lifecycle.
///
/// The factory routes by manifest name so we can stage a baseline
/// (root), an improvement (promoted), and a regression (rejected) in
/// the same ledger. The promoted node's `eval_results` carries the
/// per-metric report as evaluator evidence; signatures verify after a
/// round-trip; the rejected node records the failing metric.
#[cfg(feature = "rag")]
#[tokio::test]
async fn retrieval_evaluator_drives_promote_and_reject_with_signed_evidence() {
    use futures::FutureExt as _;
    use rig_evals_rag::{MetricReport, MultiReport, RegressionGate};
    use rig_veh::{ReportFactory, RetrievalEvaluator, verify_node};

    fn report(metric: &str, per_query: &[(&str, f64)]) -> MultiReport {
        MultiReport::new(vec![MetricReport::from_per_query(
            metric.to_string(),
            per_query
                .iter()
                .map(|(q, s)| ((*q).to_string(), *s))
                .collect(),
        )])
    }

    // Per-manifest report routing. Scores: baseline 0.50, v2 0.80, v3 0.40.
    let factory: ReportFactory = Arc::new(|artifact| {
        let name = artifact.manifest.name.clone().unwrap_or_default();
        async move {
            let r = match name.as_str() {
                "baseline" => report("recall@10", &[("q1", 0.5), ("q2", 0.5)]),
                "v2" => report("recall@10", &[("q1", 1.0), ("q2", 0.6)]),
                "v3" => report("recall@10", &[("q1", 0.4), ("q2", 0.4)]),
                other => panic!("unexpected artifact name {other}"),
            };
            Ok(r)
        }
        .boxed()
    });

    let evaluator = Arc::new(RetrievalEvaluator::new("recall@10", factory));
    let ledger = Arc::new(InMemoryLineage::new());
    let key = SigningKey::generate(&mut OsRng);
    let veh = Veh::new(ledger.clone(), evaluator.clone(), key);

    // Commit root with a pre-recorded baseline score so the strict-
    // improvement check has a parent score to compare against.
    let baseline_eval = evaluator
        .evaluate(&AgentArtifact::new(manifest("baseline")))
        .await
        .unwrap();
    assert!((baseline_eval.score - 0.5).abs() < 1e-9);
    let baseline_report: MultiReport =
        serde_json::from_value(baseline_eval.details.clone()).unwrap();

    let root = veh
        .commit_root(
            AgentArtifact::new(manifest("baseline")),
            AllowedScope::default(),
            "2026-05-13T00:00:00Z",
        )
        .await
        .unwrap();
    // Inject the baseline score+report into the root so run_cycle's
    // FR-5 check sees a parent score to beat. We do this via a second
    // promoted cycle, since AgentNode is immutable post-commit.
    let warmup = veh
        .run_cycle(CycleInputs {
            parent: &root,
            intent: MutationIntent::new("warmup"),
            candidate: AgentArtifact::new(manifest("baseline")),
            allowed_scope: AllowedScope::default(),
            created_at: "2026-05-13T00:00:30Z".into(),
        })
        .await
        .unwrap();
    let baseline_node = match warmup {
        // First cycle promotes because root has no parent score.
        CycleOutcome::Promoted { node, .. } => node,
        _ => panic!("expected baseline warmup to promote"),
    };

    // Improvement: recall@10 climbs 0.50 -> 0.80. Expect Promoted with
    // signed evidence in metadata.
    let promote = veh
        .run_cycle(CycleInputs {
            parent: &baseline_node,
            intent: MutationIntent::new("tighten_preamble"),
            candidate: AgentArtifact::new(manifest("v2")),
            allowed_scope: AllowedScope::default(),
            created_at: "2026-05-13T00:01:00Z".into(),
        })
        .await
        .unwrap();
    let (promoted_node, promoted_eval) = match promote {
        CycleOutcome::Promoted { node, eval } => (node, eval),
        _ => panic!("expected promoted candidate"),
    };
    assert!((promoted_eval.score - 0.8).abs() < 1e-9);

    // Evaluator evidence survives into node metadata.
    let evidence = promoted_node
        .eval_results
        .as_ref()
        .expect("promoted node must carry evaluator evidence");
    let evidence_score = evidence.get("score").and_then(|v| v.as_f64()).unwrap();
    assert!((evidence_score - 0.8).abs() < 1e-9);
    let candidate_report: MultiReport =
        serde_json::from_value(evidence.get("details").cloned().unwrap()).unwrap();
    assert_eq!(candidate_report.metrics.len(), 1);
    assert_eq!(candidate_report.metrics[0].metric, "recall@10");
    assert!((candidate_report.metrics[0].mean - 0.8).abs() < 1e-9);

    // Signature verifies after reload.
    verify_node(&promoted_node).expect("promoted node signature must verify");
    let reloaded = ledger.get(&promoted_node.agent_id).await.unwrap();
    verify_node(&reloaded).expect("reloaded promoted node must verify");
    assert_eq!(reloaded.eval_results.as_ref(), Some(evidence));

    // ReportDiff handoff: the regression gate operates on the same
    // report pair, which is the boundary `rig-veh` consumes from
    // `rig-evals-rag`. Promotion implies no regression.
    let promote_diff = candidate_report.diff(&baseline_report).unwrap();
    let gate = RegressionGate::new().with_threshold("recall@10", 0.05);
    assert!(promote_diff.regressions(&gate).is_empty());

    // Regression: recall@10 drops 0.80 -> 0.40. Expect Rejected with
    // the failing metric recorded in node metadata.
    let reject = veh
        .run_cycle(CycleInputs {
            parent: &promoted_node,
            intent: MutationIntent::new("aggressive_prune"),
            candidate: AgentArtifact::new(manifest("v3")),
            allowed_scope: AllowedScope::default(),
            created_at: "2026-05-13T00:02:00Z".into(),
        })
        .await
        .unwrap();
    let (rejected_node, reason) = match reject {
        CycleOutcome::Rejected { node, reason } => (node, reason),
        _ => panic!("expected rejected candidate"),
    };
    assert!(
        reason.contains("0.4"),
        "reason should cite failing score: {reason}"
    );
    let rejected_evidence = rejected_node
        .eval_results
        .as_ref()
        .expect("rejected node must still carry the failing report");
    let rejected_report: MultiReport =
        serde_json::from_value(rejected_evidence.get("details").cloned().unwrap()).unwrap();
    let reject_diff = rejected_report.diff(&candidate_report).unwrap();
    let regressed = reject_diff.regressions(&gate);
    assert_eq!(regressed.len(), 1);
    assert_eq!(regressed[0].metric, "recall@10");

    // Lineage is intact and replayable.
    verify_node(&rejected_node).unwrap();
    assert_eq!(
        ledger.head().await.unwrap(),
        Some(promoted_node.agent_id.clone()),
        "head should remain on the last promoted node",
    );
}
