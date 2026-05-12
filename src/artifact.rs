//! Canonical agent artifact.
//!
//! [`AgentArtifact`] wraps a [`rig_compose::AgentManifest`] and exposes
//! a deterministic byte representation suitable for hashing. The
//! canonical form is sorted-key JSON with no insignificant whitespace,
//! so two artifacts with semantically-identical contents produce
//! identical bytes regardless of how they were constructed.

use rig_compose::AgentManifest;
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// An immutable, hashable wrapper around an [`AgentManifest`].
///
/// The wrapper exists so that the manifest schema in `rig-compose`
/// stays the source of truth — `rig-veh` does not redefine tool /
/// delegate / instructions shapes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentArtifact {
    /// The composable agent definition.
    pub manifest: AgentManifest,
}

impl AgentArtifact {
    /// Wrap an existing manifest.
    pub fn new(manifest: AgentManifest) -> Self {
        Self { manifest }
    }

    /// Deterministic byte representation used for hashing.
    ///
    /// Implemented as sorted-key canonical JSON via the
    /// [`canonical_json`] helper. Two artifacts that differ only in
    /// HashMap iteration order produce identical canonical bytes.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>> {
        canonical_json(&serde_json::to_value(&self.manifest)?)
    }

    /// Capability SBOM — the manifest of all tools, MCP servers, and
    /// delegates this artifact is allowed to invoke. Produced as a
    /// sorted, deduplicated list so it is stable across runs.
    pub fn capability_sbom(&self) -> Vec<String> {
        let mut out = Vec::new();
        for t in &self.manifest.tools {
            match t {
                rig_compose::ToolSpec::Local { name } => {
                    out.push(format!("tool:{name}"));
                }
            }
        }
        for m in &self.manifest.mcp_servers {
            out.push(format!("mcp:{}", m.name));
        }
        for d in &self.manifest.delegates {
            out.push(format!("delegate:{}", d.name));
        }
        out.sort();
        out.dedup();
        out
    }
}

/// Serialise a [`serde_json::Value`] to canonical bytes: keys sorted
/// lexicographically, no whitespace, numbers in their JSON-numeric
/// form. Required so [`AgentArtifact::canonical_bytes`] is
/// byte-deterministic across platforms and serde versions.
pub fn canonical_json(value: &serde_json::Value) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    write_canonical(value, &mut out).map_err(|e| Error::Canonical(e.to_string()))?;
    Ok(out)
}

fn write_canonical(value: &serde_json::Value, out: &mut Vec<u8>) -> std::io::Result<()> {
    use std::io::Write as _;
    match value {
        serde_json::Value::Null => out.write_all(b"null"),
        serde_json::Value::Bool(b) => out.write_all(if *b { b"true" } else { b"false" }),
        serde_json::Value::Number(n) => write!(out, "{n}"),
        serde_json::Value::String(s) => {
            // serde_json knows how to escape strings safely.
            let encoded = serde_json::to_string(s).map_err(std::io::Error::other)?;
            out.write_all(encoded.as_bytes())
        }
        serde_json::Value::Array(items) => {
            out.write_all(b"[")?;
            for (idx, item) in items.iter().enumerate() {
                if idx > 0 {
                    out.write_all(b",")?;
                }
                write_canonical(item, out)?;
            }
            out.write_all(b"]")
        }
        serde_json::Value::Object(map) => {
            out.write_all(b"{")?;
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            for (idx, key) in keys.into_iter().enumerate() {
                if idx > 0 {
                    out.write_all(b",")?;
                }
                let encoded_key = serde_json::to_string(key).map_err(std::io::Error::other)?;
                out.write_all(encoded_key.as_bytes())?;
                out.write_all(b":")?;
                let v = map.get(key).ok_or_else(|| {
                    std::io::Error::other("canonical_json: key vanished mid-iteration")
                })?;
                write_canonical(v, out)?;
            }
            out.write_all(b"}")
        }
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::expect_used
)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn canonical_json_sorts_keys() {
        let a = json!({ "b": 1, "a": 2 });
        let b = json!({ "a": 2, "b": 1 });
        assert_eq!(canonical_json(&a).unwrap(), canonical_json(&b).unwrap());
    }

    #[test]
    fn canonical_json_no_whitespace() {
        let v = json!({ "k": [1, 2, 3] });
        assert_eq!(canonical_json(&v).unwrap(), b"{\"k\":[1,2,3]}");
    }

    #[test]
    fn artifact_capability_sbom_is_sorted() {
        let yaml = r#"
name: demo
tools:
  - kind: local
    name: zeta
  - kind: local
    name: alpha
mcp_servers:
  - name: mcp_b
    command: [ "x" ]
"#;
        let manifest = AgentManifest::from_yaml(yaml).unwrap();
        let artifact = AgentArtifact::new(manifest);
        let sbom = artifact.capability_sbom();
        assert_eq!(sbom, vec!["mcp:mcp_b", "tool:alpha", "tool:zeta"]);
    }

    #[test]
    fn artifact_canonical_bytes_are_yaml_key_order_independent() {
        // Two manifests that differ only in YAML key order must produce
        // byte-identical canonical bytes. This is the load-bearing
        // invariant for AgentNode hashing and Ed25519 signing: any future
        // serde/serde_yaml change that lets insertion order leak into the
        // hashed payload would break ledger reproducibility silently.
        let yaml_a = r#"
name: demo
tools:
  - kind: local
    name: alpha
  - kind: local
    name: zeta
mcp_servers:
  - name: mcp_b
    command: [ "x" ]
"#;
        let yaml_b = r#"
mcp_servers:
  - command: [ "x" ]
    name: mcp_b
tools:
  - name: alpha
    kind: local
  - name: zeta
    kind: local
name: demo
"#;
        let a = AgentArtifact::new(AgentManifest::from_yaml(yaml_a).unwrap());
        let b = AgentArtifact::new(AgentManifest::from_yaml(yaml_b).unwrap());
        let bytes_a = a.canonical_bytes().unwrap();
        let bytes_b = b.canonical_bytes().unwrap();
        assert_eq!(
            bytes_a, bytes_b,
            "canonical bytes diverged across YAML key orderings — hash/signature \
             reproducibility is broken"
        );
    }

    #[test]
    fn canonical_json_is_object_insertion_order_independent() {
        // Property-style: repeatedly build a serde_json::Value with shuffled
        // key insertion order — same (key → value) bindings every time, only
        // the order changes — and confirm canonical_json output is stable.
        let bindings = [
            ("alpha", 1_i64),
            ("beta", 2),
            ("gamma", 3),
            ("delta", 4),
            ("epsilon", 5),
        ];
        let mut reference: Option<Vec<u8>> = None;
        for shift in 0..bindings.len() {
            let mut map = serde_json::Map::new();
            for offset in 0..bindings.len() {
                let (k, v) = bindings[(shift + offset) % bindings.len()];
                map.insert(k.to_string(), json!(v));
            }
            let nested = json!({
                "outer": serde_json::Value::Object(map),
                "list": [3, 1, 2],
            });
            let bytes = canonical_json(&nested).unwrap();
            match &reference {
                None => reference = Some(bytes),
                Some(r) => assert_eq!(
                    &bytes, r,
                    "canonical_json output drifted under insertion-order rotation {shift}",
                ),
            }
        }
    }
}
