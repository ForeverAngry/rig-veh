//! Integration tests for FR-1 (cryptographic identity).

#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use ed25519_dalek::SigningKey;
use rand_core::OsRng;
use rig_compose::AgentManifest;
use rig_veh::{
    AgentArtifact, AllowedScope, CommitInputs, MutationIntent, NodeStatus, sign_node, verify_node,
};

fn manifest(name: &str) -> AgentManifest {
    AgentManifest::from_yaml(&format!("name: {name}\ntools: []\n")).unwrap()
}

#[test]
fn deterministic_agent_id_for_same_inputs() {
    let key = SigningKey::from_bytes(&[7u8; 32]);
    let artifact = AgentArtifact::new(manifest("root"));
    let intent = MutationIntent::new("genesis");
    let scope = AllowedScope::default();

    let inputs = || CommitInputs {
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

    let a = sign_node(&inputs(), &key).unwrap();
    let b = sign_node(&inputs(), &key).unwrap();
    assert_eq!(a.agent_id, b.agent_id);
    assert_eq!(a.signature, b.signature);
}

#[test]
fn tampering_with_status_breaks_verification() {
    let key = SigningKey::generate(&mut OsRng);
    let artifact = AgentArtifact::new(manifest("root"));
    let intent = MutationIntent::new("genesis");
    let scope = AllowedScope::default();
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
    verify_node(&node).unwrap();

    node.status = NodeStatus::Rejected;
    let err = verify_node(&node).unwrap_err();
    assert!(
        matches!(err, rig_veh::Error::HashMismatch { .. }),
        "expected HashMismatch, got {err:?}"
    );
}

#[test]
fn tampering_with_signature_is_rejected() {
    let key = SigningKey::generate(&mut OsRng);
    let artifact = AgentArtifact::new(manifest("root"));
    let intent = MutationIntent::new("genesis");
    let scope = AllowedScope::default();
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
    // Flip one byte of the signature.
    let mut sig_bytes = hex::decode(&node.signature).unwrap();
    sig_bytes[0] ^= 0x01;
    node.signature = hex::encode(sig_bytes);
    let err = verify_node(&node).unwrap_err();
    // The hash is recomputed from payload, not signature, so we expect either
    // HashMismatch or SignatureInvalid depending on which check fires first.
    assert!(
        matches!(
            err,
            rig_veh::Error::HashMismatch { .. } | rig_veh::Error::SignatureInvalid(_)
        ),
        "unexpected error: {err:?}"
    );
}
