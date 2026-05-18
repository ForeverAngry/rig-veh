#![allow(clippy::unwrap_used, clippy::expect_used)]

use ed25519_dalek::SigningKey;
use rand_core::OsRng;
use rig_compose::AgentManifest;
use rig_veh::{
    AgentArtifact, AllowedScope, CycleInputs, InMemoryLineage, MutationIntent, StubEvaluator, Veh,
    export_dot,
};
use std::sync::Arc;

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let ledger = Arc::new(InMemoryLineage::new());
    let evaluator = Arc::new(StubEvaluator::new(0.8));
    let key = SigningKey::generate(&mut OsRng);
    let veh = Veh::new(ledger.clone(), evaluator, key);

    let manifest = AgentManifest::from_yaml("name: root\ntools: []\n").unwrap();
    let root = veh
        .commit_root(
            AgentArtifact::new(manifest),
            AllowedScope::default(),
            "2026-05-11T00:00:00Z",
        )
        .await?;

    let candidate_manifest = AgentManifest::from_yaml("name: v2\ntools: []\n").unwrap();
    let _outcome = veh
        .run_cycle(CycleInputs {
            parent: &root,
            intent: MutationIntent::new("tighten_preamble"),
            candidate: AgentArtifact::new(candidate_manifest),
            allowed_scope: AllowedScope::default(),
            created_at: "2026-05-11T00:01:00Z".into(),
        })
        .await?;

    let dot = export_dot(ledger.as_ref()).await?;
    println!("{}", dot);

    Ok(())
}
