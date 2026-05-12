# AGENTS.md

Guidance for AI coding agents working in `rig-veh`. Mirrors
[.github/copilot-instructions.md](.github/copilot-instructions.md).

## Project

`rig-veh` implements the Verifiable Evolutionary Hyperagent spec on top
of the `rig` ecosystem. Public primitives:

- `AgentArtifact` / `AgentNode` — canonical, hashed, Ed25519-signed
  ledger entries ([src/artifact.rs](src/artifact.rs), [src/node.rs](src/node.rs)).
- `MutationIntent` / `AllowedScope` — declarative why + governance
  boundary ([src/intent.rs](src/intent.rs)).
- `LineageStore` / `InMemoryLineage` / `JsonlLineage` — append-only DAG
  ([src/ledger.rs](src/ledger.rs)).
- `PolicyGate` / `DefaultPolicyGate` — deterministic firewall
  ([src/policy.rs](src/policy.rs)).
- `Evaluator` / `StubEvaluator` / `RetrievalEvaluator` — pluggable
  scoring ([src/evaluator.rs](src/evaluator.rs), [src/rag_evaluator.rs](src/rag_evaluator.rs)).
- `Veh` — five-state lifecycle runner with `AwaitingApproval` pause
  ([src/graph.rs](src/graph.rs)).

## Rules

- Rust 2024, MSRV 1.89. Library is runtime-agnostic; do not add `tokio`
  to `[dependencies]`.
- Errors: typed `thiserror` enum in [src/error.rs](src/error.rs); return
  `Result<_, Error>`. No ad-hoc `Box<dyn Error>` or `String` error types.
- Never `.await` while holding a `Mutex`/`RwLock` guard. Scope-drop
  first (`clippy::await_holding_lock = deny`).
- No `unwrap`, `expect`, `panic!`, `todo!`, `unimplemented!`, `dbg!`,
  indexing/slicing, or `unreachable!` in library code — clippy
  `deny`/`forbid`. Use `?`, `ok_or(Error::...)`, `get(..)`, `match`.
  Allowed in `tests/`, `examples/`, `#[cfg(test)]` blocks (gate with
  `#[allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]`).
- Use `tracing` for logs; no `println!` in library code.
- Document new `pub` items with `///` rustdoc. Re-export from
  [src/lib.rs](src/lib.rs).
- **Canonical JSON is load-bearing.** Any change to the on-disk shape of
  `AgentNode` or the `CommitInputs::signing_payload` field set is a
  hash-breaking change. Bump the version and document it in
  [CHANGELOG.md](CHANGELOG.md).

## Feature flags

Default = none. Optional: `rag`, `jsonl-ledger`, `dot-export`.
Gate optional code with `#[cfg(feature = "...")]`.

## Validation

```sh
just check
# fmt --check + clippy --all-features -- -D warnings + cargo test --all-features
```

Integration tests live in [tests/](tests/). Examples must keep building:
`cargo build --examples`.

## Scope

Do not depend on `rig-memvid`, `rig-resources`, or `rig-mcp` — this
crate has to evaluate agents built on top of all of them. Do not vendor
`rig-core` or `rig-compose`. Update [README.md](README.md) and
[CHANGELOG.md](CHANGELOG.md) for user-visible changes.

## `graph-flow`

`graph-flow` 0.5 is *not* a dependency. Its manifest pulls
`tokio[full]` and `sqlx[postgres]` unconditionally, which violates the
runtime-agnostic rule. The state machine in [src/graph.rs](src/graph.rs)
covers the same surface (`Context` ≈ `CycleInputs`, `WaitForInput` ≈
`AwaitingApproval`, `FlowRunner` ≈ `run_cycle`). If a host wants
`graph-flow` integration, ship it behind a feature with task wrappers,
not as a default dep.
