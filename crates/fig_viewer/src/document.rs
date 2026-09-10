//! The document layer of the Figma viewer: loading `.fig` files and Fanta
//! projects into a [`Doc`], tracking edits, and persisting them back to disk
//! as an unwrapped `fanta-project` directory.

use std::{
    collections::{BTreeMap, HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use anyhow::{Context as _, Result};
use fanta_doc::{AssetId, Doc, NodeId, Operation, Viewport};
use fanta_fig_interop::{fig_to_doc, read_fig};
use fanta_render::{AssetResolver, DecodedImage, asset::LazyAssetResolver, solve_scene_layout};
use futures::FutureExt as _;
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

/// Cap on the encoded bytes of one image ingested as a project asset (a paste,
/// a drop, an agent's `create_image`). Generous for any generated PNG while
/// keeping a bad payload from ballooning the document — and enforced before
/// decoding, since a decoder handed arbitrary bytes can allocate far more
/// than it was given.
pub(crate) const MAX_IMAGE_SOURCE_BYTES: usize = 32 * 1024 * 1024;

#[derive(Default)]
struct ProjectWrites {
    state: Mutex<ProjectWriteState>,
    #[cfg(test)]
    next_save_barrier: Mutex<Option<SaveTestBarrier>>,
}

#[cfg(test)]
struct SaveTestBarrier {
    reached: futures::channel::oneshot::Sender<()>,
    resume: futures::channel::oneshot::Receiver<()>,
    after_write: bool,
}

#[cfg(test)]
impl SaveTestBarrier {
    async fn wait(self) {
        self.reached.send(()).expect("save test observes writer");
        self.resume.await.expect("save test releases writer");
    }
}

#[derive(Default)]
struct ProjectWriteState {
    active: usize,
    changing_destination: bool,
    quiet_until: Option<Instant>,
    last_write: Option<futures::future::Shared<futures::future::BoxFuture<'static, ()>>>,
}

struct ProjectWriteLease {
    writes: Arc<ProjectWrites>,
    changing_destination: bool,
    predecessor: Option<futures::future::Shared<futures::future::BoxFuture<'static, ()>>>,
    completed: Option<futures::channel::oneshot::Sender<()>>,
}

impl ProjectWrites {
    fn begin(self: &Arc<Self>) -> Arc<ProjectWriteLease> {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        self.enqueue(&mut state, false)
    }

    fn begin_destination_change(self: &Arc<Self>) -> Result<Arc<ProjectWriteLease>> {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        anyhow::ensure!(
            state.active == 0,
            "A save is already running. Try Save As again when it finishes."
        );
        Ok(self.enqueue(&mut state, true))
    }

    fn enqueue(
        self: &Arc<Self>,
        state: &mut ProjectWriteState,
        changing_destination: bool,
    ) -> Arc<ProjectWriteLease> {
        let predecessor = state.last_write.take();
        let (completed, completion) = futures::channel::oneshot::channel();
        // Reserve order when the snapshot is captured, not when its task is
        // first polled. A canceled queued save must still wait for its
        // predecessor before letting subsequent writers touch the tree.
        state.last_write = Some(
            {
                let predecessor = predecessor.clone();
                async move {
                    if let Some(predecessor) = predecessor {
                        predecessor.await;
                    }
                    if completion.await.is_err() {
                        log::debug!("a project write lease ended without its completion signal");
                    }
                }
            }
            .boxed()
            .shared(),
        );
        state.active += 1;
        state.changing_destination |= changing_destination;
        Arc::new(ProjectWriteLease {
            writes: self.clone(),
            changing_destination,
            predecessor,
            completed: Some(completed),
        })
    }

    fn changing_destination(&self) -> bool {
        self.state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .changing_destination
    }

    fn suppresses_watcher(&self, now: Instant) -> bool {
        let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state.active > 0 || state.quiet_until.is_some_and(|until| now < until)
    }
}

impl ProjectWriteLease {
    async fn wait_for_turn(&self) {
        if let Some(predecessor) = self.predecessor.clone() {
            predecessor.await;
        }
    }
}

impl Drop for ProjectWriteLease {
    fn drop(&mut self) {
        let mut state = self
            .writes
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        state.active = state.active.saturating_sub(1);
        if self.changing_destination {
            state.changing_destination = false;
        }
        if let Some(completed) = self.completed.take()
            && completed.send(()).is_err()
        {
            log::debug!("project write completion no longer has a waiting queue");
        }
        if state.active == 0 {
            state.last_write = None;
        }
        state.quiet_until = Some(Instant::now() + SELF_WRITE_SUPPRESS_WINDOW);
    }
}

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
    /// Ignore worktree events until this instant; set around our own project
    /// writes so saving from the canvas does not trigger a self-reload.
    suppress_watcher_until: Option<Instant>,
    // A synchronous writer cannot be interrupted when its foreground task is
    // canceled. Its shared lease keeps watcher suppression alive until both
    // sides have released that save, including overlapping or failed saves.
    project_writes: Arc<ProjectWrites>,
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
    /// Per-design projection memo from the last project write, so an
    /// autosave re-prints only the designs that changed. Taken by the
    /// in-flight save and handed back when it completes; a save that finds
    /// it absent starts from an empty one. Dropped whenever the document or
    /// its project root is replaced.
    write_cache: Option<fanta_format::ProjectWriteCache>,
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
    /// The document finished (re)loading, or Save As replaced its
    /// persisted state. Listeners treat this as "the document may have been
    /// replaced": in-flight text sessions, inspector previews and rename
    /// gestures are dropped. Ordinary persistence deliberately emits
    /// [`Saved`](Self::Saved) instead, so it never cancels what the user is
    /// doing.
    StateChanged,
    /// A document snapshot was written without replacing the live document.
    /// Newer edits, if any, remain dirty and also emit [`Edited`](Self::Edited).
    Saved,
    /// The document was replaced by an external reload (`merged: false`) or by
    /// a clean three-way merge of the external edit into the canvas's unsaved
    /// edits (`merged: true`). Only the watcher-driven paths emit this; the
    /// user's own discard-and-reload does not, so it never reads as somebody
    /// else's change.
    ReloadedFromDisk { merged: bool },
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

/// Whether a save came from the user (Cmd-S, File > Save) or from the
/// canvas's debounced autosave. Both announce [`FigItemEvent::Saved`], which
/// listeners must not treat as a reload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveKind {
    Explicit,
    Auto,
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
    /// The load-time resolver behind [`asset_resolver`](Self::asset_resolver):
    /// decodes an embedded image the first time it is drawn and keeps the
    /// decoded pixels under a byte budget, so opening a document costs the
    /// encoded bytes, not the RGBA of every image on every page. Kept
    /// concretely so the loader can prewarm the opening page's images.
    embedded_assets: Option<Arc<LazyAssetResolver>>,
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
    /// Moves when the variable registry or the active modes may have changed:
    /// every committed edit, undo, redo, and a variables-panel preview. Drawn
    /// from a process-wide counter so no two documents ever share a value —
    /// the canvas memoizes the (expensive) registry hash on it across a
    /// document swap. Unlike `render_generation` it does NOT move for a drag
    /// or text-edit preview frame.
    variables_generation: u64,
    /// Page roots whose images have been (or are being) decoded ahead of
    /// their first frame; each page is prewarmed once per document.
    prewarmed_pages: HashSet<NodeId>,
}

/// The decode work for one page's images, handed out by
/// [`FigDocument::take_page_prewarm`] to run off the UI thread.
pub(crate) struct PagePrewarm {
    resolver: Arc<LazyAssetResolver>,
    assets: Vec<AssetId>,
}

impl PagePrewarm {
    /// Decode the page's images into the shared resolver. Safe to run on any
    /// thread; a frame that draws one of them first simply wins the decode.
    ///
    /// Decodes one asset at a time, stopping as soon as this task is the last
    /// owner of the resolver. The document that owns it can be replaced while
    /// the walk runs — a reload or a merge installs a brand-new `FigDocument`
    /// with a brand-new resolver — and from that moment nothing will ever read
    /// what this decodes, while the orphaned cache would still fill to the full
    /// decode budget alongside the replacement's own decode of the same images.
    pub(crate) fn run(self) {
        for asset in self.assets {
            if Arc::strong_count(&self.resolver) <= 1 {
                return;
            }
            self.resolver.prewarm([asset]);
        }
    }
}

fn next_variables_generation() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
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
        let started = Instant::now();
        fanta_doc::strip_redundant_instance_overrides(&mut doc.scene, &doc.components);
        crate::report_slow("document load: strip redundant overrides", started);
        // Legacy migration: projects saved before vectors carried an SVG viewport
        // get one inferred from geometry, so a stroke thickened past the box is
        // clipped like a freshly imported file. A no-op once the doc is viewport-aware.
        let started = Instant::now();
        fanta_doc::backfill_vector_viewports(&mut doc.scene);
        crate::report_slow("document load: backfill vector viewports", started);
        let visible_page_roots = visible_page_roots(&doc);
        let default_page_root = default_page_root(&doc, &visible_page_roots);
        let uses_auto_layout = scene_uses_auto_layout(&doc.scene);
        let mut solved_pages = HashSet::new();
        if let Some(page_root) = default_page_root {
            let started = Instant::now();
            solve_scene_layout(&mut doc.scene, page_root);
            crate::report_slow("document load: solve default page layout", started);
            solved_pages.insert(page_root);
        }
        let started = Instant::now();
        let gpui_images = decode_gpui_images(&raw_assets);
        crate::report_slow("document load: gpui thumbnails", started);
        let raw_assets = Arc::new(raw_assets);
        let embedded_assets = (!raw_assets.is_empty()).then(|| {
            Arc::new(LazyAssetResolver::new(
                raw_assets.clone(),
                Arc::new(decode_embedded_image),
            ))
        });
        let asset_resolver = embedded_assets
            .clone()
            .map(|resolver| resolver as Arc<dyn AssetResolver>);
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
        // page). Imports can instead arrive with a hidden library page active,
        // because add_page activates the first root. Rendering that root hides
        // every new shape even though the default viewport fits a visible page.
        if doc.active_page().is_none_or(|active| {
            pages
                .iter()
                .any(|page| page.root == Some(active) && page.hidden)
        }) {
            doc.set_active_page(pages.get(default_page_index).and_then(|page| page.root));
        }

        Self {
            doc,
            pages,
            default_page_index,
            solved_pages,
            asset_resolver,
            embedded_assets,
            raw_assets,
            gpui_images,
            agent_asset_overlay: None,
            uses_auto_layout,
            render_generation: 0,
            variables_generation: next_variables_generation(),
            prewarmed_pages: HashSet::new(),
        }
    }

    pub(crate) fn variables_generation(&self) -> u64 {
        self.variables_generation
    }

    /// Record that the variable registry or active modes may have changed, so
    /// the canvas re-hashes them. Called on every committed edit (cheap next
    /// to the edit) and by variable previews, which are the only transient
    /// writes that touch the registry.
    pub(crate) fn mark_variables_changed(&mut self) {
        self.variables_generation = next_variables_generation();
    }

    pub(crate) fn render_generation(&self) -> u64 {
        self.render_generation
    }

    /// Decode the images the opening page draws, so its first frame does not
    /// stall on decodes. Runs on the background load thread; every other
    /// page's images are prewarmed when the page is first activated (see
    /// [`Self::take_page_prewarm`]).
    pub(crate) fn prewarm_default_page_assets(&mut self) {
        let Some(page_root) = self
            .pages
            .get(self.default_page_index)
            .and_then(|page| page.root)
        else {
            return;
        };
        if let Some(prewarm) = self.take_page_prewarm(page_root) {
            let started = Instant::now();
            prewarm.run();
            crate::report_slow("document load: prewarm default page images", started);
        }
    }

    /// The decode work for `page_root`'s images the first time that page is
    /// activated, or `None` when the page was already prewarmed (or there are
    /// no embedded assets). Without this, a page switch decodes every image
    /// the page draws serially inside its first frame on the render thread.
    /// The caller runs the result on a background thread.
    pub(crate) fn take_page_prewarm(&mut self, page_root: NodeId) -> Option<PagePrewarm> {
        let resolver = self.embedded_assets.clone()?;
        if !self.prewarmed_pages.insert(page_root) {
            return None;
        }
        let assets = page_image_assets(&self.doc, page_root);
        if assets.is_empty() {
            return None;
        }
        Some(PagePrewarm { resolver, assets })
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
        anyhow::ensure!(
            bytes.len() <= MAX_IMAGE_SOURCE_BYTES,
            "the image is {} bytes; the limit is {MAX_IMAGE_SOURCE_BYTES} bytes",
            bytes.len()
        );
        let decoded = image::load_from_memory(&bytes).context("decoding image bytes")?;
        let rgba = decoded.to_rgba8();
        let (width, height) = rgba.dimensions();
        anyhow::ensure!(width > 0 && height > 0, "the image has no pixels");
        let id = AssetId::new();

        // `raw_assets` is shared behind an `Arc` with the load-time asset
        // resolver, so ingesting usually clones the byte map (the resolver
        // keeps serving the map it was built over; the new image reaches it
        // through the overlay below). Fine for occasional agent placements;
        // batch imports should get a shared-bytes representation first.
        Arc::make_mut(self.raw_assets).insert(id, bytes.clone());

        let overlay = self.overlay.get_or_insert_with(|| {
            Arc::new(OverlayAssetResolver {
                base: self.asset_resolver.clone(),
                added: std::sync::RwLock::new(HashMap::default()),
            })
        });
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
            Arc::make_mut(self.raw_assets).remove(&id);
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
                                        let materialized =
                                            materialize_project_on_open(&load_path, &document);
                                        (document, materialized)
                                    }),
                                }
                            })
                            .await;

                        let (document, materialized) = match load_result {
                            Ok((document, materialized)) => (Ok(document), materialized),
                            Err(error) => (Err(error), None),
                        };
                        if let Err(error) = this.update(cx, |this: &mut FigItem, cx| {
                            this.adopt_initial_load(document, materialized, cx);
                        }) {
                            log::debug!("dropping loaded update for closed .fig item: {error:#}");
                            return;
                        }

                        // Surface the project as a visible worktree so its
                        // `fanta.json` / `.fnx` / asset files show in the
                        // project panel — the editor is now launched "from the
                        // folder". This runs for EVERY successful load, not
                        // just the one that materialized a new directory:
                        // opening a `.fig` that already had a sibling project,
                        // or a project source file from outside any worktree,
                        // otherwise leaves the workspace with no open project
                        // at all — and then the Agent Panel refuses to start a
                        // thread, and the disk-sync watcher (which only sees
                        // events for paths inside an open worktree) never
                        // reports an external edit. Never fails the open.
                        let project_root = match this
                            .read_with(cx, |this, _| this.project_root.clone())
                        {
                            Ok(project_root) => project_root,
                            Err(error) => {
                                log::debug!(
                                    "skipping the worktree check for a closed .fig item: {error:#}"
                                );
                                return;
                            }
                        };
                        if let Some(root) = project_root
                            && let Some(project) = project.upgrade()
                        {
                            // A project nested inside an already-open folder
                            // is reachable through that worktree; adding a
                            // second root for it would clutter the project
                            // panel with a duplicate tree.
                            let already_visible = project.read_with(cx, |project, cx| {
                                project
                                    .visible_worktrees(cx)
                                    .any(|worktree| root.starts_with(worktree.read(cx).abs_path()))
                            });
                            if !already_visible {
                                let worktree = project.update(cx, |project, cx| {
                                    project.find_or_create_worktree(root.clone(), true, cx)
                                });
                                if let Err(error) = worktree.await {
                                    log::error!(
                                        "adding Fanta project {} to the workspace failed: {error:#}",
                                        root.display()
                                    );
                                }
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
                    suppress_watcher_until: None,
                    project_writes: Arc::default(),
                    merge_base: None,
                    pending_scope: initial_scope,
                    last_scope: None,
                    sync_epoch: 0,
                    reload_task: None,
                    _load_task: Some(load_task),
                    write_cache: None,
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

    /// Test-only. Nothing in the app locks the canvas any more: the source view
    /// is read-only, so there is no unsaved buffer to protect the document from.
    /// The guards that read the flag are still live code, and these tests are
    /// what keeps them honest if source editing ever comes back.
    #[cfg(test)]
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
    pub(crate) fn subscribe_to_project(
        &mut self,
        project: &Entity<Project>,
        cx: &mut Context<Self>,
    ) {
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
        let now = Instant::now();
        if self.project_writes.suppresses_watcher(now)
            || self.suppress_watcher_until.is_some_and(|until| now < until)
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
                    cx.emit(FigItemEvent::ReloadedFromDisk { merged: false });
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
                        cx.emit(FigItemEvent::ReloadedFromDisk { merged: true });
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
    fn adopt_merged_document(&mut self, merged: Doc, disk: FigDocument, cx: &mut Context<Self>) {
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
            self.document
                .ready()
                .map(|current| current.render_generation()),
        );
        self.document = FigDocumentState::Ready(document);
        self.merge_base = Some(disk.doc);
        self.write_cache = None;
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
                    Ok(document) => {
                        this.apply_reloaded_document(document, cx);
                        cx.emit(FigItemEvent::ReloadedFromDisk { merged: false });
                    }
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
            self.document
                .ready()
                .map(|current| current.render_generation()),
        );
        self.merge_base = Some(document.doc.clone());
        self.document = FigDocumentState::Ready(document);
        self.write_cache = None;
        self.dirty = false;
        self.preview_dirty_before = None;
        self.sync_epoch += 1;
        self.set_conflict(false, cx);
        cx.emit(FigItemEvent::StateChanged);
        cx.notify();
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
        document.mark_variables_changed();
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
                document.mark_variables_changed();
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
            document.mark_variables_changed();
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
            document.mark_variables_changed();
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

    /// Install the initial load's outcome: the parsed document and, for a bare
    /// `.fig`, the project directory the load materialized next to it. The
    /// materializing write's memo becomes the item's write cache, so the first
    /// autosave re-prints only what changed instead of starting cold and
    /// re-projecting every page. A no-op when an external change already
    /// installed a newer document while the load ran.
    fn adopt_initial_load(
        &mut self,
        document: Result<FigDocument>,
        materialized: Option<MaterializedProject>,
        cx: &mut Context<Self>,
    ) {
        if self.sync_epoch != 0 {
            // The project root was known from open, so the watcher was live
            // and reloaded a newer document; installing this older snapshot
            // would regress it and poison merge_base for the next save.
            return;
        }
        self.write_cache = None;
        if let Some(MaterializedProject { root, write_cache }) = materialized {
            // The materializing write echoes back through the worktree
            // watcher once the folder is adopted; suppress it exactly like a
            // save's self-write.
            self.suppress_watcher_until = Some(Instant::now() + SELF_WRITE_SUPPRESS_WINDOW);
            self.project_root = Some(root.clone());
            self.write_cache = Some(write_cache);
            // The project directory now exists; share this item so later
            // scoped opens reuse it.
            let key = root.canonicalize().unwrap_or(root);
            register_shared_project_item(key, &cx.entity(), cx);
        }
        self.document = FigDocumentState::from_result(document);
        if let Some(scope) = self.pending_scope.take()
            && let FigDocumentState::Ready(document) = &mut self.document
        {
            apply_scope(document, scope);
            self.last_scope = Some(scope);
            cx.emit(FigItemEvent::ScopeApplied(scope, ScopeRequester::Load));
        }
        self.merge_base = self.document.ready().map(|document| document.doc.clone());
        cx.emit(FigItemEvent::StateChanged);
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
    ///
    /// A save persists its snapshot without replacing the live document.
    /// Newer edits remain dirty and rearm autosave when the write completes.
    pub fn save(
        &mut self,
        kind: SaveKind,
        cx: &mut Context<Self>,
    ) -> Task<Result<Option<PathBuf>>> {
        if self.project_writes.changing_destination() {
            return Task::ready(match kind {
                SaveKind::Auto => Ok(None),
                SaveKind::Explicit => Err(anyhow::anyhow!(
                    "Save As is still running. Wait for it to finish before saving again."
                )),
            });
        }
        if self.source_edit_locked {
            return Task::ready(Err(anyhow::anyhow!(
                "the FNX source is dirty; save it through the code workspace before saving the canvas"
            )));
        }
        let Some(document) = self.document.ready() else {
            return Task::ready(Err(anyhow::anyhow!("the document is still loading")));
        };

        // The write needs the content, not the presence state: the undo
        // stack's subtree snapshots can outweigh the scene, and cloning them
        // once per autosave was the save path's memory high-water mark.
        let doc = document.doc.clone_for_persist();
        let raw_assets = document.raw_assets.clone();
        let generation = document.render_generation();
        // Our own writes echo back through the worktree watcher; suppress it
        // both from save start (covers sub-second saves entirely) and again at
        // completion (covers watcher latency after longer saves). On the
        // materializing save this window also spans the moment the directory is
        // adopted as a worktree, so its initial scan does not bounce back as a
        // reload.
        let write_lease = self.project_writes.begin();
        self.suppress_watcher_until = Some(Instant::now() + SELF_WRITE_SUPPRESS_WINDOW);
        // A pending merge/reload holds a disk snapshot from before this save;
        // cancel it so it can't adopt that stale snapshot after the write
        // lands (silently reverting — and on the next save destroying — the
        // content saved here).
        self.reload_task = None;
        #[cfg(test)]
        let save_barrier = self
            .project_writes
            .next_save_barrier
            .lock()
            .expect("save test barrier")
            .take();
        cx.spawn(async move |this, cx| {
            write_lease.wait_for_turn().await;
            let (target, materializing, write_cache) = this.update(cx, |this, _| {
                anyhow::ensure!(
                    !this.source_edit_locked,
                    "the FNX source is dirty; save it through the code workspace before saving the canvas"
                );
                anyhow::ensure!(this.document.ready().is_some(), "the document is still loading");
                let materializing = this.project_root.is_none();
                // An earlier queued save may have just materialized the
                // project. Resolve its adopted root only after that save ends.
                let target = this
                    .project_root
                    .clone()
                    .unwrap_or_else(|| available_project_dir(&this.abs_path));
                anyhow::ensure!(
                    !materializing || !fanta_format::is_project_dir(&target),
                    "a Fanta project already exists at {}; open it directly instead of overwriting it with {}",
                    target.display(),
                    this.abs_path.display()
                );
                Ok::<_, anyhow::Error>((
                    target,
                    materializing,
                    this.write_cache.take().unwrap_or_default(),
                ))
            })??;
            // The persisted clone travels through the write and comes back
            // to become the merge base — one clone per save, not two.
            let (result, saved_doc, write_cache) = cx
                .background_spawn({
                    let target = target.clone();
                    let write_lease = write_lease.clone();
                    async move {
                        let _write_lease = write_lease;
                        let mut write_cache = write_cache;
                        #[cfg(test)]
                        let mut save_barrier = save_barrier;
                        #[cfg(test)]
                        if save_barrier.as_ref().is_some_and(|barrier| !barrier.after_write) {
                            save_barrier.take().expect("before-write barrier").wait().await;
                        }
                        let result =
                            write_project_cached(&target, &doc, &raw_assets, &mut write_cache);
                        #[cfg(test)]
                        if let Some(barrier) = save_barrier {
                            barrier.wait().await;
                        }
                        (result, doc, write_cache)
                    }
                })
                .await;
            this.update(cx, |this, cx| {
                this.suppress_watcher_until = Some(Instant::now() + SELF_WRITE_SUPPRESS_WINDOW);
                if result.is_ok() {
                    if materializing {
                        this.project_root = Some(target.clone());
                    }
                    // The disk contains this snapshot even if the live
                    // document advanced while the writer was running.
                    this.merge_base = Some(saved_doc);
                    this.dirty = this
                        .document
                        .ready()
                        .is_none_or(|document| document.render_generation() != generation);
                    if !this.dirty {
                        this.preview_dirty_before = None;
                    }
                    this.sync_epoch += 1;
                    this.set_conflict(false, cx);
                    cx.emit(FigItemEvent::Saved);
                    if this.dirty {
                        // Other views may have consumed their debounce while
                        // this snapshot was queued or writing.
                        cx.emit(FigItemEvent::Edited);
                    }
                    cx.notify();
                }
                // The memo is only worth keeping for the tree it describes; a
                // failed write leaves it valid too (entries are content
                // fingerprints, not disk state).
                if this.project_root.as_deref() == Some(target.as_path()) {
                    this.write_cache = Some(write_cache);
                }
            })?;
            drop(write_lease);
            result.map(|()| materializing.then_some(target))
        })
    }

    pub(crate) fn save_as(
        &mut self,
        project: Entity<Project>,
        destination: ProjectPath,
        cx: &mut Context<Self>,
    ) -> Task<Result<()>> {
        let preparation = (|| {
            anyhow::ensure!(
                !self.source_edit_locked,
                "Save or discard the current FNX edit before using Save As."
            );
            let document = self
                .document
                .ready()
                .context("The design is still loading.")?;
            let target = project
                .read(cx)
                .absolute_path(&destination, cx)
                .context("Save As requires a local destination folder.")?;
            let manifest_path = ProjectPath {
                worktree_id: destination.worktree_id,
                path: destination
                    .path
                    .join(util::rel_path::RelPath::unix("fanta.json")?),
            };
            let lease = self.project_writes.begin_destination_change()?;
            Ok((
                target,
                manifest_path,
                document.doc.clone_for_persist(),
                document.raw_assets.clone(),
                document.render_generation(),
                lease,
            ))
        })();
        let (target, manifest_path, document, assets, generation, lease) = match preparation {
            Ok(preparation) => preparation,
            Err(error) => {
                if self.dirty {
                    cx.emit(FigItemEvent::Edited);
                    cx.notify();
                }
                return Task::ready(Err(error));
            }
        };
        let previous_root = self.project_root.clone();
        self.reload_task = None;
        self.sync_epoch += 1;
        cx.spawn(async move |this, cx| {
            let result: Result<()> = async {
                let (root, document, cache) = cx
                    .background_spawn({
                        let lease = lease.clone();
                        async move {
                            let _lease = lease;
                            let (root, cache) = write_project_copy(
                                &target,
                                previous_root.as_deref(),
                                &document,
                                &assets,
                            )?;
                            anyhow::Ok((root, document, cache))
                        }
                    })
                    .await?;
                let projects = this.update(cx, |this, cx| {
                    this.entry_id = project
                        .read(cx)
                        .entry_for_path(&manifest_path, cx)
                        .map(|entry| entry.id);
                    this.path = manifest_path;
                    this.abs_path = root.join("fanta.json");
                    this.project_root = Some(root.clone());
                    this.merge_base = Some(document);
                    this.write_cache = Some(cache);
                    this.dirty = this
                        .document
                        .ready()
                        .is_none_or(|document| document.render_generation() != generation);
                    this.preview_dirty_before = None;
                    this.sync_epoch += 1;
                    this.set_conflict(false, cx);
                    this.suppress_watcher_until = Some(Instant::now() + SELF_WRITE_SUPPRESS_WINDOW);
                    this.subscribe_to_project(&project, cx);
                    // All split/scoped views share this document. Remove its old
                    // lookup keys so reopening the original reads the original tree.
                    let entity_id = cx.entity_id();
                    cx.default_global::<SharedProjectItems>()
                        .0
                        .retain(|_, item| item.entity_id() != entity_id);
                    let key = root.canonicalize().unwrap_or_else(|_| root.clone());
                    register_shared_project_item(key, &cx.entity(), cx);
                    cx.emit(FigItemEvent::StateChanged);
                    if this.dirty {
                        cx.emit(FigItemEvent::Edited);
                    }
                    cx.notify();
                    this.project_subscriptions
                        .iter()
                        .filter_map(|(project, _)| project.upgrade())
                        .collect::<Vec<_>>()
                })?;
                drop(lease);
                // Other windows sharing this document need a watcher for its new
                // directory even after the window performing Save As closes.
                for project in projects {
                    let worktree = project.update(cx, |project, cx| {
                        project.find_or_create_worktree(root.clone(), true, cx)
                    });
                    if let Err(error) = worktree.await {
                        log::error!(
                            "adding saved Fanta project {} to the workspace failed: {error:#}",
                            root.display()
                        );
                    }
                }
                Ok(())
            }
            .await;
            if result.is_err() {
                // Save As consumed each view's autosave timer while it held
                // the destination lease. Re-arm them after releasing it so
                // a rejected copy does not silently disable normal saving.
                if let Err(error) = this.update(cx, |this, cx| {
                    if this.dirty {
                        cx.emit(FigItemEvent::Edited);
                        cx.notify();
                    }
                }) {
                    log::debug!("the design closed before autosave could resume: {error:#}");
                }
            }
            result
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
    ready_item_with_root_for_test(project, abs_path, None, doc, cx)
}

/// As [`ready_item_for_test`], with an already-materialized project root — the
/// state every save path after the first one runs in.
#[cfg(test)]
pub(crate) fn ready_item_with_root_for_test(
    project: &Entity<Project>,
    abs_path: PathBuf,
    project_root: Option<PathBuf>,
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
        project_root,
        dirty: false,
        preview_dirty_before: None,
        conflict: false,
        source_edit_locked: false,
        suppress_watcher_until: None,
        project_writes: Arc::default(),
        merge_base: None,
        pending_scope: None,
        last_scope: None,
        sync_epoch: 0,
        reload_task: None,
        _load_task: None,
        write_cache: None,
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

/// A project directory materialized while a bare `.fig` loaded, with the
/// write cache that write warmed: every design's projection is memoized in
/// it, so handing it to the item makes the first autosave incremental.
struct MaterializedProject {
    root: PathBuf,
    write_cache: fanta_format::ProjectWriteCache,
}

/// Materialize `document` (parsed from the bare `.fig` at `fig_path`) into a
/// project directory next to it, so the editor is project-backed and editable
/// from the first frame instead of leaving a folder to appear only on the
/// first save. Runs on the background load thread. `None` when a project has
/// appeared at the target since `try_open`'s redirect check (it is never
/// overwritten) or the write failed — the parse is still shown in-memory and
/// the first save retries the write.
fn materialize_project_on_open(
    fig_path: &Path,
    document: &FigDocument,
) -> Option<MaterializedProject> {
    let target = available_project_dir(fig_path);
    if fanta_format::is_project_dir(&target) {
        return None;
    }
    let created_here = !target.exists();
    let mut write_cache = fanta_format::ProjectWriteCache::default();
    let started = Instant::now();
    let written = write_project_cached(
        &target,
        &document.doc,
        &document.raw_assets,
        &mut write_cache,
    );
    crate::report_slow("document load: materialize project", started);
    match written {
        Ok(()) => Some(MaterializedProject {
            root: target,
            write_cache,
        }),
        Err(error) => {
            log::error!(
                "materializing Fanta project at {} on open failed: {error:#}",
                target.display()
            );
            // A half-written dir is already tagged as a project (the manifest
            // is scaffolded first), so leaving it would hijack every reopen
            // AND block the save that could repair it. Remove what we
            // created; the first save re-materializes.
            if created_here && let Err(error) = std::fs::remove_dir_all(&target) {
                log::error!(
                    "cleaning up partial Fanta project at {} failed: {error:#}",
                    target.display()
                );
            }
            None
        }
    }
}

pub(crate) fn write_project(
    root: &Path,
    doc: &Doc,
    raw_assets: &BTreeMap<AssetId, Vec<u8>>,
) -> Result<()> {
    write_project_cached(
        root,
        doc,
        raw_assets,
        &mut fanta_format::ProjectWriteCache::default(),
    )
}

fn write_project_copy(
    target: &Path,
    source: Option<&Path>,
    document: &Doc,
    assets: &BTreeMap<AssetId, Vec<u8>>,
) -> Result<(PathBuf, fanta_format::ProjectWriteCache)> {
    let requested_target = target.to_path_buf();
    let target = validate_project_copy_destination(target, source)?;
    let parent = target
        .parent()
        .context("Choose a destination folder with a parent directory.")?;
    // Publish only a complete project. A failed/cancelled background write
    // must not leave a half-written fanta.json that future opens would adopt.
    let staging = tempfile::Builder::new()
        .prefix(".fanta-save-as-")
        .tempdir_in(parent)?;
    let mut cache = fanta_format::ProjectWriteCache::default();
    write_project_cached(staging.path(), document, assets, &mut cache)?;
    std::fs::rename(staging.path(), &target)
        .with_context(|| format!("saving the copied design at {}", target.display()))?;
    Ok((requested_target, cache))
}

pub(crate) fn validate_project_copy_destination(
    target: &Path,
    source: Option<&Path>,
) -> Result<PathBuf> {
    let parent = target
        .parent()
        .context("Choose a destination folder with a parent directory.")?
        .canonicalize()?;
    let target = parent.join(
        target
            .file_name()
            .context("Choose a name for the copied design.")?,
    );
    if let Some(source) = source {
        match source.canonicalize() {
            Ok(source) => anyhow::ensure!(
                !target.starts_with(&source),
                "Choose a destination outside the original Fanta project."
            ),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error).context("checking the original project directory"),
        }
    }
    match std::fs::symlink_metadata(&target) {
        Ok(metadata) => {
            anyhow::ensure!(
                metadata.is_dir() && !metadata.file_type().is_symlink(),
                "{} already exists. Choose a new or empty folder.",
                target.display()
            );
            anyhow::ensure!(
                std::fs::read_dir(&target)?.next().is_none(),
                "{} is not empty. Choose a new or empty folder.",
                target.display()
            );
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error).with_context(|| format!("checking {}", target.display())),
    }
    Ok(target)
}

/// [`write_project`] reusing the per-design projection memoized in `cache`
/// by the previous write of this document, so a save after a small edit
/// re-prints only the designs that changed.
pub(crate) fn write_project_cached(
    root: &Path,
    doc: &Doc,
    raw_assets: &BTreeMap<AssetId, Vec<u8>>,
    cache: &mut fanta_format::ProjectWriteCache,
) -> Result<()> {
    fanta_format::scaffold_project_tree(root)
        .with_context(|| format!("scaffolding Fanta project at {}", root.display()))?;
    fanta_format::write_project_tree_cached(root, doc, raw_assets, cache)
        .with_context(|| format!("writing Fanta project at {}", root.display()))?;
    git_init_if_needed(root);
    Ok(())
}

/// Turn a freshly written project into a git repository, so that every later
/// canvas edit is a reviewable diff.
///
/// The check walks the ancestors, not just `root`: a nested repository created
/// inside an existing checkout would shadow the outer repo for `git_ui`, so the
/// design's changes would stop appearing in the history the user already has.
///
/// A missing `git`, or a `git init` that fails, is logged and swallowed: a save
/// must never fail because version control is unavailable.
fn git_init_if_needed(root: &Path) {
    if root.ancestors().any(|dir| dir.join(".git").exists()) {
        return;
    }
    let git_binary = project_git_binary(std::env::current_exe().ok().as_deref());
    // The macOS system Git can be an Xcode installation shim. Prefer the
    // bundled binary so a designer can create history without developer tools.
    #[allow(
        clippy::disallowed_methods,
        reason = "write_project is sync and only ever runs on background_spawn"
    )]
    let result = util::command::new_std_command(git_binary)
        .args(["init", "-q"])
        .current_dir(root)
        .output();
    match result {
        Ok(output) if output.status.success() => {}
        Ok(output) => log::warn!(
            "git init in {} exited with {}: {}",
            root.display(),
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ),
        Err(error) => log::warn!("running git init in {} failed: {error}", root.display()),
    }
}

fn project_git_binary(executable: Option<&Path>) -> PathBuf {
    executable
        .and_then(Path::parent)
        .map(|directory| directory.join(if cfg!(windows) { "git.exe" } else { "git" }))
        .filter(|binary| binary.is_file())
        .unwrap_or_else(|| PathBuf::from("git"))
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
        return fanta_format::is_project_dir(root)
            .then(|| (root.to_path_buf(), FigScope::Variables));
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
    let started = Instant::now();
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    crate::report_slow("document load: read .fig", started);
    let started = Instant::now();
    let fig = read_fig(&bytes).context("parsing .fig")?;
    crate::report_slow("document load: parse .fig", started);
    let started = Instant::now();
    let (doc, _report, assets) = fig_to_doc(&fig).context("mapping .fig to Fanta document")?;
    crate::report_slow("document load: map .fig to doc", started);
    let mut document = FigDocument::from_doc(doc, assets.into_iter().collect());
    document.prewarm_default_page_assets();
    Ok(document)
}

fn load_project_document(root: &Path) -> Result<FigDocument> {
    let started = Instant::now();
    let (doc, assets) = fanta_format::read_project_tree(root)
        .with_context(|| format!("reading Fanta project at {}", root.display()))?;
    crate::report_slow("document load: read project tree", started);
    let mut document = FigDocument::from_doc(doc, assets);
    document.prewarm_default_page_assets();
    Ok(document)
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

pub(crate) fn try_page_bounds(doc: &Doc, page_root: Option<NodeId>) -> Option<fanta_doc::Bounds> {
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

/// Decode one embedded asset to the straight-alpha RGBA8 the canvas renderer
/// draws. Installed in the document's [`LazyAssetResolver`], which calls it
/// the first time an image is drawn (or prewarmed) and remembers a failure
/// so a corrupt asset is logged once, not once per frame.
fn decode_embedded_image(asset_id: AssetId, bytes: &[u8]) -> Option<DecodedImage> {
    match image::load_from_memory(bytes) {
        Ok(image) => {
            let image = image.to_rgba8();
            let (width, height) = image.dimensions();
            Some(DecodedImage::new(Arc::new(image.into_raw()), width, height))
        }
        Err(error) => {
            log::warn!("failed to decode embedded .fig image asset {asset_id}: {error}");
            None
        }
    }
}

/// The image assets drawn somewhere under `page_root`: every bitmap's asset,
/// every video's poster frame, and every image paint in a fill, stroke, or
/// frame background. An instance draws its component master's subtree, which
/// lives on the (hidden) components page rather than under this one, so the
/// masters the page instantiates are walked too, along with the paints the
/// instances' overrides swap in. Other asset kinds (audio, 3D models, the
/// videos themselves) are not decoded as images.
fn page_image_assets(doc: &Doc, page_root: NodeId) -> Vec<AssetId> {
    use fanta_doc::{NodeData, OverrideValue};

    let mut assets = Vec::new();
    let mut pending_roots = vec![page_root];
    let mut visited_components = HashSet::new();
    while let Some(root) = pending_roots.pop() {
        for node_id in doc.scene.descendants_of(root) {
            let Some(node) = doc.scene.get(node_id) else {
                continue;
            };
            match &node.data {
                NodeData::Bitmap(bitmap) => assets.push(bitmap.asset),
                NodeData::Video(video) => assets.extend(video.poster),
                NodeData::Group(group) => {
                    assets.extend(group.background.iter().filter_map(image_fill_asset));
                    assets.extend(group.background_fills.iter().filter_map(image_fill_asset));
                    assets.extend(group.strokes.iter().filter_map(image_stroke_asset));
                }
                NodeData::Vector(vector) => {
                    assets.extend(vector.fills.iter().filter_map(image_fill_asset));
                    assets.extend(vector.strokes.iter().filter_map(image_stroke_asset));
                }
                NodeData::Boolean(boolean) => {
                    assets.extend(boolean.fills.iter().filter_map(image_fill_asset));
                    assets.extend(boolean.strokes.iter().filter_map(image_stroke_asset));
                }
                NodeData::Instance(instance) => {
                    for override_entry in &instance.overrides {
                        match &override_entry.value {
                            OverrideValue::Fills { fills } => {
                                assets.extend(fills.iter().filter_map(image_fill_asset));
                            }
                            OverrideValue::Strokes { strokes } => {
                                assets.extend(strokes.iter().filter_map(image_stroke_asset));
                            }
                            // A swap redirects a descendant of the expansion
                            // to a DIFFERENT master, whose subtree the page
                            // then draws.
                            OverrideValue::SwapInstance { component } => {
                                if visited_components.insert(*component)
                                    && let Some(def) = doc.components.defs.get(component)
                                {
                                    pending_roots.push(def.root);
                                }
                            }
                            OverrideValue::Text { .. }
                            | OverrideValue::Visible { .. }
                            | OverrideValue::Field { .. } => {}
                        }
                    }
                    for derived in &instance.derived {
                        assets.extend(derived.fills.iter().flatten().filter_map(image_fill_asset));
                    }
                    if visited_components.insert(instance.component)
                        && let Some(def) = doc.components.defs.get(&instance.component)
                    {
                        pending_roots.push(def.root);
                    }
                }
                NodeData::Text(_)
                | NodeData::Audio(_)
                | NodeData::NodeGraph(_)
                | NodeData::Model3d(_)
                | NodeData::AiArtifact(_)
                | NodeData::Embed(_) => {}
            }
        }
    }
    assets.sort();
    assets.dedup();
    assets
}

fn image_fill_asset(fill: &fanta_doc::Fill) -> Option<AssetId> {
    match fill {
        fanta_doc::Fill::Image { asset, .. } => Some(*asset),
        fanta_doc::Fill::Solid { .. } | fanta_doc::Fill::Gradient { .. } => None,
    }
}

fn image_stroke_asset(stroke: &fanta_doc::Stroke) -> Option<AssetId> {
    image_fill_asset(&stroke.paint)
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
            // The ENCODED bytes go to GPUI (not `decode_embedded_image`'s straight-alpha
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
    fn project_git_prefers_the_app_bundle_without_depending_on_system_tools() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let executable = directory.path().join("fanta");
        let git = directory
            .path()
            .join(if cfg!(windows) { "git.exe" } else { "git" });
        assert_eq!(project_git_binary(Some(&executable)), PathBuf::from("git"));
        std::fs::write(&git, b"bundled git")?;
        assert_eq!(project_git_binary(Some(&executable)), git);
        assert_eq!(project_git_binary(None), PathBuf::from("git"));
        Ok(())
    }

    #[test]
    fn a_page_prewarm_stops_once_the_document_that_owns_its_resolver_is_gone() {
        use fanta_render::DecodedImage;
        use std::sync::atomic::{AtomicUsize, Ordering};

        let decodes = Arc::new(AtomicUsize::new(0));
        let assets: Vec<AssetId> = (1..=4).map(AssetId::from_u128).collect();
        let encoded: BTreeMap<AssetId, Vec<u8>> =
            assets.iter().map(|id| (*id, vec![2u8; 4])).collect();
        let build = || {
            let decodes = decodes.clone();
            Arc::new(LazyAssetResolver::new(
                Arc::new(encoded.clone()),
                Arc::new(move |_id, _bytes: &[u8]| {
                    decodes.fetch_add(1, Ordering::SeqCst);
                    Some(DecodedImage::new(Arc::new(vec![0u8; 4]), 1, 1))
                }),
            ))
        };

        // A document swap drops the resolver's other owners before the task
        // runs: nothing will read what it decodes, so it must decode nothing.
        let orphaned = PagePrewarm {
            resolver: build(),
            assets: assets.clone(),
        };
        orphaned.run();
        assert_eq!(
            decodes.load(Ordering::SeqCst),
            0,
            "an orphaned prewarm must not fill a cache nobody will read"
        );

        // While the document is alive, the walk still decodes the whole page.
        let live = build();
        PagePrewarm {
            resolver: live.clone(),
            assets: assets.clone(),
        }
        .run();
        assert_eq!(decodes.load(Ordering::SeqCst), assets.len());
        drop(live);
    }

    /// A doc with one page and one component master (an ordinary group
    /// promoted via DefineComponent), for scope tests.
    fn doc_with_page_and_component() -> (Doc, NodeId, fanta_doc::ComponentId, NodeId) {
        use fanta_doc::{CanvasNode, ComponentDef, ComponentId, GroupNode, NodeData};
        let mut doc = Doc::new();
        let mut page = CanvasNode::new(NodeData::Group(GroupNode::default()));
        page.name = "Page 1".to_owned();
        let page_root = page.id;
        doc.apply(Operation::create_node(page))
            .expect("create page");
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
                preview_rev: 0,
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
        let page_path = root.join("pages").join(page.to_string()).join("page.fnx");
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
        assert_eq!(
            scoped_project_source(&root.join("doc").join("motion.json")),
            None
        );
        assert_eq!(
            scoped_project_source(&root.join("pages").join("not-an-id").join("page.fnx")),
            None
        );
        let outside = dir.path().join("not-a-project");
        std::fs::create_dir_all(outside.join("pages").join(page.to_string())).expect("mkdir");
        assert_eq!(
            scoped_project_source(
                &outside
                    .join("pages")
                    .join(page.to_string())
                    .join("page.fnx")
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

    fn prepared_video_fixture() -> Result<crate::generation_media::PreparedVideo> {
        let mut png = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            2,
            3,
            image::Rgba([34, 197, 94, 255]),
        ))
        .write_to(&mut png, image::ImageFormat::Png)?;
        Ok(crate::generation_media::PreparedVideo {
            bytes: Arc::from(&b"exact video source bytes"[..]),
            metadata: crate::generation_media::VideoMetadata {
                width: 180,
                height: 320,
                duration_us: 2_000_000,
            },
            poster: Some(crate::generation_media::VideoPoster {
                png: png.into_inner().into(),
                time_us: 100_000,
            }),
        })
    }

    #[test]
    fn generated_video_poster_survives_undo_redo_save_and_reopen() -> Result<()> {
        let (doc, page, _, _) = doc_with_page_and_component();
        let mut document = FigDocument::from_doc(doc, BTreeMap::new());
        document.doc.set_active_page(Some(page));
        let prepared = prepared_video_fixture()?;
        let expected_png = prepared
            .poster
            .as_ref()
            .context("fixture poster")?
            .png
            .clone();
        let expected_video = prepared.bytes.clone();
        let provenance = serde_json::json!({"generation_id":"video-poster-fixture"});
        let (result, change) = crate::generation_media::place_video(
            &mut document,
            prepared,
            24.,
            48.,
            Some(provenance.clone()),
        );
        result?;
        assert!(matches!(change, DocChange::Content));
        let node_id = *document
            .doc
            .scene
            .children_of(Some(page))
            .last()
            .context("video node")?;
        let node = document
            .doc
            .scene
            .get(node_id)
            .context("placed video")?
            .clone();
        let fanta_doc::NodeData::Video(video) = &node.data else {
            anyhow::bail!("video node expected");
        };
        let poster_id = video.poster.context("placed poster")?;
        assert_eq!(video.natural_size, [180, 320]);
        assert_eq!(video.local_size, [180., 320.]);
        assert_eq!(video.poster_frame_us, Some(100_000));
        assert_eq!(
            node.transform,
            fanta_doc::Transform2D::translation(24., 48.)
        );
        assert_eq!(node.meta, provenance);
        let pixels = document
            .asset_resolver
            .as_ref()
            .and_then(|resolver| resolver.resolve(poster_id))
            .context("poster resolves immediately")?;
        assert_eq!((pixels.width, pixels.height), (2, 3));
        assert_eq!(pixels.pixels_rgba.as_slice(), [34, 197, 94, 255].repeat(6));
        assert!(document.gpui_images.contains_key(&poster_id));
        assert!(document.doc.undo()?);
        assert!(!document.doc.scene.contains(node_id));
        assert!(document.doc.redo()?);
        assert_eq!(document.doc.scene.get(node_id), Some(&node));
        let directory = tempfile::tempdir()?;
        fanta_format::write_project_tree(directory.path(), &document.doc, &document.raw_assets)?;
        let reopened = load_project_document(directory.path())?;
        let restored = reopened.doc.scene.get(node_id).context("reopened video")?;
        assert_eq!(restored.data, node.data);
        assert_eq!(restored.transform, node.transform);
        assert_eq!(restored.meta, provenance);
        assert_eq!(
            reopened
                .raw_assets
                .get(&video.asset)
                .context("saved MP4")?
                .as_slice(),
            expected_video.as_ref()
        );
        assert_eq!(
            reopened
                .raw_assets
                .get(&poster_id)
                .context("saved PNG")?
                .as_slice(),
            expected_png.as_ref()
        );
        let restored_pixels = reopened
            .asset_resolver
            .as_ref()
            .and_then(|resolver| resolver.resolve(poster_id))
            .context("reopened poster")?;
        assert_eq!(restored_pixels.pixels_rgba, pixels.pixels_rgba);
        Ok(())
    }

    #[test]
    fn failed_video_placement_removes_poster_from_asset_stores() -> Result<()> {
        let mut document = FigDocument::from_doc(Doc::new(), BTreeMap::new());
        // A stale page registry forces insertion to fail after adding the poster.
        document.doc.add_page(NodeId::new());
        let (result, change) = crate::generation_media::place_video(
            &mut document,
            prepared_video_fixture()?,
            0.,
            0.,
            None,
        );
        assert!(result.is_err());
        assert!(matches!(change, DocChange::None));
        assert!(document.doc.scene.is_empty());
        assert!(document.raw_assets.is_empty());
        assert!(document.gpui_images.is_empty());
        assert!(
            document
                .agent_asset_overlay
                .as_ref()
                .context("poster overlay")?
                .added
                .read()
                .expect("overlay lock")
                .is_empty()
        );
        Ok(())
    }

    /// Prewarming decodes what a page's first frame will draw. Images that
    /// only appear as paints (a rectangle's image fill, an image stroke, a
    /// frame background) or through an instance's component master used to
    /// be skipped, so a prewarmed page still stalled on decoding them.
    #[test]
    fn page_image_assets_covers_paints_and_instance_masters() {
        use fanta_doc::{
            BitmapNode, BlendMode, BoundProp, CanvasNode, Color, Fill, GroupNode, ImageAdjust,
            ImageFitMode, InstanceNode, NodeData, Override, OverridePath, OverrideValue, Stroke,
            VectorNode,
        };

        fn image_fill(asset: AssetId) -> Fill {
            Fill::Image {
                asset,
                mode: ImageFitMode::Fill,
                opacity: 1.0,
                crop: None,
                scale: None,
                rotation: None,
                blend: BlendMode::Normal,
                adjust: ImageAdjust::default(),
            }
        }

        let (mut doc, page_root, component, master_root) = doc_with_page_and_component();
        let fill_asset = AssetId::new();
        let stroke_asset = AssetId::new();
        let background_asset = AssetId::new();
        let override_asset = AssetId::new();
        let master_asset = AssetId::new();
        let swapped_asset = AssetId::new();
        let unrelated_asset = AssetId::new();

        let mut rectangle = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
            0.0,
            0.0,
            10.0,
            10.0,
            Color::WHITE,
        )));
        if let NodeData::Vector(vector) = &mut rectangle.data {
            vector.fills.clear();
            vector.fills.push(image_fill(fill_asset));
            let mut stroke = Stroke::solid(Color::WHITE, 1.0);
            stroke.paint = image_fill(stroke_asset);
            vector.strokes.push(stroke);
        }
        rectangle.parent = Some(page_root);
        doc.apply(Operation::create_node(rectangle))
            .expect("create rectangle");

        // The rectangle's fill asset again as a stacked frame background:
        // the result lists it once.
        let mut frame = CanvasNode::new(NodeData::Group(GroupNode {
            clip_size: Some([20.0, 20.0]),
            background: Some(image_fill(background_asset)),
            ..GroupNode::default()
        }));
        if let NodeData::Group(group) = &mut frame.data {
            group.background_fills.push(image_fill(fill_asset));
        }
        frame.parent = Some(page_root);
        doc.apply(Operation::create_node(frame))
            .expect("create frame");

        let mut master_bitmap = CanvasNode::new(NodeData::Bitmap(BitmapNode {
            asset: master_asset,
            natural_size: [1, 1],
            local_size: [10.0, 10.0],
            crop: None,
            fit: ImageFitMode::Fill,
            tint: None,
        }));
        master_bitmap.parent = Some(master_root);
        let master_bitmap_id = master_bitmap.id;
        doc.apply(Operation::create_node(master_bitmap))
            .expect("create master bitmap");

        // A second master the page never instantiates directly — an instance
        // inside the first one's expansion is SWAPPED onto it, so the page
        // still draws its bitmap.
        let mut swapped_master = CanvasNode::new(NodeData::Group(GroupNode::default()));
        let swapped_master_root = swapped_master.id;
        swapped_master.name = "Icon".to_owned();
        doc.apply(Operation::create_node(swapped_master))
            .expect("create swap target master");
        let swapped_component = fanta_doc::ComponentId::new();
        doc.apply(Operation::DefineComponent {
            def: Box::new(fanta_doc::ComponentDef::new(
                swapped_component,
                swapped_master_root,
                "Icon",
            )),
        })
        .expect("define swap target component");
        let mut swapped_bitmap = CanvasNode::new(NodeData::Bitmap(BitmapNode {
            asset: swapped_asset,
            natural_size: [1, 1],
            local_size: [10.0, 10.0],
            crop: None,
            fit: ImageFitMode::Fill,
            tint: None,
        }));
        swapped_bitmap.parent = Some(swapped_master_root);
        doc.apply(Operation::create_node(swapped_bitmap))
            .expect("create swap target bitmap");

        let mut instance = CanvasNode::new(NodeData::Instance(InstanceNode {
            component,
            overrides: vec![
                Override {
                    target_path: OverridePath::from_iter([master_bitmap_id]),
                    target_prop: BoundProp::FillColor { index: 0 },
                    value: OverrideValue::Fills {
                        fills: [image_fill(override_asset)].into_iter().collect(),
                    },
                },
                Override {
                    target_path: OverridePath::from_iter([master_bitmap_id]),
                    target_prop: BoundProp::Visible,
                    value: OverrideValue::SwapInstance {
                        component: swapped_component,
                    },
                },
            ],
            prop_values: BTreeMap::new(),
            derived: Vec::new(),
            local_size: [10.0, 10.0],
        }));
        instance.parent = Some(page_root);
        doc.apply(Operation::create_node(instance))
            .expect("create instance");

        // A shape outside the page is not part of its prewarm.
        let mut elsewhere = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
            0.0,
            0.0,
            10.0,
            10.0,
            Color::WHITE,
        )));
        if let NodeData::Vector(vector) = &mut elsewhere.data {
            vector.fills.clear();
            vector.fills.push(image_fill(unrelated_asset));
        }
        doc.apply(Operation::create_node(elsewhere))
            .expect("create unrelated rectangle");

        let mut expected = vec![
            fill_asset,
            stroke_asset,
            background_asset,
            override_asset,
            master_asset,
            swapped_asset,
        ];
        expected.sort();
        assert_eq!(page_image_assets(&doc, page_root), expected);
    }

    /// The cap is checked before the decoder sees the bytes: a decoder handed
    /// an arbitrary payload can allocate far more than it was given.
    #[test]
    fn add_image_refuses_bytes_over_the_source_cap() {
        let mut stores = TestAssetStores::default();
        let error = stores
            .stores()
            .add_image(vec![0; MAX_IMAGE_SOURCE_BYTES + 1])
            .expect_err("an over-cap payload is refused");
        assert!(
            error
                .to_string()
                .contains(&MAX_IMAGE_SOURCE_BYTES.to_string()),
            "the error names the cap: {error:#}"
        );
        assert!(stores.raw_assets().is_empty(), "nothing is ingested");
        assert!(
            stores.resolver().is_none(),
            "no overlay resolver is installed for a refused image"
        );
    }

    /// The write that materializes a bare `.fig`'s project on open projects
    /// every design already; its memo is what makes the first autosave
    /// incremental, so it must come back warm alongside the adopted root.
    #[test]
    fn materializing_on_open_warms_the_write_cache_and_never_overwrites_a_project() {
        let dir = tempfile::tempdir().expect("tempdir");
        let fig_path = dir.path().join("Design.fig");
        let document = FigDocument::from_doc(doc_with_one_page(), BTreeMap::new());

        let materialized = materialize_project_on_open(&fig_path, &document)
            .expect("materializes next to the .fig");
        assert_eq!(materialized.root, dir.path().join("Design"));
        assert!(fanta_format::is_project_dir(&materialized.root));
        assert_eq!(
            materialized.write_cache.cached_designs(),
            1,
            "the one page's projection is memoized for the first autosave"
        );

        // A project that appeared at the target since the open began is left
        // alone.
        assert!(materialize_project_on_open(&fig_path, &document).is_none());
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

    struct SaveGenerationFixture {
        _directory: tempfile::TempDir,
        root: PathBuf,
        item: Entity<FigItem>,
        view: Entity<crate::FigView>,
        window: gpui::WindowHandle<gpui::Empty>,
        page: NodeId,
        text: NodeId,
    }

    async fn save_generation_fixture(cx: &mut TestAppContext) -> SaveGenerationFixture {
        let project = empty_project(cx).await;
        cx.update(|cx| {
            assets::Assets.load_test_fonts(cx);
            theme_settings::init(theme::LoadThemes::JustBase, cx);
            release_channel::init(semver::Version::new(0, 0, 0), cx);
            editor::init(cx);
            #[cfg(feature = "fanta-gpui-ui")]
            {
                gpui_component::init(cx);
                fanta_gpui::init(cx);
                crate::theme_bridge::init(cx);
            }
        });
        let directory = tempfile::tempdir().expect("temporary save project");
        let root = directory.path().join("Design");
        let mut doc = doc_with_one_page();
        let page = doc.active_page().expect("active page");
        let mut text = fanta_doc::CanvasNode::new(fanta_doc::NodeData::Text(
            fanta_doc::TextNode::new("Before saving", 180.0, 30.0),
        ));
        text.parent = Some(page);
        let text_id = text.id;
        doc.apply(Operation::create_node(text))
            .expect("create text");
        write_project(&root, &doc, &BTreeMap::new()).expect("write initial project");
        let item = ready_item(
            &project,
            root.join("fanta.json"),
            Some(root.clone()),
            doc,
            cx,
        );
        let window = cx.add_window(|_, _| gpui::Empty);
        let view = window
            .update(cx, |_, window, cx| {
                cx.new(|cx| crate::FigView::new(item.clone(), project, window, cx))
            })
            .expect("create autosaving view");
        SaveGenerationFixture {
            _directory: directory,
            root,
            item,
            view,
            window,
            page,
            text: text_id,
        }
    }

    fn pause_next_save(
        item: &Entity<FigItem>,
        cx: &mut TestAppContext,
    ) -> (
        futures::channel::oneshot::Receiver<()>,
        futures::channel::oneshot::Sender<()>,
    ) {
        pause_next_save_at(item, false, cx)
    }

    fn pause_next_save_at(
        item: &Entity<FigItem>,
        after_write: bool,
        cx: &mut TestAppContext,
    ) -> (
        futures::channel::oneshot::Receiver<()>,
        futures::channel::oneshot::Sender<()>,
    ) {
        let (reached, arrived) = futures::channel::oneshot::channel();
        let (resume, wait) = futures::channel::oneshot::channel();
        item.read_with(cx, |item, _| {
            let previous = item
                .project_writes
                .next_save_barrier
                .lock()
                .expect("save test barrier")
                .replace(SaveTestBarrier {
                    reached,
                    resume: wait,
                    after_write,
                });
            assert!(previous.is_none(), "only one next-save barrier is armed");
        });
        (arrived, resume)
    }

    fn rename_saved_page(fixture: &SaveGenerationFixture, name: &str, cx: &mut TestAppContext) {
        fixture.item.update(cx, |item, cx| {
            let old = item
                .doc()
                .expect("document")
                .scene
                .get(fixture.page)
                .expect("page")
                .name
                .clone();
            item.apply(
                Operation::SetName {
                    id: fixture.page,
                    old,
                    new: name.into(),
                },
                cx,
            )
            .expect("rename page");
        });
    }

    fn place_image_during_save(
        fixture: &SaveGenerationFixture,
        cx: &mut TestAppContext,
    ) -> (NodeId, AssetId, Vec<u8>) {
        let mut bytes = Vec::new();
        image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            2,
            2,
            image::Rgba([34, 197, 94, 255]),
        ))
        .write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Png,
        )
        .expect("encode image");
        let (node, asset) = fixture.item.update(cx, |item, cx| {
            item.with_document(cx, |document| {
                let (doc, mut assets) = document.doc_and_assets();
                let (asset, natural_size) = assets.add_image(bytes.clone()).expect("ingest image");
                let mut bitmap = fanta_doc::CanvasNode::new(fanta_doc::NodeData::Bitmap(
                    fanta_doc::BitmapNode {
                        asset,
                        natural_size,
                        local_size: [24.0, 24.0],
                        crop: None,
                        fit: fanta_doc::ImageFitMode::Fill,
                        tint: None,
                    },
                ));
                bitmap.parent = Some(fixture.page);
                let node = bitmap.id;
                doc.apply(Operation::create_node(bitmap))
                    .expect("place image");
                ((node, asset), DocChange::Content)
            })
            .expect("loaded document")
        });
        (node, asset, bytes)
    }

    async fn assert_save_generation_preserves_later_media(kind: SaveKind, cx: &mut TestAppContext) {
        let fixture = save_generation_fixture(cx).await;
        rename_saved_page(&fixture, "Snapshot A", cx);
        let (arrived, resume) = pause_next_save(&fixture.item, cx);
        let save = fixture.item.update(cx, |item, cx| item.save(kind, cx));
        arrived.await.expect("save reached background writer");
        rename_saved_page(&fixture, "Newer edit B", cx);
        let (node, asset, bytes) = place_image_during_save(&fixture, cx);
        resume.send(()).expect("resume first save");
        save.await.expect("first save succeeds");
        cx.run_until_parked();

        fixture.item.read_with(cx, |item, _| {
            assert!(
                item.is_dirty(),
                "an older snapshot cannot mark newer edits clean"
            );
            assert_eq!(
                item.merge_base
                    .as_ref()
                    .expect("saved merge base")
                    .scene
                    .get(fixture.page)
                    .expect("saved page")
                    .name,
                "Snapshot A"
            );
            assert_eq!(
                item.document().expect("document").raw_assets.get(&asset),
                Some(&bytes)
            );
        });
        let (first, first_assets) =
            fanta_format::read_project_tree(&fixture.root).expect("first save");
        assert_eq!(
            first.scene.get(fixture.page).expect("page").name,
            "Snapshot A"
        );
        assert!(!first.scene.contains(node));
        assert!(!first_assets.contains_key(&asset));

        cx.executor().advance_clock(Duration::from_secs(2));
        cx.run_until_parked();
        assert!(
            !fixture.item.read_with(cx, |item, _| item.is_dirty()),
            "the real view must autosave the remaining edit"
        );
        let (reopened, assets) =
            fanta_format::read_project_tree(&fixture.root).expect("reopen autosaved project");
        assert_eq!(
            reopened.scene.get(fixture.page).expect("page").name,
            "Newer edit B"
        );
        let fanta_doc::NodeData::Bitmap(bitmap) =
            &reopened.scene.get(node).expect("new image node").data
        else {
            panic!("placed image must reopen as a bitmap");
        };
        assert_eq!(bitmap.asset, asset);
        assert_eq!(assets.get(&asset), Some(&bytes));
        let reopened = FigDocument::from_doc(reopened, assets);
        assert!(
            reopened.gpui_images.contains_key(&asset),
            "reopened image is decodable"
        );
    }

    #[gpui::test]
    async fn save_generation_explicit_preserves_later_edits_and_image_assets(
        cx: &mut TestAppContext,
    ) {
        assert_save_generation_preserves_later_media(SaveKind::Explicit, cx).await;
    }

    #[gpui::test]
    async fn save_generation_auto_preserves_later_edits_and_image_assets(cx: &mut TestAppContext) {
        assert_save_generation_preserves_later_media(SaveKind::Auto, cx).await;
    }

    #[gpui::test]
    async fn save_generation_preserves_text_input_started_after_its_snapshot(
        cx: &mut TestAppContext,
    ) {
        let fixture = save_generation_fixture(cx).await;
        let (arrived, resume) = pause_next_save(&fixture.item, cx);
        let save = fixture
            .item
            .update(cx, |item, cx| item.save(SaveKind::Explicit, cx));
        arrived.await.expect("save reached background writer");
        fixture
            .window
            .update(cx, |_, window, cx| {
                fixture.view.update(cx, |view, cx| {
                    view.open_text_edit(
                        fixture.text,
                        crate::view::TextEditSeed::SelectAll,
                        window,
                        cx,
                    );
                    gpui::EntityInputHandler::replace_text_in_range(
                        view,
                        None,
                        "Typed during save",
                        window,
                        cx,
                    );
                });
            })
            .expect("edit text while saving");
        resume.send(()).expect("resume save");
        save.await.expect("save completes");
        cx.run_until_parked();
        fixture.view.read_with(cx, |view, _| {
            let edit = view
                .text_edit
                .as_ref()
                .expect("saving must not discard a newer text session");
            assert_eq!(edit.session.buffer(), "Typed during save");
        });
        fixture
            .view
            .update(cx, |view, cx| view.commit_text_edit(cx));
        cx.run_until_parked();
        cx.executor().advance_clock(Duration::from_secs(2));
        cx.run_until_parked();
        let (reopened, _) =
            fanta_format::read_project_tree(&fixture.root).expect("reopen committed text");
        assert_eq!(
            reopened
                .scene
                .get(fixture.text)
                .expect("text node")
                .data
                .as_text()
                .expect("text")
                .content,
            "Typed during save"
        );
        assert!(!fixture.item.read_with(cx, |item, _| item.is_dirty()));
    }

    #[gpui::test]
    async fn save_generation_serializes_writers_and_rearms_newer_edits(cx: &mut TestAppContext) {
        let fixture = save_generation_fixture(cx).await;
        rename_saved_page(&fixture, "Snapshot A", cx);
        let (arrived, resume) = pause_next_save(&fixture.item, cx);
        let first = fixture
            .item
            .update(cx, |item, cx| item.save(SaveKind::Explicit, cx));
        arrived.await.expect("first writer paused");
        rename_saved_page(&fixture, "Snapshot B", cx);
        let mut second = fixture
            .item
            .update(cx, |item, cx| item.save(SaveKind::Auto, cx));
        cx.run_until_parked();
        assert!(
            futures::poll!(&mut second).is_pending(),
            "overlapping saves must not write the same tree concurrently"
        );
        let (before, _) = fanta_format::read_project_tree(&fixture.root).expect("original tree");
        assert_eq!(before.scene.get(fixture.page).expect("page").name, "Page 1");
        rename_saved_page(&fixture, "Current edit C", cx);
        resume.send(()).expect("release first writer");
        first.await.expect("first save");
        second.await.expect("second save");
        cx.run_until_parked();
        fixture.item.read_with(cx, |item, _| {
            assert!(item.is_dirty(), "neither snapshot contains C");
            assert_eq!(
                item.merge_base
                    .as_ref()
                    .expect("base")
                    .scene
                    .get(fixture.page)
                    .expect("page")
                    .name,
                "Snapshot B"
            );
        });
        let (saved, _) = fanta_format::read_project_tree(&fixture.root).expect("second tree");
        assert_eq!(
            saved.scene.get(fixture.page).expect("page").name,
            "Snapshot B"
        );
        cx.executor().advance_clock(Duration::from_secs(2));
        cx.run_until_parked();
        let (current, _) =
            fanta_format::read_project_tree(&fixture.root).expect("current autosave");
        assert_eq!(
            current.scene.get(fixture.page).expect("page").name,
            "Current edit C"
        );
        assert!(!fixture.item.read_with(cx, |item, _| item.is_dirty()));
    }

    #[gpui::test]
    async fn save_generation_queue_preserves_capture_order_when_polled_backwards(
        _cx: &mut TestAppContext,
    ) {
        let writes = Arc::new(ProjectWrites::default());
        let first = writes.begin();
        let second = writes.begin();
        let mut second_turn = Box::pin(second.wait_for_turn());
        assert!(futures::poll!(&mut second_turn).is_pending());
        first.wait_for_turn().await;
        assert!(futures::poll!(&mut second_turn).is_pending());
        drop(first);
        second_turn.await;
        drop(second);
        let state = writes.state.lock().expect("write state");
        assert_eq!(state.active, 0);
        assert!(
            state.last_write.is_none(),
            "completed chains must not accumulate"
        );
    }

    #[gpui::test]
    async fn save_generation_rechecks_source_lock_before_a_queued_write(cx: &mut TestAppContext) {
        let fixture = save_generation_fixture(cx).await;
        rename_saved_page(&fixture, "Snapshot A", cx);
        let (arrived, resume) = pause_next_save(&fixture.item, cx);
        let first = fixture
            .item
            .update(cx, |item, cx| item.save(SaveKind::Explicit, cx));
        arrived.await.expect("first writer paused");
        rename_saved_page(&fixture, "Newer edit B", cx);
        let second = fixture
            .item
            .update(cx, |item, cx| item.save(SaveKind::Explicit, cx));
        fixture
            .item
            .update(cx, |item, cx| item.set_source_edit_locked(true, cx));
        resume.send(()).expect("release already started writer");
        first.await.expect("first save");
        let error = second
            .await
            .expect_err("queued canvas write must respect the new source lock");
        assert!(error.to_string().contains("FNX source is dirty"));
        let (saved, _) = fanta_format::read_project_tree(&fixture.root).expect("saved tree");
        assert_eq!(
            saved.scene.get(fixture.page).expect("page").name,
            "Snapshot A"
        );
        assert!(fixture.item.read_with(cx, |item, _| item.is_dirty()));
        fixture
            .item
            .update(cx, |item, cx| item.set_source_edit_locked(false, cx));
        fixture
            .item
            .update(cx, |item, cx| item.save(SaveKind::Explicit, cx))
            .await
            .expect("save after source lock released");
        let (saved, _) = fanta_format::read_project_tree(&fixture.root).expect("current tree");
        assert_eq!(
            saved.scene.get(fixture.page).expect("page").name,
            "Newer edit B"
        );
    }

    #[gpui::test]
    async fn save_generation_queue_waits_for_a_canceled_predecessors_worker(
        _cx: &mut TestAppContext,
    ) {
        let writes = Arc::new(ProjectWrites::default());
        let foreground = writes.begin();
        let background = foreground.clone();
        let canceled_queued = writes.begin();
        let surviving = writes.begin();
        let mut turn = Box::pin(surviving.wait_for_turn());
        drop(foreground);
        drop(canceled_queued);
        assert!(
            futures::poll!(&mut turn).is_pending(),
            "canceling the middle save cannot bypass the first writer"
        );
        assert!(writes.suppresses_watcher(Instant::now() + Duration::from_secs(120)));
        drop(background);
        turn.await;
        drop(surviving);
        let state = writes.state.lock().expect("write state");
        assert_eq!(state.active, 0);
        assert!(state.last_write.is_none());
    }

    #[gpui::test]
    async fn save_generation_overlapping_materialization_reuses_the_adopted_root(
        cx: &mut TestAppContext,
    ) {
        let fixture = save_generation_fixture(cx).await;
        let source = fixture._directory.path().join("Imported.fig");
        let root = fixture._directory.path().join("Imported");
        let source_bytes = b"untouched original import";
        std::fs::write(&source, source_bytes).expect("original source");
        fixture.item.update(cx, |item, _| {
            item.project_root = None;
            item.abs_path = source.clone();
        });
        rename_saved_page(&fixture, "Snapshot A", cx);
        let (arrived, resume) = pause_next_save_at(&fixture.item, true, cx);
        let first = fixture
            .item
            .update(cx, |item, cx| item.save(SaveKind::Explicit, cx));
        arrived
            .await
            .expect("project written, adoption still paused");
        assert!(fanta_format::is_project_dir(&root));
        assert!(
            fixture
                .item
                .read_with(cx, |item, _| item.project_root().is_none())
        );
        rename_saved_page(&fixture, "Newer edit B", cx);
        let (node, asset, bytes) = place_image_during_save(&fixture, cx);
        let mut second = fixture
            .item
            .update(cx, |item, cx| item.save(SaveKind::Explicit, cx));
        cx.run_until_parked();
        assert!(
            futures::poll!(&mut second).is_pending(),
            "the second save must wait for adoption of the first root"
        );
        resume.send(()).expect("finish materialization");
        assert_eq!(first.await.expect("first save"), Some(root.clone()));
        assert_eq!(second.await.expect("second save"), None);
        cx.run_until_parked();
        assert_eq!(
            fixture
                .item
                .read_with(cx, |item, _| item.project_root().map(Path::to_path_buf)),
            Some(root.clone())
        );
        let (reopened, assets) =
            fanta_format::read_project_tree(&root).expect("reopen adopted root");
        assert_eq!(
            reopened.scene.get(fixture.page).expect("page").name,
            "Newer edit B"
        );
        assert!(reopened.scene.contains(node));
        assert_eq!(assets.get(&asset), Some(&bytes));
        assert_eq!(
            std::fs::read(&source).expect("original source"),
            source_bytes
        );
        assert!(!fixture._directory.path().join("Imported-2").exists());
        assert!(!fixture.item.read_with(cx, |item, _| item.is_dirty()));
    }

    #[gpui::test]
    async fn save_generation_canceled_materialization_refuses_to_overwrite_external_edits(
        cx: &mut TestAppContext,
    ) {
        let fixture = save_generation_fixture(cx).await;
        let source = fixture._directory.path().join("Imported.fig");
        let root = fixture._directory.path().join("Imported");
        let source_bytes = b"untouched original import";
        std::fs::write(&source, source_bytes).expect("original source");
        fixture.item.update(cx, |item, _| {
            item.project_root = None;
            item.abs_path = source.clone();
        });
        rename_saved_page(&fixture, "Snapshot A", cx);
        let (arrived, resume) = pause_next_save_at(&fixture.item, true, cx);
        let first = fixture
            .item
            .update(cx, |item, cx| item.save(SaveKind::Explicit, cx));
        arrived.await.expect("write completed before cancellation");
        rename_saved_page(&fixture, "Unsaved edit B", cx);
        let (node, asset, bytes) = place_image_during_save(&fixture, cx);
        drop(first);
        // The worker may already have observed cancellation; either outcome
        // releases the test gate without letting its foreground adopt the root.
        match resume.send(()) {
            Ok(()) | Err(()) => {}
        }
        cx.run_until_parked();
        let (mut external, assets) =
            fanta_format::read_project_tree(&root).expect("completed tree");
        external
            .apply(Operation::SetName {
                id: fixture.page,
                old: "Snapshot A".into(),
                new: "External edit".into(),
            })
            .expect("external edit");
        write_project(&root, &external, &assets).expect("write external edit");
        let error = fixture
            .item
            .update(cx, |item, cx| item.save(SaveKind::Explicit, cx))
            .await
            .expect_err("do not adopt an unverified foreign tree");
        assert!(error.to_string().contains("already exists"));
        fixture.item.read_with(cx, |item, _| {
            assert!(item.is_dirty());
            assert!(item.project_root().is_none());
            assert!(item.doc().expect("document").scene.contains(node));
            assert_eq!(
                item.document().expect("document").raw_assets.get(&asset),
                Some(&bytes)
            );
        });
        let (unchanged, _) =
            fanta_format::read_project_tree(&root).expect("external tree preserved");
        assert_eq!(
            unchanged.scene.get(fixture.page).expect("page").name,
            "External edit"
        );
        assert_eq!(
            std::fs::read(&source).expect("original source"),
            source_bytes
        );
    }

    #[gpui::test]
    async fn save_as_preserves_original_edits_assets_and_subsequent_destination(
        cx: &mut TestAppContext,
    ) {
        let result: Result<()> = async {
            init_test(cx);
            let directory = tempfile::tempdir()?;
            let original = directory.path().join("Original");
            let copy = directory.path().join("Copy");
            let document = doc_with_one_page();
            let page = document.active_page().context("active page")?;
            write_project(&original, &document, &BTreeMap::new())?;
            let original_source =
                fanta_format::locate_page_source(&original, page).context("page source")?;
            let original_bytes = std::fs::read(&original_source)?;
            let original_manifest = std::fs::read(original.join("fanta.json"))?;
            let file_system = FakeFs::new(cx.executor());
            file_system
                .insert_tree(
                    directory.path(),
                    serde_json::json!({"Original": {"fanta.json": "{}"}}),
                )
                .await;
            let project = Project::test(file_system, [directory.path()], cx).await;
            let worktree_id = project.read_with(cx, |project, cx| {
                project
                    .worktrees(cx)
                    .next()
                    .expect("worktree")
                    .read(cx)
                    .id()
            });
            let item = ready_item(
                &project,
                original.join("fanta.json"),
                Some(original.clone()),
                document,
                cx,
            );
            let original_key = original.canonicalize()?;
            cx.update(|cx| register_shared_project_item(original_key.clone(), &item, cx));
            let asset = AssetId::new();
            let bytes = b"encoded asset copied without conversion".to_vec();
            item.update(cx, |item, cx| {
                item.with_document(cx, |document| {
                    Arc::make_mut(&mut document.raw_assets).insert(asset, bytes.clone());
                    ((), DocChange::Content)
                });
                item.apply(
                    Operation::SetName {
                        id: page,
                        old: "Page 1".into(),
                        new: "Current edits".into(),
                    },
                    cx,
                )
            })?;
            let destination = ProjectPath {
                worktree_id,
                path: util::rel_path::rel_path("Copy").into(),
            };
            let save_as = item.update(cx, |item, cx| {
                item.save_as(project.clone(), destination, cx)
            });
            // Edits while the snapshot is being written must remain dirty and
            // must never be autosaved back to the old project during the copy.
            item.update(cx, |item, cx| {
                item.apply(
                    Operation::SetName {
                        id: page,
                        old: "Current edits".into(),
                        new: "Edited during copy".into(),
                    },
                    cx,
                )
            })?;
            assert!(
                item.update(cx, |item, cx| item.save(SaveKind::Auto, cx))
                    .await?
                    .is_none()
            );
            save_as.await?;
            let (copied_document, copied_assets) = fanta_format::read_project_tree(&copy)?;
            assert_eq!(
                copied_document.scene.get(page).context("copied page")?.name,
                "Current edits"
            );
            assert_eq!(copied_assets.get(&asset), Some(&bytes));
            assert_eq!(std::fs::read(&original_source)?, original_bytes);
            assert_eq!(
                std::fs::read(original.join("fanta.json"))?,
                original_manifest
            );
            item.read_with(cx, |item, cx| {
                assert!(
                    item.is_dirty(),
                    "the newer edit is not part of the copied snapshot"
                );
                assert_eq!(item.project_root(), Some(copy.as_path()));
                assert_eq!(item.abs_path(), copy.join("fanta.json"));
                assert_eq!(
                    project.read(cx).absolute_path(&item.path, cx),
                    Some(copy.join("fanta.json"))
                );
                assert_eq!(item.title().as_ref(), "Copy");
            });
            cx.update(|cx| {
                assert!(shared_project_item(&original_key, cx).is_none());
                assert_eq!(
                    shared_project_item(&copy.canonicalize().expect("saved copy"), cx)
                        .map(|item| item.entity_id()),
                    Some(item.entity_id())
                );
            });
            item.update(cx, |item, cx| item.save(SaveKind::Explicit, cx))
                .await?;
            let (saved_again, saved_assets) = fanta_format::read_project_tree(&copy)?;
            assert_eq!(
                saved_again.scene.get(page).context("saved page")?.name,
                "Edited during copy"
            );
            assert_eq!(saved_assets.get(&asset), Some(&bytes));
            assert_eq!(std::fs::read(&original_source)?, original_bytes);
            assert_eq!(
                std::fs::read(original.join("fanta.json"))?,
                original_manifest
            );
            assert!(!item.read_with(cx, |item, _| item.is_dirty()));
            Ok(())
        }
        .await;
        result.expect(
            "Save As preserves the original, copies edits/assets and retargets subsequent saves",
        );
    }

    #[gpui::test]
    async fn save_as_rejects_existing_content_without_retargeting(cx: &mut TestAppContext) {
        let result: Result<()> = async {
            init_test(cx);
            let directory = tempfile::tempdir()?;
            let original = directory.path().join("Original");
            let occupied = directory.path().join("Occupied");
            let document = doc_with_one_page();
            write_project(&original, &document, &BTreeMap::new())?;
            std::fs::create_dir(&occupied)?;
            std::fs::write(occupied.join("keep.txt"), "keep this file")?;
            let file_system = FakeFs::new(cx.executor());
            file_system
                .insert_tree(
                    directory.path(),
                    serde_json::json!({"Original": {}, "Occupied": {}}),
                )
                .await;
            let project = Project::test(file_system, [directory.path()], cx).await;
            let worktree_id = project.read_with(cx, |project, cx| {
                project
                    .worktrees(cx)
                    .next()
                    .expect("worktree")
                    .read(cx)
                    .id()
            });
            let item = ready_item(
                &project,
                original.join("fanta.json"),
                Some(original.clone()),
                document,
                cx,
            );
            item.update(cx, |item, _| item.dirty = true);
            let destination = ProjectPath {
                worktree_id,
                path: util::rel_path::rel_path("Occupied").into(),
            };
            let result = item
                .update(cx, |item, cx| {
                    item.save_as(project.clone(), destination, cx)
                })
                .await;
            assert!(result.is_err());
            assert_eq!(
                std::fs::read_to_string(occupied.join("keep.txt"))?,
                "keep this file"
            );
            assert!(!occupied.join("fanta.json").exists());
            item.read_with(cx, |item, _| {
                assert_eq!(item.project_root(), Some(original.as_path()));
                assert_eq!(item.abs_path(), original.join("fanta.json"));
                assert!(item.is_dirty());
                assert!(!item.project_writes.changing_destination());
            });
            Ok(())
        }
        .await;
        result.expect("a failed Save As preserves the occupied destination and current design");
    }

    #[test]
    fn save_as_recovers_when_original_folder_is_missing() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let original = directory.path().join("Original");
        let copy = directory.path().join("Recovered");
        let document = doc_with_one_page();
        let page = document.active_page().context("active page")?;
        write_project(&original, &document, &BTreeMap::new())?;
        std::fs::remove_dir_all(&original)?;
        write_project_copy(&copy, Some(&original), &document, &BTreeMap::new())?;
        let (recovered, _) = fanta_format::read_project_tree(&copy)?;
        assert!(recovered.scene.contains(page));
        assert!(
            !original.exists(),
            "recovery must not recreate the original folder"
        );
        Ok(())
    }

    #[test]
    fn save_as_rejects_nested_and_aliased_destinations() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let original = directory.path().join("Original");
        let document = doc_with_one_page();
        write_project(&original, &document, &BTreeMap::new())?;
        assert!(
            write_project_copy(&original, Some(&original), &document, &BTreeMap::new()).is_err()
        );
        assert!(
            write_project_copy(
                &original.join("Nested"),
                Some(&original),
                &document,
                &BTreeMap::new()
            )
            .is_err()
        );
        assert!(!original.join("Nested").exists());
        #[cfg(unix)]
        {
            let alias = directory.path().join("Alias");
            std::os::unix::fs::symlink(&original, &alias)?;
            assert!(
                write_project_copy(&alias, Some(&original), &document, &BTreeMap::new()).is_err()
            );
            assert!(
                write_project_copy(
                    &alias.join("Nested"),
                    Some(&original),
                    &document,
                    &BTreeMap::new()
                )
                .is_err()
            );
        }
        let empty = directory.path().join("Empty");
        std::fs::create_dir(&empty)?;
        write_project_copy(&empty, Some(&original), &document, &BTreeMap::new())?;
        assert!(empty.join("fanta.json").is_file());
        Ok(())
    }

    #[test]
    fn save_as_write_lease_survives_foreground_cancellation() -> Result<()> {
        let writes = Arc::new(ProjectWrites::default());
        let save = writes.begin();
        assert!(writes.begin_destination_change().is_err());
        drop(save);
        let foreground = writes.begin_destination_change()?;
        let background = foreground.clone();
        assert!(writes.changing_destination());
        drop(foreground);
        assert!(writes.changing_destination());
        assert!(writes.begin_destination_change().is_err());
        drop(background);
        assert!(!writes.changing_destination());
        assert!(writes.begin_destination_change().is_ok());
        Ok(())
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

    /// An already-set visible active page (e.g. carried by a `.fig` import)
    /// wins over the default.
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

    #[test]
    fn from_doc_replaces_an_imported_hidden_active_page_with_the_visible_default() -> Result<()> {
        use fanta_doc::{CanvasNode, Color, Fill, GroupNode, NodeData, NodeFlags, VectorNode};

        let mut doc = Doc::new();
        let mut hidden_page = CanvasNode::new(NodeData::Group(GroupNode::default()));
        hidden_page.name = "Internal Only Canvas".to_owned();
        hidden_page.flags.insert(NodeFlags::HIDDEN);
        hidden_page.meta = serde_json::json!({"hidden_page": true});
        let hidden_root = hidden_page.id;
        doc.apply(Operation::create_node(hidden_page))?;
        doc.add_page(hidden_root);

        let mut visible_page = CanvasNode::new(NodeData::Group(GroupNode {
            background: Some(Fill::solid(Color::WHITE)),
            ..GroupNode::default()
        }));
        visible_page.name = "Design".to_owned();
        let visible_root = visible_page.id;
        doc.apply(Operation::create_node(visible_page))?;
        doc.add_page(visible_root);
        assert_eq!(doc.active_page(), Some(hidden_root));

        let mut document = FigDocument::from_doc(doc, BTreeMap::new());
        assert_eq!(document.doc.active_page(), Some(visible_root));
        assert_eq!(
            document.page(None).and_then(|page| page.root),
            Some(visible_root)
        );
        assert!(
            document
                .pages
                .iter()
                .any(|page| page.root == Some(hidden_root) && page.hidden)
        );
        assert!(
            document
                .doc
                .scene
                .get(hidden_root)
                .is_some_and(|node| node.flags.contains(NodeFlags::HIDDEN))
        );

        let mut rectangle = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
            -10.0,
            -10.0,
            20.0,
            20.0,
            Color::rgb(34, 197, 94),
        )));
        rectangle.parent = document.doc.active_page();
        document.doc.apply(Operation::create_node(rectangle))?;
        let mut renderer = fanta_render::RasterRenderer::new(64, 64)?;
        renderer.render_page(
            &document.doc.scene,
            &Viewport {
                center: [0.0, 0.0],
                zoom: 1.0,
            },
            document.doc.active_page(),
        );
        let pixels = renderer.copy_rgba();
        assert_eq!(pixels.get(..4), Some([255, 255, 255, 255].as_slice()));
        let center = (32 * 64 + 32) * 4;
        assert_eq!(
            pixels.get(center..center + 4),
            Some([34, 197, 94, 255].as_slice())
        );
        Ok(())
    }

    /// The load that materializes a bare `.fig` hands the item both the
    /// adopted root and the materializing write's memo; without the memo the
    /// first autosave starts cold and re-prints every page.
    #[gpui::test]
    async fn a_save_suppresses_its_watcher_after_the_initial_deadline_expires(
        cx: &mut TestAppContext,
    ) {
        init_test(cx);
        let directory = tempfile::tempdir().expect("temporary project");
        let root = directory.path().join("Design");
        let fs = FakeFs::new(cx.executor());
        fs.insert_tree(&root, serde_json::json!({"fanta.json":"{}"}))
            .await;
        let project = Project::test(fs, [root.as_path()], cx).await;
        let worktree_id = project.read_with(cx, |project, cx| {
            project
                .worktrees(cx)
                .next()
                .expect("worktree")
                .read(cx)
                .id()
        });
        let item = ready_item(
            &project,
            directory.path().join("Design.fig"),
            Some(root),
            doc_with_one_page(),
            cx,
        );
        let changes: UpdatedEntriesSet = vec![(
            util::rel_path::rel_path("pages/page-1/page.fnx").into(),
            ProjectEntryId::from_proto(1),
            PathChange::Updated,
        )]
        .into();
        let save = item.update(cx, |item, cx| {
            item.dirty = true;
            let save = item.save(SaveKind::Explicit, cx);
            // Model a writer still running after the old one-second deadline.
            // Wall-clock Instant does not follow the GPUI test clock.
            item.suppress_watcher_until = Some(Instant::now() - Duration::from_secs(2));
            item.worktree_entries_updated(&project, worktree_id, &changes, cx);
            assert!(
                item.reload_task.is_none(),
                "our own save must not start a reload or merge"
            );
            assert!(
                !item.has_conflict(),
                "our own save must not be treated as an external edit"
            );
            assert!(
                item.project_writes
                    .suppresses_watcher(Instant::now() + Duration::from_secs(2))
            );
            save
        });
        save.await.expect("save completes");
        item.update(cx, |item, cx| {
            assert_eq!(
                item.project_writes
                    .state
                    .lock()
                    .expect("write state")
                    .active,
                0
            );
            assert!(
                item.project_writes.suppresses_watcher(Instant::now()),
                "completion keeps the delivery cooldown"
            );
            item.suppress_watcher_until = None;
            item.project_writes
                .state
                .lock()
                .expect("write state")
                .quiet_until = Some(Instant::now() - Duration::from_secs(1));
            item.worktree_entries_updated(&project, worktree_id, &changes, cx);
            assert!(
                item.reload_task.is_some(),
                "external edits still reload after the cooldown"
            );
            item.reload_task = None;
        });
    }

    #[test]
    fn write_leases_cover_cancellation_overlaps_and_background_completion() {
        let writes = Arc::new(ProjectWrites::default());
        let foreground = writes.begin();
        let worker = foreground.clone();
        let overlapping = writes.begin();
        drop(foreground);
        drop(overlapping);
        assert!(
            writes.suppresses_watcher(Instant::now() + Duration::from_secs(120)),
            "a canceled foreground must not unprotect its still-running writer"
        );
        drop(worker);
        assert!(!writes.suppresses_watcher(Instant::now() + SELF_WRITE_SUPPRESS_WINDOW * 2));
        assert!(
            writes.suppresses_watcher(Instant::now()),
            "even canceled writes get a completion cooldown"
        );
    }

    #[gpui::test]
    async fn failed_and_canceled_saves_release_their_write_leases(cx: &mut TestAppContext) {
        let project = empty_project(cx).await;
        let directory = tempfile::tempdir().expect("temporary project");
        let root = directory.path().join("not-a-directory");
        std::fs::write(&root, b"occupied").expect("block project directory creation");
        let item = ready_item(
            &project,
            directory.path().join("Design.fig"),
            Some(root),
            doc_with_one_page(),
            cx,
        );
        let save = item.update(cx, |item, cx| item.save(SaveKind::Explicit, cx));
        assert!(save.await.is_err());
        let writes = item.read_with(cx, |item, _| item.project_writes.clone());
        assert_eq!(
            writes.state.lock().expect("write state").active,
            0,
            "a failed writer releases its lease"
        );
        let save = item.update(cx, |item, cx| item.save(SaveKind::Explicit, cx));
        let overlapping = item.update(cx, |item, cx| item.save(SaveKind::Explicit, cx));
        drop(save);
        cx.run_until_parked();
        drop(overlapping);
        cx.run_until_parked();
        assert_eq!(
            writes.state.lock().expect("write state").active,
            0,
            "canceling queued or completed saves releases every lease"
        );
    }

    #[gpui::test]
    async fn closing_an_item_does_not_retain_its_document_entity_for_a_save(
        cx: &mut TestAppContext,
    ) {
        let project = empty_project(cx).await;
        let directory = tempfile::tempdir().expect("temporary project");
        let item = ready_item(
            &project,
            directory.path().join("Design.fig"),
            Some(directory.path().join("Design")),
            doc_with_one_page(),
            cx,
        );
        let weak_item = item.downgrade();
        let writes = item.read_with(cx, |item, _| item.project_writes.clone());
        let save = item.update(cx, |item, cx| item.save(SaveKind::Explicit, cx));
        cx.update(|_| drop(item));
        assert!(
            weak_item.upgrade().is_none(),
            "the save captures a weak document entity"
        );
        assert!(
            save.await.is_err(),
            "the completed write cannot update a closed document"
        );
        assert_eq!(writes.state.lock().expect("write state").active, 0);
    }

    #[gpui::test]
    async fn the_initial_load_adopts_the_materialized_root_and_its_write_cache(
        cx: &mut TestAppContext,
    ) {
        let project = empty_project(cx).await;
        let dir = tempfile::tempdir().expect("tempdir");
        let fig_path = dir.path().join("Design.fig");
        let item = ready_item(&project, fig_path.clone(), None, Doc::new(), cx);
        item.update(cx, |item, _| {
            item.document = FigDocumentState::Loading {
                message: "Opening document...".into(),
            };
        });

        let document = FigDocument::from_doc(doc_with_one_page(), BTreeMap::new());
        let materialized = materialize_project_on_open(&fig_path, &document)
            .expect("materializes next to the .fig");
        let root = materialized.root.clone();
        item.update(cx, |item, cx| {
            item.adopt_initial_load(Ok(document), Some(materialized), cx)
        });

        item.read_with(cx, |item, _| {
            assert!(item.has_ready_document());
            assert_eq!(item.project_root(), Some(root.as_path()));
            assert_eq!(
                item.write_cache
                    .as_ref()
                    .map(|cache| cache.cached_designs()),
                Some(1),
                "the materializing write's memo seeds the first autosave"
            );
            assert!(
                item.merge_base.is_some(),
                "disk and canvas agree right after the materializing write"
            );
        });
        let key = root.canonicalize().unwrap_or_else(|_| root.clone());
        let shared = cx.update(|cx| shared_project_item(&key, cx));
        assert_eq!(
            shared.map(|shared| shared.entity_id()),
            Some(item.entity_id()),
            "the materialized project is registered for scoped re-opens"
        );

        // The first autosave takes the seeded memo and hands it back.
        item.update(cx, |item, _| item.dirty = true);
        item.update(cx, |item, cx| item.save(SaveKind::Auto, cx))
            .await
            .expect("autosave into the materialized project");
        item.read_with(cx, |item, _| {
            assert!(!item.is_dirty());
            assert_eq!(
                item.write_cache
                    .as_ref()
                    .map(|cache| cache.cached_designs()),
                Some(1),
                "the memo survives the save round trip"
            );
        });
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
            suppress_watcher_until: None,
            project_writes: Arc::default(),
            merge_base: None,
            pending_scope: None,
            last_scope: None,
            sync_epoch: 0,
            reload_task: None,
            _load_task: None,
            write_cache: None,
            project_subscriptions: Vec::new(),
        });
        item.update(cx, |item, cx| item.subscribe_to_project(project, cx));
        item
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

        let materialized = item
            .update(cx, |item, cx| item.save(SaveKind::Explicit, cx))
            .await
            .unwrap();
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
        let again = item
            .update(cx, |item, cx| item.save(SaveKind::Explicit, cx))
            .await
            .unwrap();
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
    async fn an_autosave_announces_saved_instead_of_state_changed(cx: &mut TestAppContext) {
        // `StateChanged` means "the document may have been replaced": every
        // listener drops in-flight text sessions, inspector previews and
        // rename gestures on it. The debounced autosave must not do that.
        let project = empty_project(cx).await;
        let dir = tempfile::tempdir().unwrap();
        let item = ready_item(
            &project,
            dir.path().join("Design.fig"),
            None,
            doc_with_one_page(),
            cx,
        );
        item.update(cx, |item, cx| item.save(SaveKind::Explicit, cx))
            .await
            .unwrap();

        let events = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let _subscription = cx.update(|cx| {
            cx.subscribe(&item, {
                let events = events.clone();
                move |_, event: &FigItemEvent, _| events.borrow_mut().push(*event)
            })
        });

        item.update(cx, |item, _| item.dirty = true);
        item.update(cx, |item, cx| item.save(SaveKind::Auto, cx))
            .await
            .unwrap();
        cx.run_until_parked();

        let observed = events.borrow().clone();
        assert!(
            observed.contains(&FigItemEvent::Saved),
            "an autosave announces Saved, saw {observed:?}"
        );
        assert!(
            !observed.contains(&FigItemEvent::StateChanged),
            "an autosave must not announce StateChanged, saw {observed:?}"
        );
        item.read_with(cx, |item, _| assert!(!item.is_dirty()));
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

        let result = item
            .update(cx, |item, cx| item.save(SaveKind::Explicit, cx))
            .await;
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
    async fn a_second_windows_open_watches_its_project_for_external_edits(cx: &mut TestAppContext) {
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
