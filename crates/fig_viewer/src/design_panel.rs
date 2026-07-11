//! The Fanta design panel: Pages, Layers, Components, and Assets sections for
//! the active Figma canvas, mirroring the original Fanta left sidebar.

use std::collections::hash_map::Entry;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::ops::Range;
use std::sync::Arc;

use anyhow::Result;
use editor::{
    Editor, EditorEvent,
    actions::{Cancel, SelectAll},
};
use fanta_doc::{
    AssetId, CanvasNode, ComponentId, Doc, Fill, GroupNode, IndexKey, NodeData, NodeFlags, NodeId,
    Operation, Scene,
};
use fs::Fs;
use gpui::{
    AnyElement, App, AsyncWindowContext, ClickEvent, Context, DragMoveEvent, Empty, Entity,
    EventEmitter, FocusHandle, Focusable, Image, KeyDownEvent, MouseButton, MouseDownEvent,
    MouseUpEvent, ObjectFit, Pixels, Role, ScrollStrategy, SharedString, Subscription,
    UniformListScrollHandle, WeakEntity, Window, actions, deferred, img, px, uniform_list,
};
use settings::{Settings as _, update_settings_file};
use ui::{ListHeader, ListItem, ListItemSpacing, Tooltip, prelude::*};
use util::size::format_file_size;
use workspace::{
    Workspace,
    dock::{DockPosition, Panel, PanelEvent},
};

use crate::document::{DocChange, FigDocument, FigPage, page_bounds};
use crate::panel_settings::FantaDesignPanelSettings;
use crate::view::{
    CopySelection, CutSelection, DeleteSelection, DuplicateSelection, FigView, PasteSelection,
};

actions!(
    fanta_design_panel,
    [
        /// Toggle focus on the Fanta design panel.
        ToggleFocus
    ]
);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Section {
    Pages,
    Layers,
    Components,
    Assets,
}

/// The design sidebar uses the same compact 28 px rhythm as Zed's outline and
/// project panels. Pinning the height also keeps `uniform_list` measurements in
/// sync with the rows it virtualizes.
const SECTION_ROW_HEIGHT: f32 = 28.;
const MIN_SECTION_HEIGHT: Pixels = px(56.);
const DEFAULT_PAGES_HEIGHT: Pixels = px(168.);
const DEFAULT_COMPONENTS_HEIGHT: Pixels = px(168.);
const DEFAULT_ASSETS_HEIGHT: Pixels = px(200.);
const DIVIDER_HITBOX_SIZE: Pixels = px(6.);
/// Ceiling on any single section so the other section headers and a usable
/// slice of the Layers list always stay visible while dragging.
const SECTION_RESIZE_RESERVE: Pixels = px(160.);
/// Base indent applied to every content row so rows align under their section
/// header instead of sitting flush against the panel edge. The indent lives
/// inside the row (`ListItem::indent_level`), keeping hover targets full width.
const SECTION_INDENT_STEP: Pixels = px(12.);
/// Asset rows are taller than the rest: they carry a thumbnail beside a
/// two-line label. `uniform_list` sizes every row from the first one, so the
/// row pins itself to this height and the list container is measured with it.
const ASSET_ROW_HEIGHT: f32 = 40.;
const ASSET_THUMBNAIL_SIZE: Pixels = px(28.);

fn section_list_height(row_count: usize, row_height: f32, stored: Pixels) -> Pixels {
    px((row_count as f32 * row_height).min(stored.as_f32()))
}

fn section_count_label(visible: usize, total: usize, filtered: bool) -> SharedString {
    if filtered && visible != total {
        SharedString::from(format!("{visible} / {total}"))
    } else {
        SharedString::from(visible.to_string())
    }
}

/// Keep the pages whose name contains `query` (already lowercased), preserving
/// each survivor's true `index` into `document.pages` so a filtered row still
/// selects, renames, and deletes the right page.
fn filter_pages(pages: Vec<PageEntry>, query: Option<&str>) -> Vec<PageEntry> {
    match query {
        Some(query) => pages
            .into_iter()
            .filter(|entry| entry.name.to_lowercase().contains(query))
            .collect(),
        None => pages,
    }
}

/// The draggable boundaries between sidebar sections. Each divider resizes the
/// fixed-height section adjacent to it while Layers (`flex_1`) absorbs the
/// remaining space.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SectionDivider {
    PagesLayers,
    LayersComponents,
    ComponentsAssets,
}

#[derive(Clone)]
struct DraggedSectionDivider(SectionDivider);

impl Render for DraggedSectionDivider {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        Empty
    }
}

#[derive(Clone)]
struct DraggedLayer {
    id: NodeId,
    name: SharedString,
    icon: IconName,
}

impl Render for DraggedLayer {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        h_flex().pl_3().pt_3().child(
            h_flex()
                .max_w(px(240.))
                .min_w_0()
                .h(px(SECTION_ROW_HEIGHT))
                .gap_1()
                .px_2()
                .rounded_lg()
                .border_1()
                .border_color(cx.theme().colors().border)
                .bg(cx.theme().colors().background)
                .shadow_md()
                .child(
                    Icon::new(self.icon)
                        .size(IconSize::Small)
                        .color(Color::Muted),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(Label::new(self.name.clone()).single_line().truncate()),
                ),
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LayerDropError {
    MissingDragged,
    MissingTarget,
    TargetIsNotContainer,
    TargetParentIsNotContainer,
    SelfDrop,
    DescendantCycle,
    PageRoot,
    ContainsComponentMaster,
    RecursiveComponentInstance,
    NonInvertibleTarget,
    AlreadyInPosition,
    IndexPrecisionExhausted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LayerDropPlacement {
    /// Above the target row, which is later (higher) in paint order because
    /// layer rows display the scene's bottom-first child order in reverse.
    Above,
    /// As the target container's topmost child.
    Inside,
    /// Below the target row, which is earlier (lower) in paint order.
    Below,
}

fn node_is_within(scene: &Scene, node: NodeId, ancestor: NodeId) -> bool {
    node == ancestor
        || scene
            .ancestors_of(node)
            .any(|candidate| candidate.id == ancestor)
}

fn insertion_index(
    scene: &Scene,
    siblings_without_dragged: &[NodeId],
    insertion_position: usize,
) -> std::result::Result<IndexKey, LayerDropError> {
    let previous = insertion_position
        .checked_sub(1)
        .and_then(|index| siblings_without_dragged.get(index))
        .and_then(|id| scene.get(*id))
        .map(|node| node.index);
    let next = siblings_without_dragged
        .get(insertion_position)
        .and_then(|id| scene.get(*id))
        .map(|node| node.index);
    match (previous, next) {
        (Some(previous), Some(next)) => {
            if IndexKey::near_precision_limit(previous, next) {
                return Err(LayerDropError::IndexPrecisionExhausted);
            }
            Ok(IndexKey::between(previous, next))
        }
        (Some(previous), None) => Ok(IndexKey::after(previous)),
        (None, Some(next)) => Ok(IndexKey::before(next)),
        (None, None) => Ok(IndexKey::FIRST),
    }
}

fn layer_move_operations(
    doc: &Doc,
    dragged: NodeId,
    target: NodeId,
    placement: LayerDropPlacement,
) -> std::result::Result<Vec<Operation>, LayerDropError> {
    let dragged_node = doc
        .scene
        .get(dragged)
        .ok_or(LayerDropError::MissingDragged)?;
    let target_node = doc.scene.get(target).ok_or(LayerDropError::MissingTarget)?;
    if dragged == target {
        return Err(LayerDropError::SelfDrop);
    }
    if doc.pages().contains(&dragged) {
        return Err(LayerDropError::PageRoot);
    }

    let (new_parent, siblings_without_dragged, insertion_position) = match placement {
        LayerDropPlacement::Inside => {
            if !matches!(target_node.data, NodeData::Group(_)) {
                return Err(LayerDropError::TargetIsNotContainer);
            }
            let siblings: Vec<_> = doc
                .scene
                .children_of(Some(target))
                .iter()
                .copied()
                .filter(|id| *id != dragged)
                .collect();
            let position = siblings.len();
            (Some(target), siblings, position)
        }
        LayerDropPlacement::Above | LayerDropPlacement::Below => {
            let parent = target_node.parent;
            if dragged_node.parent != parent
                && let Some(parent) = parent
                && !matches!(
                    doc.scene.get(parent).map(|node| &node.data),
                    Some(NodeData::Group(_))
                )
            {
                return Err(LayerDropError::TargetParentIsNotContainer);
            }
            let siblings: Vec<_> = doc
                .scene
                .children_of(parent)
                .iter()
                .copied()
                .filter(|id| *id != dragged)
                .collect();
            let target_position = siblings
                .iter()
                .position(|id| *id == target)
                .ok_or(LayerDropError::MissingTarget)?;
            let position = if placement == LayerDropPlacement::Above {
                target_position + 1
            } else {
                target_position
            };
            (parent, siblings, position)
        }
    };

    if new_parent.is_some_and(|parent| node_is_within(&doc.scene, parent, dragged)) {
        return Err(LayerDropError::DescendantCycle);
    }
    if doc
        .components
        .defs
        .values()
        .any(|definition| node_is_within(&doc.scene, definition.root, dragged))
    {
        return Err(LayerDropError::ContainsComponentMaster);
    }

    let target_components: Vec<_> = new_parent
        .into_iter()
        .flat_map(|parent| {
            doc.components
                .defs
                .values()
                .filter(move |definition| node_is_within(&doc.scene, parent, definition.root))
        })
        .collect();
    if !target_components.is_empty() {
        let creates_recursive_instance = doc.scene.descendants_of(dragged).any(|node_id| {
            let Some(NodeData::Instance(instance)) = doc.scene.get(node_id).map(|node| &node.data)
            else {
                return false;
            };
            target_components.iter().any(|definition| {
                instance.component == definition.id
                    || definition
                        .variant_of
                        .as_ref()
                        .is_some_and(|membership| instance.component == membership.set)
            })
        });
        if creates_recursive_instance {
            return Err(LayerDropError::RecursiveComponentInstance);
        }
    }

    let mut requested_order = siblings_without_dragged.clone();
    requested_order.insert(insertion_position, dragged);
    if dragged_node.parent == new_parent {
        if requested_order == doc.scene.children_of(new_parent) {
            return Err(LayerDropError::AlreadyInPosition);
        }
    }

    let (new_index, mut normalization_operations) =
        match insertion_index(&doc.scene, &siblings_without_dragged, insertion_position) {
            Ok(index) => (index, Vec::new()),
            Err(LayerDropError::IndexPrecisionExhausted) => {
                let mut operations = Vec::new();
                let mut dragged_index = IndexKey::FIRST;
                for (position, id) in requested_order.iter().copied().enumerate() {
                    let normalized = IndexKey::from_raw(position as f64 + 1.0);
                    if id == dragged {
                        dragged_index = normalized;
                        continue;
                    }
                    let Some(node) = doc.scene.get(id) else {
                        return Err(LayerDropError::MissingTarget);
                    };
                    if node.index != normalized {
                        operations.push(Operation::SetIndex {
                            id,
                            old: node.index,
                            new: normalized,
                        });
                    }
                }
                (dragged_index, operations)
            }
            Err(error) => return Err(error),
        };

    if dragged_node.parent == new_parent {
        if dragged_node.index != new_index {
            normalization_operations.push(Operation::SetIndex {
                id: dragged,
                old: dragged_node.index,
                new: new_index,
            });
        }
        return Ok(normalization_operations);
    }

    let world = doc
        .scene
        .world_transform(dragged)
        .ok_or(LayerDropError::MissingDragged)?;
    let target_world = match new_parent {
        Some(parent) => doc
            .scene
            .world_transform(parent)
            .ok_or(LayerDropError::MissingTarget)?,
        None => fanta_doc::Transform2D::IDENTITY,
    };
    let [a, b, c, d, _, _] = target_world.to_components();
    let determinant = a * d - b * c;
    if !target_world.is_finite() || !determinant.is_finite() || determinant.abs() <= f64::EPSILON {
        return Err(LayerDropError::NonInvertibleTarget);
    }
    let new_local = world.then(&target_world.inverse());
    if !new_local.is_finite() {
        return Err(LayerDropError::NonInvertibleTarget);
    }

    normalization_operations.push(Operation::Reparent {
        id: dragged,
        old_parent: dragged_node.parent,
        old_index: dragged_node.index,
        new_parent,
        new_index,
    });
    if new_local != dragged_node.transform {
        normalization_operations.push(Operation::SetTransform {
            id: dragged,
            old: dragged_node.transform,
            new: new_local,
        });
    }
    Ok(normalization_operations)
}

#[derive(Clone, Copy)]
struct DividerDragState {
    divider: SectionDivider,
    start_mouse_y: Pixels,
    start_height: Pixels,
}

/// One row of the Assets section. Everything here is precomputed in
/// [`FantaDesignPanel::rebuild_assets`] — decoding, hashing, or scanning the
/// scene from the row builder would run once per visible row per frame.
struct AssetEntry {
    id: AssetId,
    /// The name of the layer that references this asset, or a synthetic
    /// `"Image {n}"` when nothing in the scene uses it.
    name: SharedString,
    /// The encoded bytes wrapped for GPUI, which decodes and caches the
    /// texture behind the content hash. `None` when GPUI has no decoder for
    /// the format, in which case the row falls back to a generic glyph.
    image: Option<Arc<Image>>,
    /// Natural pixel size, from the decoded asset. `None` if it failed to
    /// decode at load.
    dimensions: Option<(u32, u32)>,
    /// `"PNG"`, `"JPG"`, … Empty when the encoding could not be identified.
    format_label: SharedString,
    byte_count: usize,
}

/// A name recovered for an asset from the scene node that references it.
struct AssetName {
    name: SharedString,
    /// A `Bitmap` layer names its asset directly; a shape painted with an image
    /// *fill* only lends its own name. Tracked so the former can upgrade the
    /// latter regardless of which is met first in scene order.
    from_bitmap: bool,
}

/// One visible row of the flattened layer tree, rebuilt every render.
struct LayerRow {
    id: NodeId,
    depth: usize,
    has_children: bool,
    expanded: bool,
    icon: IconName,
    name: SharedString,
    /// Instances and component masters get the accent tint, mirroring the
    /// original panel's violet/purple layer names.
    accent: bool,
    hidden: bool,
    locked: bool,
    selected: bool,
}

pub struct FantaDesignPanel {
    focus_handle: FocusHandle,
    fs: Arc<dyn Fs>,
    active_view: Option<WeakEntity<FigView>>,
    width: Option<Pixels>,
    expanded_nodes: HashSet<NodeId>,
    collapsed_sections: HashSet<Section>,
    // Cached section state, rebuilt on document events (never in render — see
    // `render_sections`).
    layer_rows: Vec<LayerRow>,
    pages_cache: Vec<PageEntry>,
    components_cache: Vec<(SharedString, NodeId)>,
    components_total_count: usize,
    assets_cache: Vec<AssetEntry>,
    /// The `raw_assets` map `assets_cache`'s thumbnails were built from.
    /// `Image::from_bytes` content-hashes every asset byte, so the thumbnails
    /// are rebuilt only when the document swaps its (immutable) asset map on
    /// load — not on every selection change, which also rebuilds this panel.
    assets_source: Option<Arc<BTreeMap<AssetId, Vec<u8>>>>,
    /// The scene revision `assets_cache`'s recovered names were resolved at.
    /// Recovering names is a whole-scene walk, so it reruns only when the scene
    /// actually changed — not on the selection changes that also rebuild here.
    assets_names_revision: Option<u64>,
    /// Indices into `assets_cache` surviving the Assets filter. The cache
    /// itself stays whole so filtering never invalidates the thumbnails.
    visible_assets: Vec<usize>,
    current_page_index: Option<usize>,
    document_editable: bool,
    document_ready: bool,
    layers_scroll_handle: UniformListScrollHandle,
    last_reveal_anchor: Option<NodeId>,
    pages_height: Pixels,
    components_height: Pixels,
    assets_height: Pixels,
    divider_drag: Option<DividerDragState>,
    filter_editor: Entity<Editor>,
    filter_target: Option<Section>,
    /// Scope the Components list to masters actually used (instanced) on the
    /// page being viewed, rather than every master in the document.
    components_this_page: bool,
    /// The page or layer being renamed inline, if any; the shared
    /// `rename_editor` carries the edited text.
    renaming: Option<RenameTarget>,
    rename_editor: Entity<Editor>,
    _subscriptions: Vec<Subscription>,
    _active_view_subscription: Option<Subscription>,
}

/// A row in the Pages section. `index` is the true index into
/// `document.pages` (preserved across a filter), and `root` is the page's scene
/// node — `None` for the synthetic single page of a doc without explicit page
/// roots, which cannot be renamed or deleted.
#[derive(Clone)]
struct PageEntry {
    name: SharedString,
    root: Option<NodeId>,
    index: usize,
}

/// An inline rename in progress. Both a page and a layer rename a scene node by
/// id (via `SetName`); a page also tracks its row index so the editor renders in
/// the right Pages row.
#[derive(Clone, Copy)]
enum RenameTarget {
    Page { index: usize, root: NodeId },
    Layer { node: NodeId },
}

impl RenameTarget {
    /// The scene node whose name is being edited.
    fn node(&self) -> NodeId {
        match *self {
            RenameTarget::Page { root, .. } => root,
            RenameTarget::Layer { node } => node,
        }
    }
}

impl FantaDesignPanel {
    pub async fn load(
        workspace: WeakEntity<Workspace>,
        mut cx: AsyncWindowContext,
    ) -> Result<Entity<Self>> {
        workspace.update_in(&mut cx, |workspace, window, cx| {
            Self::new(workspace, window, cx)
        })
    }

    pub(crate) fn new_embedded(
        active_view: Entity<FigView>,
        fs: Arc<dyn Fs>,
        window: &mut Window,
        cx: &mut Context<FigView>,
    ) -> Entity<Self> {
        let panel = cx.new(|cx| Self::build(fs, None, window, cx, Vec::new()));
        cx.defer({
            let panel = panel.clone();
            move |cx| {
                let _ = panel.update(cx, |panel, cx| {
                    panel.set_active_view(Some(active_view), cx);
                });
            }
        });
        panel
    }

    fn new(
        workspace: &mut Workspace,
        window: &mut Window,
        cx: &mut Context<Workspace>,
    ) -> Entity<Self> {
        let fs = workspace.app_state().fs.clone();
        // The workspace entity is mid-update here, so the initial active view
        // must come from the `&mut Workspace` we were handed — reading the
        // entity would double-lease and panic.
        let initial_view = workspace
            .active_item(cx)
            .and_then(|item| item.downcast::<FigView>());
        let workspace_entity = cx.entity();
        cx.new(|cx| {
            let workspace_subscription = cx.subscribe_in(
                &workspace_entity,
                window,
                |this: &mut Self, workspace, event, window, cx| {
                    if matches!(event, workspace::Event::ActiveItemChanged) {
                        this.update_active_view(workspace, window, cx);
                    }
                },
            );
            Self::build(fs, initial_view, window, cx, vec![workspace_subscription])
        })
    }

    fn build(
        fs: Arc<dyn Fs>,
        initial_view: Option<Entity<FigView>>,
        window: &mut Window,
        cx: &mut Context<Self>,
        mut subscriptions: Vec<Subscription>,
    ) -> Self {
        let filter_editor = cx.new(|cx| Editor::single_line(window, cx));
        subscriptions.push(cx.subscribe(
            &filter_editor,
            |this: &mut Self, _, event: &EditorEvent, cx| {
                if matches!(event, EditorEvent::BufferEdited) {
                    this.rebuild_layer_rows(cx);
                    cx.notify();
                }
            },
        ));
        let rename_editor = cx.new(|cx| Editor::single_line(window, cx));
        // Clicking away from the rename field keeps the typed name (Figma /
        // Finder behavior). Enter/Escape empty `renaming_page` first, so the
        // blur they trigger is a no-op.
        subscriptions.push(cx.subscribe_in(
            &rename_editor,
            window,
            |this: &mut Self, _, event: &EditorEvent, window, cx| {
                if matches!(event, EditorEvent::Blurred) && this.renaming.is_some() {
                    this.commit_rename(window, cx);
                }
            },
        ));
        let mut this = Self {
            focus_handle: cx.focus_handle(),
            fs,
            active_view: None,
            width: None,
            expanded_nodes: HashSet::new(),
            collapsed_sections: HashSet::new(),
            layer_rows: Vec::new(),
            pages_cache: Vec::new(),
            components_cache: Vec::new(),
            components_total_count: 0,
            assets_cache: Vec::new(),
            assets_source: None,
            assets_names_revision: None,
            visible_assets: Vec::new(),
            current_page_index: None,
            document_ready: false,
            document_editable: false,
            layers_scroll_handle: UniformListScrollHandle::new(),
            last_reveal_anchor: None,
            pages_height: DEFAULT_PAGES_HEIGHT,
            components_height: DEFAULT_COMPONENTS_HEIGHT,
            assets_height: DEFAULT_ASSETS_HEIGHT,
            divider_drag: None,
            filter_editor,
            filter_target: None,
            components_this_page: false,
            renaming: None,
            rename_editor,
            _subscriptions: subscriptions,
            _active_view_subscription: None,
        };
        this.set_active_view(initial_view, cx);
        this
    }

    fn update_active_view(
        &mut self,
        workspace: &Entity<Workspace>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let active_view = workspace
            .read(cx)
            .active_item(cx)
            .and_then(|item| item.downcast::<FigView>());
        self.set_active_view(active_view, cx);
    }

    fn set_active_view(&mut self, active_view: Option<Entity<FigView>>, cx: &mut Context<Self>) {
        match active_view {
            Some(view) => {
                let is_same = self
                    .active_view
                    .as_ref()
                    .is_some_and(|previous| previous.entity_id() == view.entity_id());
                if !is_same {
                    // Subscribe to the item's event stream rather than
                    // observing the view: the view notifies on every pan and
                    // pointer-move frame. The layer rows are rebuilt HERE, on
                    // document events, never in render — preview frames can't
                    // change tree structure, so they're skipped too.
                    let item = view.read(cx).item().clone();
                    self._active_view_subscription = Some(cx.subscribe(
                        &item,
                        |this, _, event: &crate::document::FigItemEvent, cx| {
                            if !matches!(
                                event,
                                crate::document::FigItemEvent::EditedTransient
                                    | crate::document::FigItemEvent::TextSelectionChanged
                            ) {
                                this.rebuild_layer_rows(cx);
                                cx.notify();
                            }
                        },
                    ));
                    self.active_view = Some(view.downgrade());
                    self.expanded_nodes.clear();
                    self.last_reveal_anchor = None;
                    self.rebuild_layer_rows(cx);
                }
            }
            None => {
                // Keep the last canvas bound while the user visits other
                // items, matching how the outline panel retains its editor.
            }
        }
        cx.notify();
    }

    fn active_view(&self, _cx: &App) -> Option<Entity<FigView>> {
        self.active_view.as_ref().and_then(|view| view.upgrade())
    }

    // === Section and tree state ===========================================

    fn section_open(&self, section: Section) -> bool {
        !self.collapsed_sections.contains(&section)
    }

    fn toggle_section(&mut self, section: Section, window: &mut Window, cx: &mut Context<Self>) {
        if self.collapsed_sections.remove(&section) {
            cx.notify();
            return;
        }

        if self.filter_target == Some(section) {
            self.close_filter(window, cx);
        }
        self.collapsed_sections.insert(section);
        cx.notify();
    }

    fn toggle_node_expanded(&mut self, id: NodeId, cx: &mut Context<Self>) {
        if !self.expanded_nodes.remove(&id) {
            self.expanded_nodes.insert(id);
        }
        self.rebuild_layer_rows(cx);
        cx.notify();
    }

    // === Search / filter ====================================================

    /// The active, non-empty filter query for `section`, lowercased for
    /// case-insensitive matching.
    fn filter_query(&self, section: Section, cx: &App) -> Option<String> {
        if self.filter_target != Some(section) {
            return None;
        }
        let text = self.filter_editor.read(cx).text(cx);
        let query = text.trim().to_lowercase();
        (!query.is_empty()).then_some(query)
    }

    fn toggle_components_this_page(&mut self, cx: &mut Context<Self>) {
        self.components_this_page = !self.components_this_page;
        self.rebuild_layer_rows(cx);
        cx.notify();
    }

    fn toggle_filter(&mut self, section: Section, window: &mut Window, cx: &mut Context<Self>) {
        if self.filter_target == Some(section) {
            self.close_filter(window, cx);
            return;
        }
        // A header remains actionable while collapsed. Reveal the editor before
        // focusing it so keyboard input can never be captured by invisible UI.
        self.collapsed_sections.remove(&section);
        self.filter_target = Some(section);
        self.filter_editor.update(cx, |editor, cx| {
            editor.set_text("", window, cx);
            // Exhaustive so a new section can't silently inherit the wrong hint.
            let placeholder = match section {
                Section::Pages => "Filter pages…",
                Section::Layers => "Filter layers…",
                Section::Components => "Filter components…",
                Section::Assets => "Filter assets…",
            };
            editor.set_placeholder_text(placeholder, window, cx);
        });
        self.filter_editor.focus_handle(cx).focus(window, cx);
        self.rebuild_layer_rows(cx);
        cx.notify();
    }

    fn close_filter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.filter_target.take().is_none() {
            return;
        }
        self.filter_editor
            .update(cx, |editor, cx| editor.set_text("", window, cx));
        if self
            .filter_editor
            .focus_handle(cx)
            .contains_focused(window, cx)
        {
            self.focus_handle.focus(window, cx);
        }
        // A node selected from the flat results may sit in a collapsed branch;
        // clearing the reveal anchor makes the rebuild expand and scroll to it.
        self.last_reveal_anchor = None;
        self.rebuild_layer_rows(cx);
        cx.notify();
    }

    // === Section resizing ===================================================

    fn divider_resizable(&self, divider: SectionDivider) -> bool {
        // Each divider resizes exactly one fixed-height section; when that
        // section is collapsed there is nothing to resize.
        match divider {
            SectionDivider::PagesLayers => self.section_open(Section::Pages),
            SectionDivider::LayersComponents => self.section_open(Section::Components),
            SectionDivider::ComponentsAssets => self.section_open(Section::Assets),
        }
    }

    /// The height the divider's section is currently rendered at, which can be
    /// smaller than the stored height when the list is short. Starting drags
    /// from this value keeps the divider tracking the pointer.
    fn section_rendered_height(&self, divider: SectionDivider) -> Pixels {
        match divider {
            SectionDivider::PagesLayers => section_list_height(
                self.pages_cache.len(),
                SECTION_ROW_HEIGHT,
                self.pages_height,
            ),
            SectionDivider::LayersComponents => section_list_height(
                self.components_cache.len(),
                SECTION_ROW_HEIGHT,
                self.components_height,
            ),
            SectionDivider::ComponentsAssets => section_list_height(
                self.visible_assets.len(),
                ASSET_ROW_HEIGHT,
                self.assets_height,
            ),
        }
    }

    fn set_section_height(&mut self, divider: SectionDivider, height: Pixels) {
        match divider {
            SectionDivider::PagesLayers => self.pages_height = height,
            SectionDivider::LayersComponents => self.components_height = height,
            SectionDivider::ComponentsAssets => self.assets_height = height,
        }
    }

    fn reset_section_height(&mut self, divider: SectionDivider, cx: &mut Context<Self>) {
        let default_height = match divider {
            SectionDivider::PagesLayers => DEFAULT_PAGES_HEIGHT,
            SectionDivider::LayersComponents => DEFAULT_COMPONENTS_HEIGHT,
            SectionDivider::ComponentsAssets => DEFAULT_ASSETS_HEIGHT,
        };
        self.set_section_height(divider, default_height);
        cx.notify();
    }

    fn handle_divider_drag(
        &mut self,
        event: &DragMoveEvent<DraggedSectionDivider>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let dragged_divider = event.drag(cx).0;
        let Some(drag_state) = self.divider_drag else {
            return;
        };
        if drag_state.divider != dragged_divider {
            return;
        }
        // Dragging the boundary down grows the section above it (Pages) or
        // shrinks the section below it (Components, Assets); Layers flexes.
        let delta = event.event.position.y - drag_state.start_mouse_y;
        let proposed = match dragged_divider {
            SectionDivider::PagesLayers => drag_state.start_height + delta,
            SectionDivider::LayersComponents | SectionDivider::ComponentsAssets => {
                drag_state.start_height - delta
            }
        };
        let max_height =
            (event.bounds.size.height - SECTION_RESIZE_RESERVE).max(MIN_SECTION_HEIGHT);
        self.set_section_height(
            dragged_divider,
            proposed.clamp(MIN_SECTION_HEIGHT, max_height),
        );
        cx.notify();
    }

    // === Document mutations ================================================

    fn select_page(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(view) = self.active_view(cx) else {
            return;
        };
        view.update(cx, |view, cx| {
            view.finish_document_edits_for_external_change(cx);
            view.select_page(index, cx);
        });
    }

    fn select_node(&mut self, id: NodeId, extend: bool, cx: &mut Context<Self>) {
        let Some(view) = self.active_view(cx) else {
            return;
        };
        view.update(cx, |view, cx| {
            view.finish_document_edits_for_external_change(cx);
        });
        let item = view.read(cx).item().clone();
        item.update(cx, |item, cx| {
            item.with_document(cx, |document| {
                if extend {
                    document.doc.selection.toggle(id);
                } else {
                    document.doc.selection.select_only(id);
                }
                ((), DocChange::Selection)
            });
        });
        // A plain click on a row reveals the node in the canvas (the panel →
        // canvas half of the sync); extending a multi-selection does not, so
        // shift-clicking down the list doesn't yank the viewport around.
        if !extend {
            view.update(cx, |view, cx| view.reveal_node_in_canvas(id, cx));
        }
    }

    fn drop_layer(
        &mut self,
        dragged: NodeId,
        target: NodeId,
        placement: LayerDropPlacement,
        cx: &mut Context<Self>,
    ) {
        let Some(view) = self.active_view(cx) else {
            return;
        };
        view.update(cx, |view, cx| {
            view.finish_document_edits_for_external_change(cx);
        });
        let item = view.read(cx).item().clone();
        let result: Result<bool> = item.update(cx, |item, cx| {
            if !item.is_editable() {
                return Ok(false);
            }
            item.with_document(cx, |document| {
                let operations =
                    match layer_move_operations(&document.doc, dragged, target, placement) {
                        Ok(operations) => operations,
                        Err(_) => return (Ok(false), DocChange::None),
                    };
                let doc = &mut document.doc;
                doc.history.begin("Move layer", &mut doc.scene);
                for operation in operations {
                    if let Err(error) = doc.apply(operation) {
                        let rollback = doc.history.abort(&mut doc.scene);
                        let error = match rollback {
                            Ok(()) => anyhow::anyhow!("reparenting layer: {error}"),
                            Err(rollback_error) => anyhow::anyhow!(
                                "reparenting layer: {error}; rolling back: {rollback_error}"
                            ),
                        };
                        return (Err(error), DocChange::None);
                    }
                }
                doc.history.commit(&mut doc.scene);
                (Ok(true), DocChange::Content)
            })
            .unwrap_or_else(|| Err(anyhow::anyhow!("the document is no longer available")))
        });
        match result {
            Ok(true) => {
                if placement == LayerDropPlacement::Inside {
                    self.expanded_nodes.insert(target);
                }
                self.rebuild_layer_rows(cx);
                cx.notify();
            }
            Ok(false) => {}
            Err(error) => log::error!("fanta design panel: failed to move layer: {error:#}"),
        }
    }

    /// Jump to a component master: switch to the page that holds it (the
    /// importer keeps masters on a hidden Components page), center it in the
    /// canvas, and select it.
    fn focus_component(&mut self, target: NodeId, cx: &mut Context<Self>) {
        let Some(view) = self.active_view(cx) else {
            return;
        };
        view.update(cx, |view, cx| {
            view.finish_document_edits_for_external_change(cx);
        });
        let item = view.read(cx).item().clone();
        let selected_page = view.read(cx).selected_page_index();
        let (page_index, current_index) = {
            let fig_item = item.read(cx);
            let Some(document) = fig_item.document() else {
                return;
            };
            (
                document.page_index_of_node(target),
                document.page_index(selected_page),
            )
        };
        view.update(cx, |view, cx| {
            if let Some(index) = page_index
                && Some(index) != current_index
            {
                view.select_page(index, cx);
            }
            view.focus_node(target, cx);
        });
        item.update(cx, |item, cx| {
            item.with_document(cx, |document| {
                document.doc.selection.select_only(target);
                ((), DocChange::Selection)
            });
        });
    }

    fn toggle_node_flag(&mut self, id: NodeId, flag: NodeFlags, cx: &mut Context<Self>) {
        let Some(view) = self.active_view(cx) else {
            return;
        };
        view.update(cx, |view, cx| {
            view.finish_document_edits_for_external_change(cx);
        });
        let item = view.read(cx).item().clone();
        item.update(cx, |item, cx| {
            if !item.is_editable() {
                return;
            }
            let Some(old) = item
                .document()
                .and_then(|document| document.doc.scene.get(id))
                .map(|node| node.flags)
            else {
                return;
            };
            let turning_on = !old.contains(flag);
            let operation = Operation::SetFlags {
                id,
                old,
                new: old ^ flag,
            };
            if let Err(error) = item.apply(operation, cx) {
                log::error!("fanta design panel: failed to toggle layer flags: {error:#}");
                return;
            }
            // Hiding or locking a node drops it from the selection so it can't
            // be nudged or dragged while it is non-interactive, matching the
            // canvas hit-test which no longer targets hidden/locked nodes.
            if turning_on {
                item.with_document(cx, |document| {
                    let was_selected = document.doc.selection.contains(id);
                    document.doc.selection.remove(id);
                    (
                        (),
                        if was_selected {
                            DocChange::Selection
                        } else {
                            DocChange::None
                        },
                    )
                });
            }
        });
    }

    fn add_page(&mut self, cx: &mut Context<Self>) {
        let Some(view) = self.active_view(cx) else {
            return;
        };
        let item = view.read(cx).item().clone();
        if !item.read(cx).is_editable() {
            return;
        }
        view.update(cx, |view, cx| {
            view.finish_document_edits_for_external_change(cx);
        });
        let Some((page_node, page_name)) = ({
            let fig_item = item.read(cx);
            fig_item.document().and_then(|document| {
                // A doc without explicit page roots renders every root as one
                // implicit page; adding a real page there would hide all of
                // that content behind the new empty active page.
                if !document.pages.iter().all(|page| page.root.is_some()) {
                    return None;
                }
                let mut page_node = CanvasNode::new(NodeData::Group(GroupNode::default()));
                let visible_count = document.pages.iter().filter(|page| !page.hidden).count();
                page_node.name = format!("Page {}", visible_count + 1);
                page_node.index = document.doc.scene.next_root_index();
                let page_name = SharedString::from(page_node.name.clone());
                Some((page_node, page_name))
            })
        }) else {
            return;
        };
        let root = page_node.id;

        let applied = item.update(cx, |item, cx| {
            item.apply(Operation::create_node(page_node), cx)
        });
        if let Err(error) = applied {
            log::error!("fanta design panel: failed to add a page: {error:#}");
            return;
        }
        let new_page_index = item.update(cx, |item, cx| {
            item.with_document(cx, |document| {
                document.doc.add_page(root);
                document.doc.set_active_page(Some(root));
                let bounds = page_bounds(&document.doc, Some(root));
                document.pages.push(FigPage {
                    root: Some(root),
                    name: page_name,
                    bounds,
                    hidden: false,
                });
                (document.pages.len() - 1, DocChange::Content)
            })
        });
        if let Some(new_page_index) = new_page_index {
            view.update(cx, |view, cx| view.select_page(new_page_index, cx));
        }
    }

    fn delete_page(&mut self, page_index: usize, cx: &mut Context<Self>) {
        let Some(view) = self.active_view(cx) else {
            return;
        };
        let item = view.read(cx).item().clone();
        if !item.read(cx).is_editable() {
            return;
        }
        view.update(cx, |view, cx| {
            view.finish_document_edits_for_external_change(cx);
        });
        let Some((root, snapshot)) = ({
            let fig_item = item.read(cx);
            fig_item.document().and_then(|document| {
                if document.pages.len() <= 1 {
                    return None;
                }
                let root = document.pages.get(page_index)?.root?;
                // `descendants_of` yields the page root first, which is what
                // `DeleteSubtree` expects its snapshot to start with.
                let snapshot: Vec<CanvasNode> = document
                    .doc
                    .scene
                    .descendants_of(root)
                    .filter_map(|node_id| document.doc.scene.get(node_id).cloned())
                    .collect();
                (!snapshot.is_empty()).then_some((root, snapshot))
            })
        }) else {
            return;
        };

        let applied = item.update(cx, |item, cx| {
            item.apply(Operation::DeleteSubtree { snapshot }, cx)
        });
        if let Err(error) = applied {
            log::error!("fanta design panel: failed to delete the page: {error:#}");
            return;
        }
        let next_page_index = item.update(cx, |item, cx| {
            item.with_document(cx, |document| {
                document.doc.remove_page(root);
                if page_index < document.pages.len() {
                    document.pages.remove(page_index);
                }
                let active_root = document.doc.active_page();
                let next_page_index = document
                    .pages
                    .iter()
                    .position(|page| page.root == active_root)
                    .unwrap_or(0);
                (next_page_index, DocChange::Content)
            })
        });
        if let Some(next_page_index) = next_page_index {
            view.update(cx, |view, cx| view.select_page(next_page_index, cx));
        }
    }

    // === Layer tree flattening =============================================

    fn rebuild_layer_rows(&mut self, cx: &mut App) {
        self.layer_rows.clear();
        self.pages_cache.clear();
        self.components_cache.clear();
        self.components_total_count = 0;
        self.current_page_index = None;
        self.document_editable = false;
        self.document_ready = false;
        let Some(view) = self.active_view(cx) else {
            self.last_reveal_anchor = None;
            self.clear_assets(cx);
            return;
        };
        // Capture document readiness in a scoped read so `cx` stays free (as
        // `&mut App`) to evict the previous document's cached thumbnails when
        // the active view has no ready document.
        let document_ready = {
            let fig_item = view.read(cx).item().read(cx);
            self.document_editable = fig_item.is_editable();
            fig_item.document().is_some()
        };
        if !document_ready {
            self.clear_assets(cx);
            return;
        }
        // The Pages/Components/Layers caches are pure reads of the document; the
        // Assets cache is rebuilt after, since evicting the outgoing document's
        // thumbnails needs `&mut App` and cannot run while `document` borrows it.
        self.rebuild_tree(&view, cx);
        self.rebuild_assets(&view, cx);
    }

    /// Flatten the active document's Pages, Components, and Layers into their
    /// caches. A pure read of the document — Assets are rebuilt separately (see
    /// [`FantaDesignPanel::rebuild_assets`]), as they need `&mut App` to evict
    /// thumbnails.
    fn rebuild_tree(&mut self, view: &Entity<FigView>, cx: &App) {
        let view = view.read(cx);
        let fig_item = view.item().read(cx);
        let Some(document) = fig_item.document() else {
            return;
        };
        self.document_ready = true;
        let doc = &document.doc;
        let page_root = document
            .page(view.selected_page_index())
            .and_then(|page| page.root);

        self.current_page_index = document.page_index(view.selected_page_index());
        // Read each real page's name live from its scene node so an inline
        // rename (and its undo) reflects without maintaining the `FigPage`
        // cache; the synthetic page (root `None`) falls back to its stored name.
        self.pages_cache = document
            .pages
            .iter()
            .enumerate()
            // Hidden library pages (e.g. the Components page) stay navigable via
            // the canvas but never appear in the Pages panel.
            .filter(|(_, page)| !page.hidden)
            .map(|(index, page)| {
                let name = page
                    .root
                    .and_then(|root| doc.scene.get(root))
                    .map(|node| SharedString::from(node.name.clone()))
                    .unwrap_or_else(|| page.name.clone());
                PageEntry {
                    name,
                    root: page.root,
                    index,
                }
            })
            .collect();
        // Abandon an inline rename whose node no longer exists (deleted, or the
        // document reloaded from disk).
        if let Some(rename) = self.renaming
            && doc.scene.get(rename.node()).is_none()
        {
            self.renaming = None;
        }
        // When scoping to the current page, collect the master/set ids present
        // on it — both instances placed there AND masters *defined* there. A
        // design system's Icons/Typography page holds the component masters
        // themselves (the importer only relocates orphaned library masters to
        // the hidden Components page), so counting instances alone would show
        // nothing there. A variant (instance or master) counts toward its set.
        let used_on_page: Option<HashSet<ComponentId>> = (self.components_this_page).then(|| {
            let root_to_component: HashMap<NodeId, ComponentId> = doc
                .components
                .defs
                .values()
                .map(|def| (def.root, def.id))
                .collect();
            let master_of = |component: ComponentId| {
                doc.components
                    .defs
                    .get(&component)
                    .and_then(|def| def.variant_of.as_ref().map(|m| m.set))
                    .unwrap_or(component)
            };
            let mut used = HashSet::new();
            if let Some(root) = page_root {
                for node_id in doc.scene.descendants_of(root) {
                    // A component master defined on this page.
                    if let Some(&component) = root_to_component.get(&node_id) {
                        used.insert(master_of(component));
                    }
                    // An instance placed on this page.
                    if let Some(node) = doc.scene.get(node_id)
                        && let NodeData::Instance(instance) = &node.data
                    {
                        used.insert(master_of(instance.component));
                    }
                }
            }
            used
        });
        let is_used = |id: ComponentId| used_on_page.as_ref().is_none_or(|ids| ids.contains(&id));

        // Show only component MASTERS: standalone components, plus one row per
        // variant SET (its variants collapse into it). Individual variants and
        // instances are never listed — a set like Button has thousands of
        // variants but is one master to the user.
        let mut components: Vec<(SharedString, NodeId)> = Vec::new();
        for def in doc.components.defs.values() {
            if def.variant_of.is_none() && is_used(def.id) {
                components.push((SharedString::from(def.name.clone()), def.root));
            }
        }
        for set in doc.components.sets.values() {
            if !is_used(set.id) {
                continue;
            }
            // Navigate to the set's frame — a member variant's parent — so all
            // its variants come into view, not just one.
            let member_root = doc
                .components
                .defs
                .get(&set.default_variant)
                .or_else(|| {
                    set.members
                        .first()
                        .and_then(|id| doc.components.defs.get(id))
                })
                .map(|def| def.root);
            let Some(member_root) = member_root else {
                continue;
            };
            let target = doc
                .scene
                .get(member_root)
                .and_then(|node| node.parent)
                .unwrap_or(member_root);
            components.push((SharedString::from(set.name.clone()), target));
        }
        self.components_cache = components;
        self.components_cache
            .sort_by(|left, right| left.0.as_ref().cmp(right.0.as_ref()));
        self.components_total_count = self.components_cache.len();
        if let Some(query) = self.filter_query(Section::Components, cx) {
            self.components_cache
                .retain(|(name, _)| name.to_lowercase().contains(&query));
        }
        let layers_query = self.filter_query(Section::Layers, cx);

        // Reveal the selection: when the anchor changes (typically from a
        // canvas click), expand its ancestor chain so its row exists, then
        // scroll it into view once the rows are rebuilt. While filtering, the
        // list is flat, so expansion is skipped and only the scroll applies.
        let anchor = doc.selection.anchor();
        let mut reveal_target = None;
        if anchor != self.last_reveal_anchor {
            self.last_reveal_anchor = anchor;
            if let Some(anchor) = anchor {
                if layers_query.is_none() {
                    for ancestor in doc.scene.ancestors_of(anchor) {
                        self.expanded_nodes.insert(ancestor.id);
                    }
                }
                reveal_target = Some(anchor);
            }
        }

        let component_roots: HashSet<NodeId> =
            doc.components.defs.values().map(|def| def.root).collect();

        // Children are stored bottom-first in z-order; popping the stack from
        // the end lists the topmost sibling first, like the original panel.
        let mut stack: Vec<(NodeId, usize)> = match page_root {
            Some(root) => doc
                .scene
                .children_of(Some(root))
                .iter()
                .map(|id| (*id, 0))
                .collect(),
            None => doc.scene.roots().iter().map(|id| (*id, 0)).collect(),
        };
        if let Some(query) = &layers_query {
            // Filtered rows are flat, like the outline panel's search results:
            // every matching node on the page appears at depth zero.
            while let Some((id, _)) = stack.pop() {
                let Some(node) = doc.scene.get(id) else {
                    continue;
                };
                stack.extend(
                    doc.scene
                        .children_of(Some(id))
                        .iter()
                        .map(|child| (*child, 0)),
                );
                if !node.name.to_lowercase().contains(query) {
                    continue;
                }
                self.layer_rows.push(LayerRow {
                    id,
                    depth: 0,
                    has_children: false,
                    expanded: false,
                    icon: layer_icon(node),
                    name: SharedString::from(node.name.clone()),
                    accent: matches!(node.data, NodeData::Instance(_))
                        || component_roots.contains(&id),
                    hidden: node.flags.contains(NodeFlags::HIDDEN),
                    locked: node.flags.contains(NodeFlags::LOCKED),
                    selected: doc.selection.contains(id),
                });
            }
        } else {
            while let Some((id, depth)) = stack.pop() {
                let Some(node) = doc.scene.get(id) else {
                    continue;
                };
                let children = doc.scene.children_of(Some(id));
                let has_children = !children.is_empty();
                let expanded = has_children && self.expanded_nodes.contains(&id);
                self.layer_rows.push(LayerRow {
                    id,
                    depth,
                    has_children,
                    expanded,
                    icon: layer_icon(node),
                    name: SharedString::from(node.name.clone()),
                    accent: matches!(node.data, NodeData::Instance(_))
                        || component_roots.contains(&id),
                    hidden: node.flags.contains(NodeFlags::HIDDEN),
                    locked: node.flags.contains(NodeFlags::LOCKED),
                    selected: doc.selection.contains(id),
                });
                if expanded {
                    stack.extend(children.iter().map(|child| (*child, depth + 1)));
                }
            }
        }

        if let Some(reveal_target) = reveal_target
            && let Some(index) = self
                .layer_rows
                .iter()
                .position(|row| row.id == reveal_target)
        {
            self.layers_scroll_handle
                .scroll_to_item(index, ScrollStrategy::Center);
        }
    }

    // === Assets =============================================================

    /// Evict the current thumbnails from GPUI's global asset cache, then clear
    /// the Assets rows. GPUI keys a decoded texture by its `Image`'s content
    /// hash and does NOT free it when the `Arc<Image>` drops, so a swap to
    /// another document (or losing the active document) must remove them.
    fn clear_assets(&mut self, cx: &mut App) {
        self.evict_asset_thumbnails(cx);
        self.assets_source = None;
        self.assets_names_revision = None;
        self.visible_assets.clear();
    }

    /// Drop the cached-thumbnail entries, removing each decoded texture from
    /// GPUI's global asset cache. `remove_asset` is a no-op for a thumbnail that
    /// was never scrolled into view (and so was never decoded).
    fn evict_asset_thumbnails(&mut self, cx: &mut App) {
        for entry in self.assets_cache.drain(..) {
            if let Some(image) = entry.image {
                image.remove_asset(cx);
            }
        }
    }

    /// Refresh the Assets rows for the active view's document. Thumbnails,
    /// dimensions, and formats are derived from the (immutable) asset bytes, so
    /// they are rebuilt only when the document swaps its asset map on load —
    /// evicting the previous map's cached textures as it does. Only the
    /// recovered names (which track layer renames and their undo) and the filter
    /// are recomputed on every rebuild.
    fn rebuild_assets(&mut self, view: &Entity<FigView>, cx: &mut App) {
        // Everything that reads the document happens first, under a scoped
        // shared borrow. The swap path defers installing the freshly built
        // entries until that borrow is released, so the outgoing document's
        // thumbnails can be evicted with `&mut App`.
        let rebuilt = {
            let fig_item = view.read(cx).item().read(cx);
            let Some(document) = fig_item.document() else {
                return;
            };
            let same_assets = self
                .assets_source
                .as_ref()
                .is_some_and(|source| Arc::ptr_eq(source, &document.raw_assets));
            if same_assets {
                // The same immutable asset map: no thumbnails to rebuild or
                // evict. Only the scene-gated names and the filter can change.
                self.refresh_asset_names(document);
                self.filter_assets(cx);
                return;
            }
            let mut entries = build_asset_entries(document);
            let names_revision = document.doc.scene.revision();
            apply_asset_names(&mut entries, &document.doc.scene);
            (entries, document.raw_assets.clone(), names_revision)
        };

        let (entries, source, names_revision) = rebuilt;
        self.evict_asset_thumbnails(cx);
        self.assets_cache = entries;
        self.assets_source = Some(source);
        self.assets_names_revision = Some(names_revision);
        self.filter_assets(cx);
    }

    /// Recover asset names from the scene, gated on the scene revision: this
    /// rebuild also runs on every `SelectionChanged` (i.e. every canvas click),
    /// but names can only change when the scene itself does, and the selection
    /// lives outside the scene.
    fn refresh_asset_names(&mut self, document: &FigDocument) {
        let revision = document.doc.scene.revision();
        if self.assets_names_revision != Some(revision) {
            self.assets_names_revision = Some(revision);
            apply_asset_names(&mut self.assets_cache, &document.doc.scene);
        }
    }

    /// Narrow the visible rows to those whose name matches the Assets filter.
    /// The cache itself stays whole so filtering never invalidates thumbnails.
    fn filter_assets(&mut self, cx: &App) {
        let query = self.filter_query(Section::Assets, cx);
        self.visible_assets = self
            .assets_cache
            .iter()
            .enumerate()
            .filter(|(_, entry)| {
                query
                    .as_ref()
                    .is_none_or(|query| entry.name.to_lowercase().contains(query))
            })
            .map(|(index, _)| index)
            .collect();
    }

    // === Rendering =========================================================

    // GPUI re-renders every visible view on each window redraw, so render
    // must consume the cached rows: flattening the layer tree here would run
    // O(document) work on every canvas frame while the panel is open.
    fn render_sections(&mut self, _view: &Entity<FigView>, cx: &mut Context<Self>) -> AnyElement {
        let render_started = std::time::Instant::now();
        let editable = self.document_editable;
        if !self.document_ready {
            return centered_message("The document is still loading");
        }
        let pages = self.pages_cache.clone();
        let current_page_index = self.current_page_index;
        let component_count = self.components_cache.len();
        let component_total_count = self.components_total_count;
        let asset_count = self.visible_assets.len();
        let asset_total_count = self.assets_cache.len();

        let pages_open = self.section_open(Section::Pages);
        let layers_open = self.section_open(Section::Layers);
        let components_open = self.section_open(Section::Components);
        let assets_open = self.section_open(Section::Assets);

        let can_add_page = editable && pages.iter().all(|entry| entry.root.is_some());
        let can_delete_page = editable && pages.len() > 1;
        let layer_row_count = self.layer_rows.len();

        let pages_filter_open = self.filter_target == Some(Section::Pages);
        let pages_query = self.filter_query(Section::Pages, cx);
        let pages = filter_pages(pages, pages_query.as_deref());
        let layers_filter_open = self.filter_target == Some(Section::Layers);
        let components_filter_open = self.filter_target == Some(Section::Components);
        let assets_filter_open = self.filter_target == Some(Section::Assets);
        let components_this_page = self.components_this_page;
        let layers_filtering = self.filter_query(Section::Layers, cx).is_some();
        let components_filtering = self.filter_query(Section::Components, cx).is_some();
        let assets_filtering = self.filter_query(Section::Assets, cx).is_some();

        let element = v_flex()
            .size_full()
            .overflow_hidden()
            .child(
                v_flex()
                    .flex_none()
                    .child(
                        ListHeader::new("Pages")
                            .inset(true)
                            .toggle(Some(pages_open))
                            .on_toggle(cx.listener(|this, _, window, cx| {
                                this.toggle_section(Section::Pages, window, cx)
                            }))
                            .end_slot(
                                h_flex()
                                    .gap_1()
                                    .child(
                                        IconButton::new(
                                            "fanta-pages-filter",
                                            IconName::MagnifyingGlass,
                                        )
                                        .icon_size(IconSize::Small)
                                        .toggle_state(pages_filter_open)
                                        .aria_label("Filter pages")
                                        .tooltip(Tooltip::text("Filter Pages"))
                                        .on_click(
                                            cx.listener(|this, _, window, cx| {
                                                this.toggle_filter(Section::Pages, window, cx)
                                            }),
                                        ),
                                    )
                                    .when(can_add_page, |slot| {
                                        slot.child(
                                            IconButton::new("fanta-page-add", IconName::Plus)
                                                .icon_size(IconSize::Small)
                                                .aria_label("Add page")
                                                .tooltip(Tooltip::text("Add Page"))
                                                .on_click(
                                                    cx.listener(|this, _, _, cx| this.add_page(cx)),
                                                ),
                                        )
                                    }),
                            ),
                    )
                    .when(pages_open && pages_filter_open, |section| {
                        section.child(self.render_filter_row(cx))
                    })
                    .when(pages_open, |section| {
                        if pages.is_empty() && pages_query.is_some() {
                            section.child(empty_section_label(
                                "fanta-pages-empty",
                                "No pages match the filter",
                            ))
                        } else {
                            section.child(
                                div()
                                    .id("fanta-pages-list")
                                    .max_h(self.pages_height)
                                    .overflow_y_scroll()
                                    .child(v_flex().children(pages.iter().map(|entry| {
                                        self.render_page_row(
                                            entry,
                                            current_page_index == Some(entry.index),
                                            can_delete_page && entry.root.is_some(),
                                            cx,
                                        )
                                    }))),
                            )
                        }
                    }),
            )
            .child(self.render_section_divider(SectionDivider::PagesLayers, cx))
            .child(
                v_flex()
                    .when(layers_open, |section| section.flex_1())
                    .overflow_hidden()
                    .child(
                        ListHeader::new("Layers")
                            .inset(true)
                            .toggle(Some(layers_open))
                            .on_toggle(cx.listener(|this, _, window, cx| {
                                this.toggle_section(Section::Layers, window, cx)
                            }))
                            .end_slot(
                                h_flex()
                                    .gap_1()
                                    .when(!self.expanded_nodes.is_empty(), |slot| {
                                        slot.child(
                                            IconButton::new(
                                                "fanta-layers-collapse",
                                                IconName::ListCollapse,
                                            )
                                            .icon_size(IconSize::Small)
                                            .aria_label("Collapse all layers")
                                            .tooltip(Tooltip::text("Collapse All Layers"))
                                            .on_click(
                                                cx.listener(|this, _, _, cx| {
                                                    this.expanded_nodes.clear();
                                                    this.rebuild_layer_rows(cx);
                                                    cx.notify();
                                                }),
                                            ),
                                        )
                                    })
                                    .child(
                                        IconButton::new(
                                            "fanta-layers-filter",
                                            IconName::MagnifyingGlass,
                                        )
                                        .icon_size(IconSize::Small)
                                        .toggle_state(layers_filter_open)
                                        .aria_label("Filter layers")
                                        .tooltip(Tooltip::text("Filter Layers"))
                                        .on_click(
                                            cx.listener(|this, _, window, cx| {
                                                this.toggle_filter(Section::Layers, window, cx)
                                            }),
                                        ),
                                    ),
                            ),
                    )
                    .when(layers_open && layers_filter_open, |section| {
                        section.child(self.render_filter_row(cx))
                    })
                    .when(layers_open, |section| {
                        if layer_row_count == 0 {
                            section.child(empty_section_label(
                                "fanta-layers-empty",
                                if layers_filtering {
                                    "No layers match the filter"
                                } else {
                                    "No layers on this page"
                                },
                            ))
                        } else {
                            section.child(
                                uniform_list(
                                    "fanta-layers",
                                    layer_row_count,
                                    cx.processor(|this, range: Range<usize>, _window, cx| {
                                        let mut rows = Vec::with_capacity(range.len());
                                        for index in range {
                                            if let Some(row) = this.layer_rows.get(index) {
                                                rows.push(this.render_layer_row(index, row, cx));
                                            }
                                        }
                                        rows
                                    }),
                                )
                                .size_full()
                                .flex_shrink_1()
                                .track_scroll(&self.layers_scroll_handle),
                            )
                        }
                    }),
            )
            .child(self.render_section_divider(SectionDivider::LayersComponents, cx))
            .child(
                v_flex()
                    .flex_none()
                    .child(
                        ListHeader::new("Components")
                            .inset(true)
                            .toggle(Some(components_open))
                            .on_toggle(cx.listener(|this, _, window, cx| {
                                this.toggle_section(Section::Components, window, cx)
                            }))
                            .end_slot(
                                h_flex()
                                    .gap_1()
                                    .child(
                                        IconButton::new(
                                            "fanta-components-this-page",
                                            IconName::ListFilter,
                                        )
                                        .icon_size(IconSize::Small)
                                        .toggle_state(components_this_page)
                                        .aria_label("Only show components on this page")
                                        .tooltip(Tooltip::text("Only Components on This Page"))
                                        .on_click(
                                            cx.listener(|this, _, _, cx| {
                                                this.toggle_components_this_page(cx)
                                            }),
                                        ),
                                    )
                                    .child(
                                        IconButton::new(
                                            "fanta-components-filter",
                                            IconName::MagnifyingGlass,
                                        )
                                        .icon_size(IconSize::Small)
                                        .toggle_state(components_filter_open)
                                        .aria_label("Filter components")
                                        .tooltip(Tooltip::text("Filter Components"))
                                        .on_click(
                                            cx.listener(|this, _, window, cx| {
                                                this.toggle_filter(Section::Components, window, cx)
                                            }),
                                        ),
                                    )
                                    .child(
                                        Label::new(section_count_label(
                                            component_count,
                                            component_total_count,
                                            components_filtering,
                                        ))
                                        .size(LabelSize::Small)
                                        .color(Color::Muted),
                                    ),
                            ),
                    )
                    .when(components_open && components_filter_open, |section| {
                        section.child(self.render_filter_row(cx))
                    })
                    .when(components_open, |section| {
                        if component_count == 0 {
                            section.child(empty_section_label(
                                "fanta-components-empty",
                                if components_filtering {
                                    "No components match the filter"
                                } else {
                                    "No components"
                                },
                            ))
                        } else {
                            // Virtualized: component libraries can hold
                            // thousands of entries, and this runs on every
                            // window redraw.
                            section.child(
                                uniform_list(
                                    "fanta-components",
                                    component_count,
                                    cx.processor(|this, range: Range<usize>, _window, cx| {
                                        let mut rows = Vec::with_capacity(range.len());
                                        for index in range {
                                            if let Some((name, root)) =
                                                this.components_cache.get(index)
                                            {
                                                rows.push(this.render_component_row(
                                                    index,
                                                    name.clone(),
                                                    *root,
                                                    cx,
                                                ));
                                            }
                                        }
                                        rows
                                    }),
                                )
                                .w_full()
                                .h(section_list_height(
                                    component_count,
                                    SECTION_ROW_HEIGHT,
                                    self.components_height,
                                )),
                            )
                        }
                    }),
            )
            .child(self.render_section_divider(SectionDivider::ComponentsAssets, cx))
            .child(
                v_flex()
                    .flex_none()
                    .child(
                        ListHeader::new("Assets")
                            .inset(true)
                            .toggle(Some(assets_open))
                            .on_toggle(cx.listener(|this, _, window, cx| {
                                this.toggle_section(Section::Assets, window, cx)
                            }))
                            .end_slot(
                                h_flex()
                                    .gap_1()
                                    .child(
                                        IconButton::new(
                                            "fanta-assets-filter",
                                            IconName::MagnifyingGlass,
                                        )
                                        .icon_size(IconSize::Small)
                                        .toggle_state(assets_filter_open)
                                        .aria_label("Filter assets")
                                        .tooltip(Tooltip::text("Filter Assets"))
                                        .on_click(
                                            cx.listener(|this, _, window, cx| {
                                                this.toggle_filter(Section::Assets, window, cx)
                                            }),
                                        ),
                                    )
                                    .child(
                                        Label::new(section_count_label(
                                            asset_count,
                                            asset_total_count,
                                            assets_filtering,
                                        ))
                                        .size(LabelSize::Small)
                                        .color(Color::Muted),
                                    ),
                            ),
                    )
                    .when(assets_open && assets_filter_open, |section| {
                        section.child(self.render_filter_row(cx))
                    })
                    .when(assets_open, |section| {
                        if asset_count == 0 {
                            section.child(empty_section_label(
                                "fanta-assets-empty",
                                if assets_filtering {
                                    "No assets match the filter"
                                } else {
                                    "No assets"
                                },
                            ))
                        } else {
                            section.child(
                                uniform_list(
                                    "fanta-assets",
                                    asset_count,
                                    cx.processor(|this, range: Range<usize>, _window, cx| {
                                        let mut rows = Vec::with_capacity(range.len());
                                        for index in range {
                                            if let Some(entry) = this
                                                .visible_assets
                                                .get(index)
                                                .and_then(|asset| this.assets_cache.get(*asset))
                                            {
                                                rows.push(this.render_asset_row(index, entry, cx));
                                            }
                                        }
                                        rows
                                    }),
                                )
                                .w_full()
                                .h(section_list_height(
                                    asset_count,
                                    ASSET_ROW_HEIGHT,
                                    self.assets_height,
                                )),
                            )
                        }
                    }),
            )
            .into_any_element();
        crate::report_slow("design panel render", render_started);
        element
    }

    fn render_filter_row(&self, cx: &mut Context<Self>) -> AnyElement {
        h_flex()
            .w_full()
            .h(px(SECTION_ROW_HEIGHT))
            .px_2()
            .gap_1p5()
            .on_action(cx.listener(|this, _: &Cancel, window, cx| this.close_filter(window, cx)))
            .child(
                Icon::new(IconName::MagnifyingGlass)
                    .size(IconSize::Small)
                    .color(Color::Muted),
            )
            .child(div().flex_1().min_w_0().child(self.filter_editor.clone()))
            .child(
                IconButton::new("fanta-filter-close", IconName::Close)
                    .icon_size(IconSize::XSmall)
                    .icon_color(Color::Muted)
                    .aria_label("Close filter")
                    .tooltip(Tooltip::text("Close Filter"))
                    .on_click(cx.listener(|this, _, window, cx| this.close_filter(window, cx))),
            )
            .into_any_element()
    }

    fn render_section_divider(
        &self,
        divider: SectionDivider,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let line = div()
            .w_full()
            .h_px()
            .flex_none()
            .bg(cx.theme().colors().border_variant);
        if !self.divider_resizable(divider) {
            return line.into_any_element();
        }
        // Mirrors the dock's resize handle: an invisible hitbox straddling the
        // one-pixel divider line, deferred so it wins hit-testing over the
        // list rows it overlaps.
        line.relative()
            .child(deferred(
                div()
                    .id(("fanta-section-divider", divider as usize))
                    .absolute()
                    .top(-DIVIDER_HITBOX_SIZE / 2.)
                    .left_0()
                    .w_full()
                    .h(DIVIDER_HITBOX_SIZE)
                    .cursor_ns_resize()
                    .occlude()
                    .on_drag(DraggedSectionDivider(divider), |dragged, _, _, cx| {
                        cx.stop_propagation();
                        cx.new(|_| dragged.clone())
                    })
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                            this.divider_drag = Some(DividerDragState {
                                divider,
                                start_mouse_y: event.position.y,
                                start_height: this.section_rendered_height(divider),
                            });
                            cx.stop_propagation();
                        }),
                    )
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(move |this, event: &MouseUpEvent, _, cx| {
                            if event.click_count == 2 {
                                this.reset_section_height(divider, cx);
                                cx.stop_propagation();
                            }
                        }),
                    ),
            ))
            .into_any_element()
    }

    /// The inline rename field shared by a page or layer row: Enter/Escape
    /// commit or cancel; clicking away commits via the blur subscription.
    fn render_rename_editor(&self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .w_full()
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                match event.keystroke.key.as_str() {
                    "enter" => {
                        cx.stop_propagation();
                        this.commit_rename(window, cx);
                    }
                    "escape" => {
                        cx.stop_propagation();
                        this.cancel_rename(window, cx);
                    }
                    _ => {}
                }
            }))
            .child(self.rename_editor.clone())
            .into_any_element()
    }

    fn render_page_row(
        &self,
        entry: &PageEntry,
        is_current: bool,
        deletable: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let index = entry.index;
        let renaming =
            matches!(self.renaming, Some(RenameTarget::Page { index: i, .. }) if i == index);
        if renaming {
            return ListItem::new(("fanta-page", index))
                .spacing(ListItemSpacing::ExtraDense)
                .height(px(SECTION_ROW_HEIGHT))
                .indent_level(1)
                .indent_step_size(SECTION_INDENT_STEP)
                .toggle_state(is_current)
                .aria_role(Role::ListItem)
                .aria_label(entry.name.clone())
                .child(self.render_rename_editor(cx))
                .into_any_element();
        }

        let root = entry.root;
        let page_name = entry.name.clone();
        ListItem::new(("fanta-page", index))
            .spacing(ListItemSpacing::ExtraDense)
            .height(px(SECTION_ROW_HEIGHT))
            .indent_level(1)
            .indent_step_size(SECTION_INDENT_STEP)
            .toggle_state(is_current)
            .aria_role(Role::ListItem)
            .aria_label(page_name.clone())
            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                this.focus_handle.focus(window, cx);
                // Double-click a real page to rename it in place; a single click
                // just switches to it.
                if event.click_count() >= 2 {
                    if let Some(root) = root {
                        this.begin_page_rename(index, root, window, cx);
                    }
                } else {
                    this.select_page(index, cx);
                }
            }))
            .child(
                div()
                    .id(("fanta-page-name", index))
                    .flex_1()
                    .min_w_0()
                    .tooltip(Tooltip::text(page_name.clone()))
                    .child(Label::new(page_name).single_line().truncate()),
            )
            .when(deletable, |item| {
                item.end_slot(
                    IconButton::new(("fanta-page-delete", index), IconName::Close)
                        .icon_size(IconSize::XSmall)
                        .icon_color(Color::Muted)
                        .aria_label("Delete page")
                        .tooltip(Tooltip::text("Delete Page"))
                        .visible_on_hover("list_item")
                        .on_click(cx.listener(move |this, _, _, cx| this.delete_page(index, cx))),
                )
            })
            .into_any_element()
    }

    fn begin_page_rename(
        &mut self,
        index: usize,
        root: NodeId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.begin_rename(RenameTarget::Page { index, root }, window, cx);
    }

    fn begin_layer_rename(&mut self, node: NodeId, window: &mut Window, cx: &mut Context<Self>) {
        self.begin_rename(RenameTarget::Layer { node }, window, cx);
    }

    /// Enter inline-rename mode for a page or layer: seed the shared editor with
    /// the node's current name, select it all, and focus it.
    fn begin_rename(&mut self, target: RenameTarget, window: &mut Window, cx: &mut Context<Self>) {
        let Some(view) = self.active_view(cx) else {
            return;
        };
        let item = view.read(cx).item().clone();
        if !item.read(cx).is_editable() {
            return;
        }
        let name = item
            .read(cx)
            .document()
            .and_then(|document| document.doc.scene.get(target.node()))
            .map(|node| node.name.clone())
            .unwrap_or_default();
        self.renaming = Some(target);
        self.rename_editor.update(cx, |editor, cx| {
            editor.set_text(name, window, cx);
            editor.select_all(&SelectAll, window, cx);
        });
        self.rename_editor.focus_handle(cx).focus(window, cx);
        cx.notify();
    }

    fn cancel_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.renaming.take().is_some() {
            self.focus_handle.focus(window, cx);
            cx.notify();
        }
    }

    fn commit_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(target) = self.renaming.take() else {
            return;
        };
        self.focus_handle.focus(window, cx);
        cx.notify();
        let new_name = self.rename_editor.read(cx).text(cx).trim().to_string();
        if new_name.is_empty() {
            return;
        }
        let Some(view) = self.active_view(cx) else {
            return;
        };
        let item = view.read(cx).item().clone();
        if !item.read(cx).is_editable() {
            return;
        }
        view.update(cx, |view, cx| {
            view.finish_document_edits_for_external_change(cx);
        });
        let node = target.node();
        let old_name = item
            .read(cx)
            .document()
            .and_then(|document| document.doc.scene.get(node))
            .map(|node| node.name.clone());
        let Some(old_name) = old_name else {
            return;
        };
        if old_name == new_name {
            return;
        }
        let applied = item.update(cx, |item, cx| {
            item.apply(
                Operation::SetName {
                    id: node,
                    old: old_name,
                    new: new_name,
                },
                cx,
            )
        });
        if let Err(error) = applied {
            log::error!("fanta design panel: failed to rename: {error:#}");
        }
    }

    fn render_layer_row(
        &self,
        _index: usize,
        row: &LayerRow,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = row.id;
        if matches!(self.renaming, Some(RenameTarget::Layer { node }) if node == id) {
            return ListItem::new(format!("fanta-layer-{id}"))
                .indent_level(row.depth + 1)
                .indent_step_size(SECTION_INDENT_STEP)
                .spacing(ListItemSpacing::ExtraDense)
                .height(px(SECTION_ROW_HEIGHT))
                .toggle_state(row.selected)
                .aria_role(Role::TreeItem)
                .aria_label(row.name.clone())
                .child(self.render_rename_editor(cx))
                .into_any_element();
        }
        let name_color = if row.hidden {
            Color::Muted
        } else if row.accent {
            Color::Accent
        } else {
            Color::Default
        };
        let icon_color = if row.hidden {
            Color::Muted
        } else if row.accent {
            Color::Accent
        } else {
            Color::Muted
        };
        let layer_name = row.name.clone();

        let mut item = ListItem::new(format!("fanta-layer-{id}"))
            .indent_level(row.depth + 1)
            .indent_step_size(SECTION_INDENT_STEP)
            .spacing(ListItemSpacing::ExtraDense)
            .height(px(SECTION_ROW_HEIGHT))
            .toggle_state(row.selected)
            .aria_role(Role::TreeItem)
            .aria_label(layer_name.clone())
            .child(
                h_flex()
                    .w_full()
                    .min_w_0()
                    .gap_1()
                    .child(Icon::new(row.icon).size(IconSize::Small).color(icon_color))
                    .child(
                        div()
                            .id(format!("fanta-layer-name-{id}"))
                            .flex_1()
                            .min_w_0()
                            .tooltip(Tooltip::text(layer_name.clone()))
                            .child(
                                Label::new(layer_name)
                                    .single_line()
                                    .truncate()
                                    .color(name_color),
                            ),
                    ),
            );

        if row.has_children {
            item = item
                .toggle(Some(row.expanded))
                .always_show_disclosure_icon(true)
                .on_toggle(cx.listener(move |this, _, _, cx| this.toggle_node_expanded(id, cx)));
        }

        if self.document_editable {
            let eye_button = IconButton::new(
                format!("fanta-layer-eye-{id}"),
                if row.hidden {
                    IconName::EyeOff
                } else {
                    IconName::Eye
                },
            )
            .icon_size(IconSize::XSmall)
            .icon_color(if row.hidden {
                Color::Default
            } else {
                Color::Muted
            })
            .aria_label(if row.hidden {
                "Show layer"
            } else {
                "Hide layer"
            })
            .tooltip(Tooltip::text(if row.hidden {
                "Show Layer"
            } else {
                "Hide Layer"
            }))
            .on_click(
                cx.listener(move |this, _, _, cx| this.toggle_node_flag(id, NodeFlags::HIDDEN, cx)),
            )
            .when(!row.hidden, |button| button.visible_on_hover("list_item"));

            let lock_button = IconButton::new(
                format!("fanta-layer-lock-{id}"),
                if row.locked {
                    IconName::Lock
                } else {
                    IconName::LockOff
                },
            )
            .icon_size(IconSize::XSmall)
            .icon_color(if row.locked {
                Color::Default
            } else {
                Color::Muted
            })
            .aria_label(if row.locked {
                "Unlock layer"
            } else {
                "Lock layer"
            })
            .tooltip(Tooltip::text(if row.locked {
                "Unlock Layer"
            } else {
                "Lock Layer"
            }))
            .on_click(
                cx.listener(move |this, _, _, cx| this.toggle_node_flag(id, NodeFlags::LOCKED, cx)),
            )
            .when(!row.locked, |button| button.visible_on_hover("list_item"));

            item = item.end_slot(
                h_flex()
                    .gap_1()
                    .flex_none()
                    .child(eye_button)
                    .child(lock_button),
            );
        }

        if !self.document_editable {
            return item.into_any_element();
        }
        let active_item = self
            .active_view(cx)
            .map(|view| view.read(cx).item().clone());
        let inside_item = active_item.clone();
        let above_item = active_item.clone();
        let below_item = active_item;
        let dragged_layer = DraggedLayer {
            id,
            name: row.name.clone(),
            icon: row.icon,
        };
        div()
            .id(format!("fanta-layer-drop-{id}"))
            .relative()
            .w_full()
            .cursor_move()
            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                // Keep the whole row as the drag source. A click that lands on
                // one of the thin before/after drop hitboxes still bubbles here
                // and selects exactly like a click on the row's content.
                this.focus_handle.focus(window, cx);
                if event.click_count() >= 2 {
                    this.begin_layer_rename(id, window, cx);
                } else {
                    let modifiers = event.modifiers();
                    this.select_node(id, modifiers.secondary() || modifiers.shift, cx);
                }
                cx.stop_propagation();
            }))
            .on_drag(dragged_layer, |dragged, _, _, cx| {
                cx.new(|_| dragged.clone())
            })
            .can_drop(move |value, _, cx| {
                let Some(dragged) = value.downcast_ref::<DraggedLayer>() else {
                    return false;
                };
                let Some(item) = inside_item.as_ref() else {
                    return false;
                };
                let item = item.read(cx);
                item.is_editable()
                    && item.document().is_some_and(|document| {
                        layer_move_operations(
                            &document.doc,
                            dragged.id,
                            id,
                            LayerDropPlacement::Inside,
                        )
                        .is_ok()
                    })
            })
            .drag_over::<DraggedLayer>(|style, _, _, cx| {
                style.bg(cx.theme().colors().drop_target_background)
            })
            .on_drop(cx.listener(move |this, dragged: &DraggedLayer, _, cx| {
                this.drop_layer(dragged.id, id, LayerDropPlacement::Inside, cx);
                cx.stop_propagation();
            }))
            .child(item)
            .child(
                div()
                    .id(format!("fanta-layer-drop-above-{id}"))
                    .absolute()
                    .top_0()
                    .left_0()
                    .right_0()
                    .h(px(6.))
                    .can_drop(move |value, _, cx| {
                        let Some(dragged) = value.downcast_ref::<DraggedLayer>() else {
                            return false;
                        };
                        let Some(item) = above_item.as_ref() else {
                            return false;
                        };
                        let item = item.read(cx);
                        item.is_editable()
                            && item.document().is_some_and(|document| {
                                layer_move_operations(
                                    &document.doc,
                                    dragged.id,
                                    id,
                                    LayerDropPlacement::Above,
                                )
                                .is_ok()
                            })
                    })
                    .drag_over::<DraggedLayer>(|style, _, _, cx| {
                        style
                            .border_t_2()
                            .border_color(cx.theme().colors().drop_target_border)
                    })
                    .on_drop(cx.listener(move |this, dragged: &DraggedLayer, _, cx| {
                        this.drop_layer(dragged.id, id, LayerDropPlacement::Above, cx);
                        cx.stop_propagation();
                    })),
            )
            .child(
                div()
                    .id(format!("fanta-layer-drop-below-{id}"))
                    .absolute()
                    .bottom_0()
                    .left_0()
                    .right_0()
                    .h(px(6.))
                    .can_drop(move |value, _, cx| {
                        let Some(dragged) = value.downcast_ref::<DraggedLayer>() else {
                            return false;
                        };
                        let Some(item) = below_item.as_ref() else {
                            return false;
                        };
                        let item = item.read(cx);
                        item.is_editable()
                            && item.document().is_some_and(|document| {
                                layer_move_operations(
                                    &document.doc,
                                    dragged.id,
                                    id,
                                    LayerDropPlacement::Below,
                                )
                                .is_ok()
                            })
                    })
                    .drag_over::<DraggedLayer>(|style, _, _, cx| {
                        style
                            .border_b_2()
                            .border_color(cx.theme().colors().drop_target_border)
                    })
                    .on_drop(cx.listener(move |this, dragged: &DraggedLayer, _, cx| {
                        this.drop_layer(dragged.id, id, LayerDropPlacement::Below, cx);
                        cx.stop_propagation();
                    })),
            )
            .into_any_element()
    }

    fn render_component_row(
        &self,
        index: usize,
        name: SharedString,
        root: NodeId,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let component_name = name.clone();
        ListItem::new(("fanta-component", index))
            .spacing(ListItemSpacing::ExtraDense)
            .height(px(SECTION_ROW_HEIGHT))
            .indent_level(1)
            .indent_step_size(SECTION_INDENT_STEP)
            .aria_role(Role::ListItem)
            .aria_label(component_name.clone())
            .on_click(cx.listener(move |this, _, window, cx| {
                this.focus_handle.focus(window, cx);
                this.focus_component(root, cx);
            }))
            .child(
                h_flex()
                    .w_full()
                    .min_w_0()
                    .gap_1()
                    .child(
                        Icon::new(IconName::Blocks)
                            .size(IconSize::Small)
                            .color(Color::Accent),
                    )
                    .child(
                        div()
                            .id(("fanta-component-name", index))
                            .flex_1()
                            .min_w_0()
                            .tooltip(Tooltip::text(component_name.clone()))
                            .child(
                                Label::new(name)
                                    .single_line()
                                    .truncate()
                                    .color(Color::Accent),
                            ),
                    ),
            )
            .into_any_element()
    }

    fn render_asset_row(&self, index: usize, entry: &AssetEntry, cx: &App) -> AnyElement {
        ListItem::new(("fanta-asset", index))
            .spacing(ListItemSpacing::ExtraDense)
            .indent_level(1)
            .indent_step_size(SECTION_INDENT_STEP)
            .height(px(ASSET_ROW_HEIGHT))
            .selectable(false)
            .aria_role(Role::ListItem)
            .aria_label(entry.name.clone())
            .tooltip(Tooltip::text(asset_tooltip_text(entry)))
            .child(
                h_flex()
                    .w_full()
                    .min_w_0()
                    .gap_2()
                    .child(render_asset_thumbnail(entry, cx))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .child(
                                Label::new(entry.name.clone())
                                    .single_line()
                                    .truncate()
                                    .line_height_style(LineHeightStyle::UiLabel),
                            )
                            .when_some(asset_detail(entry), |column, detail| {
                                column.child(
                                    Label::new(detail)
                                        .size(LabelSize::XSmall)
                                        .color(Color::Muted)
                                        .single_line()
                                        .truncate()
                                        .line_height_style(LineHeightStyle::UiLabel),
                                )
                            }),
                    ),
            )
            .end_slot(
                Label::new(format_file_size(entry.byte_count as u64, true))
                    .size(LabelSize::Small)
                    .color(Color::Muted),
            )
            .into_any_element()
    }
}

/// The asset's thumbnail over an opaque tile, so a transparent PNG reads as an
/// image rather than a hole in the panel. Assets GPUI cannot decode keep the
/// generic glyph.
fn render_asset_thumbnail(entry: &AssetEntry, cx: &App) -> AnyElement {
    let tile = div()
        .flex_none()
        .size(ASSET_THUMBNAIL_SIZE)
        .rounded_sm()
        .overflow_hidden()
        .bg(cx.theme().colors().element_background);
    match &entry.image {
        Some(image) => tile.child(
            img(image.clone())
                .size(ASSET_THUMBNAIL_SIZE)
                .object_fit(ObjectFit::Contain)
                .rounded_sm(),
        ),
        None => tile.items_center().justify_center().child(
            Icon::new(IconName::Image)
                .size(IconSize::Small)
                .color(Color::Muted),
        ),
    }
    .into_any_element()
}

/// The muted secondary line: `"512×512 · PNG"`, dropping whichever half could
/// not be recovered.
fn asset_detail(entry: &AssetEntry) -> Option<SharedString> {
    let dimensions = entry
        .dimensions
        .map(|(width, height)| format!("{width}×{height}"));
    let format = (!entry.format_label.is_empty()).then(|| entry.format_label.as_ref());
    match (dimensions, format) {
        (Some(dimensions), Some(format)) => Some(format!("{dimensions} · {format}").into()),
        (Some(dimensions), None) => Some(dimensions.into()),
        (None, Some(format)) => Some(SharedString::from(format.to_owned())),
        (None, None) => None,
    }
}

fn asset_tooltip_text(entry: &AssetEntry) -> SharedString {
    let file_size = format_file_size(entry.byte_count as u64, true);
    let metadata = match asset_detail(entry) {
        Some(detail) => format!("{detail} · {file_size}"),
        None => file_size,
    };
    SharedString::from(format!(
        "{}\n{}\nAsset ID: {}",
        entry.name, metadata, entry.id
    ))
}

/// Precompute one [`AssetEntry`] per embedded asset. `name` is left empty for
/// [`apply_asset_names`] to fill from the scene. The GPUI thumbnail is cloned
/// from the document's precomputed set (built on the background load thread), so
/// no asset bytes are hashed here on the foreground.
fn build_asset_entries(document: &FigDocument) -> Vec<AssetEntry> {
    document
        .raw_assets
        .iter()
        .map(|(asset_id, bytes)| {
            let image = document.gpui_images.get(asset_id).cloned();
            let dimensions = document
                .asset_resolver
                .as_ref()
                .and_then(|resolver| resolver.resolve(*asset_id))
                .map(|decoded| (decoded.width, decoded.height));
            AssetEntry {
                id: *asset_id,
                name: SharedString::default(),
                image,
                dimensions,
                // Guessing the format reads only the header, not the whole
                // asset, so identifying the label stays cheap on the foreground.
                format_label: image::guess_format(bytes)
                    .ok()
                    .map(format_label)
                    .unwrap_or_default(),
                byte_count: bytes.len(),
            }
        })
        .collect()
}

/// A short, uppercase label for an encoded format, from its canonical extension
/// (`Jpeg` → `"JPG"`).
fn format_label(format: image::ImageFormat) -> SharedString {
    match format.extensions_str().first() {
        Some(extension) => SharedString::from(extension.to_uppercase()),
        None => SharedString::default(),
    }
}

/// Fill each entry's `name` from the scene node that references it, numbering
/// the assets nothing references (`"Image 1"`, `"Image 2"`, …).
fn apply_asset_names(entries: &mut [AssetEntry], scene: &Scene) {
    let names = collect_asset_names(scene);
    let mut unnamed = 0usize;
    for entry in entries {
        entry.name = match names.get(&entry.id) {
            Some(recovered) => recovered.name.clone(),
            None => {
                unnamed += 1;
                SharedString::from(format!("Image {unnamed}"))
            }
        };
    }
}

/// The best name for every asset the scene references. An asset with no
/// reference is absent, and gets a synthetic name from the caller.
fn collect_asset_names(scene: &Scene) -> HashMap<AssetId, AssetName> {
    let mut names = HashMap::new();
    for root in scene.roots() {
        for node_id in scene.descendants_of(*root) {
            if let Some(node) = scene.get(node_id) {
                record_asset_names(node, &mut names);
            }
        }
    }
    names
}

/// Record the names `node` lends to the assets it references: its own name, for
/// its bitmap or any image fill it paints with.
fn record_asset_names(node: &CanvasNode, names: &mut HashMap<AssetId, AssetName>) {
    // The name is cloned only where it is kept: a page of image-filled shapes
    // walks this for every one of them.
    let mut record = |asset: AssetId, from_bitmap: bool| {
        let name = || AssetName {
            name: SharedString::from(node.name.clone()),
            from_bitmap,
        };
        match names.entry(asset) {
            Entry::Vacant(slot) => {
                slot.insert(name());
            }
            // A bitmap layer is named after the asset itself, so it replaces a
            // name merely borrowed from a shape that paints with it. Between
            // two references of the same strength, the first in scene order
            // wins, keeping the list stable.
            Entry::Occupied(mut slot) if from_bitmap && !slot.get().from_bitmap => {
                slot.insert(name());
            }
            Entry::Occupied(_) => {}
        }
    };

    match &node.data {
        NodeData::Bitmap(bitmap) => record(bitmap.asset, true),
        NodeData::Vector(vector) => {
            for fill in &vector.fills {
                if let Fill::Image { asset, .. } = fill {
                    record(*asset, false);
                }
            }
        }
        NodeData::Group(group) => {
            for fill in group.background.iter().chain(group.background_fills.iter()) {
                if let Fill::Image { asset, .. } = fill {
                    record(*asset, false);
                }
            }
        }
        _ => {}
    }
}

fn layer_icon(node: &CanvasNode) -> IconName {
    match &node.data {
        NodeData::Group(group) => {
            if group.is_frame_surface() {
                IconName::ToolFrame
            } else {
                IconName::Blocks
            }
        }
        NodeData::Vector(_) => IconName::ToolRect,
        NodeData::Text(_) => IconName::Font,
        NodeData::Bitmap(_) => IconName::Image,
        NodeData::Video(_) => IconName::PlayOutlined,
        NodeData::Audio(_) => IconName::AudioOn,
        NodeData::Instance(_) => IconName::Sparkle,
        NodeData::Boolean(_) => IconName::Blocks,
        NodeData::NodeGraph(_)
        | NodeData::Model3d(_)
        | NodeData::AiArtifact(_)
        | NodeData::Embed(_) => IconName::SquareDot,
    }
}

// A non-selectable list item keeps the placeholder aligned with real rows.
fn empty_section_label(id: &'static str, text: &'static str) -> AnyElement {
    ListItem::new(id)
        .spacing(ListItemSpacing::ExtraDense)
        .height(px(SECTION_ROW_HEIGHT))
        .indent_level(1)
        .indent_step_size(SECTION_INDENT_STEP)
        .selectable(false)
        .child(Label::new(text).size(LabelSize::Small).color(Color::Muted))
        .into_any_element()
}

fn centered_message(text: impl Into<SharedString>) -> AnyElement {
    v_flex()
        .size_full()
        .items_center()
        .justify_center()
        .child(Label::new(text).color(Color::Muted))
        .into_any_element()
}

impl Render for FantaDesignPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body = match self.active_view(cx) {
            None => centered_message("Open a Figma document to browse its layers"),
            Some(view) => {
                let (loading_message, has_error) = {
                    let fig_item = view.read(cx).item().read(cx);
                    (
                        fig_item.document.loading_message(),
                        fig_item.document.error().is_some(),
                    )
                };
                if let Some(message) = loading_message {
                    centered_message(message)
                } else if has_error {
                    centered_message("Could not open this document")
                } else {
                    self.render_sections(&view, cx)
                }
            }
        };

        v_flex()
            .key_context("FantaDesignPanel")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(|this, _: &DeleteSelection, _, cx| {
                if let Some(view) = this.active_view(cx) {
                    view.update(cx, |view, cx| view.delete_selected_nodes(cx));
                }
            }))
            .on_action(cx.listener(|this, _: &CopySelection, _, cx| {
                if let Some(view) = this.active_view(cx) {
                    view.update(cx, |view, cx| view.copy_selected_nodes(cx));
                }
            }))
            .on_action(cx.listener(|this, _: &CutSelection, _, cx| {
                if let Some(view) = this.active_view(cx) {
                    view.update(cx, |view, cx| view.cut_selected_nodes(cx));
                }
            }))
            .on_action(cx.listener(|this, _: &PasteSelection, _, cx| {
                if let Some(view) = this.active_view(cx) {
                    view.update(cx, |view, cx| view.paste_selected_nodes(cx));
                }
            }))
            .on_action(cx.listener(|this, _: &DuplicateSelection, _, cx| {
                if let Some(view) = this.active_view(cx) {
                    view.update(cx, |view, cx| view.duplicate_selected_nodes(cx));
                }
            }))
            .size_full()
            .overflow_hidden()
            .bg(cx.theme().colors().panel_background)
            .on_drag_move(cx.listener(Self::handle_divider_drag))
            .child(body)
    }
}

impl EventEmitter<PanelEvent> for FantaDesignPanel {}

impl Focusable for FantaDesignPanel {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Panel for FantaDesignPanel {
    fn persistent_name() -> &'static str {
        "Fanta Design Panel"
    }

    fn panel_key() -> &'static str {
        "FantaDesignPanel"
    }

    fn position(&self, _window: &Window, cx: &App) -> DockPosition {
        FantaDesignPanelSettings::get_global(cx).dock
    }

    fn position_is_valid(&self, position: DockPosition) -> bool {
        matches!(position, DockPosition::Left | DockPosition::Right)
    }

    fn set_position(
        &mut self,
        position: DockPosition,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        update_settings_file(self.fs.clone(), cx, move |settings, _| {
            settings.fanta_design_panel.get_or_insert_default().dock = Some(position.into());
        });
    }

    fn default_size(&self, _window: &Window, cx: &App) -> Pixels {
        self.width
            .unwrap_or_else(|| FantaDesignPanelSettings::get_global(cx).default_width)
    }

    fn icon(&self, _window: &Window, cx: &App) -> Option<IconName> {
        (FantaDesignPanelSettings::get_global(cx).button && self.active_view.is_some())
            .then_some(IconName::Blocks)
    }

    fn icon_tooltip(&self, _window: &Window, _cx: &App) -> Option<&'static str> {
        Some("Fanta Design Panel")
    }

    fn toggle_action(&self) -> Box<dyn gpui::Action> {
        Box::new(ToggleFocus)
    }

    fn activation_priority(&self) -> u32 {
        4
    }

    fn enabled(&self, _cx: &App) -> bool {
        self.active_view.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fanta_doc::{
        BitmapNode, BlendMode, ComponentDef, ImageAdjust, ImageFitMode, InstanceNode, Transform2D,
        VectorNode,
    };
    use gpui::TestAppContext;
    use project::FakeFs;

    fn init_panel_test(cx: &mut TestAppContext) {
        cx.update(|cx| {
            zlog::init_test();
            assets::Assets.load_test_fonts(cx);
            let store = settings::SettingsStore::test(cx);
            cx.set_global(store);
            theme_settings::init(theme::LoadThemes::JustBase, cx);
            release_channel::init(semver::Version::new(0, 0, 0), cx);
            editor::init(cx);
        });
    }

    fn insert_node(doc: &mut Doc, mut node: CanvasNode) -> NodeId {
        node.index = doc.scene.next_child_index(node.parent);
        let id = node.id;
        doc.scene.insert(node).expect("test node should be valid");
        id
    }

    fn group_node(parent: Option<NodeId>, transform: Transform2D) -> CanvasNode {
        let mut node = CanvasNode::new(NodeData::Group(GroupNode::default()));
        node.parent = parent;
        node.transform = transform;
        node
    }

    fn vector_node(parent: Option<NodeId>, transform: Transform2D) -> CanvasNode {
        let mut node = CanvasNode::new(NodeData::Vector(VectorNode::default()));
        node.parent = parent;
        node.transform = transform;
        node
    }

    fn instance_node(parent: Option<NodeId>, component: ComponentId) -> CanvasNode {
        let mut node = CanvasNode::new(NodeData::Instance(InstanceNode {
            component,
            overrides: Vec::new(),
            prop_values: BTreeMap::new(),
            derived: Vec::new(),
            local_size: [100.0, 100.0],
        }));
        node.parent = parent;
        node
    }

    fn assert_transform_close(actual: Transform2D, expected: Transform2D) {
        for (actual, expected) in actual
            .to_components()
            .into_iter()
            .zip(expected.to_components())
        {
            assert!(
                (actual - expected).abs() < 1e-9,
                "transform component {actual} differs from {expected}"
            );
        }
    }

    #[test]
    fn layer_drop_reparents_at_end_preserves_world_transform_and_undoes_once() {
        let mut doc = Doc::new();
        let old_parent = insert_node(
            &mut doc,
            group_node(None, Transform2D::translation(120.0, -30.0)),
        );
        let target = insert_node(
            &mut doc,
            group_node(
                None,
                Transform2D::scale_xy(1.5, 0.75).then(&Transform2D::translation(-40.0, 90.0)),
            ),
        );
        let existing_target_child = insert_node(
            &mut doc,
            vector_node(Some(target), Transform2D::translation(5.0, 10.0)),
        );
        let old_local = Transform2D::rotation(0.25).then(&Transform2D::translation(16.0, 24.0));
        let dragged = insert_node(&mut doc, vector_node(Some(old_parent), old_local));
        let old_index = doc
            .scene
            .get(dragged)
            .expect("dragged node should exist")
            .index;
        let world_before = doc
            .scene
            .world_transform(dragged)
            .expect("dragged node should have a world transform");
        let expected_index = doc.scene.next_child_index(Some(target));

        let operations = layer_move_operations(&doc, dragged, target, LayerDropPlacement::Inside)
            .expect("the layer drop should be valid");
        assert!(matches!(
            operations.first(),
            Some(Operation::Reparent {
                id,
                new_parent: Some(parent),
                new_index,
                ..
            }) if *id == dragged && *parent == target && *new_index == expected_index
        ));
        assert!(matches!(
            operations.get(1),
            Some(Operation::SetTransform { id, .. }) if *id == dragged
        ));

        let undo_depth = doc.history.undo_depth();
        doc.history.begin("Move layer", &mut doc.scene);
        for operation in operations {
            doc.apply(operation)
                .expect("reparent operation should apply");
        }
        doc.history.commit(&mut doc.scene);

        assert_eq!(doc.history.undo_depth(), undo_depth + 1);
        assert_eq!(
            doc.scene.children_of(Some(target)),
            &[existing_target_child, dragged]
        );
        assert_transform_close(
            doc.scene
                .world_transform(dragged)
                .expect("dragged node should retain a world transform"),
            world_before,
        );
        assert_eq!(
            layer_move_operations(&doc, dragged, target, LayerDropPlacement::Inside)
                .expect_err("the last child is already in the requested position"),
            LayerDropError::AlreadyInPosition
        );

        assert!(doc.undo().expect("move should undo"));
        let restored = doc
            .scene
            .get(dragged)
            .expect("dragged node should still exist after undo");
        assert_eq!(restored.parent, Some(old_parent));
        assert_eq!(restored.index, old_index);
        assert_eq!(restored.transform, old_local);
        assert_transform_close(
            doc.scene
                .world_transform(dragged)
                .expect("restored node should have a world transform"),
            world_before,
        );
    }

    #[test]
    fn layer_drop_above_and_below_reorders_visual_z_index() {
        let mut doc = Doc::new();
        let parent = insert_node(&mut doc, group_node(None, Transform2D::IDENTITY));
        let bottom = insert_node(&mut doc, vector_node(Some(parent), Transform2D::IDENTITY));
        let middle = insert_node(&mut doc, vector_node(Some(parent), Transform2D::IDENTITY));
        let top = insert_node(&mut doc, vector_node(Some(parent), Transform2D::IDENTITY));

        let operations = layer_move_operations(&doc, bottom, middle, LayerDropPlacement::Above)
            .expect("a lower sibling can move above the target row");
        assert!(matches!(operations.as_slice(), [Operation::SetIndex { id, .. }] if *id == bottom));
        for operation in operations {
            doc.apply(operation).expect("reorder should apply");
        }
        assert_eq!(
            doc.scene.children_of(Some(parent)),
            &[middle, bottom, top],
            "scene order is bottom-first, while Above means visually above the row"
        );

        let operations = layer_move_operations(&doc, top, middle, LayerDropPlacement::Below)
            .expect("a higher sibling can move below the target row");
        for operation in operations {
            doc.apply(operation).expect("reorder should apply");
        }
        assert_eq!(doc.scene.children_of(Some(parent)), &[top, middle, bottom]);
        assert_eq!(
            layer_move_operations(&doc, top, middle, LayerDropPlacement::Below)
                .expect_err("dropping into the current visual slot is a no-op"),
            LayerDropError::AlreadyInPosition
        );
    }

    #[test]
    fn layer_drop_rebalances_an_exhausted_fractional_index_gap() {
        let mut doc = Doc::new();
        let parent = insert_node(&mut doc, group_node(None, Transform2D::IDENTITY));
        let mut lower = vector_node(Some(parent), Transform2D::IDENTITY);
        lower.index = IndexKey::from_raw(1.0);
        let lower = {
            let id = lower.id;
            doc.scene.insert(lower).expect("lower node is valid");
            id
        };
        let mut upper = vector_node(Some(parent), Transform2D::IDENTITY);
        upper.index = IndexKey::from_raw(1.0 + f64::EPSILON);
        let upper = {
            let id = upper.id;
            doc.scene.insert(upper).expect("upper node is valid");
            id
        };
        let dragged = insert_node(&mut doc, vector_node(Some(parent), Transform2D::IDENTITY));

        let operations = layer_move_operations(&doc, dragged, lower, LayerDropPlacement::Above)
            .expect("an exhausted gap should be normalized instead of rejecting the drop");
        assert!(
            !operations.is_empty(),
            "normalization updates at least one adjacent key"
        );
        for operation in operations {
            doc.apply(operation)
                .expect("normalized reorder should apply");
        }
        assert_eq!(
            doc.scene.children_of(Some(parent)),
            &[lower, dragged, upper]
        );
        doc.scene
            .validate()
            .expect("normalizing z-order preserves scene invariants");
    }

    #[test]
    fn layer_drop_between_rows_reparents_at_that_slot_and_preserves_world_transform() {
        let mut doc = Doc::new();
        let old_parent = insert_node(
            &mut doc,
            group_node(None, Transform2D::translation(40.0, -20.0)),
        );
        let new_parent = insert_node(
            &mut doc,
            group_node(
                None,
                Transform2D::scale_xy(1.5, 0.5).then(&Transform2D::translation(-80.0, 60.0)),
            ),
        );
        let target = insert_node(
            &mut doc,
            vector_node(Some(new_parent), Transform2D::IDENTITY),
        );
        let top = insert_node(
            &mut doc,
            vector_node(Some(new_parent), Transform2D::IDENTITY),
        );
        let dragged = insert_node(
            &mut doc,
            vector_node(
                Some(old_parent),
                Transform2D::rotation(0.2).then(&Transform2D::translation(12.0, 30.0)),
            ),
        );
        let world_before = doc
            .scene
            .world_transform(dragged)
            .expect("dragged node has world geometry");

        let operations = layer_move_operations(&doc, dragged, target, LayerDropPlacement::Above)
            .expect("a row can move between children of another group");
        assert!(matches!(
            operations.first(),
            Some(Operation::Reparent {
                id,
                new_parent: Some(parent),
                ..
            }) if *id == dragged && *parent == new_parent
        ));
        for operation in operations {
            doc.apply(operation).expect("reparent should apply");
        }
        assert_eq!(
            doc.scene.children_of(Some(new_parent)),
            &[target, dragged, top]
        );
        assert_transform_close(
            doc.scene
                .world_transform(dragged)
                .expect("reparented node retains world geometry"),
            world_before,
        );
    }

    #[test]
    fn layer_drop_rejects_page_roots_and_relative_descendant_cycles() {
        let mut doc = Doc::new();
        let page = insert_node(&mut doc, group_node(None, Transform2D::IDENTITY));
        doc.add_page(page);
        let parent = insert_node(&mut doc, group_node(Some(page), Transform2D::IDENTITY));
        let child = insert_node(&mut doc, vector_node(Some(parent), Transform2D::IDENTITY));

        assert_eq!(
            layer_move_operations(&doc, page, parent, LayerDropPlacement::Above)
                .expect_err("page roots are managed by the Pages section"),
            LayerDropError::PageRoot
        );
        assert_eq!(
            layer_move_operations(&doc, parent, child, LayerDropPlacement::Above)
                .expect_err("a row cannot be moved beside a descendant under itself"),
            LayerDropError::DescendantCycle
        );
    }

    #[test]
    fn layer_drop_rejects_self_descendant_and_non_container_targets() {
        let mut doc = Doc::new();
        let parent = insert_node(&mut doc, group_node(None, Transform2D::IDENTITY));
        let child = insert_node(&mut doc, group_node(Some(parent), Transform2D::IDENTITY));
        let leaf = insert_node(&mut doc, vector_node(Some(child), Transform2D::IDENTITY));
        let other = insert_node(&mut doc, vector_node(None, Transform2D::IDENTITY));
        let instance_target = insert_node(&mut doc, instance_node(None, ComponentId::new()));

        assert_eq!(
            layer_move_operations(&doc, parent, parent, LayerDropPlacement::Inside)
                .expect_err("a node cannot be dropped on itself"),
            LayerDropError::SelfDrop
        );
        assert_eq!(
            layer_move_operations(&doc, parent, child, LayerDropPlacement::Inside)
                .expect_err("a node cannot be dropped into its descendant"),
            LayerDropError::DescendantCycle
        );
        assert_eq!(
            layer_move_operations(&doc, other, leaf, LayerDropPlacement::Inside)
                .expect_err("a vector cannot accept children"),
            LayerDropError::TargetIsNotContainer
        );
        assert_eq!(
            layer_move_operations(&doc, other, instance_target, LayerDropPlacement::Inside)
                .expect_err("an instance cannot accept real scene children"),
            LayerDropError::TargetIsNotContainer
        );
    }

    #[test]
    fn layer_drop_rejects_component_masters_and_recursive_instances() {
        let mut doc = Doc::new();
        let master = insert_node(&mut doc, group_node(None, Transform2D::IDENTITY));
        let other_container = insert_node(&mut doc, group_node(None, Transform2D::IDENTITY));
        let component = ComponentId::new();
        doc.components
            .defs
            .insert(component, ComponentDef::new(component, master, "Button"));
        let instance = insert_node(&mut doc, instance_node(None, component));

        assert_eq!(
            layer_move_operations(&doc, master, other_container, LayerDropPlacement::Inside)
                .expect_err("a component master cannot leave the component library"),
            LayerDropError::ContainsComponentMaster
        );
        assert_eq!(
            layer_move_operations(&doc, instance, master, LayerDropPlacement::Inside)
                .expect_err("a component cannot contain an instance of itself"),
            LayerDropError::RecursiveComponentInstance
        );
    }

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

    fn bitmap_layer(asset: AssetId, name: &str) -> CanvasNode {
        let mut node = CanvasNode::new(NodeData::Bitmap(BitmapNode {
            asset,
            natural_size: [64, 64],
            local_size: [64., 64.],
            crop: None,
            fit: ImageFitMode::Fill,
            tint: None,
        }));
        node.name = name.to_owned();
        node
    }

    fn frame_filled_with(asset: AssetId, name: &str) -> CanvasNode {
        let mut node = CanvasNode::new(NodeData::Group(GroupNode {
            background: Some(image_fill(asset)),
            ..GroupNode::default()
        }));
        node.name = name.to_owned();
        node
    }

    #[test]
    fn asset_names_prefer_a_bitmap_layer_over_a_shape_that_paints_with_the_asset() {
        let asset = AssetId::new();
        let mut names = HashMap::new();

        // A shape painted with an image fill only lends its own name.
        record_asset_names(&frame_filled_with(asset, "Hero card"), &mut names);
        assert_eq!(
            names.get(&asset).map(|name| name.name.as_ref()),
            Some("Hero card")
        );

        // A bitmap layer names the asset itself, so it upgrades that name even
        // though it was met second.
        record_asset_names(&bitmap_layer(asset, "avatar.png"), &mut names);
        assert_eq!(
            names.get(&asset).map(|name| name.name.as_ref()),
            Some("avatar.png")
        );

        // Between two references of equal strength the first one wins, so the
        // list does not shuffle as the scene is walked.
        record_asset_names(&bitmap_layer(asset, "avatar copy.png"), &mut names);
        record_asset_names(&frame_filled_with(asset, "Other card"), &mut names);
        assert_eq!(
            names.get(&asset).map(|name| name.name.as_ref()),
            Some("avatar.png")
        );

        // An asset no node references stays unnamed; the panel numbers it.
        assert!(!names.contains_key(&AssetId::new()));
    }

    #[test]
    fn encoded_formats_map_to_short_labels() {
        assert_eq!(format_label(image::ImageFormat::Png).as_ref(), "PNG");
        assert_eq!(format_label(image::ImageFormat::Jpeg).as_ref(), "JPG");
        assert_eq!(format_label(image::ImageFormat::WebP).as_ref(), "WEBP");
    }

    #[test]
    fn the_asset_detail_line_drops_whichever_half_is_unknown() {
        let entry = |dimensions, format_label: &str| AssetEntry {
            id: AssetId::new(),
            name: SharedString::default(),
            image: None,
            dimensions,
            format_label: SharedString::from(format_label.to_owned()),
            byte_count: 0,
        };

        assert_eq!(
            asset_detail(&entry(Some((512, 384)), "PNG")).as_deref(),
            Some("512×384 · PNG")
        );
        assert_eq!(
            asset_detail(&entry(Some((512, 384)), "")).as_deref(),
            Some("512×384")
        );
        assert_eq!(asset_detail(&entry(None, "PNG")).as_deref(), Some("PNG"));
        assert_eq!(asset_detail(&entry(None, "")), None);
    }

    #[test]
    fn asset_tooltip_exposes_human_context_and_stable_id() {
        let id = AssetId::new();
        let entry = AssetEntry {
            id,
            name: SharedString::from("Product mark"),
            image: None,
            dimensions: Some((512, 384)),
            format_label: SharedString::from("PNG"),
            byte_count: 4096,
        };

        let tooltip = asset_tooltip_text(&entry);
        assert!(tooltip.contains("Product mark"));
        assert!(tooltip.contains("512×384 · PNG"));
        assert!(tooltip.contains(&id.to_string()));
    }

    #[test]
    fn filtered_counts_retain_the_total_context() {
        assert_eq!(section_count_label(3, 12, true).as_ref(), "3 / 12");
        assert_eq!(section_count_label(12, 12, true).as_ref(), "12");
        assert_eq!(section_count_label(3, 12, false).as_ref(), "3");
    }

    #[gpui::test]
    fn filtering_expands_its_section_and_collapsing_closes_the_editor(cx: &mut TestAppContext) {
        init_panel_test(cx);
        let fs: Arc<dyn Fs> = FakeFs::new(cx.executor());
        let panel = cx.add_window(move |window, cx| {
            FantaDesignPanel::build(fs, None, window, cx, Vec::new())
        });

        panel
            .update(cx, |panel, window, cx| {
                panel.collapsed_sections.insert(Section::Layers);
                panel.toggle_filter(Section::Layers, window, cx);
                assert!(panel.section_open(Section::Layers));
                assert_eq!(panel.filter_target, Some(Section::Layers));

                panel.filter_editor.update(cx, |editor, cx| {
                    editor.set_text("button", window, cx);
                });
                panel.toggle_section(Section::Layers, window, cx);

                assert!(!panel.section_open(Section::Layers));
                assert_eq!(panel.filter_target, None);
                assert!(panel.filter_editor.read(cx).text(cx).is_empty());
            })
            .expect("design panel window remains available");
    }

    fn page(name: &str, index: usize) -> PageEntry {
        PageEntry {
            name: SharedString::from(name.to_owned()),
            root: Some(NodeId::new()),
            index,
        }
    }

    #[test]
    fn filter_pages_preserves_true_indices_and_matches_case_insensitively() {
        let pages = vec![
            page("Icons", 0),
            page("Typography", 1),
            page("Dark Theme", 2),
        ];

        // No query returns every page untouched.
        assert_eq!(filter_pages(pages.clone(), None).len(), 3);

        // The query is matched against the (already lowercased) name, and each
        // survivor keeps its ORIGINAL index into `document.pages` — not its
        // position in the filtered list — so actions still hit the right page.
        let matched = filter_pages(pages.clone(), Some("theme"));
        assert_eq!(matched.len(), 1);
        assert_eq!(matched[0].name.as_ref(), "Dark Theme");
        assert_eq!(matched[0].index, 2);

        // A non-matching query yields the empty-state list.
        assert!(filter_pages(pages, Some("zzz")).is_empty());
    }
}
