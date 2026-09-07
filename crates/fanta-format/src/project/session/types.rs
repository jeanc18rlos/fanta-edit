//! Shared session types: ids, dirty state, conflict, save results.

use super::hash::ContentHash;
use crate::project::merge::NodeMapEdition;
use crate::project::read::ScopedDesign;
use fanta_doc::{ComponentId, NodeId};
use fanta_fnx::{ArtifactIr, ArtifactKind};
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Arc;

/// Stable identity of a design artifact in the workspace index.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ArtifactId {
    Workspace,
    Page(NodeId),
    Component(ComponentId),
    Graphics(String),
    Prototype(String),
    Motion(String),
    Audio(String),
}

impl ArtifactId {
    pub fn from_scoped(s: ScopedDesign) -> Self {
        match s {
            ScopedDesign::Page(id) => Self::Page(id),
            ScopedDesign::Component(id) => Self::Component(id),
        }
    }

    pub fn to_scoped(&self) -> Option<ScopedDesign> {
        match self {
            Self::Page(id) => Some(ScopedDesign::Page(*id)),
            Self::Component(id) => Some(ScopedDesign::Component(*id)),
            _ => None,
        }
    }

    pub fn kind(&self) -> ArtifactKind {
        match self {
            Self::Workspace => ArtifactKind::Workspace,
            Self::Page(_) => ArtifactKind::Page,
            Self::Component(_) => ArtifactKind::Component,
            Self::Graphics(_) => ArtifactKind::Graphics,
            Self::Prototype(_) => ArtifactKind::Prototype,
            Self::Motion(_) => ArtifactKind::Motion,
            Self::Audio(_) => ArtifactKind::Audio,
        }
    }

    pub fn debug_label(&self) -> String {
        match self {
            Self::Workspace => "workspace".into(),
            Self::Page(id) => format!("page:{id}"),
            Self::Component(id) => format!("component:{id}"),
            Self::Graphics(s) => format!("graphics:{s}"),
            Self::Prototype(s) => format!("prototype:{s}"),
            Self::Motion(s) => format!("motion:{s}"),
            Self::Audio(s) => format!("audio:{s}"),
        }
    }
}

/// Index entry for one design on disk (not necessarily open).
#[derive(Debug, Clone)]
pub struct ArtifactMeta {
    pub id: ArtifactId,
    pub kind: ArtifactKind,
    pub slug: String,
    /// Project-relative design directory, e.g. `pages/home`.
    pub design_dir: PathBuf,
    pub disk_hash: ContentHash,
}

/// Scoped document: scene + selection + history live on `doc`.
#[derive(Debug, Clone)]
pub struct ScopedDoc {
    pub doc: fanta_doc::Doc,
    pub root: NodeId,
}

/// Dirty / conflict state for one open artifact.
#[derive(Debug, Clone)]
pub enum ArtifactDirty {
    Clean,
    DirtyCanvas,
    /// Buffer is sole content primary.
    DirtyText,
    Conflict(ConflictState),
    Invalid {
        error: String,
        last_good: Option<Box<ScopedDoc>>,
    },
}

impl ArtifactDirty {
    pub fn is_dirty(&self) -> bool {
        !matches!(self, Self::Clean)
    }

    pub fn is_conflict(&self) -> bool {
        matches!(self, Self::Conflict(_))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictOrigin {
    Canvas,
    Text,
}

#[derive(Debug, Clone)]
pub enum EditionSide {
    Present(Arc<ArtifactEdition>),
    Deleted,
}

#[derive(Debug, Clone)]
pub struct ArtifactEdition {
    pub file_hash: ContentHash,
    pub ir: ArtifactIr,
    pub text: Option<String>,
    pub node_map: NodeMapEdition,
}

#[derive(Debug, Clone)]
pub struct MergePreview {
    pub merged_nodes: serde_json::Map<String, Value>,
    pub merged_header: Value,
    pub merged_ir: ArtifactIr,
    pub conflicts: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ConflictState {
    pub base: Arc<ArtifactEdition>,
    pub ours: Arc<ArtifactEdition>,
    pub theirs: EditionSide,
    pub auto: MergePreview,
    pub origin: ConflictOrigin,
}

#[derive(Debug, Clone)]
pub enum ConflictResolution {
    KeepOurs,
    TakeTheirs,
    AcceptAuto,
    Manual(ArtifactIr),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClosePolicy {
    Discard,
    Save,
    CancelIfDirty,
}

#[derive(Debug, Clone)]
pub enum SaveResult {
    NoOp,
    Wrote { paths: Vec<PathBuf> },
}

/// Severity of a non-fatal source-authoring diagnostic. There is deliberately
/// no `Error` arm: anything fatal fails the parse/materialize path itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceSeverity {
    Warning,
    Info,
}

/// One non-fatal diagnostic from ingesting hand-written `.fnx` source (e.g. a
/// typo'd attribute that serde would otherwise swallow silently). These never
/// block a commit — unknown FUTURE fields must keep riding through — the
/// caller decides whether a policy (such as the harness `--deny`) promotes
/// them.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
pub struct SourceDiagnostic {
    pub severity: SourceSeverity,
    /// Stable machine code, e.g. `source.unknown_attribute`.
    pub code: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node_name: Option<String>,
    pub message: String,
}

/// How the retained FNX source changed after a successful canvas transaction.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case", tag = "mode")]
pub enum SourceSync {
    /// The operation changed document state that is not represented by this
    /// artifact's FNX node tree.
    Unchanged,
    /// Only these node tags/properties were patched in the retained source.
    PatchedNodes { nodes: Vec<NodeId> },
    /// The semantic tree changed structurally, so this version rebuilt the FNX
    /// projection. This is explicit so callers/tests never mistake it for a
    /// property-local edit.
    Rebuilt { reason: SourceRebuildReason },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceRebuildReason {
    StructuralOperation,
    StaleProjection,
    PatchUnavailable,
    MergeInstall,
}

/// Persistence owner/shape impact for every [`fanta_doc::Operation`].
///
/// The classifier is intentionally exhaustive in the session implementation:
/// adding a future operation requires choosing its source boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactOpImpact {
    NodeAttributes,
    Structure,
    ComponentHeader,
    Workspace,
    Motion,
    Flow,
}

/// Workspace shared dirty (N19): single DirtyShared, not split registry/fnx.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkspaceDirty {
    Clean,
    DirtyShared,
    Conflict,
    Invalid,
}

#[derive(Debug, Clone)]
pub enum SessionEvent {
    Reloaded {
        id: ArtifactId,
    },
    Removed {
        id: ArtifactId,
    },
    EnteredConflict {
        id: ArtifactId,
        conflicts: Vec<String>,
    },
    Invalidated {
        id: ArtifactId,
        error: String,
    },
    ComponentRevBumped {
        id: ComponentId,
        rev: u64,
    },
    WorkspaceGeneration {
        generation: u64,
    },
}

#[derive(Debug, Clone)]
pub enum FsEvent {
    /// File or directory under the project changed.
    Modified {
        path: PathBuf,
    },
    Created {
        path: PathBuf,
    },
    Removed {
        path: PathBuf,
    },
}
