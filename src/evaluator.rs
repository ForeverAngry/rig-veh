//! Sandboxed evaluation surface.
//!
//! [`Evaluator`] is the generic benchmark interface. v0.1 ships a
//! generic trait + a [`StubEvaluator`] for tests; an adapter over
//! `rig-evals-rag` lives behind the `rag` feature in
//! [`crate::rag_evaluator`].

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::artifact::AgentArtifact;
use crate::error::Result;

/// Output of one evaluation pass.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvalResult {
    /// Scalar score on the host's primary metric. Higher is better.
    pub score: f64,
    /// Per-metric breakdown for human / JSON inspection.
    #[serde(default)]
    pub details: serde_json::Value,
}

impl EvalResult {
    /// Build a bare scalar result.
    pub fn from_score(score: f64) -> Self {
        Self {
            score,
            details: serde_json::Value::Null,
        }
    }
}

/// Outcome of a single stage in a multi-stage evaluator. Mirrors the
/// `staged_eval` short-circuit in HyperAgents'
/// [`generate_loop.py`](https://github.com/facebookresearch/HyperAgents/blob/main/generate_loop.py)
/// so hosts can cheaply rule out weak candidates before paying for
/// the full benchmark.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EvalStage {
    /// Candidate cleared this stage; the driver should continue to
    /// the next stage if any remain.
    Continue,
    /// Candidate is good enough to promote without further stages.
    Promote,
    /// Candidate failed; abandon further evaluation.
    Stop,
}

/// One stage of a staged evaluation pipeline.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StagedResult {
    /// Numeric result emitted by this stage.
    pub result: EvalResult,
    /// Whether the driver should keep evaluating, promote, or abort.
    pub decision: EvalStage,
}

impl StagedResult {
    /// Convenience constructor.
    pub fn new(result: EvalResult, decision: EvalStage) -> Self {
        Self { result, decision }
    }
}

/// Generic benchmark interface.
#[async_trait]
pub trait Evaluator: Send + Sync {
    /// Evaluate a candidate artifact. The implementation is responsible
    /// for instantiating whatever runtime the artifact needs and for
    /// keeping the sandbox isolated (SR-3).
    async fn evaluate(&self, artifact: &AgentArtifact) -> Result<EvalResult>;

    /// Evaluate one stage of a staged pipeline. The default
    /// implementation runs [`Evaluator::evaluate`] and returns
    /// [`EvalStage::Continue`], so existing single-stage evaluators
    /// continue to work unchanged.
    async fn evaluate_staged(
        &self,
        artifact: &AgentArtifact,
        _stage: usize,
    ) -> Result<StagedResult> {
        let result = self.evaluate(artifact).await?;
        Ok(StagedResult::new(result, EvalStage::Continue))
    }
}

/// Deterministic evaluator that returns a fixed score. Useful for
/// tests and examples that need to exercise the lifecycle without a
/// real RAG pipeline.
pub struct StubEvaluator {
    score: f64,
}

impl StubEvaluator {
    /// Build a stub that always returns `score`.
    pub fn new(score: f64) -> Self {
        Self { score }
    }
}

#[async_trait]
impl Evaluator for StubEvaluator {
    async fn evaluate(&self, _artifact: &AgentArtifact) -> Result<EvalResult> {
        Ok(EvalResult::from_score(self.score))
    }
}

/// Sequenced evaluator that runs each stage in order and stops as
/// soon as a stage returns [`EvalStage::Stop`] or
/// [`EvalStage::Promote`].
///
/// The composite is itself an [`Evaluator`]; its [`Evaluator::evaluate`]
/// returns the result from the final stage that ran. Hosts that want
/// stage-by-stage visibility should call [`CompositeEvaluator::run`]
/// directly.
pub struct CompositeEvaluator {
    stages: Vec<std::sync::Arc<dyn Evaluator>>,
}

impl CompositeEvaluator {
    /// Build a composite from a non-empty list of stages. Stages run
    /// in the order supplied.
    pub fn new(stages: Vec<std::sync::Arc<dyn Evaluator>>) -> Self {
        Self { stages }
    }

    /// Run every stage in order, returning the per-stage results.
    /// Stops on the first `Stop` or `Promote` decision.
    pub async fn run(&self, artifact: &AgentArtifact) -> Result<Vec<StagedResult>> {
        let mut out = Vec::with_capacity(self.stages.len());
        for (idx, stage) in self.stages.iter().enumerate() {
            let staged = stage.evaluate_staged(artifact, idx).await?;
            let decision = staged.decision;
            out.push(staged);
            if matches!(decision, EvalStage::Stop | EvalStage::Promote) {
                break;
            }
        }
        Ok(out)
    }
}

#[async_trait]
impl Evaluator for CompositeEvaluator {
    async fn evaluate(&self, artifact: &AgentArtifact) -> Result<EvalResult> {
        let runs = self.run(artifact).await?;
        runs.into_iter()
            .next_back()
            .map(|s| s.result)
            .ok_or_else(|| crate::error::Error::Evaluator("composite has no stages".into()))
    }
}
