//! Ensemble selection — pick a group of agents that, together,
//! perform better than any individual.
//!
//! HyperAgents'
//! [`ensemble.py`](https://github.com/facebookresearch/HyperAgents/blob/main/ensemble.py)
//! takes the best agent per question across the archive and combines
//! them. VEH does not own task-level scoring (that lives in the host
//! evaluator), so this module exposes a generic seam: given the
//! ledger, return `N` promoted [`AgentId`]s. Hosts plug their own
//! domain-specific strategy on top.
//!
//! ## Example
//!
//! ```no_run
//! use std::sync::Arc;
//! use rig_veh::{BestByMetric, EnsembleSelector, InMemoryLineage};
//!
//! # async fn run() -> rig_veh::Result<()> {
//! let ledger: Arc<InMemoryLineage> = Arc::new(InMemoryLineage::new());
//! let selector = BestByMetric::top_k(3);
//! let _ids = selector.select(ledger.as_ref()).await?;
//! # Ok(()) }
//! ```

use async_trait::async_trait;

use crate::error::Result;
use crate::ledger::LineageStore;
use crate::node::{AgentId, AgentNode};

/// Pluggable ensemble-selection strategy.
#[async_trait]
pub trait EnsembleSelector: Send + Sync {
    /// Pick up to `k` parents. The returned vector may be shorter
    /// than `k` if the ledger has fewer promoted nodes.
    /// Implementations must only consider
    /// nodes where `valid_parent` is true.
    async fn select(&self, store: &dyn LineageStore) -> Result<Vec<AgentId>>;

    /// Extract a scalar score for a node. Default reads
    /// `eval_results.score`; returns `0.0` if absent.
    fn score_of(&self, node: &AgentNode) -> f64 {
        node.eval_results
            .as_ref()
            .and_then(|v| v.get("score"))
            .and_then(|s| s.as_f64())
            .unwrap_or(0.0)
    }
}

/// Pick the top `k` promoted nodes by score (descending). Ties
/// broken by insertion order.
pub struct BestByMetric {
    k: usize,
}

impl BestByMetric {
    /// Select the top `k` agents by score.
    pub fn top_k(k: usize) -> Self {
        Self { k }
    }
}

#[async_trait]
impl EnsembleSelector for BestByMetric {
    async fn select(&self, store: &dyn LineageStore) -> Result<Vec<AgentId>> {
        let mut promoted: Vec<AgentNode> = store
            .nodes()
            .await?
            .into_iter()
            .filter(|n| n.valid_parent)
            .collect();
        promoted.sort_by(|a, b| {
            self.score_of(b)
                .partial_cmp(&self.score_of(a))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        Ok(promoted
            .into_iter()
            .take(self.k)
            .map(|n| n.agent_id)
            .collect())
    }
}
