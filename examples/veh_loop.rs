//! End-to-end VEH loop: genesis → mutate → policy gate → promote.
//!
//! Run with:
//!
//! ```sh
//! cargo run --example veh_loop
//! ```

use std::sync::Arc;

use ed25519_dalek::SigningKey;
use rand_core::OsRng;
use rig_compose::AgentManifest;
use rig_veh::{
    AgentArtifact, AllowedScope, CycleInputs, CycleOutcome, InMemoryLineage, LineageStore,
    MutationIntent, StubEvaluator, Veh,
};

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info,rig_veh=debug")),
        )
        .init();

    let ledger = Arc::new(InMemoryLineage::new());
    let evaluator = Arc::new(StubEvaluator::new(0.65));
    let key = SigningKey::generate(&mut OsRng);
    let veh = Veh::new(ledger.clone(), evaluator, key).with_strict_improvement(false);

    let root_manifest = AgentManifest::from_yaml(
        "name: root_agent
tools:
  - kind: local
    name: search
",
    )?;
    let root = veh
        .commit_root(
            AgentArtifact::new(root_manifest),
            AllowedScope {
                allowed_tools: vec!["search".into(), "summarize".into()],
                ..AllowedScope::default()
            },
            "2026-05-11T00:00:00Z",
        )
        .await?;
    println!("root agent_id   = {}", root.agent_id);

    let candidate_manifest = AgentManifest::from_yaml(
        "name: candidate
tools:
  - kind: local
    name: search
  - kind: local
    name: summarize
",
    )?;
    let outcome = veh
        .run_cycle(CycleInputs {
            parent: &root,
            intent: MutationIntent::new("add_summarize_tool")
                .with_rationale("compress long retrieval results before answer synthesis")
                .with_expected_improvement(0.05),
            candidate: AgentArtifact::new(candidate_manifest),
            allowed_scope: AllowedScope {
                allowed_tools: vec!["search".into(), "summarize".into()],
                ..AllowedScope::default()
            },
            created_at: "2026-05-11T00:01:00Z".into(),
        })
        .await?;

    match outcome {
        CycleOutcome::Promoted { node, eval } => {
            println!("promoted        = {} (score {})", node.agent_id, eval.score);
        }
        CycleOutcome::Rejected { node, reason } => {
            println!("rejected        = {} ({reason})", node.agent_id);
        }
        CycleOutcome::AwaitingApproval(pending) => {
            println!("awaiting approval: {}", pending.reason());
            let approver = SigningKey::generate(&mut OsRng);
            let promoted = veh.resume_with_approval(pending, &approver).await?;
            println!("approved        = {}", promoted.agent_id);
        }
    }

    println!("head            = {:?}", ledger.head().await?);
    Ok(())
}
