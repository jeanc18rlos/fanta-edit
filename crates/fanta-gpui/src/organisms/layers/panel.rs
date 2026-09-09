use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use gpui::{
    AnyElement, App, AppContext as _, Bounds, Context, Entity, EventEmitter, FocusHandle,
    Focusable, InteractiveElement as _, IntoElement, ParentElement as _, Pixels, Point, Render,
    ScrollHandle, ScrollStrategy, SharedString, StatefulInteractiveElement as _, Styled as _,
    Subscription, UniformListScrollHandle, Window, div, prelude::FluentBuilder as _, px,
};
use gpui_component::{
    ActiveTheme as _, Icon, IconName, Sizable as _, StyledExt as _, h_flex,
    input::{Input, InputEvent, InputState, SelectAll},
    scroll::Scrollbar,
    tooltip::Tooltip,
    v_flex,
};

use crate::atoms::{
    ActivateControl, CONTROL_KEY_CONTEXT, ControlExt as _, ControlIcon, icon_button,
    render_control_icon, track_bounds, truncating_label,
};
use crate::molecules::list_row;

use super::{
    CloseLayersOverlay, CollapseLayer, ConfirmLayersTextEntry, ExpandLayer, FocusNextLayer,
    FocusPreviousLayer, LAYERS_PANEL_KEY_CONTEXT, LAYERS_TEXT_ENTRY_KEY_CONTEXT, LayersPanelAction,
    LayersPanelContextAction, LayersPanelDropPosition, LayersPanelItem, LayersPanelNodeKind,
    LayersPanelSelectionMode, OpenLayerContextMenu, ToggleLayerLock, ToggleLayerVisibility,
};

mod events;
mod menu;
#[cfg(test)]
mod tests;
mod tree;

/// Narrowest width the panel lays out without clipping: Figma's narrow
/// left rail. Layer titles truncate ahead of their lock/visibility
/// satellites down to this floor (ARCHITECTURE.md §5).
pub const LAYERS_PANEL_MIN_WIDTH: f32 = 240.;
/// Shortest height the expanded panel stays useful at; the tree scrolls.
pub const LAYERS_PANEL_MIN_HEIGHT: f32 = 400.;

const HEADER_HEIGHT: f32 = 40.;
/// Every tree row — plain, editing, or dragging preview — is this tall. The
/// tree is virtualized with `uniform_list`, which measures the first row and
/// positions the rest arithmetically, so the height must be uniform.
const LAYER_ROW_HEIGHT: f32 = 28.;
/// Indent per nesting level. Figma uses a compact step so deep trees keep a
/// legible title column inside a narrow rail.
const LAYER_INDENT: f32 = 12.;
/// Horizontal room a row always keeps for its icon slots, title, and
/// satellites: the indent is capped so a deeply nested row never pushes its
/// content out of the panel.
const LAYER_ROW_MIN_CONTENT_WIDTH: f32 = 132.;
const LAYER_ICON_SLOT: f32 = 18.;

/// Host hook consulted while a row is dragged over another: `(dragged id,
/// target id, placement)` -> whether the drop would be accepted. Lets the
/// drop highlight tell the truth about document rules (page roots, component
/// masters, recursive instances, …) the panel cannot know from the tree alone.
pub type LayersDropValidator =
    Rc<dyn Fn(&SharedString, &SharedString, LayersPanelDropPosition, &App) -> bool>;

/// One flattened tree entry: the host item's scalar fields plus its
/// pre-order position. Ancestry, subtree membership, and visibility are all
/// answered from indices, so per-frame work never walks (or clones) the
/// host's tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct LayerNode {
    pub(super) id: SharedString,
    pub(super) title: SharedString,
    pub(super) kind: LayersPanelNodeKind,
    pub(super) visible: bool,
    pub(super) locked: bool,
    pub(super) depth: usize,
    pub(super) parent: Option<usize>,
    /// One past the last pre-order index of this node's subtree; the subtree
    /// is exactly `index + 1..subtree_end`.
    pub(super) subtree_end: usize,
    /// The host's `LayersPanelItem::has_children` hint: the node has children
    /// the host left out of the tree (a pruned collapsed subtree).
    pub(super) has_children: bool,
}

/// The host tree flattened in pre-order (parents before children, siblings in
/// render order) with an id → index map. Built once per `set_nodes`.
#[derive(Default, Debug)]
pub(super) struct LayerArena {
    nodes: Vec<LayerNode>,
    index_by_id: HashMap<SharedString, usize>,
}

impl LayerArena {
    pub(super) fn from_items(items: &[LayersPanelItem]) -> Self {
        let mut arena = Self::default();
        for item in items {
            arena.push_item(item, 0, None);
        }
        arena
    }

    fn push_item(&mut self, item: &LayersPanelItem, depth: usize, parent: Option<usize>) {
        let index = self.nodes.len();
        self.nodes.push(LayerNode {
            id: item.id.clone(),
            title: item.title.clone(),
            kind: item.kind,
            visible: item.visible,
            locked: item.locked,
            depth,
            parent,
            subtree_end: index + 1,
            has_children: item.has_children,
        });
        self.index_by_id.insert(item.id.clone(), index);
        for child in &item.children {
            self.push_item(child, depth + 1, Some(index));
        }
        self.nodes[index].subtree_end = self.nodes.len();
    }

    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.nodes.len()
    }

    pub(super) fn get(&self, index: usize) -> Option<&LayerNode> {
        self.nodes.get(index)
    }

    pub(super) fn index_of(&self, id: &SharedString) -> Option<usize> {
        self.index_by_id.get(id).copied()
    }

    pub(super) fn contains(&self, id: &SharedString) -> bool {
        self.index_by_id.contains_key(id)
    }

    /// Whether the row shows a disclosure arrow: it has children in the
    /// arena, or the host says it has children it did not include.
    pub(super) fn has_children(&self, index: usize) -> bool {
        self.nodes
            .get(index)
            .is_some_and(|node| node.subtree_end > index + 1 || node.has_children)
    }

    /// Whether `index` is `ancestor` itself or lies inside its subtree.
    pub(super) fn is_within(&self, index: usize, ancestor: usize) -> bool {
        self.nodes
            .get(ancestor)
            .is_some_and(|node| ancestor <= index && index < node.subtree_end)
    }

    /// Ancestor ids of `index`, root first.
    #[cfg(test)]
    pub(super) fn ancestor_ids(&self, index: usize) -> Vec<SharedString> {
        let mut ancestors = Vec::new();
        let mut cursor = self.nodes.get(index).and_then(|node| node.parent);
        while let Some(parent) = cursor {
            let node = &self.nodes[parent];
            ancestors.push(node.id.clone());
            cursor = node.parent;
        }
        ancestors.reverse();
        ancestors
    }

    /// Pre-order indices of the rows a tree with `expanded` containers shows.
    pub(super) fn visible_indices(&self, expanded: &HashSet<SharedString>) -> Vec<usize> {
        let mut visible = Vec::new();
        let mut index = 0;
        while index < self.nodes.len() {
            let node = &self.nodes[index];
            visible.push(index);
            if node.subtree_end > index + 1 && !expanded.contains(&node.id) {
                index = node.subtree_end;
            } else {
                index += 1;
            }
        }
        visible
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct LayerEditorTarget {
    node_id: SharedString,
    original_title: SharedString,
}

#[derive(Clone, Debug)]
struct LayerMenuState {
    node: LayerNode,
    anchor: Point<Pixels>,
    return_focus: Option<FocusHandle>,
}

/// The drag payload: identity and preview data only. Validity is decided at
/// hover/drop time from the arena (and the host validator), never by carrying
/// the dragged subtree around.
#[derive(Clone, Debug)]
struct LayerDrag {
    node_id: SharedString,
    title: SharedString,
    kind: LayersPanelNodeKind,
}

/// Stateful, host-controlled Figma-like Layers panel.
///
/// The host supplies the node tree, selection, expanded identifiers, visibility,
/// and lock state. The panel owns only interaction continuity such as hover,
/// focus, rename drafts, scrolling, and the open contextual menu.
///
/// The tree is virtualized: only the rows inside the viewport are built each
/// frame, so 30k-node pages render at the same cost as 30-node ones.
pub struct LayersPanel {
    id: SharedString,
    focus_handle: FocusHandle,
    menu_focus_handle: FocusHandle,
    arena: LayerArena,
    /// Arena indices of the rows currently shown, top to bottom. Rebuilt when
    /// the tree or the expansion set changes — never per frame.
    visible_rows: Vec<usize>,
    panel_expanded: bool,
    selected_node_ids: HashSet<SharedString>,
    expanded_node_ids: HashSet<SharedString>,
    editing: Option<LayerEditorTarget>,
    select_editor_after_render: bool,
    rename_input: Entity<InputState>,
    menu: Option<LayerMenuState>,
    panel_bounds: Option<Bounds<Pixels>>,
    node_row_bounds: HashMap<SharedString, Bounds<Pixels>>,
    row_focus_handles: HashMap<SharedString, FocusHandle>,
    list_scroll_handle: UniformListScrollHandle,
    menu_scroll_handle: ScrollHandle,
    hovered_node: Option<SharedString>,
    drop_validator: Option<LayersDropValidator>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<LayersPanelAction> for LayersPanel {}

impl LayersPanel {
    /// Creates a Layers panel and expands every populated root container.
    pub fn new(
        id: impl Into<SharedString>,
        nodes: Vec<LayersPanelItem>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let rename_input = cx.new(|cx| InputState::new(window, cx).placeholder("Layer name"));
        let subscription = cx.subscribe_in(
            &rename_input,
            window,
            |this, _, event: &InputEvent, window, cx| match event {
                InputEvent::PressEnter { .. } => this.commit_rename(true, window, cx),
                InputEvent::Blur => this.commit_rename(false, window, cx),
                InputEvent::Change | InputEvent::Focus => {}
            },
        );
        let expanded_node_ids: HashSet<SharedString> = nodes
            .iter()
            .filter(|node| !node.children.is_empty())
            .map(|node| node.id.clone())
            .collect();
        let arena = LayerArena::from_items(&nodes);
        let visible_rows = arena.visible_indices(&expanded_node_ids);

        Self {
            id: id.into(),
            focus_handle: cx.focus_handle(),
            menu_focus_handle: cx.focus_handle(),
            arena,
            visible_rows,
            panel_expanded: true,
            selected_node_ids: HashSet::new(),
            expanded_node_ids,
            editing: None,
            select_editor_after_render: false,
            rename_input,
            menu: None,
            panel_bounds: None,
            node_row_bounds: HashMap::new(),
            row_focus_handles: HashMap::new(),
            list_scroll_handle: UniformListScrollHandle::new(),
            menu_scroll_handle: ScrollHandle::new(),
            hovered_node: None,
            drop_validator: None,
            _subscriptions: vec![subscription],
        }
    }

    /// Replaces the host-controlled tree.
    pub fn set_nodes(&mut self, nodes: Vec<LayersPanelItem>, cx: &mut Context<Self>) {
        self.arena = LayerArena::from_items(&nodes);
        self.node_row_bounds
            .retain(|node_id, _| self.arena.contains(node_id));
        self.row_focus_handles
            .retain(|node_id, _| self.arena.contains(node_id));
        if self
            .editing
            .as_ref()
            .is_some_and(|target| !self.arena.contains(&target.node_id))
        {
            self.editing = None;
        }
        if self
            .menu
            .as_ref()
            .is_some_and(|menu| !self.arena.contains(&menu.node.id))
        {
            self.menu = None;
        }
        if self
            .hovered_node
            .as_ref()
            .is_some_and(|node_id| !self.arena.contains(node_id))
        {
            self.hovered_node = None;
        }
        self.rebuild_visible_rows();
        cx.notify();
    }

    /// Replaces the host-controlled selection.
    pub fn set_selected_node_ids(
        &mut self,
        selected_node_ids: Vec<SharedString>,
        cx: &mut Context<Self>,
    ) {
        self.selected_node_ids = selected_node_ids.into_iter().collect();
        cx.notify();
    }

    /// Replaces the host-controlled expansion state of the whole panel.
    pub fn set_expanded(&mut self, expanded: bool, cx: &mut Context<Self>) {
        self.panel_expanded = expanded;
        if !expanded {
            self.menu = None;
            self.hovered_node = None;
        }
        cx.notify();
    }

    /// Whether the panel body is disclosed (`false` shows only the header).
    pub fn is_expanded(&self) -> bool {
        self.panel_expanded
    }

    /// Replaces the host-controlled expanded node identifiers.
    pub fn set_expanded_node_ids(
        &mut self,
        expanded_node_ids: Vec<SharedString>,
        cx: &mut Context<Self>,
    ) {
        let expanded_node_ids: HashSet<SharedString> = expanded_node_ids.into_iter().collect();
        // Hosts echo expansion on every document event (each canvas click);
        // an unchanged set keeps the cached rows.
        if expanded_node_ids != self.expanded_node_ids {
            self.expanded_node_ids = expanded_node_ids;
            self.rebuild_visible_rows();
        }
        cx.notify();
    }

    /// Returns the current expanded identifiers, including locally-triggered
    /// presentation updates that a host may accept or replace.
    pub fn expanded_node_ids(&self) -> Vec<SharedString> {
        self.expanded_node_ids.iter().cloned().collect()
    }

    /// Installs (or clears) the host's drop validator. While a row is dragged
    /// over a target, the panel first rejects self/descendant targets and
    /// `Inside` on non-container kinds, then asks the validator; a target the
    /// validator refuses shows no drop highlight and never emits `MoveRequested`.
    pub fn set_drop_validator(
        &mut self,
        validator: Option<LayersDropValidator>,
        cx: &mut Context<Self>,
    ) {
        self.drop_validator = validator;
        cx.notify();
    }

    /// Scrolls the row for `node_id` into the middle of the viewport. Returns
    /// `false` (and does nothing) when the node is not currently shown —
    /// unknown, or under a collapsed ancestor; expand ancestors first through
    /// `set_expanded_node_ids`.
    pub fn reveal_node(&mut self, node_id: &SharedString, cx: &mut Context<Self>) -> bool {
        let Some(row) = self.visible_row_of(node_id) else {
            return false;
        };
        self.list_scroll_handle
            .scroll_to_item(row, ScrollStrategy::Center);
        cx.notify();
        true
    }

    /// Ids of the rows currently shown, top to bottom — the order a host needs
    /// for shift-click range selection over what the user actually sees.
    pub fn visible_row_ids(&self) -> Vec<SharedString> {
        self.visible_rows
            .iter()
            .filter_map(|index| self.arena.get(*index))
            .map(|node| node.id.clone())
            .collect()
    }

    fn rebuild_visible_rows(&mut self) {
        self.visible_rows = self.arena.visible_indices(&self.expanded_node_ids);
    }

    /// Position of `node_id` in the shown rows, if it is shown.
    pub(super) fn visible_row_of(&self, node_id: &SharedString) -> Option<usize> {
        let index = self.arena.index_of(node_id)?;
        self.visible_rows.binary_search(&index).ok()
    }

    fn render_header(&self, cx: &mut Context<Self>) -> AnyElement {
        let expanded = self.panel_expanded;
        h_flex()
            .id(SharedString::from(format!("{}-header", self.id)))
            .debug_selector(|| "layers-header".to_owned())
            .key_context(CONTROL_KEY_CONTEXT)
            .tab_index(0)
            .h(px(HEADER_HEIGHT))
            .w_full()
            .flex_shrink_0()
            .px_3()
            .justify_between()
            .border_b_1()
            .border_color(if expanded {
                cx.theme().border
            } else {
                cx.theme().transparent
            })
            .cursor_pointer()
            .hover(|style| style.bg(cx.theme().sidebar_accent.opacity(0.55)))
            .focus(|style| {
                style
                    .bg(cx.theme().sidebar_accent.opacity(0.55))
                    .border_color(cx.theme().selection)
            })
            .on_activate(cx.listener(|this, _, _, cx| {
                this.toggle_panel_expanded(cx);
            }))
            .child(
                h_flex()
                    .flex_1()
                    .min_w(px(0.))
                    .gap_1()
                    .child(
                        Icon::new(if expanded {
                            IconName::ChevronDown
                        } else {
                            IconName::ChevronRight
                        })
                        .xsmall(),
                    )
                    .child(div().text_sm().font_semibold().child("Layers")),
            )
            .child(
                icon_button(
                    SharedString::from(format!("{}-collapse-all", self.id)),
                    px(26.),
                    px(4.),
                    cx,
                )
                .debug_selector(|| "layers-collapse-all".to_owned())
                .tooltip(|window, cx| Tooltip::new("Collapse all layers").build(window, cx))
                .on_activate(cx.listener(|this, _, _, cx| {
                    cx.stop_propagation();
                    this.collapse_all(cx);
                }))
                .child(Icon::new(IconName::ChevronUp).xsmall()),
            )
            .into_any_element()
    }
}

impl Focusable for LayersPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for LayersPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.select_editor_after_render {
            self.select_editor_after_render = false;
            window.on_next_frame(|window, cx| {
                window.dispatch_action(Box::new(SelectAll), cx);
            });
        }

        v_flex()
            .id(self.id.clone())
            .debug_selector(|| "layers-panel".to_owned())
            .key_context(LAYERS_PANEL_KEY_CONTEXT)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::on_close_overlay))
            // The focused single-line Input already emitted PressEnter (which
            // commits the rename draft) before propagating Enter; consuming
            // the propagated action here keeps the keystroke's "\n" key_char
            // out of the single-line field.
            .on_action(cx.listener(|_, _: &ConfirmLayersTextEntry, _, _| {}))
            .on_action(cx.listener(Self::on_focus_previous_layer))
            .on_action(cx.listener(Self::on_focus_next_layer))
            .on_action(cx.listener(Self::on_collapse_layer))
            .on_action(cx.listener(Self::on_expand_layer))
            .relative()
            .w_full()
            .max_h_full()
            .min_h(px(0.))
            .when(self.panel_expanded, |panel| panel.size_full())
            .when(!self.panel_expanded, |panel| {
                panel.h(px(HEADER_HEIGHT)).flex_none()
            })
            .bg(cx.theme().sidebar)
            .text_color(cx.theme().sidebar_foreground)
            .border_1()
            .border_color(cx.theme().border)
            .child(track_bounds(cx.entity(), |this, bounds| {
                this.panel_bounds = Some(bounds);
            }))
            .child(self.render_header(cx))
            .when(self.panel_expanded, |panel| {
                panel.child(self.render_tree(cx))
            })
            .when(self.panel_expanded && self.menu.is_some(), |panel| {
                panel.child(self.render_context_menu(window, cx))
            })
    }
}
