//! The document layer of the Figma viewer: loading `.fig` files and Fanta
//! projects into a [`Doc`], tracking edits, and persisting them back to disk
//! as an unwrapped `fanta-project` directory.

use std::{
    collections::{BTreeMap, HashMap, HashSet},
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::{Context as _, Result};
use fanta_doc::{AssetId, Doc, NodeId, Operation, Viewport};
use fanta_fig_interop::{fig_to_doc, read_fig};
use fanta_render::{AssetResolver, DecodedImage, InMemoryAssetResolver, solve_scene_layout};
use gpui::{
    App, AppContext as _, Context, Entity, EventEmitter, Image, ImageFormat, SharedString,
    Subscription, Task,
};
use project::{Project, ProjectPath};
use worktree::{PathChange, ProjectEntryId, UpdatedEntriesSet, WorktreeId};

/// How long to let a burst of external writes (an agent rewriting several
/// project files) settle before reloading from disk.
const RELOAD_DEBOUNCE: Duration = Duration::from_millis(300);

/// How long after our own project writes to keep ignoring watcher events:
/// long enough to cover the file-system watcher's delivery latency, short
/// enough that a genuinely external edit right after a save is only briefly
/// missed (the next edit's events will still arrive).
const SELF_WRITE_SUPPRESS_WINDOW: Duration = Duration::from_secs(1);

pub struct FigItem {
    pub(crate) path: ProjectPath,
    pub(crate) abs_path: PathBuf,
    pub(crate) entry_id: Option<ProjectEntryId>,
    pub(crate) document: FigDocumentState,
    project_root: Option<PathBuf>,
    dirty: bool,
    /// The project changed on disk while the canvas had unsaved edits.
    conflict: bool,
    /// Ignore worktree events until this instant; set around our own project
    /// writes so saving from the canvas does not trigger a self-reload.
    suppress_watcher_until: Option<Instant>,
    reload_task: Option<Task<()>>,
    _load_task: Option<Task<()>>,
    _project_subscription: Subscription,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FigItemEvent {
    /// The document content changed through an operation, undo, or redo.
    Edited,
    /// A drag preview frame changed document content transiently. Fires per
    /// pointer move, so listeners doing non-trivial work per change (like the
    /// layers tree) should ignore it and wait for the committing [`Edited`].
    EditedTransient,
    /// The selection or another non-persistent view state changed.
    SelectionChanged,
    /// The document finished (re)loading or was saved.
    StateChanged,
    /// The project diverged from disk while the canvas had unsaved edits, or
    /// that conflict was resolved by saving or reloading.
    ConflictChanged,
}

impl EventEmitter<FigItemEvent> for FigItem {}

pub(crate) enum FigDocumentState {
    Loading { message: SharedString },
    Ready(FigDocument),
    Error(Arc<anyhow::Error>),
}

impl FigDocumentState {
    fn from_result(result: Result<FigDocument>) -> Self {
        match result {
            Ok(document) => Self::Ready(document),
            Err(error) => Self::Error(Arc::new(error)),
        }
    }

    pub(crate) fn ready(&self) -> Option<&FigDocument> {
        match self {
            Self::Ready(document) => Some(document),
            Self::Loading { .. } | Self::Error(_) => None,
        }
    }

    pub(crate) fn ready_mut(&mut self) -> Option<&mut FigDocument> {
        match self {
            Self::Ready(document) => Some(document),
            Self::Loading { .. } | Self::Error(_) => None,
        }
    }

    pub(crate) fn loading_message(&self) -> Option<SharedString> {
        match self {
            Self::Loading { message } => Some(message.clone()),
            Self::Ready(_) | Self::Error(_) => None,
        }
    }

    pub(crate) fn error(&self) -> Option<Arc<anyhow::Error>> {
        match self {
            Self::Error(error) => Some(error.clone()),
            Self::Loading { .. } | Self::Ready(_) => None,
        }
    }
}

pub struct FigDocument {
    pub doc: Doc,
    pub pages: Vec<FigPage>,
    pub default_page_index: usize,
    pub(crate) solved_pages: HashSet<NodeId>,
    pub asset_resolver: Option<Arc<dyn AssetResolver>>,
    /// Original encoded asset bytes, kept for writing the project tree.
    pub raw_assets: Arc<BTreeMap<AssetId, Vec<u8>>>,
    /// GPUI-renderable thumbnails for every embedded asset GPUI can decode,
    /// built once on the background load thread. Each wraps the asset's ENCODED
    /// bytes behind the content hash GPUI caches its decoded texture by, so the
    /// Assets panel clones the shared `Arc` per row instead of re-hashing every
    /// asset byte on the foreground. Assets in a format GPUI has no decoder for
    /// are absent (their row falls back to a generic glyph).
    pub gpui_images: HashMap<AssetId, Arc<Image>>,
    /// Whether any node in the scene uses auto layout. Computed once at load;
    /// documents without it skip the whole-page layout re-solve after every
    /// edit, which includes text measurement and is far too slow to run per
    /// interaction on large pages.
    uses_auto_layout: bool,
}

pub struct FigPage {
    pub root: Option<NodeId>,
    pub name: SharedString,
    pub bounds: fanta_doc::Bounds,
    /// A page the design surface can navigate to but the Pages panel hides —
    /// the importer's internal library pages (e.g. the Components page holding
    /// every component master). Kept in `pages` so a component can be focused by
    /// switching to it, yet filtered out of the user-facing page list.
    pub hidden: bool,
}

impl FigDocument {
    fn from_doc(mut doc: Doc, raw_assets: BTreeMap<AssetId, Vec<u8>>) -> Self {
        // Figma bakes each instance's fully-resolved paints as sparse overrides;
        // the ones that merely restate the master pin the instance and block
        // master edits from propagating. Drop them on load (both `.fig` imports
        // and already-materialized projects) so editing a component master
        // reaches its unmodified instances.
        fanta_doc::strip_redundant_instance_overrides(&mut doc.scene, &doc.components);
        // Legacy migration: projects saved before vectors carried an SVG viewport
        // get one inferred from geometry, so a stroke thickened past the box is
        // clipped like a freshly imported file. A no-op once the doc is viewport-aware.
        fanta_doc::backfill_vector_viewports(&mut doc.scene);
        let visible_page_roots = visible_page_roots(&doc);
        let default_page_root = default_page_root(&doc, &visible_page_roots);
        let uses_auto_layout = scene_uses_auto_layout(&doc.scene);
        let mut solved_pages = HashSet::new();
        if let Some(page_root) = default_page_root {
            solve_scene_layout(&mut doc.scene, page_root);
            solved_pages.insert(page_root);
        }
        let resolver = decode_assets(&raw_assets);
        let asset_resolver =
            (!resolver.is_empty()).then(|| Arc::new(resolver) as Arc<dyn AssetResolver>);
        let gpui_images = decode_gpui_images(&raw_assets);
        let pages = collect_pages(&doc, &visible_page_roots);
        let default_page_index = default_page_root
            .and_then(|root| pages.iter().position(|page| page.root == Some(root)))
            .or_else(|| pages.iter().position(|page| !page.hidden))
            .unwrap_or(0);

        Self {
            doc,
            pages,
            default_page_index,
            solved_pages,
            asset_resolver,
            raw_assets: Arc::new(raw_assets),
            gpui_images,
            uses_auto_layout,
        }
    }

    pub fn page(&self, selected_page_index: Option<usize>) -> Option<&FigPage> {
        let index = selected_page_index
            .filter(|index| *index < self.pages.len())
            .unwrap_or(
                self.default_page_index
                    .min(self.pages.len().saturating_sub(1)),
            );
        self.pages.get(index)
    }

    pub fn page_index(&self, selected_page_index: Option<usize>) -> Option<usize> {
        if self.pages.is_empty() {
            return None;
        }
        Some(
            selected_page_index
                .filter(|index| *index < self.pages.len())
                .unwrap_or(self.default_page_index.min(self.pages.len() - 1)),
        )
    }

    /// The index into [`pages`](Self::pages) of the page that contains `node`
    /// (walking up to its top-level root), including hidden pages. Used to
    /// navigate to a component master, which lives on the hidden Components
    /// page.
    pub fn page_index_of_node(&self, node: NodeId) -> Option<usize> {
        let mut current = node;
        loop {
            let parent = self.doc.scene.get(current)?.parent;
            match parent {
                Some(parent) => current = parent,
                None => break,
            }
        }
        self.pages
            .iter()
            .position(|page| page.root == Some(current))
    }

    pub fn ensure_page_solved(&mut self, page_index: usize) {
        let Some(page_root) = self.pages.get(page_index).and_then(|page| page.root) else {
            return;
        };
        if self.solved_pages.insert(page_root) {
            solve_scene_layout(&mut self.doc.scene, page_root);
            self.refresh_page_bounds(page_index);
        }
    }

    fn refresh_page_bounds(&mut self, page_index: usize) {
        let Some(page) = self.pages.get(page_index) else {
            return;
        };
        let bounds = page_bounds(&self.doc, page.root);
        if let Some(page) = self.pages.get_mut(page_index) {
            page.bounds = bounds;
        }
    }

    /// Re-solve layout for a page after an edit. Skipped entirely for
    /// documents without auto layout: nothing there depends on the solver,
    /// and running it (with its text measurement) after every interaction
    /// dominates the frame budget on large pages.
    fn resolve_after_edit(&mut self, page_root: Option<NodeId>) {
        if !self.uses_auto_layout {
            return;
        }
        let Some(page_root) = page_root else {
            return;
        };
        solve_scene_layout(&mut self.doc.scene, page_root);
        self.solved_pages.insert(page_root);
        if let Some(index) = self
            .pages
            .iter()
            .position(|page| page.root == Some(page_root))
        {
            self.refresh_page_bounds(index);
        }
    }
}

impl project::ProjectItem for FigItem {
    fn try_open(
        project: &Entity<Project>,
        path: &ProjectPath,
        cx: &mut App,
    ) -> Option<Task<Result<Entity<Self>>>> {
        let abs_path = project.read(cx).absolute_path(path, cx);
        let is_fig = is_fig_file(path) || abs_path.as_deref().is_some_and(path_has_fig_extension);
        let is_manifest = is_fanta_manifest(path.path.as_std_path())
            || abs_path.as_deref().is_some_and(is_fanta_manifest);
        if !is_fig && !is_manifest {
            return None;
        }

        let path = path.clone();
        let entry_id = project
            .read(cx)
            .entry_for_path(&path, cx)
            .map(|entry| entry.id);
        let project = project.clone();

        Some(cx.spawn(async move |cx| {
            let abs_path =
                abs_path.context("Figma viewer only supports local .fig files and projects")?;
            let project_root = if is_manifest {
                abs_path.parent().map(Path::to_path_buf)
            } else {
                // A `.fig` that was already materialized into a sibling Fanta
                // project must open — and save into — that project, not a stale
                // re-parse of the `.fig`. Reusing `available_project_dir` makes
                // the redirect target the exact directory a save would write, so
                // reopening `Design.fig` loads the up-to-date `Design/` (and
                // activates conflict detection) instead of clobbering newer
                // on-disk edits with the stale import on the next save.
                let candidate = available_project_dir(&abs_path);
                fanta_format::is_project_dir(&candidate).then_some(candidate)
            };
            let item = cx.new(|cx| {
                let load_path = abs_path.clone();
                let load_project_root = project_root.clone();
                let load_task = cx.spawn(async move |this, cx| {
                    let set_loading = |message: &'static str| {
                        let message = SharedString::from(message);
                        move |this: &mut FigItem, cx: &mut Context<FigItem>| {
                            this.document = FigDocumentState::Loading { message };
                            cx.notify();
                        }
                    };
                    if let Err(error) = this.update(cx, set_loading("Reading document...")) {
                        log::debug!("dropping load update for closed .fig item: {error:#}");
                        return;
                    }

                    let document = cx
                        .background_spawn(async move {
                            match load_project_root {
                                Some(root) => load_project_document(&root),
                                None => load_fig_document(&load_path),
                            }
                        })
                        .await;

                    if let Err(error) = this.update(cx, |this: &mut FigItem, cx| {
                        this.document = FigDocumentState::from_result(document);
                        cx.emit(FigItemEvent::StateChanged);
                        cx.notify();
                    }) {
                        log::debug!("dropping loaded update for closed .fig item: {error:#}");
                    }
                });

                let project_subscription =
                    cx.subscribe(&project, |this: &mut Self, project, event, cx| {
                        if let project::Event::WorktreeUpdatedEntries(worktree_id, changes) = event
                        {
                            this.worktree_entries_updated(&project, *worktree_id, changes, cx);
                        }
                    });

                Self {
                    path,
                    abs_path,
                    entry_id,
                    document: FigDocumentState::Loading {
                        message: "Opening document...".into(),
                    },
                    project_root,
                    dirty: false,
                    conflict: false,
                    suppress_watcher_until: None,
                    reload_task: None,
                    _load_task: Some(load_task),
                    _project_subscription: project_subscription,
                }
            });
            Ok(item)
        }))
    }

    fn entry_id(&self, _: &App) -> Option<ProjectEntryId> {
        self.entry_id
    }

    fn project_path(&self, _: &App) -> Option<ProjectPath> {
        Some(self.path.clone())
    }

    fn is_dirty(&self) -> bool {
        self.dirty
    }
}

impl FigItem {
    pub fn doc(&self) -> Option<&Doc> {
        self.document.ready().map(|document| &document.doc)
    }

    pub fn document(&self) -> Option<&FigDocument> {
        self.document.ready()
    }

    pub fn abs_path(&self) -> &Path {
        &self.abs_path
    }

    /// A document is editable as soon as it has parsed, whether or not it has
    /// been materialized to an on-disk Fanta project yet. A freshly opened
    /// `.fig` edits in memory from the first parse; the first save writes the
    /// project directory (see [`FigItem::save`]).
    pub fn is_editable(&self) -> bool {
        self.document.ready().is_some()
    }

    pub fn project_root(&self) -> Option<&Path> {
        self.project_root.as_deref()
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    pub fn has_conflict(&self) -> bool {
        self.conflict
    }

    fn set_conflict(&mut self, conflict: bool, cx: &mut Context<Self>) {
        if self.conflict != conflict {
            self.conflict = conflict;
            cx.emit(FigItemEvent::ConflictChanged);
            cx.notify();
        }
    }

    /// React to worktree file events, refreshing the canvas when something
    /// else (typically the AI agent editing `.fnx` sources as text) changes
    /// the project on disk. Only works while the project directory lives
    /// inside an open worktree; a project outside the workspace gets no
    /// events and therefore no auto-reload.
    fn worktree_entries_updated(
        &mut self,
        project: &Entity<Project>,
        worktree_id: WorktreeId,
        changes: &UpdatedEntriesSet,
        cx: &mut Context<Self>,
    ) {
        let Some(project_root) = self.project_root.clone() else {
            return;
        };
        if self
            .suppress_watcher_until
            .is_some_and(|until| Instant::now() < until)
        {
            return;
        }
        let Some(worktree) = project.read(cx).worktree_for_id(worktree_id, cx) else {
            return;
        };
        let worktree = worktree.read(cx);
        let relevant = changes.iter().any(|(path, _, change)| {
            // `Loaded` entries come from the initial worktree scan, not from
            // anything changing on disk.
            !matches!(change, PathChange::Loaded)
                && is_relevant_project_change(&project_root, &worktree.absolutize(path))
        });
        if !relevant {
            return;
        }
        if self.dirty {
            // Unsaved canvas edits win over disk; surface the divergence as
            // a conflict instead of clobbering them.
            self.set_conflict(true, cx);
        } else {
            self.schedule_reload(cx);
        }
    }

    /// Debounce disk changes, then reload the project in the background.
    /// Content gestures dirty the item on their first preview frame, so a
    /// mid-gesture reload can only interrupt selection-style gestures, which
    /// tolerate having their selection reset.
    fn schedule_reload(&mut self, cx: &mut Context<Self>) {
        let Some(root) = self.project_root.clone() else {
            return;
        };
        self.reload_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(RELOAD_DEBOUNCE).await;
            let loaded = cx
                .background_spawn(async move { load_project_document(&root) })
                .await;
            if let Err(error) = this.update(cx, |this, cx| {
                if this.dirty {
                    // Canvas edits landed while the reload was in flight;
                    // keep them and flag the divergence.
                    this.set_conflict(true, cx);
                    return;
                }
                match loaded {
                    Ok(document) => this.apply_reloaded_document(document, cx),
                    Err(error) => {
                        log::error!(
                            "reloading Fanta project after a disk change failed: {error:#}"
                        );
                        // Keep the last good document: the failure may be a
                        // half-written batch of files whose next watcher
                        // event will reload it in full.
                        if this.document.ready().is_none() {
                            this.document = FigDocumentState::Error(Arc::new(error));
                            cx.emit(FigItemEvent::StateChanged);
                            cx.notify();
                        }
                    }
                }
            }) {
                log::debug!("dropping reload for closed Fanta project item: {error:#}");
            }
        }));
    }

    /// Swap in a document reloaded from disk, preserving the active page (by
    /// its root node id) when it still exists. The viewport lives on the
    /// view and survives untouched; the selection resets with the new
    /// document.
    fn apply_reloaded_document(&mut self, mut document: FigDocument, cx: &mut Context<Self>) {
        let previous_page_root = self
            .document
            .ready()
            .and_then(|current| current.doc.active_page());
        if let Some(root) = previous_page_root
            && let Some(index) = document
                .pages
                .iter()
                .position(|page| page.root == Some(root))
        {
            document.ensure_page_solved(index);
            document.doc.set_active_page(Some(root));
            document.default_page_index = index;
        }
        self.document = FigDocumentState::Ready(document);
        self.dirty = false;
        self.set_conflict(false, cx);
        cx.emit(FigItemEvent::StateChanged);
        cx.notify();
    }

    /// Reload the project from disk immediately, discarding unsaved canvas
    /// edits. This backs the workspace's "discard and reload" choice in the
    /// conflict prompt.
    pub fn reload_from_disk(&mut self, cx: &mut Context<Self>) -> Task<Result<()>> {
        let Some(root) = self.project_root.clone() else {
            return Task::ready(Ok(()));
        };
        self.reload_task = None;
        cx.spawn(async move |this, cx| {
            let document = cx
                .background_spawn(async move { load_project_document(&root) })
                .await?;
            this.update(cx, |this, cx| this.apply_reloaded_document(document, cx))?;
            Ok(())
        })
    }

    /// The display name of the document: the project directory name once a
    /// Fanta project exists, the `.fig` file name before that.
    pub fn title(&self) -> SharedString {
        let name = self
            .project_root
            .as_deref()
            .and_then(Path::file_name)
            .or_else(|| self.abs_path.file_name())
            .and_then(|name| name.to_str())
            .unwrap_or("Figma");
        name.to_string().into()
    }

    /// Apply an undoable operation to the document, re-solve the affected
    /// page's layout, and mark the item dirty.
    pub fn apply(&mut self, operation: Operation, cx: &mut Context<Self>) -> Result<()> {
        let document = self
            .document
            .ready_mut()
            .context("the document is still loading")?;
        let target = operation.primary_target();
        document
            .doc
            .apply(operation)
            .context("applying canvas operation")?;
        // `uses_auto_layout` gates the whole-page re-solve and is computed once
        // at load. An edit that introduces the document's FIRST auto layout must
        // flip it on, or that frame would never be solved. Checking only the op's
        // primary target keeps this O(1); the flag is sticky (clearing the last
        // auto layout only costs a redundant solve, never a stale layout).
        if !document.uses_auto_layout
            && let Some(target) = target
            && node_uses_auto_layout(&document.doc.scene, target)
        {
            document.uses_auto_layout = true;
        }
        let active_page = document.doc.active_page();
        document.resolve_after_edit(active_page);
        self.mark_edited(false, cx);
        Ok(())
    }

    /// Give tools and inspectors scoped mutable access to the document. The
    /// caller reports back whether it changed doc content (as opposed to only
    /// selection or viewport), which drives dirty tracking.
    pub fn with_document<R>(
        &mut self,
        cx: &mut Context<Self>,
        f: impl FnOnce(&mut FigDocument) -> (R, DocChange),
    ) -> Option<R> {
        let document = self.document.ready_mut()?;
        let (result, change) = f(document);
        match change {
            DocChange::None => {}
            DocChange::Selection => {
                cx.emit(FigItemEvent::SelectionChanged);
                cx.notify();
            }
            DocChange::Content => {
                // A multi-op transaction (which bypasses `apply`'s per-op gate
                // check) could introduce the document's first auto layout; keep
                // the re-solve gate honest. The scan runs only while the gate is
                // closed, and the flag is sticky, so this is a one-time cost.
                if !document.uses_auto_layout && scene_uses_auto_layout(&document.doc.scene) {
                    document.uses_auto_layout = true;
                }
                let active_page = document.doc.active_page();
                document.resolve_after_edit(active_page);
                self.mark_edited(false, cx);
            }
            DocChange::ContentPreview => {
                self.mark_edited(true, cx);
            }
        }
        Some(result)
    }

    pub fn undo(&mut self, cx: &mut Context<Self>) -> Result<bool> {
        let document = self
            .document
            .ready_mut()
            .context("the document is still loading")?;
        let did = document.doc.undo().context("undoing canvas operation")?;
        if did {
            let active_page = document.doc.active_page();
            document.resolve_after_edit(active_page);
            self.mark_edited(false, cx);
        }
        Ok(did)
    }

    pub fn redo(&mut self, cx: &mut Context<Self>) -> Result<bool> {
        let document = self
            .document
            .ready_mut()
            .context("the document is still loading")?;
        let did = document.doc.redo().context("redoing canvas operation")?;
        if did {
            let active_page = document.doc.active_page();
            document.resolve_after_edit(active_page);
            self.mark_edited(false, cx);
        }
        Ok(did)
    }

    fn mark_edited(&mut self, transient: bool, cx: &mut Context<Self>) {
        let was_dirty = self.dirty;
        if self.is_editable() {
            self.dirty = true;
        }
        // Preview frames fire per pointer move; once the dirty transition has
        // been announced, heavyweight listeners can ignore the rest.
        if transient && self.dirty == was_dirty {
            cx.emit(FigItemEvent::EditedTransient);
        } else {
            cx.emit(FigItemEvent::Edited);
        }
        cx.notify();
    }

    /// Persist the current document state, materializing an on-disk Fanta
    /// project the first time a lone `.fig` is saved.
    ///
    /// A `.fig` opened straight from disk has no `project_root` and edits live
    /// only in memory. The first save scaffolds a project directory next to it
    /// (`Design.fig` → `Design/`, with collision handling) and adopts it as the
    /// editable source of truth; every later save overwrites that same tree.
    /// The source `.fig` is never rewritten.
    ///
    /// Returns the newly materialized project directory on the save that
    /// creates it — so the view can add it to the workspace as a visible
    /// worktree — and `None` when the project already existed.
    pub fn save(&mut self, cx: &mut Context<Self>) -> Task<Result<Option<PathBuf>>> {
        let Some(document) = self.document.ready() else {
            return Task::ready(Err(anyhow::anyhow!("the document is still loading")));
        };

        let doc = document.doc.clone();
        let raw_assets = document.raw_assets.clone();
        let existing_root = self.project_root.clone();
        let materializing = existing_root.is_none();
        // Materialize next to the `.fig` on the first save; reuse the existing
        // project directory on every save after that.
        let target = existing_root.unwrap_or_else(|| available_project_dir(&self.abs_path));
        if materializing && fanta_format::is_project_dir(&target) {
            // A Fanta project already exists at the destination that this
            // in-memory `.fig` was never loaded from (it appeared after we
            // opened). Materializing would full-overwrite newer on-disk content
            // with a stale import — refuse rather than destroy it. Reopening the
            // `.fig` will redirect to and load that project (see `try_open`).
            return Task::ready(Err(anyhow::anyhow!(
                "a Fanta project already exists at {}; open it directly instead of overwriting it with {}",
                target.display(),
                self.abs_path.display()
            )));
        }
        // Our own writes echo back through the worktree watcher; suppress it
        // both from save start (covers sub-second saves entirely) and again at
        // completion (covers watcher latency after longer saves). On the
        // materializing save this window also spans the moment the directory is
        // adopted as a worktree, so its initial scan does not bounce back as a
        // reload.
        self.suppress_watcher_until = Some(Instant::now() + SELF_WRITE_SUPPRESS_WINDOW);
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn({
                    let target = target.clone();
                    async move { write_project(&target, &doc, &raw_assets) }
                })
                .await;
            this.update(cx, |this, cx| {
                this.suppress_watcher_until = Some(Instant::now() + SELF_WRITE_SUPPRESS_WINDOW);
                if result.is_ok() {
                    if materializing {
                        this.project_root = Some(target.clone());
                    }
                    this.dirty = false;
                    this.set_conflict(false, cx);
                    cx.emit(FigItemEvent::StateChanged);
                    cx.notify();
                }
            })?;
            result.map(|()| materializing.then_some(target))
        })
    }
}

/// How a scoped document mutation affects persistence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocChange {
    /// Nothing observable changed.
    None,
    /// Only the selection changed; the document is not dirtied.
    Selection,
    /// Document content changed; the item becomes dirty.
    Content,
    /// Document content changed transiently mid-gesture (e.g. a drag preview
    /// frame): the item is dirtied but layout re-solving is deferred to the
    /// gesture's committing event.
    ContentPreview,
}

fn write_project(root: &Path, doc: &Doc, raw_assets: &BTreeMap<AssetId, Vec<u8>>) -> Result<()> {
    fanta_format::scaffold_project_tree(root)
        .with_context(|| format!("scaffolding Fanta project at {}", root.display()))?;
    fanta_format::write_project_tree(root, doc, raw_assets)
        .with_context(|| format!("writing Fanta project at {}", root.display()))?;
    Ok(())
}

/// Pick a directory for a new project next to the source `.fig` file:
/// `Design.fig` becomes `Design/`, falling back to `Design-2/`, `Design-3/`, …
/// when the name is taken by something that is not already a Fanta project.
fn available_project_dir(fig_path: &Path) -> PathBuf {
    let base = fig_path.with_extension("");
    if !base.exists() || fanta_format::is_project_dir(&base) {
        return base;
    }
    let name = base
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("fanta-project")
        .to_string();
    for suffix in 2.. {
        let candidate = base.with_file_name(format!("{name}-{suffix}"));
        if !candidate.exists() || fanta_format::is_project_dir(&candidate) {
            return candidate;
        }
    }
    base
}

/// Whether an externally changed path should refresh the canvas: any file
/// under the project root except the generated `previews/` and `exports/`
/// outputs (which never feed back into the document) and the root `AGENTS.md`
/// agent guide (a doc for the agent, not part of the design — editing it must
/// not reload/replace the canvas).
fn is_relevant_project_change(project_root: &Path, abs_path: &Path) -> bool {
    let Ok(relative) = abs_path.strip_prefix(project_root) else {
        return false;
    };
    let mut components = relative.components();
    // An entry for the root directory itself (e.g. an mtime bump) carries no
    // content change.
    let Some(first_component) = components.next() else {
        return false;
    };
    let first = first_component.as_os_str().to_str();
    if matches!(first, Some("previews" | "exports")) {
        return false;
    }
    // The agent guide lives at the project root (a single path component); a
    // nested file that merely happens to be named AGENTS.md is still design.
    if first == Some("AGENTS.md") && components.next().is_none() {
        return false;
    }
    true
}

pub(crate) fn is_fig_file(path: &ProjectPath) -> bool {
    path_has_fig_extension(path.path.as_std_path())
}

fn path_has_fig_extension(path: &Path) -> bool {
    path.extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("fig"))
}

fn is_fanta_manifest(path: &Path) -> bool {
    path.file_name()
        .is_some_and(|name| name.eq_ignore_ascii_case("fanta.json"))
}

fn load_fig_document(path: &Path) -> Result<FigDocument> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let fig = read_fig(&bytes).context("parsing .fig")?;
    let (doc, _report, assets) = fig_to_doc(&fig).context("mapping .fig to Fanta document")?;
    Ok(FigDocument::from_doc(doc, assets.into_iter().collect()))
}

fn load_project_document(root: &Path) -> Result<FigDocument> {
    let (doc, assets) = fanta_format::read_project_tree(root)
        .with_context(|| format!("reading Fanta project at {}", root.display()))?;
    Ok(FigDocument::from_doc(doc, assets))
}

/// Whether this single node carries an auto layout. O(1) — the incremental
/// check [`FigItem::apply`] runs against an op's primary target.
fn node_uses_auto_layout(scene: &fanta_doc::Scene, id: NodeId) -> bool {
    scene.get(id).is_some_and(|node| match &node.data {
        fanta_doc::NodeData::Group(group) => group.auto_layout.is_some(),
        _ => false,
    })
}

fn scene_uses_auto_layout(scene: &fanta_doc::Scene) -> bool {
    let roots: Vec<NodeId> = scene.roots().to_vec();
    roots.into_iter().any(|root| {
        scene
            .descendants_of(root)
            .any(|id| node_uses_auto_layout(scene, id))
    })
}

fn visible_page_roots(doc: &Doc) -> Vec<NodeId> {
    doc.pages()
        .iter()
        .copied()
        .filter(|page| {
            doc.scene
                .get(*page)
                .and_then(|node| node.meta.get("hidden_page"))
                .and_then(|value| value.as_bool())
                != Some(true)
        })
        .collect()
}

fn default_page_root(doc: &Doc, visible_page_roots: &[NodeId]) -> Option<NodeId> {
    if visible_page_roots.is_empty() {
        return None;
    }

    let active_page = doc
        .active_page()
        .filter(|page| visible_page_roots.contains(page));
    if let Some(active_page) = active_page
        && page_content_area(doc, Some(active_page)) > 1.0
    {
        return Some(active_page);
    }

    visible_page_roots.iter().copied().max_by(|left, right| {
        page_content_area(doc, Some(*left)).total_cmp(&page_content_area(doc, Some(*right)))
    })
}

fn collect_pages(doc: &Doc, visible_page_roots: &[NodeId]) -> Vec<FigPage> {
    let all_roots = doc.pages();
    if all_roots.is_empty() {
        return vec![FigPage {
            root: None,
            name: "Document".into(),
            bounds: page_bounds(doc, None),
            hidden: false,
        }];
    }

    // Every page is included so the canvas can navigate to a hidden library
    // page (to focus a component master); `hidden` marks the ones the Pages
    // panel filters out. `index + 1` numbers unnamed pages by their absolute
    // position, which stays stable as pages are added/removed.
    let visible: HashSet<NodeId> = visible_page_roots.iter().copied().collect();
    all_roots
        .iter()
        .enumerate()
        .map(|(index, root)| FigPage {
            root: Some(*root),
            name: doc
                .page_name(*root)
                .filter(|name| !name.is_empty())
                .map(SharedString::from)
                .unwrap_or_else(|| format!("Page {}", index + 1).into()),
            bounds: page_bounds(doc, Some(*root)),
            hidden: !visible.contains(root),
        })
        .collect()
}

fn page_content_area(doc: &Doc, page_root: Option<NodeId>) -> f64 {
    try_page_bounds(doc, page_root)
        .map(|bounds| bounds.width() * bounds.height())
        .unwrap_or(0.0)
}

pub(crate) fn page_bounds(doc: &Doc, page_root: Option<NodeId>) -> fanta_doc::Bounds {
    try_page_bounds(doc, page_root)
        .unwrap_or_else(|| fanta_doc::Bounds::from_xywh(0.0, 0.0, 1024.0, 768.0))
}

fn try_page_bounds(doc: &Doc, page_root: Option<NodeId>) -> Option<fanta_doc::Bounds> {
    let mut bounds: Option<fanta_doc::Bounds> = None;
    let mut include = |node_id| {
        if let Some(node_bounds) = doc.scene.world_bounds(node_id)
            && node_bounds.is_finite()
            && node_bounds.width() > 0.0
            && node_bounds.height() > 0.0
        {
            bounds = Some(match bounds {
                Some(bounds) => bounds.union(&node_bounds),
                None => node_bounds,
            });
        }
    };

    if let Some(page_root) = page_root {
        for node_id in doc.scene.descendants_of(page_root) {
            if node_id != page_root {
                include(node_id);
            }
        }
    } else {
        for root in doc.scene.roots() {
            for node_id in doc.scene.descendants_of(*root) {
                include(node_id);
            }
        }
    }

    bounds
}

fn decode_assets(assets: &BTreeMap<AssetId, Vec<u8>>) -> InMemoryAssetResolver {
    let mut resolver = InMemoryAssetResolver::new();
    for (asset_id, bytes) in assets {
        match image::load_from_memory(bytes) {
            Ok(image) => {
                let image = image.to_rgba8();
                let (width, height) = image.dimensions();
                resolver.insert(
                    *asset_id,
                    DecodedImage::new(Arc::new(image.into_raw()), width, height),
                );
            }
            Err(error) => {
                log::warn!("failed to decode embedded .fig image asset {asset_id}: {error}");
            }
        }
    }
    resolver
}

/// Wrap every embedded asset GPUI can decode as an [`Image`] behind its content
/// hash, skipping formats GPUI has no decoder for. Runs on the background load
/// thread so `Image::from_bytes`'s hash of every asset byte never lands on the
/// foreground; the Assets panel then clones these `Arc`s per thumbnail row.
fn decode_gpui_images(assets: &BTreeMap<AssetId, Vec<u8>>) -> HashMap<AssetId, Arc<Image>> {
    assets
        .iter()
        .filter_map(|(asset_id, bytes)| {
            let format = gpui_image_format(image::guess_format(bytes).ok()?)?;
            // The ENCODED bytes go to GPUI (not `decode_assets`' straight-alpha
            // RGBA8, which the canvas renderer wants): handing over the source
            // bytes lets GPUI decode, swap channels to BGRA, and cache the
            // texture behind the content hash `Image::from_bytes` computes.
            Some((
                *asset_id,
                Arc::new(Image::from_bytes(format, bytes.clone())),
            ))
        })
        .collect()
}

/// GPUI's decoder for `format`, or `None` when it has none. Unlike a paste from
/// the clipboard, a `.fig` can legitimately embed a format GPUI cannot draw, so
/// this returns `None` rather than treating it as a bug.
fn gpui_image_format(format: image::ImageFormat) -> Option<ImageFormat> {
    match format {
        image::ImageFormat::Png => Some(ImageFormat::Png),
        image::ImageFormat::Jpeg => Some(ImageFormat::Jpeg),
        image::ImageFormat::WebP => Some(ImageFormat::Webp),
        image::ImageFormat::Gif => Some(ImageFormat::Gif),
        image::ImageFormat::Bmp => Some(ImageFormat::Bmp),
        image::ImageFormat::Tiff => Some(ImageFormat::Tiff),
        image::ImageFormat::Ico => Some(ImageFormat::Ico),
        image::ImageFormat::Pnm => Some(ImageFormat::Pnm),
        _ => None,
    }
}

/// A viewport fitted around `bounds` with `padding` logical pixels of margin.
pub(crate) fn fit_bounds(
    bounds: fanta_doc::Bounds,
    screen_size: (f64, f64),
    padding: f64,
    min_zoom: f32,
    max_zoom: f32,
) -> Viewport {
    let usable_width = (screen_size.0 - padding * 2.0).max(1.0);
    let usable_height = (screen_size.1 - padding * 2.0).max(1.0);
    let bounds_width = bounds.width().max(1.0);
    let bounds_height = bounds.height().max(1.0);
    let zoom = (usable_width / bounds_width)
        .min(usable_height / bounds_height)
        .clamp(f64::from(min_zoom), f64::from(max_zoom));
    Viewport {
        center: [
            bounds.min_x + bounds.width() * 0.5,
            bounds.min_y + bounds.height() * 0.5,
        ],
        zoom,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_bounds_centers_and_fits_the_larger_axis() {
        let bounds = fanta_doc::Bounds::from_xywh(100.0, 200.0, 400.0, 100.0);
        let viewport = fit_bounds(bounds, (848.0, 696.0), 24.0, 0.1, 20.0);
        assert!((viewport.center[0] - 300.0).abs() < 1e-9);
        assert!((viewport.center[1] - 250.0).abs() < 1e-9);
        // Width is the constraining axis: (848 - 48) / 400 = 2.0.
        assert!((viewport.zoom - 2.0).abs() < 1e-9);
    }

    #[test]
    fn encoded_formats_map_to_gpui_decoders() {
        assert_eq!(
            gpui_image_format(image::ImageFormat::Png),
            Some(ImageFormat::Png)
        );
        assert_eq!(
            gpui_image_format(image::ImageFormat::Jpeg),
            Some(ImageFormat::Jpeg)
        );
        assert_eq!(
            gpui_image_format(image::ImageFormat::WebP),
            Some(ImageFormat::Webp)
        );
        // GPUI cannot decode this one, so the panel keeps the generic glyph.
        assert_eq!(gpui_image_format(image::ImageFormat::Avif), None);
    }

    #[test]
    fn decode_gpui_images_wraps_decodable_assets_and_skips_the_rest() {
        let mut png = Vec::new();
        image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            1,
            1,
            image::Rgba([1, 2, 3, 4]),
        ))
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .expect("encoding a 1x1 png");
        let png_id = AssetId::new();
        let junk_id = AssetId::new();
        let mut assets = BTreeMap::new();
        assets.insert(png_id, png.clone());
        assets.insert(junk_id, b"not an image at all".to_vec());

        let images = decode_gpui_images(&assets);

        assert_eq!(images.len(), 1, "only the decodable asset is wrapped");
        let image = images.get(&png_id).expect("the png is wrapped");
        assert_eq!(image.format, ImageFormat::Png);
        assert_eq!(
            image.bytes, png,
            "the ENCODED bytes are handed to GPUI unchanged"
        );
        assert!(
            !images.contains_key(&junk_id),
            "an undecodable blob is left for the generic glyph"
        );
    }

    #[test]
    fn from_doc_precomputes_a_thumbnail_per_decodable_asset() {
        let mut png = Vec::new();
        image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            2,
            2,
            image::Rgba([9, 8, 7, 6]),
        ))
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .expect("encoding a 2x2 png");
        let asset_id = AssetId::new();
        let mut assets = BTreeMap::new();
        assets.insert(asset_id, png);

        let document = FigDocument::from_doc(Doc::new(), assets);
        assert!(
            document.gpui_images.contains_key(&asset_id),
            "from_doc precomputes the GPUI thumbnail during the background load"
        );
    }

    #[test]
    fn project_dir_prefers_the_fig_files_stem() {
        let dir = available_project_dir(Path::new("/tmp/definitely-missing-dir/Design.fig"));
        assert_eq!(dir, Path::new("/tmp/definitely-missing-dir/Design"));
    }

    #[test]
    fn watcher_relevance_covers_sources_but_not_outputs_or_foreign_paths() {
        let root = Path::new("/tmp/project");
        assert!(is_relevant_project_change(
            root,
            Path::new("/tmp/project/pages/page-1.fnx")
        ));
        assert!(is_relevant_project_change(
            root,
            Path::new("/tmp/project/fanta.json")
        ));
        assert!(is_relevant_project_change(
            root,
            Path::new("/tmp/project/assets/logo.png")
        ));
        assert!(!is_relevant_project_change(
            root,
            Path::new("/tmp/project/previews/page-1.png")
        ));
        assert!(!is_relevant_project_change(
            root,
            Path::new("/tmp/project/exports/page-1.svg")
        ));
        // The root AGENTS.md agent guide is documentation, not design: editing
        // it must not reload the canvas.
        assert!(!is_relevant_project_change(
            root,
            Path::new("/tmp/project/AGENTS.md")
        ));
        // A file that merely happens to be named AGENTS.md deeper in the tree
        // is still project content.
        assert!(is_relevant_project_change(
            root,
            Path::new("/tmp/project/pages/AGENTS.md")
        ));
        assert!(!is_relevant_project_change(root, Path::new("/tmp/project")));
        assert!(!is_relevant_project_change(
            root,
            Path::new("/tmp/other/pages/page-1.fnx")
        ));
    }

    #[test]
    fn project_dir_avoids_a_non_project_collision_but_reuses_a_project() {
        let dir = tempfile::tempdir().unwrap();
        let fig = dir.path().join("Design.fig");
        // A plain directory already occupies `Design/`; fall back to a
        // numbered sibling rather than scaffolding on top of it.
        std::fs::create_dir(dir.path().join("Design")).unwrap();
        assert_eq!(available_project_dir(&fig), dir.path().join("Design-2"));

        // Once `Design/` is itself a Fanta project it is reused in place.
        write_project(&dir.path().join("Design"), &Doc::new(), &BTreeMap::new()).unwrap();
        assert_eq!(available_project_dir(&fig), dir.path().join("Design"));
    }

    use gpui::TestAppContext;
    use project::FakeFs;
    use settings::SettingsStore;
    use util::rel_path::RelPath;

    fn init_test(cx: &mut TestAppContext) {
        cx.update(|cx| {
            let settings_store = SettingsStore::test(cx);
            cx.set_global(settings_store);
        });
    }

    async fn empty_project(cx: &mut TestAppContext) -> Entity<Project> {
        init_test(cx);
        let fs = FakeFs::new(cx.executor());
        let roots: [&Path; 0] = [];
        Project::test(fs, roots, cx).await
    }

    /// A document with a single (empty) page, so a save writes a `pages/<id>/`
    /// subtree instead of pruning the empty `pages/` directory.
    fn doc_with_one_page() -> Doc {
        use fanta_doc::{CanvasNode, GroupNode, NodeData};
        let mut doc = Doc::new();
        let mut page = CanvasNode::new(NodeData::Group(GroupNode::default()));
        page.name = "Page 1".to_owned();
        let root = page.id;
        doc.apply(Operation::create_node(page))
            .expect("create page root node");
        doc.add_page(root);
        doc
    }

    /// Build a `FigItem` around an already-parsed document, bypassing the async
    /// load so document-level behavior (editability, save) can be exercised
    /// without a canvas or workspace.
    fn ready_item(
        project: &Entity<Project>,
        abs_path: PathBuf,
        project_root: Option<PathBuf>,
        doc: Doc,
        cx: &mut TestAppContext,
    ) -> Entity<FigItem> {
        cx.new(|cx| {
            let subscription = cx.subscribe(
                project,
                |_: &mut FigItem, _: Entity<Project>, _: &project::Event, _| {},
            );
            FigItem {
                path: ProjectPath {
                    worktree_id: WorktreeId::from_usize(0),
                    path: RelPath::empty_arc(),
                },
                abs_path,
                entry_id: None,
                document: FigDocumentState::Ready(FigDocument::from_doc(doc, BTreeMap::new())),
                project_root,
                dirty: false,
                conflict: false,
                suppress_watcher_until: None,
                reload_task: None,
                _load_task: None,
                _project_subscription: subscription,
            }
        })
    }

    #[gpui::test]
    async fn a_parsed_fig_is_editable_without_a_project_root(cx: &mut TestAppContext) {
        let project = empty_project(cx).await;
        let item = ready_item(
            &project,
            PathBuf::from("/tmp/nowhere/Design.fig"),
            None,
            Doc::new(),
            cx,
        );
        item.read_with(cx, |item, _| {
            assert!(item.project_root().is_none());
            assert!(
                item.is_editable(),
                "a parsed .fig edits in memory immediately"
            );
        });
    }

    #[gpui::test]
    async fn first_save_materializes_the_project_then_reuses_its_root(cx: &mut TestAppContext) {
        let project = empty_project(cx).await;
        let dir = tempfile::tempdir().unwrap();
        let fig_path = dir.path().join("Design.fig");
        let item = ready_item(&project, fig_path, None, doc_with_one_page(), cx);

        item.update(cx, |item, _| item.dirty = true);

        let materialized = item.update(cx, |item, cx| item.save(cx)).await.unwrap();
        let root = dir.path().join("Design");
        assert_eq!(
            materialized.as_deref(),
            Some(root.as_path()),
            "the first save reports the directory it materialized"
        );
        item.read_with(cx, |item, _| {
            assert_eq!(item.project_root(), Some(root.as_path()));
            assert!(!item.is_dirty());
        });
        assert!(root.join("fanta.json").is_file(), "manifest written");
        assert!(root.join("AGENTS.md").is_file(), "agent guide written");
        assert!(root.join("pages").is_dir(), "pages directory scaffolded");

        // A second save overwrites the same tree and materializes nothing new.
        item.update(cx, |item, _| item.dirty = true);
        let again = item.update(cx, |item, cx| item.save(cx)).await.unwrap();
        assert_eq!(
            again, None,
            "a save into an existing project materializes nothing"
        );
        item.read_with(cx, |item, _| {
            assert_eq!(item.project_root(), Some(root.as_path()));
            assert!(!item.is_dirty());
        });
    }

    #[gpui::test]
    async fn saving_a_stale_fig_refuses_to_overwrite_an_existing_project(cx: &mut TestAppContext) {
        // A .fig whose sibling project already exists on disk (materialized in a
        // prior session, since diverged) must NOT be clobbered by a save of a
        // stale in-memory parse: that would destroy newer on-disk content (the
        // agent-edits-the-project workflow). The item here has project_root=None
        // (never loaded the project) yet its save target already is a project.
        let project = empty_project(cx).await;
        let dir = tempfile::tempdir().unwrap();
        let fig_path = dir.path().join("Design.fig");
        let existing = dir.path().join("Design");
        fanta_format::scaffold_project_tree(&existing).expect("scaffold the pre-existing project");
        // A sentinel inside a subtree that a real write would `remove_dir_all`.
        let pages = existing.join("pages");
        std::fs::create_dir_all(&pages).unwrap();
        let sentinel = pages.join("SENTINEL");
        std::fs::write(&sentinel, b"newer on-disk content").unwrap();

        let item = ready_item(&project, fig_path, None, doc_with_one_page(), cx);
        item.update(cx, |item, _| item.dirty = true);

        let result = item.update(cx, |item, cx| item.save(cx)).await;
        assert!(
            result.is_err(),
            "saving a stale .fig over an existing project must fail, not overwrite"
        );
        assert!(
            sentinel.is_file(),
            "the existing project's contents must be left untouched"
        );
        item.read_with(cx, |item, _| {
            assert!(
                item.project_root().is_none(),
                "a refused materialization must not adopt the project it declined to overwrite"
            );
        });
    }

    #[gpui::test]
    async fn introducing_an_auto_layout_opens_the_resolve_gate(cx: &mut TestAppContext) {
        use fanta_doc::{AutoLayout, NodeData};

        // `uses_auto_layout` gates the whole-page re-solve and is cached at load.
        // A frame that gains its FIRST auto layout (the inspector's toggle) must
        // flip the gate on, or its children would never be laid out.
        let project = empty_project(cx).await;
        let doc = doc_with_one_page();
        let page_root = doc.pages().first().copied().expect("one page");
        let item = ready_item(
            &project,
            PathBuf::from("/tmp/nowhere/Design.fig"),
            None,
            doc,
            cx,
        );

        item.read_with(cx, |item, _| {
            assert!(
                !item.document().expect("ready").uses_auto_layout,
                "a document with no auto layout starts with the gate closed"
            );
        });

        let (old, new) = item.read_with(cx, |item, _| {
            let data = item
                .document()
                .expect("ready")
                .doc
                .scene
                .get(page_root)
                .expect("page node")
                .data
                .clone();
            let mut updated = data.clone();
            if let NodeData::Group(group) = &mut updated {
                group.auto_layout = Some(AutoLayout::default());
            }
            (data, updated)
        });
        item.update(cx, |item, cx| {
            item.apply(
                Operation::ReplaceData {
                    id: page_root,
                    old: Box::new(old),
                    new: Box::new(new),
                },
                cx,
            )
        })
        .expect("applying an auto layout");

        item.read_with(cx, |item, _| {
            assert!(
                item.document().expect("ready").uses_auto_layout,
                "introducing the first auto layout must open the re-solve gate"
            );
        });
    }
}
