//! Shared fixtures for the instance-expansion suite — only the cross-cutting
//! ones (used by more than one section). Section-specific builders
//! (`master_with_vector`, `variant_set`, `two_axis_set`, `nested_masters`)
//! live next to the tests that use them. Reached via `use super::*;`
//! (re-exported by the `instance_tests` module root).

use super::*;

/// Build a master in `scene`: a frame root with a text child. Returns
/// (component id in a fresh library, root id, child id).
pub(crate) fn master(scene: &mut Scene) -> (ComponentLibrary, ComponentId, NodeId, NodeId) {
    let mut root = CanvasNode::new(NodeData::Group(GroupNode {
        clip_size: Some([100.0, 40.0]),
        background: None,
        explicit_modes: Default::default(),
        ..Default::default()
    }));
    root.name = "Button".into();
    let root_id = root.id;
    scene.insert(root).unwrap();

    let mut label = CanvasNode::new(NodeData::Text(TextNode::new("Label", 80.0, 20.0)));
    label.parent = Some(root_id);
    let label_id = label.id;
    scene.insert(label).unwrap();

    let comp_id = ComponentId::new();
    let mut lib = ComponentLibrary::new();
    lib.defs
        .insert(comp_id, ComponentDef::new(comp_id, root_id, "Button"));
    (lib, comp_id, root_id, label_id)
}
