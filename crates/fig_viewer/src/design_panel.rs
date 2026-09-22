//! Host adapter for the shared GPUI FileInspectorSidebar and its Pages/Layers
//! children. Document mutations and canvas navigation remain in the editor.
#![cfg_attr(not(feature = "fanta-gpui-ui"), allow(dead_code))]

use std::collections::HashSet;
use std::sync::Arc;

use anyhow::Result;
use fanta_doc::{
    CanvasNode, Doc, GroupNode, IndexKey, NodeData, NodeFlags, NodeId, Operation, Scene,
};
use fs::Fs;
#[cfg(test)]
use gpui::px;
use gpui::{
    AnyElement, App, AsyncWindowContext, Context, Entity, EventEmitter, FocusHandle, Focusable,
    Pixels, SharedString, Subscription, WeakEntity, Window, actions,
};
use settings::{Settings as _, update_settings_file};
use ui::prelude::*;
use workspace::{
    Workspace,
    dock::{DockPosition, Panel, PanelEvent},
};

use crate::document::{DocChange, FigDocument};
use crate::panel_settings::FantaDesignPanelSettings;
use crate::view::{
    CopySelection, CutSelection, DeleteSelection, DuplicateSelection, FigView, FrameSelection,
    GroupSelection, PasteSelection, UngroupSelection,
};

actions!(
    fanta_design_panel,
    [
        /// Toggle focus on the Fanta design panel.
        ToggleFocus
    ]
);

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

pub struct FantaDesignPanel {
    focus_handle: FocusHandle,
    fs: Arc<dyn Fs>,
    active_view: Option<WeakEntity<FigView>>,
    width: Option<Pixels>,
    /// Host-controlled expansion of the layer tree, echoed into the gpui
    /// panel on every refresh (the panel's own expansion interactions come
    /// back as `ExpansionChanged` and land here). Mutated only through
    /// `set_node_expanded` / `clear_expanded_nodes`, which keep
    /// `expansion_generation` honest.
    expanded_nodes: HashSet<NodeId>,
    /// Bumped whenever `expanded_nodes` changes. The layer tree read model
    /// holds only the children of expanded containers, so it is memoized on
    /// this counter alongside the document's render generation.
    expansion_generation: u64,
    #[cfg(feature = "fanta-gpui-ui")]
    gpui_pages: Option<crate::gpui_adapters::pages::PagesAdapter>,
    #[cfg(feature = "fanta-gpui-ui")]
    gpui_layers: Option<crate::gpui_adapters::layers::LayersAdapter>,
    #[cfg(feature = "fanta-gpui-ui")]
    file_inspector: Option<Entity<fanta_gpui::file_inspector::FileInspectorSidebar>>,
    file_inspector_collapsed: bool,
    current_page_index: Option<usize>,
    /// The selection anchor the layer tree last revealed. When the anchor
    /// changes (typically from a canvas click) its ancestors are expanded
    /// and the row is scrolled into view; a repeat of the same anchor is
    /// left alone so browsing the list never yanks it around.
    last_reveal_anchor: Option<NodeId>,
    /// A node to scroll into view on the next layers echo — set by
    /// `rebuild_tree` when the reveal anchor changes, consumed by
    /// `refresh_gpui_layers` after the tree and expansion are echoed.
    pending_reveal: Option<NodeId>,
    _subscriptions: Vec<Subscription>,
    _active_view_subscription: Option<Subscription>,
}

impl FantaDesignPanel {
    pub(crate) fn set_file_inspector_collapsed(&mut self, collapsed: bool, cx: &mut Context<Self>) {
        if self.file_inspector_collapsed != collapsed {
            self.file_inspector_collapsed = collapsed;
            #[cfg(feature = "fanta-gpui-ui")]
            if let Some(panel) = &self.file_inspector {
                panel.update(cx, |panel, cx| panel.set_collapsed(collapsed, cx));
            }
            cx.notify();
        }
    }

    #[cfg(feature = "fanta-gpui-ui")]
    fn ensure_file_inspector(&mut self, cx: &mut Context<Self>) {
        if self.file_inspector.is_none() {
            let (Some(pages), Some(layers)) = (&self.gpui_pages, &self.gpui_layers) else {
                return;
            };
            let pages = pages.panel.clone();
            let layers = layers.panel.clone();
            let inspector = cx.new(|cx| {
                fanta_gpui::file_inspector::FileInspectorSidebar::new(
                    "fanta-file-inspector",
                    pages,
                    layers,
                    cx,
                )
            });
            self._subscriptions.push(
                cx.subscribe(&inspector, |this, _, event, cx| {
                    let fanta_gpui::file_inspector::FileInspectorAction::CollapsedChanged {
                        collapsed,
                    } = event;
                    this.file_inspector_collapsed = *collapsed;
                    cx.emit(FileInspectorVisibilityChanged(!collapsed));
                    cx.notify();
                }),
            );
            self.file_inspector = Some(inspector);
        }
        let title = self
            .active_view(cx)
            .map(|view| view.read(cx).item().read(cx).title())
            .unwrap_or_else(|| "Untitled".into());
        if let Some(inspector) = &self.file_inspector {
            inspector.update(cx, |inspector, cx| {
                inspector.set_project_name(title, cx);
                inspector.set_collapsed(self.file_inspector_collapsed, cx);
            });
        }
    }

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
        _window: &mut Window,
        cx: &mut Context<Self>,
        subscriptions: Vec<Subscription>,
    ) -> Self {
        let mut this = Self {
            focus_handle: cx.focus_handle(),
            fs,
            active_view: None,
            width: None,
            expanded_nodes: HashSet::new(),
            expansion_generation: 0,
            #[cfg(feature = "fanta-gpui-ui")]
            gpui_pages: None,
            #[cfg(feature = "fanta-gpui-ui")]
            gpui_layers: None,
            #[cfg(feature = "fanta-gpui-ui")]
            file_inspector: None,
            file_inspector_collapsed: false,
            current_page_index: None,
            last_reveal_anchor: None,
            pending_reveal: None,
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
                    self.clear_expanded_nodes();
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

    /// Expands or collapses one layer-tree node. A no-op change (already in
    /// that state) leaves `expansion_generation` alone so the memoized tree
    /// survives the reveal echoes that re-expand an already open chain.
    fn set_node_expanded(&mut self, id: NodeId, expanded: bool) {
        let changed = if expanded {
            self.expanded_nodes.insert(id)
        } else {
            self.expanded_nodes.remove(&id)
        };
        if changed {
            self.expansion_generation = self.expansion_generation.wrapping_add(1);
        }
    }

    fn clear_expanded_nodes(&mut self) {
        if self.expanded_nodes.is_empty() {
            return;
        }
        self.expanded_nodes.clear();
        self.expansion_generation = self.expansion_generation.wrapping_add(1);
    }

    fn active_view(&self, _cx: &App) -> Option<Entity<FigView>> {
        self.active_view.as_ref().and_then(|view| view.upgrade())
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
                        let rollback = doc.abort_transaction();
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
                    self.set_node_expanded(target, true);
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
        let Some(page_node) = ({
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
                Some(page_node)
            })
        }) else {
            return;
        };
        let root = page_node.id;

        self.apply_document_ops(
            "Add page",
            |doc| {
                let old = doc.pages().to_vec();
                let mut new = old.clone();
                new.push(root);
                vec![
                    Operation::create_node(page_node),
                    Operation::SetPages { old, new },
                ]
            },
            cx,
        );
        let new_page_index = item.read(cx).document().and_then(|document| {
            document
                .pages
                .iter()
                .position(|page| page.root == Some(root))
        });
        if let Some(index) = new_page_index {
            view.update(cx, |view, cx| view.select_page(index, cx));
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
                if document.pages.iter().filter(|page| !page.hidden).count() <= 1 {
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

        self.apply_document_ops(
            "Delete page",
            |doc| {
                let old = doc.pages().to_vec();
                let new = old.iter().copied().filter(|id| *id != root).collect();
                vec![
                    Operation::SetPages { old, new },
                    Operation::DeleteSubtree { snapshot },
                ]
            },
            cx,
        );
        let next_page_index = item.read(cx).document().and_then(|document| {
            document
                .pages
                .iter()
                .position(|page| page.root == document.doc.active_page())
        });
        if let Some(index) = next_page_index {
            view.update(cx, |view, cx| view.select_page(index, cx));
        }
    }

    // === Document caches ===================================================

    /// Refresh the current page index and the reveal
    /// bookkeeping from the active view's document. Runs on document events,
    /// never in render.
    fn rebuild_caches(&mut self, cx: &mut App) {
        let rebuild_started = std::time::Instant::now();
        self.current_page_index = None;
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

    /// Refresh page selection and the layer-tree reveal state. A pure read
    /// of the document.
    fn rebuild_tree(&mut self, view: &Entity<FigView>, cx: &App) {
        let view = view.read(cx);
        let fig_item = view.item().read(cx);
        let Some(document) = fig_item.document() else {
            return;
        };
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
        // Reveal the selection: when the anchor changes (typically from a
        // canvas click), expand its ancestor chain so its row exists, then
        // scroll it into view once the tree is echoed into the layers panel.
        let anchor = doc.selection.anchor();
        if anchor != self.last_reveal_anchor {
            self.last_reveal_anchor = anchor;
            if let Some(anchor) = anchor {
                // The page root is never a row, so expanding it would only
                // invalidate the memoized tree for nothing.
                for ancestor in doc
                    .scene
                    .ancestors_of(anchor)
                    .filter(|ancestor| Some(ancestor.id) != page_root)
                {
                    self.set_node_expanded(ancestor.id, true);
                }
                self.pending_reveal = Some(anchor);
            }
        }
    }
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
        #[cfg(feature = "fanta-gpui-ui")]
        self.ensure_file_inspector(cx);
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
                    #[cfg(feature = "fanta-gpui-ui")]
                    let inspector = self
                        .file_inspector
                        .as_ref()
                        .map(|panel| panel.clone().into_any_element());
                    #[cfg(not(feature = "fanta-gpui-ui"))]
                    let inspector: Option<AnyElement> = None;
                    inspector.unwrap_or_else(|| {
                        centered_message("File inspector needs the fanta-gpui UI runtime")
                    })
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
            .on_action(cx.listener(|this, _: &GroupSelection, window, cx| {
                if let Some(view) = this.active_view(cx) {
                    view.update(cx, |view, cx| view.group_nodes(None, window, cx));
                }
            }))
            .on_action(cx.listener(|this, _: &UngroupSelection, window, cx| {
                if let Some(view) = this.active_view(cx) {
                    view.update(cx, |view, cx| view.ungroup_nodes(None, window, cx));
                }
            }))
            .on_action(cx.listener(|this, _: &FrameSelection, window, cx| {
                if let Some(view) = this.active_view(cx) {
                    view.update(cx, |view, cx| view.frame_nodes(None, window, cx));
                }
            }))
            .size_full()
            .overflow_hidden()
            .when(!self.file_inspector_collapsed, |panel| {
                panel.bg(cx.theme().colors().editor_background)
            })
            .child(body)
    }
}

pub(crate) struct FileInspectorVisibilityChanged(pub bool);

impl EventEmitter<FileInspectorVisibilityChanged> for FantaDesignPanel {}
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
    async fn page_menu_moves_duplicates_and_deletes_round_trip_through_undo(
        cx: &mut TestAppContext,
    ) {
        use fanta_gpui::pages::{PagesPanelAction, PagesPanelMoveDirection};
        let (_panel, view, pages, first, second, mut cx) = setup_panel(cx).await;
        let item = view.read_with(&cx, |view, _| view.item().clone());
        let order = |cx: &VisualTestContext| {
            item.read_with(cx, |item, _| {
                item.document().expect("ready").doc.pages().to_vec()
            })
        };
        pages.update_in(&mut cx, |_, _, cx| {
            cx.emit(PagesPanelAction::MoveRequested {
                page_id: second.to_string().into(),
                direction: PagesPanelMoveDirection::Top,
            })
        });
        cx.run_until_parked();
        assert_eq!(order(&cx), vec![second, first]);
        assert_eq!(
            pages.read_with(&cx, |pages, _| pages.selected_page().cloned()),
            Some(first.to_string().into())
        );
        item.update_in(&mut cx, |item, _, cx| item.undo(cx).expect("undo move"));
        cx.run_until_parked();
        assert_eq!(order(&cx), vec![first, second]);
        pages.update_in(&mut cx, |_, _, cx| {
            cx.emit(PagesPanelAction::DuplicateRequested {
                page_id: first.to_string().into(),
            })
        });
        cx.run_until_parked();
        let copied = order(&cx);
        assert_eq!(copied.len(), 3);
        let copy = copied[1];
        assert_ne!(copy, first);
        item.read_with(&cx, |item, _| {
            let doc = &item.document().expect("ready").doc;
            assert_eq!(doc.scene.get(copy).expect("copy").name, "Page 1 Copy");
            assert_eq!(doc.scene.children_of(Some(copy)).len(), 1);
            assert_ne!(
                doc.scene.children_of(Some(copy)),
                doc.scene.children_of(Some(first))
            );
        });
        item.update_in(&mut cx, |item, _, cx| {
            item.undo(cx).expect("undo duplicate")
        });
        cx.run_until_parked();
        assert_eq!(order(&cx), vec![first, second]);
        item.update_in(&mut cx, |item, _, cx| {
            item.redo(cx).expect("redo duplicate")
        });
        cx.run_until_parked();
        assert_eq!(order(&cx), copied);
        pages.update_in(&mut cx, |_, _, cx| {
            cx.emit(PagesPanelAction::DeleteRequested {
                page_id: copy.to_string().into(),
            })
        });
        cx.run_until_parked();
        assert_eq!(order(&cx), vec![first, second]);
        item.update_in(&mut cx, |item, _, cx| item.undo(cx).expect("undo delete"));
        cx.run_until_parked();
        assert_eq!(order(&cx), copied);
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

    fn tree_key(harness: &Harness) -> Option<crate::gpui_adapters::layers::LayersTreeKey> {
        harness.panel.read_with(&harness.cx, |panel, _| {
            panel.gpui_layers.as_ref().unwrap().tree_key
        })
    }

    fn shown_rows(harness: &Harness) -> Vec<SharedString> {
        harness
            .layers
            .read_with(&harness.cx, |layers, _| layers.visible_row_ids())
    }

    fn disclosure_bounds(harness: &mut Harness, id: NodeId) -> Option<gpui::Bounds<Pixels>> {
        let selector: &'static str = Box::leak(format!("layers-expand-{id}").into_boxed_str());
        harness.cx.debug_bounds(selector)
    }

    #[gpui::test]
    async fn selection_changes_do_not_rebuild_the_layer_tree(cx: &mut TestAppContext) {
        let mut harness = setup(cx, 20).await;
        let leaves = harness.fixture.leaves.clone();
        let key_mounted = tree_key(&harness);
        assert!(key_mounted.is_some());
        // The first reveal opens the frame, which the pruned tree must be
        // rebuilt for; after that the chain is open and selection alone
        // never rebuilds.
        select_on_canvas(&mut harness, leaves[0]);
        let key_before = tree_key(&harness);
        assert_ne!(key_mounted, key_before, "revealing a pruned leaf rebuilds");
        select_on_canvas(&mut harness, leaves[1]);
        select_on_canvas(&mut harness, leaves[2]);
        let key_after = tree_key(&harness);
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
        let key_edited = tree_key(&harness);
        assert_ne!(key_before, key_edited);
    }

    #[gpui::test]
    async fn expanding_a_row_supplies_its_children_and_collapsing_prunes_them(
        cx: &mut TestAppContext,
    ) {
        let mut harness = setup(cx, 5).await;
        let frame = harness.fixture.frame;
        let master = harness.fixture.master;
        let instance = harness.fixture.instance;
        let leaves = harness.fixture.leaves.clone();

        // Collapsed on mount: the tree holds the page's top level only, yet
        // the frame keeps its disclosure arrow from the children hint, and
        // the childless master does not get one.
        assert_eq!(
            shown_rows(&harness),
            vec![row_id(instance), row_id(master), row_id(frame)]
        );
        assert!(disclosure_bounds(&mut harness, frame).is_some());
        assert!(disclosure_bounds(&mut harness, master).is_none());
        assert!(row_bounds(&mut harness, leaves[0]).is_none());
        let key_collapsed = tree_key(&harness);

        // The panel's expansion intent makes the host rebuild with the
        // frame's children (topmost first) — no document edit involved.
        emit(
            &mut harness,
            LayersPanelAction::ExpansionChanged {
                node_id: row_id(frame),
                expanded: true,
            },
        );
        let key_expanded = tree_key(&harness);
        assert_ne!(key_collapsed, key_expanded);
        let mut expected = vec![row_id(instance), row_id(master), row_id(frame)];
        expected.extend(leaves.iter().rev().map(|leaf| row_id(*leaf)));
        assert_eq!(shown_rows(&harness), expected);
        assert!(row_bounds(&mut harness, leaves[4]).is_some());

        // Re-echoing the same expansion (a reveal of a leaf whose chain is
        // already open) does not rebuild.
        select_on_canvas(&mut harness, leaves[2]);
        assert_eq!(tree_key(&harness), key_expanded);

        // Collapsing prunes the subtree from the tree again.
        emit(
            &mut harness,
            LayersPanelAction::ExpansionChanged {
                node_id: row_id(frame),
                expanded: false,
            },
        );
        assert_ne!(tree_key(&harness), key_expanded);
        assert_eq!(
            shown_rows(&harness),
            vec![row_id(instance), row_id(master), row_id(frame)]
        );
        assert!(disclosure_bounds(&mut harness, frame).is_some());
        let key_pruned = tree_key(&harness);

        // Collapse-all over an already collapsed tree changes nothing, so
        // the memoized tree survives.
        emit(&mut harness, LayersPanelAction::CollapseAllRequested);
        assert_eq!(tree_key(&harness), key_pruned);
        assert!(
            harness
                .panel
                .read_with(&harness.cx, |panel, _| panel.expanded_nodes.is_empty())
        );
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
    async fn cast_duplicate_delete_and_unsupported_actions_use_the_addressed_layer(
        cx: &mut TestAppContext,
    ) {
        let mut harness = setup(cx, 2).await;
        let target = harness.fixture.leaves[0];
        let other = harness.fixture.leaves[1];
        let frame = harness.fixture.frame;
        select_on_canvas(&mut harness, other);
        context_action(&mut harness, target, LayersPanelContextAction::Duplicate);
        let copied = selection(&harness)[0];
        assert_ne!(copied, target);
        assert_eq!(
            with_doc(&harness, |doc| doc.scene.children_of(Some(frame)).len()),
            3
        );
        select_on_canvas(&mut harness, other);
        context_action(&mut harness, copied, LayersPanelContextAction::Delete);
        assert!(!with_doc(&harness, |doc| doc.scene.contains(copied)));
        assert!(with_doc(&harness, |doc| doc.scene.contains(other)));
        context_action(
            &mut harness,
            target,
            LayersPanelContextAction::SendToFigmaMake,
        );
        assert!(with_doc(&harness, |doc| doc.scene.contains(target)));
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
        let (mut items, id_map) = crate::gpui_adapters::pages::pages_view_data(document);
        for page in &mut items {
            page.editable &= fig_item.is_editable();
            page.can_copy_link = id_map
                .get(&page.id)
                .and_then(|page| page.root)
                .is_some_and(|root| fig_item.has_saved_page(root));
        }
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
        _window: &mut Window,
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
            PagesPanelAction::MoveRequested { page_id, direction } => {
                let root = self
                    .gpui_pages
                    .as_ref()
                    .and_then(|adapter| adapter.id_map.get(page_id))
                    .and_then(|page| page.root);
                if let Some(root) = root {
                    self.apply_document_ops(
                        "Move page",
                        |doc| {
                            let old = doc.pages().to_vec();
                            let visible: Vec<_> = old
                                .iter()
                                .copied()
                                .filter(|id| {
                                    doc.scene
                                        .get(*id)
                                        .and_then(|node| node.meta.get("hidden_page"))
                                        .and_then(|value| value.as_bool())
                                        != Some(true)
                                })
                                .collect();
                            let Some(index) = visible.iter().position(|id| *id == root) else {
                                return Vec::new();
                            };
                            let Some(target) = direction.destination(index, visible.len()) else {
                                return Vec::new();
                            };
                            let mut reordered = visible.clone();
                            reordered.remove(index);
                            reordered.insert(target, root);
                            let mut replacements = reordered.into_iter();
                            let new = old
                                .iter()
                                .map(|id| {
                                    if visible.contains(id) {
                                        replacements.next().unwrap_or(*id)
                                    } else {
                                        *id
                                    }
                                })
                                .collect();
                            vec![Operation::SetPages { old, new }]
                        },
                        cx,
                    );
                }
            }
            PagesPanelAction::DuplicateRequested { page_id } => {
                let root = self
                    .gpui_pages
                    .as_ref()
                    .and_then(|adapter| adapter.id_map.get(page_id))
                    .and_then(|page| page.root);
                if let Some(root) = root {
                    self.apply_document_ops(
                        "Duplicate page",
                        |doc| match crate::clipboard::duplicate_page_operations(doc, root) {
                            Ok(operations) => operations,
                            Err(error) => {
                                log::error!("Cannot duplicate page: {error:#}");
                                Vec::new()
                            }
                        },
                        cx,
                    );
                }
            }
            PagesPanelAction::CopyLinkRequested { page_id } => {
                let root = self
                    .gpui_pages
                    .as_ref()
                    .and_then(|adapter| adapter.id_map.get(page_id))
                    .and_then(|page| page.root);
                let path = root.and_then(|root| {
                    self.active_view(cx).and_then(|view| {
                        let item = view.read(cx).item().read(cx);
                        item.project_root()
                            .and_then(|path| fanta_format::locate_page_source(path, root))
                    })
                });
                if let Some(path) = path
                    && let Ok(url) = url::Url::from_file_path(path)
                {
                    let link = url.as_str().replacen("file://", "fanta://file", 1);
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(link));
                }
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
                        let rollback = doc.abort_transaction();
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

    /// Apply a page or layer rename as a document operation.
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
    /// scroll a pending reveal into view. The tree read model holds only the
    /// children of expanded containers and is memoized on
    /// [`LayersTreeKey`](crate::gpui_adapters::layers::LayersTreeKey):
    /// selection changes — every canvas click — reach the panel as two
    /// id-list setters, never as a rebuild, and a rebuild (an edit, or an
    /// expansion toggle) costs the shown rows, not the 30k-node page.
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
        let tree_key = crate::gpui_adapters::layers::LayersTreeKey {
            page_root,
            render_generation: document.render_generation(),
            expansion_generation: self.expansion_generation,
            editable: fig_item.is_editable(),
        };
        let tree = (self
            .gpui_layers
            .as_ref()
            .is_some_and(|adapter| adapter.tree_key != Some(tree_key)))
        .then(|| {
            let mut tree =
                crate::gpui_adapters::layers::layers_tree(doc, page_root, &self.expanded_nodes);
            if !fig_item.is_editable() {
                crate::gpui_adapters::layers::restrict_read_only(&mut tree);
            }
            tree
        });
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
                // The panel already shows the toggle; the refresh supplies
                // (or prunes) the subtree the tree read model left out.
                self.set_node_expanded(id, *expanded);
                self.refresh_gpui_layers(cx);
                cx.notify();
            }
            LayersPanelAction::CollapseAllRequested => {
                self.clear_expanded_nodes();
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
            LayersPanelAction::PanelExpansionChanged { .. } => {}
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
        let allowed = self.active_view(cx).is_some_and(|view| {
            let item = view.read(cx).item().read(cx);
            item.document().is_some_and(|document| {
                crate::gpui_adapters::layers::context_actions(&document.doc, id).contains(&action)
            }) && (item.is_editable()
                || matches!(
                    action,
                    LayersPanelContextAction::Copy | LayersPanelContextAction::GoToMainComponent
                ))
        });
        if !allowed {
            return;
        }
        if matches!(
            action,
            LayersPanelContextAction::Copy
                | LayersPanelContextAction::Duplicate
                | LayersPanelContextAction::Delete
        ) && self.active_view(cx).is_some_and(|view| {
            view.read(cx)
                .item()
                .read(cx)
                .document()
                .is_some_and(|document| !document.doc.selection.contains(id))
        }) {
            self.select_node(id, false, cx);
        }
        match action {
            LayersPanelContextAction::Duplicate => {
                if let Some(view) = self.active_view(cx) {
                    view.update(cx, |view, cx| view.duplicate_selected_nodes(cx));
                }
            }
            LayersPanelContextAction::Delete => {
                if let Some(view) = self.active_view(cx) {
                    view.update(cx, |view, cx| view.delete_selected_nodes(cx));
                }
            }
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
            // The view decides whether the clicked row stands in for the
            // selection (it is part of it) or is the sole target.
            LayersPanelContextAction::GroupSelection => {
                if let Some(view) = self.active_view(cx) {
                    view.update(cx, |view, cx| view.group_nodes(Some(id), window, cx));
                }
            }
            LayersPanelContextAction::FrameSelection => {
                if let Some(view) = self.active_view(cx) {
                    view.update(cx, |view, cx| view.frame_nodes(Some(id), window, cx));
                }
            }
            // Removing a frame is ungrouping it: the frame dissolves and its
            // children keep their place, exactly what shift-cmd-G does.
            LayersPanelContextAction::Ungroup | LayersPanelContextAction::RemoveFrame => {
                if let Some(view) = self.active_view(cx) {
                    view.update(cx, |view, cx| view.ungroup_nodes(Some(id), window, cx));
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
}
