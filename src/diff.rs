//! Unified diff between two agent manifests.
//!
//! Used to populate [`crate::node::AgentNode::mutation_diff`] so the
//! ledger records exactly what changed between a parent and its
//! candidate.

use similar::TextDiff;

use crate::artifact::AgentArtifact;
use crate::error::Result;

/// Produce a unified-diff string of two artifacts' canonical JSON.
///
/// The output is intended for human review; it is not part of the
/// hash payload (the canonical artifact bytes already are) so its
/// exact format may evolve.
pub fn manifest_diff(parent: &AgentArtifact, candidate: &AgentArtifact) -> Result<String> {
    let parent_bytes = parent.canonical_bytes()?;
    let candidate_bytes = candidate.canonical_bytes()?;
    let parent_str = String::from_utf8_lossy(&parent_bytes);
    let candidate_str = String::from_utf8_lossy(&candidate_bytes);
    let diff = TextDiff::from_lines(parent_str.as_ref(), candidate_str.as_ref());
    Ok(diff
        .unified_diff()
        .context_radius(3)
        .header("parent", "candidate")
        .to_string())
}
