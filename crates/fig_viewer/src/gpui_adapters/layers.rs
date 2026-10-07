//! LayersPanel adapter: builds the layer tree read model for the active
//! page and maps the panel's intents onto the existing selection / flag /
//! rename / drop paths. The read model carries only the rows the panel can
//! show — the page's top level plus the children of expanded containers —
//! so an edit on a 30k-node page costs O(expanded rows), not O(page). The
//! host memoizes it on [`LayersTreeKey`] — see
//! `FantaDesignPanel::refresh_gpui_layers`.

use std::collections::HashSet;

use fanta_doc::{CanvasNode, Doc, NodeData, NodeId, ParametricShape, PathData, PathSegment};
use fanta_gpui::layers::{
    LayersPanel, LayersPanelDropPosition, LayersPanelItem, LayersPanelNodeKind,
};
use gpui::{Entity, SharedString, Subscription};

use crate::design_panel::LayerDropPlacement;

pub(crate) struct LayersAdapter {
    pub panel: Entity<LayersPanel>,
    /// What the panel's tree was last built from; `None` forces a rebuild on
    /// the next refresh.
    pub tree_key: Option<LayersTreeKey>,
    pub _subscription: Subscription,
}

/// Everything `layers_tree` reads that can change its output. The host
/// rebuilds the panel's tree only when this differs from the last build.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct LayersTreeKey {
    pub page_root: Option<NodeId>,
    pub render_generation: u64,
    /// The host's expansion counter: the tree only holds the children of
    /// expanded containers, so a change in the expanded set changes the tree
    /// even when the document did not.
    pub expansion_generation: u64,
    pub editable: bool,
}

/// The engine facts the kind mapping needs beyond a node's own data.
pub(crate) struct KindContext {
    /// Roots of component masters (`doc.components.defs`).
    pub component_roots: HashSet<NodeId>,
    /// Frames that hold a variant set: the set's `root`, or, for a set
    /// without one, the frame (never a page) its members' masters sit in.
    pub component_set_frames: HashSet<NodeId>,
}

impl KindContext {
    pub(crate) fn from_doc(doc: &Doc) -> Self {
        let component_roots: HashSet<NodeId> =
            doc.components.defs.values().map(|def| def.root).collect();
        let pages = doc.pages();
        let component_set_frames: HashSet<NodeId> = doc
            .components
            .sets
            .values()
            .flat_map(|set| match set.root {
                Some(root) => vec![root],
                None => set
                    .members
                    .iter()
                    .filter_map(|member| doc.components.defs.get(member))
                    .filter_map(|def| doc.scene.get(def.root))
                    .filter_map(|node| node.parent)
                    .filter(|parent| !pages.contains(parent))
                    .collect(),
            })
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
            } else if crate::layer_context_ops::is_section(node) {
                LayersPanelNodeKind::Section
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
        NodeData::Text(_) | NodeData::TextPath(_) => LayersPanelNodeKind::Text,
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
///
/// Only the children of `expanded` containers are built; a collapsed
/// container is a single item flagged `has_children`, so the panel still
/// draws its disclosure arrow and asks the host (`ExpansionChanged`) for the
/// subtree when it is opened.
pub(crate) fn layers_tree(
    doc: &Doc,
    page_root: Option<NodeId>,
    expanded: &HashSet<NodeId>,
) -> Vec<LayersPanelItem> {
    let context = KindContext::from_doc(doc);
    if let Some(root) = page_root
        && context.component_roots.contains(&root)
    {
        return build_node(doc, root, expanded, &context)
            .into_iter()
            .collect();
    }
    build_children(doc, page_root, expanded, &context)
}

fn build_node(
    doc: &Doc,
    id: NodeId,
    expanded: &HashSet<NodeId>,
    context: &KindContext,
) -> Option<LayersPanelItem> {
    let node = doc.scene.get(id)?;
    let kind = layers_kind(id, node, context);
    let has_children = !doc.scene.children_of(Some(id)).is_empty();
    let children = if has_children && expanded.contains(&id) {
        build_children(doc, Some(id), expanded, context)
    } else {
        Vec::new()
    };
    Some(LayersPanelItem {
        id: SharedString::from(id.to_string()),
        title: SharedString::from(node.name.clone()),
        kind,
        children,
        has_children,
        visible: !node.flags.contains(fanta_doc::NodeFlags::HIDDEN),
        locked: node.flags.contains(fanta_doc::NodeFlags::LOCKED),
        context_actions: Some(context_actions_with_kind(doc, id, kind, context)),
    })
}

fn build_children(
    doc: &Doc,
    parent: Option<NodeId>,
    expanded: &HashSet<NodeId>,
    context: &KindContext,
) -> Vec<LayersPanelItem> {
    let children = match parent {
        Some(parent) => doc.scene.children_of(Some(parent)),
        None => doc.scene.roots(),
    };
    children
        .iter()
        .rev()
        .filter_map(|child| build_node(doc, *child, expanded, context))
        .collect()
}

pub(crate) fn context_actions(
    doc: &Doc,
    id: NodeId,
) -> Vec<fanta_gpui::layers::LayersPanelContextAction> {
    let Some(node) = doc.scene.get(id) else {
        return Vec::new();
    };
    let context = KindContext::from_doc(doc);
    context_actions_with_kind(doc, id, layers_kind(id, node, &context), &context)
}

fn context_actions_with_kind(
    doc: &Doc,
    id: NodeId,
    kind: LayersPanelNodeKind,
    context: &KindContext,
) -> Vec<fanta_gpui::layers::LayersPanelContextAction> {
    use fanta_gpui::layers::LayersPanelContextAction as Action;
    let Some(node) = doc.scene.get(id) else {
        return Vec::new();
    };
    let inherited_lock = doc
        .scene
        .ancestors_of(id)
        .any(|node| node.flags.contains(fanta_doc::NodeFlags::LOCKED));
    let locked = node.flags.contains(fanta_doc::NodeFlags::LOCKED);
    let master = context.component_roots.contains(&id);
    let has_main_component = crate::component_actions::main_component_root(doc, id).is_some();
    let mut actions = fanta_gpui::layers::context_actions_for_kind(kind);
    actions.retain(|action| {
        if matches!(
            action,
            Action::GoToMainComponent | Action::DetachInstance | Action::ResetInstance
        ) && !has_main_component
        {
            return false;
        }
        if inherited_lock {
            return matches!(
                action,
                Action::Copy | Action::CopyPasteAs | Action::GoToMainComponent
            );
        }
        if locked {
            return matches!(
                action,
                Action::Copy | Action::CopyPasteAs | Action::LockUnlock | Action::GoToMainComponent
            );
        }
        match action {
            Action::Flatten => crate::layer_context_ops::can_flatten_with_component_roots(
                doc,
                id,
                &context.component_roots,
            ),
            Action::OutlineStroke => match &node.data {
                NodeData::Text(_) => crate::layer_context_ops::can_flatten_with_component_roots(
                    doc,
                    id,
                    &context.component_roots,
                ),
                NodeData::Vector(value) => !value.strokes.is_empty(),
                NodeData::Group(value) => !value.strokes.is_empty(),
                NodeData::Boolean(value) => !value.strokes.is_empty(),
                _ => false,
            },
            Action::Ungroup | Action::RemoveFrame => {
                !master && matches!(node.data, NodeData::Group(_))
            }
            Action::MoveToPage => doc.pages().len() > 1,
            Action::CreateComponent => !master && !matches!(kind, LayersPanelNodeKind::Other),
            _ => true,
        }
    });
    actions
}

pub(crate) fn restrict_read_only(items: &mut [LayersPanelItem]) {
    use fanta_gpui::layers::LayersPanelContextAction as Action;
    for item in items {
        if let Some(actions) = &mut item.context_actions {
            actions.retain(|action| {
                matches!(
                    action,
                    Action::Copy | Action::CopyPasteAs | Action::GoToMainComponent
                )
            });
        }
        restrict_read_only(&mut item.children);
    }
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

        let tree = layers_tree(&doc, Some(page), &HashSet::new());
        assert_eq!(
            tree.iter()
                .map(|item| item.title.as_ref())
                .collect::<Vec<_>>(),
            vec!["Top", "Bottom"]
        );
        assert_eq!(node_id(&tree[0].id), Some(top));
        assert_eq!(node_id(&tree[1].id), Some(bottom));
        assert!(tree.iter().all(|item| !item.has_children));

        // No page root: the scene roots are the layers. The page is a
        // collapsed container until it is expanded.
        let rootless = layers_tree(&doc, None, &HashSet::new());
        assert_eq!(rootless.len(), 1);
        assert_eq!(node_id(&rootless[0].id), Some(page));
        assert!(rootless[0].has_children);
        assert!(rootless[0].children.is_empty());
        let rootless = layers_tree(&doc, None, &HashSet::from([page]));
        assert_eq!(rootless[0].children.len(), 2);
    }

    #[test]
    fn children_are_built_only_for_expanded_containers() {
        let mut doc = Doc::new();
        let page = insert(&mut doc, group(), None);
        let mut frame = group();
        frame.name = "Frame".into();
        let frame = insert(&mut doc, frame, Some(page));
        let mut inner = group();
        inner.name = "Inner".into();
        let inner = insert(&mut doc, inner, Some(frame));
        let mut leaf = rect();
        leaf.name = "Leaf".into();
        let leaf = insert(&mut doc, leaf, Some(inner));
        let mut empty = group();
        empty.name = "Empty".into();
        let empty = insert(&mut doc, empty, Some(page));

        // Nothing expanded: the page's top level only, containers flagged.
        let tree = layers_tree(&doc, Some(page), &HashSet::new());
        assert_eq!(
            tree.iter()
                .map(|item| (item.title.as_ref(), item.has_children, item.children.len()))
                .collect::<Vec<_>>(),
            vec![("Empty", false, 0), ("Frame", true, 0)]
        );
        assert_eq!(node_id(&tree[1].id), Some(frame));
        assert_eq!(node_id(&tree[0].id), Some(empty));

        // Expanding the frame builds its children, but not the collapsed
        // inner group's.
        let tree = layers_tree(&doc, Some(page), &HashSet::from([frame]));
        let frame_item = &tree[1];
        assert_eq!(frame_item.children.len(), 1);
        let inner_item = &frame_item.children[0];
        assert_eq!(node_id(&inner_item.id), Some(inner));
        assert!(inner_item.has_children);
        assert!(inner_item.children.is_empty());

        // An expanded node under a collapsed ancestor stays pruned with it.
        let tree = layers_tree(&doc, Some(page), &HashSet::from([inner]));
        assert!(tree[1].children.is_empty());

        // Expanding the whole chain reaches the leaf.
        let tree = layers_tree(&doc, Some(page), &HashSet::from([frame, inner]));
        let leaf_item = &tree[1].children[0].children[0];
        assert_eq!(node_id(&leaf_item.id), Some(leaf));
        assert!(!leaf_item.has_children);
    }

    #[test]
    fn a_component_canvas_keeps_its_editable_master_and_children_in_the_tree() {
        let mut doc = Doc::new();
        let page = insert(&mut doc, group(), None);
        doc.add_page(page);
        let master = insert(&mut doc, group(), Some(page));
        let child = insert(&mut doc, rect(), Some(master));
        let component = ComponentId::new();
        doc.components
            .defs
            .insert(component, ComponentDef::new(component, master, "Card"));

        let tree = layers_tree(&doc, Some(master), &HashSet::new());
        let root = tree.first().expect("editable master row");
        assert_eq!(tree.len(), 1);
        assert_eq!(node_id(&root.id), Some(master));
        assert_eq!(root.kind, LayersPanelNodeKind::Component);
        assert!(root.has_children);
        assert!(root.children.is_empty());

        let tree = layers_tree(&doc, Some(master), &HashSet::from([master]));
        let children = &tree.first().expect("expanded master row").children;
        assert_eq!(children.len(), 1);
        assert_eq!(
            node_id(&children.first().expect("child row").id),
            Some(child)
        );

        let tree = layers_tree(&doc, Some(page), &HashSet::new());
        assert_eq!(
            node_id(&tree.first().expect("page child row").id),
            Some(master)
        );
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
                root: None,
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
    #[test]
    fn layer_menu_actions_follow_node_kind_and_current_capabilities() {
        use fanta_gpui::layers::LayersPanelContextAction as Action;
        let mut doc = Doc::new();
        let frame = insert(
            &mut doc,
            CanvasNode::new(NodeData::Group(GroupNode {
                clip_size: Some([100., 100.]),
                ..Default::default()
            })),
            None,
        );
        let text = insert(
            &mut doc,
            CanvasNode::new(NodeData::Text(fanta_doc::TextNode::new("Hello", 100., 30.))),
            Some(frame),
        );
        let rectangle = insert(&mut doc, rect(), Some(frame));
        let image = insert(
            &mut doc,
            CanvasNode::new(NodeData::Bitmap(fanta_doc::BitmapNode {
                asset: fanta_doc::AssetId::new(),
                natural_size: [100, 100],
                local_size: [100., 100.],
                crop: None,
                tint: None,
                fit: fanta_doc::ImageFitMode::Fill,
            })),
            Some(frame),
        );
        assert!(context_actions(&doc, text).contains(&Action::EditText));
        assert!(!context_actions(&doc, rectangle).contains(&Action::EditText));
        assert!(context_actions(&doc, image).contains(&Action::CropImage));
        assert!(context_actions(&doc, image).contains(&Action::ReplaceMedia));
        assert!(!context_actions(&doc, text).contains(&Action::CropImage));
        assert!(context_actions(&doc, frame).contains(&Action::ConvertToSection));
        assert!(!context_actions(&doc, rectangle).contains(&Action::OutlineStroke));
        if let NodeData::Vector(vector) = &mut doc.scene.get_mut(rectangle).expect("rectangle").data
        {
            vector
                .strokes
                .push(fanta_doc::Stroke::solid(Color::BLACK, 2.));
        }
        assert!(context_actions(&doc, rectangle).contains(&Action::OutlineStroke));
        doc.scene.get_mut(frame).expect("frame").meta =
            serde_json::json!({"fanta_kind": "section"});
        assert!(context_actions(&doc, frame).contains(&Action::ConvertToFrame));
        assert!(!context_actions(&doc, frame).contains(&Action::ConvertToSection));
        for id in [frame, text, rectangle, image] {
            assert!(!context_actions(&doc, id).contains(&Action::SendToFigmaMake));
        }
    }

    #[test]
    fn variant_instance_actions_intersect_resolution_lock_and_read_only_policy() {
        use fanta_gpui::layers::LayersPanelContextAction as Action;
        for invalid in [
            None,
            Some("missing component"),
            Some("missing root"),
            Some("empty set"),
        ] {
            for lock in [None, Some("own"), Some("ancestor")] {
                let mut fixture = crate::component_actions::tests::set_instance_fixture();
                match invalid {
                    Some("missing component") => {
                        fixture.doc.components.defs.clear();
                    }
                    Some("missing root") => {
                        fixture
                            .doc
                            .components
                            .defs
                            .get_mut(&fixture.members[1])
                            .expect("default")
                            .root = NodeId::new();
                    }
                    Some("empty set") => {
                        let set = fixture
                            .doc
                            .components
                            .sets
                            .get_mut(&fixture.set)
                            .expect("set");
                        set.members.clear();
                        set.default_variant = fanta_doc::ComponentId::new();
                    }
                    _ => {}
                }
                if let Some(lock) = lock {
                    fixture
                        .doc
                        .scene
                        .get_mut(if lock == "own" {
                            fixture.instance
                        } else {
                            fixture.parent
                        })
                        .expect("locked node")
                        .flags
                        .insert(fanta_doc::NodeFlags::LOCKED);
                }
                let actions = context_actions(&fixture.doc, fixture.instance);
                assert_eq!(
                    actions.contains(&Action::GoToMainComponent),
                    invalid.is_none(),
                    "navigation invalid={invalid:?}, lock={lock:?}"
                );
                for action in [Action::DetachInstance, Action::ResetInstance] {
                    assert_eq!(
                        actions.contains(&action),
                        invalid.is_none() && lock.is_none(),
                        "{action:?}, invalid={invalid:?}, lock={lock:?}"
                    );
                }
                let mut tree = layers_tree(
                    &fixture.doc,
                    fixture.doc.active_page(),
                    &HashSet::from([fixture.parent]),
                );
                restrict_read_only(&mut tree);
                let instance = tree
                    .iter()
                    .find(|row| row.id.as_ref() == fixture.parent.to_string())
                    .expect("parent row")
                    .children
                    .iter()
                    .find(|row| row.id.as_ref() == fixture.instance.to_string())
                    .expect("instance row");
                let actions = instance.context_actions.as_ref().expect("actions");
                assert_eq!(
                    actions.contains(&Action::GoToMainComponent),
                    invalid.is_none()
                );
                assert!(!actions.contains(&Action::DetachInstance));
                assert!(!actions.contains(&Action::ResetInstance));
            }
        }
        let mut fixture = crate::component_actions::tests::set_instance_fixture();
        let NodeData::Instance(instance) = &mut fixture
            .doc
            .scene
            .get_mut(fixture.instance)
            .expect("instance")
            .data
        else {
            panic!("instance")
        };
        instance.component = fixture.members[0];
        for action in [
            Action::GoToMainComponent,
            Action::DetachInstance,
            Action::ResetInstance,
        ] {
            assert!(
                context_actions(&fixture.doc, fixture.instance).contains(&action),
                "direct master control {action:?}"
            );
        }
    }

    #[test]
    fn shared_component_index_preserves_nested_master_action_eligibility() {
        use fanta_gpui::layers::LayersPanelContextAction as Action;

        let mut doc = Doc::new();
        let page = insert(&mut doc, group(), None);
        doc.add_page(page);
        let ordinary = insert(&mut doc, group(), Some(page));
        insert(&mut doc, rect(), Some(ordinary));
        let container = insert(&mut doc, group(), Some(page));
        let master = insert(&mut doc, group(), Some(container));
        insert(&mut doc, rect(), Some(master));
        let vector_master = insert(&mut doc, rect(), Some(page));
        for root in [master, vector_master] {
            let component = ComponentId::new();
            doc.components.defs.insert(
                component,
                ComponentDef::new(component, root, "Protected component"),
            );
        }
        let context = KindContext::from_doc(&doc);
        for (id, allowed) in [
            (ordinary, true),
            (container, false),
            (master, false),
            (vector_master, false),
        ] {
            assert_eq!(crate::layer_context_ops::can_flatten(&doc, id), allowed);
            assert_eq!(
                crate::layer_context_ops::can_flatten_with_component_roots(
                    &doc,
                    id,
                    &context.component_roots
                ),
                allowed
            );
            assert_eq!(
                context_actions(&doc, id).contains(&Action::Flatten),
                allowed
            );
        }
        let tree = layers_tree(
            &doc,
            Some(page),
            &HashSet::from([ordinary, container, master]),
        );
        let mut pending: Vec<_> = tree.iter().collect();
        while let Some(row) = pending.pop() {
            let id = node_id(&row.id).expect("node id");
            assert_eq!(
                row.context_actions.as_ref().expect("projected actions"),
                &context_actions(&doc, id)
            );
            pending.extend(&row.children);
        }
        assert!(context_actions(&doc, ordinary).contains(&Action::Ungroup));
        assert!(!context_actions(&doc, master).contains(&Action::Ungroup));
    }

    #[test]
    #[ignore = "bounded CPU projection benchmark; run explicitly with --ignored --nocapture"]
    fn component_heavy_layer_actions_bench() {
        use sha2::{Digest as _, Sha256};
        use std::time::Instant;

        let mut doc = Doc::new();
        let page = insert(&mut doc, group(), None);
        doc.add_page(page);
        let component_page = insert(&mut doc, group(), None);
        doc.add_page(component_page);
        for _ in 0..2000 {
            insert(&mut doc, rect(), Some(page));
            let root = insert(&mut doc, group(), Some(component_page));
            let component = ComponentId::new();
            doc.components.defs.insert(
                component,
                ComponentDef::new(component, root, "Benchmark component"),
            );
        }
        let expanded = HashSet::new();
        for _ in 0..3 {
            std::hint::black_box(layers_tree(&doc, Some(page), &expanded));
        }
        let expected_actions: Vec<_> = layers_tree(&doc, Some(page), &expanded)
            .into_iter()
            .map(|row| row.context_actions)
            .collect();
        assert_eq!(expected_actions.len(), 2000);
        let action_signature = format!("{:x}", Sha256::digest(format!("{expected_actions:?}")));
        let mut milliseconds = Vec::new();
        for _ in 0..11 {
            let started = Instant::now();
            let rows = layers_tree(&doc, Some(page), &expanded);
            milliseconds.push(started.elapsed().as_secs_f64() * 1000.0);
            assert_eq!(
                rows.into_iter()
                    .map(|row| row.context_actions)
                    .collect::<Vec<_>>(),
                expected_actions
            );
        }
        milliseconds.sort_by(f64::total_cmp);
        println!(
            "{}",
            serde_json::json!({
                "measurement": "layer-tree projection CPU microbenchmark; not native or Spectrum drag latency",
                "debug_assertions": cfg!(debug_assertions),
                "displayed_vectors": 2000,
                "component_definitions": 2000,
                "warmup_projections": 3,
                "measured_projections": milliseconds.len(),
                "median_ms": milliseconds.get(5).expect("median sample"),
                "p95_ms": milliseconds.last().expect("p95 sample"),
                "max_ms": milliseconds.last().expect("max sample"),
                "action_signature_sha256": action_signature,
            })
        );
    }

    #[test]
    fn layer_menu_read_only_and_inherited_locks_only_offer_safe_commands() {
        use fanta_gpui::layers::LayersPanelContextAction as Action;
        let mut doc = Doc::new();
        let parent = insert(&mut doc, group(), None);
        let child = insert(&mut doc, rect(), Some(parent));
        doc.scene
            .get_mut(parent)
            .expect("parent")
            .flags
            .insert(fanta_doc::NodeFlags::LOCKED);
        let actions = context_actions(&doc, child);
        assert_eq!(actions, vec![Action::Copy, Action::CopyPasteAs]);
        let parent_actions = context_actions(&doc, parent);
        assert!(parent_actions.contains(&Action::LockUnlock));
        let mut tree = layers_tree(&doc, None, &HashSet::from([parent]));
        restrict_read_only(&mut tree);
        assert!(
            tree.iter()
                .flat_map(|item| item.context_actions.as_ref().expect("policy"))
                .all(|action| matches!(
                    action,
                    Action::Copy | Action::CopyPasteAs | Action::GoToMainComponent
                ))
        );
    }
}
