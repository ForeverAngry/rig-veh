//! Live Ollama smoke test for a host-owned VEH evaluator.
//!
//! Run with:
//!
//! ```sh
//! OLLAMA_MODEL=qwen3.5:9b cargo test --test live_ollama -- --ignored --nocapture
//! ```

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::process::Command;
use std::sync::Arc;

use async_trait::async_trait;
use ed25519_dalek::SigningKey;
use rand_core::OsRng;
use rig_compose::AgentManifest;
use rig_veh::{
    AgentArtifact, AllowedScope, CycleInputs, CycleOutcome, Error, EvalResult, Evaluator,
    InMemoryLineage, LineageStore, MutationIntent, Result, Veh, verify_node,
};

struct OllamaEvaluator {
    model: String,
}

#[async_trait]
impl Evaluator for OllamaEvaluator {
    async fn evaluate(&self, artifact: &AgentArtifact) -> Result<EvalResult> {
        let candidate_name = artifact.manifest.name.as_deref().unwrap_or("unnamed");
        let prompt = format!(
            "You are scoring a VEH candidate named '{candidate_name}'. Reply with exactly: SCORE 0.82"
        );
        let output = Command::new("ollama")
            .args(["run", &self.model, &prompt])
            .output()
            .map_err(|err| Error::Evaluator(format!("failed to run ollama: {err}")))?;

        if !output.status.success() {
            return Err(Error::Evaluator(format!(
                "ollama exited with status {}: {}",
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }

        let response = String::from_utf8_lossy(&output.stdout).to_string();
        let score = if response.contains("0.82") { 0.82 } else { 0.5 };
        Ok(EvalResult {
            score,
            details: serde_json::json!({
                "model": self.model,
                "response": response.trim(),
            }),
        })
    }
}

fn manifest_with_tools(name: &str, tools: &[&str]) -> AgentManifest {
    let mut yaml = format!("name: {name}\ntools:\n");
    for tool in tools {
        yaml.push_str(&format!("  - kind: local\n    name: {tool}\n"));
    }
    AgentManifest::from_yaml(&yaml).unwrap()
}

#[tokio::test]
#[ignore = "requires a running Ollama server and qwen3.5:9b or OLLAMA_MODEL"]
async fn live_ollama_evaluator_drives_promotion_and_approval() {
    let model = std::env::var("OLLAMA_MODEL").unwrap_or_else(|_| "qwen3.5:9b".to_string());
    let ledger = Arc::new(InMemoryLineage::new());
    let evaluator = Arc::new(OllamaEvaluator { model });
    let runner_key = SigningKey::generate(&mut OsRng);
    let veh = Veh::new(ledger.clone(), evaluator, runner_key).with_strict_improvement(false);

    let scope = AllowedScope {
        allowed_tools: vec!["search".into()],
        ..AllowedScope::default()
    };
    let root = veh
        .commit_root(
            AgentArtifact::new(manifest_with_tools("root_agent", &["search"])),
            scope.clone(),
            "2026-05-12T00:00:00Z",
        )
        .await
        .unwrap();

    let promoted = match veh
        .run_cycle(CycleInputs {
            parent: &root,
            intent: MutationIntent::new("live_ollama_in_scope"),
            candidate: AgentArtifact::new(manifest_with_tools("candidate_same_scope", &["search"])),
            allowed_scope: scope.clone(),
            created_at: "2026-05-12T00:01:00Z".into(),
        })
        .await
        .unwrap()
    {
        CycleOutcome::Promoted { node, eval } => {
            assert!(eval.score >= 0.5);
            node
        }
        _ => panic!("expected in-scope candidate to promote"),
    };
    verify_node(&promoted).unwrap();
    assert_eq!(
        ledger.head().await.unwrap(),
        Some(promoted.agent_id.clone())
    );

    let pending = match veh
        .run_cycle(CycleInputs {
            parent: &promoted,
            intent: MutationIntent::new("live_ollama_out_of_scope"),
            candidate: AgentArtifact::new(manifest_with_tools(
                "candidate_needs_approval",
                &["search", "summarize"],
            )),
            allowed_scope: scope,
            created_at: "2026-05-12T00:02:00Z".into(),
        })
        .await
        .unwrap()
    {
        CycleOutcome::AwaitingApproval(pending) => pending,
        _ => panic!("expected out-of-scope candidate to pause for approval"),
    };
    assert_eq!(ledger.head().await.unwrap(), Some(promoted.agent_id));

    let approval_key = SigningKey::generate(&mut OsRng);
    let approved = veh
        .resume_with_approval(pending, &approval_key)
        .await
        .unwrap();
    verify_node(&approved).unwrap();
    assert_eq!(ledger.head().await.unwrap(), Some(approved.agent_id));
}
