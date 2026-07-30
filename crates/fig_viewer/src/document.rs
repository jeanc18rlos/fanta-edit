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
    App, AppContext as _, Context, Entity, EntityId, EventEmitter, Image, ImageFormat,
    SharedString, Subscription, Task, WeakEntity,
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
    /// Dirty state before the first transient preview frame. Cleared by a
    /// committed edit; canceling a preview restores it so opening and closing
    /// an inspector gesture without a change does not dirty the document.
    preview_dirty_before: Option<bool>,
    /// The project changed on disk while the canvas had unsaved edits.
    conflict: bool,
    /// An open FNX buffer has edits that are not yet represented by the
    /// persisted project tree. The validated source may still be previewed,
    /// but canvas-authored content changes must not race it.
    source_edit_locked: bool,
    /// Serializes source persistence/reconciliation across split views that
    /// share this item and its project buffer.
    source_edit_pipeline_in_progress: bool,
    /// Ignore worktree events until this instant; set around our own project
    /// writes so saving from the canvas does not trigger a self-reload.
    suppress_watcher_until: Option<Instant>,
    /// The last document state both the canvas and the disk agreed on (as of
    /// the last load, reload, or save). The common ancestor for the
    /// three-way merge that reconciles concurrent canvas edits with external
    /// (agent/hand) file edits instead of forcing Overwrite/Discard.
    merge_base: Option<Doc>,
    /// A scope requested while the document was still loading (from the open
    /// path — `page.fnx` / `master.fnx` / `doc/variables.json` — or a scoped
    /// re-open of the shared item). Applied and cleared when the load lands.
    pending_scope: Option<FigScope>,
    /// The most recently applied scope, so a view created after the fact
    /// (e.g. in a second window) can land in the right workspace.
    last_scope: Option<FigScope>,
    /// Bumped every time the authoritative document/disk state changes (save,
    /// source-edit adoption, reload, merge adoption). In-flight merge/reload
    /// tasks capture it when scheduled and abort if it moved — otherwise a
    /// task that loaded a disk snapshot BEFORE a save completed would adopt
    /// that stale snapshot afterwards, silently reverting (and on the next
    /// save destroying) freshly saved content.
    sync_epoch: u64,
    reload_task: Option<Task<()>>,
    _load_task: Option<Task<()>>,
    /// Worktree-event subscriptions, one per [`Project`] that opened this
    /// item. The item is shared across windows and each window brings its OWN
    /// `Project` entity, so watching only the first opener's project would
    /// silently end external-edit detection (reload/merge/conflict) when that
    /// window closes — and the next save would clobber newer disk state.
    /// Deduped by project entity id; dead entries are pruned opportunistically.
    project_subscriptions: Vec<(WeakEntity<Project>, Subscription)>,
}

/// One live [`FigItem`] per materialized project directory, so every open —
/// `fanta.json`, a `page.fnx`, a `master.fnx`, `doc/variables.json` — shares
/// the SAME document entity. Sharing is what makes scoped opens instant (no
/// re-read of the project tree), makes an edit in any tab land in every other
/// view immediately, and removes the self-inflicted disk conflicts two
/// parallel items produced. Weak handles: a closed project drops its item.
#[derive(Default)]
struct SharedProjectItems(HashMap<PathBuf, gpui::WeakEntity<FigItem>>);

impl gpui::Global for SharedProjectItems {}

fn shared_project_item(root: &Path, cx: &mut App) -> Option<Entity<FigItem>> {
    cx.default_global::<SharedProjectItems>()
        .0
        .get(root)
        .and_then(|item| item.upgrade())
}

fn register_shared_project_item(root: PathBuf, item: &Entity<FigItem>, cx: &mut App) {
    let registry = &mut cx.default_global::<SharedProjectItems>().0;
    registry.retain(|_, item| item.upgrade().is_some());
    registry.insert(root, item.downgrade());
}

/// Hand-off from [`FigItem::try_open`] to the [`FigView`](crate::FigView) the
/// workspace builds next: the worktree entry the user actually clicked and the
/// scope that path implies. The shared item cannot carry this per-open state
/// (it is ONE entity behind many openable paths), and the workspace's
/// item-building plumbing has no side channel — so a crate global bridges the
/// two, exactly like `agent_surface::set_active_item`. Sound because the
/// foreground is serial between `try_open` resolving and the view being
/// constructed, and every `try_open` overwrites the slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FigViewDescriptor {
    pub(crate) entry_id: Option<ProjectEntryId>,
    pub(crate) scope: Option<FigScope>,
}

#[derive(Default)]
struct PendingViewDescriptor(Option<FigViewDescriptor>);

impl gpui::Global for PendingViewDescriptor {}

pub(crate) fn set_pending_view_descriptor(descriptor: FigViewDescriptor, cx: &mut App) {
    cx.default_global::<PendingViewDescriptor>().0 = Some(descriptor);
}

pub(crate) fn take_pending_view_descriptor(cx: &mut App) -> Option<FigViewDescriptor> {
    cx.default_global::<PendingViewDescriptor>().0.take()
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
    /// The caret or character selection inside an active text-edit session
    /// changed, or its effective typography changed. This refreshes inspector
    /// values without changing which document node the inspector is bound to.
    TextSelectionChanged,
    /// The document finished (re)loading or was saved.
    StateChanged,
    /// The project diverged from disk while the canvas had unsaved edits, or
    /// that conflict was resolved by saving or reloading.
    ConflictChanged,
    /// An FNX buffer became dirty or returned to its persisted version.
    SourceEditLockChanged,
    /// A scoped open (`page.fnx` / `master.fnx` / `doc/variables.json`)
    /// re-targeted this shared document: the requesting view should refit its
    /// viewport to the new root and switch workspaces (Variables ↔ Canvas)
    /// accordingly.
    ScopeApplied(FigScope, ScopeRequester),
}

/// Who initiated a scope change, carried on [`FigItemEvent::ScopeApplied`] so
/// views can tell whether the re-target is theirs to follow. Keying the refit
/// on the requester (instead of each view's cached focus flag) matters
/// because GPUI focus listeners only run at the next draw: at event delivery
/// the PREVIOUS tab can still believe it is focused, and following the event
/// would clobber its saved viewport.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScopeRequester {
    /// A view re-asserted or navigated to this scope; only the view with this
    /// entity id follows.
    View(EntityId),
    /// A scoped open re-targeted the document for a tab that does not exist
    /// yet; no live view follows (the new view initializes from the already
    /// re-rooted document).
    Open,
    /// The pending scope was applied when the initial load landed; there is
    /// no requesting view, so the focused view follows.
    Load,
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
    /// Decoded images the agent ingested after load, layered over the
    /// load-time resolver so placing one image never re-decodes the whole
    /// asset set. Created lazily on the first ingestion; reset by reload
    /// (which rebuilds the resolver from disk anyway).
    agent_asset_overlay: Option<Arc<OverlayAssetResolver>>,
    /// Whether any node in the scene uses auto layout. Computed once at load;
    /// documents without it skip the whole-page layout re-solve after every
    /// edit, which includes text measurement and is far too slow to run per
    /// interaction on large pages.
    uses_auto_layout: bool,
    /// Monotonic render epoch for every persistent or transient document
    /// mutation, including variables and motion edits that do not change the
    /// scene graph's own revision.
    render_generation: u64,
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
        // The active page is runtime state, not persisted by the project tree,
        // so a freshly loaded document has none until the user clicks a page.
        // Every creation tool parents into `doc.active_page()` while the canvas
        // renders it with a selected-page fallback — leaving it unset sends new
        // shapes/text/frames to the scene root where the page-scoped render
        // never paints them (invisible until a drag reparents them into the
        // page). Default it to the page the canvas will show.
        if doc.active_page().is_none() {
            doc.set_active_page(pages.get(default_page_index).and_then(|page| page.root));
        }

        Self {
            doc,
            pages,
            default_page_index,
            solved_pages,
            asset_resolver,
            raw_assets: Arc::new(raw_assets),
            gpui_images,
            agent_asset_overlay: None,
            uses_auto_layout,
            render_generation: 0,
        }
    }

    pub(crate) fn render_generation(&self) -> u64 {
        self.render_generation
    }

    /// Continue the generation sequence of a document this one replaces.
    /// A fresh `from_doc` restarts at 0; without reseeding, an in-flight
    /// merge that captured generation G on the OLD document could collide
    /// with the NEW document reaching G and adopt a merge built from the
    /// pre-swap snapshot, dropping edits.
    pub(crate) fn continue_generation_after(&mut self, previous: Option<u64>) {
        if let Some(previous) = previous {
            self.render_generation = self.render_generation.max(previous.wrapping_add(1));
        }
    }

    fn advance_render_generation(&mut self) {
        self.render_generation = self.render_generation.wrapping_add(1);
    }

    /// Every distinct font family the document's text nodes reference (node
    /// style + per-run overrides), for pre-warming font downloads at open so a
    /// missing family is fetched off the paint thread instead of stalling it.
    pub fn used_font_families(&self) -> Vec<String> {
        let mut families: Vec<String> = Vec::new();
        let mut push = |name: &str| {
            let name = name.trim();
            if !name.is_empty() && !families.iter().any(|f| f.eq_ignore_ascii_case(name)) {
                families.push(name.to_string());
            }
        };
        for root in self.doc.scene.roots().to_vec() {
            for id in self.doc.scene.descendants_of(root) {
                if let Some(node) = self.doc.scene.get(id)
                    && let fanta_doc::NodeData::Text(text) = &node.data
                {
                    push(&text.style.font_family);
                    for run in &text.style_runs {
                        push(&run.style.font_family);
                    }
                }
            }
        }
        families
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

    /// Lazily solve layout for an arbitrary subtree root — a component master
    /// scoped into its own view, which is not listed in [`Self::pages`] and so
    /// can't go through [`Self::ensure_page_solved`]. Shares the same
    /// solved-once tracking, keyed by root id.
    pub fn ensure_root_solved(&mut self, root: NodeId) {
        if self.doc.scene.get(root).is_some() && self.solved_pages.insert(root) {
            solve_scene_layout(&mut self.doc.scene, root);
        }
    }

    /// Re-activate `root` in a freshly swapped-in document (disk reload,
    /// source adoption, merge): a listed page restores its index, a component
    /// master root restores the component scope. Returns whether the root was
    /// restored — `false` means it no longer exists and the caller should let
    /// the document fall back to its default page.
    pub(crate) fn restore_active_root(&mut self, root: NodeId) -> bool {
        if let Some(index) = self.pages.iter().position(|page| page.root == Some(root)) {
            self.ensure_page_solved(index);
            self.doc.set_active_page(Some(root));
            self.default_page_index = index;
            return true;
        }
        if self.doc.is_component_root(root) && self.doc.scene.get(root).is_some() {
            self.ensure_root_solved(root);
            self.doc.set_active_page(Some(root));
            return true;
        }
        false
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

    /// Split the document into the scene [`Doc`] and mutable views of the
    /// asset stores, so an edit batch can create nodes and ingest their image
    /// assets in the same pass without a double borrow.
    pub(crate) fn doc_and_assets(&mut self) -> (&mut Doc, AssetStores<'_>) {
        (
            &mut self.doc,
            AssetStores {
                raw_assets: &mut self.raw_assets,
                asset_resolver: &mut self.asset_resolver,
                overlay: &mut self.agent_asset_overlay,
                gpui_images: &mut self.gpui_images,
            },
        )
    }
}

/// Layers agent-ingested images over the resolver built at load, so placing
/// one image is O(1) instead of a re-decode of every embedded asset. Interior
/// mutability because the resolver is shared as an `Arc<dyn AssetResolver>`
/// with background renders; reads take the lock only on the overlay map.
pub(crate) struct OverlayAssetResolver {
    base: Option<Arc<dyn AssetResolver>>,
    added: std::sync::RwLock<HashMap<AssetId, DecodedImage>>,
}

impl AssetResolver for OverlayAssetResolver {
    fn resolve(&self, id: AssetId) -> Option<DecodedImage> {
        if let Some(image) = self.added.read().unwrap().get(&id) {
            return Some(image.clone());
        }
        self.base.as_ref()?.resolve(id)
    }

    fn resolve_bytes(&self, id: AssetId) -> Option<Arc<Vec<u8>>> {
        self.base.as_ref()?.resolve_bytes(id)
    }
}

/// Mutable views of every store an ingested image must reach: the raw bytes
/// (persisted on save), the render resolver, and the GPUI thumbnail cache.
pub(crate) struct AssetStores<'a> {
    raw_assets: &'a mut Arc<BTreeMap<AssetId, Vec<u8>>>,
    asset_resolver: &'a mut Option<Arc<dyn AssetResolver>>,
    overlay: &'a mut Option<Arc<OverlayAssetResolver>>,
    gpui_images: &'a mut HashMap<AssetId, Arc<Image>>,
}

/// Owned backing for [`AssetStores`] — for tests that exercise batch
/// application without a full [`FigDocument`].
#[cfg(test)]
#[derive(Default)]
pub(crate) struct TestAssetStores {
    raw_assets: Arc<BTreeMap<AssetId, Vec<u8>>>,
    asset_resolver: Option<Arc<dyn AssetResolver>>,
    overlay: Option<Arc<OverlayAssetResolver>>,
    gpui_images: HashMap<AssetId, Arc<Image>>,
}

#[cfg(test)]
impl TestAssetStores {
    pub(crate) fn stores(&mut self) -> AssetStores<'_> {
        AssetStores {
            raw_assets: &mut self.raw_assets,
            asset_resolver: &mut self.asset_resolver,
            overlay: &mut self.overlay,
            gpui_images: &mut self.gpui_images,
        }
    }

    pub(crate) fn raw_assets(&self) -> &BTreeMap<AssetId, Vec<u8>> {
        &self.raw_assets
    }

    pub(crate) fn resolver(&self) -> Option<&Arc<dyn AssetResolver>> {
        self.asset_resolver.as_ref()
    }
}

impl AssetStores<'_> {
    /// Ingest encoded image bytes as a fresh project asset. Decodes eagerly
    /// (so a corrupt payload fails the op instead of rendering a placeholder)
    /// and returns the new id plus the natural pixel size.
    pub(crate) fn add_image(&mut self, bytes: Vec<u8>) -> Result<(AssetId, [u32; 2])> {
        let decoded = image::load_from_memory(&bytes).context("decoding image bytes")?;
        let rgba = decoded.to_rgba8();
        let (width, height) = rgba.dimensions();
        anyhow::ensure!(width > 0 && height > 0, "the image has no pixels");
        let id = AssetId::new();

        // `raw_assets` is shared behind an `Arc` (the Assets panel keys its
        // cache on pointer identity), so ingesting clones the byte map. Fine
        // for occasional agent placements; batch imports should get a
        // shared-bytes representation first.
        let mut raw = (**self.raw_assets).clone();
        raw.insert(id, bytes.clone());
        *self.raw_assets = Arc::new(raw);

        if self.overlay.is_none() {
            *self.overlay = Some(Arc::new(OverlayAssetResolver {
                base: self.asset_resolver.clone(),
                added: std::sync::RwLock::new(HashMap::default()),
            }));
        }
        let overlay = self.overlay.as_ref().expect("just ensured above");
        overlay.added.write().unwrap().insert(
            id,
            DecodedImage::new(Arc::new(rgba.into_raw()), width, height),
        );
        *self.asset_resolver = Some(overlay.clone() as Arc<dyn AssetResolver>);

        if let Some(format) = image::guess_format(&bytes).ok().and_then(gpui_image_format) {
            self.gpui_images
                .insert(id, Arc::new(Image::from_bytes(format, bytes)));
        }
        Ok((id, [width, height]))
    }

    /// Drop an asset ingested by [`add_image`](Self::add_image) again — the
    /// rollback path when a batch fails after ingesting.
    pub(crate) fn remove(&mut self, id: AssetId) {
        if self.raw_assets.contains_key(&id) {
            let mut raw = (**self.raw_assets).clone();
            raw.remove(&id);
            *self.raw_assets = Arc::new(raw);
        }
        if let Some(overlay) = self.overlay.as_ref() {
            overlay.added.write().unwrap().remove(&id);
        }
        self.gpui_images.remove(&id);
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
        // A project source file opens the editor SCOPED to what it describes:
        // `pages/<id>/page.fnx` → that page, `components/<id>/master.fnx` →
        // that component master alone (atomic component editing),
        // `doc/variables.json` → the variables space. The check includes an
        // `is_project_dir` probe (one tiny fanta.json read) so a loose
        // look-alike path still falls through to the text editor.
        let scoped = abs_path.as_deref().and_then(scoped_project_source);
        if !is_fig && !is_manifest && scoped.is_none() {
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
            let initial_scope = scoped.as_ref().map(|(_, scope)| *scope);
            let project_root = if let Some((root, _)) = scoped {
                Some(root)
            } else if is_manifest {
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

            // One live item per project directory: a scoped open (page.fnx,
            // master.fnx, variables.json) of an already-open project reuses
            // the SHARED document — instant (no re-read of the tree), and the
            // pane's entry-id dedupe then activates the existing editor tab,
            // making the click pure navigation. Applying the scope re-targets
            // that shared document.
            let registry_key = project_root
                .as_deref()
                .map(|root| root.canonicalize().unwrap_or_else(|_| root.to_path_buf()));
            if let Some(key) = &registry_key
                && let Some(item) = cx.update(|cx| shared_project_item(key, cx))
            {
                item.update(cx, |item, cx| {
                    // A second window reaches this branch with its OWN
                    // `Project` entity; the shared item must watch that
                    // project's worktree events too, or external-edit
                    // detection dies with the original window.
                    item.subscribe_to_project(&project, cx);
                    if let Some(scope) = initial_scope {
                        item.request_scope(scope, ScopeRequester::Open, cx);
                    }
                });
                cx.update(|cx| {
                    set_pending_view_descriptor(
                        FigViewDescriptor {
                            entry_id,
                            scope: initial_scope,
                        },
                        cx,
                    )
                });
                return Ok(item);
            }
            let item = cx.new(|cx| {
                let load_path = abs_path.clone();
                let load_project_root = project_root.clone();
                let load_task = cx.spawn({
                    let project = project.downgrade();
                    async move |this, cx| {
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

                        let load_result = cx
                            .background_spawn(async move {
                                match load_project_root {
                                    Some(root) => {
                                        load_project_document(&root).map(|document| (document, None))
                                    }
                                    // First open of a bare `.fig`: parse it AND
                                    // materialize the project directory right
                                    // away, so the editor is project-backed and
                                    // editable from the first frame instead of
                                    // leaving a folder to appear only on the
                                    // first save. A failed materialization
                                    // degrades to the old in-memory mode — the
                                    // parse is still shown and the first save
                                    // retries the write.
                                    None => load_fig_document(&load_path).map(|document| {
                                        let target = available_project_dir(&load_path);
                                        if fanta_format::is_project_dir(&target) {
                                            // Appeared since the redirect check
                                            // in `try_open`; never overwrite it.
                                            return (document, None);
                                        }
                                        let created_here = !target.exists();
                                        match write_project(
                                            &target,
                                            &document.doc,
                                            &document.raw_assets,
                                        ) {
                                            Ok(()) => (document, Some(target)),
                                            Err(error) => {
                                                log::error!(
                                                    "materializing Fanta project at {} on open failed: {error:#}",
                                                    target.display()
                                                );
                                                // A half-written dir is already
                                                // tagged as a project (the
                                                // manifest is scaffolded first),
                                                // so leaving it would hijack
                                                // every reopen AND block the
                                                // save that could repair it.
                                                // Remove what we created; the
                                                // first save re-materializes.
                                                if created_here
                                                    && let Err(error) =
                                                        std::fs::remove_dir_all(&target)
                                                {
                                                    log::error!(
                                                        "cleaning up partial Fanta project at {} failed: {error:#}",
                                                        target.display()
                                                    );
                                                }
                                                (document, None)
                                            }
                                        }
                                    }),
                                }
                            })
                            .await;

                        let adopted_root = load_result
                            .as_ref()
                            .ok()
                            .and_then(|(_, root)| root.clone());
                        let document = load_result.map(|(document, _)| document);
                        if let Err(error) = this.update(cx, |this: &mut FigItem, cx| {
                            if this.sync_epoch != 0 {
                                // An external change already reloaded a newer
                                // document while this initial load ran (the
                                // project root was known from open, so the
                                // watcher was live); installing this older
                                // snapshot would regress it and poison
                                // merge_base for the next save.
                                return;
                            }
                            if let Some(root) = adopted_root.clone() {
                                // The write above echoes back through the
                                // worktree watcher once the folder is adopted;
                                // suppress it exactly like a save's self-write.
                                this.suppress_watcher_until =
                                    Some(Instant::now() + SELF_WRITE_SUPPRESS_WINDOW);
                                this.project_root = Some(root.clone());
                                // The project directory now exists; share this
                                // item so later scoped opens reuse it.
                                let key = root.canonicalize().unwrap_or(root);
                                register_shared_project_item(key, &cx.entity(), cx);
                            }
                            this.document = FigDocumentState::from_result(document);
                            if let Some(scope) = this.pending_scope.take()
                                && let FigDocumentState::Ready(document) = &mut this.document
                            {
                                apply_scope(document, scope);
                                this.last_scope = Some(scope);
                                cx.emit(FigItemEvent::ScopeApplied(scope, ScopeRequester::Load));
                            }
                            this.merge_base =
                                this.document.ready().map(|document| document.doc.clone());
                            cx.emit(FigItemEvent::StateChanged);
                            cx.notify();
                        }) {
                            log::debug!("dropping loaded update for closed .fig item: {error:#}");
                            return;
                        }

                        // Surface the freshly materialized project as a visible
                        // worktree so its `fanta.json` / `.fnx` / asset files
                        // show in the project panel — the editor is now
                        // launched "from the folder". Never fails the open.
                        if let Some(root) = adopted_root
                            && let Some(project) = project.upgrade()
                        {
                            let worktree = project.update(cx, |project, cx| {
                                project.find_or_create_worktree(root.clone(), true, cx)
                            });
                            if let Err(error) = worktree.await {
                                log::error!(
                                    "adding materialized Fanta project {} to the workspace failed: {error:#}",
                                    root.display()
                                );
                            }
                        }
                    }
                });

                let project_subscription = Self::project_subscription(&project, cx);

                Self {
                    path,
                    abs_path,
                    entry_id,
                    document: FigDocumentState::Loading {
                        message: "Opening document...".into(),
                    },
                    project_root,
                    dirty: false,
                    preview_dirty_before: None,
                    conflict: false,
                    source_edit_locked: false,
                    source_edit_pipeline_in_progress: false,
                    suppress_watcher_until: None,
                    merge_base: None,
                    pending_scope: initial_scope,
                    last_scope: None,
                    sync_epoch: 0,
                    reload_task: None,
                    _load_task: Some(load_task),
                    project_subscriptions: vec![(project.downgrade(), project_subscription)],
                }
            });
            if let Some(key) = registry_key {
                cx.update(|cx| register_shared_project_item(key, &item, cx));
            }
            cx.update(|cx| {
                set_pending_view_descriptor(
                    FigViewDescriptor {
                        entry_id,
                        scope: initial_scope,
                    },
                    cx,
                )
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

    /// A document is editable as soon as it has parsed, unless a dirty FNX
    /// buffer currently owns the source of truth. A freshly opened `.fig`
    /// still edits in memory from the first parse; the first save writes the
    /// project directory (see [`FigItem::save`]).
    pub fn is_editable(&self) -> bool {
        self.document.ready().is_some() && !self.source_edit_locked
    }

    pub fn has_ready_document(&self) -> bool {
        self.document.ready().is_some()
    }

    pub fn source_edit_locked(&self) -> bool {
        self.source_edit_locked
    }

    pub(crate) fn set_source_edit_locked(
        &mut self,
        source_edit_locked: bool,
        cx: &mut Context<Self>,
    ) {
        if self.source_edit_locked == source_edit_locked {
            return;
        }
        self.source_edit_locked = source_edit_locked;
        cx.emit(FigItemEvent::SourceEditLockChanged);
        cx.notify();
    }

    fn begin_source_edit_save(&mut self) {
        self.source_edit_pipeline_in_progress = true;
        self.suppress_watcher_until = Some(Instant::now() + SELF_WRITE_SUPPRESS_WINDOW);
    }

    pub(crate) fn try_begin_source_edit_pipeline(&mut self) -> bool {
        if self.source_edit_pipeline_in_progress {
            return false;
        }
        self.begin_source_edit_save();
        true
    }

    pub(crate) fn finish_source_edit_pipeline(&mut self) {
        self.source_edit_pipeline_in_progress = false;
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

    fn project_subscription(project: &Entity<Project>, cx: &mut Context<Self>) -> Subscription {
        cx.subscribe(project, |this: &mut Self, project, event, cx| {
            if let project::Event::WorktreeUpdatedEntries(worktree_id, changes) = event {
                this.worktree_entries_updated(&project, *worktree_id, changes, cx);
            }
        })
    }

    /// Watch `project`'s worktree events for external edits to the project
    /// tree. Idempotent per project entity; called for every window that
    /// opens this shared item, so external-edit detection outlives any single
    /// window's `Project`.
    pub(crate) fn subscribe_to_project(&mut self, project: &Entity<Project>, cx: &mut Context<Self>) {
        self.project_subscriptions
            .retain(|(project, _)| project.upgrade().is_some());
        if self
            .project_subscriptions
            .iter()
            .any(|(existing, _)| existing.entity_id() == project.entity_id())
        {
            return;
        }
        let subscription = Self::project_subscription(project, cx);
        self.project_subscriptions
            .push((project.downgrade(), subscription));
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
        if self.source_edit_locked {
            // A dirty FNX buffer owns its live preview; merging under it
            // would race the text the user is still editing.
            self.set_conflict(true, cx);
        } else if self.dirty {
            // Unsaved canvas edits + external file edits: try a three-way
            // merge against the last agreed state instead of forcing the
            // binary Overwrite/Discard choice.
            self.schedule_merge(cx);
        } else {
            self.schedule_reload(cx);
        }
    }

    /// Debounce an external disk change that landed while the canvas has
    /// unsaved edits, then three-way merge disk against the canvas. A clean
    /// merge is adopted silently (the canvas stays dirty — its half is not
    /// on disk yet); any real conflict falls back to the conflict banner.
    fn schedule_merge(&mut self, cx: &mut Context<Self>) {
        let Some(root) = self.project_root.clone() else {
            self.set_conflict(true, cx);
            return;
        };
        let Some(base) = self.merge_base.clone() else {
            self.set_conflict(true, cx);
            return;
        };
        let epoch = self.sync_epoch;
        self.reload_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(RELOAD_DEBOUNCE).await;
            let loaded = cx
                .background_spawn({
                    let root = root.clone();
                    async move { load_project_document(&root) }
                })
                .await;
            let theirs = match loaded {
                Ok(theirs) => theirs,
                Err(error) => {
                    log::error!(
                        "loading external changes for merge from {} failed: {error:#}",
                        root.display()
                    );
                    // Possibly a half-written batch; the next watcher event
                    // retries. Keep the canvas and flag the divergence.
                    if let Err(error) = this.update(cx, |this, cx| {
                        if this.sync_epoch == epoch {
                            this.set_conflict(true, cx);
                        }
                    }) {
                        log::debug!("dropping merge for closed Fanta project item: {error:#}");
                    }
                    return;
                }
            };
            let ours = this.read_with(cx, |this, _| {
                this.document
                    .ready()
                    .map(|document| (document.doc.clone(), document.render_generation()))
            });
            let Ok(Some((ours, ours_generation))) = ours else {
                return;
            };
            let merge = cx
                .background_spawn({
                    let theirs_doc = theirs.doc.clone();
                    async move { fanta_format::merge_docs(&base, &ours, &theirs_doc) }
                })
                .await;
            if let Err(error) = this.update(cx, |this, cx| {
                if this.sync_epoch != epoch {
                    // A save/adoption/reload advanced the authoritative state
                    // while this task held a pre-advance disk snapshot;
                    // adopting it now would revert that newer state. The
                    // watcher event that scheduled this task is spent, so
                    // re-check disk under the new epoch instead of silently
                    // diverging.
                    this.schedule_resync(cx);
                    return;
                }
                if this.source_edit_locked {
                    this.set_conflict(true, cx);
                    return;
                }
                if !this.dirty {
                    // The canvas edits were saved or discarded mid-merge;
                    // plain reload semantics apply.
                    this.apply_reloaded_document(theirs, cx);
                    return;
                }
                if this
                    .document
                    .ready()
                    .is_none_or(|document| document.render_generation() != ours_generation)
                {
                    // The user kept editing while the merge computed; the
                    // `ours` snapshot is stale and adopting its merge would
                    // silently drop those newer edits. Merge again from the
                    // current state.
                    this.schedule_merge(cx);
                    return;
                }
                match merge {
                    Ok(merge) if merge.is_clean() => {
                        this.adopt_merged_document(merge.doc, theirs, cx);
                    }
                    Ok(merge) => {
                        log::info!(
                            "external changes conflict with unsaved canvas edits at: {}",
                            merge.conflicts.join(", ")
                        );
                        this.set_conflict(true, cx);
                    }
                    Err(error) => {
                        log::warn!("merging external changes failed: {error:#}");
                        this.set_conflict(true, cx);
                    }
                }
            }) {
                log::debug!("dropping merge for closed Fanta project item: {error:#}");
            }
        }));
    }

    /// Swap in a cleanly merged document: the union of the canvas's unsaved
    /// edits and the external file edits. The canvas stays dirty (its half
    /// of the merge is not on disk yet) and the merge base advances to the
    /// disk state so the next external change merges against the right
    /// ancestor.
    fn adopt_merged_document(
        &mut self,
        merged: Doc,
        disk: FigDocument,
        cx: &mut Context<Self>,
    ) {
        let mut raw_assets: BTreeMap<AssetId, Vec<u8>> = (*disk.raw_assets).clone();
        if let Some(current) = self.document.ready() {
            for (id, bytes) in current.raw_assets.iter() {
                raw_assets.entry(*id).or_insert_with(|| bytes.clone());
            }
        }
        let active_page = merged.active_page();
        let mut document = FigDocument::from_doc(merged, raw_assets);
        if let Some(root) = active_page
            && !document.restore_active_root(root)
        {
            // The external edit deleted the page/master the user was viewing;
            // a dangling active root would swallow new shapes (tools parent
            // into it) and blank the scoped render. Fall back like a page
            // deletion does.
            let fallback = document
                .pages
                .get(document.default_page_index)
                .and_then(|page| page.root);
            document.doc.set_active_page(fallback);
        }
        // The merge keeps the local selection verbatim; drop entries whose
        // nodes the external edit deleted so inspectors and alignment ops
        // never operate on dead ids.
        let surviving: Vec<_> = document
            .doc
            .selection
            .as_slice()
            .iter()
            .copied()
            .filter(|node| document.doc.scene.get(*node).is_some())
            .collect();
        document.doc.selection.replace_with(surviving);
        document.continue_generation_after(
            self.document.ready().map(|current| current.render_generation()),
        );
        self.document = FigDocumentState::Ready(document);
        self.merge_base = Some(disk.doc);
        self.sync_epoch += 1;
        self.set_conflict(false, cx);
        cx.emit(FigItemEvent::StateChanged);
        cx.notify();
    }

    /// Re-dispatch after a stale task aborted on the epoch guard: the watcher
    /// event that scheduled it is spent, so disk must be re-checked under the
    /// new epoch or an external edit that raced a save would silently
    /// diverge from the canvas (and be overwritten by the next save).
    fn schedule_resync(&mut self, cx: &mut Context<Self>) {
        if self.source_edit_locked {
            self.set_conflict(true, cx);
        } else if self.dirty {
            self.schedule_merge(cx);
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
        let epoch = self.sync_epoch;
        self.reload_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(RELOAD_DEBOUNCE).await;
            let loaded = cx
                .background_spawn(async move { load_project_document(&root) })
                .await;
            if let Err(error) = this.update(cx, |this, cx| {
                if this.sync_epoch != epoch {
                    // A save/adoption advanced the authoritative state while
                    // this reload held an older disk snapshot; applying it
                    // would revert the newer state. Re-check disk under the
                    // new epoch — the watcher event this task consumed will
                    // not fire again.
                    this.schedule_resync(cx);
                    return;
                }
                if this.dirty || this.source_edit_locked {
                    // Canvas or FNX edits landed while the reload was in
                    // flight; keep them and flag the divergence.
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
        if let Some(root) = previous_page_root {
            document.restore_active_root(root);
        }
        document.continue_generation_after(
            self.document.ready().map(|current| current.render_generation()),
        );
        self.merge_base = Some(document.doc.clone());
        self.document = FigDocumentState::Ready(document);
        self.dirty = false;
        self.preview_dirty_before = None;
        self.sync_epoch += 1;
        self.set_conflict(false, cx);
        cx.emit(FigItemEvent::StateChanged);
        cx.notify();
    }

    pub(crate) fn adopt_source_edit(
        &mut self,
        source_edit: fanta_format::ProjectSourceEdit,
        cx: &mut Context<Self>,
    ) {
        let previous_page_root = self
            .document
            .ready()
            .and_then(|current| current.doc.active_page());
        let previous_selection = self
            .document
            .ready()
            .map(|current| current.doc.selection.as_slice().to_vec())
            .unwrap_or_default();
        let previous_viewport = self
            .document
            .ready()
            .map(|current| current.doc.viewport)
            .unwrap_or_default();
        let mut document = FigDocument::from_doc(source_edit.document, source_edit.assets);
        if let Some(root) = previous_page_root {
            document.restore_active_root(root);
        }
        let preserved_selection: Vec<_> = previous_selection
            .into_iter()
            .filter(|node| document.doc.scene.get(*node).is_some())
            .collect();
        document.doc.selection.replace_with(preserved_selection);
        document.doc.viewport = previous_viewport;
        document.continue_generation_after(
            self.document.ready().map(|current| current.render_generation()),
        );
        self.document = FigDocumentState::Ready(document);
        self.dirty = false;
        self.preview_dirty_before = None;
        self.set_conflict(false, cx);
        cx.emit(FigItemEvent::StateChanged);
        cx.notify();
    }

    pub(crate) fn adopt_saved_source_edit(
        &mut self,
        source_edit: fanta_format::ProjectSourceEdit,
        cx: &mut Context<Self>,
    ) {
        self.suppress_watcher_until = Some(Instant::now() + SELF_WRITE_SUPPRESS_WINDOW);
        // A pending merge/reload holds a disk snapshot from before this save;
        // cancel it — the adoption below is the newer authoritative state.
        self.reload_task = None;
        self.adopt_source_edit(source_edit, cx);
        // The persisted source edit is now the on-disk state; advance the
        // merge ancestor with it — from the ADOPTED (normalized) document,
        // not the reader-raw one, so a later merge doesn't see phantom
        // base-vs-ours differences on solver-derived geometry. (The
        // unsaved-preview adopt path must NOT advance the ancestor — the
        // disk still holds the older tree there.)
        self.merge_base = self.document.ready().map(|document| document.doc.clone());
        self.sync_epoch += 1;
    }

    /// Reload the project from disk immediately, discarding unsaved canvas
    /// edits. This backs the workspace's "discard and reload" choice in the
    /// conflict prompt.
    pub fn reload_from_disk(&mut self, cx: &mut Context<Self>) -> Task<Result<()>> {
        if self.source_edit_locked {
            return Task::ready(Err(anyhow::anyhow!(
                "save or discard the current FNX edit before reloading the canvas"
            )));
        }
        self.reload_from_disk_unchecked(cx)
    }

    pub(crate) fn discard_canvas_edits_for_source_resolution(
        &mut self,
        cx: &mut Context<Self>,
    ) -> Task<Result<()>> {
        self.reload_from_disk_unchecked(cx)
    }

    fn reload_from_disk_unchecked(&mut self, cx: &mut Context<Self>) -> Task<Result<()>> {
        let Some(root) = self.project_root.clone() else {
            return Task::ready(Ok(()));
        };
        self.reload_task = None;
        let epoch = self.sync_epoch;
        cx.spawn(async move |this, cx| {
            let document = cx
                .background_spawn(async move { load_project_document(&root) })
                .await?;
            this.update(cx, |this, cx| {
                // A save/adoption superseded this discard while its snapshot
                // loaded; applying the older snapshot would revert (and on
                // the next save destroy) the newer state.
                if this.sync_epoch == epoch {
                    this.apply_reloaded_document(document, cx);
                }
            })?;
            Ok(())
        })
    }

    /// The display name of the document: the project directory name once a
    /// Fanta project exists, the `.fig` file name before that. One shared
    /// item backs every open of the project, so the tab names the project —
    /// scoped opens navigate this same editor rather than adding tabs.
    pub fn title(&self) -> SharedString {
        let project = self
            .project_root
            .as_deref()
            .and_then(Path::file_name)
            .or_else(|| self.abs_path.file_name())
            .and_then(|name| name.to_str())
            .unwrap_or("Figma");
        project.to_string().into()
    }

    /// Re-target the shared document onto `scope` — the effect of clicking a
    /// `page.fnx` / `master.fnx` / `doc/variables.json` while the project is
    /// already open, or of a tab re-asserting its scope on focus. Applies
    /// immediately on a ready document (the `requester` follows via
    /// [`FigItemEvent::ScopeApplied`]); queues until the load lands otherwise
    /// (the load-time apply emits [`ScopeRequester::Load`]).
    pub(crate) fn request_scope(
        &mut self,
        scope: FigScope,
        requester: ScopeRequester,
        cx: &mut Context<Self>,
    ) {
        match &mut self.document {
            FigDocumentState::Ready(document) => {
                // Focused tabs re-assert their scope on every tab switch; when
                // the document already shows this scope's root there is
                // nothing to re-target, and emitting `ScopeApplied` anyway
                // would make the requesting view reset its viewport for no
                // actual change.
                if scope_target_root(document, scope)
                    .is_none_or(|root| document.doc.active_page() == Some(root))
                {
                    self.last_scope = Some(scope);
                    return;
                }
                apply_scope(document, scope);
                self.last_scope = Some(scope);
                cx.emit(FigItemEvent::ScopeApplied(scope, requester));
                // The active-page change is presence state; this nudges the
                // code workspace and panels to re-bind their sources.
                cx.emit(FigItemEvent::SelectionChanged);
                cx.notify();
            }
            _ => self.pending_scope = Some(scope),
        }
    }

    /// The most recently applied open scope, if any.
    pub(crate) fn last_scope(&self) -> Option<FigScope> {
        self.last_scope
    }

    /// Whether a variables-space scope is queued behind the initial load.
    pub(crate) fn pending_variables_scope(&self) -> bool {
        self.pending_scope == Some(FigScope::Variables)
    }

    /// Apply an undoable operation to the document, re-solve the affected
    /// page's layout, and mark the item dirty.
    pub fn apply(&mut self, operation: Operation, cx: &mut Context<Self>) -> Result<()> {
        if self.source_edit_locked {
            anyhow::bail!("save or discard the current FNX edit before editing the canvas");
        }
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
        document.advance_render_generation();
        self.preview_dirty_before = None;
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
                document.advance_render_generation();
                self.preview_dirty_before = None;
                self.mark_edited(false, cx);
            }
            DocChange::ContentPreview => {
                if self.preview_dirty_before.is_none() {
                    self.preview_dirty_before = Some(self.dirty);
                }
                document.advance_render_generation();
                self.mark_edited(true, cx);
            }
        }
        Some(result)
    }

    pub fn undo(&mut self, cx: &mut Context<Self>) -> Result<bool> {
        if self.source_edit_locked {
            anyhow::bail!("save or discard the current FNX edit before editing the canvas");
        }
        let document = self
            .document
            .ready_mut()
            .context("the document is still loading")?;
        let did = document.doc.undo().context("undoing canvas operation")?;
        if did {
            let surviving_selection = document
                .doc
                .selection
                .iter()
                .copied()
                .filter(|id| document.doc.scene.contains(*id))
                .collect::<Vec<_>>();
            document.doc.selection.replace_with(surviving_selection);
            let active_page = document.doc.active_page();
            document.resolve_after_edit(active_page);
            document.advance_render_generation();
            self.preview_dirty_before = None;
            self.mark_edited(false, cx);
        }
        Ok(did)
    }

    pub fn redo(&mut self, cx: &mut Context<Self>) -> Result<bool> {
        if self.source_edit_locked {
            anyhow::bail!("save or discard the current FNX edit before editing the canvas");
        }
        let document = self
            .document
            .ready_mut()
            .context("the document is still loading")?;
        let did = document.doc.redo().context("redoing canvas operation")?;
        if did {
            let surviving_selection = document
                .doc
                .selection
                .iter()
                .copied()
                .filter(|id| document.doc.scene.contains(*id))
                .collect::<Vec<_>>();
            document.doc.selection.replace_with(surviving_selection);
            let active_page = document.doc.active_page();
            document.resolve_after_edit(active_page);
            document.advance_render_generation();
            self.preview_dirty_before = None;
            self.mark_edited(false, cx);
        }
        Ok(did)
    }

    fn mark_edited(&mut self, transient: bool, cx: &mut Context<Self>) {
        let was_dirty = self.dirty;
        if self.document.ready().is_some() {
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

    pub(crate) fn finish_content_preview(&mut self, committed: bool, cx: &mut Context<Self>) {
        let Some(dirty_before) = self.preview_dirty_before.take() else {
            return;
        };
        if !committed && self.dirty != dirty_before {
            self.dirty = dirty_before;
            cx.emit(FigItemEvent::Edited);
            cx.notify();
        }
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
        if self.source_edit_locked {
            return Task::ready(Err(anyhow::anyhow!(
                "the FNX source is dirty; save it through the code workspace before saving the canvas"
            )));
        }
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
        // A pending merge/reload holds a disk snapshot from before this save;
        // cancel it so it can't adopt that stale snapshot after the write
        // lands (silently reverting — and on the next save destroying — the
        // content saved here).
        self.reload_task = None;
        cx.spawn(async move |this, cx| {
            let saved_doc = doc.clone();
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
                    // Disk and canvas agree again — this is the new merge
                    // ancestor for reconciling future concurrent edits.
                    this.merge_base = Some(saved_doc);
                    this.dirty = false;
                    this.preview_dirty_before = None;
                    this.sync_epoch += 1;
                    this.set_conflict(false, cx);
                    cx.emit(FigItemEvent::StateChanged);
                    cx.notify();
                }
            })?;
            result.map(|()| materializing.then_some(target))
        })
    }
}

#[cfg(test)]
pub(crate) fn ready_item_for_test(
    project: &Entity<Project>,
    abs_path: PathBuf,
    doc: Doc,
    cx: &mut gpui::TestAppContext,
) -> Entity<FigItem> {
    use util::rel_path::RelPath;

    let item = cx.new(|_| FigItem {
        path: ProjectPath {
            worktree_id: WorktreeId::from_usize(0),
            path: RelPath::empty_arc(),
        },
        abs_path,
        entry_id: None,
        document: FigDocumentState::Ready(FigDocument::from_doc(doc, BTreeMap::new())),
        project_root: None,
        dirty: false,
        preview_dirty_before: None,
        conflict: false,
        source_edit_locked: false,
        source_edit_pipeline_in_progress: false,
        suppress_watcher_until: None,
        merge_base: None,
        pending_scope: None,
        last_scope: None,
        sync_epoch: 0,
        reload_task: None,
        _load_task: None,
        project_subscriptions: Vec::new(),
    });
    item.update(cx, |item, cx| item.subscribe_to_project(project, cx));
    item
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

/// Whether an externally changed path is an input to the project document.
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
    match first_component.as_os_str().to_str() {
        Some("fanta.json") => components.next().is_none(),
        Some("doc" | "pages" | "components" | "assets") => true,
        _ => false,
    }
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

/// What a scoped open focuses the editor on. Carried from [`FigItem::try_open`]
/// (when the opened path is a project source file) into the loaded document
/// and the view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FigScope {
    /// `pages/<id>/page.fnx` — the canvas scoped to that page.
    Page(NodeId),
    /// `components/<id>/master.fnx` — the canvas scoped to that component's
    /// master subtree alone: atomic component editing, every instance follows.
    Component(fanta_doc::ComponentId),
    /// `doc/variables.json` — the variables space.
    Variables,
}

/// Parse a scoped-open path inside a materialized Fanta project:
/// `<root>/pages/<slug>/page.fnx`, `<root>/components/<slug>/master.fnx`
/// (identity resolved from the design's JSON header — dir names are v3 slugs,
/// with the v2 id-named fallback handled by `fanta_format`), or
/// `<root>/doc/variables.json`. Returns the project root and the scope, or
/// `None` (including when the candidate root is not a tagged project dir, so
/// look-alike paths outside a project keep opening as plain text).
fn scoped_project_source(path: &Path) -> Option<(PathBuf, FigScope)> {
    if let Some((root, design)) = fanta_format::page_scope_of_source(path) {
        if !fanta_format::is_project_dir(&root) {
            return None;
        }
        let scope = match design {
            fanta_format::ScopedDesign::Page(page) => FigScope::Page(page),
            fanta_format::ScopedDesign::Component(component) => FigScope::Component(component),
        };
        return Some((root, scope));
    }
    let doc_directory = path.parent()?;
    if path.file_name()? == "variables.json" && doc_directory.file_name()? == "doc" {
        let root = doc_directory.parent()?;
        return fanta_format::is_project_dir(root).then(|| (root.to_path_buf(), FigScope::Variables));
    }
    None
}

/// The root node [`apply_scope`] would activate for `scope`, when that target
/// still exists in the document. `None` for the variables space (it never
/// re-roots the canvas) and for targets that are gone.
fn scope_target_root(document: &FigDocument, scope: FigScope) -> Option<NodeId> {
    match scope {
        FigScope::Page(root) => document
            .pages
            .iter()
            .find_map(|page| (page.root == Some(root)).then_some(root)),
        FigScope::Component(component) => document
            .doc
            .components
            .defs
            .get(&component)
            .map(|def| def.root)
            .filter(|root| document.doc.scene.get(*root).is_some()),
        FigScope::Variables => None,
    }
}

/// Focus a freshly loaded document on its open scope: activate the page, or
/// activate the component master's root (the canvas renders, hit-tests, and
/// parents into that subtree alone). A scope whose target no longer exists is
/// ignored — the document opens on its default page instead of failing.
fn apply_scope(document: &mut FigDocument, scope: FigScope) {
    match scope {
        FigScope::Page(root) => {
            if let Some(index) = document
                .pages
                .iter()
                .position(|page| page.root == Some(root))
            {
                document.ensure_page_solved(index);
                document.doc.set_active_page(Some(root));
                document.default_page_index = index;
            }
        }
        FigScope::Component(component) => {
            if let Some(root) = document
                .doc
                .components
                .defs
                .get(&component)
                .map(|def| def.root)
                .filter(|root| document.doc.scene.get(*root).is_some())
            {
                document.ensure_root_solved(root);
                document.doc.set_active_page(Some(root));
            }
        }
        // The variables space is a view concern (the Variables workspace);
        // the document itself opens unscoped.
        FigScope::Variables => {}
    }
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

    /// A doc with one page and one component master (an ordinary group
    /// promoted via DefineComponent), for scope tests.
    fn doc_with_page_and_component() -> (Doc, NodeId, fanta_doc::ComponentId, NodeId) {
        use fanta_doc::{CanvasNode, ComponentDef, ComponentId, GroupNode, NodeData};
        let mut doc = Doc::new();
        let mut page = CanvasNode::new(NodeData::Group(GroupNode::default()));
        page.name = "Page 1".to_owned();
        let page_root = page.id;
        doc.apply(Operation::create_node(page)).expect("create page");
        doc.add_page(page_root);
        let mut master = CanvasNode::new(NodeData::Group(GroupNode::default()));
        master.name = "Button".to_owned();
        let master_root = master.id;
        doc.apply(Operation::create_node(master))
            .expect("create master");
        let component = ComponentId::new();
        doc.apply(Operation::DefineComponent {
            def: Box::new(ComponentDef {
                id: component,
                root: master_root,
                name: "Button".into(),
                variant_of: None,
                props: Vec::new(),
                rev: 0,
            }),
        })
        .expect("define component");
        (doc, page_root, component, master_root)
    }

    #[test]
    fn scoped_project_source_parses_only_project_backed_paths() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        fanta_format::scaffold_project_tree(root).expect("scaffold");

        let page = NodeId::new();
        let page_path = root
            .join("pages")
            .join(page.to_string())
            .join("page.fnx");
        assert_eq!(
            scoped_project_source(&page_path),
            Some((root.to_path_buf(), FigScope::Page(page)))
        );

        let component = fanta_doc::ComponentId::new();
        let master_path = root
            .join("components")
            .join(component.to_string())
            .join("master.fnx");
        assert_eq!(
            scoped_project_source(&master_path),
            Some((root.to_path_buf(), FigScope::Component(component)))
        );

        assert_eq!(
            scoped_project_source(&root.join("doc").join("variables.json")),
            Some((root.to_path_buf(), FigScope::Variables))
        );

        // Wrong file names, malformed ids, and look-alike paths outside a
        // tagged project all decline (falling through to the text editor).
        assert_eq!(scoped_project_source(&root.join("doc").join("motion.json")), None);
        assert_eq!(
            scoped_project_source(&root.join("pages").join("not-an-id").join("page.fnx")),
            None
        );
        let outside = dir.path().join("not-a-project");
        std::fs::create_dir_all(outside.join("pages").join(page.to_string())).expect("mkdir");
        assert_eq!(
            scoped_project_source(
                &outside.join("pages").join(page.to_string()).join("page.fnx")
            ),
            None
        );
    }

    #[test]
    fn component_scope_activates_the_master_root() {
        let (doc, page_root, component, master_root) = doc_with_page_and_component();
        let mut document = FigDocument::from_doc(doc, BTreeMap::new());
        // The default open lands on the page.
        assert_eq!(document.doc.active_page(), Some(page_root));

        apply_scope(&mut document, FigScope::Component(component));
        assert_eq!(document.doc.active_page(), Some(master_root));

        // Restoring after a reload/merge swap keeps the component scope.
        let (doc2, _, _, _) = {
            let (d, p, c, m) = doc_with_page_and_component();
            (d, p, c, m)
        };
        drop(doc2);
        assert!(document.restore_active_root(master_root));
        assert_eq!(document.doc.active_page(), Some(master_root));
    }

    #[test]
    fn page_scope_activates_that_page() {
        let (doc, page_root, _, _) = doc_with_page_and_component();
        let mut document = FigDocument::from_doc(doc, BTreeMap::new());
        document.doc.set_active_page(None);
        apply_scope(&mut document, FigScope::Page(page_root));
        assert_eq!(document.doc.active_page(), Some(page_root));

        // A vanished page is ignored — the document keeps its default view.
        apply_scope(&mut document, FigScope::Page(NodeId::new()));
        assert_eq!(document.doc.active_page(), Some(page_root));
    }

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
        assert!(!is_relevant_project_change(
            root,
            Path::new("/tmp/project/fnx.d.ts")
        ));
        assert!(!is_relevant_project_change(
            root,
            Path::new("/tmp/project/.prettierrc.json")
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

    /// The active page is runtime state the project tree does not persist, so
    /// loading must default it — the creation tools parent new nodes into
    /// `doc.active_page()`, and with it unset every new shape/text/frame lands
    /// at the scene root where the page-scoped canvas render never paints it
    /// (invisible until a drag reparents it into the page).
    #[test]
    fn from_doc_defaults_the_active_page_to_the_default_page() {
        let mut doc = doc_with_one_page();
        let expected = doc.pages().first().copied();
        // The project reader strips presence state, so a loaded doc arrives
        // with no active page even though `add_page` set one in memory.
        doc.set_active_page(None);
        let document = FigDocument::from_doc(doc, BTreeMap::new());
        assert_eq!(document.doc.active_page(), expected);
        assert!(document.doc.active_page().is_some());
    }

    /// An already-set active page (e.g. carried by a `.fig` import) wins over
    /// the default.
    #[test]
    fn from_doc_keeps_an_existing_active_page() {
        use fanta_doc::{CanvasNode, GroupNode, NodeData};
        let mut doc = doc_with_one_page();
        let mut second = CanvasNode::new(NodeData::Group(GroupNode::default()));
        second.name = "Page 2".to_owned();
        let second_root = second.id;
        doc.apply(Operation::create_node(second))
            .expect("create second page root");
        doc.add_page(second_root);
        doc.set_active_page(Some(second_root));
        let document = FigDocument::from_doc(doc, BTreeMap::new());
        assert_eq!(document.doc.active_page(), Some(second_root));
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
        let item = cx.new(|_| FigItem {
            path: ProjectPath {
                worktree_id: WorktreeId::from_usize(0),
                path: RelPath::empty_arc(),
            },
            abs_path,
            entry_id: None,
            document: FigDocumentState::Ready(FigDocument::from_doc(doc, BTreeMap::new())),
            project_root,
            dirty: false,
            preview_dirty_before: None,
            conflict: false,
            source_edit_locked: false,
            source_edit_pipeline_in_progress: false,
            suppress_watcher_until: None,
            merge_base: None,
            pending_scope: None,
            last_scope: None,
            sync_epoch: 0,
            reload_task: None,
            _load_task: None,
            project_subscriptions: Vec::new(),
        });
        item.update(cx, |item, cx| item.subscribe_to_project(project, cx));
        item
    }

    #[gpui::test]
    async fn source_pipeline_is_serialized_across_split_views(cx: &mut TestAppContext) {
        let project = empty_project(cx).await;
        let item = ready_item(
            &project,
            PathBuf::from("/tmp/design/fanta.json"),
            Some(PathBuf::from("/tmp/design")),
            doc_with_one_page(),
            cx,
        );

        assert!(item.update(cx, |item, _| item.try_begin_source_edit_pipeline()));
        assert!(!item.update(cx, |item, _| item.try_begin_source_edit_pipeline()));
        item.update(cx, |item, _| item.finish_source_edit_pipeline());
        assert!(item.update(cx, |item, _| item.try_begin_source_edit_pipeline()));
        item.update(cx, |item, _| item.finish_source_edit_pipeline());
    }

    #[gpui::test]
    async fn source_edit_adoption_preserves_page_and_selection(cx: &mut TestAppContext) {
        let project = empty_project(cx).await;
        let mut document = doc_with_one_page();
        let page = document.pages()[0];
        document.selection.select_only(page);
        document.viewport = Viewport {
            center: [275.0, -42.0],
            zoom: 2.5,
        };
        let item = ready_item(
            &project,
            PathBuf::from("/tmp/design/fanta.json"),
            Some(PathBuf::from("/tmp/design")),
            document.clone(),
            cx,
        );
        document.scene.get_mut(page).expect("page node").name = "Edited source".to_owned();
        let source_edit = fanta_format::ProjectSourceEdit {
            document,
            assets: BTreeMap::new(),
            source_path: PathBuf::from("/tmp/design/pages/page/page.fnx"),
        };

        item.update(cx, |item, cx| item.adopt_source_edit(source_edit, cx));
        item.read_with(cx, |item, _| {
            let document = item.document().expect("adopted source document");
            assert_eq!(document.doc.active_page(), Some(page));
            assert_eq!(document.doc.selection.as_slice(), &[page]);
            assert_eq!(
                document.doc.viewport,
                Viewport {
                    center: [275.0, -42.0],
                    zoom: 2.5,
                }
            );
            assert_eq!(document.doc.scene.get(page).unwrap().name, "Edited source");
            assert!(!item.is_dirty());
        });
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

    #[gpui::test]
    async fn render_generation_tracks_content_but_not_selection_changes(cx: &mut TestAppContext) {
        let project = empty_project(cx).await;
        let item = ready_item(
            &project,
            PathBuf::from("/tmp/nowhere/Design.fig"),
            None,
            doc_with_one_page(),
            cx,
        );

        assert_eq!(
            item.read_with(cx, |item, _| item
                .document()
                .expect("ready")
                .render_generation()),
            0
        );
        item.update(cx, |item, cx| {
            item.with_document(cx, |_| ((), DocChange::Selection));
        });
        assert_eq!(
            item.read_with(cx, |item, _| item
                .document()
                .expect("ready")
                .render_generation()),
            0
        );
        item.update(cx, |item, cx| {
            item.with_document(cx, |_| ((), DocChange::ContentPreview));
        });
        assert_eq!(
            item.read_with(cx, |item, _| item
                .document()
                .expect("ready")
                .render_generation()),
            1
        );
        item.update(cx, |item, cx| {
            item.with_document(cx, |_| ((), DocChange::Content));
        });
        assert_eq!(
            item.read_with(cx, |item, _| item
                .document()
                .expect("ready")
                .render_generation()),
            2
        );
    }

    #[gpui::test]
    async fn canceling_a_content_preview_restores_the_prior_dirty_state(cx: &mut TestAppContext) {
        let project = empty_project(cx).await;
        let item = ready_item(
            &project,
            PathBuf::from("/tmp/nowhere/Design.fig"),
            None,
            doc_with_one_page(),
            cx,
        );

        item.update(cx, |item, cx| {
            item.with_document(cx, |_| ((), DocChange::ContentPreview));
            assert!(item.dirty);
            item.finish_content_preview(false, cx);
            assert!(!item.dirty);

            item.with_document(cx, |_| ((), DocChange::Content));
            assert!(item.dirty);
            item.with_document(cx, |_| ((), DocChange::ContentPreview));
            item.finish_content_preview(false, cx);
            assert!(item.dirty);
        });
    }

    /// A second window opens the same project directory through its OWN
    /// `Project` entity and reuses the shared item. The item must watch that
    /// project's worktree events too — otherwise external-edit detection
    /// (reload/merge/conflict) dies with the first window's project, and the
    /// next save clobbers newer disk state.
    #[gpui::test]
    async fn a_second_windows_open_watches_its_project_for_external_edits(
        cx: &mut TestAppContext,
    ) {
        use project::ProjectItem as _;

        async fn open_via_new_project(
            root: &Path,
            cx: &mut TestAppContext,
        ) -> (Entity<Project>, Entity<FigItem>) {
            let file_system = Arc::new(fs::RealFs::new(None, cx.executor()));
            let project = Project::test(file_system, [root], cx).await;
            let worktree_id = project.update(cx, |project, cx| {
                project
                    .worktrees(cx)
                    .next()
                    .expect("project worktree")
                    .read(cx)
                    .id()
            });
            let path = ProjectPath {
                worktree_id,
                path: util::rel_path::rel_path("fanta.json").into(),
            };
            let item = cx
                .update(|cx| FigItem::try_open(&project, &path, cx))
                .expect("fanta.json opens as a FigItem")
                .await
                .expect("open FigItem");
            cx.run_until_parked();
            (project, item)
        }

        init_test(cx);
        cx.executor().allow_parking();
        let temporary = tempfile::tempdir().expect("temporary project");
        write_project(temporary.path(), &doc_with_one_page(), &BTreeMap::new())
            .expect("write project tree");

        let (first_project, first_item) = open_via_new_project(temporary.path(), cx).await;
        let (second_project, second_item) = open_via_new_project(temporary.path(), cx).await;
        assert_eq!(
            second_item.entity_id(),
            first_item.entity_id(),
            "the second window shares the project's one item"
        );
        second_item.read_with(cx, |item, _| {
            assert_eq!(
                item.project_subscriptions.len(),
                2,
                "the shared item watches both windows' projects"
            );
        });

        // Reopening through an already-watched project adds no duplicate.
        let worktree_id = second_project.update(cx, |project, cx| {
            project
                .worktrees(cx)
                .next()
                .expect("project worktree")
                .read(cx)
                .id()
        });
        let reopened = cx
            .update(|cx| {
                FigItem::try_open(
                    &second_project,
                    &ProjectPath {
                        worktree_id,
                        path: util::rel_path::rel_path("fanta.json").into(),
                    },
                    cx,
                )
            })
            .expect("fanta.json opens as a FigItem")
            .await
            .expect("reopen FigItem");
        cx.run_until_parked();
        assert_eq!(reopened.entity_id(), first_item.entity_id());
        second_item.read_with(cx, |item, _| {
            assert_eq!(
                item.project_subscriptions.len(),
                2,
                "reopening dedupes per project entity"
            );
        });

        // The first window closes (its project drops); an external edit
        // reported by the SECOND project must still reach the item —
        // observable as the conflict flag while an FNX buffer is locked.
        drop(first_project);
        second_item.update(cx, |item, cx| item.set_source_edit_locked(true, cx));
        let changes: UpdatedEntriesSet = vec![(
            util::rel_path::rel_path("pages/some-page/page.fnx").into(),
            ProjectEntryId::from_proto(1),
            PathChange::Updated,
        )]
        .into();
        second_project.update(cx, |_, cx| {
            cx.emit(project::Event::WorktreeUpdatedEntries(worktree_id, changes));
        });
        cx.run_until_parked();
        second_item.read_with(cx, |item, _| {
            assert!(
                item.has_conflict(),
                "the second project's worktree events must reach the shared item"
            );
        });
    }
}
