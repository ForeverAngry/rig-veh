# rig-veh

> Verifiable Evolutionary Hyperagent — **Git for cognition**.

`rig-veh` is the cryptographic, policy-gated evolution loop that sits on
top of the [`rig`](https://crates.io/crates/rig-core) ecosystem. Every
agent revision is hashed with SHA-256, signed with Ed25519, and recorded
in an append-only lineage DAG. Mutations must declare their intent, run
through a host-supplied evaluator, and pass a deterministic policy gate
before they are committed.

The crate is runtime-agnostic, library-only, and depends only on
[`rig-compose`](https://crates.io/crates/rig-compose) for the
`AgentManifest` schema. A `rig-evals-rag` adapter ships behind feature
`rag` for retrieval-quality scoring; that feature intentionally brings in
`rig-core` transitively through `rig-evals-rag` because the adapter works
against Rig's `VectorStoreIndexDyn` retrieval surface.

The crate-local maturity plan lives in [ROADMAP.md](ROADMAP.md). Cross-crate
coordination lives in
[`rig-ecosystem/docs/roadmap.md`](../rig-ecosystem/docs/roadmap.md).

## What you get

| Primitive | Role |
| --- | --- |
| `AgentArtifact` | Canonical, hash-stable wrapper around `AgentManifest`. |
| `AgentNode` | Signed, immutable ledger entry (FR-1). |
| `MutationIntent` / `AllowedScope` | Declarative *why* + governance boundary (FR-3, FR-4). |
| `LineageStore` / `InMemoryLineage` / `JsonlLineage` | Append-only lineage DAG (FR-2). |
| `PolicyGate` / `DefaultPolicyGate` | Deterministic, LLM-free firewall against capability creep. |
| `Evaluator` / `StubEvaluator` / `RetrievalEvaluator` | Pluggable sandboxed scoring. |
| `Veh` | Five-state lifecycle runner with `WaitForInput`-style pause for SR-1 multi-sig approval. |
| `ParentSelector` (`Latest`, `Best`, `Random`, `ScoreProportional`, `ScoreChildProportional`) | Open-ended-search parent strategies modelled on Meta HyperAgents' `select_next_parent`. |
| `Mutator` / `StaticMutator` | LLM-agnostic seam for producing candidate artifacts. |
| `Sandbox` / `NoopSandbox` | Isolation assertion surface enforcing `AllowedScope` before evaluation. |
| `EvolutionDriver` | Wires selector + mutator + sandbox + `Veh` into one `run_generation` step. |
| `CompositeEvaluator` / `EvalStage` | Multi-stage evaluation that short-circuits on `Stop` or `Promote`, mirroring HyperAgents' `staged_eval`. |
| `EnsembleSelector` / `BestByMetric` | Top-`k` ensemble selection over the lineage archive. |

## Quick start

```rust,no_run
use std::sync::Arc;
use ed25519_dalek::SigningKey;
use rand_core::OsRng;
use rig_compose::AgentManifest;
use rig_veh::{
    AgentArtifact, AllowedScope, CycleInputs, CycleOutcome, InMemoryLineage,
    MutationIntent, StubEvaluator, Veh,
};

# async fn run() -> rig_veh::Result<()> {
let ledger    = Arc::new(InMemoryLineage::new());
let evaluator = Arc::new(StubEvaluator::new(0.8));
let key       = SigningKey::generate(&mut OsRng);
let veh       = Veh::new(ledger, evaluator, key);

let root_manifest = AgentManifest::from_yaml("name: root\ntools: []\n")
    .map_err(|e| rig_veh::Error::Canonical(e.to_string()))?;
let root = veh
    .commit_root(AgentArtifact::new(root_manifest), AllowedScope::default(), "2026-05-11T00:00:00Z")
    .await?;

let candidate = AgentManifest::from_yaml("name: v2\ntools: []\n")
    .map_err(|e| rig_veh::Error::Canonical(e.to_string()))?;
match veh
    .run_cycle(CycleInputs {
        parent: &root,
        intent: MutationIntent::new("tighten_preamble"),
        candidate: AgentArtifact::new(candidate),
        allowed_scope: AllowedScope::default(),
        created_at: "2026-05-11T00:01:00Z".into(),
    })
    .await?
{
    CycleOutcome::Promoted { node, eval }       => println!("promoted {} @ {}", node.agent_id, eval.score),
    CycleOutcome::Rejected { node, reason }     => println!("rejected {}: {}", node.agent_id, reason),
    CycleOutcome::AwaitingApproval(pending)     => println!("needs approval: {}", pending.reason()),
}
# Ok(()) }
```

See [`examples/veh_loop.rs`](examples/veh_loop.rs) for an end-to-end run.

## Lifecycle

The five-state spec from the VEH PRD maps 1:1 onto `Veh::run_cycle`:

```text
INTENT_GENERATION ──► CANDIDATE_SPAWNING ──► BENCHMARK_EVALUATION
                                                      │
                                                      ▼
                                                POLICY_GATE
                                              ┌──────┴─────┐
                                              ▼            ▼
                                      Approve / Deny  RequireApproval
                                              │            │
                                              ▼            ▼
                                       COMMIT_OR_DISCARD   (resume_with_approval)
```

When the gate returns `RequireApproval`, `run_cycle` yields
`CycleOutcome::AwaitingApproval(pending)`. The host carries `pending`
out-of-band (e.g. to a human approver, another agent, or a multi-sig
quorum) and calls `Veh::resume_with_approval(pending, &approval_key)`.
The signature on the committed node is produced by the **approver's**
key, so the audit trail proves exactly which authority unlocked the
out-of-scope mutation.

## Feature flags

| Flag | Pulls | What it enables |
| --- | --- | --- |
| *default* | — | `InMemoryLineage`, `DefaultPolicyGate`, `StubEvaluator`, `Veh`. |
| `rag` | `rig-evals-rag`, `futures` | `RetrievalEvaluator` adapter. |
| `jsonl-ledger` | — | File-backed `JsonlLineage`. |
| `dot-export` | — | `export_dot` for Graphviz visualisation. |

## Design choices

- **`graph-flow` is not a dependency.** Its 0.5 manifest pulls
  `tokio[full]` and `sqlx[postgres]` unconditionally, which violates the
  "runtime-agnostic, no `tokio` in `[dependencies]`" rule and forces
  every downstream onto Postgres. The lifecycle runner is a ~200-LoC
  async state machine on the same surface (`Context` ≈ `CycleInputs`,
  `WaitForInput` ≈ `AwaitingApproval`, `FlowRunner` ≈ `run_cycle`). A
  `graph-flow` adapter can ship behind a feature later if there is
  demand.
- **Canonical JSON is hand-rolled** in `artifact::canonical_json` —
  sorted keys, no whitespace — so the hash is reproducible across
  toolchains and serde versions.
- **No locks held across `.await`.** `clippy::await_holding_lock` is
  `deny`. The `InMemoryLineage` guards are explicitly scope-dropped
  before any async return.
- **No panics in library code.** `unwrap`, `expect`, `panic!`, `todo!`,
  `unimplemented!`, `dbg!`, indexing, and `unreachable!` are all clippy
  `deny`/`forbid`. They are allowed in `tests/` and `examples/` only.

## Validation

```sh
just check
# fmt --check + clippy --all-features -- -D warnings + cargo test --all-features

# optional live smoke, requires local Ollama and the requested model
OLLAMA_MODEL=qwen3.5:9b cargo test --test live_ollama -- --ignored --nocapture
```

## Scope

`rig-veh` does not depend on `rig-memvid`, `rig-resources`, or `rig-mcp`
— the crate must evaluate agents built on top of any of them. Bring your
own `LineageStore` and `Evaluator` if the in-tree ones don't fit.

## License

Dual-licensed under MIT or Apache-2.0, at your option.
