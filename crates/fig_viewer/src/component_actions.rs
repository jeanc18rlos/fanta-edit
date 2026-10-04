//! Working with components from the canvas: make an instance of a main
//! component or of a whole variant set, and find an instance's main
//! component.

use anyhow::{Context as _, Result, bail};
use fanta_doc::{
    CanvasNode, ComponentId, Doc, InstanceNode, NodeData, NodeId, Operation, Transform2D,
};

use crate::properties_snapshot::resolved_instance_def;

/// How far to the right of its source a new instance lands.
const INSTANCE_GAP: f64 = 40.0;

/// The component an instance of `node` uses: the variant set whose frame
/// `node` is, or the component whose master `node` is.
pub(crate) fn instantiable(doc: &Doc, node: NodeId) -> Option<ComponentId> {
    doc.components
        .sets
        .values()
        .find(|set| set.root == Some(node))
        .map(|set| set.id)
        .or_else(|| {
            doc.components
                .defs
                .values()
                .find(|def| def.root == node)
                .map(|def| def.id)
        })
}

/// The master an instance draws (for an instance of a variant set, the
/// variant it shows), when that master is in the scene.
pub(crate) fn main_component_root(doc: &Doc, instance: NodeId) -> Option<NodeId> {
    let NodeData::Instance(instance) = &doc.scene.get(instance)?.data else {
        return None;
    };
    resolved_instance_def(&doc.components, instance)
        .map(|def| def.root)
        .filter(|root| doc.scene.contains(*root))
}

/// The operations that place a new instance of the component behind `node`
/// (a master, or a variant set's frame) to its right, and the new instance.
/// An instance of a variant inside its set's frame lands beside the frame,
/// not inside it.
pub(crate) fn create_instance_operations(
    doc: &Doc,
    node: NodeId,
) -> Result<(Vec<Operation>, NodeId)> {
    let Some(component) = instantiable(doc, node) else {
        bail!("Choose a main component or a component set");
    };
    let (name, master) = match doc.components.sets.get(&component) {
        Some(set) => (
            set.name.clone(),
            doc.components
                .def(set.default_variant)
                .map(|def| def.root)
                .context("the component set has no default variant")?,
        ),
        None => {
            let def = doc.components.def(component).context("missing component")?;
            (def.name.clone(), def.root)
        }
    };
    // A variant sits in its set's frame; its instance goes beside the frame.
    let anchor = doc
        .components
        .def(component)
        .and_then(|def| def.variant_of.as_ref())
        .and_then(|membership| doc.components.sets.get(&membership.set))
        .and_then(|set| set.root)
        .filter(|frame| doc.scene.get(node).and_then(|n| n.parent) == Some(*frame))
        .unwrap_or(node);
    let bounds = doc
        .scene
        .world_bounds(anchor)
        .context("the component has no bounds")?;
    let parent = doc
        .scene
        .get(anchor)
        .and_then(|anchor| anchor.parent)
        .or_else(|| doc.active_page());
    let parent_world = parent
        .and_then(|parent| doc.scene.world_transform(parent))
        .unwrap_or(Transform2D::IDENTITY);
    let size = master_size(doc, master).unwrap_or([bounds.width(), bounds.height()]);
    let mut instance = CanvasNode::new(NodeData::Instance(InstanceNode {
        component,
        overrides: Vec::new(),
        prop_values: Default::default(),
        derived: Vec::new(),
        local_size: size,
    }));
    instance.name = name;
    instance.parent = parent;
    instance.index = doc.scene.next_child_index(parent);
    instance.transform = Transform2D::translation(bounds.max_x + INSTANCE_GAP, bounds.min_y)
        .then(&parent_world.inverse());
    let id = instance.id;
    Ok((
        vec![Operation::CreateInstance {
            node: Box::new(instance),
        }],
        id,
    ))
}

/// A master's box: its frame size, or its content bounds for a plain group.
fn master_size(doc: &Doc, root: NodeId) -> Option<[f64; 2]> {
    let node = doc.scene.get(root)?;
    if let NodeData::Group(group) = &node.data
        && let Some(size) = group.clip_size.or(group.local_size)
    {
        return Some(size);
    }
    let bounds = doc.scene.local_bounds(root)?;
    Some([bounds.width(), bounds.height()])
}

#[cfg(test)]
mod tests {
    use super::*;
    use fanta_doc::{ComponentDef, GroupNode};

    /// A page with two 100×40 masters named after their values.
    fn doc_with_masters() -> (Doc, NodeId, Vec<NodeId>) {
        let mut doc = Doc::new();
        let page = CanvasNode::new(NodeData::Group(GroupNode::default()));
        let page_id = page.id;
        doc.apply(Operation::create_node(page)).unwrap();
        doc.add_page(page_id);
        doc.set_active_page(Some(page_id));
        let mut roots = Vec::new();
        for (index, name) in ["Size=S", "Size=L"].into_iter().enumerate() {
            let mut root = CanvasNode::new(NodeData::Group(GroupNode {
                clip_size: Some([100.0, 40.0]),
                ..Default::default()
            }));
            root.name = name.into();
            root.parent = Some(page_id);
            root.index = doc.scene.next_child_index(Some(page_id));
            root.transform = Transform2D::translation(index as f64 * 150.0, 0.0);
            let root_id = root.id;
            doc.apply(Operation::create_node(root)).unwrap();
            let component = ComponentId::new();
            doc.apply(Operation::DefineComponent {
                def: Box::new(ComponentDef::new(component, root_id, name)),
            })
            .unwrap();
            roots.push(root_id);
        }
        (doc, page_id, roots)
    }

    fn instance_of(doc: &Doc, id: NodeId) -> &InstanceNode {
        match &doc.scene.get(id).unwrap().data {
            NodeData::Instance(instance) => instance,
            _ => panic!("expected an instance"),
        }
    }

    #[test]
    fn an_instance_of_a_main_component_lands_beside_it() {
        let (mut doc, page, roots) = doc_with_masters();
        let (operations, instance) = create_instance_operations(&doc, roots[0]).unwrap();
        for operation in operations {
            doc.apply(operation).unwrap();
        }
        let node = doc.scene.get(instance).unwrap();
        assert_eq!(node.parent, Some(page));
        assert_eq!(node.name, "Size=S");
        let bounds = doc.scene.world_bounds(instance).unwrap();
        assert_eq!((bounds.min_x, bounds.min_y), (140.0, 0.0));
        assert_eq!(instance_of(&doc, instance).local_size, [100.0, 40.0]);
        assert_eq!(main_component_root(&doc, instance), Some(roots[0]));
        // Only components can be instanced.
        assert!(create_instance_operations(&doc, page).is_err());
        assert!(create_instance_operations(&doc, instance).is_err());
    }

    #[test]
    fn an_instance_of_a_set_shows_its_default_and_lands_beside_the_set_frame() {
        let (mut doc, page, roots) = doc_with_masters();
        let edit = crate::variant_sets::combine_variants(&doc, &roots, Some("Chip")).unwrap();
        for operation in edit.operations {
            doc.apply(operation).unwrap();
        }
        let frame = edit.frame.unwrap();
        fanta_doc::solve_auto_layout(&mut doc.scene, frame, &mut |_| (0.0, 0.0));
        let frame_bounds = doc.scene.world_bounds(frame).unwrap();

        // From the set's frame: an instance of the whole set.
        let (operations, of_set) = create_instance_operations(&doc, frame).unwrap();
        for operation in operations {
            doc.apply(operation).unwrap();
        }
        assert_eq!(instance_of(&doc, of_set).component, edit.set);
        assert_eq!(doc.scene.get(of_set).unwrap().name, "Chip");
        assert_eq!(doc.scene.get(of_set).unwrap().parent, Some(page));
        assert_eq!(
            main_component_root(&doc, of_set),
            Some(roots[0]),
            "Go to main component lands on the variant it shows"
        );

        // From a variant inside the frame: an instance of that variant,
        // beside the frame rather than inside it.
        let (operations, of_variant) = create_instance_operations(&doc, roots[1]).unwrap();
        for operation in operations {
            doc.apply(operation).unwrap();
        }
        assert_eq!(doc.scene.get(of_variant).unwrap().parent, Some(page));
        let bounds = doc.scene.world_bounds(of_variant).unwrap();
        assert_eq!(bounds.min_x, frame_bounds.max_x + INSTANCE_GAP);
        assert_eq!(main_component_root(&doc, of_variant), Some(roots[1]));
    }
}
