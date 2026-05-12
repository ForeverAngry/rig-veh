# Changelog

<!-- markdownlint-disable MD024 -->

All notable changes to this crate are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/)
and this crate adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.2.0] — 2025-03-XX

### Added

- **Breaking**: Added `parent_agent_success`, `valid_parent`, and `eval_stage` 
  fields to `AgentNode`. This alters the canonical JSON layout and breaks existing 
  hashes (`CommitInputs::signing_payload`).
- `EnsembleSelector` trait + `BestByMetric` implementation —
  picks the top-`k` promoted agents by score, mirroring HyperAgents'
  `ensemble.py`. The trait is an LLM-agnostic seam; hosts plug
  domain-specific ensemble logic on top.
- `EvalStage`, `StagedResult`, `CompositeEvaluator`, and
  `Evaluator::evaluate_staged` — multi-stage evaluation pipeline that
  short-circuits on `Stop` / `Promote`, mirroring HyperAgents'
  `staged_eval` cost-saving heuristic. Existing single-stage
  evaluators continue to work via a default `evaluate_staged`
  implementation.
- `EvolutionDriver` — wires `ParentSelector` + `Mutator` + `Sandbox` +
  `Veh` into a single `run_generation` step, mirroring HyperAgents'
  `generate_loop.py` without committing the host to a specific runtime.
- `MutationContext` — optional advisory passed to `Mutator::propose`
  carrying `iterations_left` and free-form metadata. Generalises
  HyperAgents' `iterations_left` signal.
- `ParentSelector` trait with `Latest`, `Best`, `Random`,
  `ScoreProportional`, and `ScoreChildProportional` strategies, mirroring
  the open-ended-search options in Meta's HyperAgents reference
  implementation. Selectors only read the ledger; they never mutate it.
- `Mutator` trait + `StaticMutator` — LLM-agnostic seam for producing
  candidate artifacts. Hosts plug their own provider.
- `Sandbox` trait + `NoopSandbox` — assertion surface for enforcing
  `AllowedScope` before evaluating a candidate.
- `LineageStore::nodes()` accessor (default impl decodes from
  `export_dag_json`; cheap overrides on `InMemoryLineage` and
  `JsonlLineage`).
- `tests/selector.rs` — coverage for each selector strategy plus the
  empty-ledger error path.
- Add crate-local `ROADMAP.md` documenting maturity status, next work, and
  non-goals for verifiable agent evolution.
- Add optional-feature and live Ollama validation coverage for JSONL lineage,
  DOT export, `RetrievalEvaluator`, and the host-owned evaluator path.

## [0.1.0] — 2026-05-11

### Added

- Initial public release of `rig-veh`.
- `AgentArtifact` with canonical JSON serialisation and capability SBOM
  derivation from `rig_compose::AgentManifest`.
- `AgentNode` schema implementing FR-1: SHA-256 + Ed25519 signed
  immutable lineage entries.
- `MutationIntent` and `AllowedScope` (FR-3, FR-4).
- `LineageStore` trait with `InMemoryLineage` (always available) and
  `JsonlLineage` (feature `jsonl-ledger`).
- `PolicyGate` trait and `DefaultPolicyGate` enforcing allow-list scope,
  generation caps, and `gated_capabilities` approval requirements.
- `Evaluator` trait, `StubEvaluator`, and `RetrievalEvaluator` adapter
  for [`rig-evals-rag`](https://crates.io/crates/rig-evals-rag) behind
  feature `rag`.
- `Veh` lifecycle runner implementing the five-state spec including
  `CycleOutcome::AwaitingApproval` + `resume_with_approval` for SR-1
  out-of-band approvals.
- `manifest_diff` (line-level unified diff) and `export_dot` (Graphviz)
  helpers behind feature `dot-export`.
- Integration tests for identity, lineage, policy gate, and rollback.
