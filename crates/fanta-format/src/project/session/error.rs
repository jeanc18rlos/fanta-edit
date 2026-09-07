//! Session-layer errors.

use crate::error::FormatError;
use fanta_fnx::FnxError;
use std::path::PathBuf;

/// Why a save was refused without writing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SaveBlocked {
    InConflict,
    EnteredConflict,
    Invalid,
    DiskChangedAgain,
}

/// Errors from [`super::WorkspaceSession`] / [`super::ArtifactSession`].
#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error(transparent)]
    Format(#[from] FormatError),

    #[error("fnx: {0}")]
    Fnx(#[from] FnxError),

    #[error("not a fanta project: {}", path.display())]
    NotAProject { path: PathBuf },

    #[error("artifact not found: {0:?}")]
    ArtifactNotFound(String),

    #[error("artifact already open")]
    AlreadyOpen,

    #[error("artifact is not open")]
    NotOpen,

    #[error("import not allowed: {feature}")]
    ImportNotAllowed { feature: String },

    #[error("operation not allowed in this state: {0}")]
    InvalidState(String),

    #[error("artifact generation changed: expected {expected}, actual {actual}")]
    GenerationConflict { expected: u64, actual: u64 },

    #[error("artifact base changed: expected {expected}, actual {actual}")]
    BaseRevisionConflict { expected: String, actual: String },

    #[error("merge review still has unresolved conflicts")]
    ReviewIncomplete,

    #[error("merge review conflict {conflict_id} was not found")]
    ReviewConflictNotFound { conflict_id: u32 },

    #[error("operation precondition does not match the live node {node}")]
    OperationPrecondition { node: String },

    #[error("save blocked: {0:?}")]
    SaveBlocked(SaveBlocked),

    #[error("variable ops must go through the workspace session")]
    UseWorkspaceArtifact,

    #[error("master not in scope for this page session")]
    MasterNotInScope,

    #[error("invalid source: {0}")]
    InvalidSource(String),

    #[error("doc reassembly failed: {0}")]
    DocAssemble(String),

    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("{0}")]
    Other(String),
}

impl SessionError {
    pub fn other(msg: impl Into<String>) -> Self {
        Self::Other(msg.into())
    }
}
