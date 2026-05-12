//! Integration test for the staged evaluator pipeline.

#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use std::sync::Arc;

use async_trait::async_trait;
use rig_compose::AgentManifest;
use rig_veh::{AgentArtifact, CompositeEvaluator, EvalResult, EvalStage, Evaluator, StagedResult};

struct ScriptedStage {
    score: f64,
    decision: EvalStage,
}

#[async_trait]
impl Evaluator for ScriptedStage {
    async fn evaluate(&self, _artifact: &AgentArtifact) -> rig_veh::Result<EvalResult> {
        Ok(EvalResult::from_score(self.score))
    }

    async fn evaluate_staged(
        &self,
        artifact: &AgentArtifact,
        _stage: usize,
    ) -> rig_veh::Result<StagedResult> {
        let result = self.evaluate(artifact).await?;
        Ok(StagedResult::new(result, self.decision))
    }
}

fn artifact() -> AgentArtifact {
    AgentArtifact::new(AgentManifest::from_yaml("name: a\ntools: []\n").unwrap())
}

#[tokio::test]
async fn composite_runs_all_stages_when_each_continues() {
    let composite = CompositeEvaluator::new(vec![
        Arc::new(ScriptedStage {
            score: 0.4,
            decision: EvalStage::Continue,
        }),
        Arc::new(ScriptedStage {
            score: 0.7,
            decision: EvalStage::Continue,
        }),
    ]);
    let runs = composite.run(&artifact()).await.unwrap();
    assert_eq!(runs.len(), 2);
    assert!((runs[1].result.score - 0.7).abs() < f64::EPSILON);
}

#[tokio::test]
async fn composite_short_circuits_on_stop() {
    let composite = CompositeEvaluator::new(vec![
        Arc::new(ScriptedStage {
            score: 0.1,
            decision: EvalStage::Stop,
        }),
        Arc::new(ScriptedStage {
            score: 0.9,
            decision: EvalStage::Continue,
        }),
    ]);
    let runs = composite.run(&artifact()).await.unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].decision, EvalStage::Stop);
}

#[tokio::test]
async fn composite_short_circuits_on_promote() {
    let composite = CompositeEvaluator::new(vec![
        Arc::new(ScriptedStage {
            score: 0.95,
            decision: EvalStage::Promote,
        }),
        Arc::new(ScriptedStage {
            score: 0.1,
            decision: EvalStage::Continue,
        }),
    ]);
    let runs = composite.run(&artifact()).await.unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].decision, EvalStage::Promote);
}

#[tokio::test]
async fn evaluate_returns_final_stage_result() {
    let composite = CompositeEvaluator::new(vec![
        Arc::new(ScriptedStage {
            score: 0.2,
            decision: EvalStage::Continue,
        }),
        Arc::new(ScriptedStage {
            score: 0.8,
            decision: EvalStage::Promote,
        }),
    ]);
    let result = composite.evaluate(&artifact()).await.unwrap();
    assert!((result.score - 0.8).abs() < f64::EPSILON);
}
