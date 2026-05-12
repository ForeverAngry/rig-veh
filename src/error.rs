//! Typed error surface for `rig-veh`.

use thiserror::Error;

/// Convenience [`Result`] alias for crate APIs.
pub type Result<T> = std::result::Result<T, Error>;

/// Errors produced by the Verifiable Evolutionary Hyperagent crate.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum Error {
    /// The canonical bytes of an artifact could not be serialised.
    #[error("canonical serialisation failed: {0}")]
    Canonical(String),

    /// A computed hash does not match the recorded `agent_id`.
    #[error("agent id hash mismatch: expected {expected}, computed {actual}")]
    HashMismatch {
        /// Hash recorded on the node.
        expected: String,
        /// Hash computed from the canonical bytes.
        actual: String,
    },

    /// The cryptographic signature on a node failed to verify.
    #[error("signature verification failed: {0}")]
    SignatureInvalid(String),

    /// A signing key was malformed or the wrong length.
    #[error("invalid signing key: {0}")]
    InvalidKey(String),

    /// The policy gate denied promotion of a candidate.
    #[error("policy denied: {0}")]
    PolicyDenied(String),

    /// The evaluation sandbox failed to produce a result.
    #[error("evaluator error: {0}")]
    Evaluator(String),

    /// A lineage store operation failed.
    #[error("ledger error: {0}")]
    Ledger(String),

    /// The requested node was not found in the ledger.
    #[error("agent not found: {0}")]
    NotFound(String),

    /// A parent reference was required but missing.
    #[error("missing parent for non-root agent")]
    MissingParent,

    /// A JSON error occurred while encoding or decoding metadata.
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),

    /// An I/O error occurred while reading or writing the ledger.
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}
