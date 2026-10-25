//! Builders that turn one edit into document [`Operation`]s, shared by the
//! editor's inspector and the design-op applier so both produce the same
//! operations.

use std::collections::{BTreeSet, HashMap};

use fanta_canvas::{
    ResizeHandle, resize_box_keep_rotation, resize_transform_keep_rotation, rotate_about,
    transform_angle,
};
use fanta_doc::{
    Blur, BoundProp, Color, ComponentDef, ComponentId, Doc, ModeId, NodeData, NodeId, Operation,
    Shadow, ShadowKind, Stroke, TextAutoResize, Transform2D, VarValue, VariableId,
};
use glam::DVec2;
use smallvec::SmallVec;

pub const DEFAULT_FILL_COLOR: Color = Color::rgb(217, 217, 217);

pub fn replace_data_operation(
    doc: &Doc,
    id: NodeId,
    mutate: impl FnOnce(&mut NodeData),
) -> Vec<Operation> {
    let Some(node) = doc.scene.get(id) else {
        return Vec::new();
    };
    let old = node.data.clone();
    let mut new = old.clone();
    mutate(&mut new);
    if new == old {
        return Vec::new();
    }
    vec![Operation::ReplaceData {
        id,
        old: Box::new(old),
        new: Box::new(new),
    }]
}

pub fn effects_operations(
    doc: &Doc,
    id: NodeId,
    mutate: impl FnOnce(&mut SmallVec<[Shadow; 0]>),
) -> Vec<Operation> {
    let Some(node) = doc.scene.get(id) else {
        return Vec::new();
    };
    let old = node.effects.clone();
    let mut new = old.clone();
    mutate(&mut new);
    if new == old {
        return Vec::new();
    }
    vec![Operation::SetEffects { id, old, new }]
}

/// The blur twin of [`effects_operations`]: snapshot the node's blur stack,
/// mutate a copy, and emit one `SetBlurs` when it actually changed.
pub fn blurs_operations(
    doc: &Doc,
    id: NodeId,
    mutate: impl FnOnce(&mut SmallVec<[Blur; 0]>),
) -> Vec<Operation> {
    let Some(node) = doc.scene.get(id) else {
        return Vec::new();
    };
    let old = node.blurs.clone();
    let mut new = old.clone();
    mutate(&mut new);
    if new == old {
        return Vec::new();
    }
    vec![Operation::SetBlurs { id, old, new }]
}

/// Promote the frame or group rooted at `id` into a new component master —
/// one undoable [`Operation::DefineComponent`], named after the node. The
/// master stays where it is on the canvas (Figma behavior). Non-groups,
/// instances, and nodes that already are masters are refused.
pub fn create_component_operations(doc: &Doc, id: NodeId) -> Vec<Operation> {
    let Some(node) = doc.scene.get(id) else {
        return Vec::new();
    };
    if !matches!(node.data, NodeData::Group(_)) {
        return Vec::new();
    }
    if doc.components.defs.values().any(|def| def.root == id) {
        return Vec::new();
    }
    let name = node.name.trim();
    let name = if name.is_empty() { "Component" } else { name };
    let component = ComponentId::new();
    vec![Operation::DefineComponent {
        def: Box::new(ComponentDef::new(component, id, name)),
    }]
}

/// Change one oriented dimension of a node by pinning the opposite edge, the
/// same math the canvas resize handles use so rotation is preserved. Text
/// changes its content box and reflows; groups change their own box without
/// scaling descendants; leaf geometry resizes through its transform.
pub fn resize_operations(doc: &Doc, id: NodeId, new_size: f64, horizontal: bool) -> Vec<Operation> {
    if new_size <= 0.0 {
        return Vec::new();
    }
    let scene = &doc.scene;
    let Some(node) = scene.get(id) else {
        return Vec::new();
    };
    let Some(scene_local) = authored_local_bounds(&doc.scene, id) else {
        return Vec::new();
    };
    let Some(scene_world_transform) = scene.world_transform(id) else {
        return Vec::new();
    };
    let parent_world = node
        .parent
        .and_then(|parent| scene.world_transform(parent))
        .unwrap_or(Transform2D::IDENTITY);
    let parent_determinant = parent_world.0.matrix2.determinant();
    if !parent_determinant.is_finite() || parent_determinant.abs() <= f64::EPSILON {
        return Vec::new();
    }
    let mut normalized_children = HashMap::new();
    let (local, world_transform) = match &node.data {
        NodeData::Group(group) => match group.clip_size.or(group.local_size) {
            Some([width, height]) => (
                fanta_doc::Bounds::from_xywh(0.0, 0.0, width, height),
                scene_world_transform,
            ),
            None => {
                let origin = DVec2::new(scene_local.min_x, scene_local.min_y);
                let inverse_origin = Transform2D::translation(-origin.x, -origin.y);
                for child_id in scene.children_of(Some(id)) {
                    if let Some(child) = scene.get(*child_id) {
                        normalized_children
                            .insert(*child_id, child.transform.then(&inverse_origin));
                    }
                }
                (
                    fanta_doc::Bounds::from_xywh(
                        0.0,
                        0.0,
                        scene_local.width(),
                        scene_local.height(),
                    ),
                    Transform2D::translation(origin.x, origin.y).then(&scene_world_transform),
                )
            }
        },
        _ => (scene_local, scene_world_transform),
    };
    let theta = transform_angle(&world_transform);
    let rotation = Transform2D::rotation(theta);
    let rotation_inverse = rotation.inverse();
    let frame_bounds = local.transformed(&world_transform.then(&rotation_inverse));
    let (cursor_frame, handle) = if horizontal {
        (
            DVec2::new(frame_bounds.min_x + new_size, frame_bounds.center().y),
            ResizeHandle::East,
        )
    } else {
        (
            DVec2::new(frame_bounds.center().x, frame_bounds.min_y + new_size),
            ResizeHandle::South,
        )
    };
    let cursor_world = rotation.transform_point(cursor_frame);

    let resizes_content_box = matches!(node.data, NodeData::Text(_) | NodeData::Group(_));
    if resizes_content_box {
        let current_size = if horizontal {
            frame_bounds.width()
        } else {
            frame_bounds.height()
        };
        if (new_size - current_size).abs() <= f64::EPSILON {
            return Vec::new();
        }

        let (new_world, width, height) =
            resize_box_keep_rotation(world_transform, local, handle, cursor_world, false, false);
        let new_local = new_world.then(&parent_world.inverse());
        if !new_local
            .to_components()
            .iter()
            .all(|component| component.is_finite())
        {
            return Vec::new();
        }

        let mut new_data = node.data.clone();
        match &mut new_data {
            NodeData::Text(text) => {
                text.local_size = [width, height];
                text.auto_resize = if horizontal {
                    TextAutoResize::Height
                } else {
                    TextAutoResize::None
                };
            }
            NodeData::Group(group) => {
                if group.clip_size.is_some() {
                    group.clip_size = Some([width, height]);
                } else {
                    group.local_size = Some([width, height]);
                }
            }
            _ => return Vec::new(),
        }

        let mut operations = Vec::with_capacity(2 + normalized_children.len());
        if new_local != node.transform {
            operations.push(Operation::SetTransform {
                id,
                old: node.transform,
                new: new_local,
            });
        }
        if new_data != node.data {
            operations.push(Operation::ReplaceData {
                id,
                old: Box::new(node.data.clone()),
                new: Box::new(new_data),
            });
        }
        if let NodeData::Group(group) = &node.data {
            let old_size = group
                .clip_size
                .or(group.local_size)
                .unwrap_or([local.width(), local.height()]);
            let new_box_size = [width, height];
            for child_id in scene.children_of(Some(id)) {
                let Some(child) = scene.get(*child_id) else {
                    continue;
                };
                let base_transform = normalized_children
                    .get(child_id)
                    .copied()
                    .unwrap_or(child.transform);
                let new_child_transform = if group.auto_layout.is_none() {
                    let mut normalized_child = child.clone();
                    normalized_child.transform = base_transform;
                    normalized_child
                        .apply_constraints(old_size, new_box_size)
                        .unwrap_or(base_transform)
                } else {
                    base_transform
                };
                if new_child_transform != child.transform {
                    operations.push(Operation::SetTransform {
                        id: *child_id,
                        old: child.transform,
                        new: new_child_transform,
                    });
                }
            }
        }
        return operations;
    }

    let new_world =
        resize_transform_keep_rotation(world_transform, local, handle, cursor_world, false, false);
    let new_local = new_world.then(&parent_world.inverse());
    vec![Operation::SetTransform {
        id,
        old: node.transform,
        new: new_local,
    }]
}

pub fn rotation_operations(doc: &Doc, id: NodeId, degrees: f64) -> Vec<Operation> {
    let scene = &doc.scene;
    let Some(node) = scene.get(id) else {
        return Vec::new();
    };
    let Some(bounds) = scene.world_bounds(id) else {
        return Vec::new();
    };
    let Some(world_transform) = scene.world_transform(id) else {
        return Vec::new();
    };
    let parent_world = node
        .parent
        .and_then(|parent| scene.world_transform(parent))
        .unwrap_or(Transform2D::IDENTITY);
    let determinant = parent_world.0.matrix2.determinant();
    if !determinant.is_finite() || determinant.abs() <= f64::EPSILON {
        return Vec::new();
    }
    let pivot = authored_local_bounds(&doc.scene, id)
        .map(|local| world_transform.transform_point(local.center()))
        .unwrap_or_else(|| bounds.center());
    let current = transform_angle(&world_transform);
    let delta = degrees.to_radians() - current;
    if delta.abs() < 1e-9 {
        return Vec::new();
    }
    let new_world = world_transform.then(&rotate_about(pivot, delta));
    let new = new_world.then(&parent_world.inverse());
    vec![Operation::SetTransform {
        id,
        old: node.transform,
        new,
    }]
}

pub fn set_corner_radius(data: &mut NodeData, radius: f64) {
    let (corner_radius, corner_radii) = match data {
        NodeData::Vector(vector) => (&mut vector.corner_radius, &mut vector.corner_radii),
        NodeData::Group(group) => (&mut group.corner_radius, &mut group.corner_radii),
        _ => return,
    };
    *corner_radius = (radius > 0.0).then_some(radius);
    *corner_radii = None;
}

pub fn stroke_list_mut(data: &mut NodeData) -> Option<&mut SmallVec<[Stroke; 1]>> {
    data.strokes_mut()
}

pub fn default_shadow() -> Shadow {
    Shadow {
        kind: ShadowKind::Drop,
        color: Color::rgba(0, 0, 0, 64),
        blur: 4.0,
        spread: 0.0,
        offset: [0.0, 2.0],
        // Figma's default: the shadow is knocked out under the node's body.
        show_behind_node: false,
    }
}

pub fn parse_color(text: &str) -> Option<Color> {
    let trimmed = text.trim();
    if let Some(color) = Color::from_hex(trimmed) {
        return Some(color);
    }
    Color::from_hex(&format!("#{trimmed}"))
}

pub fn authored_local_bounds(scene: &fanta_doc::Scene, id: NodeId) -> Option<fanta_doc::Bounds> {
    let node = scene.get(id)?;
    match &node.data {
        fanta_doc::NodeData::Group(group) => group
            .clip_size
            .or(group.local_size)
            .map(|[width, height]| fanta_doc::Bounds::from_xywh(0.0, 0.0, width, height))
            .or_else(|| scene.local_bounds(id)),
        fanta_doc::NodeData::TextPath(text_path) => {
            fanta_render::text_path_visual_bounds(text_path).or_else(|| scene.local_bounds(id))
        }
        _ => scene.local_bounds(id),
    }
}

pub fn variable_binding_operation(
    doc: &Doc,
    node_id: NodeId,
    prop: BoundProp,
    variable: Option<VariableId>,
) -> Result<Option<Operation>, &'static str> {
    match variable {
        Some(variable) => bind_property_operation(doc, node_id, prop, variable),
        None => unbind_property_operation(doc, node_id, prop),
    }
}

fn bind_property_operation(
    doc: &Doc,
    node_id: NodeId,
    prop: BoundProp,
    variable_id: VariableId,
) -> Result<Option<Operation>, &'static str> {
    let node = doc
        .scene
        .get(node_id)
        .ok_or("The selected layer no longer exists")?;
    if !prop.applies_to(node) {
        return Err("The property does not apply to the selected layer");
    }
    let variable = doc
        .variables
        .variables
        .get(&variable_id)
        .ok_or("The variable no longer exists")?;
    if !doc.variables.collections.contains_key(&variable.collection) {
        return Err("The variable's collection no longer exists");
    }
    if !prop.accepts_variable_type(variable.ty) {
        return Err("The variable type is not compatible with this property");
    }
    let old = node.bindings.get(&prop).copied();
    if old == Some(variable_id) {
        return Ok(None);
    }
    Ok(Some(Operation::BindProperty {
        node: node_id,
        prop,
        old,
        new: variable_id,
    }))
}

pub fn unbind_property_operation(
    doc: &Doc,
    node_id: NodeId,
    prop: BoundProp,
) -> Result<Option<Operation>, &'static str> {
    let node = doc
        .scene
        .get(node_id)
        .ok_or("The selected layer no longer exists")?;
    let Some(variable) = node.bindings.get(&prop).copied() else {
        return Ok(None);
    };
    let old_data = node.data.clone();
    let old_opacity = node.opacity;
    let old_flags = node.flags;
    let mut baked = node.clone();
    if let Some(value) = fanta_doc::resolve_bound_value(
        &doc.variables,
        &doc.scene,
        node_id,
        &doc.active_modes,
        variable,
    ) {
        prop.apply_resolved(&mut baked, value);
    }
    Ok(Some(Operation::UnbindProperty {
        node: node_id,
        prop,
        variable,
        old_data: Box::new(old_data),
        new_data: Box::new(baked.data),
        old_opacity,
        new_opacity: baked.opacity,
        old_flags,
        new_flags: baked.flags,
    }))
}

pub fn set_variable_value_operation(
    doc: &Doc,
    variable_id: VariableId,
    mode_id: ModeId,
    new_value: VarValue,
) -> Result<Option<Operation>, &'static str> {
    let variable = doc
        .variables
        .variables
        .get(&variable_id)
        .ok_or("The variable no longer exists")?;
    let collection = doc
        .variables
        .collections
        .get(&variable.collection)
        .ok_or("The variable's collection no longer exists")?;
    if !collection.has_mode(mode_id) {
        return Err("The mode no longer exists");
    }
    if let VarValue::Alias { variable: target } = &new_value {
        validate_alias(doc, variable_id, *target)?;
    } else if new_value.variable_type() != Some(variable.ty) {
        return Err("The value does not match the variable type");
    }
    let old = variable.values_by_mode.get(&mode_id).cloned();
    if old.as_ref() == Some(&new_value) {
        return Ok(None);
    }
    Ok(Some(Operation::SetVariableValue {
        variable: variable_id,
        mode: mode_id,
        old,
        new: Some(new_value),
    }))
}

pub fn validate_alias(
    doc: &Doc,
    source: VariableId,
    target: VariableId,
) -> Result<(), &'static str> {
    let source_type = doc
        .variables
        .variables
        .get(&source)
        .ok_or("The variable no longer exists")?
        .ty;
    let mut pending = vec![target];
    let mut visited = BTreeSet::new();
    while let Some(id) = pending.pop() {
        if id == source {
            return Err("Variable aliases cannot form a cycle");
        }
        if !visited.insert(id) {
            continue;
        }
        let variable = doc
            .variables
            .variables
            .get(&id)
            .ok_or("The alias target no longer exists")?;
        if variable.ty != source_type {
            return Err("The alias must have the same variable type");
        }
        pending.extend(
            variable
                .values_by_mode
                .values()
                .filter_map(|value| match value {
                    VarValue::Alias { variable } => Some(*variable),
                    _ => None,
                }),
        );
    }
    Ok(())
}
