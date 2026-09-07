//! The Fanta design panel: the Pages and Layers sections for the active
//! Figma canvas, mirroring the original Fanta left sidebar. Layers is the
//! fanta-gpui `LayersPanel` (virtualized, so a 30k-node page costs the same
//! per frame as a 30-node one); this file owns the host side of its
//! contract — the read model, the intent → operation mapping, and the echo.
//!
//! Without the `fanta-gpui-ui` feature (a diagnostic build; the feature is
//! on by default) there is no layers UI, so the layer-move machinery and the
//! selection/drop paths have no caller.
#![cfg_attr(not(feature = "fanta-gpui-ui"), allow(dead_code))]

use std::collections::HashSet;
use std::sync::Arc;

use anyhow::Result;
use editor::{
    Editor, EditorEvent,
    actions::{Cancel, SelectAll},
};
use fanta_doc::{
    CanvasNode, Doc, GroupNode, IndexKey, NodeData, NodeFlags, NodeId, Operation, Scene,
};
use fs::Fs;
use gpui::{
    AnyElement, App, AsyncWindowContext, ClickEvent, Context, DragMoveEvent, Empty, Entity,
    EventEmitter, FocusHandle, Focusable, KeyDownEvent, MouseButton, MouseDownEvent, MouseUpEvent,
    Pixels, Role, SharedString, Subscription, WeakEntity, Window, actions, deferred, px,
};
use settings::{Settings as _, update_settings_file};
use ui::{ListHeader, ListItem, ListItemSpacing, Tooltip, prelude::*};
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

/// The native (fallback) sections with a collapsible header and a filter
/// field. Layers has no native section any more — the fanta-gpui panel owns
/// its own header, collapse state, and (virtualized) tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Section {
    Pages,
}

/// The design sidebar uses the same compact 28 px rhythm as Zed's outline and
/// project panels.
const SECTION_ROW_HEIGHT: f32 = 28.;
const MIN_SECTION_HEIGHT: Pixels = px(56.);
const DEFAULT_PAGES_HEIGHT: Pixels = px(168.);
const DIVIDER_HITBOX_SIZE: Pixels = px(6.);
/// Ceiling on any single section so the other section headers and a usable
/// slice of the Layers list always stay visible while dragging.
const SECTION_RESIZE_RESERVE: Pixels = px(160.);
/// Base indent applied to every content row so rows align under their section
/// header instead of sitting flush against the panel edge. The indent lives
/// inside the row (`ListItem::indent_level`), keeping hover targets full width.
const SECTION_INDENT_STEP: Pixels = px(12.);

fn section_list_height(row_count: usize, row_height: f32, stored: Pixels) -> Pixels {
    px((row_count as f32 * row_height).min(stored.as_f32()))
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

/// The draggable boundary between sidebar sections. It resizes the
/// fixed-height Pages section above it while Layers (`flex_1`) absorbs the
/// remaining space.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SectionDivider {
    PagesLayers,
}

#[derive(Clone)]
struct DraggedSectionDivider(SectionDivider);

impl Render for DraggedSectionDivider {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        Empty
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
pub(crate) enum LayerDropPlacement {
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

pub struct FantaDesignPanel {
    focus_handle: FocusHandle,
    fs: Arc<dyn Fs>,
    active_view: Option<WeakEntity<FigView>>,
    width: Option<Pixels>,
    /// Host-controlled expansion of the layer tree, echoed into the gpui
    /// panel on every refresh (the panel's own expansion interactions come
    /// back as `ExpansionChanged` and land here).
    expanded_nodes: HashSet<NodeId>,
    collapsed_sections: HashSet<Section>,
    // Cached section state, rebuilt on document events (never in render — see
    // `render_sections`).
    pages_cache: Vec<PageEntry>,
    #[cfg(feature = "fanta-gpui-ui")]
    gpui_pages: Option<crate::gpui_adapters::pages::PagesAdapter>,
    #[cfg(feature = "fanta-gpui-ui")]
    gpui_layers: Option<crate::gpui_adapters::layers::LayersAdapter>,
    /// The gpui Layers panel's own header collapsed it: the section then
    /// takes only its header height instead of flexing over the sidebar.
    layers_collapsed: bool,
    current_page_index: Option<usize>,
    document_editable: bool,
    document_ready: bool,
    /// The selection anchor the layer tree last revealed. When the anchor
    /// changes (typically from a canvas click) its ancestors are expanded
    /// and the row is scrolled into view; a repeat of the same anchor is
    /// left alone so browsing the list never yanks it around.
    last_reveal_anchor: Option<NodeId>,
    /// A node to scroll into view on the next layers echo — set by
    /// `rebuild_tree` when the reveal anchor changes, consumed by
    /// `refresh_gpui_layers` after the tree and expansion are echoed.
    pending_reveal: Option<NodeId>,
    pages_height: Pixels,
    divider_drag: Option<DividerDragState>,
    filter_editor: Entity<Editor>,
    filter_target: Option<Section>,
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

/// An inline rename in progress in the native Pages section: a page renames
/// its root scene node (via `SetName`) and tracks its row index so the editor
/// renders in the right Pages row. (Layer renames are the gpui panel's own
/// inline editor, landing here as `RenameRequested`.)
#[derive(Clone, Copy)]
enum RenameTarget {
    Page { index: usize, root: NodeId },
}

impl RenameTarget {
    /// The scene node whose name is being edited.
    fn node(&self) -> NodeId {
        match *self {
            RenameTarget::Page { root, .. } => root,
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
                    this.rebuild_caches(cx);
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
            pages_cache: Vec::new(),
            #[cfg(feature = "fanta-gpui-ui")]
            gpui_pages: None,
            #[cfg(feature = "fanta-gpui-ui")]
            gpui_layers: None,
            layers_collapsed: false,
            current_page_index: None,
            document_ready: false,
            document_editable: false,
            last_reveal_anchor: None,
            pending_reveal: None,
            pages_height: DEFAULT_PAGES_HEIGHT,
            divider_drag: None,
            filter_editor,
            filter_target: None,
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
                    // pointer-move frame. The caches are rebuilt HERE, on
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
                                this.rebuild_caches(cx);
                                #[cfg(feature = "fanta-gpui-ui")]
                                this.refresh_gpui_pages(cx);
                                #[cfg(feature = "fanta-gpui-ui")]
                                this.refresh_gpui_layers(cx);
                                cx.notify();
                            }
                        },
                    ));
                    self.active_view = Some(view.downgrade());
                    self.expanded_nodes.clear();
                    self.last_reveal_anchor = None;
                    self.pending_reveal = None;
                    // Another document may reuse a (root, generation) key —
                    // the same .fig open in two tabs — so the memoized layer
                    // tree must not survive a view switch.
                    #[cfg(feature = "fanta-gpui-ui")]
                    if let Some(adapter) = self.gpui_layers.as_mut() {
                        adapter.tree_key = None;
                    }
                    self.rebuild_caches(cx);
                    #[cfg(feature = "fanta-gpui-ui")]
                    self.refresh_gpui_pages(cx);
                    #[cfg(feature = "fanta-gpui-ui")]
                    self.refresh_gpui_layers(cx);
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
            };
            editor.set_placeholder_text(placeholder, window, cx);
        });
        self.filter_editor.focus_handle(cx).focus(window, cx);
        self.rebuild_caches(cx);
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
        self.rebuild_caches(cx);
        cx.notify();
    }

    // === Section resizing ===================================================

    fn divider_resizable(&self, divider: SectionDivider) -> bool {
        // Each divider resizes exactly one fixed-height section; when that
        // section is collapsed there is nothing to resize.
        match divider {
            SectionDivider::PagesLayers => self.section_open(Section::Pages),
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
        }
    }

    fn set_section_height(&mut self, divider: SectionDivider, height: Pixels) {
        match divider {
            SectionDivider::PagesLayers => self.pages_height = height,
        }
    }

    fn reset_section_height(&mut self, divider: SectionDivider, cx: &mut Context<Self>) {
        let default_height = match divider {
            SectionDivider::PagesLayers => DEFAULT_PAGES_HEIGHT,
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
        // Dragging the boundary down grows the section above it (Pages);
        // Layers flexes.
        let delta = event.event.position.y - drag_state.start_mouse_y;
        let proposed = match dragged_divider {
            SectionDivider::PagesLayers => drag_state.start_height + delta,
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
                        Err(reason) => {
                            // The panel's drop highlight consults the same
                            // rules (`layer_drop_allowed`), so a refusal here
                            // is a race with a concurrent edit, not a UI lie.
                            log::debug!(
                                "fanta design panel: layer move {dragged} -> {target} \
                                 ({placement:?}) refused: {reason:?}"
                            );
                            return (Ok(false), DocChange::None);
                        }
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
                self.rebuild_caches(cx);
                cx.notify();
            }
            Ok(false) => {}
            Err(error) => log::error!("fanta design panel: failed to move layer: {error:#}"),
        }
    }

    /// Whether `layer_move_operations` would accept this drop right now — the
    /// truth the gpui panel's drop highlight is wired to, so a target the
    /// document refuses (a page root, a component master leaving its library,
    /// a recursive instance, a non-container `Inside`, …) never lights up.
    fn layer_drop_allowed(
        &self,
        dragged: NodeId,
        target: NodeId,
        placement: LayerDropPlacement,
        cx: &App,
    ) -> bool {
        let Some(view) = self.active_view(cx) else {
            return false;
        };
        let item = view.read(cx).item().read(cx);
        item.is_editable()
            && item.document().is_some_and(|document| {
                layer_move_operations(&document.doc, dragged, target, placement).is_ok()
            })
    }

    /// Reorder `id` to the top (`front`) or bottom of its siblings — Figma's
    /// Bring to front / Send to back — through the same move path a drag uses,
    /// so undo, transform preservation, and the document rules are shared.
    fn move_layer_to_extreme(&mut self, id: NodeId, front: bool, cx: &mut Context<Self>) {
        let Some(view) = self.active_view(cx) else {
            return;
        };
        let target = {
            let item = view.read(cx).item().read(cx);
            let Some(document) = item.document() else {
                return;
            };
            let scene = &document.doc.scene;
            let Some(node) = scene.get(id) else {
                return;
            };
            // Siblings are bottom-first in paint order: the last child is
            // the topmost row.
            let siblings = scene.children_of(node.parent);
            let extreme = if front {
                siblings.last()
            } else {
                siblings.first()
            };
            match extreme.copied() {
                Some(target) if target != id => target,
                _ => return,
            }
        };
        let placement = if front {
            LayerDropPlacement::Above
        } else {
            LayerDropPlacement::Below
        };
        self.drop_layer(id, target, placement, cx);
    }

    /// Apply the operations `build` derives from the document as one undo
    /// step. Empty operation lists are a no-op; failures roll back and log.
    fn apply_document_ops(
        &mut self,
        label: &'static str,
        build: impl FnOnce(&Doc) -> Vec<Operation>,
        cx: &mut Context<Self>,
    ) {
        let Some(view) = self.active_view(cx) else {
            return;
        };
        view.update(cx, |view, cx| {
            view.finish_document_edits_for_external_change(cx);
        });
        let item = view.read(cx).item().clone();
        let result: Option<Result<bool>> = item.update(cx, |item, cx| {
            if !item.is_editable() {
                return None;
            }
            item.with_document(cx, |document| {
                let operations = build(&document.doc);
                match crate::clipboard::apply_transaction(&mut document.doc, label, operations) {
                    Ok(true) => (Ok(true), DocChange::Content),
                    Ok(false) => (Ok(false), DocChange::None),
                    Err(error) => (Err(error), DocChange::None),
                }
            })
        });
        if let Some(Err(error)) = result {
            log::error!("fanta design panel: {label} failed: {error:#}");
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

    // === Document caches ===================================================

    /// Refresh the cached page rows, the current page index, and the reveal
    /// bookkeeping from the active view's document. Runs on document events,
    /// never in render.
    fn rebuild_caches(&mut self, cx: &mut App) {
        let rebuild_started = std::time::Instant::now();
        self.pages_cache.clear();
        self.current_page_index = None;
        self.document_editable = false;
        self.document_ready = false;
        let Some(view) = self.active_view(cx) else {
            self.last_reveal_anchor = None;
            self.pending_reveal = None;
            return;
        };
        self.rebuild_tree(&view, cx);
        crate::report_slow("design panel caches", rebuild_started);
    }

    /// The root the layer tree lists: the ACTIVE page first (which may be a
    /// component master root in a component-scoped view), selected-page
    /// fallback — the same root the canvas renders and hit-tests. `None` is a
    /// document without page roots, whose scene roots are the layers.
    fn layers_page_root(view: &FigView, document: &FigDocument) -> Option<NodeId> {
        document.doc.active_page().or_else(|| {
            document
                .page(view.selected_page_index())
                .and_then(|page| page.root)
        })
    }

    /// Refresh the Pages cache and the layer-tree reveal state. A pure read
    /// of the document.
    fn rebuild_tree(&mut self, view: &Entity<FigView>, cx: &App) {
        let view = view.read(cx);
        let fig_item = view.item().read(cx);
        self.document_editable = fig_item.is_editable();
        let Some(document) = fig_item.document() else {
            return;
        };
        self.document_ready = true;
        let doc = &document.doc;
        // Follow the same root the canvas renders and hit-tests. Deriving it
        // differently made the panel list a page the scoped canvas never
        // paints, so clicking a layer selected/edited an invisible node.
        let page_root = Self::layers_page_root(view, document);

        // The page highlight and This-page search must follow the page the
        // canvas actually renders — `page_root` above — not the view's last
        // clicked index: canvas-side navigation (scoped opens, prototype
        // jumps, component focus) moves the ACTIVE page without touching
        // `selected_page_index`, and the default-index fallback can disagree
        // with the active page outright (fresh open of a document whose
        // active page is not the largest). Deriving the index from any other
        // root made the Pages panel highlight the wrong row and scope its
        // search to a page the user was not looking at.
        self.current_page_index = page_root
            .and_then(|root| {
                document
                    .pages
                    .iter()
                    .position(|page| page.root == Some(root))
            })
            .or_else(|| document.page_index(view.selected_page_index()));
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

        // Reveal the selection: when the anchor changes (typically from a
        // canvas click), expand its ancestor chain so its row exists, then
        // scroll it into view once the tree is echoed into the layers panel.
        let anchor = doc.selection.anchor();
        if anchor != self.last_reveal_anchor {
            self.last_reveal_anchor = anchor;
            if let Some(anchor) = anchor {
                for ancestor in doc.scene.ancestors_of(anchor) {
                    self.expanded_nodes.insert(ancestor.id);
                }
                self.pending_reveal = Some(anchor);
            }
        }
    }

    // === Rendering =========================================================

    // GPUI re-renders every visible view on each window redraw, so render
    // must consume the cached rows: rebuilding them here would run
    // O(document) work on every canvas frame while the panel is open.
    fn render_sections(&mut self, _view: &Entity<FigView>, cx: &mut Context<Self>) -> AnyElement {
        let render_started = std::time::Instant::now();
        let editable = self.document_editable;
        if !self.document_ready {
            return centered_message("The document is still loading");
        }
        let pages = self.pages_cache.clone();
        let current_page_index = self.current_page_index;

        let pages_open = self.section_open(Section::Pages);

        let can_add_page = editable && pages.iter().all(|entry| entry.root.is_some());
        let can_delete_page = editable && pages.len() > 1;

        let pages_filter_open = self.filter_target == Some(Section::Pages);
        let pages_query = self.filter_query(Section::Pages, cx);
        let pages = filter_pages(pages, pages_query.as_deref());

        #[cfg(feature = "fanta-gpui-ui")]
        let gpui_pages_section: Option<AnyElement> = self.gpui_pages_section_element(cx);
        #[cfg(not(feature = "fanta-gpui-ui"))]
        let gpui_pages_section: Option<AnyElement> = None;
        #[cfg(feature = "fanta-gpui-ui")]
        let gpui_layers_section: Option<AnyElement> = self.gpui_layers_section_element(cx);
        #[cfg(not(feature = "fanta-gpui-ui"))]
        let gpui_layers_section: Option<AnyElement> = None;
        let element = v_flex()
            .size_full()
            .overflow_hidden()
            .child({
                let native = v_flex()
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
                    });
                match gpui_pages_section {
                    Some(section) => section,
                    None => native.into_any_element(),
                }
            })
            .child(self.render_section_divider(SectionDivider::PagesLayers, cx))
            .child(match gpui_layers_section {
                Some(section) => section,
                // The fanta-gpui LayersPanel is the only layers UI. It is
                // absent only when the runtime is switched off (build without
                // `fanta-gpui-ui`, `FANTA_GPUI_UI=0`, or a host that skipped
                // `gpui_component::init`), which is a diagnostic state, not a
                // mode: say so instead of rendering nothing.
                None => v_flex()
                    .flex_1()
                    .overflow_hidden()
                    .child(ListHeader::new("Layers").inset(true))
                    .child(empty_section_label(
                        "fanta-layers-unavailable",
                        "Layers need the fanta-gpui UI runtime",
                    ))
                    .into_any_element(),
            })
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

    /// The inline rename field of a page row: Enter/Escape commit or cancel;
    /// clicking away commits via the blur subscription.
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

    /// Enter inline-rename mode for a page: seed the shared editor with the
    /// node's current name, select it all, and focus it.
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
        #[cfg(feature = "fanta-gpui-ui")]
        self.ensure_gpui_pages(_window, cx);
        #[cfg(feature = "fanta-gpui-ui")]
        self.ensure_gpui_layers(_window, cx);
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
            .bg(cx.theme().colors().editor_background)
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
    use std::collections::BTreeMap;

    use super::*;
    use fanta_doc::{ComponentDef, ComponentId, InstanceNode, Transform2D, VectorNode};
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

    #[gpui::test]
    fn filtering_expands_its_section_and_collapsing_closes_the_editor(cx: &mut TestAppContext) {
        init_panel_test(cx);
        let fs: Arc<dyn Fs> = FakeFs::new(cx.executor());
        let panel = cx.add_window(move |window, cx| {
            FantaDesignPanel::build(fs, None, window, cx, Vec::new())
        });

        panel
            .update(cx, |panel, window, cx| {
                panel.collapsed_sections.insert(Section::Pages);
                panel.toggle_filter(Section::Pages, window, cx);
                assert!(panel.section_open(Section::Pages));
                assert_eq!(panel.filter_target, Some(Section::Pages));

                panel.filter_editor.update(cx, |editor, cx| {
                    editor.set_text("cover", window, cx);
                });
                panel.toggle_section(Section::Pages, window, cx);

                assert!(!panel.section_open(Section::Pages));
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

#[cfg(all(test, feature = "fanta-gpui-ui"))]
mod gpui_pages_tests {
    use std::path::PathBuf;

    use super::*;
    use fanta_doc::VectorNode;
    use gpui::{TestAppContext, VisualTestContext};
    use project::{FakeFs, Project};

    fn init_test(cx: &mut TestAppContext) {
        cx.update(|cx| {
            zlog::init_test();
            assets::Assets.load_test_fonts(cx);
            let store = settings::SettingsStore::test(cx);
            cx.set_global(store);
            theme_settings::init(theme::LoadThemes::JustBase, cx);
            release_channel::init(semver::Version::new(0, 0, 0), cx);
            editor::init(cx);
            gpui_component::init(cx);
            fanta_gpui::init(cx);
            crate::theme_bridge::init(cx);
        });
    }

    fn named_page(doc: &mut Doc, page_name: &str, child_name: &str) -> NodeId {
        let mut page = CanvasNode::new(NodeData::Group(GroupNode::default()));
        page.name = page_name.to_owned();
        let root = page.id;
        doc.apply(Operation::create_node(page))
            .expect("create page root");
        doc.add_page(root);
        let mut child = CanvasNode::new(NodeData::Vector(VectorNode::default()));
        child.name = child_name.to_owned();
        child.parent = Some(root);
        doc.apply(Operation::create_node(child))
            .expect("create page child");
        root
    }

    /// Two visible pages, each holding one named vector node.
    fn doc_with_two_named_pages() -> (Doc, NodeId, NodeId) {
        let mut doc = Doc::new();
        let page_one = named_page(&mut doc, "Page 1", "Star Button");
        let page_two = named_page(&mut doc, "Page 2", "Star Chart");
        doc.set_active_page(Some(page_one));
        (doc, page_one, page_two)
    }

    async fn setup_panel(
        cx: &mut TestAppContext,
    ) -> (
        Entity<FantaDesignPanel>,
        Entity<FigView>,
        Entity<fanta_gpui::pages::PagesPanel>,
        NodeId,
        NodeId,
        VisualTestContext,
    ) {
        init_test(cx);
        let fs = FakeFs::new(cx.executor());
        let project = Project::test(fs.clone(), [], cx).await;
        let (doc, page_one, page_two) = doc_with_two_named_pages();
        let item = crate::document::ready_item_for_test(
            &project,
            PathBuf::from("/tmp/Design.fig"),
            doc,
            cx,
        );
        let view_slot = std::rc::Rc::new(std::cell::RefCell::new(None));
        let captured_view = view_slot.clone();
        let (panel, cx) = cx.add_window_view(move |window, cx| {
            let view = cx.new(|cx| FigView::new(item.clone(), project.clone(), window, cx));
            *captured_view.borrow_mut() = Some(view.clone());
            FantaDesignPanel::build(fs, Some(view), window, cx, Vec::new())
        });
        let view = view_slot
            .borrow_mut()
            .take()
            .expect("the fig view should be captured during window setup");
        cx.run_until_parked();
        let pages_panel = panel.read_with(cx, |panel, _| {
            panel
                .gpui_pages
                .as_ref()
                .expect("the fanta-gpui pages adapter should mount in a themed window")
                .panel
                .clone()
        });
        let cx = cx.clone();
        (panel, view, pages_panel, page_one, page_two, cx)
    }

    fn click(cx: &mut VisualTestContext, selector: &str) {
        // `debug_bounds` wants a `&'static str`; tests can afford the leak.
        let selector: &'static str = Box::leak(selector.to_owned().into_boxed_str());
        let center = |cx: &mut VisualTestContext| {
            cx.debug_bounds(selector)
                .unwrap_or_else(|| panic!("missing rendered selector: {selector}"))
                .center()
        };
        // The move draws a fresh frame, letting reveal animations (advanced
        // via the test clock) settle before the click's hit test runs.
        let position = center(cx);
        cx.simulate_mouse_move(position, None, gpui::Modifiers::none());
        let position = center(cx);
        cx.simulate_click(position, gpui::Modifiers::none());
    }

    #[gpui::test]
    async fn typing_in_pages_search_returns_matches_from_the_document(cx: &mut TestAppContext) {
        let (panel, _view, pages_panel, _page_one, _page_two, mut cx) = setup_panel(cx).await;
        let cx = &mut cx;

        click(cx, "pages-search-trigger");
        cx.run_until_parked();
        cx.simulate_input("Star");
        cx.run_until_parked();

        let results = pages_panel.read_with(cx, |panel, _| panel.search_results().clone());
        assert_eq!(
            results
                .items
                .iter()
                .map(|item| item.title.to_string())
                .collect::<Vec<_>>(),
            vec!["Star Button".to_owned()],
            "the default This-page scope should surface the current page's match"
        );
        assert_eq!(results.total, 1);
        assert!(
            cx.debug_bounds("pages-result-0").is_some(),
            "the match should be rendered as a result row"
        );

        // Widening the scope reaches the second page too.
        click(cx, "pages-scope-trigger");
        click(cx, "pages-scope-all-pages");
        cx.run_until_parked();
        let results = pages_panel.read_with(cx, |panel, _| panel.search_results().clone());
        assert_eq!(
            results
                .items
                .iter()
                .map(|item| item.title.to_string())
                .collect::<Vec<_>>(),
            vec!["Star Button".to_owned(), "Star Chart".to_owned()],
            "the All-pages scope should surface matches on every visible page"
        );

        // Host bookkeeping matches what the panel shows.
        panel.read_with(cx, |panel, _| {
            let adapter = panel.gpui_pages.as_ref().expect("adapter");
            assert_eq!(adapter.result_order.len(), 2);
            assert_eq!(adapter.result_map.len(), 2);
        });
    }

    #[gpui::test]
    async fn switching_pages_keeps_the_panel_highlight_in_sync(cx: &mut TestAppContext) {
        let (_panel, view, pages_panel, page_one, page_two, mut cx) = setup_panel(cx).await;
        let cx = &mut cx;

        let page_one_id = SharedString::from(page_one.to_string());
        let page_two_id = SharedString::from(page_two.to_string());

        // The freshly mounted panel highlights the page the canvas shows.
        assert_eq!(
            pages_panel.read_with(cx, |panel, _| panel.selected_page().cloned()),
            Some(page_one_id.clone()),
            "the initial echo should select the active page"
        );

        // A panel row activation round-trips: intent -> host page switch ->
        // echo. The row-click -> SelectRequested half lives in the
        // component's own tests; the pages reveal animation never settles
        // under the test scheduler, so the intent is emitted directly here.
        pages_panel.update_in(cx, |_, _, cx| {
            cx.emit(fanta_gpui::pages::PagesPanelAction::SelectRequested {
                page_id: page_two_id.clone(),
            });
        });
        cx.run_until_parked();
        assert_eq!(
            view.read_with(cx, |view, _| view.selected_page_index()),
            Some(1),
            "the panel intent should switch the canvas page"
        );
        assert_eq!(
            pages_panel.read_with(cx, |panel, _| panel.selected_page().cloned()),
            Some(page_two_id),
            "the panel highlight should follow the page switch"
        );

        // A canvas-side page switch (no panel involvement) echoes too.
        view.update_in(cx, |view, _window, cx| view.select_page(0, cx));
        cx.run_until_parked();
        assert_eq!(
            pages_panel.read_with(cx, |panel, _| panel.selected_page().cloned()),
            Some(page_one_id),
            "a canvas-driven page switch should re-highlight the panel row"
        );
    }
}

#[cfg(all(test, feature = "fanta-gpui-ui"))]
mod gpui_layers_tests {
    use std::path::PathBuf;

    use super::*;
    use fanta_doc::{Color, ComponentDef, ComponentId, InstanceNode, VectorNode};
    use fanta_gpui::layers::{
        LayersPanel, LayersPanelAction, LayersPanelContextAction, LayersPanelDropPosition,
        LayersPanelSelectionMode,
    };
    use gpui::{TestAppContext, VisualTestContext};
    use project::{FakeFs, Project};

    fn init_test(cx: &mut TestAppContext) {
        cx.update(|cx| {
            zlog::init_test();
            assets::Assets.load_test_fonts(cx);
            let store = settings::SettingsStore::test(cx);
            cx.set_global(store);
            theme_settings::init(theme::LoadThemes::JustBase, cx);
            release_channel::init(semver::Version::new(0, 0, 0), cx);
            editor::init(cx);
            gpui_component::init(cx);
            fanta_gpui::init(cx);
            crate::theme_bridge::init(cx);
        });
    }

    fn insert(doc: &mut Doc, mut node: CanvasNode, parent: Option<NodeId>, name: &str) -> NodeId {
        node.parent = parent;
        node.name = name.to_owned();
        // Bottom-first insertion order, like the importer produces.
        node.index = doc.scene.next_child_index(parent);
        let id = node.id;
        doc.apply(Operation::create_node(node))
            .expect("create test node");
        id
    }

    fn rect(x: f64, y: f64) -> CanvasNode {
        CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
            x,
            y,
            40.0,
            40.0,
            Color::BLACK,
        )))
    }

    fn group() -> CanvasNode {
        CanvasNode::new(NodeData::Group(GroupNode::default()))
    }

    /// The fixture page:
    ///
    /// ```text
    /// Page
    ///   Frame            (group; the panel lists its children topmost first)
    ///     Leaf 0..leaf_count   (rects; Leaf N is the topmost)
    ///   Master           (group; a component master)
    ///   Instance         (instance of Master)
    /// ```
    struct Fixture {
        doc: Doc,
        page: NodeId,
        frame: NodeId,
        leaves: Vec<NodeId>,
        master: NodeId,
        instance: NodeId,
    }

    fn fixture(leaf_count: usize) -> Fixture {
        let mut doc = Doc::new();
        let page = insert(&mut doc, group(), None, "Page 1");
        doc.add_page(page);
        doc.set_active_page(Some(page));
        let frame = insert(&mut doc, group(), Some(page), "Frame");
        let leaves = (0..leaf_count)
            .map(|index| {
                insert(
                    &mut doc,
                    rect(index as f64 * 50.0, 0.0),
                    Some(frame),
                    &format!("Leaf {index}"),
                )
            })
            .collect();
        let master = insert(&mut doc, group(), Some(page), "Master");
        let component = ComponentId::new();
        doc.components
            .defs
            .insert(component, ComponentDef::new(component, master, "Master"));
        let instance = insert(
            &mut doc,
            CanvasNode::new(NodeData::Instance(InstanceNode {
                component,
                overrides: Vec::new(),
                prop_values: Default::default(),
                derived: Vec::new(),
                local_size: [100.0, 100.0],
            })),
            Some(page),
            "Instance",
        );
        Fixture {
            doc,
            page,
            frame,
            leaves,
            master,
            instance,
        }
    }

    struct Harness {
        panel: Entity<FantaDesignPanel>,
        view: Entity<FigView>,
        layers: Entity<LayersPanel>,
        fixture: Fixture,
        cx: VisualTestContext,
    }

    async fn setup(cx: &mut TestAppContext, leaf_count: usize) -> Harness {
        init_test(cx);
        let fs = FakeFs::new(cx.executor());
        let project = Project::test(fs.clone(), [], cx).await;
        let mut fixture = fixture(leaf_count);
        let doc = std::mem::replace(&mut fixture.doc, Doc::new());
        let item = crate::document::ready_item_for_test(
            &project,
            PathBuf::from("/tmp/Layers.fig"),
            doc,
            cx,
        );
        let view_slot = std::rc::Rc::new(std::cell::RefCell::new(None));
        let captured_view = view_slot.clone();
        let (panel, cx) = cx.add_window_view(move |window, cx| {
            let view = cx.new(|cx| FigView::new(item.clone(), project.clone(), window, cx));
            *captured_view.borrow_mut() = Some(view.clone());
            FantaDesignPanel::build(fs, Some(view), window, cx, Vec::new())
        });
        let view = view_slot
            .borrow_mut()
            .take()
            .expect("the fig view should be captured during window setup");
        cx.run_until_parked();
        let layers = panel.read_with(cx, |panel, _| {
            panel
                .gpui_layers
                .as_ref()
                .expect("the fanta-gpui layers adapter should mount in a themed window")
                .panel
                .clone()
        });
        let cx = cx.clone();
        Harness {
            panel,
            view,
            layers,
            fixture,
            cx,
        }
    }

    fn row_id(id: NodeId) -> SharedString {
        SharedString::from(id.to_string())
    }

    fn selection(harness: &Harness) -> Vec<NodeId> {
        harness.view.read_with(&harness.cx, |view, cx| {
            view.item()
                .read(cx)
                .document()
                .map(|document| document.doc.selection.iter().copied().collect())
                .unwrap_or_default()
        })
    }

    fn select_on_canvas(harness: &mut Harness, id: NodeId) {
        let item = harness
            .view
            .read_with(&harness.cx, |view, _| view.item().clone());
        harness.cx.update(|_, cx| {
            item.update(cx, |item, cx| {
                item.with_document(cx, |document| {
                    document.doc.selection.select_only(id);
                    ((), DocChange::Selection)
                });
            });
        });
        harness.cx.run_until_parked();
    }

    fn with_doc<R>(harness: &Harness, read: impl FnOnce(&Doc) -> R) -> R {
        harness.view.read_with(&harness.cx, |view, cx| {
            let item = view.item().read(cx);
            read(&item.document().expect("document is ready").doc)
        })
    }

    fn emit(harness: &mut Harness, action: LayersPanelAction) {
        let layers = harness.layers.clone();
        harness.cx.update(|_, cx| {
            layers.update(cx, |_, cx| cx.emit(action));
        });
        harness.cx.run_until_parked();
    }

    fn context_action(harness: &mut Harness, id: NodeId, action: LayersPanelContextAction) {
        emit(
            harness,
            LayersPanelAction::ContextActionRequested {
                node_id: row_id(id),
                action,
            },
        );
    }

    fn row_bounds(harness: &mut Harness, id: NodeId) -> Option<gpui::Bounds<Pixels>> {
        let selector: &'static str = Box::leak(format!("layers-row-{id}").into_boxed_str());
        harness.cx.debug_bounds(selector)
    }

    #[gpui::test]
    async fn canvas_selection_echoes_into_the_panel_expanding_ancestors_and_revealing_the_row(
        cx: &mut TestAppContext,
    ) {
        let mut harness = setup(cx, 300).await;
        let frame = harness.fixture.frame;
        let leaves = harness.fixture.leaves.clone();
        // Nothing is expanded on mount: the frame's leaves are not shown.
        let leaf = leaves[3];
        assert!(row_bounds(&mut harness, leaf).is_none());
        assert!(row_bounds(&mut harness, frame).is_some());

        // A canvas click on a leaf near the BOTTOM of the frame (Leaf 3 is
        // the 297th of 300 rows under the frame) must expand the frame and
        // scroll the row into the viewport.
        select_on_canvas(&mut harness, leaf);
        assert!(harness.panel.read_with(&harness.cx, |panel, _| {
            panel.expanded_nodes.contains(&frame)
        }));
        let selected = harness
            .layers
            .read_with(&harness.cx, |layers, _| layers.visible_row_ids());
        assert!(selected.contains(&row_id(leaf)), "the leaf row is shown");
        let row = row_bounds(&mut harness, leaf).expect("the revealed row is rendered");
        let viewport = harness
            .cx
            .debug_bounds("layers-tree-viewport")
            .expect("the tree renders");
        assert!(
            row.top() >= viewport.top() && row.bottom() <= viewport.bottom(),
            "the revealed row must sit inside the viewport: {row:?} in {viewport:?}"
        );
        assert!(row_bounds(&mut harness, leaves[299]).is_none());
    }

    #[gpui::test]
    async fn selection_changes_do_not_rebuild_the_layer_tree(cx: &mut TestAppContext) {
        let mut harness = setup(cx, 20).await;
        let leaves = harness.fixture.leaves.clone();
        let key_before = harness.panel.read_with(&harness.cx, |panel, _| {
            panel.gpui_layers.as_ref().unwrap().tree_key
        });
        assert!(key_before.is_some());
        select_on_canvas(&mut harness, leaves[0]);
        select_on_canvas(&mut harness, leaves[1]);
        let key_after = harness.panel.read_with(&harness.cx, |panel, _| {
            panel.gpui_layers.as_ref().unwrap().tree_key
        });
        assert_eq!(
            key_before, key_after,
            "selection is echoed without a rebuild"
        );

        // A content edit advances the generation and rebuilds.
        let leaf = harness.fixture.leaves[0];
        harness.panel.update_in(&mut harness.cx, |panel, _, cx| {
            panel.rename_node_to(leaf, "Renamed".into(), cx);
        });
        harness.cx.run_until_parked();
        let key_edited = harness.panel.read_with(&harness.cx, |panel, _| {
            panel.gpui_layers.as_ref().unwrap().tree_key
        });
        assert_ne!(key_before, key_edited);
    }

    #[gpui::test]
    async fn row_intents_select_toggle_and_range_select_over_the_shown_rows(
        cx: &mut TestAppContext,
    ) {
        let mut harness = setup(cx, 6).await;
        let leaves = harness.fixture.leaves.clone();
        let frame = harness.fixture.frame;

        emit(
            &mut harness,
            LayersPanelAction::SelectRequested {
                node_id: row_id(frame),
                mode: LayersPanelSelectionMode::Replace,
            },
        );
        assert_eq!(selection(&harness), vec![frame]);

        // Expand the frame (as the panel would after a disclosure click),
        // then range-select from Leaf 4 (anchor) down to Leaf 1. Rows show
        // topmost first: Leaf 5, 4, 3, 2, 1, 0.
        emit(
            &mut harness,
            LayersPanelAction::ExpansionChanged {
                node_id: row_id(frame),
                expanded: true,
            },
        );
        select_on_canvas(&mut harness, leaves[4]);
        emit(
            &mut harness,
            LayersPanelAction::SelectRequested {
                node_id: row_id(leaves[1]),
                mode: LayersPanelSelectionMode::Range,
            },
        );
        let mut selected = selection(&harness);
        selected.sort();
        let mut expected = vec![leaves[4], leaves[3], leaves[2], leaves[1]];
        expected.sort();
        assert_eq!(selected, expected);
        // The anchor survives the range, so a second range extends from it.
        assert_eq!(
            with_doc(&harness, |doc| doc.selection.anchor()),
            Some(leaves[4])
        );
        emit(
            &mut harness,
            LayersPanelAction::SelectRequested {
                node_id: row_id(leaves[5]),
                mode: LayersPanelSelectionMode::Range,
            },
        );
        let mut selected = selection(&harness);
        selected.sort();
        let mut expected = vec![leaves[5], leaves[4]];
        expected.sort();
        assert_eq!(selected, expected);

        emit(
            &mut harness,
            LayersPanelAction::SelectRequested {
                node_id: row_id(leaves[0]),
                mode: LayersPanelSelectionMode::Toggle,
            },
        );
        assert!(selection(&harness).contains(&leaves[0]));
        assert_eq!(selection(&harness).len(), 3);
    }

    #[gpui::test]
    async fn move_intents_reparent_and_the_highlight_validator_matches_the_document(
        cx: &mut TestAppContext,
    ) {
        let mut harness = setup(cx, 3).await;
        let leaf = harness.fixture.leaves[0];
        let master = harness.fixture.master;
        let page = harness.fixture.page;
        let instance = harness.fixture.instance;

        // The validator the panel's drop highlight consults agrees with the
        // document rules: a leaf can move into the master, the master itself
        // may not leave its library, a page root never moves, and an instance
        // accepts no children.
        harness.panel.read_with(&harness.cx, |panel, cx| {
            assert!(panel.layer_drop_allowed(leaf, master, LayerDropPlacement::Inside, cx));
            assert!(!panel.layer_drop_allowed(master, leaf, LayerDropPlacement::Above, cx));
            assert!(!panel.layer_drop_allowed(page, master, LayerDropPlacement::Above, cx));
            assert!(!panel.layer_drop_allowed(leaf, instance, LayerDropPlacement::Inside, cx));
        });

        emit(
            &mut harness,
            LayersPanelAction::MoveRequested {
                node_id: row_id(leaf),
                target_node_id: row_id(master),
                position: LayersPanelDropPosition::Inside,
            },
        );
        assert_eq!(
            with_doc(&harness, |doc| doc.scene.get(leaf).unwrap().parent),
            Some(master)
        );
        assert!(harness.panel.read_with(&harness.cx, |panel, _| {
            panel.expanded_nodes.contains(&master)
        }));
    }

    #[gpui::test]
    async fn context_menu_z_order_component_and_copy_actions_hit_real_operations(
        cx: &mut TestAppContext,
    ) {
        let mut harness = setup(cx, 3).await;
        let leaves = harness.fixture.leaves.clone();
        let frame = harness.fixture.frame;
        let master = harness.fixture.master;
        let instance = harness.fixture.instance;

        // Bring to front / send to back reorder among siblings (bottom-first
        // scene order: the last child is the topmost row).
        context_action(
            &mut harness,
            leaves[0],
            LayersPanelContextAction::BringToFront,
        );
        assert_eq!(
            with_doc(&harness, |doc| doc.scene.children_of(Some(frame)).to_vec()),
            vec![leaves[1], leaves[2], leaves[0]]
        );
        context_action(
            &mut harness,
            leaves[2],
            LayersPanelContextAction::SendToBack,
        );
        assert_eq!(
            with_doc(&harness, |doc| doc.scene.children_of(Some(frame)).to_vec()),
            vec![leaves[2], leaves[1], leaves[0]]
        );

        // Create component promotes the frame into a master; detach turns the
        // instance into a plain group.
        context_action(
            &mut harness,
            frame,
            LayersPanelContextAction::CreateComponent,
        );
        assert!(with_doc(&harness, |doc| doc
            .components
            .defs
            .values()
            .any(|def| def.root == frame)));
        context_action(
            &mut harness,
            instance,
            LayersPanelContextAction::DetachInstance,
        );
        assert!(with_doc(&harness, |doc| matches!(
            doc.scene.get(instance).unwrap().data,
            NodeData::Group(_)
        )));

        select_on_canvas(&mut harness, master);
        assert_eq!(selection(&harness), vec![master]);

        // Copy puts the selection on the clipboard.
        select_on_canvas(&mut harness, leaves[1]);
        context_action(&mut harness, leaves[1], LayersPanelContextAction::Copy);
        let clipboard = harness.cx.update(|_, cx| cx.read_from_clipboard());
        assert!(
            clipboard.is_some_and(|item| !item.text().unwrap_or_default().is_empty()),
            "copy must place the selected layer on the clipboard"
        );
    }

    #[gpui::test]
    async fn main_component_action_navigates_from_an_instance_to_its_master(
        cx: &mut TestAppContext,
    ) {
        let mut harness = setup(cx, 1).await;
        let master = harness.fixture.master;
        let instance = harness.fixture.instance;
        select_on_canvas(&mut harness, instance);
        context_action(
            &mut harness,
            instance,
            LayersPanelContextAction::GoToMainComponent,
        );
        assert_eq!(selection(&harness), vec![master]);
        let row = row_bounds(&mut harness, master).expect("the master row is shown");
        assert!(row.size.height > px(0.));
    }

    #[gpui::test]
    async fn collapsing_the_panel_header_frees_the_sidebar_and_a_large_page_renders(
        cx: &mut TestAppContext,
    ) {
        let mut harness = setup(cx, 3_000).await;
        let frame = harness.fixture.frame;
        let leaves = harness.fixture.leaves.clone();
        // The whole page is echoed once (3,003 items) and only the viewport's
        // rows are built.
        emit(
            &mut harness,
            LayersPanelAction::ExpansionChanged {
                node_id: row_id(frame),
                expanded: true,
            },
        );
        select_on_canvas(&mut harness, leaves[2_999]);
        let shown = harness
            .layers
            .read_with(&harness.cx, |layers, _| layers.visible_row_ids().len());
        assert_eq!(shown, 3_003);
        assert!(row_bounds(&mut harness, leaves[2_999]).is_some());
        assert!(row_bounds(&mut harness, leaves[0]).is_none());

        let expanded_height = harness
            .cx
            .debug_bounds("layers-panel")
            .expect("panel renders")
            .size
            .height;
        emit(
            &mut harness,
            LayersPanelAction::PanelExpansionChanged { expanded: false },
        );
        assert!(
            harness
                .panel
                .read_with(&harness.cx, |panel, _| panel.layers_collapsed)
        );
        harness.layers.update_in(&mut harness.cx, |layers, _, cx| {
            layers.set_expanded(false, cx);
        });
        harness.cx.run_until_parked();
        let collapsed_height = harness
            .cx
            .debug_bounds("layers-panel")
            .expect("panel renders")
            .size
            .height;
        assert!(
            collapsed_height < expanded_height,
            "the collapsed panel must give the sidebar back: {collapsed_height:?} < {expanded_height:?}"
        );
    }
}

#[cfg(feature = "fanta-gpui-ui")]
impl FantaDesignPanel {
    /// Build the PagesPanel adapter once a window is available and
    /// gpui_component has been initialized; no-op otherwise.
    fn ensure_gpui_pages(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.gpui_pages.is_some() || !crate::gpui_adapters::runtime_enabled(cx) {
            return;
        }
        let panel = cx.new(|cx| {
            fanta_gpui::pages::PagesPanel::new("fanta-gpui-pages", Vec::new(), window, cx)
        });
        let subscription = cx.subscribe_in(&panel, window, Self::handle_pages_action);
        self.gpui_pages = Some(crate::gpui_adapters::pages::PagesAdapter {
            panel,
            id_map: std::collections::HashMap::new(),
            result_map: std::collections::HashMap::new(),
            result_order: Vec::new(),
            last_search: None,
            _subscription: subscription,
        });
        self.refresh_gpui_pages(cx);
    }

    /// Echo document state into the panel: page rows, selection, and the
    /// open search (re-run against the fresh document).
    fn refresh_gpui_pages(&mut self, cx: &mut App) {
        let Some(view) = self.active_view(cx) else {
            return;
        };
        let Some(adapter) = self.gpui_pages.as_mut() else {
            return;
        };
        let item = view.read(cx).item().clone();
        let fig_item = item.read(cx);
        let Some(document) = fig_item.document() else {
            return;
        };
        let (items, id_map) = crate::gpui_adapters::pages::pages_view_data(document);
        let selected = self
            .current_page_index
            .and_then(|index| document.pages.get(index).map(|page| (index, page.root)))
            .map(|(index, root)| crate::gpui_adapters::pages::page_id(root, index));
        let search = adapter.last_search.clone();
        let search_update = search.as_ref().map(|request| {
            crate::gpui_adapters::pages::search_pages(document, self.current_page_index, request)
        });
        adapter.id_map = id_map;
        adapter.panel.update(cx, |panel, cx| {
            panel.set_pages(items, cx);
            panel.set_selected_page(selected, cx);
            if let Some((results, hit_map, order)) = search_update {
                panel.set_search_results(results, cx);
                let adapter_maps = (hit_map, order);
                // written back below; panel update borrow ends first
                cx.notify();
                let _ = adapter_maps;
            }
        });
        if let Some(request) = search {
            let fig_item = item.read(cx);
            if let Some(document) = fig_item.document() {
                let (_, hit_map, order) = crate::gpui_adapters::pages::search_pages(
                    document,
                    self.current_page_index,
                    &request,
                );
                if let Some(adapter) = self.gpui_pages.as_mut() {
                    adapter.result_map = hit_map;
                    adapter.result_order = order;
                }
            }
        }
    }

    fn select_search_hit(
        &mut self,
        hit: crate::gpui_adapters::pages::SearchHit,
        cx: &mut Context<Self>,
    ) {
        let Some(view) = self.active_view(cx) else {
            return;
        };
        let switch_page = self.current_page_index != Some(hit.page_index);
        view.update(cx, |view, cx| {
            if switch_page {
                view.select_page(hit.page_index, cx);
            }
        });
        self.select_node(hit.node, false, cx);
    }

    fn handle_pages_action(
        &mut self,
        _panel: &Entity<fanta_gpui::pages::PagesPanel>,
        action: &fanta_gpui::pages::PagesPanelAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use fanta_gpui::pages::PagesPanelAction;
        match action {
            PagesPanelAction::SelectRequested { page_id } => {
                let index = self
                    .gpui_pages
                    .as_ref()
                    .and_then(|adapter| adapter.id_map.get(page_id))
                    .map(|page| page.index);
                if let Some(index) = index {
                    self.select_page(index, cx);
                }
            }
            PagesPanelAction::CreateRequested { title } => {
                self.add_page(cx);
                // `add_page` names the page "Page N" and selects it; apply
                // the requested title on top when one was typed.
                let title = title.trim();
                if !title.is_empty() {
                    let root = self.active_view(cx).and_then(|view| {
                        let item = view.read(cx).item().clone();
                        let fig_item = item.read(cx);
                        fig_item
                            .document()
                            .and_then(|document| document.pages.last())
                            .and_then(|page| page.root)
                    });
                    if let Some(root) = root {
                        self.rename_node_to(root, title.to_string(), cx);
                    }
                }
            }
            PagesPanelAction::RenameRequested { page_id, title } => {
                let root = self
                    .gpui_pages
                    .as_ref()
                    .and_then(|adapter| adapter.id_map.get(page_id))
                    .and_then(|page| page.root);
                if let Some(root) = root {
                    self.rename_node_to(root, title.to_string(), cx);
                }
            }
            PagesPanelAction::DeleteRequested { page_id } => {
                let index = self
                    .gpui_pages
                    .as_ref()
                    .and_then(|adapter| adapter.id_map.get(page_id))
                    .map(|page| page.index);
                if let Some(index) = index {
                    self.delete_page(index, cx);
                }
            }
            // Both of these are hard-coded entries in the vendored Pages
            // panel's page menu, which offers the host no way to hide them.
            // Duplicating a page has no document operation yet, and the
            // `fanta://page/<id>` link the old Copy-link arm put on the
            // clipboard resolves to nothing — `open_listener` answers every
            // `fanta://` url by focusing the app. Handing the user a link that
            // silently goes nowhere is worse than declining to make one.
            PagesPanelAction::DuplicateRequested { .. } => {
                crate::view::notify_unavailable("Duplicating a page", window, cx);
            }
            PagesPanelAction::CopyLinkRequested { .. } => {
                crate::view::notify_unavailable("Links to a page", window, cx);
            }
            PagesPanelAction::SearchRequested(request) => {
                self.run_gpui_pages_search(request.clone(), cx);
            }
            PagesPanelAction::SearchClosed => {
                if let Some(adapter) = self.gpui_pages.as_mut() {
                    adapter.last_search = None;
                    adapter.result_map.clear();
                    adapter.result_order.clear();
                }
            }
            PagesPanelAction::SearchResultSelected { result_id } => {
                let hit = self
                    .gpui_pages
                    .as_ref()
                    .and_then(|adapter| adapter.result_map.get(result_id))
                    .copied();
                if let Some(hit) = hit {
                    self.select_search_hit(hit, cx);
                }
            }
            PagesPanelAction::NavigateResults {
                direction,
                result_id,
            } => {
                use fanta_gpui::pages::PagesPanelResultDirection;
                let hit = self.gpui_pages.as_ref().and_then(|adapter| {
                    if adapter.result_order.is_empty() {
                        return None;
                    }
                    let current = result_id
                        .as_ref()
                        .and_then(|id| adapter.result_order.iter().position(|other| other == id));
                    let target = match (direction, current) {
                        (PagesPanelResultDirection::Next, Some(index)) => {
                            (index + 1) % adapter.result_order.len()
                        }
                        (PagesPanelResultDirection::Previous, Some(index)) => {
                            (index + adapter.result_order.len() - 1) % adapter.result_order.len()
                        }
                        (PagesPanelResultDirection::Next, None) => 0,
                        (PagesPanelResultDirection::Previous, None) => {
                            adapter.result_order.len() - 1
                        }
                    };
                    let id = adapter.result_order[target].clone();
                    adapter.result_map.get(&id).copied()
                });
                if let Some(hit) = hit {
                    self.select_search_hit(hit, cx);
                }
            }
            PagesPanelAction::ReplaceRequested {
                request,
                result_id,
                replacement,
            } => {
                let hits: Vec<crate::gpui_adapters::pages::SearchHit> = self
                    .gpui_pages
                    .as_ref()
                    .and_then(|adapter| {
                        result_id
                            .as_ref()
                            .and_then(|id| adapter.result_map.get(id).copied())
                    })
                    .into_iter()
                    .collect();
                self.apply_gpui_pages_replace(request, &hits, replacement, "Replace", cx);
            }
            PagesPanelAction::ReplaceAllRequested {
                request,
                replacement,
            } => {
                let hits: Vec<crate::gpui_adapters::pages::SearchHit> = self
                    .gpui_pages
                    .as_ref()
                    .map(|adapter| {
                        adapter
                            .result_order
                            .iter()
                            .filter_map(|id| adapter.result_map.get(id).copied())
                            .collect()
                    })
                    .unwrap_or_default();
                self.apply_gpui_pages_replace(request, &hits, replacement, "Replace all", cx);
            }
            PagesPanelAction::ExpansionChanged { .. } => {}
        }
    }

    fn run_gpui_pages_search(
        &mut self,
        request: fanta_gpui::pages::PagesPanelSearchRequest,
        cx: &mut Context<Self>,
    ) {
        let Some(view) = self.active_view(cx) else {
            return;
        };
        let item = view.read(cx).item().clone();
        let fig_item = item.read(cx);
        let Some(document) = fig_item.document() else {
            return;
        };
        let (results, hit_map, order) =
            crate::gpui_adapters::pages::search_pages(document, self.current_page_index, &request);
        if let Some(adapter) = self.gpui_pages.as_mut() {
            adapter.last_search = Some(request);
            adapter.result_map = hit_map;
            adapter.result_order = order;
            adapter
                .panel
                .update(cx, |panel, cx| panel.set_search_results(results, cx));
        }
    }

    /// One undo step per replace gesture: name hits via `SetName`, text hits
    /// via `ReplaceData` on the text node, inside a history transaction.
    fn apply_gpui_pages_replace(
        &mut self,
        request: &fanta_gpui::pages::PagesPanelSearchRequest,
        hits: &[crate::gpui_adapters::pages::SearchHit],
        replacement: &str,
        label: &'static str,
        cx: &mut Context<Self>,
    ) {
        use crate::gpui_adapters::pages::{HitField, replace_matches};
        if hits.is_empty() {
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
        let query = request.query.to_string();
        let match_case = request.match_case;
        let hits = hits.to_vec();
        let replacement = replacement.to_string();
        let result: Option<anyhow::Result<()>> = item.update(cx, |item, cx| {
            item.with_document(cx, |document| {
                let doc = &mut document.doc;
                let mut operations = Vec::new();
                for hit in &hits {
                    let Some(node) = doc.scene.get(hit.node) else {
                        continue;
                    };
                    match hit.field {
                        HitField::Name => {
                            let old = node.name.clone();
                            let new = replace_matches(&old, &query, &replacement, match_case);
                            if new != old {
                                operations.push(Operation::SetName {
                                    id: hit.node,
                                    old,
                                    new,
                                });
                            }
                        }
                        HitField::TextContent => {
                            if let NodeData::Text(text) = &node.data {
                                let mut updated = text.clone();
                                updated.content = replace_matches(
                                    &text.content,
                                    &query,
                                    &replacement,
                                    match_case,
                                );
                                if updated.content != text.content {
                                    operations.push(Operation::ReplaceData {
                                        id: hit.node,
                                        old: Box::new(node.data.clone()),
                                        new: Box::new(NodeData::Text(updated)),
                                    });
                                }
                            }
                        }
                    }
                }
                if operations.is_empty() {
                    return (Ok(()), DocChange::None);
                }
                doc.history.begin(label, &mut doc.scene);
                for operation in operations {
                    if let Err(error) = doc.apply(operation) {
                        let rollback = doc.history.abort(&mut doc.scene);
                        let error = match rollback {
                            Ok(()) => anyhow::anyhow!("{label}: {error}"),
                            Err(rollback_error) => {
                                anyhow::anyhow!("{label}: {error}; rolling back: {rollback_error}")
                            }
                        };
                        return (Err(error), DocChange::None);
                    }
                }
                doc.history.commit(&mut doc.scene);
                (Ok(()), DocChange::Content)
            })
        });
        if let Some(Err(error)) = result {
            log::error!("fanta-gpui pages: {label} failed: {error:#}");
        }
        // The Edited echo refreshes rows; re-run the search so result rows
        // reflect the replacement immediately.
        if let Some(request) = self
            .gpui_pages
            .as_ref()
            .and_then(|adapter| adapter.last_search.clone())
        {
            self.run_gpui_pages_search(request, cx);
        }
    }

    /// Renames a node with the SetName pattern the native rename editor uses.
    fn rename_node_to(&mut self, node: NodeId, new_name: String, cx: &mut Context<Self>) {
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
            log::error!("fanta-gpui pages: rename failed: {error:#}");
        }
    }

    /// The mounted PagesPanel wrapped to respect the section height, or None
    /// when the adapter is off (native section renders instead).
    fn gpui_pages_section_element(&self, _cx: &mut Context<Self>) -> Option<AnyElement> {
        let adapter = self.gpui_pages.as_ref()?;
        Some(
            v_flex()
                .flex_none()
                .max_h(self.pages_height + px(96.))
                .overflow_hidden()
                .child(adapter.panel.clone())
                .into_any_element(),
        )
    }
}

#[cfg(feature = "fanta-gpui-ui")]
impl FantaDesignPanel {
    fn ensure_gpui_layers(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.gpui_layers.is_some() || !crate::gpui_adapters::runtime_enabled(cx) {
            return;
        }
        let panel = cx.new(|cx| {
            fanta_gpui::layers::LayersPanel::new("fanta-gpui-layers", Vec::new(), window, cx)
        });
        // The drop highlight consults `layer_move_operations` through this
        // hook, so it agrees with what `drop_layer` will accept.
        let host = cx.entity().downgrade();
        let validator: fanta_gpui::layers::LayersDropValidator =
            std::rc::Rc::new(move |dragged, target, position, cx| {
                use crate::gpui_adapters::layers::{drop_placement, node_id};
                let (Some(dragged), Some(target)) = (node_id(dragged), node_id(target)) else {
                    return false;
                };
                host.upgrade().is_some_and(|host| {
                    host.read(cx)
                        .layer_drop_allowed(dragged, target, drop_placement(position), cx)
                })
            });
        panel.update(cx, |panel, cx| {
            panel.set_drop_validator(Some(validator), cx)
        });
        let subscription = cx.subscribe_in(&panel, window, Self::handle_layers_action);
        self.gpui_layers = Some(crate::gpui_adapters::layers::LayersAdapter {
            panel,
            tree_key: None,
            _subscription: subscription,
        });
        self.refresh_gpui_layers(cx);
    }

    /// Echo the layer tree, selection, and expansion into the panel, then
    /// scroll a pending reveal into view. The tree read model is memoized on
    /// (page root, render generation): selection changes — every canvas
    /// click — reach the panel as two id-list setters, never as a rebuild of
    /// a 30k-item tree.
    fn refresh_gpui_layers(&mut self, cx: &mut App) {
        let refresh_started = std::time::Instant::now();
        let Some(view) = self.active_view(cx) else {
            return;
        };
        if self.gpui_layers.is_none() {
            return;
        }
        let view = view.read(cx);
        let fig_item = view.item().read(cx);
        let Some(document) = fig_item.document() else {
            return;
        };
        let doc = &document.doc;
        let page_root = Self::layers_page_root(view, document);
        let tree_key = (page_root, document.render_generation());
        let tree = (self
            .gpui_layers
            .as_ref()
            .is_some_and(|adapter| adapter.tree_key != Some(tree_key)))
        .then(|| crate::gpui_adapters::layers::layers_tree(doc, page_root));
        let selected: Vec<SharedString> = doc
            .selection
            .iter()
            .map(|id| SharedString::from(id.to_string()))
            .collect();
        let expanded: Vec<SharedString> = self
            .expanded_nodes
            .iter()
            .map(|id| SharedString::from(id.to_string()))
            .collect();
        let reveal = self
            .pending_reveal
            .take()
            .map(|id| SharedString::from(id.to_string()));
        if let Some(adapter) = self.gpui_layers.as_mut() {
            adapter.tree_key = Some(tree_key);
            adapter.panel.update(cx, |panel, cx| {
                if let Some(tree) = tree {
                    panel.set_nodes(tree, cx);
                }
                panel.set_selected_node_ids(selected, cx);
                panel.set_expanded_node_ids(expanded, cx);
                if let Some(reveal) = reveal {
                    panel.reveal_node(&reveal, cx);
                }
            });
        }
        crate::report_slow("gpui layers refresh", refresh_started);
    }

    fn handle_layers_action(
        &mut self,
        _panel: &Entity<fanta_gpui::layers::LayersPanel>,
        action: &fanta_gpui::layers::LayersPanelAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use crate::gpui_adapters::layers::{drop_placement, node_id};
        use fanta_gpui::layers::{LayersPanelAction, LayersPanelSelectionMode};
        match action {
            LayersPanelAction::SelectRequested { node_id: id, mode } => {
                let Some(id) = node_id(id) else {
                    return;
                };
                match mode {
                    LayersPanelSelectionMode::Replace => self.select_node(id, false, cx),
                    LayersPanelSelectionMode::Toggle => self.select_node(id, true, cx),
                    LayersPanelSelectionMode::Range => self.select_node_range_to(id, cx),
                }
            }
            LayersPanelAction::ExpansionChanged {
                node_id: id,
                expanded,
            } => {
                let Some(id) = node_id(id) else {
                    return;
                };
                if *expanded {
                    self.expanded_nodes.insert(id);
                } else {
                    self.expanded_nodes.remove(&id);
                }
                cx.notify();
            }
            LayersPanelAction::CollapseAllRequested => {
                self.expanded_nodes.clear();
                self.refresh_gpui_layers(cx);
                cx.notify();
            }
            LayersPanelAction::RenameRequested { node_id: id, title } => {
                let Some(id) = node_id(id) else {
                    return;
                };
                self.rename_node_to(id, title.to_string(), cx);
            }
            LayersPanelAction::VisibilityChanged { node_id: id, .. } => {
                let Some(id) = node_id(id) else {
                    return;
                };
                self.toggle_node_flag(id, NodeFlags::HIDDEN, cx);
            }
            LayersPanelAction::LockChanged { node_id: id, .. } => {
                let Some(id) = node_id(id) else {
                    return;
                };
                self.toggle_node_flag(id, NodeFlags::LOCKED, cx);
            }
            LayersPanelAction::MoveRequested {
                node_id: dragged,
                target_node_id: target,
                position,
            } => {
                let (Some(dragged), Some(target)) = (node_id(dragged), node_id(target)) else {
                    return;
                };
                self.drop_layer(dragged, target, drop_placement(*position), cx);
            }
            LayersPanelAction::ContextActionRequested {
                node_id: id,
                action,
            } => {
                let Some(id) = node_id(id) else {
                    return;
                };
                self.handle_layers_context_action(id, *action, window, cx);
            }
            LayersPanelAction::PanelExpansionChanged { expanded } => {
                self.layers_collapsed = !expanded;
                cx.notify();
            }
        }
    }

    /// The context-menu entries the host has an operation for. Everything
    /// else is Figma-only or has no engine op yet; those are declined out
    /// loud rather than faked or silently dropped.
    fn handle_layers_context_action(
        &mut self,
        id: NodeId,
        action: fanta_gpui::layers::LayersPanelContextAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use fanta_gpui::layers::LayersPanelContextAction;
        match action {
            LayersPanelContextAction::ShowHide => self.toggle_node_flag(id, NodeFlags::HIDDEN, cx),
            LayersPanelContextAction::LockUnlock => {
                self.toggle_node_flag(id, NodeFlags::LOCKED, cx)
            }
            // The panel opens its own inline rename for Rename.
            LayersPanelContextAction::Rename => {}
            // Opening the menu selected the row, so the selection is the node.
            LayersPanelContextAction::Copy => {
                if let Some(view) = self.active_view(cx) {
                    view.update(cx, |view, cx| view.copy_selected_nodes(cx));
                }
            }
            LayersPanelContextAction::BringToFront => self.move_layer_to_extreme(id, true, cx),
            LayersPanelContextAction::SendToBack => self.move_layer_to_extreme(id, false, cx),
            LayersPanelContextAction::CreateComponent => {
                self.apply_document_ops(
                    "Create component",
                    |doc| crate::properties_ops::create_component_operations(doc, id),
                    cx,
                );
            }
            LayersPanelContextAction::DetachInstance => {
                self.apply_document_ops(
                    "Detach instance",
                    |doc| crate::properties_ops::detach_instance_operations(doc, id),
                    cx,
                );
            }
            LayersPanelContextAction::GoToMainComponent => {
                let master = self.active_view(cx).and_then(|view| {
                    let item = view.read(cx).item().read(cx);
                    let document = item.document()?;
                    let NodeData::Instance(instance) = &document.doc.scene.get(id)?.data else {
                        return None;
                    };
                    document
                        .doc
                        .components
                        .defs
                        .get(&instance.component)
                        .map(|def| def.root)
                });
                if let Some(master) = master {
                    self.focus_component(master, cx);
                }
            }
            other => crate::view::notify_unavailable(other.label(), window, cx),
        }
    }

    /// Shift-click range selection over the rows the panel currently shows,
    /// anchored on the selection anchor (the last plain click). The anchor is
    /// echoed last so it survives as the anchor of the new selection and a
    /// second shift-click extends from the same row.
    fn select_node_range_to(&mut self, target: NodeId, cx: &mut Context<Self>) {
        let Some(view) = self.active_view(cx) else {
            return;
        };
        let rows: Vec<NodeId> = self
            .gpui_layers
            .as_ref()
            .map(|adapter| {
                adapter
                    .panel
                    .read(cx)
                    .visible_row_ids()
                    .iter()
                    .filter_map(crate::gpui_adapters::layers::node_id)
                    .collect()
            })
            .unwrap_or_default();
        let Some(end) = rows.iter().position(|id| *id == target) else {
            self.select_node(target, false, cx);
            return;
        };
        let item = view.read(cx).item().clone();
        let anchor = {
            let fig_item = item.read(cx);
            let Some(document) = fig_item.document() else {
                return;
            };
            document
                .doc
                .selection
                .anchor()
                .and_then(|anchor| rows.iter().position(|id| *id == anchor))
                .or_else(|| {
                    rows.iter()
                        .position(|id| document.doc.selection.contains(*id))
                })
                .unwrap_or(end)
        };
        let (from, to) = (anchor.min(end), anchor.max(end));
        let anchor_id = rows[anchor];
        let mut ids: Vec<NodeId> = rows[from..=to]
            .iter()
            .copied()
            .filter(|id| *id != anchor_id)
            .collect();
        ids.push(anchor_id);
        view.update(cx, |view, cx| {
            view.finish_document_edits_for_external_change(cx);
        });
        item.update(cx, |item, cx| {
            item.with_document(cx, |document| {
                document.doc.selection.replace_with(ids.iter().copied());
                ((), DocChange::Selection)
            });
        });
    }

    /// The mounted LayersPanel, flexing over the sidebar remainder — or only
    /// its header tall while the user collapsed it. None when the fanta-gpui
    /// runtime is off (the section renders its unavailable notice instead).
    fn gpui_layers_section_element(&self, _cx: &mut Context<Self>) -> Option<AnyElement> {
        let adapter = self.gpui_layers.as_ref()?;
        Some(
            v_flex()
                .when(!self.layers_collapsed, |section| section.flex_1())
                .when(self.layers_collapsed, |section| section.flex_none())
                .overflow_hidden()
                .child(adapter.panel.clone())
                .into_any_element(),
        )
    }
}
