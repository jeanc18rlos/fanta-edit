//! The Fanta design panel: Pages, Layers, Components, and Assets sections for
//! the active Figma canvas, mirroring the original Fanta left sidebar.

use std::collections::HashSet;
use std::ops::Range;
use std::sync::Arc;

use anyhow::Result;
use editor::{
    Editor, EditorEvent,
    actions::{Cancel, SelectAll},
};
use fanta_doc::{AssetId, CanvasNode, GroupNode, NodeData, NodeFlags, NodeId, Operation};
use fs::Fs;
use gpui::{
    AnyElement, App, AsyncWindowContext, ClickEvent, Context, DragMoveEvent, Empty, Entity,
    EventEmitter, FocusHandle, Focusable, KeyDownEvent, MouseButton, MouseDownEvent, MouseUpEvent,
    Pixels, ScrollStrategy, SharedString, Subscription, UniformListScrollHandle, WeakEntity,
    Window, actions, deferred, px, uniform_list,
};
use settings::{Settings as _, update_settings_file};
use ui::{ListHeader, ListItem, ListItemSpacing, Tooltip, prelude::*};
use util::size::format_file_size;
use workspace::{
    Workspace,
    dock::{DockPosition, Panel, PanelEvent},
};

use crate::document::{DocChange, FigPage, page_bounds};
use crate::panel_settings::FantaDesignPanelSettings;
use crate::view::FigView;

actions!(
    fanta_design_panel,
    [
        /// Toggle focus on the Fanta design panel.
        ToggleFocus
    ]
);

pub fn init(cx: &mut App) {
    cx.observe_new(|workspace: &mut Workspace, _window, _cx| {
        workspace.register_action(|workspace, _: &ToggleFocus, window, cx| {
            workspace.toggle_panel_focus::<FantaDesignPanel>(window, cx);
        });
    })
    .detach();
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Section {
    Pages,
    Layers,
    Components,
    Assets,
}

/// Estimated height of one list row, matching the `uniform_list` containers
/// which have always sized themselves as `row_count * 24`.
const SECTION_ROW_HEIGHT: f32 = 24.;
const MIN_SECTION_HEIGHT: Pixels = px(48.);
const DEFAULT_PAGES_HEIGHT: Pixels = px(168.);
const DEFAULT_COMPONENTS_HEIGHT: Pixels = px(160.);
const DEFAULT_ASSETS_HEIGHT: Pixels = px(192.);
const DIVIDER_HITBOX_SIZE: Pixels = px(6.);
/// Ceiling on any single section so the other section headers and a usable
/// slice of the Layers list always stay visible while dragging.
const SECTION_RESIZE_RESERVE: Pixels = px(160.);
/// Base indent applied to every content row so rows align under their section
/// header instead of sitting flush against the panel edge. The indent lives
/// inside the row (`ListItem::indent_level`), keeping hover targets full width.
const SECTION_INDENT_STEP: Pixels = px(12.);

fn section_list_height(row_count: usize, stored: Pixels) -> Pixels {
    px((row_count as f32 * SECTION_ROW_HEIGHT).min(stored.as_f32()))
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

#[derive(Clone, Copy)]
struct DividerDragState {
    divider: SectionDivider,
    start_mouse_y: Pixels,
    start_height: Pixels,
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
    assets_cache: Vec<(AssetId, usize)>,
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
            let filter_editor = cx.new(|cx| Editor::single_line(window, cx));
            let filter_subscription = cx.subscribe(
                &filter_editor,
                |this: &mut Self, _, event: &EditorEvent, cx| {
                    if matches!(event, EditorEvent::BufferEdited) {
                        this.rebuild_layer_rows(cx);
                        cx.notify();
                    }
                },
            );
            let rename_editor = cx.new(|cx| Editor::single_line(window, cx));
            // Clicking away from the rename field keeps the typed name (Figma /
            // Finder behavior). Enter/Escape empty `renaming_page` first, so the
            // blur they trigger is a no-op.
            let rename_subscription = cx.subscribe_in(
                &rename_editor,
                window,
                |this: &mut Self, _, event: &EditorEvent, window, cx| {
                    if matches!(event, EditorEvent::Blurred) && this.renaming.is_some() {
                        this.commit_rename(window, cx);
                    }
                },
            );
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
                assets_cache: Vec::new(),
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
                renaming: None,
                rename_editor,
                _subscriptions: vec![
                    workspace_subscription,
                    filter_subscription,
                    rename_subscription,
                ],
                _active_view_subscription: None,
            };
            this.set_active_view(initial_view, cx);
            this
        })
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
                            if !matches!(event, crate::document::FigItemEvent::EditedTransient) {
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

    fn toggle_section(&mut self, section: Section, cx: &mut Context<Self>) {
        if !self.collapsed_sections.remove(&section) {
            self.collapsed_sections.insert(section);
        }
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

    fn toggle_filter(&mut self, section: Section, window: &mut Window, cx: &mut Context<Self>) {
        if self.filter_target == Some(section) {
            self.close_filter(window, cx);
            return;
        }
        self.filter_target = Some(section);
        self.filter_editor.update(cx, |editor, cx| {
            editor.set_text("", window, cx);
            let placeholder = match section {
                Section::Components => "Filter components…",
                _ => "Filter layers…",
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
            SectionDivider::PagesLayers => {
                section_list_height(self.pages_cache.len(), self.pages_height)
            }
            SectionDivider::LayersComponents => {
                section_list_height(self.components_cache.len(), self.components_height)
            }
            SectionDivider::ComponentsAssets => {
                section_list_height(self.assets_cache.len(), self.assets_height)
            }
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
        view.update(cx, |view, cx| view.select_page(index, cx));
    }

    fn select_node(&mut self, id: NodeId, extend: bool, cx: &mut Context<Self>) {
        let Some(view) = self.active_view(cx) else {
            return;
        };
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

    fn toggle_node_flag(&mut self, id: NodeId, flag: NodeFlags, cx: &mut Context<Self>) {
        let Some(view) = self.active_view(cx) else {
            return;
        };
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
                page_node.name = format!("Page {}", document.pages.len() + 1);
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

    fn rebuild_layer_rows(&mut self, cx: &App) {
        self.layer_rows.clear();
        self.pages_cache.clear();
        self.components_cache.clear();
        self.assets_cache.clear();
        self.current_page_index = None;
        self.document_editable = false;
        self.document_ready = false;
        let Some(view) = self.active_view(cx) else {
            self.last_reveal_anchor = None;
            return;
        };
        let view = view.read(cx);
        let fig_item = view.item().read(cx);
        self.document_editable = fig_item.is_editable();
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
        self.components_cache = doc
            .components
            .defs
            .values()
            .map(|def| (SharedString::from(def.name.clone()), def.root))
            .collect();
        self.components_cache
            .sort_by(|left, right| left.0.as_ref().cmp(right.0.as_ref()));
        if let Some(query) = self.filter_query(Section::Components, cx) {
            self.components_cache
                .retain(|(name, _)| name.to_lowercase().contains(&query));
        }
        self.assets_cache = document
            .raw_assets
            .iter()
            .map(|(asset_id, bytes)| (*asset_id, bytes.len()))
            .collect();

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
        let asset_count = self.assets_cache.len();

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
        let layers_filtering = self.filter_query(Section::Layers, cx).is_some();
        let components_filtering = self.filter_query(Section::Components, cx).is_some();

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
                            .on_toggle(
                                cx.listener(|this, _, _, cx| {
                                    this.toggle_section(Section::Pages, cx)
                                }),
                            )
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
                            .on_toggle(cx.listener(|this, _, _, cx| {
                                this.toggle_section(Section::Layers, cx)
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
                            .on_toggle(cx.listener(|this, _, _, cx| {
                                this.toggle_section(Section::Components, cx)
                            }))
                            .end_slot(
                                h_flex()
                                    .gap_1()
                                    .child(
                                        IconButton::new(
                                            "fanta-components-filter",
                                            IconName::MagnifyingGlass,
                                        )
                                        .icon_size(IconSize::Small)
                                        .toggle_state(components_filter_open)
                                        .tooltip(Tooltip::text("Filter Components"))
                                        .on_click(
                                            cx.listener(|this, _, window, cx| {
                                                this.toggle_filter(Section::Components, window, cx)
                                            }),
                                        ),
                                    )
                                    .child(
                                        Label::new(component_count.to_string())
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
                                .h(section_list_height(component_count, self.components_height)),
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
                            .on_toggle(cx.listener(|this, _, _, cx| {
                                this.toggle_section(Section::Assets, cx)
                            }))
                            .end_slot(
                                Label::new(asset_count.to_string())
                                    .size(LabelSize::Small)
                                    .color(Color::Muted),
                            ),
                    )
                    .when(assets_open, |section| {
                        if asset_count == 0 {
                            section.child(empty_section_label("fanta-assets-empty", "No assets"))
                        } else {
                            section.child(
                                uniform_list(
                                    "fanta-assets",
                                    asset_count,
                                    cx.processor(|this, range: Range<usize>, _window, _cx| {
                                        let mut rows = Vec::with_capacity(range.len());
                                        for index in range {
                                            if let Some((asset_id, byte_count)) =
                                                this.assets_cache.get(index)
                                            {
                                                rows.push(this.render_asset_row(
                                                    index,
                                                    *asset_id,
                                                    *byte_count,
                                                ));
                                            }
                                        }
                                        rows
                                    }),
                                )
                                .w_full()
                                .h(section_list_height(asset_count, self.assets_height)),
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
            .px_2()
            .py_0p5()
            .gap_1p5()
            .on_action(cx.listener(|this, _: &Cancel, window, cx| this.close_filter(window, cx)))
            .child(
                Icon::new(IconName::MagnifyingGlass)
                    .size(IconSize::Small)
                    .color(Color::Muted),
            )
            .child(div().flex_1().min_w_0().child(self.filter_editor.clone()))
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
                .indent_level(1)
                .indent_step_size(SECTION_INDENT_STEP)
                .toggle_state(is_current)
                .child(self.render_rename_editor(cx))
                .into_any_element();
        }

        let root = entry.root;
        ListItem::new(("fanta-page", index))
            .spacing(ListItemSpacing::ExtraDense)
            .indent_level(1)
            .indent_step_size(SECTION_INDENT_STEP)
            .toggle_state(is_current)
            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
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
            .child(Label::new(entry.name.clone()).single_line())
            .when(deletable, |item| {
                item.end_slot(
                    IconButton::new(("fanta-page-delete", index), IconName::Close)
                        .icon_size(IconSize::XSmall)
                        .icon_color(Color::Muted)
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

    fn render_layer_row(&self, index: usize, row: &LayerRow, cx: &mut Context<Self>) -> AnyElement {
        let id = row.id;
        if matches!(self.renaming, Some(RenameTarget::Layer { node }) if node == id) {
            return ListItem::new(("fanta-layer", index))
                .indent_level(row.depth + 1)
                .indent_step_size(SECTION_INDENT_STEP)
                .spacing(ListItemSpacing::ExtraDense)
                .toggle_state(row.selected)
                .child(self.render_rename_editor(cx))
                .into_any_element();
        }
        let name_color = if row.accent {
            Color::Accent
        } else if row.hidden {
            Color::Muted
        } else {
            Color::Default
        };
        let icon_color = if row.accent {
            Color::Accent
        } else {
            Color::Muted
        };

        let mut item = ListItem::new(("fanta-layer", index))
            .indent_level(row.depth + 1)
            .indent_step_size(SECTION_INDENT_STEP)
            .spacing(ListItemSpacing::ExtraDense)
            .toggle_state(row.selected)
            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                // Double-click renames the layer in place; a single click
                // selects it (Shift/Cmd extends the selection).
                if event.click_count() >= 2 {
                    this.begin_layer_rename(id, window, cx);
                } else {
                    let modifiers = event.modifiers();
                    this.select_node(id, modifiers.secondary() || modifiers.shift, cx);
                }
            }))
            .child(
                h_flex()
                    .gap_1()
                    .child(Icon::new(row.icon).size(IconSize::Small).color(icon_color))
                    .child(Label::new(row.name.clone()).single_line().color(name_color)),
            );

        if row.has_children {
            item = item
                .toggle(Some(row.expanded))
                .always_show_disclosure_icon(true)
                .on_toggle(cx.listener(move |this, _, _, cx| this.toggle_node_expanded(id, cx)));
        }

        if self.document_editable {
            let eye_button = IconButton::new(
                ("fanta-layer-eye", index),
                if row.hidden {
                    IconName::EyeOff
                } else {
                    IconName::Eye
                },
            )
            .icon_size(IconSize::XSmall)
            .icon_color(if row.hidden {
                Color::Warning
            } else {
                Color::Muted
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
                ("fanta-layer-lock", index),
                if row.locked {
                    IconName::Lock
                } else {
                    IconName::LockOff
                },
            )
            .icon_size(IconSize::XSmall)
            .icon_color(if row.locked {
                Color::Conflict
            } else {
                Color::Muted
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

        item.into_any_element()
    }

    fn render_component_row(
        &self,
        index: usize,
        name: SharedString,
        root: NodeId,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        ListItem::new(("fanta-component", index))
            .spacing(ListItemSpacing::ExtraDense)
            .indent_level(1)
            .indent_step_size(SECTION_INDENT_STEP)
            .on_click(cx.listener(move |this, _, _, cx| this.select_node(root, false, cx)))
            .child(
                h_flex()
                    .gap_1()
                    .child(
                        Icon::new(IconName::Blocks)
                            .size(IconSize::Small)
                            .color(Color::Accent),
                    )
                    .child(Label::new(name).single_line().color(Color::Accent)),
            )
            .into_any_element()
    }

    fn render_asset_row(&self, index: usize, asset_id: AssetId, byte_count: usize) -> AnyElement {
        let full_id = SharedString::from(asset_id.to_string());
        ListItem::new(("fanta-asset", index))
            .spacing(ListItemSpacing::ExtraDense)
            .indent_level(1)
            .indent_step_size(SECTION_INDENT_STEP)
            .tooltip(Tooltip::text(full_id.clone()))
            .child(
                h_flex()
                    .gap_1()
                    .child(
                        Icon::new(IconName::Image)
                            .size(IconSize::Small)
                            .color(Color::Muted),
                    )
                    .child(Label::new(short_asset_label(&full_id)).single_line()),
            )
            .end_slot(
                Label::new(format_file_size(byte_count as u64, true))
                    .size(LabelSize::Small)
                    .color(Color::Muted),
            )
            .into_any_element()
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
        NodeData::NodeGraph(_)
        | NodeData::Model3d(_)
        | NodeData::AiArtifact(_)
        | NodeData::Embed(_) => IconName::SquareDot,
    }
}

fn short_asset_label(full_id: &str) -> SharedString {
    let tail_start = full_id.len().saturating_sub(6);
    match full_id.get(tail_start..) {
        Some(tail) if tail_start > 2 => format!("a_…{tail}").into(),
        _ => full_id.to_string().into(),
    }
}

// A non-selectable list item keeps the placeholder aligned with real rows.
fn empty_section_label(id: &'static str, text: &'static str) -> AnyElement {
    ListItem::new(id)
        .spacing(ListItemSpacing::ExtraDense)
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
