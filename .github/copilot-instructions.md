# rig-veh — Copilot Instructions

See [AGENTS.md](../AGENTS.md) for the authoritative copy. Summary:

- Rust 2024, MSRV `1.89`. Library is runtime-agnostic — do not add
  `tokio` to `[dependencies]`.
- Typed `thiserror::Error` in [src/error.rs](../src/error.rs). Return
  `Result<_, Error>`; no ad-hoc `Box<dyn Error>` or `String` errors.
- Never `.await` while holding a `Mutex`/`RwLock` guard
  (`clippy::await_holding_lock` is `deny`).
- No `unwrap`/`expect`/`panic!`/`todo!`/`unimplemented!`/`dbg!`/indexing
  in library code (clippy `deny`/`forbid`). Use `?`,
  `ok_or(Error::…)`, `get(..)`, pattern matching.
- `unwrap`/`expect` allowed in `tests/`, `examples/`, and `#[cfg(test)]`
  blocks (gate the test module with
  `#[allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]`).
- Use `tracing` for logs; no `println!` in library code.
- Document new `pub` items with `///` rustdoc. Re-export from
  [src/lib.rs](../src/lib.rs).
- Canonical JSON is load-bearing: changing the signing payload shape is
  a hash-breaking change.

## Validation

```sh
just check
# fmt --check + clippy --all-features -- -D warnings + cargo test --all-features
```

Run before declaring any change done.

## Scope

The crate must not depend on `rig-memvid`, `rig-resources`, or
`rig-mcp`. It must remain runtime-agnostic.

## `graph-flow`

Not a dependency in v0.1. See [AGENTS.md](../AGENTS.md) for the
reasoning. If a `graph-flow` adapter ships later, it must be
feature-gated.
