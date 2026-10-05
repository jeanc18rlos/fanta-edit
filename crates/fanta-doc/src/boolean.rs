use crate::{BooleanNode, CanvasNode, Color, NodeData, NodeFlags, NodeId, Scene, VectorNode};
use serde_json::{Value, json};

#[derive(Debug, thiserror::Error)]
pub enum BooleanGeometryError {
    #[error("The Boolean geometry contains a missing node")]
    MissingNode,
    #[error("The Boolean operand {0} cannot be recomputed")]
    Unsupported(&'static str),
    #[error("The Boolean geometry is too deeply nested")]
    TooDeep,
    #[error("The Boolean geometry contains a non-finite transform")]
    NonFinite,
    #[error("The Boolean geometry could not be serialized: {0}")]
    Serialization(#[from] serde_json::Error),
}

pub fn boolean_geometry_signature(
    scene: &Scene,
    root: NodeId,
) -> Result<String, BooleanGeometryError> {
    boolean_geometry_signature_with(
        root,
        |id| scene.get(id),
        |id| scene.children_of(Some(id)).to_vec(),
    )
}

pub fn boolean_geometry_signature_with<'a>(
    root: NodeId,
    node: impl Fn(NodeId) -> Option<&'a CanvasNode>,
    children: impl Fn(NodeId) -> Vec<NodeId>,
) -> Result<String, BooleanGeometryError> {
    let root_node = node(root).ok_or(BooleanGeometryError::MissingNode)?;
    let NodeData::Boolean(boolean) = &root_node.data else {
        return Err(BooleanGeometryError::Unsupported("non-Boolean root"));
    };
    let operands = children(root)
        .into_iter()
        .map(|id| operand_signature(id, &node, &children, 0))
        .collect::<Result<Vec<_>, _>>()?;
    // Store canonical content rather than process-local stamps: a saved project
    // and an instance clone must validate the same geometry after IDs change.
    let mut signature = json!({
        "version": 1,
        "operation": boolean.op,
        "stroke_geometry": stroke_outline_geometry(boolean),
        "operands": operands,
    });
    canonicalize_saved_geometry(&mut signature);
    Ok(serde_json::to_string(&signature)?)
}

fn stroke_outline_geometry(boolean: &BooleanNode) -> Option<Value> {
    if !boolean
        .baked
        .as_ref()
        .is_some_and(|baked| baked.stroke_outline)
    {
        return None;
    }
    let mut strokes = boolean.strokes.clone();
    for stroke in &mut strokes {
        stroke.paint = crate::Fill::solid(Color::BLACK);
    }
    Some(json!({ "strokes": strokes, "has_fills": !boolean.fills.is_empty() }))
}

fn canonicalize_saved_geometry(value: &mut Value) {
    match value {
        Value::Number(number) if number.is_f64() => {
            // FNX writes float magnitudes below 1e-9 as zero. The signature
            // must survive that normalization without changing live geometry.
            if number.as_f64().is_some_and(|number| number.abs() < 1e-9) {
                *value = json!(0.0);
            }
        }
        Value::Array(values) => values.iter_mut().for_each(canonicalize_saved_geometry),
        Value::Object(values) => {
            // serde_json's preserve_order feature is unified differently in
            // the app and standalone tools; persisted signatures must agree.
            values.sort_keys();
            values.values_mut().for_each(canonicalize_saved_geometry);
        }
        _ => {}
    }
}

fn operand_signature<'a>(
    id: NodeId,
    node: &impl Fn(NodeId) -> Option<&'a CanvasNode>,
    children: &impl Fn(NodeId) -> Vec<NodeId>,
    depth: usize,
) -> Result<Value, BooleanGeometryError> {
    if depth > 128 {
        return Err(BooleanGeometryError::TooDeep);
    }
    let operand = node(id).ok_or(BooleanGeometryError::MissingNode)?;
    if !operand.transform.is_finite() {
        return Err(BooleanGeometryError::NonFinite);
    }
    let geometry = match &operand.data {
        NodeData::Vector(vector) => vector_geometry(vector),
        NodeData::Boolean(boolean) => json!({
            "kind": "boolean",
            "operation": boolean.op,
            "baked": boolean.baked.as_ref().map(|baked| vector_geometry(&baked.vector)),
            "stroke_geometry": stroke_outline_geometry(boolean),
        }),
        NodeData::Group(group) => json!({
            "kind": "group",
            "clip_size": group.clip_size,
            "local_size": group.local_size,
            "clip_content": operand.meta.get("clip_content"),
            "corner_radius": group.corner_radius,
            "corner_radii": group.corner_radii,
            "corner_smoothing": group.corner_smoothing,
            "background": group.background.is_some() || !group.background_fills.is_empty(),
        }),
        NodeData::Text(text) => serde_json::to_value(text)?,
        NodeData::TextPath(text) => serde_json::to_value(text)?,
        data => return Err(BooleanGeometryError::Unsupported(data.kind_tag())),
    };
    let descendants = children(id)
        .into_iter()
        .map(|child| operand_signature(child, node, children, depth + 1))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(json!({
        "geometry": geometry,
        "transform": operand.transform,
        "hidden": operand.flags.contains(NodeFlags::HIDDEN),
        "unclipped": operand.flags.contains(NodeFlags::UNCLIPPED_VECTOR),
        "children": descendants,
    }))
}

fn vector_geometry(vector: &VectorNode) -> Value {
    json!({
        "kind": "vector",
        "path": vector.path,
        "corner_radius": vector.corner_radius,
        "corner_radii": vector.corner_radii,
        "corner_smoothing": vector.corner_smoothing,
        "local_size": vector.local_size,
    })
}

impl BooleanNode {
    pub fn baked_vector(&self, signature: &str) -> Option<VectorNode> {
        let baked = self
            .baked
            .as_ref()
            .filter(|baked| baked.source == signature)?;
        let mut vector = baked.vector.clone();
        if baked.stroke_outline {
            if !self.fills.is_empty() || self.strokes.len() != 1 {
                return None;
            }
            vector.fills.clear();
            vector.fills.push(self.strokes.first()?.paint.clone());
            vector.strokes.clear();
        } else {
            vector.fills = self.fills.clone();
            vector.strokes = self.strokes.clone();
        }
        Some(vector)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        BooleanBakedGeometry, BooleanOp, BoundProp, ComponentDef, ComponentId, Doc, Fill,
        InstanceNode, Operation, ResolvedVarValue, Transform2D,
    };

    #[test]
    fn boolean_signature_survives_clone_ids_and_roundtrip_but_rejects_operand_edits() {
        let mut doc = Doc::new();
        let root = CanvasNode::new(NodeData::Boolean(BooleanNode {
            fills: [Fill::solid(Color::BLACK)].into_iter().collect(),
            ..Default::default()
        }));
        let root_id = root.id;
        doc.apply(Operation::create_node(root)).expect("root");
        let mut child = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
            0.,
            0.,
            20.,
            20.,
            Color::WHITE,
        )));
        child.parent = Some(root_id);
        let child_id = child.id;
        doc.apply(Operation::create_node(child)).expect("child");
        let source = boolean_geometry_signature(&doc.scene, root_id).expect("signature");
        doc.scene
            .get_mut(root_id)
            .expect("root")
            .data
            .as_boolean_mut()
            .expect("Boolean")
            .baked = Some(BooleanBakedGeometry {
            vector: VectorNode::rect_solid(0., 0., 20., 20., Color::BLACK),
            source: source.clone(),
            stroke_outline: false,
        });
        let component = ComponentId::new();
        doc.components
            .defs
            .insert(component, ComponentDef::new(component, root_id, "Boolean"));
        let expanded = crate::expand_instance(
            &doc.scene,
            &doc.components,
            &InstanceNode {
                component,
                overrides: Vec::new(),
                prop_values: Default::default(),
                derived: Vec::new(),
                local_size: [20., 20.],
            },
        );
        let cloned_root = expanded
            .iter()
            .find(|entry| entry.def_path.is_empty())
            .expect("clone")
            .node
            .id;
        let cloned_signature = boolean_geometry_signature_with(
            cloned_root,
            |id| {
                expanded
                    .iter()
                    .find(|entry| entry.node.id == id)
                    .map(|entry| &entry.node)
            },
            |id| {
                expanded
                    .iter()
                    .filter(|entry| entry.node.parent == Some(id))
                    .map(|entry| entry.node.id)
                    .collect()
            },
        )
        .expect("clone signature");
        assert_eq!(source, cloned_signature);
        let encoded = serde_json::to_vec(&doc.scene).expect("encode");
        let mut restored: Scene = serde_json::from_slice(&encoded).expect("decode");
        restored.rebuild_child_index();
        assert_eq!(
            source,
            boolean_geometry_signature(&restored, root_id).expect("restored signature")
        );
        doc.scene.get_mut(child_id).expect("child").name = "Renamed".into();
        assert_eq!(
            source,
            boolean_geometry_signature(&doc.scene, root_id).expect("rename signature")
        );
        doc.scene
            .set_transform(root_id, Transform2D::translation(100., 200.))
            .expect("move root");
        assert_eq!(
            source,
            boolean_geometry_signature(&doc.scene, root_id).expect("root placement")
        );
        doc.scene
            .set_transform(child_id, Transform2D::translation(1., 0.))
            .expect("move operand");
        assert_ne!(
            source,
            boolean_geometry_signature(&doc.scene, root_id).expect("operand edit")
        );
        doc.scene
            .set_transform(child_id, Transform2D::IDENTITY)
            .expect("restore operand");
        doc.scene
            .get_mut(root_id)
            .expect("root")
            .data
            .as_boolean_mut()
            .expect("Boolean")
            .op = BooleanOp::Subtract;
        assert_ne!(
            source,
            boolean_geometry_signature(&doc.scene, root_id).expect("operation edit")
        );
    }

    #[test]
    fn boolean_fill_binding_is_applicable_and_updates_baked_paint_without_staling_geometry() {
        let mut scene = Scene::new();
        let node = CanvasNode::new(NodeData::Boolean(BooleanNode {
            fills: [Fill::solid(Color::BLACK)].into_iter().collect(),
            ..Default::default()
        }));
        let id = scene.insert(node).expect("Boolean");
        let source = boolean_geometry_signature(&scene, id).expect("signature");
        let node = scene.get_mut(id).expect("Boolean");
        node.data.as_boolean_mut().expect("Boolean").baked = Some(BooleanBakedGeometry {
            vector: VectorNode::rect_solid(0., 0., 20., 20., Color::BLACK),
            source: source.clone(),
            stroke_outline: false,
        });
        let prop = BoundProp::FillColor { index: 0 };
        assert!(prop.applies_to(node));
        assert!(prop.apply_resolved(
            node,
            ResolvedVarValue::Color {
                value: Color::WHITE
            }
        ));
        let vector = node
            .data
            .as_boolean()
            .expect("Boolean")
            .baked_vector(&source)
            .expect("paint override keeps valid geometry");
        assert_eq!(vector.fills.first(), Some(&Fill::solid(Color::WHITE)));
        assert_eq!(
            source,
            boolean_geometry_signature(&scene, id).expect("paint signature")
        );
        assert!(!BoundProp::FillColor { index: 1 }.applies_to(scene.get(id).expect("Boolean")));
    }

    #[test]
    fn boolean_bake_survives_typed_document_json_roundtrip() {
        let mut doc = Doc::new();
        let root = CanvasNode::new(NodeData::Boolean(BooleanNode {
            fills: [Fill::solid(Color::BLACK)].into_iter().collect(),
            baked: Some(BooleanBakedGeometry {
                vector: VectorNode::rect_solid(0., 0., 20., 20., Color::BLACK),
                source: String::new(),
                stroke_outline: false,
            }),
            ..Default::default()
        }));
        let root_id = root.id;
        doc.apply(Operation::create_node(root)).expect("root");
        // This imported Spectrum coordinate changes by one ULP with the
        // default serde_json float parser, staling the preserved appearance.
        let mut operand = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
            0.22952167611888466,
            0.,
            20.,
            20.,
            Color::BLACK,
        )));
        operand.parent = Some(root_id);
        let operand_id = operand.id;
        doc.apply(Operation::create_node(operand)).expect("operand");
        let source = boolean_geometry_signature(&doc.scene, root_id).expect("signature");
        doc.scene
            .get_mut(root_id)
            .expect("root")
            .data
            .as_boolean_mut()
            .expect("Boolean")
            .baked
            .as_mut()
            .expect("bake")
            .source = source.clone();

        for encoded in [
            doc.to_json_string().expect("compact JSON"),
            doc.to_json_pretty().expect("pretty JSON"),
        ] {
            let restored = Doc::from_json_str(&encoded).expect("load document");
            let signature =
                boolean_geometry_signature(&restored.scene, root_id).expect("restored signature");
            assert_eq!(
                signature, source,
                "saving and reopening JSON must keep the imported bake active"
            );
            let boolean = restored
                .scene
                .get(root_id)
                .expect("root")
                .data
                .as_boolean()
                .expect("Boolean");
            assert!(boolean.baked_vector(&signature).is_some());
            assert_eq!(
                restored.scene.get(operand_id).expect("operand").data,
                doc.scene.get(operand_id).expect("original operand").data
            );
        }
    }
}

#[cfg(test)]
mod nested_tests {
    use super::*;
    use crate::{BooleanBakedGeometry, Fill, Stroke};
    #[test]
    fn boolean_parent_signature_tracks_nested_stroke_outline_geometry() {
        let mut scene = Scene::new();
        let outer = scene
            .insert(CanvasNode::new(NodeData::Boolean(BooleanNode::default())))
            .expect("outer");
        let mut inner = CanvasNode::new(NodeData::Boolean(BooleanNode {
            strokes: [Stroke::solid(Color::BLACK, 2.)].into_iter().collect(),
            baked: Some(BooleanBakedGeometry {
                vector: VectorNode::rect_solid(0., 0., 12., 12., Color::BLACK),
                source: String::new(),
                stroke_outline: true,
            }),
            ..Default::default()
        }));
        inner.parent = Some(outer);
        let inner = scene.insert(inner).expect("inner");
        let mut operand = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
            0.,
            0.,
            10.,
            10.,
            Color::BLACK,
        )));
        operand.parent = Some(inner);
        scene.insert(operand).expect("operand");
        let source = boolean_geometry_signature(&scene, inner).expect("inner signature");
        scene
            .get_mut(inner)
            .expect("inner")
            .data
            .as_boolean_mut()
            .expect("Boolean")
            .baked
            .as_mut()
            .expect("bake")
            .source = source;
        let source = boolean_geometry_signature(&scene, outer).expect("outer signature");
        scene
            .get_mut(inner)
            .expect("inner")
            .data
            .as_boolean_mut()
            .expect("Boolean")
            .strokes
            .first_mut()
            .expect("stroke")
            .paint = Fill::solid(Color::WHITE);
        assert_eq!(
            source,
            boolean_geometry_signature(&scene, outer).expect("recolor")
        );
        scene
            .get_mut(inner)
            .expect("inner")
            .data
            .as_boolean_mut()
            .expect("Boolean")
            .strokes
            .first_mut()
            .expect("stroke")
            .width = 4.;
        assert_ne!(
            source,
            boolean_geometry_signature(&scene, outer).expect("width edit")
        );
    }

    #[test]
    fn boolean_signature_matches_fnx_float_normalization_boundary() {
        let mut scene = Scene::new();
        let root = scene
            .insert(CanvasNode::new(NodeData::Boolean(BooleanNode::default())))
            .expect("root");
        let mut operand = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
            0.,
            0.,
            20.,
            20.,
            Color::BLACK,
        )));
        operand.parent = Some(root);
        let operand = scene.insert(operand).expect("operand");
        let baseline = boolean_geometry_signature(&scene, root).expect("baseline");
        for coordinate in [1.692105385930204e-13, -1.692105385930204e-13, -0.0] {
            scene
                .set_transform(operand, crate::Transform2D::translation(coordinate, 0.))
                .expect("transform");
            assert_eq!(
                baseline,
                boolean_geometry_signature(&scene, root).expect("saved precision")
            );
        }
        for coordinate in [1e-9, -1e-9, 0.01] {
            scene
                .set_transform(operand, crate::Transform2D::translation(coordinate, 0.))
                .expect("transform");
            assert_ne!(
                baseline,
                boolean_geometry_signature(&scene, root).expect("authored edit")
            );
        }
    }

    #[test]
    fn boolean_signature_key_order_is_independent_of_serde_features() {
        let mut geometry: Value =
            serde_json::from_str(r#"{"z":[{"y":1,"b":2}],"a":{"z":3,"a":4}}"#).expect("geometry");
        canonicalize_saved_geometry(&mut geometry);
        assert_eq!(
            serde_json::to_string(&geometry).expect("signature"),
            r#"{"a":{"a":4,"z":3},"z":[{"b":2,"y":1}]}"#
        );
    }
}
