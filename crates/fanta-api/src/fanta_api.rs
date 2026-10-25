//! Fanta's programmatic design API. A [`DesignSession`] holds one design, in
//! memory or opened from a project directory, and edits it node by node.
//! Every call applies at once and joins a pending undo step, which
//! [`DesignSession::add_undo_step`] commits. It runs without the app, over
//! `fanta-headless`, and is the surface language bindings wrap.

mod events;
mod nodes;
mod properties;

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::Context as _;
use fanta_design_ops::DesignOp;
use fanta_doc::{Doc, Operation};
use fanta_headless::{HeadlessDocument, HeadlessProject};

pub use events::{NodeEvent, NodeEventKind, NodeFilter, SubscriptionId};
pub use fanta_doc::{AssetId, Color, ComponentId, NodeId};
pub use fanta_doc::{BlurKind, ShadowKind};
pub use fanta_headless::export::{ExportEncoding, ExportSizing, ExportedImage};
pub use nodes::{NodeKind, NodeType};
pub use properties::{PropertyInfo, PropertyKind, PropertyValue};

/// Why a call was refused or failed.
#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("node {0} does not exist")]
    NodeNotFound(NodeId),
    #[error("{0}")]
    InvalidArgument(String),
    #[error("the session was not opened from a project directory")]
    NoProject,
    #[error("node {node} has no property `{path}`")]
    UnknownProperty { node: NodeId, path: String },
    #[error("`{path}` holds a {expected:?} value, not a {found:?}")]
    PropertyKind {
        path: String,
        expected: PropertyKind,
        found: PropertyKind,
    },
    #[error("`{value}` is not a value of `{path}`")]
    InvalidEnumValue { path: String, value: String },
    #[error("`{0}` is read-only")]
    ReadOnly(String),
    #[error("`{0}` is bound to a variable; unbind it before writing a value")]
    PropertyBound(String),
    #[error(transparent)]
    Engine(#[from] anyhow::Error),
}

pub type Result<T, E = ApiError> = std::result::Result<T, E>;

enum Backing {
    Memory(HeadlessDocument),
    Project(Box<HeadlessProject>),
}

/// One open design and the edits made to it.
pub struct DesignSession {
    backing: Backing,
    events: events::EventHub,
}

impl DesignSession {
    /// A new design in memory with one empty page.
    pub fn new() -> Result<Self> {
        Ok(Self::with_backing(Backing::Memory(
            HeadlessDocument::empty()?,
        )))
    }

    /// Open the Fanta project directory at `root`; [`save`](Self::save)
    /// writes the edits back to it.
    pub fn open_project(root: &Path) -> Result<Self> {
        Ok(Self::with_backing(Backing::Project(Box::new(
            HeadlessProject::open(root)?,
        ))))
    }

    /// Open a `.fant` snapshot, a design with its assets, in memory.
    pub fn open_snapshot(path: &Path) -> Result<Self> {
        let snapshot = fanta_format::FantaFile::open(path)
            .with_context(|| format!("opening {}", path.display()))?;
        let mut doc = snapshot
            .load_doc()
            .context("reading the snapshot's design")?;
        if doc.active_page().is_none() {
            let first = doc.pages().first().copied();
            doc.set_active_page(first);
        }
        let mut assets = BTreeMap::new();
        for asset in snapshot.list_assets() {
            let bytes = snapshot
                .get_asset(asset)
                .with_context(|| format!("reading asset {asset}"))?;
            assets.insert(asset, bytes);
        }
        Ok(Self::with_backing(Backing::Memory(HeadlessDocument::new(
            doc, assets,
        ))))
    }

    /// Write the edits back to the project directory the session was opened
    /// from. Sources the edits did not touch keep their bytes, and the
    /// pending undo step stays open.
    pub fn save(&mut self) -> Result<()> {
        match &mut self.backing {
            Backing::Project(project) => Ok(project.save()?),
            Backing::Memory(_) => Err(ApiError::NoProject),
        }
    }

    /// Write the design as a new Fanta project directory at `root`, which
    /// must not already hold a project. The session keeps editing its own
    /// document.
    pub fn write_project(&self, root: &Path) -> Result<()> {
        if fanta_format::is_project_dir(root) {
            return Err(ApiError::InvalidArgument(format!(
                "{} already holds a Fanta project",
                root.display()
            )));
        }
        fanta_format::scaffold_project_tree(root)
            .with_context(|| format!("creating a project at {}", root.display()))?;
        fanta_format::write_project_tree(root, self.doc(), self.document().raw_assets())
            .with_context(|| format!("writing the project at {}", root.display()))?;
        Ok(())
    }

    /// Write the design and its assets as a `.fant` snapshot at `path`.
    pub fn write_snapshot(&self, path: &Path) -> Result<()> {
        let mut snapshot = fanta_format::FantaFile::create(path)
            .with_context(|| format!("creating {}", path.display()))?;
        snapshot
            .save_doc(&self.doc().clone_for_persist())
            .context("writing the snapshot's design")?;
        for (asset, bytes) in self.document().raw_assets() {
            snapshot
                .put_asset_as(*asset, bytes)
                .with_context(|| format!("writing asset {asset}"))?;
        }
        Ok(())
    }

    /// Export `node` as `encoding`; a page root exports the page. The image
    /// covers what the node draws, with variable values applied. `sizing`
    /// sets the pixel size of PNG and JPEG; SVG and PDF use the design's own
    /// size.
    pub fn export(
        &self,
        node: NodeId,
        encoding: ExportEncoding,
        sizing: ExportSizing,
    ) -> Result<ExportedImage> {
        self.node(node)?;
        Ok(fanta_headless::export::export_node(
            self.doc(),
            self.document().asset_resolver(),
            node,
            encoding,
            sizing,
        )?)
    }

    /// The document, read-only. Edit it through the session's calls.
    pub fn doc(&self) -> &Doc {
        self.document().doc()
    }

    /// The pages' root nodes, in page order.
    pub fn pages(&self) -> Vec<NodeId> {
        self.doc().pages().to_vec()
    }

    /// The page new nodes land on when a call names no parent.
    pub fn active_page(&self) -> Option<NodeId> {
        self.doc().active_page()
    }

    pub fn set_active_page(&mut self, page: NodeId) -> Result<()> {
        if !self.doc().pages().contains(&page) {
            return Err(ApiError::InvalidArgument(format!("{page} is not a page")));
        }
        self.document_mut().doc_mut().set_active_page(Some(page));
        Ok(())
    }

    /// Add an empty page named `name` after the last page and return its
    /// root.
    pub fn create_page(&mut self, name: &str) -> Result<NodeId> {
        let name = name.trim();
        if name.is_empty() || name.len() > 512 {
            return Err(ApiError::InvalidArgument(
                "a page name must contain 1–512 bytes".into(),
            ));
        }
        if self.doc().pages().is_empty() {
            return Err(ApiError::InvalidArgument(
                "the design has no pages to add one beside".into(),
            ));
        }
        let (root, operations) =
            fanta_headless::pages::add_page_operations(self.doc(), name.to_owned());
        self.edit(operations)?;
        Ok(root)
    }

    /// Commit the pending edits as one undo step named `label` (an empty
    /// label names it after its first edit). Returns whether there was
    /// anything to commit.
    pub fn add_undo_step(&mut self, label: &str) -> Result<bool> {
        let document = self.document_mut();
        let doc = document.doc_mut();
        let pending = doc.history.open_len().is_some_and(|len| len > 0);
        if !label.is_empty() {
            doc.history.set_open_label(label);
        }
        doc.history.commit(&mut doc.scene);
        document.settle_layout()?;
        Ok(pending)
    }

    /// Undo the last step. Pending edits are committed first, so they are
    /// what is undone. Returns whether a step was undone.
    pub fn undo(&mut self) -> Result<bool> {
        let undone = (!self.events.is_empty())
            .then(|| self.doc().history.next_undo_transaction().cloned())
            .flatten();
        let did = self.document_mut().undo()?;
        if did && let Some(transaction) = undone {
            let mut events = Vec::new();
            for operation in transaction.ops.iter().rev() {
                events::node_events(
                    self.doc(),
                    operation,
                    events::Direction::Revert,
                    &mut events,
                );
            }
            self.events.record(&events);
        }
        Ok(did)
    }

    /// Redo the last undone step. Edits made since the undo branched the
    /// history, so after them there is nothing to redo. Returns whether a
    /// step was redone.
    pub fn redo(&mut self) -> Result<bool> {
        let document = self.document_mut();
        let doc = document.doc_mut();
        match doc.history.open_len() {
            Some(0) => doc
                .abort_transaction()
                .context("closing the empty pending step")?,
            Some(_) => return Ok(false),
            None => {}
        }
        let redone = (!self.events.is_empty())
            .then(|| self.doc().history.next_redo_transaction().cloned())
            .flatten();
        let did = self.document_mut().redo()?;
        if did && let Some(transaction) = redone {
            let mut events = Vec::new();
            for operation in &transaction.ops {
                events::node_events(self.doc(), operation, events::Direction::Apply, &mut events);
            }
            self.events.record(&events);
        }
        Ok(did)
    }

    pub fn can_undo(&self) -> bool {
        let history = &self.doc().history;
        history.open_len().is_some_and(|len| len > 0) || history.can_undo()
    }

    pub fn can_redo(&self) -> bool {
        let history = &self.doc().history;
        history.open_len().is_none_or(|len| len == 0) && history.can_redo()
    }

    fn with_backing(backing: Backing) -> Self {
        Self {
            backing,
            events: events::EventHub::default(),
        }
    }

    pub(crate) fn document(&self) -> &HeadlessDocument {
        match &self.backing {
            Backing::Memory(document) => document,
            Backing::Project(project) => project.document(),
        }
    }

    pub(crate) fn document_mut(&mut self) -> &mut HeadlessDocument {
        match &mut self.backing {
            Backing::Memory(document) => document,
            Backing::Project(project) => project.document_mut(),
        }
    }

    /// Apply `operations` into the pending undo step, then settle the layout
    /// they moved. When an operation fails, only this call's operations are
    /// rolled back.
    pub(crate) fn edit(&mut self, operations: Vec<Operation>) -> Result<()> {
        if operations.is_empty() {
            return Ok(());
        }
        let document = self.document_mut();
        let doc = document.doc_mut();
        let savepoint = open_pending_step(doc);
        for operation in operations {
            if let Err(error) = doc.apply(operation) {
                doc.rollback_open_transaction(savepoint)
                    .context("rolling back the failed edit")?;
                return Err(anyhow::Error::new(error)
                    .context("applying the edit")
                    .into());
            }
        }
        let moved = document.settle_layout()?;
        self.record_edit(savepoint, &moved);
        Ok(())
    }

    /// Apply one design op into the pending undo step, with parentless ops
    /// placed on the active page. Returns the node it created, if any.
    pub(crate) fn edit_with_design_op(&mut self, operation: &DesignOp) -> Result<Option<NodeId>> {
        let document = self.document_mut();
        let default_container = document.doc().active_page();
        let (created, savepoint) = {
            let (doc, mut images) = document.doc_and_images();
            let savepoint = open_pending_step(doc);
            match fanta_headless::apply::apply_design_op(
                doc,
                &mut images,
                operation,
                default_container,
            ) {
                Ok(created) => (created, savepoint),
                Err(error) => {
                    doc.rollback_open_transaction(savepoint)
                        .context("rolling back the failed edit")?;
                    return Err(error.into());
                }
            }
        };
        let moved = document.settle_layout()?;
        self.record_edit(savepoint, &moved);
        Ok(created)
    }

    /// Report the operations the pending step gained since `savepoint`, and
    /// the nodes the layout solve moved, to the event subscribers.
    fn record_edit(&mut self, savepoint: usize, moved: &BTreeSet<NodeId>) {
        if self.events.is_empty() {
            return;
        }
        let doc = self.doc();
        let mut events = Vec::new();
        if let Some(transaction) = doc.history.open_transaction() {
            for operation in transaction.ops.get(savepoint..).unwrap_or_default() {
                events::node_events(doc, operation, events::Direction::Apply, &mut events);
            }
        }
        events.extend(moved.iter().map(|node| NodeEvent {
            node: *node,
            kind: NodeEventKind::Updated,
        }));
        self.events.record(&events);
    }
}

/// Open the pending undo step if none is open and return the savepoint a
/// failing edit rolls back to.
fn open_pending_step(doc: &mut Doc) -> usize {
    if doc.history.open_len().is_none() {
        doc.history.begin("", &mut doc.scene);
    }
    doc.history.open_len().unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_session_has_one_page_and_no_history() {
        let session = DesignSession::new().expect("session");
        assert_eq!(session.pages().len(), 1);
        assert_eq!(session.active_page(), session.pages().first().copied());
        assert!(!session.can_undo());
        assert!(!session.can_redo());
    }

    #[test]
    fn edits_accumulate_until_add_undo_step() {
        let mut session = DesignSession::new().expect("session");
        let page = session.active_page().expect("page");
        let first = session
            .create_node(page, NodeKind::Rectangle)
            .expect("rectangle");
        let second = session
            .create_node(page, NodeKind::Ellipse)
            .expect("ellipse");
        assert!(session.can_undo());
        assert_eq!(session.doc().history.undo_depth(), 0, "still pending");
        assert!(session.add_undo_step("Two shapes").expect("commit"));
        assert_eq!(session.doc().history.undo_depth(), 1);
        assert_eq!(session.doc().history.next_undo_label(), Some("Two shapes"));
        assert!(!session.add_undo_step("Nothing").expect("commit"));
        assert!(session.undo().expect("undo"));
        assert!(!session.is_valid(first) && !session.is_valid(second));
        assert!(session.redo().expect("redo"));
        assert!(session.is_valid(first) && session.is_valid(second));
    }

    #[test]
    fn undo_reverts_uncommitted_edits_as_one_step() {
        let mut session = DesignSession::new().expect("session");
        let page = session.active_page().expect("page");
        let frame = session.create_node(page, NodeKind::Frame).expect("frame");
        session.set_name(frame, "Hero").expect("rename");
        assert!(session.undo().expect("undo"));
        assert!(!session.is_valid(frame));
        assert!(!session.can_undo());
        assert!(session.can_redo());
    }

    #[test]
    fn redo_is_refused_after_new_edits() {
        let mut session = DesignSession::new().expect("session");
        let page = session.active_page().expect("page");
        let frame = session.create_node(page, NodeKind::Frame).expect("frame");
        session.add_undo_step("Frame").expect("commit");
        assert!(session.undo().expect("undo"));
        let ellipse = session
            .create_node(page, NodeKind::Ellipse)
            .expect("ellipse");
        assert!(
            !session.redo().expect("redo"),
            "the new edit branched history"
        );
        assert!(!session.is_valid(frame));
        assert!(session.is_valid(ellipse));
    }

    #[test]
    fn export_renders_a_node_at_a_target_size() {
        let mut session = DesignSession::new().expect("session");
        let page = session.active_page().expect("page");
        let mut pixels = Vec::new();
        image::RgbaImage::from_pixel(4, 2, image::Rgba([0, 0, 255, 255]))
            .write_to(
                &mut std::io::Cursor::new(&mut pixels),
                image::ImageFormat::Png,
            )
            .expect("encode png");
        let photo = session
            .create_node(page, NodeKind::Image(pixels))
            .expect("image");
        let exported = session
            .export(photo, ExportEncoding::Png, ExportSizing::Height(20.0))
            .expect("export");
        assert_eq!((exported.width, exported.height), (40, 20));
        let decoded = image::load_from_memory(&exported.bytes)
            .expect("decode")
            .to_rgba8();
        assert_eq!(
            decoded.get_pixel(20, 10).0,
            [0, 0, 255, 255],
            "the ingested asset renders"
        );
        let page_export = session
            .export(page, ExportEncoding::Pdf, ExportSizing::Scale(1.0))
            .expect("page");
        assert!(page_export.bytes.starts_with(b"%PDF"));
    }

    #[test]
    fn a_failed_call_rolls_back_only_its_own_operations() {
        let mut session = DesignSession::new().expect("session");
        let page = session.active_page().expect("page");
        let rectangle = session
            .create_node(page, NodeKind::Rectangle)
            .expect("rectangle");
        let error = session
            .create_node(page, NodeKind::Path("not svg path data".into()))
            .expect_err("bad path data");
        assert!(!error.to_string().is_empty());
        assert!(session.is_valid(rectangle), "earlier pending edits survive");
        assert!(session.add_undo_step("Rectangle").expect("commit"));
        assert_eq!(session.children(page).expect("children"), vec![rectangle]);
    }
}
