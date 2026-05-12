# rig-veh Roadmap

This roadmap is the crate-local operating plan for `rig-veh`. The cross-crate coordination summary lives in [`rig-contributions/docs/roadmap.md`](../rig-contributions/docs/roadmap.md).

## Role

`rig-veh` answers the operational evolution question: where did this agent come from, what changed, how was it evaluated, and who signed off? It owns cryptographic lineage, mutation intent, policy-gated promotion, evaluator integration, rollback, and audit export surfaces without forcing hosts onto one governance stack.

## Landed

- Ed25519-signed `AgentNode`s with canonical-JSON hashing.
- `AgentArtifact`, `MutationIntent`, `AllowedScope`, and capability SBOM derivation from `rig_compose::AgentManifest`.
- Append-only `LineageStore` with `InMemoryLineage` and feature-gated `JsonlLineage`.
- `DefaultPolicyGate` with generation caps, allowed capability scope, gated capabilities, and `AwaitingApproval` pause/resume.
- `Evaluator` trait, `StubEvaluator`, and feature-gated `RetrievalEvaluator` over `rig-evals-rag::MultiReport`.
- Five-state `Veh::run_cycle` lifecycle with strict-improvement enforcement.
- Signature-verified rollback that refuses to move `head` to a `Rejected` node.
- Feature-gated `export_dot` and integration coverage for identity, lineage, policy gate, rollback, optional features, and a live Ollama evaluator smoke test.

## Prototype Grade

- Rollback moves the head pointer after verification, but it is not yet represented as its own signed audit event.
- `export_dot` has test coverage but no user-facing Graphviz or Mermaid example/consumer.
- `RetrievalEvaluator` uses a host-supplied `ReportFactory`; the shape should not stabilize until another reusable evaluator backend validates it.
- Multi-signer and threshold approval flows are not implemented and should wait for a real host requirement.
- Lineage data is not yet projected into `rig-compose` `ContextPack` for agents to inspect their own history.

## Next Work

1. Add a first-class audit evidence bundle: parent hash, mutation diff, intent, eval report, policy decision, signer key, timestamp, and verification status.
2. Represent rollback as a signed lineage/audit operation rather than only moving the head pointer.
3. Ship a `dot-export` consumer example, either Graphviz output or a Mermaid timeline renderer.
4. Validate a second evaluator backend or example before stabilizing the `ReportFactory` pattern.
5. Wire `MutationIntent`, `EvalResult`, and selected lineage evidence into `rig-compose` `ContextItem` / `ContextPack`.
6. Keep multi-signer / threshold approval behind a feature and real host need.

## Maturity Bar

- Every promoted agent has a verifiable, replayable evidence trail: diff, eval result, policy decision, signing key, and parent hash.
- Rollback is a signed operation that never silently demotes a `Promoted` node or promotes a `Rejected` one.
- The same lineage works for local-only evolution and for remote/live evaluator workflows.
- Live model smokes complement deterministic tests and never replace them.

## Non-Goals

- Do not depend on `rig-memvid`, `rig-resources`, or `rig-mcp`.
- Do not add a runtime dependency to library dependencies.
- Do not ship threshold approval, graph-flow adapters, or governance UI before a host needs them.
