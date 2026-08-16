//! LayersPanel adapter: builds the layer tree read model for the active
//! page and maps the panel's intents onto the existing selection / flag /
//! rename / drop paths. The panel is virtualized, so the read model is built
//! for whole pages (30k+ nodes) and memoized by the host on (page root,
//! render generation) — see `FantaDesignPanel::refresh_gpui_layers`.

use std::collections::HashSet;

use fanta_doc::{CanvasNode, Doc, NodeData, NodeId, ParametricShape, PathData, PathSegment};
use fanta_gpui::layers::{
    LayersPanel, LayersPanelDropPosition, LayersPanelItem, LayersPanelNodeKind,
};
use gpui::{Entity, SharedString, Subscription};

use crate::design_panel::LayerDropPlacement;

pub(crate) struct LayersAdapter {
    pub panel: Entity<LayersPanel>,
    /// The (page root, render generation) the panel's tree was last built
    /// from; `None` forces a rebuild on the next refresh.
    pub tree_key: Option<(Option<NodeId>, u64)>,
    pub _subscription: Subscription,
}

/// The engine facts the kind mapping needs beyond a node's own data.
pub(crate) struct KindContext {
    /// Roots of component masters (`doc.components.defs`).
    pub component_roots: HashSet<NodeId>,
    /// Frames that hold a variant set: the parent of every set member's
    /// master root. The engine has no set node of its own, so the set is the
    /// frame the importer wraps its variants in.
    pub component_set_frames: HashSet<NodeId>,
}

impl KindContext {
    pub(crate) fn from_doc(doc: &Doc) -> Self {
        let component_roots: HashSet<NodeId> =
            doc.components.defs.values().map(|def| def.root).collect();
        let component_set_frames: HashSet<NodeId> = doc
            .components
            .sets
            .values()
            .flat_map(|set| set.members.iter())
            .filter_map(|member| doc.components.defs.get(member))
            .filter_map(|def| doc.scene.get(def.root))
            .filter_map(|node| node.parent)
            .collect();
        Self {
            component_roots,
            component_set_frames,
        }
    }
}

/// The fold from engine node data to the panel's kinds. Groups split into
/// component set / component / frame / group; masks win over their shape;
/// vectors recover Rectangle / Ellipse / Polygon / Star / Line where the
/// engine kept the provenance (`PathData::is_rect`, `VectorNode::parametric`,
/// a two-point open path); everything else stays at the family kind.
pub(crate) fn layers_kind(
    id: NodeId,
    node: &CanvasNode,
    context: &KindContext,
) -> LayersPanelNodeKind {
    if node.is_mask {
        return LayersPanelNodeKind::Mask;
    }
    match &node.data {
        NodeData::Group(group) => {
            if context.component_set_frames.contains(&id) {
                LayersPanelNodeKind::ComponentSet
            } else if context.component_roots.contains(&id) {
                LayersPanelNodeKind::Component
            } else if group.is_frame_surface() {
                LayersPanelNodeKind::Frame
            } else {
                LayersPanelNodeKind::Group
            }
        }
        NodeData::Vector(vector) => match vector.parametric {
            Some(ParametricShape::Arc { .. }) => LayersPanelNodeKind::Ellipse,
            Some(ParametricShape::Star { .. }) => LayersPanelNodeKind::Star,
            Some(ParametricShape::Polygon { .. }) => LayersPanelNodeKind::Polygon,
            None => {
                if vector.path.is_rect() {
                    LayersPanelNodeKind::Rectangle
                } else if is_two_point_open_path(&vector.path) {
                    LayersPanelNodeKind::Line
                } else {
                    LayersPanelNodeKind::Vector
                }
            }
        },
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

/// A single open subpath with exactly two anchors — what the line tool draws.
fn is_two_point_open_path(path: &PathData) -> bool {
    matches!(
        path.segments.as_slice(),
        [PathSegment::Move { .. }, PathSegment::Line { .. }]
    )
}

/// Builds the panel tree for one page. `page_root` `None` is a document
/// without page roots, whose scene roots are the layers — the same fallback
/// the canvas paints. Children appear in the same visual order as Figma's
/// panel: topmost paint order first, i.e. the scene's child order reversed.
pub(crate) fn layers_tree(doc: &Doc, page_root: Option<NodeId>) -> Vec<LayersPanelItem> {
    let context = KindContext::from_doc(doc);
    build_children(doc, page_root, &context)
}

fn build_children(
    doc: &Doc,
    parent: Option<NodeId>,
    context: &KindContext,
) -> Vec<LayersPanelItem> {
    let children = match parent {
        Some(parent) => doc.scene.children_of(Some(parent)),
        None => doc.scene.roots(),
    };
    children
        .iter()
        .rev()
        .filter_map(|child| {
            let node = doc.scene.get(*child)?;
            let kind = layers_kind(*child, node, context);
            Some(LayersPanelItem {
                id: SharedString::from(child.to_string()),
                title: SharedString::from(node.name.clone()),
                kind,
                children: build_children(doc, Some(*child), context),
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

/// The panel's drop placement in the host's vocabulary: `Before` is visually
/// above the target row (later in paint order), `After` below it.
pub(crate) fn drop_placement(position: LayersPanelDropPosition) -> LayerDropPlacement {
    match position {
        LayersPanelDropPosition::Before => LayerDropPlacement::Above,
        LayersPanelDropPosition::Inside => LayerDropPlacement::Inside,
        LayersPanelDropPosition::After => LayerDropPlacement::Below,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fanta_doc::{
        Color, ComponentDef, ComponentId, ComponentSet, ComponentSetMembership, GroupNode,
        VectorNode,
    };

    fn insert(doc: &mut Doc, mut node: CanvasNode, parent: Option<NodeId>) -> NodeId {
        node.parent = parent;
        node.index = doc.scene.next_child_index(parent);
        let id = node.id;
        doc.scene.insert(node).expect("insert node");
        id
    }

    fn rect() -> CanvasNode {
        CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
            0.0,
            0.0,
            10.0,
            10.0,
            Color::BLACK,
        )))
    }

    fn group() -> CanvasNode {
        CanvasNode::new(NodeData::Group(GroupNode::default()))
    }

    #[test]
    fn tree_lists_topmost_first_and_falls_back_to_scene_roots_without_a_page() {
        let mut doc = Doc::new();
        let page = insert(&mut doc, group(), None);
        let mut bottom = rect();
        bottom.name = "Bottom".into();
        let bottom = insert(&mut doc, bottom, Some(page));
        let mut top = rect();
        top.name = "Top".into();
        let top = insert(&mut doc, top, Some(page));

        let tree = layers_tree(&doc, Some(page));
        assert_eq!(
            tree.iter()
                .map(|item| item.title.as_ref())
                .collect::<Vec<_>>(),
            vec!["Top", "Bottom"]
        );
        assert_eq!(node_id(&tree[0].id), Some(top));
        assert_eq!(node_id(&tree[1].id), Some(bottom));

        // No page root: the scene roots are the layers.
        let rootless = layers_tree(&doc, None);
        assert_eq!(rootless.len(), 1);
        assert_eq!(node_id(&rootless[0].id), Some(page));
        assert_eq!(rootless[0].children.len(), 2);
    }

    #[test]
    fn kinds_recover_shape_provenance_masks_and_component_sets() {
        let mut doc = Doc::new();
        let page = insert(&mut doc, group(), None);
        let rectangle = insert(&mut doc, rect(), Some(page));

        let mut line = CanvasNode::new(NodeData::Vector(VectorNode::default()));
        if let NodeData::Vector(vector) = &mut line.data {
            vector.path.move_to(0.0, 0.0).line_to(10.0, 10.0);
        }
        let line = insert(&mut doc, line, Some(page));

        let mut mask = rect();
        mask.is_mask = true;
        let mask = insert(&mut doc, mask, Some(page));

        let master = insert(&mut doc, group(), Some(page));
        let component = ComponentId::new();
        doc.components
            .defs
            .insert(component, ComponentDef::new(component, master, "Button"));

        let set_frame = insert(&mut doc, group(), Some(page));
        let variant_root = insert(&mut doc, group(), Some(set_frame));
        let variant = ComponentId::new();
        let set = ComponentId::new();
        let mut variant_def = ComponentDef::new(variant, variant_root, "Size=Small");
        variant_def.variant_of = Some(ComponentSetMembership {
            set,
            axis_values: Default::default(),
        });
        doc.components.defs.insert(variant, variant_def);
        doc.components.sets.insert(
            set,
            ComponentSet {
                id: set,
                name: "Chip".into(),
                axes: Vec::new(),
                members: vec![variant],
                default_variant: variant,
            },
        );

        let context = KindContext::from_doc(&doc);
        let kind = |id: NodeId| layers_kind(id, doc.scene.get(id).unwrap(), &context);
        assert_eq!(kind(rectangle), LayersPanelNodeKind::Rectangle);
        assert_eq!(kind(line), LayersPanelNodeKind::Line);
        assert_eq!(kind(mask), LayersPanelNodeKind::Mask);
        assert_eq!(kind(master), LayersPanelNodeKind::Component);
        assert_eq!(kind(set_frame), LayersPanelNodeKind::ComponentSet);
        assert_eq!(kind(variant_root), LayersPanelNodeKind::Component);
        assert_eq!(kind(page), LayersPanelNodeKind::Group);
    }
}
