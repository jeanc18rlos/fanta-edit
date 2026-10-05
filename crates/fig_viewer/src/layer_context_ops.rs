//! Pure, reversible document commands used by the shared Layers menu.
use anyhow::{Context as _, Result, bail, ensure};
use fanta_doc::{
    AutoLayout, Bounds, CanvasNode, Doc, Fill, LayoutMode, NodeData, NodeId, Operation, PathData,
    Transform2D, VectorNode,
};
use fanta_gpui::layers::LayersPanelContextAction as Action;
use smallvec::SmallVec;

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

pub(crate) fn can_flatten(doc: &Doc, id: NodeId) -> bool {
    validate_flatten(doc, id).is_ok()
}

fn validate_flatten(doc: &Doc, id: NodeId) -> Result<()> {
    let node = doc.scene.get(id).context("Missing layer")?;
    ensure!(
        node.bindings.is_empty(),
        "Flatten cannot preserve variable bindings"
    );
    ensure!(
        !doc.scene
            .descendants_of(id)
            .any(|child| doc.is_component_root(child)),
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
        NodeData::Boolean(_) => validate_boolean_flatten_geometry(doc, id)?,
        _ => bail!("This layer cannot be flattened without changing its appearance"),
    }
    Ok(())
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
        NodeData::Boolean(_) => {}
        _ => bail!("Flatten cannot preserve this Boolean operand"),
    }
    for child in doc.scene.children_of(Some(id)) {
        validate_boolean_flatten_geometry(doc, *child)?;
    }
    Ok(())
}

fn flatten(doc: &Doc, id: NodeId) -> Result<Vec<Operation>> {
    validate_flatten(doc, id)?;
    let node = doc.scene.get(id).context("Missing layer")?;
    let outline = local_outline(doc, id)?;
    let mut path =
        PathData::from_svg_d(&outline.to_svg()).context("Could not serialize vector outline")?;
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
    let strokes = match &node.data {
        NodeData::Vector(value) => value.strokes.clone(),
        NodeData::Boolean(value) => value.strokes.clone(),
        NodeData::Group(value) => value.strokes.clone(),
        _ => SmallVec::new(),
    };
    operations.push(replace_data(
        node,
        NodeData::Vector(VectorNode {
            path,
            fills,
            strokes,
            local_size: match &node.data {
                NodeData::Vector(vector) => vector.local_size,
                _ => None,
            },
            ..Default::default()
        }),
    ));
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
    for node in doc
        .scene
        .roots()
        .iter()
        .flat_map(|id| doc.scene.descendants_of(*id))
        .filter_map(|id| doc.scene.get(id))
    {
        if removed.contains(&node.id) {
            continue;
        }
        if let NodeData::Instance(instance) = &node.data
            && components.contains(&instance.component)
        {
            let detach = crate::properties_ops::detach_instance_operations(doc, node.id);
            ensure!(
                !detach.is_empty(),
                "Cannot preserve an instance of this component"
            );
            operations.extend(detach);
        }
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
