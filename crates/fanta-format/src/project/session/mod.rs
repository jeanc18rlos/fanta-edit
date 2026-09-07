//! Artifact-scoped workspace session (design: artifact-ir-workspace-session).
//!
//! Replaces monodoc reload on the editor hot path with:
//! - per-artifact IR + scoped Scene
//! - content-hash no-ops
//! - directory-scoped save
//! - three-way merge on concurrent disk edits

mod artifact;
mod catalog;
pub(crate) mod diagnostics;
mod error;
mod graphics;
mod guard;
mod hash;
mod materialize;
mod motion;
mod render_snapshot;
mod review;
mod sync;
mod types;
mod workspace;
mod workspace_fnx;

#[cfg(test)]
mod tests;

pub use artifact::{ArtifactSession, artifact_op_impact, write_artifact_files};
pub use catalog::{ComponentCatalog, MasterRef};
pub use error::{SaveBlocked, SessionError};
pub use graphics::{import_component_as_recreation, materialize_graphics};
pub use guard::{DocMutGuard, PresenceView};
pub use hash::{ContentHash, hash_file_set, hash_named_files_in_dir};
pub use materialize::{
    collect_subtree_nodes, materialize_component, materialize_page, project_scene_to_node_map,
};
pub use motion::{MotionIndex, MotionSource, read_motion_dual};
pub use render_snapshot::{ArtifactRenderRevision, ArtifactRenderSnapshot};
pub use review::{
    MergeReview, ProposalApplied, ReviewConflict, ReviewResolution, SourceProposalOutcome,
};
pub use sync::ApplyReport;
pub use types::{
    ArtifactDirty, ArtifactEdition, ArtifactId, ArtifactMeta, ArtifactOpImpact, ClosePolicy,
    ConflictOrigin, ConflictResolution, ConflictState, EditionSide, FsEvent, MergePreview,
    SaveResult, ScopedDoc, SessionEvent, SourceDiagnostic, SourceRebuildReason, SourceSeverity,
    SourceSync, WorkspaceDirty,
};
pub use workspace::{WorkspaceSession, WorkspaceSharedState};
pub use workspace_fnx::{DependencyGraph, WorkspaceIr, synthesize_workspace_fnx};

// Re-export merge substrate for hosts that need it.
pub use crate::project::merge::{
    ArtifactAddress, ArtifactMerge, JsonPathSegment, NodeMapEdition, PresenceValue,
    PropertyConflict, merge_artifact,
};
