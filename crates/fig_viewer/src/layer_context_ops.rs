//! Pure, reversible document commands used by the shared Layers menu.
use anyhow::{Context as _, Result, bail, ensure};
use fanta_doc::{
    AutoLayout, Bounds, CanvasNode, Doc, Fill, LayoutMode, NodeData, NodeId, Operation, PathData,
    Transform2D, VectorNode,
};
use fanta_gpui::layers::LayersPanelContextAction as Action;
use smallvec::SmallVec;
use std::collections::HashSet;

pub(crate) fn replace_data(node: &CanvasNode, data: NodeData) -> Operation {
    Operation::ReplaceData {
        id: node.id,
        old: Box::new(node.data.clone()),
        new: Box::new(data),
    }
}

pub(crate) fn targets(doc: &Doc, id: NodeId) -> Vec<NodeId> {
    if doc.selection.contains(id) {
        crate::clipboard::editable_selection_roots(doc)
    } else {
        vec![id]
    }
}

pub(crate) fn editable(doc: &Doc, id: NodeId) -> bool {
    doc.scene
        .get(id)
        .is_some_and(|node| !node.flags.contains(fanta_doc::NodeFlags::LOCKED))
        && !doc.pages().contains(&id)
        && !doc
            .scene
            .ancestors_of(id)
            .any(|node| node.flags.contains(fanta_doc::NodeFlags::LOCKED))
}

pub(crate) fn is_section(node: &CanvasNode) -> bool {
    match node
        .meta
        .get("fanta_kind")
        .and_then(serde_json::Value::as_str)
    {
        Some(kind) => kind == "section",
        None => {
            node.meta
                .get("figma_type")
                .and_then(serde_json::Value::as_str)
                == Some("SECTION")
        }
    }
}

pub(crate) fn simple(doc: &Doc, id: NodeId, action: Action) -> Result<Vec<Operation>> {
    ensure!(editable(doc, id), "The layer is locked or is a page");
    let node = doc.scene.get(id).context("The layer no longer exists")?;
    if matches!(
        action,
        Action::FlipHorizontal
            | Action::FlipVertical
            | Action::CreateComponent
            | Action::AddAutoLayout
            | Action::Flatten
            | Action::OutlineStroke
    ) {
        for target in targets(doc, id) {
            ensure!(editable(doc, target), "A selected layer is locked");
        }
    }
    match action {
        Action::UseAsMask => Ok(vec![Operation::SetMask {
            id,
            old: node.is_mask,
            new: !node.is_mask,
        }]),
        Action::FlipHorizontal | Action::FlipVertical => {
            let targets = targets(doc, id);
            let bounds = targets
                .iter()
                .filter_map(|id| doc.scene.world_bounds(*id))
                .reduce(|a, b| a.union(&b))
                .context("The selection has no bounds")?;
            let center = bounds.center();
            targets
                .into_iter()
                .map(|id| {
                    ensure!(editable(doc, id), "A selected layer is locked");
                    let node = doc.scene.get(id).context("Missing selected layer")?;
                    let horizontal = action == Action::FlipHorizontal;
                    let reflection = Transform2D::translation(-center.x, -center.y)
                        .then(&Transform2D::scale_xy(
                            if horizontal { -1. } else { 1. },
                            if horizontal { 1. } else { -1. },
                        ))
                        .then(&Transform2D::translation(center.x, center.y));
                    let parent = node
                        .parent
                        .and_then(|id| doc.scene.world_transform(id))
                        .unwrap_or(Transform2D::IDENTITY);
                    let world = doc
                        .scene
                        .world_transform(id)
                        .context("Missing world transform")?;
                    let new = world.then(&reflection).then(&parent.inverse());
                    ensure!(new.is_finite(), "The parent transform is singular");
                    Ok(Operation::SetTransform {
                        id,
                        old: node.transform,
                        new,
                    })
                })
                .collect()
        }
        Action::ConvertToFrame | Action::ConvertToSection => {
            ensure!(
                !doc.is_component_root(id),
                "A component cannot be converted into a section"
            );
            let NodeData::Group(group) = &node.data else {
                bail!("Choose a group or frame");
            };
            let mut group = group.clone();
            let mut operations = Vec::new();
            if group.clip_size.is_none() {
                let bounds = doc
                    .scene
                    .local_bounds(id)
                    .unwrap_or(Bounds::from_xywh(0., 0., 100., 100.));
                group.local_size = Some([bounds.width(), bounds.height()]);
                if bounds.min_x != 0. || bounds.min_y != 0. {
                    operations.push(Operation::SetTransform {
                        id,
                        old: node.transform,
                        new: Transform2D::translation(bounds.min_x, bounds.min_y)
                            .then(&node.transform),
                    });
                    for child in doc.scene.children_of(Some(id)) {
                        let child = doc.scene.get(*child).context("Missing child")?;
                        operations.push(Operation::SetTransform {
                            id: child.id,
                            old: child.transform,
                            new: child
                                .transform
                                .then(&Transform2D::translation(-bounds.min_x, -bounds.min_y)),
                        });
                    }
                }
                if action == Action::ConvertToFrame {
                    group.clip_size = group.local_size;
                }
            }
            let mut meta = node.meta.clone();
            if !meta.is_object() {
                meta = serde_json::json!({});
            }
            if action == Action::ConvertToSection {
                meta["fanta_kind"] = serde_json::json!("section");
                group.local_size = group.clip_size.or(group.local_size);
                group.clip_size = None;
                group.auto_layout = None;
            } else if let Some(meta) = meta.as_object_mut() {
                meta.insert("fanta_kind".into(), serde_json::json!("frame"));
                meta.remove("clip_content");
            }
            operations.extend([
                replace_data(node, NodeData::Group(group)),
                Operation::SetMeta {
                    id,
                    old: node.meta.clone(),
                    new: meta,
                },
            ]);
            Ok(operations)
        }
        Action::ResetInstance => {
            let NodeData::Instance(instance) = &node.data else {
                bail!("Choose a component instance");
            };
            let mut instance = instance.clone();
            instance.overrides.clear();
            instance.prop_values.clear();
            instance.derived.clear();
            if let Some(master) = doc
                .components
                .def(instance.component)
                .and_then(|def| doc.scene.get(def.root))
                .and_then(|node| node.data.local_bounds())
            {
                instance.local_size = [master.width(), master.height()];
            }
            Ok(vec![replace_data(node, NodeData::Instance(instance))])
        }
        Action::AddAutoLayout => auto_layout(doc, id, Some(LayoutMode::Horizontal)),
        Action::CreateComponent => {
            if matches!(node.data, NodeData::Group(_)) {
                return Ok(crate::properties_ops::create_component_operations(doc, id));
            }
            let grouped = crate::structure::frame_selection_operations(
                doc,
                &targets(doc, id),
                Some(&node.name),
            )?;
            let mut scratch = doc.clone();
            for operation in &grouped.operations {
                scratch.apply(operation.clone())?;
            }
            let mut operations = grouped.operations;
            operations.extend(crate::properties_ops::create_component_operations(
                &scratch,
                grouped.group,
            ));
            Ok(operations)
        }
        Action::Flatten => {
            let targets = targets(doc, id);
            if targets.len() <= 1 {
                return flatten(doc, id);
            }
            let grouped = crate::structure::group_operations(doc, &targets, None)?;
            let mut scratch = doc.clone();
            for operation in &grouped.operations {
                scratch.apply(operation.clone())?;
            }
            let mut operations = grouped.operations;
            operations.extend(flatten(&scratch, grouped.group)?);
            Ok(operations)
        }
        Action::OutlineStroke => {
            let mut scratch = doc.clone();
            let mut operations = Vec::new();
            for target in targets(doc, id) {
                let edits = outline_strokes(&scratch, target)?;
                for operation in &edits {
                    scratch.apply(operation.clone())?;
                }
                operations.extend(edits);
            }
            Ok(operations)
        }
        _ => bail!("This command requires a picker or an editor interaction"),
    }
}

pub(crate) fn auto_layout(
    doc: &Doc,
    id: NodeId,
    mode: Option<LayoutMode>,
) -> Result<Vec<Operation>> {
    ensure!(editable(doc, id), "The layer is locked");
    let node = doc.scene.get(id).context("Missing layer")?;
    for target in targets(doc, id) {
        ensure!(editable(doc, target), "A selected layer is locked");
    }
    if let NodeData::Group(group) = &node.data {
        let mut group = group.clone();
        if group.local_size.is_none() && group.clip_size.is_none() {
            group.local_size = doc
                .scene
                .local_bounds(id)
                .map(|bounds| [bounds.width(), bounds.height()]);
        }
        group.auto_layout = mode.map(|mode| AutoLayout {
            mode,
            ..group.auto_layout.unwrap_or_default()
        });
        return Ok(vec![replace_data(node, NodeData::Group(group))]);
    }
    ensure!(
        mode.is_some(),
        "Only a layout frame can have its layout removed"
    );
    let grouped = crate::structure::frame_selection_operations(doc, &targets(doc, id), None)?;
    let mut scratch = doc.clone();
    for operation in &grouped.operations {
        scratch.apply(operation.clone())?;
    }
    let mut operations = grouped.operations;
    operations.extend(auto_layout(&scratch, grouped.group, mode)?);
    Ok(operations)
}

pub(crate) fn move_to_page(doc: &Doc, id: NodeId, page: NodeId) -> Result<Vec<Operation>> {
    ensure!(doc.pages().contains(&page), "The destination page is gone");
    let mut scratch = doc.clone();
    let mut operations = Vec::new();
    for id in targets(doc, id) {
        ensure!(editable(&scratch, id), "A selected layer is locked");
        let node = scratch.scene.get(id).context("Missing layer")?;
        ensure!(
            !scratch.scene.ancestors_of(page).any(|node| node.id == id),
            "Cannot move into a descendant"
        );
        let parent = scratch
            .scene
            .world_transform(page)
            .context("Missing page transform")?;
        let new = scratch
            .scene
            .world_transform(id)
            .context("Missing layer transform")?
            .then(&parent.inverse());
        ensure!(
            new.is_finite(),
            "The destination page transform is singular"
        );
        let edits = vec![
            Operation::Reparent {
                id,
                old_parent: node.parent,
                old_index: node.index,
                new_parent: Some(page),
                new_index: scratch.scene.next_child_index(Some(page)),
            },
            Operation::SetTransform {
                id,
                old: node.transform,
                new,
            },
        ];
        for operation in &edits {
            scratch.apply(operation.clone())?;
        }
        operations.extend(edits);
    }
    Ok(operations)
}

pub(crate) fn crop(doc: &Doc, id: NodeId, aspect: Option<f64>) -> Result<Vec<Operation>> {
    ensure!(editable(doc, id), "The layer is locked");
    let node = doc.scene.get(id).context("Missing image")?;
    let NodeData::Bitmap(bitmap) = &node.data else {
        bail!("Choose an image layer");
    };
    let mut bitmap = bitmap.clone();
    if let Some(aspect) = aspect {
        ensure!(
            aspect.is_finite() && aspect > 0.,
            "Crop ratio must be positive"
        );
    }
    bitmap.crop = aspect.map(|aspect| {
        let source = bitmap.natural_size[0] as f64 / bitmap.natural_size[1].max(1) as f64;
        if source > aspect {
            let width = (aspect / source) as f32;
            [(1. - width) / 2., 0., width, 1.]
        } else {
            let height = (source / aspect) as f32;
            [0., (1. - height) / 2., 1., height]
        }
    });
    let aspect = aspect
        .unwrap_or(bitmap.natural_size[0].max(1) as f64 / bitmap.natural_size[1].max(1) as f64);
    bitmap.local_size[1] = bitmap.local_size[0] / aspect;
    Ok(vec![replace_data(node, NodeData::Bitmap(bitmap))])
}

pub(crate) fn paints(data: &NodeData) -> SmallVec<[Fill; 1]> {
    match data {
        NodeData::Vector(vector) => vector.fills.clone(),
        NodeData::Boolean(boolean) => boolean.fills.clone(),
        NodeData::Group(group) => group
            .background
            .iter()
            .chain(group.background_fills.iter())
            .cloned()
            .collect(),
        NodeData::Text(text) => smallvec::smallvec![Fill::solid(text.style.color)],
        NodeData::Bitmap(bitmap) => smallvec::smallvec![Fill::Image {
            asset: bitmap.asset,
            mode: bitmap.fit,
            opacity: 1.,
            crop: bitmap.crop.map(Box::new),
            scale: None,
            rotation: None,
            blend: fanta_doc::BlendMode::Normal,
            adjust: Default::default()
        }],
        _ => SmallVec::new(),
    }
}

fn local_outline(doc: &Doc, id: NodeId) -> Result<skia_safe::Path> {
    let node = doc.scene.get(id).context("Missing layer")?;
    match &node.data {
        NodeData::Vector(vector) => Ok(fanta_render::vector_outline_sk_path(
            &vector.path,
            vector.corner_radius,
            vector.corner_radii,
            vector.corner_smoothing,
        )),
        NodeData::Text(text) => Ok(fanta_render::to_sk_path(
            &fanta_render::text_node_outline(text).context("The font cannot be outlined")?,
        )),
        NodeData::Bitmap(bitmap) => Ok(fanta_render::to_sk_path(&PathData::rect(
            0.,
            0.,
            bitmap.local_size[0],
            bitmap.local_size[1],
        ))),
        NodeData::Group(_) | NodeData::Boolean(_) => {
            let operation = match &node.data {
                NodeData::Boolean(boolean) => match boolean.op {
                    fanta_doc::BooleanOp::Union => skia_safe::PathOp::Union,
                    fanta_doc::BooleanOp::Subtract => skia_safe::PathOp::Difference,
                    fanta_doc::BooleanOp::Intersect => skia_safe::PathOp::Intersect,
                    fanta_doc::BooleanOp::Exclude => skia_safe::PathOp::XOR,
                },
                _ => skia_safe::PathOp::Union,
            };
            let mut combined: Option<skia_safe::Path> = None;
            if let NodeData::Group(group) = &node.data
                && (!paints(&node.data).is_empty())
                && let Some(size) = group.clip_size.or(group.local_size)
            {
                combined = Some(fanta_render::to_sk_path(&PathData::rect(
                    0., 0., size[0], size[1],
                )));
            }
            for child in doc.scene.children_of(Some(id)) {
                let child_node = doc.scene.get(*child).context("Missing child")?;
                if child_node.flags.contains(fanta_doc::NodeFlags::HIDDEN) {
                    continue;
                }
                let mut outline = local_outline(doc, *child)?;
                outline.transform(&fanta_render::to_sk_matrix(&child_node.transform));
                combined = Some(match combined {
                    Some(previous) => previous
                        .op(&outline, operation)
                        .context("Could not combine this geometry")?,
                    None => outline,
                });
            }
            combined.context("The layer has no geometry")
        }
        _ => bail!("This layer has no editable vector geometry"),
    }
}

#[cfg(test)]
pub(crate) fn can_flatten(doc: &Doc, id: NodeId) -> bool {
    validate_flatten(doc, id).is_ok()
}

pub(crate) fn can_flatten_with_component_roots(
    doc: &Doc,
    id: NodeId,
    component_roots: &HashSet<NodeId>,
) -> bool {
    validate_flatten_with_component_roots(doc, id, component_roots).is_ok()
}

fn validate_flatten(doc: &Doc, id: NodeId) -> Result<()> {
    let component_roots = doc
        .components
        .defs
        .values()
        .map(|definition| definition.root)
        .collect();
    validate_flatten_with_component_roots(doc, id, &component_roots)
}

fn validate_flatten_with_component_roots(
    doc: &Doc,
    id: NodeId,
    component_roots: &HashSet<NodeId>,
) -> Result<()> {
    let node = doc.scene.get(id).context("Missing layer")?;
    ensure!(
        node.bindings.is_empty(),
        "Flatten cannot preserve variable bindings"
    );
    ensure!(
        !doc.scene
            .descendants_of(id)
            .any(|child| component_roots.contains(&child)),
        "Detach component content before flattening"
    );
    for child in doc.scene.descendants_of(id).filter(|child| *child != id) {
        let child = doc.scene.get(child).context("Missing child")?;
        ensure!(
            child.opacity == fanta_doc::UnitInterval::ONE
                && child.blend_mode.is_normal()
                && child.effects.is_empty()
                && child.blurs.is_empty()
                && !child.is_mask
                && !child
                    .flags
                    .intersects(fanta_doc::NodeFlags::HIDDEN | fanta_doc::NodeFlags::LOCKED)
                && child.bindings.is_empty()
                && child.reactions.is_empty(),
            "Flatten cannot preserve a child's appearance or interactions"
        );
    }
    ensure!(
        !doc.motion
            .clips
            .values()
            .any(|clip| clip.tracks.values().any(|track| {
                track.target.node != id
                    && doc
                        .scene
                        .ancestors_of(track.target.node)
                        .any(|parent| parent.id == id)
            })),
        "Flatten cannot preserve animated children"
    );
    match &node.data {
        NodeData::Vector(vector) => ensure!(
            vector.path.subpath_rules.is_empty(),
            "Flatten cannot preserve mixed path fill rules"
        ),
        NodeData::Text(text) => ensure!(
            std::iter::once(&text.style)
                .chain(text.style_runs.iter().map(|run| &run.style))
                .all(|style| style.color == text.style.color
                    && !style.underline
                    && !style.strikethrough),
            "Flatten cannot preserve mixed text colors or decorations"
        ),
        NodeData::Group(_) => {
            ensure!(
                node.opacity == fanta_doc::UnitInterval::ONE
                    && node.blend_mode.is_normal()
                    && node.effects.is_empty()
                    && node.blurs.is_empty(),
                "Flatten cannot preserve the group's compositing effects"
            );
            let mut paint = None;
            validate_flatten_group(doc, id, &mut paint)?;
            ensure!(paint.is_some(), "The layer has no painted geometry");
        }
        NodeData::Boolean(_) => {
            if active_boolean_vector(doc, id)?.is_none() {
                validate_boolean_flatten_geometry(doc, id)?;
            }
        }
        _ => bail!("This layer cannot be flattened without changing its appearance"),
    }
    Ok(())
}

fn validate_flatten_pattern_sources(doc: &Doc, id: NodeId) -> Result<()> {
    let node = doc.scene.get(id).context("Missing layer")?;
    if !matches!(node.data, NodeData::Vector(_)) || !doc.scene.children_of(Some(id)).is_empty() {
        let affected: HashSet<_> = doc.scene.descendants_of(id).collect();
        // Flatten deletes descendants and can change the retained node's
        // tile bounds. Either change would alter another node's live pattern.
        ensure!(
            !doc.scene
                .roots()
                .iter()
                .flat_map(|root| doc.scene.descendants_of(*root))
                .filter(|node| *node == id || !affected.contains(node))
                .filter_map(|node| doc.scene.get(node))
                .any(|node| references_pattern_source(&node.data, &affected)),
            "Flatten would change a pattern source used by another layer"
        );
    }
    Ok(())
}

fn references_pattern_source(data: &NodeData, affected: &HashSet<NodeId>) -> bool {
    let references = |paint: &Fill| matches!(paint, Fill::Pattern { pattern, .. } if affected.contains(&pattern.source_node_id));
    if data
        .strokes()
        .is_some_and(|strokes| strokes.iter().any(|stroke| references(&stroke.paint)))
    {
        return true;
    }
    match data {
        NodeData::Vector(vector) => vector.fills.iter().any(references),
        NodeData::Boolean(boolean) => boolean.fills.iter().any(references),
        NodeData::Group(group) => group
            .background
            .iter()
            .chain(&group.background_fills)
            .any(references),
        NodeData::Instance(instance) => {
            instance.overrides.iter().any(|entry| match &entry.value {
                fanta_doc::OverrideValue::Fills { fills } => fills.iter().any(references),
                fanta_doc::OverrideValue::Strokes { strokes } => {
                    strokes.iter().any(|stroke| references(&stroke.paint))
                }
                fanta_doc::OverrideValue::Field { value } => {
                    field_references_pattern_source(value, affected)
                }
                _ => false,
            }) || instance.derived.iter().any(|entry| {
                entry
                    .fills
                    .as_ref()
                    .is_some_and(|fills| fills.iter().any(references))
            })
        }
        _ => false,
    }
}

fn field_references_pattern_source(value: &serde_json::Value, affected: &HashSet<NodeId>) -> bool {
    match value {
        serde_json::Value::Object(fields) => {
            (value.get("kind").and_then(serde_json::Value::as_str) == Some("pattern")
                && value
                    .pointer("/pattern/source_node_id")
                    .and_then(|source| serde_json::from_value::<NodeId>(source.clone()).ok())
                    .is_some_and(|source| affected.contains(&source)))
                || fields
                    .values()
                    .any(|field| field_references_pattern_source(field, affected))
        }
        serde_json::Value::Array(values) => values
            .iter()
            .any(|field| field_references_pattern_source(field, affected)),
        _ => false,
    }
}

fn validate_flatten_group(doc: &Doc, id: NodeId, paint: &mut Option<Fill>) -> Result<()> {
    let node = doc.scene.get(id).context("Missing layer")?;
    match &node.data {
        NodeData::Group(group) => {
            ensure!(
                group.clip_size.is_none()
                    && group.background.is_none()
                    && group.background_fills.is_empty()
                    && group.strokes.is_empty()
                    && group.auto_layout.is_none()
                    && group.grid.is_none()
                    && group.explicit_modes.is_empty(),
                "Flatten cannot preserve frame paints, clipping or layout"
            );
            for child in doc.scene.children_of(Some(id)) {
                validate_flatten_group(doc, *child, paint)?;
            }
        }
        NodeData::Vector(vector) => {
            let clipped = vector.local_size.is_some_and(|[width, height]| {
                !node.flags.contains(fanta_doc::NodeFlags::UNCLIPPED_VECTOR)
                    && !vector.path.rough_bounds().is_some_and(|bounds| {
                        bounds.min_x >= 0.
                            && bounds.min_y >= 0.
                            && bounds.max_x <= width
                            && bounds.max_y <= height
                    })
            });
            ensure!(
                vector.strokes.is_empty() && !clipped && vector.path.subpath_rules.is_empty(),
                "Flatten cannot preserve child strokes or clipping"
            );
            let [fill @ Fill::Solid { color, blend }] = vector.fills.as_slice() else {
                bail!("Flatten requires one opaque solid paint per child");
            };
            ensure!(
                color.a == 255 && blend.is_normal(),
                "Flatten cannot preserve translucent paints"
            );
            if let Some(previous) = paint {
                ensure!(
                    previous == fill,
                    "Flatten cannot preserve different child paints"
                );
            } else {
                *paint = Some(fill.clone());
            }
        }
        _ => bail!("Flatten cannot preserve this group's child content"),
    }
    Ok(())
}

fn active_boolean_vector(doc: &Doc, id: NodeId) -> Result<Option<VectorNode>> {
    let node = doc.scene.get(id).context("Missing Boolean")?;
    let Some(boolean) = node
        .data
        .as_boolean()
        .filter(|boolean| boolean.baked.is_some())
    else {
        return Ok(None);
    };
    let signature = fanta_doc::boolean_geometry_signature(&doc.scene, id)?;
    Ok(boolean.baked_vector(&signature))
}

fn validate_boolean_flatten_geometry(doc: &Doc, id: NodeId) -> Result<()> {
    let node = doc.scene.get(id).context("Missing operand")?;
    match &node.data {
        NodeData::Vector(vector) => ensure!(
            vector.corner_radius.is_none()
                && vector.corner_radii.is_none()
                && vector.path.subpath_rules.is_empty(),
            "Flatten cannot preserve this Boolean operand's geometry"
        ),
        NodeData::Group(group) => ensure!(
            group.background.is_none() && group.background_fills.is_empty(),
            "Flatten cannot preserve a Boolean operand's frame background"
        ),
        NodeData::Boolean(boolean) => {
            if boolean.baked.is_some() {
                let signature = fanta_doc::boolean_geometry_signature(&doc.scene, id)?;
                ensure!(
                    boolean.baked_vector(&signature).is_none(),
                    "Flatten cannot preserve this imported Boolean's baked appearance"
                );
            }
        }
        _ => bail!("Flatten cannot preserve this Boolean operand"),
    }
    for child in doc.scene.children_of(Some(id)) {
        validate_boolean_flatten_geometry(doc, *child)?;
    }
    Ok(())
}

fn flatten(doc: &Doc, id: NodeId) -> Result<Vec<Operation>> {
    validate_flatten(doc, id)?;
    validate_flatten_pattern_sources(doc, id)?;
    let node = doc.scene.get(id).context("Missing layer")?;
    let vector = if let Some(vector) = active_boolean_vector(doc, id)? {
        vector
    } else {
        let outline = local_outline(doc, id)?;
        let mut path = PathData::from_svg_d(&outline.to_svg())
            .context("Could not serialize vector outline")?;
        path.fill_rule = match outline.fill_type() {
            skia_safe::PathFillType::EvenOdd => fanta_doc::FillRule::EvenOdd,
            _ => fanta_doc::FillRule::NonZero,
        };
        let mut fills = paints(&node.data);
        if fills.is_empty() && matches!(node.data, NodeData::Group(_)) {
            for child in doc.scene.descendants_of(id) {
                if let Some(node) = doc.scene.get(child) {
                    let paint = paints(&node.data);
                    if !paint.is_empty() {
                        fills = paint;
                    }
                }
            }
        }
        let strokes = match &node.data {
            NodeData::Vector(value) => value.strokes.clone(),
            NodeData::Boolean(value) => value.strokes.clone(),
            NodeData::Group(value) => value.strokes.clone(),
            _ => SmallVec::new(),
        };
        VectorNode {
            path,
            fills,
            strokes,
            local_size: match &node.data {
                NodeData::Vector(vector) => vector.local_size,
                _ => None,
            },
            ..Default::default()
        }
    };
    let mut operations = Vec::new();
    for child in doc.scene.children_of(Some(id)) {
        operations.push(Operation::DeleteSubtree {
            snapshot: doc
                .scene
                .descendants_of(*child)
                .filter_map(|id| doc.scene.get(id).cloned())
                .collect(),
        });
    }
    operations.push(replace_data(node, NodeData::Vector(vector)));
    Ok(operations)
}

fn stroke_geometry(
    outline: &skia_safe::Path,
    stroke: &fanta_doc::Stroke,
) -> Result<skia_safe::Path> {
    let bounds = outline.compute_tight_bounds();
    let ring = |width: f64| -> Result<skia_safe::Path> {
        let mut paint = fanta_render::stroke_to_paint(
            stroke,
            [bounds.left, bounds.top, bounds.right, bounds.bottom],
        );
        paint.set_stroke_width(
            (if stroke.align == fanta_doc::StrokeAlign::Center {
                width
            } else {
                width * 2.
            }) as f32,
        );
        let mut path = skia_safe::Path::new();
        ensure!(
            skia_safe::path_utils::fill_path_with_paint(outline, &paint, &mut path, None, None),
            "Could not outline this stroke"
        );
        match stroke.align {
            fanta_doc::StrokeAlign::Center => Ok(path),
            fanta_doc::StrokeAlign::Inside => path
                .op(outline, skia_safe::PathOp::Intersect)
                .context("Could not outline inside stroke"),
            fanta_doc::StrokeAlign::Outside => path
                .op(outline, skia_safe::PathOp::Difference)
                .context("Could not outline outside stroke"),
        }
    };
    let Some(sides) = stroke.per_side else {
        return ring(stroke.width);
    };
    let (x0, y0, x1, y1) = (bounds.left, bounds.top, bounds.right, bounds.bottom);
    let reach = sides.into_iter().fold(0., f64::max) as f32 + 2.;
    let depth = (x1 - x0).min(y1 - y0) * 0.5;
    let wedges = [
        [
            (x0 - reach, y0 - reach),
            (x1 + reach, y0 - reach),
            (x1 - depth, y0 + depth),
            (x0 + depth, y0 + depth),
        ],
        [
            (x1 + reach, y0 - reach),
            (x1 + reach, y1 + reach),
            (x1 - depth, y1 - depth),
            (x1 - depth, y0 + depth),
        ],
        [
            (x1 + reach, y1 + reach),
            (x0 - reach, y1 + reach),
            (x0 + depth, y1 - depth),
            (x1 - depth, y1 - depth),
        ],
        [
            (x0 - reach, y1 + reach),
            (x0 - reach, y0 - reach),
            (x0 + depth, y0 + depth),
            (x0 + depth, y1 - depth),
        ],
    ];
    let mut combined = skia_safe::Path::new();
    for (width, points) in sides.into_iter().zip(wedges) {
        if width <= 0. {
            continue;
        }
        let mut wedge = skia_safe::Path::new();
        for (index, point) in points.into_iter().enumerate() {
            if index == 0 {
                wedge.move_to(point);
            } else {
                wedge.line_to(point);
            }
        }
        wedge.close();
        let side = ring(width)?
            .op(&wedge, skia_safe::PathOp::Intersect)
            .context("Could not outline an individual border")?;
        combined = combined
            .op(&side, skia_safe::PathOp::Union)
            .context("Could not join the borders")?;
    }
    Ok(combined)
}

fn outline_strokes(doc: &Doc, id: NodeId) -> Result<Vec<Operation>> {
    let node = doc.scene.get(id).context("Missing layer")?;
    if matches!(node.data, NodeData::Text(_)) {
        return flatten(doc, id);
    }
    let strokes = match &node.data {
        NodeData::Vector(vector) => &vector.strokes,
        NodeData::Group(group) => &group.strokes,
        NodeData::Boolean(boolean) => &boolean.strokes,
        _ => bail!("The layer has no strokes"),
    };
    ensure!(!strokes.is_empty(), "The layer has no strokes to outline");
    let outline = if let NodeData::Group(group) = &node.data {
        let bounds = doc
            .scene
            .local_bounds(id)
            .context("The frame has no bounds")?;
        let path = PathData::rect(bounds.min_x, bounds.min_y, bounds.width(), bounds.height());
        fanta_render::vector_outline_sk_path(
            &path,
            group.corner_radius,
            group.corner_radii,
            group.corner_smoothing,
        )
    } else {
        local_outline(doc, id)?
    };
    let mut data = node.data.clone();
    match &mut data {
        NodeData::Vector(value) => value.strokes.clear(),
        NodeData::Boolean(value) => value.strokes.clear(),
        NodeData::Group(value) => value.strokes.clear(),
        _ => {}
    }
    let mut operations = Vec::new();
    let mut index = doc.scene.next_child_index(Some(id));
    // Composite effects and opacity once. A child retains the original frame's clipping and layout, while outside strokes remain outside that clip.
    let local_size = doc
        .scene
        .local_bounds(id)
        .map(|bounds| [bounds.width(), bounds.height()]);
    operations.push(replace_data(
        node,
        NodeData::Group(fanta_doc::GroupNode {
            local_size,
            ..Default::default()
        }),
    ));
    let mut content = CanvasNode::new(data);
    content.name = "Fill".into();
    content.parent = Some(id);
    content.index = index;
    index = fanta_doc::IndexKey::after(index);
    let content_id = content.id;
    operations.push(Operation::create_node(content));
    for child in doc.scene.children_of(Some(id)) {
        let child_node = doc.scene.get(*child).context("Missing child")?;
        operations.push(Operation::Reparent {
            id: *child,
            old_parent: child_node.parent,
            old_index: child_node.index,
            new_parent: Some(content_id),
            new_index: child_node.index,
        });
    }
    for stroke in strokes {
        if stroke.width <= 0. && stroke.per_side.is_none() {
            continue;
        }
        let path = stroke_geometry(&outline, stroke)?;
        let mut filled = CanvasNode::new(NodeData::Vector(VectorNode {
            path: PathData::from_svg_d(&path.to_svg())?,
            fills: smallvec::smallvec![stroke.paint.clone()],
            ..Default::default()
        }));
        filled.name = "Outline".into();
        filled.parent = Some(id);
        filled.index = index;
        index = fanta_doc::IndexKey::after(index);
        operations.push(Operation::create_node(filled));
    }
    Ok(operations)
}

pub(crate) fn paste_properties(
    doc: &Doc,
    id: NodeId,
    source: &CanvasNode,
) -> Result<Vec<Operation>> {
    let mut operations = Vec::new();
    for id in targets(doc, id) {
        ensure!(editable(doc, id), "A selected layer is locked");
        let node = doc.scene.get(id).context("Missing layer")?;
        let mut data = node.data.clone();
        let strokes = match &source.data {
            NodeData::Vector(value) => value.strokes.clone(),
            NodeData::Group(value) => value.strokes.clone(),
            NodeData::Boolean(value) => value.strokes.clone(),
            _ => SmallVec::new(),
        };
        let fills = paints(&source.data);
        match &mut data {
            NodeData::Vector(value) => {
                value.fills = fills;
                value.strokes = strokes;
            }
            NodeData::Boolean(value) => {
                value.fills = fills;
                value.strokes = strokes;
            }
            NodeData::Group(value) => {
                value.background = None;
                value.background_fills = fills;
                value.strokes = strokes;
            }
            NodeData::Text(value) => {
                if let NodeData::Text(source) = &source.data {
                    value.style = source.style.clone();
                }
            }
            _ => {}
        }
        operations.extend([
            replace_data(node, data),
            Operation::SetOpacity {
                id,
                old: node.opacity,
                new: source.opacity,
            },
            Operation::SetBlendMode {
                id,
                old: node.blend_mode,
                new: source.blend_mode,
            },
            Operation::SetEffects {
                id,
                old: node.effects.clone(),
                new: source.effects.clone(),
            },
            Operation::SetBlurs {
                id,
                old: node.blurs.clone(),
                new: source.blurs.clone(),
            },
        ]);
    }
    Ok(operations)
}

pub(crate) fn replace_media(
    document: &mut crate::document::FigDocument,
    expected_document: fanta_doc::DocId,
    id: NodeId,
    bytes: Vec<u8>,
    video: Option<crate::generation_media::PreparedVideo>,
) -> Result<()> {
    ensure!(
        document.doc.id == expected_document,
        "The document changed while choosing media"
    );
    ensure!(editable(&document.doc, id), "The layer is locked");
    let node = document
        .doc
        .scene
        .get(id)
        .context("The layer was removed")?
        .clone();
    let mut added = Vec::new();
    let data = match (&node.data, video) {
        (NodeData::Bitmap(bitmap), None) => {
            let (asset, natural_size, inserted) =
                document.doc_and_assets().1.add_image_tracked(bytes)?;
            if inserted {
                added.push(asset);
            }
            let mut bitmap = bitmap.clone();
            bitmap.asset = asset;
            bitmap.natural_size = natural_size;
            bitmap.crop = None;
            NodeData::Bitmap(bitmap)
        }
        (NodeData::Video(old), Some(video)) => {
            let asset = video.asset;
            if let Some(existing) = document.raw_assets.get(&asset) {
                ensure!(
                    existing.as_slice() == video.bytes.as_ref(),
                    "Video asset {asset} has a content hash collision"
                );
            }
            let poster = if let Some(poster) = &video.poster {
                let (asset, _, inserted) = document
                    .doc_and_assets()
                    .1
                    .add_image_tracked(poster.png.to_vec())?;
                if inserted {
                    added.push(asset);
                }
                Some(asset)
            } else {
                None
            };
            if !document.raw_assets.contains_key(&asset) {
                std::sync::Arc::make_mut(&mut document.raw_assets)
                    .insert(asset, video.bytes.to_vec());
                added.push(asset);
            }
            let mut value = old.clone();
            value.asset = asset;
            value.natural_size = [video.metadata.width, video.metadata.height];
            value.time_range_us = [0, video.metadata.duration_us];
            value.poster = poster;
            value.poster_frame_us = video.poster.map(|poster| poster.time_us);
            NodeData::Video(value)
        }
        _ => bail!("The layer type changed while choosing media"),
    };
    if let Err(error) = crate::clipboard::apply_transaction(
        &mut document.doc,
        "Replace media",
        vec![replace_data(&node, data)],
    ) {
        for asset in added {
            document.doc_and_assets().1.remove(asset);
        }
        return Err(error);
    }
    Ok(())
}

fn release_components(doc: &Doc, roots: &[NodeId]) -> Result<Vec<Operation>> {
    release_components_with_limits(doc, roots, 64, 100_000)
}

fn release_components_with_limits(
    doc: &Doc,
    roots: &[NodeId],
    maximum_depth: usize,
    maximum_nodes: usize,
) -> Result<Vec<Operation>> {
    let removed: std::collections::HashSet<_> = roots
        .iter()
        .flat_map(|id| doc.scene.descendants_of(*id))
        .collect();
    let components: std::collections::HashSet<_> = doc
        .components
        .defs
        .values()
        .filter(|def| removed.contains(&def.root))
        .map(|def| def.id)
        .collect();
    if components.is_empty() {
        return Ok(Vec::new());
    }
    let mut operations = Vec::new();
    let mut scratch = doc.clone();
    let mut pending = doc
        .scene
        .roots()
        .iter()
        .flat_map(|id| doc.scene.descendants_of(*id))
        .filter(|id| !removed.contains(id))
        .filter(|id| {
            doc.scene
                .get(*id)
                .is_some_and(|node| matches!(node.data, NodeData::Instance(_)))
        })
        .map(|id| (id, Vec::new()))
        .collect::<Vec<_>>();
    let mut detached_nodes = 0;
    while let Some((id, mut path)) = pending.pop() {
        let Some(node) = scratch.scene.get(id) else {
            continue;
        };
        let NodeData::Instance(instance) = &node.data else {
            continue;
        };
        for replacement in &instance.overrides {
            if let fanta_doc::OverrideValue::SwapInstance { component } = &replacement.value {
                let affected = components.contains(component)
                    || scratch.components.sets.get(component).is_some_and(|set| {
                        set.members.iter().any(|member| components.contains(member))
                    });
                ensure!(
                    !affected,
                    "A surviving instance uses this swapped component; detach or reset that swap before deleting its main component"
                );
            }
        }
        let Some(resolved) = fanta_doc::resolved_component_with_context(
            &scratch.scene,
            &scratch.components,
            instance,
            &fanta_doc::InstanceExpansionContext::new(
                &scratch.variables,
                &scratch.active_modes,
                id,
            ),
        )
        .filter(|resolved| components.contains(&resolved.resolved_component)) else {
            continue;
        };
        ensure!(
            !path.contains(&resolved.resolved_component),
            "Cannot preserve a cyclic component dependency; no layers were deleted"
        );
        ensure!(
            path.len() < maximum_depth,
            "Cannot safely detach more than {maximum_depth} nested components; no layers were deleted"
        );
        let count = scratch
            .scene
            .descendants_of(resolved.resolved_root)
            .take(maximum_nodes - detached_nodes + 1)
            .count();
        ensure!(
            count <= maximum_nodes - detached_nodes,
            "Cannot safely detach more than {maximum_nodes} component nodes at once; no layers were deleted"
        );
        detached_nodes += count;
        path.push(resolved.resolved_component);
        let detach = crate::properties_ops::detach_instance_operations(&scratch, id)?;
        ensure!(
            !detach.is_empty(),
            "Cannot preserve an instance of this component"
        );
        for operation in &detach {
            scratch.apply(operation.clone())?;
        }
        operations.extend(detach);
        // Expansion preserves nested instances, so materialized descendants
        // must be checked before their component definitions can be removed.
        pending.extend(
            scratch
                .scene
                .descendants_of(id)
                .filter(|id| {
                    scratch
                        .scene
                        .get(*id)
                        .is_some_and(|node| matches!(node.data, NodeData::Instance(_)))
                })
                .map(|id| (id, path.clone())),
        );
    }
    for set in doc
        .components
        .sets
        .values()
        .filter(|set| set.members.iter().any(|id| components.contains(id)))
    {
        let mut updated = set.clone();
        updated.members.retain(|id| !components.contains(id));
        if let Some(first) = updated.members.first().copied() {
            if components.contains(&updated.default_variant) {
                updated.default_variant = first;
            }
            operations.push(Operation::SetComponentSet {
                id: set.id,
                old: Box::new(set.clone()),
                new: Box::new(updated),
            });
        } else {
            operations.push(Operation::DeleteComponentSet {
                id: set.id,
                set: Box::new(set.clone()),
            });
        }
    }
    for id in components {
        let def = doc
            .components
            .def(id)
            .context("Missing component definition")?;
        operations.push(Operation::DeleteComponent {
            id,
            def: Box::new(def.clone()),
        });
    }
    Ok(operations)
}

pub(crate) fn delete_layers(doc: &Doc, id: NodeId) -> Result<Vec<Operation>> {
    let roots = targets(doc, id);
    for id in &roots {
        ensure!(editable(doc, *id), "A selected layer is locked");
    }
    let mut operations = release_components(doc, &roots)?;
    let mut scratch = doc.clone();
    for operation in &operations {
        scratch.apply(operation.clone())?;
    }
    scratch.selection.replace_with(roots);
    operations.extend(crate::clipboard::delete_operations(&scratch));
    Ok(operations)
}

pub(crate) fn stack(doc: &Doc, id: NodeId, front: bool) -> Result<Vec<Operation>> {
    let targets = targets(doc, id);
    let mut selected: Vec<_> = targets.iter().filter_map(|id| doc.scene.get(*id)).collect();
    selected.sort_by_key(|node| (node.parent, node.index));
    if !front {
        selected.reverse();
    }
    let mut scratch = doc.clone();
    let mut operations = Vec::new();
    for node in selected {
        ensure!(editable(doc, node.id), "A selected layer is locked");
        let siblings = scratch.scene.children_of(node.parent);
        let extreme = if front {
            siblings.last()
        } else {
            siblings.first()
        };
        let index = extreme
            .and_then(|id| scratch.scene.get(*id))
            .map(|node| node.index)
            .unwrap_or_default();
        let new = if front {
            fanta_doc::IndexKey::after(index)
        } else {
            fanta_doc::IndexKey::before(index)
        };
        let operation = Operation::SetIndex {
            id: node.id,
            old: node.index,
            new,
        };
        scratch.apply(operation.clone())?;
        operations.push(operation);
    }
    Ok(operations)
}

#[cfg(test)]
mod tests {
    use super::*;
    use fanta_doc::{
        BitmapNode, Color, ComponentDef, ComponentId, GroupNode, InstanceNode, Stroke, UnitInterval,
    };

    fn insert(doc: &mut Doc, mut node: CanvasNode, parent: Option<NodeId>) -> NodeId {
        node.parent = parent;
        node.index = doc.scene.next_child_index(parent);
        let id = node.id;
        doc.scene.insert(node).expect("insert fixture");
        id
    }
    fn rect() -> CanvasNode {
        CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
            0.,
            0.,
            40.,
            20.,
            Color::BLACK,
        )))
    }
    fn fixture() -> (Doc, NodeId, NodeId) {
        let mut doc = Doc::new();
        let page = insert(
            &mut doc,
            CanvasNode::new(NodeData::Group(GroupNode::default())),
            None,
        );
        doc.apply(Operation::SetPages {
            old: vec![],
            new: vec![page],
        })
        .expect("pages");
        let id = insert(&mut doc, rect(), Some(page));
        (doc, page, id)
    }
    fn roundtrip(doc: &mut Doc, operations: Vec<Operation>, verify: impl Fn(&Doc)) {
        let before = serde_json::to_value(&doc.scene).expect("serialize scene");
        let components = serde_json::to_value(&doc.components).expect("serialize components");
        assert!(crate::clipboard::apply_transaction(doc, "Menu test", operations).expect("apply"));
        verify(doc);
        assert!(doc.undo().expect("undo"));
        assert_eq!(serde_json::to_value(&doc.scene).expect("scene"), before);
        assert_eq!(
            serde_json::to_value(&doc.components).expect("components"),
            components
        );
        assert!(doc.redo().expect("redo"));
        verify(doc);
    }

    fn nested_component_delete_fixture(cyclic: bool) -> (Doc, NodeId, NodeId, fanta_doc::AssetId) {
        let mut doc = Doc::new();
        let page = insert(
            &mut doc,
            CanvasNode::new(NodeData::Group(GroupNode::default())),
            None,
        );
        doc.add_page(page);
        doc.set_active_page(Some(page));
        let mut master = CanvasNode::new(NodeData::Group(GroupNode {
            clip_size: Some([32.0, 32.0]),
            ..Default::default()
        }));
        master.transform = Transform2D::translation(500.0, 500.0);
        let outer = insert(&mut doc, master.clone(), Some(page));
        let inner = insert(
            &mut doc,
            {
                master.id = NodeId::new();
                master
            },
            Some(page),
        );
        let outer_component = ComponentId::new();
        let inner_component = ComponentId::new();
        doc.components.defs.insert(
            outer_component,
            ComponentDef::new(outer_component, outer, "Outer"),
        );
        doc.components.defs.insert(
            inner_component,
            ComponentDef::new(inner_component, inner, "Inner"),
        );
        let instance = |component| {
            CanvasNode::new(NodeData::Instance(InstanceNode {
                component,
                overrides: Vec::new(),
                prop_values: Default::default(),
                derived: Vec::new(),
                local_size: [32.0, 32.0],
            }))
        };
        let mut nested = instance(inner_component);
        nested.opacity = UnitInterval::new(0.7);
        insert(&mut doc, nested, Some(outer));
        let asset = fanta_doc::AssetId::new();
        if cyclic {
            insert(&mut doc, instance(outer_component), Some(inner));
        } else {
            insert(
                &mut doc,
                CanvasNode::new(NodeData::Bitmap(BitmapNode {
                    asset,
                    natural_size: [2, 2],
                    local_size: [32.0, 32.0],
                    crop: Some([0.0, 0.0, 0.75, 1.0]),
                    fit: fanta_doc::ImageFitMode::Fill,
                    tint: Some(Color::rgba(200, 180, 255, 230)),
                })),
                Some(inner),
            );
        }
        let mut survivor = instance(outer_component);
        survivor.opacity = UnitInterval::new(0.6);
        let survivor = insert(&mut doc, survivor, Some(page));
        doc.selection.replace_with([outer, inner]);
        doc.history = Default::default();
        (doc, outer, survivor, asset)
    }

    #[test]
    fn layer_menu_delete_nested_masters_preserves_survivor_pixels_and_undo() {
        let (mut doc, outer, survivor, asset) = nested_component_delete_fixture(false);
        let mut resolver = fanta_render::InMemoryAssetResolver::new();
        let bytes = std::sync::Arc::new(vec![
            255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 128, 255, 255, 0, 200,
        ]);
        resolver.insert(asset, fanta_render::DecodedImage::new(bytes.clone(), 2, 2));
        let resolver = std::sync::Arc::new(resolver);
        let pixels = |doc: &Doc| {
            let mut renderer = fanta_render::RasterRenderer::new(128, 128).expect("renderer");
            renderer.set_asset_resolver(resolver.clone());
            renderer.render_with(
                &doc.scene,
                &doc.viewport,
                &fanta_render::RenderInputs {
                    components: &doc.components,
                    variables: &doc.variables,
                    active_modes: &doc.active_modes,
                    ..fanta_render::RenderInputs::empty()
                },
            );
            renderer.copy_rgba()
        };
        let expected = pixels(&doc);
        assert!(expected.chunks_exact(4).any(|pixel| pixel[3] > 0));
        let operations = delete_layers(&doc, outer).expect("delete nested definitions");
        roundtrip(&mut doc, operations, |doc| {
            assert!(doc.components.is_empty());
            let descendants = doc
                .scene
                .descendants_of(survivor)
                .filter_map(|id| doc.scene.get(id))
                .collect::<Vec<_>>();
            assert!(
                descendants
                    .iter()
                    .all(|node| !matches!(node.data, NodeData::Instance(_))),
                "newly detached nested instances must not reference the removed inner master"
            );
            assert!(descendants.iter().any(
                |node| matches!(&node.data, NodeData::Bitmap(bitmap) if bitmap.asset == asset)
            ));
            assert_eq!(
                pixels(doc),
                expected,
                "nested survivor keeps exact painted appearance and asset"
            );
            assert_eq!(
                fanta_render::AssetResolver::resolve(resolver.as_ref(), asset)
                    .expect("asset")
                    .pixels_rgba
                    .as_ref(),
                bytes.as_ref()
            );
        });
    }

    #[test]
    fn layer_menu_detach_master_blend_keeps_outer_isolation_pixels() {
        for isolated in [false, true] {
            let (mut doc, outer, survivor, asset) = nested_component_delete_fixture(false);
            doc.scene.get_mut(outer).expect("master").blend_mode = fanta_doc::BlendMode::Multiply;
            let placed = doc.scene.get_mut(survivor).expect("instance");
            placed.opacity = UnitInterval::new(if isolated { 1.0 } else { 0.4 });
            placed
                .flags
                .set(fanta_doc::NodeFlags::ISOLATED_BLEND, isolated);
            let page = doc.active_page();
            let mut backdrop = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
                -64.0,
                -64.0,
                128.0,
                128.0,
                Color::rgb(255, 0, 0),
            )));
            backdrop.parent = page;
            backdrop.index = fanta_doc::IndexKey::from_raw(0.0);
            doc.scene.insert(backdrop).expect("opaque red backdrop");
            let mut resolver = fanta_render::InMemoryAssetResolver::new();
            resolver.insert(
                asset,
                fanta_render::DecodedImage::new(
                    std::sync::Arc::new([0, 0, 255, 255].repeat(4)),
                    2,
                    2,
                ),
            );
            let resolver = std::sync::Arc::new(resolver);
            let pixels = |doc: &Doc| {
                let mut renderer = fanta_render::RasterRenderer::new(128, 128).expect("renderer");
                renderer.set_asset_resolver(resolver.clone());
                renderer.render_with(
                    &doc.scene,
                    &doc.viewport,
                    &fanta_render::RenderInputs {
                        components: &doc.components,
                        variables: &doc.variables,
                        active_modes: &doc.active_modes,
                        ..fanta_render::RenderInputs::empty()
                    },
                );
                renderer.copy_rgba()
            };
            let expected = pixels(&doc);
            assert!(
                expected.chunks_exact(4).any(|pixel| pixel[2] > 0),
                "the original isolated instance preserves a blue contribution"
            );
            let before = serde_json::to_value(&doc).expect("snapshot");
            let error = crate::properties_ops::detach_instance_operations(&doc, survivor)
                .expect_err("unsafe blend hoisting must be refused");
            assert!(error.to_string().contains("blend mode across"));
            assert!(
                delete_layers(&doc, outer).is_err(),
                "destructive commands use the same appearance guard"
            );
            assert_eq!(serde_json::to_value(&doc).expect("snapshot"), before);
            assert_eq!(pixels(&doc), expected);
        }
    }

    #[test]
    fn layer_menu_delete_refuses_unsupported_instance_composition_atomically() {
        for composition in ["bound opacity", "effects", "blurs", "blend modes"] {
            let (mut doc, outer, survivor, _) = nested_component_delete_fixture(false);
            match composition {
                "bound opacity" => {
                    let collection = fanta_doc::VariableCollectionId::new();
                    let mode = fanta_doc::ModeId::new();
                    let variable = fanta_doc::VariableId::new();
                    doc.variables.collections.insert(
                        collection,
                        fanta_doc::VariableCollection {
                            id: collection,
                            name: "Opacity".into(),
                            modes: vec![fanta_doc::Mode {
                                id: mode,
                                name: "Default".into(),
                            }],
                            default_mode: mode,
                            variable_order: vec![variable],
                        },
                    );
                    doc.variables.variables.insert(
                        variable,
                        fanta_doc::Variable {
                            id: variable,
                            collection,
                            name: "Photo opacity".into(),
                            ty: fanta_doc::VariableType::Float,
                            values_by_mode: [(mode, fanta_doc::VarValue::Float { value: 0.4 })]
                                .into_iter()
                                .collect(),
                            scopes: Vec::new(),
                        },
                    );
                    doc.scene
                        .get_mut(survivor)
                        .expect("instance")
                        .bindings
                        .insert(fanta_doc::BoundProp::Opacity, variable);
                    doc.scene.get_mut(outer).expect("master").opacity = UnitInterval::new(0.5);
                }
                "effects" => {
                    for (id, offset) in [(outer, [7.0, 3.0]), (survivor, [-5.0, 2.0])] {
                        doc.scene
                            .get_mut(id)
                            .expect("node")
                            .effects
                            .push(fanta_doc::Shadow {
                                kind: Default::default(),
                                color: Color::BLACK,
                                blur: 2.0,
                                spread: 0.0,
                                offset,
                                show_behind_node: true,
                            });
                    }
                }
                "blurs" => {
                    doc.scene
                        .get_mut(outer)
                        .expect("master")
                        .blurs
                        .push(fanta_doc::Blur::layer(3.0));
                    doc.scene
                        .get_mut(survivor)
                        .expect("instance")
                        .blurs
                        .push(fanta_doc::Blur::layer(5.0));
                }
                "blend modes" => {
                    doc.scene.get_mut(outer).expect("master").blend_mode =
                        fanta_doc::BlendMode::Multiply;
                    doc.scene.get_mut(survivor).expect("instance").blend_mode =
                        fanta_doc::BlendMode::Screen;
                }
                _ => unreachable!(),
            }
            let before = serde_json::to_value(&doc).expect("snapshot");
            let detach_error = crate::properties_ops::detach_instance_operations(&doc, survivor)
                .expect_err("explicit Detach must also refuse");
            assert!(
                detach_error.to_string().contains(composition),
                "{composition}: {detach_error}"
            );
            let error = delete_layers(&doc, outer)
                .expect_err("deletion must refuse rather than change composed appearance");
            assert!(
                error.to_string().contains(composition),
                "{composition}: {error}"
            );
            assert_eq!(serde_json::to_value(&doc).expect("snapshot"), before);
        }
    }

    #[test]
    fn layer_menu_delete_nested_component_limits_refuse_without_partial_edits() {
        let (doc, _, _, _) = nested_component_delete_fixture(false);
        for (depth, nodes, reason) in [(1, 100, "nested components"), (64, 3, "component nodes")] {
            let before = serde_json::to_value(&doc).expect("snapshot");
            let error =
                release_components_with_limits(&doc, doc.selection.as_slice(), depth, nodes)
                    .expect_err("bounded materialization");
            assert!(error.to_string().contains(reason), "{error}");
            assert_eq!(serde_json::to_value(&doc).expect("snapshot"), before);
        }
    }

    #[test]
    fn layer_menu_delete_refuses_a_newly_materialized_sparse_swap() {
        let (mut doc, outer, _, _) = nested_component_delete_fixture(false);
        let original_nested = *doc
            .scene
            .children_of(Some(outer))
            .first()
            .expect("nested instance");
        let NodeData::Instance(original) = &doc.scene.get(original_nested).expect("node").data
        else {
            panic!("instance");
        };
        let removed_component = original.component;
        let page = doc.active_page();
        let master = || {
            let mut node = CanvasNode::new(NodeData::Group(GroupNode {
                clip_size: Some([32.0, 32.0]),
                ..Default::default()
            }));
            node.transform = Transform2D::translation(700.0, 700.0);
            node
        };
        let surviving_root = insert(&mut doc, master(), page);
        let default_root = insert(&mut doc, master(), page);
        let surviving_component = ComponentId::new();
        let default_component = ComponentId::new();
        doc.components.defs.insert(
            surviving_component,
            ComponentDef::new(surviving_component, surviving_root, "Surviving base"),
        );
        doc.components.defs.insert(
            default_component,
            ComponentDef::new(default_component, default_root, "Default nested"),
        );
        let default_instance = insert(
            &mut doc,
            CanvasNode::new(NodeData::Instance(InstanceNode {
                component: default_component,
                overrides: Vec::new(),
                prop_values: Default::default(),
                derived: Vec::new(),
                local_size: [32.0, 32.0],
            })),
            Some(surviving_root),
        );
        doc.scene
            .get_mut(original_nested)
            .expect("original nested")
            .data = NodeData::Instance(InstanceNode {
            component: surviving_component,
            overrides: vec![fanta_doc::Override {
                target_path: [default_instance].into_iter().collect(),
                target_prop: fanta_doc::BoundProp::Visible,
                value: fanta_doc::OverrideValue::SwapInstance {
                    component: removed_component,
                },
            }],
            prop_values: Default::default(),
            derived: Vec::new(),
            local_size: [32.0, 32.0],
        });
        let before = serde_json::to_value(&doc).expect("snapshot");
        let error = delete_layers(&doc, outer)
            .expect_err("newly materialized survivor has an unsupported sparse swap dependency");
        assert!(error.to_string().contains("swapped component"));
        assert_eq!(serde_json::to_value(&doc).expect("snapshot"), before);
    }

    #[test]
    fn layer_menu_delete_nested_component_cycle_is_refused_atomically() {
        let (doc, outer, _, _) = nested_component_delete_fixture(true);
        let before = serde_json::to_value(&doc).expect("snapshot");
        let error = delete_layers(&doc, outer)
            .expect_err("cyclic dependency cannot be materialized safely");
        assert!(error.to_string().contains("cyclic"));
        assert_eq!(serde_json::to_value(&doc).expect("snapshot"), before);
    }

    #[test]
    fn layer_menu_mask_roundtrips() {
        let (mut doc, _, id) = fixture();
        let operations = simple(&doc, id, Action::UseAsMask).expect("mask");
        roundtrip(&mut doc, operations, |doc| {
            assert!(doc.scene.get(id).expect("node").is_mask)
        });
    }

    #[test]
    fn layer_menu_flip_preserves_bounds_and_reflects_the_selection_as_one() {
        let (mut doc, page, left) = fixture();
        let mut node = rect();
        node.transform = Transform2D::translation(100., 0.);
        let right = insert(&mut doc, node, Some(page));
        doc.selection.replace_with([left, right]);
        let operations = simple(&doc, left, Action::FlipHorizontal).expect("flip");
        roundtrip(&mut doc, operations, |doc| {
            assert_eq!(doc.scene.world_bounds(left).expect("bounds").min_x, 100.);
            assert_eq!(doc.scene.world_bounds(right).expect("bounds").min_x, 0.);
        });
    }

    #[test]
    fn layer_menu_locked_ancestor_prevents_mutation() {
        let (mut doc, page, id) = fixture();
        doc.scene
            .get_mut(page)
            .expect("page")
            .flags
            .insert(fanta_doc::NodeFlags::LOCKED);
        for action in [
            Action::UseAsMask,
            Action::FlipHorizontal,
            Action::CreateComponent,
            Action::Flatten,
        ] {
            assert!(simple(&doc, id, action).is_err());
        }
        assert!(delete_layers(&doc, id).is_err());
    }

    #[test]
    fn layer_menu_frame_section_conversion_and_layout_are_reversible() {
        let (mut doc, page, _) = fixture();
        let id = insert(
            &mut doc,
            CanvasNode::new(NodeData::Group(GroupNode {
                clip_size: Some([100., 80.]),
                ..Default::default()
            })),
            Some(page),
        );
        let operations = simple(&doc, id, Action::ConvertToSection).expect("section");
        roundtrip(&mut doc, operations, |doc| {
            assert!(is_section(doc.scene.get(id).expect("node")))
        });
        let operations = simple(&doc, id, Action::ConvertToFrame).expect("frame");
        roundtrip(&mut doc, operations, |doc| {
            let node = doc.scene.get(id).expect("node");
            assert!(!is_section(node));
            assert!(matches!(&node.data, NodeData::Group(group) if group.clip_size.is_some()));
        });
        let operations = auto_layout(&doc, id, Some(LayoutMode::Vertical)).expect("layout");
        roundtrip(&mut doc, operations, |doc| {
            assert!(
                matches!(&doc.scene.get(id).expect("node").data, NodeData::Group(group) if group.auto_layout.as_ref().is_some_and(|layout| layout.mode == LayoutMode::Vertical))
            )
        });
    }

    #[test]
    fn layer_menu_move_page_preserves_world_transform() {
        let (mut doc, first, id) = fixture();
        doc.scene.get_mut(first).expect("page").transform = Transform2D::translation(37., 24.);
        let mut page = CanvasNode::new(NodeData::Group(GroupNode::default()));
        page.transform = Transform2D::translation(-100., 10.);
        let second = insert(&mut doc, page, None);
        doc.apply(Operation::SetPages {
            old: vec![first],
            new: vec![first, second],
        })
        .expect("pages");
        let world = doc.scene.world_transform(id).expect("transform");
        let operations = move_to_page(&doc, id, second).expect("move");
        roundtrip(&mut doc, operations, |doc| {
            assert_eq!(doc.scene.get(id).expect("node").parent, Some(second));
            assert_eq!(doc.scene.world_transform(id), Some(world));
        });
    }

    #[test]
    fn layer_menu_crop_and_reset_preserve_image_aspect() {
        let (mut doc, page, _) = fixture();
        let id = insert(
            &mut doc,
            CanvasNode::new(NodeData::Bitmap(BitmapNode {
                asset: fanta_doc::AssetId::new(),
                natural_size: [400, 200],
                local_size: [400., 200.],
                crop: None,
                fit: fanta_doc::ImageFitMode::Fill,
                tint: None,
            })),
            Some(page),
        );
        let operations = crop(&doc, id, Some(1.)).expect("crop");
        roundtrip(&mut doc, operations, |doc| {
            assert!(
                matches!(&doc.scene.get(id).expect("image").data, NodeData::Bitmap(image) if image.crop == Some([0.25, 0., 0.5, 1.]) && image.local_size == [400.,400.])
            )
        });
        let operations = crop(&doc, id, None).expect("reset");
        roundtrip(&mut doc, operations, |doc| {
            assert!(
                matches!(&doc.scene.get(id).expect("image").data, NodeData::Bitmap(image) if image.crop.is_none() && image.local_size == [400.,200.])
            )
        });
        assert!(crop(&doc, id, Some(f64::NAN)).is_err());
    }

    #[test]
    fn layer_menu_outline_keeps_distinct_paints_and_stacking() {
        let (mut doc, page, id) = fixture();
        if let NodeData::Vector(vector) = &mut doc.scene.get_mut(id).expect("vector").data {
            vector.strokes.push(Stroke::solid(Color::BLACK, 3.));
        }
        let next = insert(&mut doc, rect(), Some(page));
        let operations = simple(&doc, id, Action::OutlineStroke).expect("outline");
        roundtrip(&mut doc, operations, |doc| {
            let children = doc.scene.children_of(Some(page));
            assert_eq!(children.len(), 2);
            assert_eq!(children.first(), Some(&id));
            assert_eq!(children.last(), Some(&next));
            assert!(matches!(
                &doc.scene.get(id).expect("vector").data,
                NodeData::Group(_)
            ));
            assert_eq!(doc.scene.children_of(Some(id)).len(), 2);
        });
    }

    #[test]
    fn layer_menu_flatten_removes_children_and_undo_restores_them() {
        let (mut doc, page, _) = fixture();
        let group = insert(
            &mut doc,
            CanvasNode::new(NodeData::Group(GroupNode::default())),
            Some(page),
        );
        let child = insert(&mut doc, rect(), Some(group));
        let operations = simple(&doc, group, Action::Flatten).expect("flatten");
        roundtrip(&mut doc, operations, |doc| {
            assert!(!doc.scene.contains(child));
            assert!(matches!(
                doc.scene.get(group).expect("node").data,
                NodeData::Vector(_)
            ));
        });
    }

    #[test]
    fn flatten_refuses_unsupported_appearance_without_changing_the_document() {
        for unsupported in [
            "paint",
            "opacity",
            "effect",
            "mask",
            "clip",
            "stroke",
            "hidden",
            "group-opacity",
        ] {
            let (mut doc, page, _) = fixture();
            let group = insert(
                &mut doc,
                CanvasNode::new(NodeData::Group(GroupNode::default())),
                Some(page),
            );
            insert(&mut doc, rect(), Some(group));
            let child = insert(&mut doc, rect(), Some(group));
            if unsupported == "group-opacity" {
                doc.scene.get_mut(group).expect("group").opacity = UnitInterval::new(0.5);
            } else if unsupported == "clip" {
                let NodeData::Group(group) = &mut doc.scene.get_mut(group).expect("group").data
                else {
                    panic!("group fixture");
                };
                group.clip_size = Some([10., 10.]);
            } else {
                let node = doc.scene.get_mut(child).expect("child");
                match unsupported {
                    "opacity" => node.opacity = UnitInterval::new(0.5),
                    "effect" => node.effects.push(fanta_doc::Shadow {
                        kind: fanta_doc::ShadowKind::Drop,
                        color: Color::BLACK,
                        blur: 4.,
                        spread: 0.,
                        offset: [2., 2.],
                        show_behind_node: false,
                    }),
                    "mask" => node.is_mask = true,
                    "hidden" => node.flags.insert(fanta_doc::NodeFlags::HIDDEN),
                    _ => {
                        let NodeData::Vector(vector) = &mut node.data else {
                            panic!("vector fixture");
                        };
                        if unsupported == "paint" {
                            vector.fills = smallvec::smallvec![Fill::solid(Color::WHITE)];
                        } else {
                            vector.strokes.push(Stroke::solid(Color::WHITE, 4.));
                        }
                    }
                }
            }
            let before = serde_json::to_value(&doc).expect("snapshot");
            let history = doc.history.undo_depth();
            assert!(!can_flatten(&doc, group), "{unsupported}");
            assert!(
                simple(&doc, group, Action::Flatten).is_err(),
                "{unsupported}"
            );
            assert!(
                !crate::gpui_adapters::layers::context_actions(&doc, group)
                    .contains(&Action::Flatten)
            );
            assert_eq!(serde_json::to_value(&doc).expect("unchanged doc"), before);
            assert_eq!(doc.history.undo_depth(), history);
        }
        let mut doc = Doc::new();
        let mut text = fanta_doc::TextNode::new("Two colors", 100., 30.);
        let mut style = text.style.clone();
        style.color = Color::WHITE;
        text.style_runs.push(fanta_doc::TextStyleRun {
            start: 4,
            end: 10,
            style,
        });
        let text = insert(&mut doc, CanvasNode::new(NodeData::Text(text)), None);
        let before = serde_json::to_value(&doc).expect("text snapshot");
        assert!(simple(&doc, text, Action::OutlineStroke).is_err());
        assert!(
            !crate::gpui_adapters::layers::context_actions(&doc, text)
                .contains(&Action::OutlineStroke)
        );
        assert_eq!(serde_json::to_value(&doc).expect("unchanged text"), before);
    }

    fn flatten_pixels(doc: &Doc) -> Vec<u8> {
        let mut renderer = fanta_render::RasterRenderer::new(128, 128).expect("renderer");
        renderer.render(&doc.scene, &doc.viewport);
        renderer.copy_rgba()
    }

    fn node_pattern(source_node_id: NodeId) -> Fill {
        Fill::Pattern {
            pattern: Box::new(fanta_doc::PatternFill {
                source_node_id,
                tile_type: Default::default(),
                scaling_factor: 1.,
                spacing: Default::default(),
                horizontal_alignment: Default::default(),
            }),
            opacity: 1.,
            blend: fanta_doc::BlendMode::Normal,
        }
    }

    #[test]
    fn flatten_refuses_direct_and_indirect_pattern_sources_without_changing_pixels() {
        for source_kind in [
            "child",
            "nested-child",
            "container",
            "indirect",
            "boolean-child",
            "text",
        ] {
            let mut doc = Doc::new();
            let mut container = CanvasNode::new(if source_kind == "text" {
                NodeData::Text(fanta_doc::TextNode::new("Pattern", 100., 40.))
            } else if source_kind == "boolean-child" {
                NodeData::Boolean(fanta_doc::BooleanNode::default())
            } else {
                NodeData::Group(GroupNode {
                    local_size: Some([100., 100.]),
                    ..Default::default()
                })
            });
            container.transform = Transform2D::translation(200., 200.);
            let root = insert(&mut doc, container, None);
            let parent = if source_kind == "nested-child" {
                insert(
                    &mut doc,
                    CanvasNode::new(NodeData::Group(GroupNode::default())),
                    Some(root),
                )
            } else {
                root
            };
            let child = if source_kind == "text" {
                root
            } else {
                insert(&mut doc, rect(), Some(parent))
            };
            assert!(can_flatten(&doc, root));
            let mut source = if source_kind == "container" {
                root
            } else {
                child
            };
            if source_kind == "indirect" {
                let mut intermediary = VectorNode::rect_solid(0., 0., 40., 20., Color::WHITE);
                intermediary.fills = smallvec::smallvec![node_pattern(source)];
                let mut intermediary = CanvasNode::new(NodeData::Vector(intermediary));
                intermediary.transform = Transform2D::translation(400., 400.);
                source = insert(&mut doc, intermediary, None);
            }
            let mut target = VectorNode::rect_solid(-20., -20., 40., 40., Color::WHITE);
            target.fills = smallvec::smallvec![node_pattern(source)];
            insert(&mut doc, CanvasNode::new(NodeData::Vector(target)), None);
            let before = serde_json::to_value(&doc).expect("snapshot");
            let pixels = flatten_pixels(&doc);
            assert!(
                pixels.chunks_exact(4).any(|pixel| pixel[3] > 0),
                "visible pattern {source_kind}"
            );
            let history = doc.history.undo_depth();
            assert!(
                can_flatten(&doc, root),
                "appearance-only menu eligibility: {source_kind}"
            );
            let error =
                simple(&doc, root, Action::Flatten).expect_err("refuse pattern source deletion");
            assert!(
                error.to_string().contains("pattern source"),
                "{source_kind}: {error}"
            );
            assert_eq!(
                serde_json::to_value(&doc).expect("unchanged document"),
                before
            );
            assert_eq!(flatten_pixels(&doc), pixels);
            assert_eq!(doc.history.undo_depth(), history);
        }
    }

    #[test]
    fn flatten_pattern_guard_covers_every_typed_paint_slot_and_field_overrides() {
        let source = NodeId::new();
        let pattern = node_pattern(source);
        let mut stroke = Stroke::solid(Color::BLACK, 2.);
        stroke.paint = pattern.clone();
        let mut cases = vec![
            NodeData::Vector(VectorNode {
                fills: smallvec::smallvec![pattern.clone()],
                ..Default::default()
            }),
            NodeData::Vector(VectorNode {
                strokes: smallvec::smallvec![stroke.clone()],
                ..Default::default()
            }),
            NodeData::Group(GroupNode {
                background: Some(pattern.clone()),
                ..Default::default()
            }),
            NodeData::Group(GroupNode {
                background_fills: smallvec::smallvec![pattern.clone()],
                ..Default::default()
            }),
            NodeData::Group(GroupNode {
                strokes: smallvec::smallvec![stroke.clone()],
                ..Default::default()
            }),
            NodeData::Boolean(fanta_doc::BooleanNode {
                fills: smallvec::smallvec![pattern.clone()],
                ..Default::default()
            }),
            NodeData::Boolean(fanta_doc::BooleanNode {
                strokes: smallvec::smallvec![stroke.clone()],
                ..Default::default()
            }),
        ];
        let instance = InstanceNode {
            component: ComponentId::new(),
            overrides: Vec::new(),
            prop_values: Default::default(),
            derived: Vec::new(),
            local_size: [40., 20.],
        };
        for value in [
            fanta_doc::OverrideValue::Fills {
                fills: smallvec::smallvec![pattern.clone()],
            },
            fanta_doc::OverrideValue::Strokes {
                strokes: smallvec::smallvec![stroke],
            },
            fanta_doc::OverrideValue::Field {
                value: serde_json::json!({"fills": [pattern]}),
            },
        ] {
            let mut instance = instance.clone();
            instance.overrides.push(fanta_doc::Override {
                target_path: Default::default(),
                target_prop: fanta_doc::BoundProp::FillColor { index: 0 },
                value,
            });
            cases.push(NodeData::Instance(instance));
        }
        let mut instance = instance;
        instance.derived.push(fanta_doc::DerivedOverride {
            path: Default::default(),
            transform: None,
            size: None,
            fills: Some(smallvec::smallvec![pattern]),
            path_data: None,
            stroke_path: None,
            stroke_weight: None,
            text: None,
        });
        cases.push(NodeData::Instance(instance));
        let affected = HashSet::from([source]);
        let unrelated = HashSet::from([NodeId::new()]);
        for data in cases {
            assert!(references_pattern_source(&data, &affected), "{data:?}");
            assert!(!references_pattern_source(&data, &unrelated));
        }
        assert!(!field_references_pattern_source(
            &serde_json::json!({"fills": [{"kind": "pattern", "pattern": {"source_node_id": "invalid"}}]}),
            &affected,
        ));
    }

    #[test]
    fn flatten_preserves_root_bakes_and_refuses_nested_bakes_without_changing_content() {
        for (nested, stroke_outline) in [(false, false), (false, true), (true, false)] {
            let mut doc = Doc::new();
            let root = insert(
                &mut doc,
                CanvasNode::new(NodeData::Boolean(fanta_doc::BooleanNode {
                    fills: smallvec::smallvec![Fill::solid(Color::BLACK)],
                    ..Default::default()
                })),
                None,
            );
            let baked_id = if nested {
                insert(
                    &mut doc,
                    CanvasNode::new(NodeData::Boolean(fanta_doc::BooleanNode::default())),
                    Some(root),
                )
            } else {
                root
            };
            let operand = insert(&mut doc, rect(), Some(baked_id));
            if stroke_outline {
                let boolean = doc
                    .scene
                    .get_mut(baked_id)
                    .expect("Boolean")
                    .data
                    .as_boolean_mut()
                    .expect("Boolean data");
                boolean.fills.clear();
                boolean.strokes.push(Stroke::solid(Color::BLACK, 2.));
            }
            let mut vector = VectorNode::rect_solid(0., 0., 60., 30., Color::BLACK);
            vector.local_size = Some([45., 25.]);
            doc.scene
                .get_mut(baked_id)
                .expect("Boolean")
                .data
                .as_boolean_mut()
                .expect("Boolean data")
                .baked = Some(fanta_doc::BooleanBakedGeometry {
                vector,
                source: String::new(),
                stroke_outline,
            });
            let source = fanta_doc::boolean_geometry_signature(&doc.scene, baked_id)
                .expect("operand signature");
            doc.scene
                .get_mut(baked_id)
                .expect("Boolean")
                .data
                .as_boolean_mut()
                .expect("Boolean data")
                .baked
                .as_mut()
                .expect("baked geometry")
                .source = source;
            let before = serde_json::to_value(&doc).expect("document snapshot");
            let pixels = flatten_pixels(&doc);
            let history = doc.history.undo_depth();
            if !nested {
                assert!(pixels.chunks_exact(4).any(|pixel| pixel[3] > 0));
                assert!(can_flatten(&doc, root));
                let operations = simple(&doc, root, Action::Flatten).expect("flatten baked root");
                roundtrip(&mut doc, operations, |doc| {
                    assert_eq!(flatten_pixels(doc), pixels);
                    assert!(doc.scene.children_of(Some(root)).is_empty());
                    let vector = doc
                        .scene
                        .get(root)
                        .expect("root")
                        .data
                        .as_vector()
                        .expect("vector");
                    assert_eq!(vector.local_size, Some([45., 25.]));
                    if stroke_outline {
                        assert!(vector.strokes.is_empty());
                    }
                });
                continue;
            }
            assert!(!can_flatten(&doc, root));
            let error = simple(&doc, root, Action::Flatten).expect_err("preserve baked artwork");
            assert!(error.to_string().contains("baked appearance"));
            assert_eq!(
                serde_json::to_value(&doc).expect("unchanged document"),
                before
            );
            assert_eq!(flatten_pixels(&doc), pixels);
            assert_eq!(doc.history.undo_depth(), history);

            doc.scene
                .set_transform(operand, Transform2D::translation(10., 0.))
                .expect("edit operand");
            assert!(can_flatten(&doc, root), "edited operands use live geometry");
        }
    }

    #[test]
    fn supported_group_and_boolean_flatten_preserve_pixels_and_undo() {
        for kind in ["group", "union", "subtract", "unfilled"] {
            let mut doc = Doc::new();
            let data = if kind == "group" {
                NodeData::Group(GroupNode::default())
            } else {
                NodeData::Boolean(fanta_doc::BooleanNode {
                    op: if kind == "subtract" {
                        fanta_doc::BooleanOp::Subtract
                    } else {
                        fanta_doc::BooleanOp::Union
                    },
                    fills: if kind == "unfilled" {
                        SmallVec::new()
                    } else {
                        smallvec::smallvec![Fill::solid(Color::BLACK)]
                    },
                    strokes: smallvec::smallvec![Stroke::solid(Color::WHITE, 2.)],
                    baked: None,
                })
            };
            let root = insert(&mut doc, CanvasNode::new(data), None);
            let mut first = rect();
            if let NodeData::Vector(vector) = &mut first.data {
                vector.local_size = Some([40., 20.]);
            }
            insert(&mut doc, first, Some(root));
            let mut second = rect();
            second.transform = Transform2D::translation(20., 10.);
            insert(&mut doc, second, Some(root));
            let pixels = flatten_pixels(&doc);
            assert!(can_flatten(&doc, root), "{kind}");
            let operations = simple(&doc, root, Action::Flatten).expect("flatten supported shape");
            roundtrip(&mut doc, operations, |doc| {
                assert_eq!(flatten_pixels(doc), pixels, "{kind}");
                assert!(doc.scene.children_of(Some(root)).is_empty());
            });
        }
    }

    #[test]
    fn vector_flatten_preserves_even_odd_holes_and_viewport_clip() {
        let mut doc = Doc::new();
        let mut vector = VectorNode::rect_solid(0., 0., 40., 40., Color::BLACK);
        vector
            .path
            .segments
            .extend(PathData::rect(10., 10., 20., 20.).segments);
        vector.path.fill_rule = fanta_doc::FillRule::EvenOdd;
        vector.local_size = Some([35., 35.]);
        let root = insert(&mut doc, CanvasNode::new(NodeData::Vector(vector)), None);
        let pixels = flatten_pixels(&doc);
        let operations = simple(&doc, root, Action::Flatten).expect("flatten clipped vector");
        roundtrip(&mut doc, operations, |doc| {
            assert_eq!(flatten_pixels(doc), pixels)
        });
    }

    fn master_instance(doc: &mut Doc, page: NodeId) -> (NodeId, NodeId, ComponentId) {
        let master = insert(
            doc,
            CanvasNode::new(NodeData::Group(GroupNode {
                clip_size: Some([40., 20.]),
                ..Default::default()
            })),
            Some(page),
        );
        insert(doc, rect(), Some(master));
        let component = ComponentId::new();
        doc.components
            .defs
            .insert(component, ComponentDef::new(component, master, "Button"));
        let instance = insert(
            doc,
            CanvasNode::new(NodeData::Instance(InstanceNode {
                component,
                overrides: Vec::new(),
                prop_values: Default::default(),
                derived: Vec::new(),
                local_size: [40., 20.],
            })),
            Some(page),
        );
        (master, instance, component)
    }

    #[test]
    fn layer_menu_delete_master_preserves_its_instances_and_undo_restores_definition() {
        let (mut doc, page, _) = fixture();
        let (master, instance, component) = master_instance(&mut doc, page);
        let operations = delete_layers(&doc, master).expect("delete");
        roundtrip(&mut doc, operations, |doc| {
            assert!(!doc.scene.contains(master));
            assert!(doc.components.def(component).is_none());
            assert!(matches!(
                doc.scene.get(instance).expect("instance").data,
                NodeData::Group(_)
            ));
        });
    }

    #[test]
    fn layer_menu_duplicate_master_creates_an_independent_definition() {
        let (mut doc, page, _) = fixture();
        let (master, _, _) = master_instance(&mut doc, page);
        let operations =
            crate::clipboard::duplicate_layer_operations(&doc, master).expect("duplicate");
        roundtrip(&mut doc, operations, |doc| {
            assert_eq!(doc.components.defs.len(), 2);
            assert!(
                doc.components
                    .defs
                    .values()
                    .all(|def| doc.scene.contains(def.root))
            );
        });
    }

    #[test]
    fn layer_menu_paste_to_replace_preserves_target_parent_position_and_undo() {
        let (mut doc, page, source) = fixture();
        doc.selection.replace_with([source]);
        let payload = crate::clipboard::CanvasClipboard::capture(&doc).expect("copy");
        let mut node = rect();
        node.transform = Transform2D::translation(150., 80.);
        let target = insert(&mut doc, node, Some(page));
        doc.selection.replace_with([target]);
        let operations =
            crate::clipboard::paste_to_replace_operations(&doc, target, &payload).expect("replace");
        roundtrip(&mut doc, operations, |doc| {
            assert!(!doc.scene.contains(target));
            let pasted = doc
                .scene
                .children_of(Some(page))
                .iter()
                .find(|id| **id != source)
                .expect("pasted");
            let bounds = doc.scene.world_bounds(*pasted).expect("bounds");
            assert_eq!((bounds.min_x, bounds.min_y), (150., 80.));
        });
    }

    #[test]
    fn layer_menu_style_paste_changes_appearance_without_geometry() {
        let (mut doc, _, id) = fixture();
        let transform = doc.scene.get(id).expect("node").transform;
        let mut source = rect();
        source.opacity = UnitInterval::new(0.4);
        source.transform = Transform2D::translation(100., 100.);
        let operations = paste_properties(&doc, id, &source).expect("paste properties");
        roundtrip(&mut doc, operations, |doc| {
            let node = doc.scene.get(id).expect("node");
            assert_eq!(node.transform, transform);
            assert_eq!(node.opacity, source.opacity);
        });
    }

    #[test]
    fn layer_menu_create_component_wraps_leaf_and_reset_instance_restores_size() {
        let (mut doc, page, id) = fixture();
        let operations = simple(&doc, id, Action::CreateComponent).expect("create component");
        roundtrip(&mut doc, operations, |doc| {
            assert_eq!(doc.components.defs.len(), 1)
        });
        let (_, instance, _) = master_instance(&mut doc, page);
        if let NodeData::Instance(value) = &mut doc.scene.get_mut(instance).expect("instance").data
        {
            value.local_size = [500., 800.];
        }
        let operations = simple(&doc, instance, Action::ResetInstance).expect("reset");
        roundtrip(&mut doc, operations, |doc| {
            assert!(
                matches!(&doc.scene.get(instance).expect("instance").data, NodeData::Instance(value) if value.local_size == [40.,20.])
            )
        });
    }
}

pub(crate) fn crop_rectangle(doc: &Doc, id: NodeId, crop: [f64; 4]) -> Result<Vec<Operation>> {
    ensure!(editable(doc, id), "The layer is locked");
    let [x, y, width, height] = crop;
    ensure!(
        crop.into_iter().all(f64::is_finite)
            && x >= 0.
            && y >= 0.
            && width > 0.
            && height > 0.
            && x + width <= 100.
            && y + height <= 100.,
        "Crop coordinates must be percentages inside the image, with positive width and height"
    );
    let node = doc.scene.get(id).context("Missing image")?;
    let NodeData::Bitmap(bitmap) = &node.data else {
        bail!("Choose an image layer");
    };
    let mut bitmap = bitmap.clone();
    bitmap.crop = Some(crop.map(|value| (value / 100.) as f32));
    let aspect = bitmap.natural_size[0].max(1) as f64 * width
        / (bitmap.natural_size[1].max(1) as f64 * height);
    bitmap.local_size[1] = bitmap.local_size[0] / aspect;
    Ok(vec![replace_data(node, NodeData::Bitmap(bitmap))])
}

pub(crate) fn created_roots(operations: &[Operation]) -> Vec<NodeId> {
    let created: std::collections::HashSet<_> = operations
        .iter()
        .filter_map(|operation| match operation {
            Operation::CreateNode { node } => Some(node.id),
            _ => None,
        })
        .collect();
    operations
        .iter()
        .filter_map(|operation| match operation {
            Operation::CreateNode { node }
                if !node.parent.is_some_and(|parent| created.contains(&parent)) =>
            {
                Some(node.id)
            }
            _ => None,
        })
        .collect()
}
