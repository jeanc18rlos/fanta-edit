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
// The wasm build compiles out the disk writers; their std-only helpers go
// dead there while the default build keeps full dead-code analysis.
#![cfg_attr(not(feature = "native"), allow(dead_code))]

#[cfg(feature = "native")]
mod container;
mod error;
#[cfg(feature = "native")]
pub mod formats;
#[cfg(feature = "native")]
mod manifest;
mod migrate;
mod project;
pub mod runtime;

#[cfg(feature = "native")]
pub use container::FantaFile;
pub use error::{FormatError, Result};
#[cfg(feature = "native")]
pub use formats::{
    FormatDescriptor, FormatHandler, FormatHandlerResult, FormatOperation,
    FormatPath, FormatRegistry, FormatRegistryError, FormatRegistryResult, ImportedDesign,
};
#[cfg(feature = "native")]
pub use manifest::Manifest;
pub use migrate::migrate;
#[cfg(feature = "native")]
pub use project::session;
pub use project::{
    DIAGNOSTIC_CODES, LOAD_OMITTED_GEOMETRY, SourceDiagnostic, SourceSeverity,
    ArtifactAddress, ArtifactMerge, DocMerge, FormatCapabilities, JsonPathSegment, NodeMapEdition,
    PresenceValue,
    ProjectManifest, ProjectWriteCache, PropertyConflict, ScopedDesign, WebDesignReport,
    WebProjection, WebRaster, WebReport, WebRenderSource, WriteReport,
    asset_id_for_bytes, canonicalize_legacy_source, emit_web_project_tree,
    is_project_managed_path, merge_artifact,
    merge_docs, read_project_files, web_project_files,
};
#[cfg(feature = "native")]
pub use project::{
    ApplyReport, ArtifactDirty, ArtifactId, ArtifactMeta, ArtifactOpImpact,
    ArtifactRenderRevision, ArtifactRenderSnapshot, ArtifactSession, ClosePolicy,
    ConflictResolution, ContentHash, DependencyGraph, DocMutGuard, FsEvent, IncrementalDocApply,
    MediaFormat, MediaRegistry, MediaRegistryError, MergeReview, MotionIndex, MotionSource,
    ProjectSourceEdit, ProjectUpgrade, ProposalApplied, ReviewConflict, ReviewResolution,
    SaveBlocked, SaveResult, ScopedDoc, SessionError, SessionEvent,
    SourceProposalOutcome, SourceRebuildReason, SourceSync, WorkspaceDirty,
    WorkspaceDiskSnapshot, WorkspaceIr, WorkspaceSession, WorkspaceSharedState,
    apply_project_json_edit, apply_project_source_edit, check_project,
    apply_project_source_edit_with_diagnostics, artifact_op_impact, component_id_of_dir,
    ensure_project_editor_support, export_fant_snapshot, hash_file_set, import_fant_snapshot,
    is_project_dir, locate_master_source, locate_page_source, page_id_of_dir,
    page_scope_of_source, projected_design_dirs, read_motion_dual, read_project_tree,
    read_project_tree_with_warnings, scaffold_project_tree, stage_new_project_tree,
    synthesize_workspace_fnx, upgrade_project, validate_project_json_edit,
    validate_project_source_edit, validate_project_source_edit_with_diagnostics,
    write_artifact_files, write_legacy_project_tree, write_legacy_project_tree_checked,
    write_project_tree, write_project_tree_cached, write_web_project_tree,
};
