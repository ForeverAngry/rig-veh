//! Parent-selection strategies for open-ended evolution.
//!
//! VEH's default lifecycle promotes a candidate by advancing
//! [`LineageStore::head`] to it. That is greedy. A real evolutionary
//! search needs to pick *any* viable ancestor as the next parent —
//! sometimes a high-scoring leaf, sometimes a sparsely-explored
//! branch. This module mirrors the parent-selection strategies from
//! Meta's HyperAgents reference implementation
//! (<https://github.com/facebookresearch/HyperAgents/blob/main/select_next_parent.py>)
//! so hosts can drive open-ended search on top of VEH's signed
//! lineage.
//!
//! Implementations only read the ledger; they never mutate it. The
//! [`Veh`](crate::Veh) runner is unaware of selectors — the host wires
//! them into its evolution loop.
//!
//! ## Score extraction
//!
//! `EvalResult` is a flexible structure (`score: f64` + free-form
//! `details`). Selectors call [`ParentSelector::score_of`] which, by
//! default, reads `eval_results.score` if present and falls back to
//! `0.0`. Override for domain-specific metrics.
//!
//! ## Example
//!
//! ```no_run
//! use std::sync::Arc;
//! use rig_veh::{InMemoryLineage, ParentSelector, ScoreChildProportional};
//!
//! # async fn run() -> rig_veh::Result<()> {
//! let ledger: Arc<InMemoryLineage> = Arc::new(InMemoryLineage::new());
//! let selector = ScoreChildProportional::default();
//! let _parent = selector.select(ledger.as_ref()).await?;
//! # Ok(()) }
//! ```

use async_trait::async_trait;
use std::collections::HashMap;

use crate::error::{Error, Result};
use crate::ledger::LineageStore;
use crate::node::{AgentId, AgentNode};

/// Pluggable parent-selection strategy.
#[async_trait]
pub trait ParentSelector: Send + Sync {
    /// Pick the next parent from `store`. Implementations must only
    /// consider [`NodeStatus::Promoted`] nodes.
    async fn select(&self, store: &dyn LineageStore) -> Result<AgentId>;

    /// Extract the scalar score for a node. Default reads
    /// `eval_results.score` (matching [`crate::EvalResult`]); returns
    /// `0.0` for nodes without an `eval_results` payload (the root).
    fn score_of(&self, node: &AgentNode) -> f64 {
        node.eval_results
            .as_ref()
            .and_then(|v| v.get("score"))
            .and_then(|s| s.as_f64())
            .unwrap_or(0.0)
    }
}

/// Pick the most recently appended promoted node. Equivalent to
/// HyperAgents' `latest` selection method.
#[derive(Default, Clone, Copy)]
pub struct Latest;

#[async_trait]
impl ParentSelector for Latest {
    async fn select(&self, store: &dyn LineageStore) -> Result<AgentId> {
        let nodes = store.nodes().await?;
        nodes
            .into_iter()
            .rev()
            .find(|n| n.valid_parent)
            .map(|n| n.agent_id)
            .ok_or_else(|| Error::Ledger("no valid parent nodes in ledger".into()))
    }
}

/// Pick the highest-scoring promoted node. Equivalent to
/// HyperAgents' `best` selection.
#[derive(Default, Clone, Copy)]
pub struct Best;

#[async_trait]
impl ParentSelector for Best {
    async fn select(&self, store: &dyn LineageStore) -> Result<AgentId> {
        let nodes = store.nodes().await?;
        let mut best: Option<(f64, AgentId)> = None;
        for node in nodes {
            if !node.valid_parent {
                continue;
            }
            let score = self.score_of(&node);
            match &best {
                Some((b, _)) if *b >= score => {}
                _ => best = Some((score, node.agent_id)),
            }
        }
        best.map(|(_, id)| id)
            .ok_or_else(|| Error::Ledger("no valid parent nodes in ledger".into()))
    }
}

/// Sample a promoted node uniformly at random. Equivalent to
/// HyperAgents' `random` selection.
///
/// Deterministic when constructed via [`Random::with_seed`]; the
/// default uses [`fastrand`]-style hashing of the ledger contents so
/// repeated calls return different results without requiring a
/// runtime RNG.
#[derive(Default, Clone, Copy)]
pub struct Random {
    seed: Option<u64>,
}

impl Random {
    /// Build a selector seeded for reproducible tests.
    pub fn with_seed(seed: u64) -> Self {
        Self { seed: Some(seed) }
    }
}

#[async_trait]
impl ParentSelector for Random {
    async fn select(&self, store: &dyn LineageStore) -> Result<AgentId> {
        let valid: Vec<AgentNode> = store
            .nodes()
            .await?
            .into_iter()
            .filter(|n| n.valid_parent)
            .collect();
        if valid.is_empty() {
            return Err(Error::Ledger("no valid parent nodes in ledger".into()));
        }
        let len = valid.len();
        let idx = match self.seed {
            Some(seed) => (seed as usize) % len,
            None => {
                // Hash the ledger snapshot for a value that varies
                // across calls without requiring `getrandom`. Hosts
                // that need cryptographic randomness should wrap
                // their own RNG and implement [`ParentSelector`].
                use std::collections::hash_map::DefaultHasher;
                use std::hash::{Hash, Hasher};
                let mut hasher = DefaultHasher::new();
                for n in &valid {
                    n.agent_id.hash(&mut hasher);
                }
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_nanos())
                    .unwrap_or(0)
                    .hash(&mut hasher);
                (hasher.finish() as usize) % len
            }
        };
        valid
            .into_iter()
            .nth(idx)
            .map(|n| n.agent_id)
            .ok_or_else(|| Error::Ledger("random index out of bounds".into()))
    }
}

/// Sample a promoted node with probability proportional to its
/// score. Equivalent to HyperAgents' `score_prop` selection.
#[derive(Default, Clone, Copy)]
pub struct ScoreProportional {
    seed: Option<u64>,
}

impl ScoreProportional {
    /// Build a selector with a deterministic seed for tests.
    pub fn with_seed(seed: u64) -> Self {
        Self { seed: Some(seed) }
    }
}

#[async_trait]
impl ParentSelector for ScoreProportional {
    async fn select(&self, store: &dyn LineageStore) -> Result<AgentId> {
        let nodes = store.nodes().await?;
        let weighted: Vec<(f64, AgentId)> = nodes
            .into_iter()
            .filter(|n| n.valid_parent)
            .map(|n| (self.score_of(&n).max(0.0), n.agent_id))
            .collect();
        weighted_choice(&weighted, self.seed)
    }
}

/// Sample inversely to child count, weighted by score. Equivalent to
/// HyperAgents' `score_child_prop` — the default search strategy in
/// the reference implementation. Favours promising but
/// under-explored branches.
#[derive(Default, Clone, Copy)]
pub struct ScoreChildProportional {
    seed: Option<u64>,
}

impl ScoreChildProportional {
    /// Build a selector with a deterministic seed for tests.
    pub fn with_seed(seed: u64) -> Self {
        Self { seed: Some(seed) }
    }
}

#[async_trait]
impl ParentSelector for ScoreChildProportional {
    async fn select(&self, store: &dyn LineageStore) -> Result<AgentId> {
        let nodes = store.nodes().await?;
        let mut child_counts: HashMap<AgentId, u32> = HashMap::new();
        for n in &nodes {
            if let Some(parent) = &n.parent_id {
                *child_counts.entry(parent.clone()).or_insert(0) += 1;
            }
        }
        let weighted: Vec<(f64, AgentId)> = nodes
            .into_iter()
            .filter(|n| n.valid_parent)
            .map(|n| {
                let children = child_counts.get(&n.agent_id).copied().unwrap_or(0);
                let score = self.score_of(&n).max(0.0);
                // Same weighting shape as HyperAgents:
                //   weight = score / (1 + children)
                let weight = score / (1.0 + f64::from(children));
                (weight, n.agent_id)
            })
            .collect();
        weighted_choice(&weighted, self.seed)
    }
}

fn weighted_choice(weighted: &[(f64, AgentId)], seed: Option<u64>) -> Result<AgentId> {
    if weighted.is_empty() {
        return Err(Error::Ledger("no valid parent nodes in ledger".into()));
    }
    let total: f64 = weighted.iter().map(|(w, _)| *w).sum();
    if total <= 0.0 {
        // All scores zero — fall back to last promoted node so the
        // search makes progress instead of failing.
        return weighted
            .last()
            .map(|(_, id)| id.clone())
            .ok_or_else(|| Error::Ledger("weighted_choice empty fallback".into()));
    }
    let r = pseudo_unit(seed, weighted) * total;
    let mut acc = 0.0;
    for (w, id) in weighted {
        acc += *w;
        if r <= acc {
            return Ok(id.clone());
        }
    }
    weighted
        .last()
        .map(|(_, id)| id.clone())
        .ok_or_else(|| Error::Ledger("weighted_choice exhausted".into()))
}

fn pseudo_unit(seed: Option<u64>, weighted: &[(f64, AgentId)]) -> f64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    match seed {
        Some(s) => s.hash(&mut hasher),
        None => {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
                .hash(&mut hasher);
        }
    }
    for (_, id) in weighted {
        id.hash(&mut hasher);
    }
    // Map the 64-bit hash into [0, 1).
    let v = hasher.finish();
    (v as f64) / (u64::MAX as f64)
}
