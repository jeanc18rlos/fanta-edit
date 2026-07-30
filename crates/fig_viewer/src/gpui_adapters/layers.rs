//! LayersPanel adapter: builds the layer tree read model for the active
//! page, maps intents onto the existing selection/flag/rename/drop paths,
//! and enforces the node-budget guard (G5) — above the budget the native
//! virtualized section renders instead, because the component tree is not
//! virtualized yet.

use std::collections::HashSet;

use fanta_doc::{Doc, NodeData, NodeId};
use fanta_gpui::layers::{LayersPanel, LayersPanelItem, LayersPanelNodeKind};
use gpui::{Entity, SharedString, Subscription};

pub(crate) struct LayersAdapter {
    pub panel: Entity<LayersPanel>,
    pub _subscription: Subscription,
}

/// Default node budget: the biggest tree the non-virtualized panel renders
/// smoothly. Tunable via `FANTA_GPUI_LAYERS_BUDGET`; `FANTA_GPUI_LAYERS=0`
/// forces the native section outright.
pub(crate) fn node_budget() -> usize {
    std::env::var("FANTA_GPUI_LAYERS_BUDGET")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(2_500)
}

pub(crate) fn layers_enabled() -> bool {
    !matches!(
        std::env::var("FANTA_GPUI_LAYERS").as_deref(),
        Ok("0") | Ok("false") | Ok("off")
    )
}

/// Counts the subtree under `root` without building anything, so the budget
/// check stays cheap on the files that need it most.
pub(crate) fn subtree_len(doc: &Doc, root: NodeId) -> usize {
    doc.scene.descendants_of(root).count()
}

/// The 22-way fold from engine node data (12 variants) to the panel's
/// kinds. Vector provenance recovers Rectangle via `PathData::is_rect`;
/// everything the engine can't distinguish stays at the family kind.
/// (Component sets have no scene root of their own in the engine, so set
/// membership folds to Component.)
pub(crate) fn layers_kind(
    id: NodeId,
    data: &NodeData,
    component_roots: &HashSet<NodeId>,
) -> LayersPanelNodeKind {
    match data {
        NodeData::Group(group) => {
            if component_roots.contains(&id) {
                LayersPanelNodeKind::Component
            } else if group.is_frame_surface() {
                LayersPanelNodeKind::Frame
            } else {
                LayersPanelNodeKind::Group
            }
        }
        NodeData::Vector(vector) => {
            if vector.path.is_rect() {
                LayersPanelNodeKind::Rectangle
            } else {
                LayersPanelNodeKind::Vector
            }
        }
        NodeData::Text(_) => LayersPanelNodeKind::Text,
        NodeData::Bitmap(_) => LayersPanelNodeKind::Image,
        NodeData::Video(_) => LayersPanelNodeKind::Video,
        NodeData::Instance(_) => LayersPanelNodeKind::Instance,
        NodeData::Boolean(_) => LayersPanelNodeKind::BooleanOperation,
        NodeData::Audio(_)
        | NodeData::NodeGraph(_)
        | NodeData::Model3d(_)
        | NodeData::AiArtifact(_)
        | NodeData::Embed(_) => LayersPanelNodeKind::Other,
    }
}

/// Builds the panel tree for one page root. Children appear in the same
/// visual order as the native section: topmost paint order first, i.e. the
/// scene's child order reversed.
pub(crate) fn layers_tree(doc: &Doc, page_root: NodeId) -> Vec<LayersPanelItem> {
    let component_roots: HashSet<NodeId> =
        doc.components.defs.values().map(|def| def.root).collect();
    build_children(doc, page_root, &component_roots)
}

fn build_children(
    doc: &Doc,
    parent: NodeId,
    component_roots: &HashSet<NodeId>,
) -> Vec<LayersPanelItem> {
    doc.scene
        .children_of(Some(parent))
        .iter()
        .rev()
        .filter_map(|child| {
            let node = doc.scene.get(*child)?;
            let kind = layers_kind(*child, &node.data, component_roots);
            Some(LayersPanelItem {
                id: SharedString::from(child.to_string()),
                title: SharedString::from(node.name.clone()),
                kind,
                children: build_children(doc, *child, component_roots),
                visible: !node.flags.contains(fanta_doc::NodeFlags::HIDDEN),
                locked: node.flags.contains(fanta_doc::NodeFlags::LOCKED),
            })
        })
        .collect()
}

/// Parses a panel row id back to the engine id. Row ids are always node
/// ULIDs (the adapter minted them), so a parse failure means a stale row.
pub(crate) fn node_id(id: &SharedString) -> Option<NodeId> {
    id.parse().ok()
}
