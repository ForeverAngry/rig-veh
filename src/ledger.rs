//! Lineage DAG storage.
//!
//! [`LineageStore`] is the abstract ledger; v0.1 ships
//! [`InMemoryLineage`] (always available) and [`JsonlLineage`]
//! (file-backed, gated by feature `jsonl-ledger`).
//!
//! ## Locking
//!
//! In-memory implementations use [`parking_lot::RwLock`] guards that
//! are **scope-dropped before any `.await`** to honour the crate's
//! `clippy::await_holding_lock = deny` rule.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::RwLock;

use crate::error::{Error, Result};
use crate::node::{AgentId, AgentNode};

/// Pluggable lineage ledger.
#[async_trait]
pub trait LineageStore: Send + Sync {
    /// Append a node. Implementations must reject duplicate
    /// `agent_id`s with [`Error::Ledger`].
    async fn append(&self, node: AgentNode) -> Result<()>;

    /// Fetch a node by id.
    async fn get(&self, id: &AgentId) -> Result<AgentNode>;

    /// Return the current head (most recently promoted node), if any.
    async fn head(&self) -> Result<Option<AgentId>>;

    /// Atomically set the head pointer — used by [`crate::Veh::rollback_to`]
    /// to revert. Implementations must verify the target exists.
    async fn set_head(&self, id: &AgentId) -> Result<()>;

    /// Export the full DAG as a JSON-serialisable structure. Used to
    /// satisfy NFR-1 (auditability).
    async fn export_dag_json(&self) -> Result<serde_json::Value>;

    /// Return every node in insertion order. Default implementation
    /// decodes [`LineageStore::export_dag_json`] so existing custom
    /// stores continue to work without overrides; in-tree stores
    /// override this with a cheaper path.
    async fn nodes(&self) -> Result<Vec<AgentNode>> {
        let dag = self.export_dag_json().await?;
        let nodes = dag
            .get("nodes")
            .cloned()
            .ok_or_else(|| Error::Ledger("dag export missing nodes".into()))?;
        let parsed: Vec<AgentNode> = serde_json::from_value(nodes)?;
        Ok(parsed)
    }
}

/// Process-local lineage ledger. Cheap to clone via `Arc`.
#[derive(Default, Clone)]
pub struct InMemoryLineage {
    inner: Arc<RwLock<InMemoryInner>>,
}

#[derive(Default)]
struct InMemoryInner {
    nodes: HashMap<AgentId, AgentNode>,
    insertion_order: Vec<AgentId>,
    head: Option<AgentId>,
}

impl InMemoryLineage {
    /// Construct an empty ledger.
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl LineageStore for InMemoryLineage {
    async fn append(&self, node: AgentNode) -> Result<()> {
        let id = node.agent_id.clone();
        let status = node.status;
        // Drop the guard *before* returning so we never hold it across `.await`.
        {
            let mut guard = self.inner.write();
            if guard.nodes.contains_key(&id) {
                return Err(Error::Ledger(format!("duplicate agent_id {id}")));
            }
            guard.insertion_order.push(id.clone());
            guard.nodes.insert(id.clone(), node);
            if matches!(status, crate::node::NodeStatus::Promoted) {
                guard.head = Some(id);
            }
        }
        Ok(())
    }

    async fn get(&self, id: &AgentId) -> Result<AgentNode> {
        let guard = self.inner.read();
        guard
            .nodes
            .get(id)
            .cloned()
            .ok_or_else(|| Error::NotFound(id.clone()))
    }

    async fn head(&self) -> Result<Option<AgentId>> {
        Ok(self.inner.read().head.clone())
    }

    async fn set_head(&self, id: &AgentId) -> Result<()> {
        let mut guard = self.inner.write();
        if !guard.nodes.contains_key(id) {
            return Err(Error::NotFound(id.clone()));
        }
        guard.head = Some(id.clone());
        Ok(())
    }

    async fn export_dag_json(&self) -> Result<serde_json::Value> {
        let guard = self.inner.read();
        let nodes: Vec<&AgentNode> = guard
            .insertion_order
            .iter()
            .filter_map(|id| guard.nodes.get(id))
            .collect();
        Ok(serde_json::json!({
            "head": guard.head,
            "nodes": nodes,
        }))
    }

    async fn nodes(&self) -> Result<Vec<AgentNode>> {
        let guard = self.inner.read();
        Ok(guard
            .insertion_order
            .iter()
            .filter_map(|id| guard.nodes.get(id).cloned())
            .collect())
    }
}

/// Append-only JSONL file ledger.
#[cfg(feature = "jsonl-ledger")]
pub struct JsonlLineage {
    path: std::path::PathBuf,
    inner: Arc<RwLock<InMemoryInner>>,
}

#[cfg(feature = "jsonl-ledger")]
impl JsonlLineage {
    /// Open (or create) a JSONL ledger at `path`. Existing lines are
    /// replayed into memory.
    pub fn open(path: impl Into<std::path::PathBuf>) -> Result<Self> {
        use std::io::BufRead;
        let path = path.into();
        let mut inner = InMemoryInner::default();
        if path.exists() {
            let file = std::fs::File::open(&path)?;
            let reader = std::io::BufReader::new(file);
            for line in reader.lines() {
                let line = line?;
                if line.trim().is_empty() {
                    continue;
                }
                let node: AgentNode = serde_json::from_str(&line)?;
                let id = node.agent_id.clone();
                let status = node.status;
                inner.insertion_order.push(id.clone());
                if matches!(status, crate::node::NodeStatus::Promoted) {
                    inner.head = Some(id.clone());
                }
                inner.nodes.insert(id, node);
            }
        }
        Ok(Self {
            path,
            inner: Arc::new(RwLock::new(inner)),
        })
    }
}

#[cfg(feature = "jsonl-ledger")]
#[async_trait]
impl LineageStore for JsonlLineage {
    async fn append(&self, node: AgentNode) -> Result<()> {
        use std::io::Write as _;
        let serialised = serde_json::to_string(&node)?;
        let id = node.agent_id.clone();
        let status = node.status;
        {
            let mut guard = self.inner.write();
            if guard.nodes.contains_key(&id) {
                return Err(Error::Ledger(format!("duplicate agent_id {id}")));
            }
            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&self.path)?;
            writeln!(file, "{serialised}")?;
            guard.insertion_order.push(id.clone());
            guard.nodes.insert(id.clone(), node);
            if matches!(status, crate::node::NodeStatus::Promoted) {
                guard.head = Some(id);
            }
        }
        Ok(())
    }

    async fn get(&self, id: &AgentId) -> Result<AgentNode> {
        let guard = self.inner.read();
        guard
            .nodes
            .get(id)
            .cloned()
            .ok_or_else(|| Error::NotFound(id.clone()))
    }

    async fn head(&self) -> Result<Option<AgentId>> {
        Ok(self.inner.read().head.clone())
    }

    async fn set_head(&self, id: &AgentId) -> Result<()> {
        let mut guard = self.inner.write();
        if !guard.nodes.contains_key(id) {
            return Err(Error::NotFound(id.clone()));
        }
        guard.head = Some(id.clone());
        Ok(())
    }

    async fn export_dag_json(&self) -> Result<serde_json::Value> {
        let guard = self.inner.read();
        let nodes: Vec<&AgentNode> = guard
            .insertion_order
            .iter()
            .filter_map(|id| guard.nodes.get(id))
            .collect();
        Ok(serde_json::json!({
            "head": guard.head,
            "nodes": nodes,
        }))
    }

    async fn nodes(&self) -> Result<Vec<AgentNode>> {
        let guard = self.inner.read();
        Ok(guard
            .insertion_order
            .iter()
            .filter_map(|id| guard.nodes.get(id).cloned())
            .collect())
    }
}

/// Render a lineage ledger as a Graphviz DOT graph.
///
/// Behind `dot-export`. The output is suitable for `dot -Tpng` or any
/// other Graphviz pipeline.
#[cfg(feature = "dot-export")]
pub async fn export_dot(store: &dyn LineageStore) -> Result<String> {
    let dag = store.export_dag_json().await?;
    let nodes = dag
        .get("nodes")
        .and_then(|v| v.as_array())
        .ok_or_else(|| Error::Ledger("dag export missing nodes".into()))?;
    let mut out = String::new();
    out.push_str("digraph lineage {\n");
    for node in nodes {
        let id = node.get("agent_id").and_then(|v| v.as_str()).unwrap_or("?");
        let generation = node.get("generation").and_then(|v| v.as_u64()).unwrap_or(0);
        let status = node
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or("promoted");
        out.push_str(&format!(
            "  \"{id}\" [label=\"gen={generation}\\n{status}\\n{short}\"];\n",
            short = &id.chars().take(8).collect::<String>(),
        ));
        if let Some(parent) = node.get("parent_id").and_then(|v| v.as_str()) {
            out.push_str(&format!("  \"{parent}\" -> \"{id}\";\n"));
        }
    }
    out.push_str("}\n");
    Ok(out)
}
