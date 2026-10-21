//! Fanta's design engine without the app: open a project directory, review or
//! edit it with design ops, settle its layout, render its pages and save it
//! back. The cloud engine worker (`fanta-engine`) drives it and the editor
//! shares its code, so nothing here may depend on GPUI.
//!
//! The facade is frozen by `docs/cloud-engine-contracts.md`. Its methods are
//! filled in as the editor code they need moves here; until then they fail
//! with an error that names the missing piece.

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use anyhow::{Result, bail};
use fanta_design_ops::{CommentDraft, DesignOp};
use fanta_doc::{Doc, NodeId, Operation};
use serde::{Deserialize, Serialize};

/// The longest side a render may have: the most a vision model reads before
/// it downscales.
pub const MAX_RENDER_DIMENSION: u32 = 1568;

/// A page of the open project, as a request names it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged, deny_unknown_fields)]
pub enum PageRef {
    /// Its position in the editor's page list: the numbering `inspect`
    /// reports and [`CommentDraft::page`] uses.
    Index { index: usize },
    /// Its root node id in the prefixed form (`n_…`).
    Id { id: String },
    /// The page made by this session's `created`-th [`HeadlessProject::create_page`]
    /// call, counting from 0.
    Created { created: usize },
}

/// One image a render produces.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RenderTarget {
    /// Returned with the image, so a caller can match images to targets.
    pub id: String,
    pub page: PageRef,
    /// Render only this node's region of the page (prefixed id).
    #[serde(default)]
    pub node: Option<String>,
    /// Cap on the longer output side in pixels, at most [`MAX_RENDER_DIMENSION`].
    #[serde(default = "default_render_dimension")]
    pub max_dimension: u32,
    /// Sample this animation clip instead of the unanimated design.
    #[serde(default)]
    pub motion_clip: Option<String>,
    /// Animation sample time in milliseconds; needs `motion_clip`.
    #[serde(default)]
    pub playhead_ms: Option<u32>,
}

fn default_render_dimension() -> u32 {
    MAX_RENDER_DIMENSION
}

#[derive(Debug, Clone, PartialEq)]
pub struct RenderedImage {
    pub target_id: String,
    pub png: Vec<u8>,
    pub width: u32,
    pub height: u32,
    /// The world rectangle the image shows: `[x, y, width, height]`.
    pub bounds: [f64; 4],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileStatus {
    Added,
    Modified,
    Deleted,
}

/// A project file a save changed, relative to the state it was opened in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangedFile {
    /// Relative to the project root, with `/` separators.
    pub path: String,
    pub status: FileStatus,
    /// Lowercase hex SHA-256 of the new content; `None` for a deleted file.
    pub sha256: Option<String>,
}

/// An open project and the edits made to it since it was opened.
pub struct HeadlessProject {
    root: PathBuf,
    loaded_hashes: BTreeMap<String, [u8; 32]>,
    journal: Vec<Operation>,
}

impl HeadlessProject {
    /// Read the project at `root` the way the editor opens it, solve the
    /// geometry its sources leave out, and record the hash of every file so
    /// [`changeset`] can report what a later [`save`](Self::save) changed.
    pub fn open(root: &Path) -> Result<Self> {
        bail!(
            "opening {} headlessly is not implemented yet: the project loader still lives in fig_viewer",
            root.display()
        )
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The project summary an engine `inspect` step returns: pages with their
    /// top-level frames, components, the design system, the project's
    /// `fanta.md` rules, motion clips and prototype flows.
    pub fn inspect(&self) -> Result<serde_json::Value> {
        bail!("inspect is not implemented yet")
    }

    /// The root node of `page`.
    pub fn resolve_page(&self, page: &PageRef) -> Result<NodeId> {
        bail!("resolving {page:?} is not implemented yet")
    }

    /// Add an empty page named `name` after the last authored page, as the
    /// editor's Add Page does, and return its root.
    pub fn create_page(&mut self, name: &str) -> Result<NodeId> {
        bail!("creating page {name:?} is not implemented yet")
    }

    /// Apply `ops` as one transaction, with `page` as the active page that
    /// parentless ops place on. When `commit` is false the batch is checked and
    /// rolled back. The result reports per-op status and created ids, as the
    /// editor's `batch_design` does.
    pub fn apply(
        &mut self,
        _page: Option<NodeId>,
        label: &str,
        _ops: &[DesignOp],
        _commit: bool,
    ) -> Result<serde_json::Value> {
        bail!("applying {label:?} is not implemented yet: the op applier still lives in fig_viewer")
    }

    /// Place review comments: one undo step per page, labelled `label`.
    /// Returns each draft's new comment id, or why that draft was refused.
    pub fn add_comments(
        &mut self,
        label: &str,
        _drafts: &[CommentDraft],
    ) -> Result<Vec<Result<String>>> {
        bail!("adding comments for {label:?} is not implemented yet")
    }

    /// Re-solve the auto layout and auto-sized text that edits invalidated.
    pub fn settle_layout(&mut self) -> Result<()> {
        bail!("settling layout is not implemented yet")
    }

    /// `page`'s top-level layers, `depth` levels deep and at most `limit` of
    /// them, in the style projection the editor's `batch_get` returns for
    /// `detail: "style"`.
    pub fn project_page(
        &self,
        page: NodeId,
        _depth: u32,
        _limit: usize,
    ) -> Result<Vec<serde_json::Value>> {
        bail!("projecting page {page} is not implemented yet")
    }

    pub fn render(&mut self, target: &RenderTarget) -> Result<RenderedImage> {
        bail!("rendering target {:?} is not implemented yet", target.id)
    }

    /// Write the edited document back to the project directory the way the
    /// editor saves: sources the edits did not touch keep their bytes.
    pub fn save(&mut self) -> Result<()> {
        bail!("saving is not implemented yet")
    }

    /// Every operation applied since [`open`](Self::open), in order: the
    /// journal the editor replays to bring an open document up to date.
    pub fn journal(&self) -> &[Operation] {
        &self.journal
    }

    /// The SHA-256 of every project file when it was opened, keyed by its
    /// `/`-separated path relative to the root.
    pub fn loaded_hashes(&self) -> &BTreeMap<String, [u8; 32]> {
        &self.loaded_hashes
    }
}

/// Compare the files under `work_dir` with the hashes they had when the
/// project was opened.
pub fn changeset(
    _loaded_hashes: &BTreeMap<String, [u8; 32]>,
    work_dir: &Path,
) -> Result<Vec<ChangedFile>> {
    bail!(
        "comparing {} with its opened state is not implemented yet",
        work_dir.display()
    )
}

/// One `SetMeta` operation that adds every draft to `page`'s comments, so a
/// review lands as a single undo step. Each draft gets its new comment id or
/// the reason it was refused; the operation is `None` when none was accepted.
pub fn agent_add_comments_op(
    _doc: &Doc,
    page: NodeId,
    drafts: &[CommentDraft],
) -> (Vec<Result<String>>, Option<Operation>) {
    let results = drafts
        .iter()
        .map(|_| {
            Err(anyhow::anyhow!(
                "adding comments to page {page} is not implemented yet"
            ))
        })
        .collect();
    (results, None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_refs_parse_from_each_contract_form() {
        let parse = |value| serde_json::from_value::<PageRef>(value);
        assert_eq!(
            parse(serde_json::json!({"index": 2})).expect("index form"),
            PageRef::Index { index: 2 }
        );
        assert_eq!(
            parse(serde_json::json!({"id": "n_01J0000000000000000000000A"})).expect("id form"),
            PageRef::Id {
                id: "n_01J0000000000000000000000A".into()
            }
        );
        assert_eq!(
            parse(serde_json::json!({"created": 0})).expect("created form"),
            PageRef::Created { created: 0 }
        );
        assert!(parse(serde_json::json!({"index": 1, "id": "n_x"})).is_err());
        assert!(parse(serde_json::json!({"name": "Home"})).is_err());
    }

    #[test]
    fn render_targets_default_to_the_largest_useful_image() {
        let target: RenderTarget = serde_json::from_value(serde_json::json!({
            "id": "home",
            "page": {"index": 0},
        }))
        .expect("a minimal render target parses");
        assert_eq!(target.max_dimension, MAX_RENDER_DIMENSION);
        assert_eq!(target.node, None);
    }
}
