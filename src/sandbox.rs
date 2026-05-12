//! Execution-sandbox abstraction.
//!
//! Meta's HyperAgents runs every mutation, evaluation, and
//! parent-selection step inside a fresh Docker container
//! ([generate_loop.py](https://github.com/facebookresearch/HyperAgents/blob/main/generate_loop.py)).
//! `rig-veh` is library-only and runtime-agnostic, so we model the
//! sandbox as a trait that the host implements (Docker, Firecracker,
//! Wasm, in-process for tests, etc.).
//!
//! The [`Sandbox`] trait is currently an *assertion* surface — the
//! host calls [`Sandbox::enforce`] before evaluating a candidate, and
//! the implementation is responsible for actually instantiating the
//! isolation primitive (containers, syscalls, network namespaces).
//! Future versions may extend the trait to physically execute the
//! evaluator inside the sandbox; that is intentionally out of scope
//! for v0.1 since it would couple `rig-veh` to a specific runtime.

use async_trait::async_trait;

use crate::artifact::AgentArtifact;
use crate::error::Result;
use crate::intent::AllowedScope;

/// Isolation guarantee that a host can apply when evaluating a
/// candidate agent.
#[async_trait]
pub trait Sandbox: Send + Sync {
    /// Verify that running `artifact` is permitted under `scope`.
    /// Implementations may perform side effects (spawn a container,
    /// set up network filters, etc.). On success the caller is
    /// expected to evaluate the artifact and then call
    /// [`Sandbox::teardown`]; on failure the caller must reject the
    /// candidate.
    async fn enforce(&self, artifact: &AgentArtifact, scope: &AllowedScope) -> Result<()>;

    /// Release any resources allocated by [`Sandbox::enforce`].
    /// Implementations may no-op for stateless sandboxes.
    async fn teardown(&self) -> Result<()> {
        Ok(())
    }
}

/// No-op sandbox for tests and in-process hosts where isolation is
/// provided elsewhere (or accepted as the threat model). Always
/// returns `Ok(())`.
#[derive(Default, Clone, Copy)]
pub struct NoopSandbox;

#[async_trait]
impl Sandbox for NoopSandbox {
    async fn enforce(&self, _artifact: &AgentArtifact, _scope: &AllowedScope) -> Result<()> {
        Ok(())
    }
}
