//! Byte-exact snapshot test for VEH's canonical JSON / signing payload.
//!
//! Per [`AGENTS.md`](../AGENTS.md): **canonical JSON is load-bearing**.
//! Any change to the on-disk shape of [`rig_veh::AgentNode`] or the
//! field set in [`rig_veh::CommitInputs::signing_payload`] is a
//! hash-breaking change for every existing v0.1 / v0.2 ledger.
//!
//! This test pins:
//!
//! 1. The exact bytes of `CommitInputs::signing_payload()` for a fixed
//!    fixture.
//! 2. The resulting Ed25519 signature and SHA-256 `agent_id` (both
//!    derived from the bytes above plus a fixed signing key seed).
//! 3. The full canonical-JSON serialisation of the produced
//!    `AgentNode`.
//!
//! If you intentionally change canonical layout, bump the crate to a
//! new major version, document the change in
//! [`CHANGELOG.md`](../CHANGELOG.md), and refresh the expected bytes
//! below in the same commit.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use ed25519_dalek::SigningKey;
use rig_compose::AgentManifest;
use rig_veh::{
    AgentArtifact, AllowedScope, CommitInputs, MutationIntent, NodeStatus, sign_node, verify_node,
};

/// Fixed signing-key seed — chosen once, never to be changed without a
/// major-version bump.
const SEED: [u8; 32] = [7u8; 32];

/// Fixed RFC 3339 timestamp used in the fixture. Frozen so the
/// snapshot is reproducible.
const CREATED_AT: &str = "2026-01-01T00:00:00Z";

fn fixture_artifact() -> AgentArtifact {
    AgentArtifact::new(AgentManifest::from_yaml("name: root\ntools: []\n").unwrap())
}

fn fixture_inputs<'a>(
    intent: &'a MutationIntent,
    scope: &'a AllowedScope,
    artifact: &'a AgentArtifact,
) -> CommitInputs<'a> {
    CommitInputs {
        parent_id: None,
        generation: 0,
        created_at: CREATED_AT,
        mutation_intent: intent,
        mutation_diff: "",
        allowed_scope: scope,
        eval_results: None,
        artifact,
        status: NodeStatus::Promoted,
        parent_agent_success: true,
        valid_parent: true,
        eval_stage: None,
    }
}

/// Expected canonical signing-payload bytes for the fixture.
///
/// To regenerate after an intentional canonical-layout change:
/// `cargo test -p rig-veh --test canonical_snapshot -- --nocapture` will
/// print the actual bytes when a mismatch occurs.
const EXPECTED_SIGNING_PAYLOAD: &str = concat!(
    "{\"allowed_scope\":{\"allowed_delegates\":[],\"allowed_mcp_servers\":[],",
    "\"allowed_tools\":[],\"gated_capabilities\":[],\"max_generation\":null},",
    "\"capability_sbom\":[],\"created_at\":\"2026-01-01T00:00:00Z\",",
    "\"eval_results\":null,\"eval_stage\":null,\"generation\":0,",
    "\"manifest\":{\"delegates\":[],\"instructions\":null,\"knowledge\":null,",
    "\"mcp_servers\":[],\"model\":null,\"name\":\"root\",\"tools\":[]},",
    "\"mutation_diff\":\"\",\"mutation_intent\":{\"constraints\":[],",
    "\"expected_improvement\":null,\"goal\":\"genesis\",\"metadata\":null,",
    "\"rationale\":\"\"},\"parent_agent_success\":true,\"parent_id\":null,",
    "\"status\":\"promoted\",\"valid_parent\":true}",
);

/// Expected SHA-256 `agent_id` (lowercase hex) for the fixture.
const EXPECTED_AGENT_ID: &str = "d75a8c0e5eb9e5042896999c710c103193bb48cbb94cdabe7f7ed095b65c8dd0";

/// Expected hex-encoded Ed25519 signature for the fixture.
const EXPECTED_SIGNATURE: &str = concat!(
    "bc22603587a6bccb97e7a85b7a73aca10939a371ce91ee2b3896f4b39a01f291",
    "949aa0cb899c0c29221ee18758e2972e0f7088909b4d37f78877c3710d1c6701",
);

/// Expected canonical-JSON serialisation of the produced `AgentNode`.
const EXPECTED_NODE_JSON: &str = concat!(
    "{\"agent_id\":\"d75a8c0e5eb9e5042896999c710c103193bb48cbb94cdabe7f7ed095b65c8dd0\",",
    "\"allowed_scope\":{\"allowed_delegates\":[],\"allowed_mcp_servers\":[],",
    "\"allowed_tools\":[],\"gated_capabilities\":[],\"max_generation\":null},",
    "\"artifact\":{\"manifest\":{\"delegates\":[],\"instructions\":null,",
    "\"knowledge\":null,\"mcp_servers\":[],\"model\":null,\"name\":\"root\",\"tools\":[]}},",
    "\"capability_sbom\":[],\"created_at\":\"2026-01-01T00:00:00Z\",",
    "\"eval_results\":null,\"eval_stage\":null,\"generation\":0,\"mutation_diff\":\"\",",
    "\"mutation_intent\":{\"constraints\":[],\"expected_improvement\":null,",
    "\"goal\":\"genesis\",\"metadata\":null,\"rationale\":\"\"},",
    "\"parent_agent_success\":true,\"parent_id\":null,",
    "\"signature\":\"bc22603587a6bccb97e7a85b7a73aca10939a371ce91ee2b3896f4b39a01f291",
    "949aa0cb899c0c29221ee18758e2972e0f7088909b4d37f78877c3710d1c6701\",",
    "\"signer_public_key\":\"ea4a6c63e29c520abef5507b132ec5f9954776aebebe7b92421eea691446d22c\",",
    "\"status\":\"promoted\",\"valid_parent\":true}",
);

#[test]
fn signing_payload_matches_pinned_snapshot() {
    let intent = MutationIntent::new("genesis");
    let scope = AllowedScope::default();
    let artifact = fixture_artifact();
    let inputs = fixture_inputs(&intent, &scope, &artifact);

    let payload = inputs.signing_payload().unwrap();
    let payload_str = std::str::from_utf8(&payload).unwrap();

    if payload_str != EXPECTED_SIGNING_PAYLOAD {
        eprintln!("--- actual signing payload ---");
        eprintln!("{payload_str}");
        eprintln!("--- end ---");
    }
    assert_eq!(
        payload_str, EXPECTED_SIGNING_PAYLOAD,
        "canonical signing payload drifted; see stderr for actual bytes"
    );
}

#[test]
fn signed_node_matches_pinned_snapshot() {
    let key = SigningKey::from_bytes(&SEED);
    let intent = MutationIntent::new("genesis");
    let scope = AllowedScope::default();
    let artifact = fixture_artifact();
    let inputs = fixture_inputs(&intent, &scope, &artifact);

    let node = sign_node(&inputs, &key).unwrap();
    verify_node(&node).expect("snapshot fixture must verify");

    if node.agent_id != EXPECTED_AGENT_ID {
        eprintln!("--- actual agent_id ---\n{}\n--- end ---", node.agent_id);
    }
    if node.signature != EXPECTED_SIGNATURE {
        eprintln!("--- actual signature ---\n{}\n--- end ---", node.signature);
    }

    assert_eq!(node.agent_id, EXPECTED_AGENT_ID, "agent_id drifted");
    assert_eq!(node.signature, EXPECTED_SIGNATURE, "signature drifted");

    // Canonical-JSON of the full node — pins serde-field order
    // independent of the in-memory struct layout.
    let value = serde_json::to_value(&node).unwrap();
    let node_json = canonicalize(&value);

    if node_json != EXPECTED_NODE_JSON {
        eprintln!("--- actual node json ---\n{node_json}\n--- end ---");
    }
    assert_eq!(node_json, EXPECTED_NODE_JSON, "canonical node JSON drifted");
}

/// Local re-implementation of `rig_veh::artifact::canonical_json` so
/// the test does not depend on a (currently `pub(crate)`) helper. Kept
/// byte-compatible with the production canonicaliser — if you change
/// `write_canonical` in `src/artifact.rs`, mirror the change here.
fn canonicalize(value: &serde_json::Value) -> String {
    let mut out = String::new();
    write(value, &mut out);
    out
}

fn write(value: &serde_json::Value, out: &mut String) {
    use std::fmt::Write as _;
    match value {
        serde_json::Value::Null => out.push_str("null"),
        serde_json::Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        serde_json::Value::Number(n) => {
            let _ = write!(out, "{n}");
        }
        serde_json::Value::String(s) => {
            let _ = write!(out, "{}", serde_json::to_string(s).unwrap());
        }
        serde_json::Value::Array(items) => {
            out.push('[');
            for (idx, item) in items.iter().enumerate() {
                if idx > 0 {
                    out.push(',');
                }
                write(item, out);
            }
            out.push(']');
        }
        serde_json::Value::Object(map) => {
            out.push('{');
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            for (idx, key) in keys.into_iter().enumerate() {
                if idx > 0 {
                    out.push(',');
                }
                let _ = write!(out, "{}", serde_json::to_string(key).unwrap());
                out.push(':');
                if let Some(v) = map.get(key) {
                    write(v, out);
                }
            }
            out.push('}');
        }
    }
}
