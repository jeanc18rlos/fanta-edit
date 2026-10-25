//! Fanta's design engine without the app: open a project directory, review or
//! edit it with design ops, settle its layout, render its pages and save it
//! back. The cloud engine worker (`fanta-engine`) drives it and the editor
//! shares its code, so nothing here may depend on GPUI.
//!
//! The facade is frozen by `docs/cloud-engine-contracts.md`.

pub mod apply;
pub mod assets;
pub mod clipboard;
pub mod comments;
pub mod component_properties;
pub mod componentize;
pub mod edit_operations;
pub mod layout;
pub mod pages;
pub mod render;
pub mod save;
pub mod scene;
pub mod structure;
pub mod style;
pub mod variant_sets;

use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::{Arc, mpsc},
};

use anyhow::{Context as _, Result, anyhow, bail};
use fanta_design_ops::{
    CommentDraft, DEFAULT_CHILD_LIMIT, DesignOp, DesignSystemQuery, ScreenshotTarget,
};
use fanta_doc::{
    AssetId, Doc, JournalEvent, JournalSink, NodeData, NodeId, Operation, Transaction,
};
use fanta_format::{ArtifactId, ProjectWriteCache, ScopedDesign, WorkspaceSession};
use fanta_render::{AssetResolver, asset::LazyAssetResolver};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

pub use comments::agent_add_comments_op;

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

/// The most top-level layers `inspect` lists per page.
const INSPECT_FRAME_LIMIT: usize = 200;

/// An open project and the edits made to it since it was opened.
pub struct HeadlessProject {
    root: PathBuf,
    doc: Doc,
    /// The document as last read or saved: what a save compares against.
    baseline: Doc,
    /// The scene's instance and revision at `baseline`.
    saved_scene: (u64, u64),
    /// Nodes whose geometry was solved because the sources left it out. The
    /// next save writes it, as the editor's first save after opening does.
    pending_layout_output_nodes: BTreeSet<NodeId>,
    raw_assets: Arc<BTreeMap<AssetId, Vec<u8>>>,
    assets_changed: bool,
    asset_resolver: Option<Arc<dyn AssetResolver>>,
    session: WorkspaceSession,
    write_cache: ProjectWriteCache,
    layout: layout::LayoutState,
    pages: Vec<Option<NodeId>>,
    created_pages: Vec<NodeId>,
    journal_events: mpsc::Receiver<JournalEvent>,
    transactions: Vec<Transaction>,
    journal: Vec<Operation>,
    loaded_hashes: BTreeMap<String, [u8; 32]>,
    warnings: Vec<String>,
}

impl HeadlessProject {
    /// Read the project at `root` the way the editor opens it, solve the
    /// geometry its sources leave out, and record the hash of every file so
    /// [`changeset`] can report what a later [`save`](Self::save) changed.
    pub fn open(root: &Path) -> Result<Self> {
        let root = root
            .canonicalize()
            .with_context(|| format!("resolving {}", root.display()))?;
        let loaded_hashes = hash_project_files(&root)?;
        let (mut doc, raw_assets, session) = WorkspaceSession::open_project_document(&root)
            .with_context(|| format!("reading Fanta project at {}", root.display()))?;
        // Overrides that merely restate the master pin an instance and keep
        // master edits from reaching it; the editor drops them on load too.
        fanta_doc::strip_redundant_instance_overrides(&mut doc.scene, &doc.components);
        let visible_page_roots = pages::visible_page_roots(&doc);
        let default_page_root = pages::default_page_root(&doc, &visible_page_roots);
        let mut warnings = Vec::new();
        let pending_layout_output_nodes = layout::solve_pending(&mut doc).unwrap_or_else(|error| {
            warnings.push(format!("computing omitted source geometry: {error:#}"));
            BTreeSet::new()
        });
        let layout = layout::LayoutState::new(&doc);
        let page_roots = pages::editor_page_roots(&doc);
        // The active page is runtime state that the project does not store,
        // so pick the page the editor would open on, as it does.
        let default_index = default_page_root
            .and_then(|root| page_roots.iter().position(|page| *page == Some(root)))
            .or_else(|| {
                page_roots
                    .iter()
                    .position(|page| !page.is_some_and(|root| pages::page_hidden(&doc, root)))
            })
            .unwrap_or(0);
        if doc
            .active_page()
            .is_none_or(|active| pages::page_hidden(&doc, active))
        {
            doc.set_active_page(page_roots.get(default_index).copied().flatten());
        }
        let (sender, journal_events) = mpsc::channel();
        doc.set_journal_sink(Some(JournalSink::new(sender)));
        let baseline = doc.clone_for_persist();
        let saved_scene = (doc.scene.instance_id(), doc.scene.revision());
        let raw_assets = Arc::new(raw_assets);
        let asset_resolver = (!raw_assets.is_empty()).then(|| {
            Arc::new(LazyAssetResolver::with_size_reader(
                raw_assets.clone(),
                Arc::new(render::decode_embedded_image),
                Arc::new(render::embedded_image_size),
            )) as Arc<dyn AssetResolver>
        });
        Ok(Self {
            root,
            doc,
            baseline,
            saved_scene,
            pending_layout_output_nodes,
            raw_assets,
            assets_changed: false,
            asset_resolver,
            session,
            write_cache: ProjectWriteCache::default(),
            layout,
            pages: page_roots,
            created_pages: Vec::new(),
            journal_events,
            transactions: Vec::new(),
            journal: Vec::new(),
            loaded_hashes,
            warnings,
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn doc(&self) -> &Doc {
        &self.doc
    }

    /// Problems that did not stop the project from opening, worded for a
    /// person reading a review.
    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }

    /// The project summary an engine `inspect` step returns: pages with their
    /// top-level frames, components, the design system, the project's
    /// `fanta.md` rules, motion clips, prototype flows and the font families
    /// the text uses.
    pub fn inspect(&self) -> Result<Value> {
        let doc = &self.doc;
        let active = doc.active_page();
        let pages = self
            .pages
            .iter()
            .enumerate()
            .map(|(index, root)| {
                let (name, hidden) = pages::page_label(doc, *root, index);
                let layers = match root {
                    Some(root) => doc.scene.children_of(Some(*root)),
                    None => doc.scene.roots(),
                };
                json!({
                    "index": index,
                    "id": root.map(|root| root.to_string()),
                    "name": scene::truncate_summary_string(&name, scene::SUMMARY_LABEL_CHARS),
                    "hidden": hidden,
                    "active": root.is_some() && *root == active,
                    "source": root.and_then(|root| {
                        let design = if doc.pages().contains(&root) {
                            Some(ScopedDesign::Page(root))
                        } else {
                            doc.components
                                .defs
                                .values()
                                .find(|definition| definition.root == root)
                                .map(|definition| ScopedDesign::Component(definition.id))
                        };
                        design.and_then(|design| self.indexed_design_source(design))
                    }).map(|source| scene::relative_source(Some(&self.root), Some(source))),
                    "frames": layers
                        .iter()
                        .take(INSPECT_FRAME_LIMIT)
                        .filter_map(|id| {
                            let node = doc.scene.get(*id)?;
                            Some(json!({
                                "id": id.to_string(),
                                "kind": node.data.kind_tag(),
                                "name": scene::truncate_summary_string(&node.name, scene::SUMMARY_LABEL_CHARS),
                                "bounds": scene::world_bounds_json(doc, *id),
                            }))
                        })
                        .collect::<Vec<_>>(),
                    "frame_count": layers.len(),
                })
            })
            .collect::<Vec<_>>();
        let (rules, rules_error) = match fanta_design_ops::read_project_design_spec(&self.root) {
            Ok(spec) => (spec.map(|spec| spec.content), None),
            Err(error) => (None, Some(format!("{error:#}"))),
        };
        let motion = doc
            .motion
            .clips
            .values()
            .map(|clip| {
                json!({
                    "id": clip.id.to_string(), "name": clip.name, "duration_ms": clip.duration_ms,
                    "track_count": clip.tracks.len(),
                    "fps": clip.settings.fps, "loop_mode": clip.settings.loop_mode,
                    "scope": clip.settings.scope.map(|scope| scope.to_string()),
                })
            })
            .collect::<Vec<_>>();
        Ok(json!({
            "project_id": doc.id.to_string(),
            "schema_version": doc.schema_version,
            "pages": pages,
            "active_page": self.active_page_index().ok(),
            "components": scene::components_json(
                doc,
                Some(&self.root),
                0,
                scene::COMPONENT_OVERVIEW_LIMIT,
                |design| self.indexed_design_source(design),
            ),
            "component_count": doc.components.defs.len(),
            "design_system": scene::design_system_json(
                doc,
                Some(&self.root),
                DesignSystemQuery::default(),
                |design| self.indexed_design_source(design),
            )?,
            "rules": rules,
            "rules_error": rules_error,
            "motion": motion,
            "flows": doc.flows,
            "fonts": self.font_families(),
        }))
    }

    /// The font families the project's text names, in its styles and its
    /// rich-text runs.
    pub fn font_families(&self) -> BTreeSet<String> {
        let mut families = BTreeSet::new();
        for root in self.doc.scene.roots() {
            for id in self.doc.scene.descendants_of(*root) {
                let Some(node) = self.doc.scene.get(id) else {
                    continue;
                };
                let (style, runs) = match &node.data {
                    NodeData::Text(text) => (&text.style, &text.style_runs),
                    NodeData::TextPath(text) => (&text.style, &text.style_runs),
                    _ => continue,
                };
                families.extend(
                    std::iter::once(style)
                        .chain(runs.iter().map(|run| &run.style))
                        .map(|style| style.font_family.clone())
                        .filter(|family| !family.is_empty()),
                );
            }
        }
        families
    }

    /// The root node of `page`.
    pub fn resolve_page(&self, page: &PageRef) -> Result<NodeId> {
        match page {
            PageRef::Index { index } => self.page_root(*index),
            PageRef::Id { id } => {
                let root = parse_node_id(id)?;
                anyhow::ensure!(self.pages.contains(&Some(root)), "{id} is not a page");
                Ok(root)
            }
            PageRef::Created { created } => self
                .created_pages
                .get(*created)
                .copied()
                .with_context(|| format!("no create_page step made page {created}")),
        }
    }

    /// Add an empty page named `name` after the last authored page, as the
    /// editor's Add Page does, and return its root. It becomes the active
    /// page.
    pub fn create_page(&mut self, name: &str) -> Result<NodeId> {
        let name = name.trim();
        anyhow::ensure!(
            !name.is_empty() && name.len() <= 512,
            "a page name must contain 1–512 bytes"
        );
        anyhow::ensure!(
            !self.doc.pages().is_empty(),
            "the project has no pages to add one beside"
        );
        let (root, operations) = pages::add_page_operations(&self.doc, name.to_owned());
        self.transaction("Add Page", operations)?;
        self.pages = pages::editor_page_roots(&self.doc);
        self.created_pages.push(root);
        Ok(root)
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
    ) -> Result<Value> {
        bail!("applying {label:?} is not implemented yet: the op applier still lives in fig_viewer")
    }

    /// Place agent comments as one undo step labelled `label`. A draft without
    /// a `page` goes on the active page. Returns each draft's new comment id,
    /// or why that draft was refused.
    pub fn add_comments(
        &mut self,
        label: &str,
        drafts: &[CommentDraft],
    ) -> Result<Vec<Result<String>>> {
        let active = self.active_page_index();
        let mut by_page: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
        let mut results: Vec<Result<String>> = drafts
            .iter()
            .map(|_| Err(anyhow!("the comment was not placed")))
            .collect();
        for (position, draft) in drafts.iter().enumerate() {
            let page = match (draft.page, &active) {
                (Some(page), _) => page,
                (None, Ok(active)) => *active,
                (None, Err(error)) => {
                    results[position] = Err(anyhow!("{error:#}"));
                    continue;
                }
            };
            by_page.entry(page).or_default().push(position);
        }
        let mut operations = Vec::new();
        for (page, positions) in by_page {
            let root = match self.page_root(page) {
                Ok(root) => root,
                Err(error) => {
                    for position in positions {
                        results[position] = Err(anyhow!("{error:#}"));
                    }
                    continue;
                }
            };
            let page_drafts = positions
                .iter()
                .map(|position| drafts[*position].clone())
                .collect::<Vec<_>>();
            let (outcomes, operation) =
                comments::agent_add_comments_op(&self.doc, root, &page_drafts);
            operations.extend(operation);
            for (position, outcome) in positions.into_iter().zip(outcomes) {
                results[position] = outcome;
            }
        }
        self.transaction(label, operations)?;
        Ok(results)
    }

    /// Re-solve the auto layout and auto-sized text that edits invalidated.
    pub fn settle_layout(&mut self) -> Result<()> {
        let result = layout::settle(&mut self.doc, &mut self.layout);
        self.drain_journal();
        result
    }

    /// `page`'s top-level layers, `depth` levels deep and at most `limit` of
    /// them, in the style projection the editor's `batch_get` returns for
    /// `detail: "style"`.
    pub fn project_page(&self, page: NodeId, depth: u32, limit: usize) -> Result<Vec<Value>> {
        anyhow::ensure!(self.pages.contains(&Some(page)), "{page} is not a page");
        scene::page_scene(&self.doc, page, depth, limit.clamp(1, DEFAULT_CHILD_LIMIT))
    }

    pub fn render(&mut self, target: &RenderTarget) -> Result<RenderedImage> {
        let page = self.resolve_page(&target.page)?;
        let node = target.node.as_deref().map(parse_node_id).transpose()?;
        if let Some(node) = node {
            anyhow::ensure!(
                self.doc.scene.get(node).is_some(),
                "node {node} does not exist"
            );
            anyhow::ensure!(
                node == page
                    || self
                        .doc
                        .scene
                        .ancestors_of(node)
                        .any(|ancestor| ancestor.id == page),
                "node {node} is not on page {page}"
            );
        }
        self.settle_layout()?;
        let screenshot = render::render_screenshot(
            &self.doc,
            self.asset_resolver.clone(),
            Some(page),
            node,
            ScreenshotTarget {
                page: None,
                node: None,
                max_dimension: Some(target.max_dimension.min(MAX_RENDER_DIMENSION)),
                motion_clip: target.motion_clip.clone(),
                playhead_ms: target.playhead_ms,
            },
        )?;
        Ok(RenderedImage {
            target_id: target.id.clone(),
            png: screenshot.png,
            width: screenshot.width,
            height: screenshot.height,
            bounds: [
                screenshot.bounds.min_x,
                screenshot.bounds.min_y,
                screenshot.bounds.width(),
                screenshot.bounds.height(),
            ],
        })
    }

    /// Write the edited document back to the project directory the way the
    /// editor saves: sources the edits did not touch keep their bytes.
    pub fn save(&mut self) -> Result<()> {
        self.settle_layout()?;
        let mut changed_artifacts =
            save::changed_artifact_ids(&self.doc, Some(&self.baseline), Some(self.saved_scene));
        save::extend_artifact_owners(
            &self.doc,
            &mut changed_artifacts,
            self.pending_layout_output_nodes.iter().copied(),
        );
        save::save_project(
            &self.root,
            &mut self.session,
            &self.doc,
            &self.baseline,
            &self.raw_assets,
            self.assets_changed,
            &changed_artifacts,
            &mut self.write_cache,
        )?;
        self.baseline = self.doc.clone_for_persist();
        self.saved_scene = (self.doc.scene.instance_id(), self.doc.scene.revision());
        self.pending_layout_output_nodes.clear();
        self.assets_changed = false;
        Ok(())
    }

    /// Every operation committed since [`open`](Self::open), in order, with
    /// the geometry each edit's layout solve added: the journal the editor
    /// replays to bring an open document up to date.
    pub fn journal(&self) -> &[Operation] {
        &self.journal
    }

    /// The SHA-256 of every project file when it was opened, keyed by its
    /// `/`-separated path relative to the root.
    pub fn loaded_hashes(&self) -> &BTreeMap<String, [u8; 32]> {
        &self.loaded_hashes
    }

    fn page_root(&self, index: usize) -> Result<NodeId> {
        self.pages
            .get(index)
            .with_context(|| {
                format!(
                    "page index {index} is out of range (the project has {} pages)",
                    self.pages.len()
                )
            })?
            .context("the project has no pages")
    }

    fn active_page_index(&self) -> Result<usize> {
        let active = self
            .doc
            .active_page()
            .context("the project has no active page")?;
        self.pages
            .iter()
            .position(|page| *page == Some(active))
            .context("the active page is not in the page list")
    }

    fn indexed_design_source(&self, design: ScopedDesign) -> Option<PathBuf> {
        let artifact = ArtifactId::from_scoped(design);
        if !self.session.has_indexed_source(&artifact) {
            return None;
        }
        let metadata = self.session.artifacts.get(&artifact)?;
        let file_name = match design {
            ScopedDesign::Page(_) => "page.fnx",
            ScopedDesign::Component(_) => "master.fnx",
        };
        Some(self.root.join(&metadata.design_dir).join(file_name))
    }

    /// Apply `operations` as one undo step, then settle the layout it moved.
    fn transaction(&mut self, label: &str, operations: Vec<Operation>) -> Result<()> {
        if operations.is_empty() {
            return Ok(());
        }
        self.doc.history.begin(label, &mut self.doc.scene);
        for operation in operations {
            if let Err(error) = self.doc.apply(operation) {
                self.doc
                    .abort_transaction()
                    .context("rolling back the edit")?;
                return Err(error).context("applying the edit");
            }
        }
        self.doc.history.commit(&mut self.doc.scene);
        self.settle_layout()
    }

    fn drain_journal(&mut self) {
        let mut changed = false;
        while let Ok(event) = self.journal_events.try_recv() {
            changed = true;
            match event {
                JournalEvent::Commit(transaction) | JournalEvent::Redo(transaction) => {
                    self.transactions.push(transaction);
                }
                JournalEvent::Amend(transaction) => match self.transactions.last_mut() {
                    Some(last) => *last = transaction,
                    None => self.transactions.push(transaction),
                },
                JournalEvent::Undo(_) => {
                    self.transactions.pop();
                }
            }
        }
        if changed {
            self.journal = self
                .transactions
                .iter()
                .flat_map(|transaction| transaction.ops.iter().cloned())
                .collect();
        }
    }
}

/// Compare the files under `work_dir` with the hashes they had when the
/// project was opened.
pub fn changeset(
    loaded_hashes: &BTreeMap<String, [u8; 32]>,
    work_dir: &Path,
) -> Result<Vec<ChangedFile>> {
    let current = hash_project_files(work_dir)?;
    let mut changes = Vec::new();
    for (path, hash) in &current {
        let status = match loaded_hashes.get(path) {
            None => FileStatus::Added,
            Some(loaded) if loaded != hash => FileStatus::Modified,
            Some(_) => continue,
        };
        changes.push(ChangedFile {
            path: path.clone(),
            status,
            sha256: Some(hex_digest(hash)),
        });
    }
    for path in loaded_hashes.keys() {
        if !current.contains_key(path) {
            changes.push(ChangedFile {
                path: path.clone(),
                status: FileStatus::Deleted,
                sha256: None,
            });
        }
    }
    changes.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(changes)
}

/// The SHA-256 of every file under `root`, keyed by its `/`-separated path.
/// Hidden entries (`.git`, editor caches) are not project content.
fn hash_project_files(root: &Path) -> Result<BTreeMap<String, [u8; 32]>> {
    let mut hashes = BTreeMap::new();
    let mut pending = vec![(root.to_path_buf(), String::new())];
    while let Some((directory, prefix)) = pending.pop() {
        for entry in std::fs::read_dir(&directory)
            .with_context(|| format!("reading {}", directory.display()))?
        {
            let entry = entry?;
            let name = entry.file_name();
            let name = name
                .to_str()
                .with_context(|| format!("{} is not UTF-8", entry.path().display()))?;
            if name.starts_with('.') {
                continue;
            }
            let path = format!("{prefix}{name}");
            let file_type = entry.file_type()?;
            if file_type.is_dir() {
                pending.push((entry.path(), format!("{path}/")));
            } else if file_type.is_file() {
                let bytes = std::fs::read(entry.path())
                    .with_context(|| format!("reading {}", entry.path().display()))?;
                hashes.insert(path, Sha256::digest(&bytes).into());
            }
        }
    }
    Ok(hashes)
}

fn hex_digest(hash: &[u8; 32]) -> String {
    hash.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn parse_node_id(raw: &str) -> Result<NodeId> {
    raw.parse::<NodeId>()
        .map_err(|_| anyhow!("`{raw}` is not a valid node id"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use fanta_doc::{CanvasNode, GroupNode, Transform2D, VectorNode, color::Color};

    struct Fixture {
        directory: tempfile::TempDir,
        home: NodeId,
        about: NodeId,
        card: NodeId,
    }

    /// A two-page project on disk: "Home" holds a white card, "About" a dark
    /// panel.
    fn write_project() -> Fixture {
        let mut doc = Doc::new();
        let mut roots = Vec::new();
        for name in ["Home", "About"] {
            let mut page = CanvasNode::new(NodeData::Group(GroupNode::default()));
            page.name = name.into();
            let id = page.id;
            doc.apply(Operation::create_node(page)).expect("page");
            doc.add_page(id);
            roots.push(id);
        }
        let mut layers = Vec::new();
        for (page, name, color, x) in [
            (roots[0], "Card", Color::WHITE, 40.0),
            (roots[1], "Panel", Color::BLACK, 0.0),
        ] {
            let mut layer = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
                0.0, 0.0, 120.0, 80.0, color,
            )));
            layer.name = name.into();
            layer.parent = Some(page);
            layer.transform = Transform2D::translation(x, 30.0);
            layers.push(layer.id);
            doc.apply(Operation::create_node(layer)).expect("layer");
        }
        let directory = tempfile::tempdir().expect("project directory");
        fanta_format::write_project_tree(directory.path(), &doc, &BTreeMap::new())
            .expect("write the project");
        Fixture {
            directory,
            home: roots[0],
            about: roots[1],
            card: layers[0],
        }
    }

    fn finding(page: usize, node: Option<NodeId>, text: &str) -> CommentDraft {
        CommentDraft {
            page: Some(page),
            node: node.map(|node| node.to_string()),
            world: node.is_none().then_some([5.0, 5.0]),
            text: text.into(),
            author: Some("Fanta Review".into()),
            severity: Some("major".into()),
            category: Some("accessibility".into()),
            review_run_id: Some("rev_1".into()),
            ..Default::default()
        }
    }

    #[test]
    fn inspect_lists_the_pages_their_frames_and_sources() {
        let fixture = write_project();
        let project = HeadlessProject::open(fixture.directory.path()).expect("open");
        let summary = project.inspect().expect("inspect");
        let pages = summary["pages"].as_array().expect("pages");
        assert_eq!(pages.len(), 2);
        assert_eq!(pages[0]["name"], "Home");
        assert_eq!(pages[0]["id"], fixture.home.to_string());
        assert_eq!(pages[1]["name"], "About");
        assert_eq!(pages[0]["frames"][0]["id"], fixture.card.to_string());
        assert_eq!(pages[0]["frames"][0]["name"], "Card");
        assert_eq!(pages[0]["frame_count"], 1);
        assert!(
            pages[0]["source"]
                .as_str()
                .is_some_and(|source| source.starts_with("pages/") && source.ends_with("/page.fnx")),
            "{}",
            pages[0]["source"]
        );
        assert_eq!(summary["project_id"], project.doc().id.to_string());
        assert!(
            summary["rules"]
                .as_str()
                .is_some_and(|rules| rules.starts_with("# Project design specification")),
            "new projects are seeded with a fanta.md the review reads"
        );
        assert!(project.warnings().is_empty());
    }

    #[test]
    fn a_review_lands_on_each_page_saves_only_their_sources_and_reopens() {
        let fixture = write_project();
        let mut project = HeadlessProject::open(fixture.directory.path()).expect("open");
        let results = project
            .add_comments(
                "AI Review",
                &[
                    finding(0, Some(fixture.card), "**Contrast**\n\nWhite on white."),
                    finding(1, None, "**Spacing**\n\nThe panel touches the edge."),
                    finding(7, None, "On a page that does not exist."),
                ],
            )
            .expect("add comments");
        assert!(results[0].is_ok() && results[1].is_ok());
        assert!(
            results[2]
                .as_ref()
                .is_err_and(|error| error.to_string().contains("out of range"))
        );
        assert!(
            project
                .journal()
                .iter()
                .all(|operation| matches!(operation, Operation::SetMeta { .. })),
            "a review only adds comments"
        );
        assert_eq!(project.journal().len(), 2, "one comment write per page");

        project.save().expect("save");
        let changed = changeset(project.loaded_hashes(), project.root()).expect("changeset");
        // Every edit stamps `modified_at`, as in the editor, so the metadata
        // file changes too whenever the clock ticked over a second.
        let (metadata, sources): (Vec<_>, Vec<_>) = changed
            .iter()
            .partition(|file| file.path == "doc/metadata.json");
        assert!(
            metadata
                .iter()
                .all(|file| file.status == FileStatus::Modified)
        );
        assert_eq!(sources.len(), 2, "{changed:?}");
        assert!(sources.iter().all(|file| {
            file.status == FileStatus::Modified
                && file.path.starts_with("pages/")
                && file.path.ends_with("/page.fnx")
                && file.sha256.as_ref().is_some_and(|hash| hash.len() == 64)
        }));

        let reopened = HeadlessProject::open(fixture.directory.path()).expect("reopen");
        let home = comments::read_comments(reopened.doc(), fixture.home);
        assert_eq!(home.len(), 1);
        assert_eq!(home[0].author, "Fanta Review");
        assert_eq!(home[0].anchor.map(|anchor| anchor.node), Some(fixture.card));
        assert_eq!(
            comments::read_comments(reopened.doc(), fixture.about).len(),
            1
        );
        assert!(
            changeset(reopened.loaded_hashes(), reopened.root())
                .expect("changeset after reopen")
                .is_empty(),
            "opening a saved project changes nothing"
        );
    }

    #[test]
    fn renders_show_the_page_or_one_node_within_the_size_cap() {
        let fixture = write_project();
        let mut project = HeadlessProject::open(fixture.directory.path()).expect("open");
        let page = project
            .render(&RenderTarget {
                id: "home".into(),
                page: PageRef::Index { index: 0 },
                node: None,
                max_dimension: 300,
                motion_clip: None,
                playhead_ms: None,
            })
            .expect("render the page");
        assert_eq!(page.target_id, "home");
        assert!(page.png.starts_with(b"\x89PNG"));
        assert_eq!(
            page.width.max(page.height),
            240,
            "small pages upscale at most 2x"
        );
        assert_eq!(page.bounds, [40.0, 30.0, 120.0, 80.0]);

        let node = project
            .render(&RenderTarget {
                id: "card".into(),
                page: PageRef::Id {
                    id: fixture.home.to_string(),
                },
                node: Some(fixture.card.to_string()),
                max_dimension: 60,
                motion_clip: None,
                playhead_ms: None,
            })
            .expect("render one node");
        assert_eq!(node.width, 60);
        assert_eq!(node.height, 40);

        let wrong_page = project.render(&RenderTarget {
            id: "card".into(),
            page: PageRef::Index { index: 1 },
            node: Some(fixture.card.to_string()),
            max_dimension: 60,
            motion_clip: None,
            playhead_ms: None,
        });
        assert!(wrong_page.is_err());
    }

    #[test]
    fn created_pages_resolve_by_creation_order_and_save_as_new_sources() {
        let fixture = write_project();
        let mut project = HeadlessProject::open(fixture.directory.path()).expect("open");
        let review = project.create_page("Review").expect("create a page");
        assert_eq!(
            project
                .resolve_page(&PageRef::Created { created: 0 })
                .expect("the created page resolves"),
            review
        );
        assert!(
            project
                .resolve_page(&PageRef::Created { created: 1 })
                .is_err()
        );
        assert_eq!(project.doc().active_page(), Some(review));
        let results = project
            .add_comments("Notes", &[finding(2, None, "On the new page")])
            .expect("comment on the new page");
        assert!(results[0].is_ok());

        project.save().expect("save");
        let changed = changeset(project.loaded_hashes(), project.root()).expect("changeset");
        assert!(
            changed
                .iter()
                .any(|file| file.status == FileStatus::Added && file.path.ends_with("/page.fnx")),
            "{changed:?}"
        );
        let reopened = HeadlessProject::open(fixture.directory.path()).expect("reopen");
        let summary = reopened.inspect().expect("inspect");
        assert_eq!(summary["pages"][2]["name"], "Review");
        assert_eq!(comments::read_comments(reopened.doc(), review).len(), 1);
    }

    #[test]
    fn changesets_report_added_modified_and_deleted_files_and_skip_hidden_ones() {
        let directory = tempfile::tempdir().expect("directory");
        let root = directory.path();
        std::fs::create_dir_all(root.join("pages").join("home")).expect("pages");
        std::fs::write(root.join("fanta.json"), "{}").expect("manifest");
        std::fs::write(root.join("pages").join("home").join("page.fnx"), "<Page/>").expect("page");
        std::fs::write(root.join("old.txt"), "gone soon").expect("old");
        let loaded = hash_project_files(root).expect("hash");
        assert_eq!(loaded.len(), 3);

        std::fs::write(root.join("pages").join("home").join("page.fnx"), "<Page />").expect("edit");
        std::fs::remove_file(root.join("old.txt")).expect("delete");
        std::fs::write(root.join("new.txt"), "fresh").expect("add");
        std::fs::create_dir_all(root.join(".git")).expect("git");
        std::fs::write(root.join(".git").join("HEAD"), "ref").expect("hidden");

        let changed = changeset(&loaded, root).expect("changeset");
        assert_eq!(
            changed
                .iter()
                .map(|file| (file.path.as_str(), file.status))
                .collect::<Vec<_>>(),
            [
                ("new.txt", FileStatus::Added),
                ("old.txt", FileStatus::Deleted),
                ("pages/home/page.fnx", FileStatus::Modified),
            ]
        );
        assert_eq!(changed[1].sha256, None);
        assert_eq!(
            changed[0].sha256.as_deref(),
            Some(hex_digest(&Sha256::digest(b"fresh").into()).as_str())
        );
    }

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
