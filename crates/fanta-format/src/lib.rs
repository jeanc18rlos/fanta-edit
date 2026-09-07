//! On-disk formats for Fanta: the `.fant` container and the git-native
//! project tree.
//!
//! Two projections of the same [`fanta_doc::Doc`]:
//!
//! - [`FantaFile`] — the `.fant` zip container (moment-in-time export/import
//!   per spec 09 §A.4).
//! - [`write_project_tree`] / [`read_project_tree`] — the granular project
//!   directory (spec 09 §A.2), the working format a project git repo tracks.
//!   See the `project` module docs for the tree shape.
//!
//! A `.fant` is a zip container with three logical layers:
//!
//! - `manifest.json` — small metadata header. Schema version, doc id, app
//!   version, asset index, timestamps. Readable without touching the doc body.
//! - `doc.json` — canonical [`fanta_doc::Doc`] JSON projection. The same string
//!   `Doc::to_json_string` produces.
//! - `assets/<sha256_hex>.bin` — raw binary blobs (bitmap, video, audio, 3d,
//!   ai cache). Content-addressed via SHA-256 so identical bytes deduplicate
//!   automatically.
//!
//! The on-disk shape mirrors ARCHITECTURE.md §8. JSON is used in v0 (not rkyv
//! as the architecture's long-term plan calls for) because the doc model
//! itself is already JSON-projected and that's enough for the wedge. Swapping
//! the doc body to rkyv later is a schema-version bump and a new migration.
//!
//! ## Example
//!
//! ```no_run
//! use fanta_doc::Doc;
//! use fanta_format::FantaFile;
//!
//! let mut f = FantaFile::create("project.fant").unwrap();
//! let doc = Doc::new();
//! f.save_doc(&doc).unwrap();
//! let png_bytes = b"<png contents>".to_vec();
//! let asset_id = f.put_asset(&png_bytes).unwrap();
//! // Same bytes again — same id, only stored once.
//! let asset_id_again = f.put_asset(&png_bytes).unwrap();
//! assert_eq!(asset_id, asset_id_again);
//! ```

#![forbid(unsafe_code)]

mod container;
mod error;
mod manifest;
mod migrate;
mod project;

pub use container::{FantaFile, asset_id_for_bytes};
pub use error::{FormatError, Result};
pub use manifest::Manifest;
pub use migrate::migrate;
pub use project::session;
pub use project::{
    ApplyReport, ArtifactAddress, ArtifactDirty, ArtifactId, ArtifactMerge, ArtifactMeta,
    ArtifactOpImpact, ArtifactRenderRevision, ArtifactRenderSnapshot, ArtifactSession, ClosePolicy,
    ConflictResolution, ContentHash, DependencyGraph, DocMerge, DocMutGuard, FsEvent,
    JsonPathSegment, MergeReview, MotionIndex, MotionSource, NodeMapEdition, PresenceValue,
    ProjectManifest, ProjectSourceEdit, PropertyConflict, ProposalApplied, ReviewConflict,
    ReviewResolution, SaveBlocked, SaveResult, ScopedDesign, ScopedDoc, SessionError, SessionEvent,
    SourceDiagnostic, SourceProposalOutcome, SourceRebuildReason, SourceSeverity, SourceSync,
    WorkspaceDirty, WorkspaceIr, WorkspaceSession, WorkspaceSharedState, WriteReport,
    apply_project_source_edit, apply_project_source_edit_with_diagnostics, artifact_op_impact,
    canonicalize_legacy_source, component_id_of_dir, ensure_project_editor_support,
    export_fant_snapshot, hash_file_set, import_fant_snapshot, is_project_dir,
    locate_master_source, locate_page_source, merge_artifact, merge_docs, page_id_of_dir,
    page_scope_of_source, read_motion_dual, read_project_tree, scaffold_project_tree,
    synthesize_workspace_fnx, validate_project_source_edit,
    validate_project_source_edit_with_diagnostics, write_artifact_files, write_project_tree,
};
