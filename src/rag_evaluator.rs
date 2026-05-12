//! Optional adapter that bridges [`rig_evals_rag`] into an [`Evaluator`].
//!
//! The host supplies a `ReportFactory` closure that, given a candidate
//! [`AgentArtifact`], runs whatever `rig-evals-rag` driver (typically
//! [`rig_evals_rag::RetrievalHarness`]) is appropriate and hands back
//! a [`MultiReport`]. The adapter then projects the configured primary
//! metric's mean into an [`EvalResult::score`] and copies the full
//! report into `details`.
//!
//! Keeping the harness wiring host-owned avoids cross-crate HRTB
//! lifetime gymnastics and lets each host decide how to instantiate
//! its vector store from a manifest (`rig-memvid`, `rig-lancedb`,
//! anything else).

use std::sync::Arc;

use async_trait::async_trait;
use futures::future::BoxFuture;
use rig_evals_rag::MultiReport;

use crate::artifact::AgentArtifact;
use crate::error::{Error, Result};
use crate::evaluator::{EvalResult, Evaluator};

/// Host-supplied async closure that benchmarks one candidate.
///
/// The closure owns the harness lifecycle for one evaluation pass:
/// load fixtures, materialise the store, run the metrics, return the
/// [`MultiReport`]. The trait object is `Send + Sync` so the
/// evaluator can be used from any runtime.
///
/// Hosts surface harness-specific failures via [`Error::Evaluator`]
/// — typically `your_err.to_string()` — to keep this crate's public
/// error surface typed and free of `Box<dyn Error>`.
pub type ReportFactory =
    Arc<dyn for<'a> Fn(&'a AgentArtifact) -> BoxFuture<'a, Result<MultiReport>> + Send + Sync>;

/// Wraps a [`ReportFactory`] in an [`Evaluator`].
pub struct RetrievalEvaluator {
    primary_metric: String,
    factory: ReportFactory,
}

impl RetrievalEvaluator {
    /// Build an evaluator. `primary_metric` must match the
    /// [`rig_evals_rag::retrieval::RetrievalMetric::name`] of the
    /// metric the host wants used as the scalar score (e.g.
    /// `"recall@10"`).
    pub fn new(primary_metric: impl Into<String>, factory: ReportFactory) -> Self {
        Self {
            primary_metric: primary_metric.into(),
            factory,
        }
    }
}

#[async_trait]
impl Evaluator for RetrievalEvaluator {
    async fn evaluate(&self, artifact: &AgentArtifact) -> Result<EvalResult> {
        let report = (self.factory)(artifact).await?;
        let score = report
            .metrics
            .iter()
            .find(|r| r.metric == self.primary_metric)
            .map(|r| r.mean)
            .ok_or_else(|| {
                Error::Evaluator(format!("metric {} not in report", self.primary_metric))
            })?;
        let details = serde_json::to_value(&report).unwrap_or(serde_json::Value::Null);
        Ok(EvalResult { score, details })
    }
}
