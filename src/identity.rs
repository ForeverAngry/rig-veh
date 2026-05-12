//! Cryptographic identity and signing.
//!
//! Implements the hashing rule from §5 of the VEH specification:
//!
//! ```text
//! Agent_ID = SHA256( parent_id || creation_timestamp ||
//!                    mutation_manifest || creator_signature )
//! ```
//!
//! and Ed25519 sign/verify over the canonical artifact + metadata
//! bytes. Signing keys are supplied by the host; this module never
//! generates, persists, or rotates keys on its own.

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use sha2::{Digest, Sha256};

use crate::artifact::{AgentArtifact, canonical_json};
use crate::error::{Error, Result};
use crate::evaluator::EvalStage;
use crate::intent::{AllowedScope, MutationIntent};
use crate::node::{AgentId, AgentNode, NodeStatus};

/// Inputs required to commit a new node to the ledger.
///
/// The struct exists so the hashing rule and signing payload stay in
/// one place and can be reused by tests, examples, and the graph-flow
/// commit task.
#[derive(Debug, Clone)]
pub struct CommitInputs<'a> {
    /// Identifier of the parent node, or `None` for the evolutionary root.
    pub parent_id: Option<&'a AgentId>,
    /// Generation depth (parent `+ 1`, or `0` for the root).
    pub generation: u32,
    /// RFC 3339 timestamp the host wants stamped on the node.
    pub created_at: &'a str,
    /// Intent that produced this artifact.
    pub mutation_intent: &'a MutationIntent,
    /// Mutation diff (unified-diff string) relative to the parent's manifest.
    pub mutation_diff: &'a str,
    /// Scope inherited by descendants.
    pub allowed_scope: &'a AllowedScope,
    /// Evaluator output for non-root nodes.
    pub eval_results: Option<&'a serde_json::Value>,
    /// Candidate artifact being committed.
    pub artifact: &'a AgentArtifact,
    /// Lifecycle status — promoted vs. rejected.
    pub status: NodeStatus,
    /// Whether the mutation resulted in a valid, evaluable artifact.
    pub parent_agent_success: bool,
    /// Whether this node is eligible to be selected as a parent by selectors.
    pub valid_parent: bool,
    /// Which stage promoted this candidate in a staged evaluation.
    pub eval_stage: Option<EvalStage>,
}

impl CommitInputs<'_> {
    /// Build the canonical signing payload — sorted-key JSON of every
    /// field that is hashed *except* `signature` and `agent_id`. The
    /// payload is also used as input to the SHA-256 hash so the hash
    /// and the signature cover the same bytes.
    pub fn signing_payload(&self) -> Result<Vec<u8>> {
        let mut payload = serde_json::Map::new();
        payload.insert(
            "parent_id".into(),
            self.parent_id
                .map(|s| serde_json::Value::String(s.clone()))
                .unwrap_or(serde_json::Value::Null),
        );
        payload.insert("generation".into(), self.generation.into());
        payload.insert("created_at".into(), self.created_at.into());
        payload.insert(
            "mutation_intent".into(),
            serde_json::to_value(self.mutation_intent)?,
        );
        payload.insert("mutation_diff".into(), self.mutation_diff.into());
        payload.insert(
            "allowed_scope".into(),
            serde_json::to_value(self.allowed_scope)?,
        );
        payload.insert(
            "eval_results".into(),
            self.eval_results
                .cloned()
                .unwrap_or(serde_json::Value::Null),
        );
        payload.insert(
            "capability_sbom".into(),
            serde_json::to_value(self.artifact.capability_sbom())?,
        );
        payload.insert(
            "manifest".into(),
            serde_json::to_value(&self.artifact.manifest)?,
        );
        payload.insert("status".into(), serde_json::to_value(self.status)?);
        payload.insert(
            "parent_agent_success".into(),
            self.parent_agent_success.into(),
        );
        payload.insert("valid_parent".into(), self.valid_parent.into());
        payload.insert(
            "eval_stage".into(),
            self.eval_stage
                .map(|s| serde_json::to_value(s).unwrap_or(serde_json::Value::Null))
                .unwrap_or(serde_json::Value::Null),
        );
        canonical_json(&serde_json::Value::Object(payload))
    }
}

/// Compute the SHA-256 `agent_id` for the supplied payload + signature.
///
/// The signature is included in the hash input per spec §5 so the
/// identifier binds both content and creator. Returned as a lowercase
/// hex string.
pub fn compute_agent_id(payload: &[u8], signature: &Signature) -> AgentId {
    let mut hasher = Sha256::new();
    hasher.update(payload);
    hasher.update(signature.to_bytes());
    hex::encode(hasher.finalize())
}

/// Sign and seal a [`CommitInputs`] into a complete [`AgentNode`].
pub fn sign_node(inputs: &CommitInputs<'_>, signing_key: &SigningKey) -> Result<AgentNode> {
    let payload = inputs.signing_payload()?;
    let signature = signing_key.sign(&payload);
    let agent_id = compute_agent_id(&payload, &signature);
    let verifying = signing_key.verifying_key();

    Ok(AgentNode {
        agent_id,
        parent_id: inputs.parent_id.cloned(),
        generation: inputs.generation,
        created_at: inputs.created_at.to_string(),
        mutation_intent: inputs.mutation_intent.clone(),
        capability_sbom: inputs.artifact.capability_sbom(),
        eval_results: inputs.eval_results.cloned(),
        mutation_diff: inputs.mutation_diff.to_string(),
        allowed_scope: inputs.allowed_scope.clone(),
        artifact: inputs.artifact.clone(),
        signer_public_key: hex::encode(verifying.to_bytes()),
        signature: hex::encode(signature.to_bytes()),
        status: inputs.status,
        parent_agent_success: inputs.parent_agent_success,
        valid_parent: inputs.valid_parent,
        eval_stage: inputs.eval_stage,
    })
}

/// Verify both the SHA-256 identifier and the Ed25519 signature on a
/// stored [`AgentNode`].
///
/// Returns [`Error::HashMismatch`] when the recorded `agent_id` does
/// not match the recomputed hash, and [`Error::SignatureInvalid`] when
/// the signature does not verify under the recorded public key.
pub fn verify_node(node: &AgentNode) -> Result<()> {
    let inputs = CommitInputs {
        parent_id: node.parent_id.as_ref(),
        generation: node.generation,
        created_at: &node.created_at,
        mutation_intent: &node.mutation_intent,
        mutation_diff: &node.mutation_diff,
        allowed_scope: &node.allowed_scope,
        eval_results: node.eval_results.as_ref(),
        artifact: &node.artifact,
        status: node.status,
        parent_agent_success: node.parent_agent_success,
        valid_parent: node.valid_parent,
        eval_stage: node.eval_stage,
    };
    let payload = inputs.signing_payload()?;

    let signature_bytes = hex::decode(&node.signature)
        .map_err(|e| Error::SignatureInvalid(format!("hex decode: {e}")))?;
    let signature_array: [u8; Signature::BYTE_SIZE] = signature_bytes
        .as_slice()
        .try_into()
        .map_err(|_| Error::SignatureInvalid("wrong signature length".into()))?;
    let signature = Signature::from_bytes(&signature_array);

    let expected_id = compute_agent_id(&payload, &signature);
    if expected_id != node.agent_id {
        return Err(Error::HashMismatch {
            expected: node.agent_id.clone(),
            actual: expected_id,
        });
    }

    let pk_bytes = hex::decode(&node.signer_public_key)
        .map_err(|e| Error::InvalidKey(format!("hex decode: {e}")))?;
    let pk_array: [u8; 32] = pk_bytes
        .as_slice()
        .try_into()
        .map_err(|_| Error::InvalidKey("wrong key length".into()))?;
    let verifying =
        VerifyingKey::from_bytes(&pk_array).map_err(|e| Error::InvalidKey(e.to_string()))?;

    verifying
        .verify(&payload, &signature)
        .map_err(|e| Error::SignatureInvalid(e.to_string()))?;

    Ok(())
}

/// Decode a hex-encoded Ed25519 verifying key.
pub fn decode_verifying_key(hex_str: &str) -> Result<VerifyingKey> {
    let bytes = hex::decode(hex_str).map_err(|e| Error::InvalidKey(e.to_string()))?;
    let array: [u8; 32] = bytes
        .as_slice()
        .try_into()
        .map_err(|_| Error::InvalidKey("wrong key length".into()))?;
    VerifyingKey::from_bytes(&array).map_err(|e| Error::InvalidKey(e.to_string()))
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
    use ed25519_dalek::SigningKey;
    use rand_core::OsRng;
    use rig_compose::AgentManifest;

    fn fresh_artifact() -> AgentArtifact {
        let yaml = r#"
name: root
tools: []
"#;
        AgentArtifact::new(AgentManifest::from_yaml(yaml).unwrap())
    }

    #[test]
    fn sign_then_verify_round_trip() {
        let key = SigningKey::generate(&mut OsRng);
        let intent = MutationIntent::new("genesis");
        let scope = AllowedScope::default();
        let artifact = fresh_artifact();
        let inputs = CommitInputs {
            parent_id: None,
            generation: 0,
            created_at: "2026-05-11T00:00:00Z",
            mutation_intent: &intent,
            mutation_diff: "",
            allowed_scope: &scope,
            eval_results: None,
            artifact: &artifact,
            status: NodeStatus::Promoted,
            parent_agent_success: true,
            valid_parent: true,
            eval_stage: None,
        };
        let node = sign_node(&inputs, &key).unwrap();
        verify_node(&node).unwrap();
    }

    #[test]
    fn tampering_with_artifact_fails_verification() {
        let key = SigningKey::generate(&mut OsRng);
        let intent = MutationIntent::new("genesis");
        let scope = AllowedScope::default();
        let artifact = fresh_artifact();
        let inputs = CommitInputs {
            parent_id: None,
            generation: 0,
            created_at: "2026-05-11T00:00:00Z",
            mutation_intent: &intent,
            mutation_diff: "",
            allowed_scope: &scope,
            eval_results: None,
            artifact: &artifact,
            status: NodeStatus::Promoted,
            parent_agent_success: true,
            valid_parent: true,
            eval_stage: None,
        };
        let mut node = sign_node(&inputs, &key).unwrap();
        node.artifact.manifest.name = Some("tampered".into());
        let err = verify_node(&node).expect_err("must fail");
        // Either the hash mismatch or signature failure is acceptable —
        // both prove the tamper was detected.
        match err {
            Error::HashMismatch { .. } | Error::SignatureInvalid(_) => {}
            other => panic!("unexpected error: {other}"),
        }
    }
}
