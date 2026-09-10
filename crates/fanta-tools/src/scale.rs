//! Proportional scaling of selected layers and their contents.

use crate::context::ToolContext;
use crate::event::{Button, LogicalKey, ModifierKeys, PointerEvent, ToolEvent};
use crate::tool::{CursorHint, Tool, ToolResponse};
use fanta_canvas::{HitPrecision, ResizeHandle};
use fanta_doc::{
    Blur, BoundProp, Bounds, CanvasNode, Doc, Fill, ImageFitMode, NodeData, NodeFlags, NodeId,
    Operation, Shadow, Stroke, TextStyle, Transform2D, VariableId,
};
use glam::DVec2;
use smallvec::SmallVec;
use std::collections::BTreeMap;

#[derive(Debug, Default)]
pub struct ScaleTool {
    gesture: Option<ScaleGesture>,
}

#[derive(Debug)]
struct ScaleGesture {
    scene_id: u64,
    scope: Option<NodeId>,
    roots: Vec<NodeId>,
    nodes: Vec<ScaleNode>,
    frame: (Bounds, Transform2D),
    handle: ResizeHandle,
    press_world: DVec2,
    factor: f64,
}

#[derive(Debug)]
struct ScaleNode {
    id: NodeId,
    parent: Option<NodeId>,
    original: ScaleProperties,
    resolved_data: Option<NodeData>,
    root_transform: Option<(Transform2D, Transform2D)>,
}

#[derive(Debug, Clone, PartialEq)]
struct ScaleProperties {
    transform: Transform2D,
    data: NodeData,
    effects: SmallVec<[Shadow; 0]>,
    blurs: SmallVec<[Blur; 0]>,
    bindings: BTreeMap<BoundProp, VariableId>,
}

impl ScaleProperties {
    fn read(node: &CanvasNode) -> Self {
        Self {
            transform: node.transform,
            data: node.data.clone(),
            effects: node.effects.clone(),
            blurs: node.blurs.clone(),
            bindings: node.bindings.clone(),
        }
    }

    fn write(self, node: &mut CanvasNode) {
        node.transform = self.transform;
        node.data = self.data;
        node.effects = self.effects;
        node.blurs = self.blurs;
        node.bindings = self.bindings;
    }
}

impl ScaleTool {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn selection_frame(doc: &Doc, scope: Option<NodeId>) -> Option<(Bounds, Transform2D)> {
        let roots = selection_roots(doc, scope);
        if let [id] = roots.as_slice() {
            return Some((selection_bounds(doc, *id)?, doc.scene.world_transform(*id)?));
        }
        let mut bounds: Option<Bounds> = None;
        for id in roots {
            let next =
                selection_bounds(doc, id)?.try_transformed(&doc.scene.world_transform(id)?)?;
            bounds = Some(bounds.map_or(next, |bounds| bounds.union(&next)));
        }
        bounds.map(|bounds| (bounds, Transform2D::IDENTITY))
    }

    fn handle_at(&self, ctx: &ToolContext, screen: DVec2) -> Option<ResizeHandle> {
        let (local, world) = Self::selection_frame(ctx.doc, ctx.scope())?;
        fanta_canvas::handles::hit_test_resize_handle_oriented(
            local,
            &world,
            screen,
            ctx.viewport,
            ctx.screen_size,
            fanta_canvas::handles::DEFAULT_HANDLE_THRESHOLD,
        )
    }

    fn begin(&mut self, ctx: &ToolContext, handle: ResizeHandle, screen: DVec2) {
        let Some(frame) = Self::selection_frame(ctx.doc, ctx.scope()) else {
            return;
        };
        let roots = selection_roots(ctx.doc, ctx.scope());
        let mut nodes = Vec::new();
        for &root in &roots {
            let Some(root_world) = ctx.doc.scene.world_transform(root) else {
                return;
            };
            let parent = ctx.doc.scene.get(root).and_then(|node| node.parent);
            let parent_world = parent
                .and_then(|id| ctx.doc.scene.world_transform(id))
                .unwrap_or(Transform2D::IDENTITY);
            if !invertible(parent_world) {
                return;
            }
            for id in ctx.doc.scene.descendants_of(root) {
                let Some(node) = ctx.doc.scene.get(id) else {
                    return;
                };
                let original = ScaleProperties::read(node);
                let resolved_data = resolve_scale_data(ctx.doc, node);
                nodes.push(ScaleNode {
                    id,
                    parent: node.parent,
                    original,
                    resolved_data,
                    root_transform: (id == root).then_some((root_world, parent_world.inverse())),
                });
            }
        }
        self.gesture = Some(ScaleGesture {
            scene_id: ctx.doc.scene.instance_id(),
            scope: ctx.scope(),
            roots,
            nodes,
            frame,
            handle,
            press_world: ctx.screen_to_world(screen),
            factor: 1.,
        });
    }

    fn restore(&mut self, ctx: &mut ToolContext) {
        let Some(gesture) = self.gesture.take() else {
            return;
        };
        if gesture.scene_id != ctx.doc.scene.instance_id() {
            return;
        }
        for saved in gesture.nodes {
            if let Some(node) = ctx.doc.scene.get_mut(saved.id) {
                let transform = node.transform;
                let reparented = node.parent != saved.parent;
                saved.original.write(node);
                if reparented {
                    node.transform = transform;
                }
            }
        }
    }

    fn preview(&mut self, ctx: &mut ToolContext, screen: DVec2, modifiers: ModifierKeys) {
        let Some(gesture) = self.gesture.as_mut() else {
            return;
        };
        let (local, world) = gesture.frame;
        let pivot = world.transform_point(if modifiers.contains(ModifierKeys::ALT) {
            local.center()
        } else {
            gesture.handle.anchor_world(local)
        });
        let handle = world.transform_point(gesture.handle.handle_world(local));
        let ray = handle - pivot;
        if ray.length_squared() <= f64::EPSILON {
            return;
        }
        let desired = handle + ctx.screen_to_world(screen) - gesture.press_world;
        let factor = ((desired - pivot).dot(ray) / ray.length_squared()).clamp(0.001, 10_000.);
        if !factor.is_finite() {
            return;
        }
        let Some(properties) = gesture
            .nodes
            .iter()
            .map(|node| scaled_properties(node, factor, pivot))
            .collect::<Option<Vec<_>>>()
        else {
            return;
        };
        for (saved, properties) in gesture.nodes.iter().zip(properties) {
            if let Some(node) = ctx.doc.scene.get_mut(saved.id) {
                properties.write(node);
            }
        }
        gesture.factor = factor;
    }

    fn commit(&mut self, ctx: &mut ToolContext) {
        let Some(gesture) = self.gesture.take() else {
            return;
        };
        if (gesture.factor - 1.).abs() <= f64::EPSILON {
            self.gesture = Some(gesture);
            self.restore(ctx);
            return;
        }
        let mut operations = Vec::new();
        for saved in &gesture.nodes {
            let Some(node) = ctx.doc.scene.get(saved.id) else {
                self.gesture = Some(gesture);
                self.restore(ctx);
                return;
            };
            let final_state = ScaleProperties::read(node);
            let old = &saved.original;
            for (&property, &variable) in &old.bindings {
                if !final_state.bindings.contains_key(&property) {
                    operations.push(Operation::UnbindProperty {
                        node: saved.id,
                        prop: property,
                        variable,
                        old_data: Box::new(old.data.clone()),
                        new_data: Box::new(old.data.clone()),
                        old_opacity: node.opacity,
                        new_opacity: node.opacity,
                        old_flags: node.flags,
                        new_flags: node.flags,
                    });
                }
            }
            if old.transform != final_state.transform {
                operations.push(Operation::SetTransform {
                    id: saved.id,
                    old: old.transform,
                    new: final_state.transform,
                });
            }
            if old.data != final_state.data {
                operations.push(Operation::ReplaceData {
                    id: saved.id,
                    old: Box::new(old.data.clone()),
                    new: Box::new(final_state.data),
                });
            }
            if old.effects != final_state.effects {
                operations.push(Operation::SetEffects {
                    id: saved.id,
                    old: old.effects.clone(),
                    new: final_state.effects,
                });
            }
            if old.blurs != final_state.blurs {
                operations.push(Operation::SetBlurs {
                    id: saved.id,
                    old: old.blurs.clone(),
                    new: final_state.blurs,
                });
            }
        }
        for saved in &gesture.nodes {
            if let Some(node) = ctx.doc.scene.get_mut(saved.id) {
                saved.original.clone().write(node);
            }
        }
        ctx.doc.history.begin("Scale", &mut ctx.doc.scene);
        for operation in operations {
            if let Err(error) = ctx.doc.apply(operation) {
                if let Err(rollback) = ctx.doc.abort_transaction() {
                    tracing::error!(target: "fanta-tools.scale", "scale rollback failed: {rollback}");
                }
                tracing::warn!(target: "fanta-tools.scale", "scale commit failed: {error}");
                return;
            }
        }
        ctx.doc.history.commit(&mut ctx.doc.scene);
    }
}

impl Tool for ScaleTool {
    fn name(&self) -> &'static str {
        "scale"
    }

    fn handle_event(&mut self, ctx: &mut ToolContext, event: ToolEvent) -> ToolResponse {
        if self.gesture.as_ref().is_some_and(|gesture| {
            gesture.scene_id != ctx.doc.scene.instance_id()
                || gesture.scope != ctx.scope()
                || selection_roots(ctx.doc, ctx.scope()) != gesture.roots
                || gesture.nodes.iter().any(|saved| {
                    ctx.doc
                        .scene
                        .get(saved.id)
                        .is_none_or(|node| node.parent != saved.parent)
                })
        }) {
            self.restore(ctx);
            return ToolResponse::cursor(CursorHint::Default);
        }
        match event {
            ToolEvent::Pointer(PointerEvent::Press {
                screen,
                button: Button::Primary,
                modifiers,
                ..
            }) => {
                self.restore(ctx);
                let screen = DVec2::from(screen);
                if let Some(handle) = self.handle_at(ctx, screen) {
                    self.begin(ctx, handle, screen);
                } else {
                    let hit = fanta_canvas::hit_test(
                        &ctx.doc.scene,
                        ctx.screen_to_world(screen),
                        HitPrecision::Bounds,
                        ctx.scope(),
                    )
                    .map(|id| selection_target(ctx.doc, ctx.scope(), id, modifiers))
                    .filter(|&id| eligible_root(ctx.doc, ctx.scope(), id));
                    match hit {
                        Some(id) if modifiers.extend_selection() => ctx.doc.selection.toggle(id),
                        Some(id) => ctx.doc.selection.select_only(id),
                        None if !modifiers.extend_selection() => ctx.doc.selection.clear(),
                        None => {}
                    }
                }
            }
            ToolEvent::Pointer(PointerEvent::Move { screen, modifiers }) => {
                let screen = DVec2::from(screen);
                if self.gesture.is_some() {
                    self.preview(ctx, screen, modifiers);
                }
                return ToolResponse::cursor(
                    if self.gesture.is_some() || self.handle_at(ctx, screen).is_some() {
                        CursorHint::Move
                    } else {
                        CursorHint::Default
                    },
                );
            }
            ToolEvent::Pointer(PointerEvent::Release {
                button: Button::Primary,
                screen,
                modifiers,
            }) => {
                self.preview(ctx, DVec2::from(screen), modifiers);
                self.commit(ctx);
            }
            ToolEvent::Key(key) if key.key == LogicalKey::Escape => {
                if self.gesture.is_some() {
                    self.restore(ctx);
                } else {
                    return ToolResponse::exit();
                }
            }
            _ => {}
        }
        ToolResponse::cursor(CursorHint::Default)
    }

    fn deactivate(&mut self, ctx: &mut ToolContext) {
        self.restore(ctx);
    }
}

fn invertible(transform: Transform2D) -> bool {
    transform.is_finite()
        && transform.0.matrix2.determinant().is_finite()
        && transform.0.matrix2.determinant().abs() > f64::EPSILON
}

fn eligible_root(doc: &Doc, scope: Option<NodeId>, id: NodeId) -> bool {
    let Some(node) = doc.scene.get(id) else {
        return false;
    };
    !node.flags.intersects(NodeFlags::LOCKED | NodeFlags::HIDDEN)
        && !doc.scene.ancestors_of(id).any(|ancestor| {
            ancestor
                .flags
                .intersects(NodeFlags::LOCKED | NodeFlags::HIDDEN)
                || matches!(ancestor.data, NodeData::Instance(_) | NodeData::Boolean(_))
        })
        && scope.is_none_or(|root| {
            (id == root && node.parent.is_some())
                || doc
                    .scene
                    .ancestors_of(id)
                    .any(|ancestor| ancestor.id == root)
        })
        && doc.scene.world_transform(id).is_some_and(invertible)
}

fn selection_roots(doc: &Doc, scope: Option<NodeId>) -> Vec<NodeId> {
    doc.selection
        .iter()
        .copied()
        .filter(|&id| eligible_root(doc, scope, id))
        .filter(|&id| {
            !doc.scene.ancestors_of(id).any(|ancestor| {
                doc.selection.contains(ancestor.id) && eligible_root(doc, scope, ancestor.id)
            })
        })
        .collect()
}

fn selection_target(
    doc: &Doc,
    scope: Option<NodeId>,
    leaf: NodeId,
    modifiers: ModifierKeys,
) -> NodeId {
    if !modifiers.extend_selection()
        && let Some(selected) = std::iter::once(leaf)
            .chain(doc.scene.ancestors_of(leaf).map(|node| node.id))
            .find(|&id| doc.selection.contains(id))
    {
        return selected;
    }
    let container = scope.or_else(|| doc.scene.ancestors_of(leaf).last().map(|node| node.id));
    std::iter::once(leaf)
        .chain(doc.scene.ancestors_of(leaf).map(|node| node.id))
        .find(|&id| {
            doc.scene
                .get(id)
                .is_some_and(|node| node.parent == container)
        })
        .unwrap_or(leaf)
}

fn resolve_scale_data(doc: &Doc, node: &CanvasNode) -> Option<NodeData> {
    if uses_transform_scale(&node.data) {
        return None;
    }
    let mut resolved_data = None;
    for (&property, &variable) in &node.bindings {
        if is_scaled_binding(property)
            && let Some(value) = fanta_doc::resolve::resolve_bound_value(
                &doc.variables,
                &doc.scene,
                node.id,
                &doc.active_modes,
                variable,
            )
        {
            let resolved = resolved_data.get_or_insert_with(|| node.data.clone());
            let mut resolved_node = CanvasNode::new(resolved.clone());
            property.apply_resolved(&mut resolved_node, value);
            *resolved = resolved_node.data;
        }
    }
    resolved_data
}

fn selection_bounds(doc: &Doc, id: NodeId) -> Option<Bounds> {
    let node = doc.scene.get(id)?;
    let NodeData::Group(group) = &node.data else {
        return doc.scene.local_bounds(id);
    };
    // Frames render variable-bound dimensions from a scratch node, not the
    // stored literals. The handle frame and scaling pivot must use that box.
    let resolved_data = resolve_scale_data(doc, node);
    let group = resolved_data
        .as_ref()
        .and_then(NodeData::as_group)
        .unwrap_or(group);
    if let Some([width, height]) = group.clip_size.or(group.local_size) {
        return Some(Bounds::from_xywh(0., 0., width, height));
    }
    if doc.variables.variables.is_empty() {
        return doc.scene.local_bounds(id);
    }
    let mut bounds: Option<Bounds> = None;
    for &child in doc.scene.children_of(Some(id)) {
        if let Some(child_bounds) = selection_bounds(doc, child)
            .and_then(|bounds| bounds.try_transformed(&doc.scene.get(child)?.transform))
        {
            bounds = Some(bounds.map_or(child_bounds, |bounds| bounds.union(&child_bounds)));
        }
    }
    bounds
}

fn uses_transform_scale(data: &NodeData) -> bool {
    matches!(
        data,
        NodeData::Instance(_) | NodeData::Embed(_) | NodeData::NodeGraph(_) | NodeData::Model3d(_)
    ) || matches!(data, NodeData::Bitmap(bitmap) if bitmap.fit == ImageFitMode::Tile)
}

fn is_scaled_binding(property: BoundProp) -> bool {
    matches!(
        property,
        BoundProp::StrokeWidth { .. }
            | BoundProp::CornerRadius
            | BoundProp::TextStyle
            | BoundProp::ClipWidth
            | BoundProp::ClipHeight
    )
}

fn scaled_properties(saved: &ScaleNode, factor: f64, pivot: DVec2) -> Option<ScaleProperties> {
    let mut properties = saved.original.clone();
    let transform_only = uses_transform_scale(&properties.data);
    let scale = Transform2D::translation(-pivot.x, -pivot.y)
        .then(&Transform2D::scale(factor))
        .then(&Transform2D::translation(pivot.x, pivot.y));
    properties.transform = if let Some((world, parent_inverse)) = saved.root_transform {
        let transform = if transform_only {
            world
        } else {
            Transform2D::scale(1. / factor).then(&world)
        };
        transform.then(&scale).then(&parent_inverse)
    } else {
        let mut transform = saved.original.transform;
        transform.0.translation *= factor;
        if transform_only {
            transform.0.matrix2 *= factor;
        }
        transform
    };
    if !properties.transform.is_finite() {
        return None;
    }
    if !transform_only {
        if let Some(resolved) = &saved.resolved_data {
            properties.data = resolved.clone();
        }
        properties
            .bindings
            .retain(|property, _| !is_scaled_binding(*property));
        scale_data(&mut properties.data, factor)?;
        for shadow in &mut properties.effects {
            scale_number(&mut shadow.blur, factor)?;
            scale_number(&mut shadow.spread, factor)?;
            scale_array(&mut shadow.offset, factor)?;
        }
        for blur in &mut properties.blurs {
            scale_number(&mut blur.radius, factor)?;
        }
    }
    Some(properties)
}

fn scale_number(value: &mut f64, factor: f64) -> Option<()> {
    let scaled = *value * factor;
    if !scaled.is_finite() {
        return None;
    }
    *value = scaled;
    Some(())
}

fn scale_array<const N: usize>(values: &mut [f64; N], factor: f64) -> Option<()> {
    for value in values {
        scale_number(value, factor)?;
    }
    Some(())
}

fn scale_optional(value: &mut Option<f64>, factor: f64) -> Option<()> {
    if let Some(value) = value {
        scale_number(value, factor)?;
    }
    Some(())
}

fn scale_size(size: &mut Option<[f64; 2]>, factor: f64) -> Option<()> {
    if let Some(size) = size {
        scale_array(size, factor)?;
    }
    Some(())
}

fn scale_paint(fill: &mut Fill, factor: f64) -> Option<()> {
    if let Fill::Image {
        mode: ImageFitMode::Tile,
        scale,
        ..
    } = fill
    {
        let scaled = f64::from(scale.unwrap_or(1.)) * factor;
        if !scaled.is_finite() || scaled > f32::MAX as f64 {
            return None;
        }
        *scale = Some(scaled as f32);
    }
    Some(())
}

fn scale_strokes(strokes: &mut [Stroke], factor: f64) -> Option<()> {
    for stroke in strokes {
        scale_number(&mut stroke.width, factor)?;
        for dash in &mut stroke.dash {
            scale_number(dash, factor)?;
        }
        if let Some(widths) = &mut stroke.per_side {
            scale_array(widths, factor)?;
        }
        scale_paint(&mut stroke.paint, factor)?;
    }
    Some(())
}

fn scale_text(style: &mut TextStyle, factor: f64) -> Option<()> {
    scale_number(&mut style.size_px, factor)?;
    scale_number(&mut style.letter_spacing, factor)
}

fn scale_data(data: &mut NodeData, factor: f64) -> Option<()> {
    match data {
        NodeData::Vector(vector) => {
            let mut finite = true;
            vector.path.map_points_mut(|point| {
                let scaled = [point[0] * factor, point[1] * factor];
                finite &= scaled.into_iter().all(f64::is_finite);
                scaled
            });
            if !finite {
                return None;
            }
            scale_size(&mut vector.local_size, factor)?;
            scale_optional(&mut vector.corner_radius, factor)?;
            if let Some(radii) = &mut vector.corner_radii {
                scale_array(radii, factor)?;
            }
            for fill in &mut vector.fills {
                scale_paint(fill, factor)?;
            }
            scale_strokes(&mut vector.strokes, factor)?;
        }
        NodeData::Group(group) => {
            scale_size(&mut group.local_size, factor)?;
            scale_size(&mut group.clip_size, factor)?;
            scale_optional(&mut group.corner_radius, factor)?;
            if let Some(radii) = &mut group.corner_radii {
                scale_array(radii, factor)?;
            }
            scale_size(&mut group.scroll_offset, factor)?;
            if let Some(fill) = &mut group.background {
                scale_paint(fill, factor)?;
            }
            for fill in &mut group.background_fills {
                scale_paint(fill, factor)?;
            }
            scale_strokes(&mut group.strokes, factor)?;
            if let Some(layout) = &mut group.auto_layout {
                scale_number(&mut layout.spacing, factor)?;
                scale_number(&mut layout.counter_spacing, factor)?;
                scale_array(&mut layout.padding, factor)?;
                for value in layout.min_size.iter_mut().chain(&mut layout.max_size) {
                    scale_optional(value, factor)?;
                }
            }
        }
        NodeData::Text(text) => {
            scale_array(&mut text.local_size, factor)?;
            scale_text(&mut text.style, factor)?;
            for run in &mut text.style_runs {
                scale_text(&mut run.style, factor)?;
            }
            scale_number(&mut text.paragraph_spacing, factor)?;
            scale_number(&mut text.paragraph_indent, factor)?;
        }
        NodeData::TextPath(text_path) => {
            let mut finite = true;
            text_path.path.map_points_mut(|point| {
                let scaled = [point[0] * factor, point[1] * factor];
                finite &= scaled.into_iter().all(f64::is_finite);
                scaled
            });
            if !finite {
                return None;
            }
            scale_text(&mut text_path.style, factor)?;
            for run in &mut text_path.style_runs {
                scale_text(&mut run.style, factor)?;
            }
        }
        NodeData::Boolean(boolean) => {
            for fill in &mut boolean.fills {
                scale_paint(fill, factor)?;
            }
            scale_strokes(&mut boolean.strokes, factor)?;
        }
        NodeData::Bitmap(bitmap) => scale_array(&mut bitmap.local_size, factor)?,
        NodeData::Video(video) => scale_array(&mut video.local_size, factor)?,
        NodeData::Audio(audio) => scale_array(&mut audio.local_size, factor)?,
        NodeData::AiArtifact(artifact) => scale_array(&mut artifact.local_size, factor)?,
        NodeData::Instance(_)
        | NodeData::Embed(_)
        | NodeData::NodeGraph(_)
        | NodeData::Model3d(_) => {}
    }
    Some(())
}

#[cfg(test)]
mod tests {
    use crate::scale::{ScaleTool, scale_data};
    use crate::{
        Button, KeyEvent, LogicalKey, ModifierKeys, PointerEvent, Tool, ToolContext, ToolEvent,
    };
    use fanta_canvas::{ResizeHandle, SnapEngine};
    use fanta_doc::{
        AutoLayout, Blur, BoundProp, Bounds, CanvasNode, Color, ComponentDef, ComponentId,
        ConstraintH, ConstraintV, Constraints, Doc, GroupNode, InstanceNode, Mode, ModeId,
        NodeData, NodeFlags, NodeId, Operation, Shadow, ShadowKind, Stroke, TextNode, TextPathNode,
        TextPathStart, TextStyle, TextStyleRun, Transform2D, VarValue, Variable,
        VariableCollection, VariableCollectionId, VariableId, VariableType, VectorNode, Viewport,
    };
    use glam::DVec2;
    use std::collections::BTreeMap;

    fn insert(doc: &mut Doc, mut node: CanvasNode, parent: Option<NodeId>) -> NodeId {
        node.parent = parent;
        node.index = doc.scene.next_child_index(parent);
        let id = node.id;
        doc.apply(Operation::create_node(node))
            .expect("create fixture node");
        id
    }

    fn rectangle(width: f64, height: f64) -> CanvasNode {
        CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
            0.,
            0.,
            width,
            height,
            Color::BLACK,
        )))
    }

    fn context<'a>(doc: &'a mut Doc, viewport: &'a mut Viewport) -> ToolContext<'a> {
        ToolContext::new(doc, viewport, SnapEngine::default(), DVec2::new(800., 600.))
    }

    fn press(tool: &mut ScaleTool, ctx: &mut ToolContext, world: DVec2, modifiers: ModifierKeys) {
        let screen = ctx.world_to_screen(world).to_array();
        tool.handle_event(
            ctx,
            ToolEvent::Pointer(PointerEvent::Press {
                screen,
                button: Button::Primary,
                modifiers,
                count: 1,
            }),
        );
    }

    fn move_to(tool: &mut ScaleTool, ctx: &mut ToolContext, world: DVec2, modifiers: ModifierKeys) {
        let screen = ctx.world_to_screen(world).to_array();
        tool.handle_event(
            ctx,
            ToolEvent::Pointer(PointerEvent::Move { screen, modifiers }),
        );
    }

    fn release(tool: &mut ScaleTool, ctx: &mut ToolContext, world: DVec2, modifiers: ModifierKeys) {
        let screen = ctx.world_to_screen(world).to_array();
        tool.handle_event(
            ctx,
            ToolEvent::Pointer(PointerEvent::Release {
                screen,
                button: Button::Primary,
                modifiers,
            }),
        );
    }

    fn drag(
        tool: &mut ScaleTool,
        ctx: &mut ToolContext,
        from: DVec2,
        to: DVec2,
        modifiers: ModifierKeys,
    ) {
        press(tool, ctx, from, modifiers);
        move_to(tool, ctx, to, modifiers);
        release(tool, ctx, to, modifiers);
    }

    fn node(doc: &Doc, id: NodeId) -> &CanvasNode {
        doc.scene.get(id).expect("fixture node exists")
    }

    fn near(actual: f64, expected: f64) {
        assert!(
            (actual - expected).abs() < 1e-8,
            "expected {expected}, got {actual}"
        );
    }

    fn near_point(actual: DVec2, expected: DVec2) {
        near(actual.x, expected.x);
        near(actual.y, expected.y);
    }

    #[test]
    fn scale_changes_vector_geometry_and_dimensional_styles_as_one_undo_step() {
        let mut doc = Doc::new();
        let mut shape = rectangle(100., 50.);
        let vector = shape.data.as_vector_mut().expect("vector");
        vector.local_size = Some([100., 50.]);
        vector.corner_radius = Some(6.);
        vector.corner_radii = Some([1., 2., 3., 4.]);
        vector.corner_smoothing = 0.4;
        let mut stroke = Stroke::solid(Color::BLACK, 3.);
        stroke.dash = vec![4., 2.];
        stroke.per_side = Some([1., 2., 3., 4.]);
        vector.strokes.push(stroke);
        shape.effects.push(Shadow {
            kind: ShadowKind::Inner,
            color: Color::BLACK,
            blur: 7.,
            spread: 2.,
            offset: [3., -4.],
            show_behind_node: false,
        });
        shape.blurs.push(Blur::background(5.));
        let id = insert(&mut doc, shape, None);
        let original = node(&doc, id).clone();
        doc.selection.select_only(id);
        doc.history = Default::default();
        let mut viewport = Viewport::default();
        let mut ctx = context(&mut doc, &mut viewport);
        let mut tool = ScaleTool::new();
        let handle = DVec2::new(100., 50.);
        press(&mut tool, &mut ctx, handle, ModifierKeys::empty());
        move_to(&mut tool, &mut ctx, handle * 2., ModifierKeys::empty());
        let scaled = node(ctx.doc, id);
        let vector = scaled.data.as_vector().expect("vector");
        assert_eq!(vector.local_size, Some([200., 100.]));
        let bounds = vector.path.rough_bounds().expect("scaled path bounds");
        near(bounds.max_x, 200.);
        near(bounds.max_y, 100.);
        assert_eq!(vector.corner_radius, Some(12.));
        assert_eq!(vector.corner_radii, Some([2., 4., 6., 8.]));
        assert_eq!(vector.corner_smoothing, 0.4);
        let stroke = vector.strokes.first().expect("stroke");
        assert_eq!(stroke.width, 6.);
        assert_eq!(stroke.dash, [8., 4.]);
        assert_eq!(stroke.per_side, Some([2., 4., 6., 8.]));
        assert_eq!(stroke.miter_limit, 4.);
        assert_eq!(scaled.effects.first().expect("shadow").blur, 14.);
        assert_eq!(scaled.effects.first().expect("shadow").spread, 4.);
        assert_eq!(scaled.effects.first().expect("shadow").offset, [6., -8.]);
        assert_eq!(scaled.blurs.first().expect("blur").radius, 10.);
        assert_eq!(scaled.transform, original.transform);
        assert_eq!(
            ctx.doc.history.undo_depth(),
            0,
            "preview is not a history entry"
        );
        release(&mut tool, &mut ctx, handle * 2., ModifierKeys::empty());
        let final_node = node(ctx.doc, id).clone();
        assert_eq!(ctx.doc.history.undo_depth(), 1);
        assert_eq!(ctx.doc.history.next_undo_label(), Some("Scale"));
        assert!(ctx.doc.undo().expect("undo scale"));
        assert_eq!(node(ctx.doc, id).data, original.data);
        assert_eq!(node(ctx.doc, id).effects, original.effects);
        assert_eq!(node(ctx.doc, id).blurs, original.blurs);
        assert_eq!(node(ctx.doc, id).transform, original.transform);
        assert!(!ctx.doc.undo().expect("no second scale step"));
        assert!(ctx.doc.redo().expect("redo scale"));
        assert_eq!(node(ctx.doc, id).data, final_node.data);
        assert_eq!(node(ctx.doc, id).effects, final_node.effects);
        assert_eq!(node(ctx.doc, id).blurs, final_node.blurs);
    }

    #[test]
    fn scale_group_scales_text_and_layout_once_even_when_child_is_also_selected() {
        for locked in [false, true] {
            let mut doc = Doc::new();
            let group = insert(
                &mut doc,
                CanvasNode::new(NodeData::Group(GroupNode {
                    local_size: Some([200., 100.]),
                    clip_size: Some([200., 100.]),
                    corner_radius: Some(8.),
                    scroll_offset: Some([2., 3.]),
                    auto_layout: Some(AutoLayout {
                        spacing: 10.,
                        counter_spacing: 12.,
                        padding: [4., 6., 8., 10.],
                        min_size: [Some(50.), None],
                        max_size: [None, Some(200.)],
                        ..Default::default()
                    }),
                    ..Default::default()
                })),
                None,
            );
            let mut text = TextNode::new("Bold normal", 80., 30.);
            text.style.size_px = 20.;
            text.style.letter_spacing = 1.5;
            text.style.line_height = 1.4;
            text.style_runs.push(TextStyleRun {
                start: 0,
                end: 4,
                style: TextStyle {
                    size_px: 24.,
                    weight: 700,
                    letter_spacing: -0.5,
                    ..Default::default()
                },
            });
            text.paragraph_spacing = 9.;
            text.paragraph_indent = 5.;
            let mut child = CanvasNode::new(NodeData::Text(text));
            child.transform = Transform2D::translation(10., 20.);
            child.constraints = Some(Constraints {
                horizontal: ConstraintH::Right,
                vertical: ConstraintV::Bottom,
            });
            if locked {
                child.flags.insert(NodeFlags::LOCKED);
            }
            let child_id = insert(&mut doc, child, Some(group));
            let original_child = node(&doc, child_id).clone();
            doc.selection.select_only(group);
            doc.selection.toggle(child_id);
            doc.history = Default::default();
            let mut viewport = Viewport::default();
            let mut ctx = context(&mut doc, &mut viewport);
            drag(
                &mut ScaleTool::new(),
                &mut ctx,
                DVec2::new(200., 100.),
                DVec2::new(400., 200.),
                ModifierKeys::empty(),
            );
            let group_data = node(ctx.doc, group).data.as_group().expect("group");
            assert_eq!(group_data.clip_size, Some([400., 200.]));
            assert_eq!(group_data.corner_radius, Some(16.));
            assert_eq!(group_data.scroll_offset, Some([4., 6.]));
            let layout = group_data.auto_layout.as_ref().expect("layout");
            assert_eq!(layout.spacing, 20.);
            assert_eq!(layout.counter_spacing, 24.);
            assert_eq!(layout.padding, [8., 12., 16., 20.]);
            assert_eq!(layout.min_size, [Some(100.), None]);
            assert_eq!(layout.max_size, [None, Some(400.)]);
            let scaled = node(ctx.doc, child_id);
            let NodeData::Text(text) = &scaled.data else {
                panic!("text remains text")
            };
            assert_eq!(text.local_size, [160., 60.]);
            assert_eq!(text.style.size_px, 40.);
            assert_eq!(text.style.letter_spacing, 3.);
            assert_eq!(text.style.line_height, 1.4);
            let run = text.style_runs.first().expect("rich text run");
            assert_eq!((run.start, run.end, run.style.weight), (0, 4, 700));
            assert_eq!(run.style.size_px, 48.);
            assert_eq!(run.style.letter_spacing, -1.);
            assert_eq!(text.paragraph_spacing, 18.);
            assert_eq!(text.paragraph_indent, 10.);
            assert_eq!(scaled.transform, Transform2D::translation(20., 40.));
            assert_eq!(scaled.constraints, original_child.constraints);
            assert_eq!(scaled.flags, original_child.flags);
            assert_eq!(ctx.doc.history.undo_depth(), 1);
            assert!(ctx.doc.undo().expect("undo group scale"));
            assert_eq!(node(ctx.doc, child_id).data, original_child.data);
            assert_eq!(node(ctx.doc, child_id).transform, original_child.transform);
        }
    }

    #[test]
    fn scale_text_path_scales_baseline_and_typography_without_moving_its_start() {
        let mut path = fanta_doc::PathData::new();
        path.move_to(1., 2.)
            .quad_to(3., 4., 5., 6.)
            .line_to(8., 10.);
        let mut text_path = TextPathNode::new(path, "Curve");
        text_path.style.size_px = 20.;
        text_path.style.letter_spacing = 1.5;
        text_path.style_runs.push(TextStyleRun {
            start: 0,
            end: 5,
            style: TextStyle {
                size_px: 24.,
                letter_spacing: -0.5,
                ..Default::default()
            },
        });
        text_path.start = TextPathStart::new(1, 0.25).expect("valid path start");
        let mut data = NodeData::TextPath(text_path);

        scale_data(&mut data, 2.).expect("text path scales");
        let text_path = data.as_text_path().expect("text path remains text path");
        assert_eq!(
            text_path.start,
            TextPathStart::new(1, 0.25).expect("expected path start")
        );
        assert_eq!(text_path.style.size_px, 40.);
        assert_eq!(text_path.style.letter_spacing, 3.);
        let run = text_path.style_runs.first().expect("rich text run");
        assert_eq!(run.style.size_px, 48.);
        assert_eq!(run.style.letter_spacing, -1.);
        let bounds = text_path.path.rough_bounds().expect("scaled path bounds");
        assert_eq!([bounds.min_x, bounds.min_y], [2., 4.]);
        assert_eq!([bounds.max_x, bounds.max_y], [16., 20.]);
    }

    #[test]
    fn scale_multi_selection_uses_a_shared_pivot_without_compounding_moves() {
        let mut doc = Doc::new();
        let first = insert(&mut doc, rectangle(100., 50.), None);
        let mut second = rectangle(100., 50.);
        second.transform = Transform2D::translation(200., 100.);
        let second = insert(&mut doc, second, None);
        doc.selection.select_only(first);
        doc.selection.toggle(second);
        doc.history = Default::default();
        let mut viewport = Viewport {
            zoom: 2.,
            ..Default::default()
        };
        let mut ctx = context(&mut doc, &mut viewport);
        let mut tool = ScaleTool::new();
        let handle = DVec2::new(300., 150.);
        press(&mut tool, &mut ctx, handle, ModifierKeys::empty());
        move_to(&mut tool, &mut ctx, handle * 1.5, ModifierKeys::empty());
        move_to(&mut tool, &mut ctx, handle * 2., ModifierKeys::empty());
        release(&mut tool, &mut ctx, handle * 2., ModifierKeys::empty());
        let first_bounds = ctx.doc.scene.world_bounds(first).expect("first bounds");
        let second_bounds = ctx.doc.scene.world_bounds(second).expect("second bounds");
        near(first_bounds.max_x, 200.);
        near(first_bounds.max_y, 100.);
        near(second_bounds.min_x, 400.);
        near(second_bounds.min_y, 200.);
        near(second_bounds.max_x, 600.);
        assert_eq!(ctx.doc.history.undo_depth(), 1);
        assert!(ctx.doc.undo().expect("undo both"));
        near(
            ctx.doc.scene.world_bounds(second).expect("bounds").min_x,
            200.,
        );
    }

    #[test]
    fn scale_preserves_rotation_and_parent_skew_in_world_space() {
        for zoom in [0.5, 3.] {
            let mut doc = Doc::new();
            let mut parent = CanvasNode::new(NodeData::Group(GroupNode::default()));
            parent.transform = Transform2D::from_components([2., 0.5, 0.3, 0.7, 40., -20.]);
            let parent_id = insert(&mut doc, parent, None);
            let mut child = rectangle(100., 60.);
            child.transform = Transform2D::rotation(0.3).then(&Transform2D::translation(20., 30.));
            let child_id = insert(&mut doc, child, Some(parent_id));
            let before = node(&doc, child_id).transform;
            let before_world = doc
                .scene
                .world_transform(child_id)
                .expect("world transform");
            let pivot = before_world.transform_point(DVec2::ZERO);
            let handle = before_world.transform_point(DVec2::new(100., 60.));
            doc.selection.select_only(child_id);
            doc.history = Default::default();
            let mut viewport = Viewport {
                zoom,
                ..Default::default()
            };
            let mut ctx = context(&mut doc, &mut viewport);
            drag(
                &mut ScaleTool::new(),
                &mut ctx,
                handle,
                pivot + (handle - pivot) * 2.,
                ModifierKeys::empty(),
            );
            let after = node(ctx.doc, child_id).transform;
            for (actual, expected) in after
                .to_components()
                .into_iter()
                .zip(before.to_components())
            {
                near(actual, expected);
            }
            let after_world = ctx
                .doc
                .scene
                .world_transform(child_id)
                .expect("new world transform");
            near_point(
                after_world.transform_point(DVec2::new(200., 120.)),
                pivot + (handle - pivot) * 2.,
            );
            let vector = node(ctx.doc, child_id).data.as_vector().expect("vector");
            near(vector.path.rough_bounds().expect("path").max_x, 200.);
        }
    }

    #[test]
    fn scale_alt_keeps_the_center_fixed_and_crossing_pivot_never_flips() {
        let mut doc = Doc::new();
        let id = insert(&mut doc, rectangle(100., 50.), None);
        doc.selection.select_only(id);
        doc.history = Default::default();
        let mut viewport = Viewport::default();
        let mut ctx = context(&mut doc, &mut viewport);
        let mut tool = ScaleTool::new();
        drag(
            &mut tool,
            &mut ctx,
            DVec2::new(100., 50.),
            DVec2::new(150., 75.),
            ModifierKeys::ALT,
        );
        let bounds = ctx.doc.scene.world_bounds(id).expect("bounds");
        near_point(bounds.center(), DVec2::new(50., 25.));
        near(bounds.width(), 200.);
        assert!(ctx.doc.undo().expect("undo centered scale"));
        drag(
            &mut tool,
            &mut ctx,
            DVec2::new(100., 50.),
            DVec2::new(-100., -50.),
            ModifierKeys::empty(),
        );
        let bounds = ctx.doc.scene.world_bounds(id).expect("positive bounds");
        assert!(bounds.width() > 0. && bounds.width() < 1.);
        assert!(node(ctx.doc, id).transform.0.matrix2.determinant() > 0.);
    }

    #[test]
    fn scale_cancel_and_tool_switch_restore_geometry_styles_and_history() {
        for deactivate in [false, true] {
            let mut doc = Doc::new();
            let mut text = TextNode::new("Editable", 100., 50.);
            text.style.size_px = 18.;
            let id = insert(&mut doc, CanvasNode::new(NodeData::Text(text)), None);
            let original = node(&doc, id).clone();
            doc.selection.select_only(id);
            doc.history = Default::default();
            let mut viewport = Viewport::default();
            let mut ctx = context(&mut doc, &mut viewport);
            let mut tool = ScaleTool::new();
            press(
                &mut tool,
                &mut ctx,
                DVec2::new(100., 50.),
                ModifierKeys::empty(),
            );
            move_to(
                &mut tool,
                &mut ctx,
                DVec2::new(200., 100.),
                ModifierKeys::empty(),
            );
            assert_ne!(
                node(ctx.doc, id).data,
                original.data,
                "preview scales before cancellation"
            );
            if deactivate {
                tool.deactivate(&mut ctx);
            } else {
                tool.handle_event(
                    &mut ctx,
                    ToolEvent::Key(KeyEvent::press(LogicalKey::Escape)),
                );
            }
            assert_eq!(node(ctx.doc, id).data, original.data);
            assert_eq!(node(ctx.doc, id).transform, original.transform);
            assert_eq!(ctx.doc.history.undo_depth(), 0);
            release(
                &mut tool,
                &mut ctx,
                DVec2::new(200., 100.),
                ModifierKeys::empty(),
            );
            assert_eq!(ctx.doc.history.undo_depth(), 0);
        }
    }

    #[test]
    fn scale_click_selects_a_container_and_preserves_an_already_selected_child() {
        let mut doc = Doc::new();
        let page = insert(
            &mut doc,
            CanvasNode::new(NodeData::Group(GroupNode::default())),
            None,
        );
        doc.add_page(page);
        doc.set_active_page(Some(page));
        let group = insert(
            &mut doc,
            CanvasNode::new(NodeData::Group(GroupNode {
                clip_size: Some([200., 100.]),
                ..Default::default()
            })),
            Some(page),
        );
        let child = insert(&mut doc, rectangle(100., 50.), Some(group));
        doc.history = Default::default();
        let mut viewport = Viewport::default();
        let mut ctx = context(&mut doc, &mut viewport);
        let mut tool = ScaleTool::new();
        press(
            &mut tool,
            &mut ctx,
            DVec2::new(40., 20.),
            ModifierKeys::empty(),
        );
        release(
            &mut tool,
            &mut ctx,
            DVec2::new(40., 20.),
            ModifierKeys::empty(),
        );
        assert_eq!(
            ctx.doc.selection.iter().copied().collect::<Vec<_>>(),
            [group]
        );
        ctx.doc.selection.select_only(child);
        press(
            &mut tool,
            &mut ctx,
            DVec2::new(40., 20.),
            ModifierKeys::empty(),
        );
        release(
            &mut tool,
            &mut ctx,
            DVec2::new(40., 20.),
            ModifierKeys::empty(),
        );
        assert_eq!(
            ctx.doc.selection.iter().copied().collect::<Vec<_>>(),
            [child]
        );
        assert_eq!(ctx.doc.history.undo_depth(), 0);
    }

    #[test]
    fn scale_rejects_locked_hidden_out_of_scope_and_singular_selections() {
        for case in 0..5 {
            let mut doc = Doc::new();
            let parent = insert(
                &mut doc,
                CanvasNode::new(NodeData::Group(GroupNode::default())),
                None,
            );
            let id = insert(&mut doc, rectangle(100., 50.), Some(parent));
            let other = insert(
                &mut doc,
                CanvasNode::new(NodeData::Group(GroupNode::default())),
                None,
            );
            match case {
                0 => {
                    doc.scene
                        .get_mut(id)
                        .expect("node")
                        .flags
                        .insert(NodeFlags::LOCKED);
                }
                1 => {
                    doc.scene
                        .get_mut(id)
                        .expect("node")
                        .flags
                        .insert(NodeFlags::HIDDEN);
                }
                2 => {
                    doc.scene
                        .get_mut(parent)
                        .expect("parent")
                        .flags
                        .insert(NodeFlags::LOCKED);
                }
                3 => {}
                4 => {
                    doc.scene.get_mut(id).expect("node").transform = Transform2D::scale_xy(0., 1.);
                }
                _ => unreachable!(),
            }
            let original = node(&doc, id).clone();
            doc.selection.select_only(id);
            doc.history = Default::default();
            let mut viewport = Viewport::default();
            let mut ctx = context(&mut doc, &mut viewport);
            if case == 3 {
                ctx.scope_root = Some(other);
            }
            drag(
                &mut ScaleTool::new(),
                &mut ctx,
                DVec2::new(100., 50.),
                DVec2::new(200., 100.),
                ModifierKeys::empty(),
            );
            assert_eq!(node(ctx.doc, id).data, original.data, "case {case}");
            assert_eq!(
                node(ctx.doc, id).transform,
                original.transform,
                "case {case}"
            );
            assert_eq!(ctx.doc.history.undo_depth(), 0, "case {case}");
        }
    }

    #[test]
    fn scale_external_lock_cancels_preview_without_reverting_the_lock() {
        let mut doc = Doc::new();
        let id = insert(&mut doc, rectangle(100., 50.), None);
        let original = node(&doc, id).data.clone();
        doc.selection.select_only(id);
        doc.history = Default::default();
        let mut viewport = Viewport::default();
        let mut ctx = context(&mut doc, &mut viewport);
        let mut tool = ScaleTool::new();
        press(
            &mut tool,
            &mut ctx,
            DVec2::new(100., 50.),
            ModifierKeys::empty(),
        );
        move_to(
            &mut tool,
            &mut ctx,
            DVec2::new(200., 100.),
            ModifierKeys::empty(),
        );
        assert_ne!(node(ctx.doc, id).data, original);
        ctx.doc
            .scene
            .get_mut(id)
            .expect("node")
            .flags
            .insert(NodeFlags::LOCKED);
        release(
            &mut tool,
            &mut ctx,
            DVec2::new(200., 100.),
            ModifierKeys::empty(),
        );
        assert_eq!(node(ctx.doc, id).data, original);
        assert!(node(ctx.doc, id).flags.contains(NodeFlags::LOCKED));
        assert_eq!(ctx.doc.history.undo_depth(), 0);
    }

    #[test]
    fn scale_resolves_dimensional_bindings_locally_and_undo_restores_them() {
        let mut doc = Doc::new();
        let collection = VariableCollectionId::new();
        let mode = ModeId::new();
        let variable = VariableId::new();
        doc.variables.collections.insert(
            collection,
            VariableCollection {
                id: collection,
                name: "Sizes".into(),
                modes: vec![Mode {
                    id: mode,
                    name: "Default".into(),
                }],
                default_mode: mode,
                variable_order: vec![variable],
            },
        );
        doc.variables.variables.insert(
            variable,
            Variable {
                id: variable,
                collection,
                name: "Border".into(),
                ty: VariableType::Float,
                values_by_mode: BTreeMap::from([(mode, VarValue::Float { value: 8. })]),
                scopes: vec![],
            },
        );
        let mut shape = rectangle(100., 50.);
        shape
            .data
            .as_vector_mut()
            .expect("vector")
            .strokes
            .push(Stroke::solid(Color::BLACK, 2.));
        shape
            .bindings
            .insert(BoundProp::StrokeWidth { index: 0 }, variable);
        let opacity = VariableId::new();
        shape.bindings.insert(BoundProp::Opacity, opacity);
        let id = insert(&mut doc, shape, None);
        let original = node(&doc, id).clone();
        let variables = doc.variables.clone();
        doc.selection.select_only(id);
        doc.history = Default::default();
        let mut viewport = Viewport::default();
        let mut ctx = context(&mut doc, &mut viewport);
        drag(
            &mut ScaleTool::new(),
            &mut ctx,
            DVec2::new(100., 50.),
            DVec2::new(200., 100.),
            ModifierKeys::empty(),
        );
        let scaled = node(ctx.doc, id);
        assert_eq!(
            scaled
                .data
                .as_vector()
                .expect("vector")
                .strokes
                .first()
                .expect("stroke")
                .width,
            16.
        );
        assert!(
            !scaled
                .bindings
                .contains_key(&BoundProp::StrokeWidth { index: 0 })
        );
        assert_eq!(scaled.bindings.get(&BoundProp::Opacity), Some(&opacity));
        assert_eq!(
            ctx.doc.variables, variables,
            "shared tokens remain unchanged"
        );
        assert!(ctx.doc.undo().expect("undo bound scale"));
        assert_eq!(node(ctx.doc, id).data, original.data);
        assert_eq!(node(ctx.doc, id).bindings, original.bindings);
        assert!(ctx.doc.redo().expect("redo bound scale"));
        assert_eq!(
            node(ctx.doc, id)
                .data
                .as_vector()
                .expect("vector")
                .strokes
                .first()
                .expect("stroke")
                .width,
            16.
        );
    }

    #[test]
    fn scale_instance_preserves_its_source_and_scales_the_placement_transform() {
        let mut doc = Doc::new();
        let source = insert(
            &mut doc,
            CanvasNode::new(NodeData::Text(TextNode::new("Master", 100., 50.))),
            None,
        );
        let original_source = node(&doc, source).clone();
        let component = ComponentId::new();
        doc.components
            .defs
            .insert(component, ComponentDef::new(component, source, "Master"));
        let components = doc.components.clone();
        let id = insert(
            &mut doc,
            CanvasNode::new(NodeData::Instance(InstanceNode {
                component,
                overrides: vec![],
                prop_values: BTreeMap::new(),
                derived: vec![],
                local_size: [100., 50.],
            })),
            None,
        );
        let original_data = node(&doc, id).data.clone();
        doc.selection.select_only(id);
        doc.history = Default::default();
        let mut viewport = Viewport::default();
        let mut ctx = context(&mut doc, &mut viewport);
        drag(
            &mut ScaleTool::new(),
            &mut ctx,
            DVec2::new(100., 50.),
            DVec2::new(200., 100.),
            ModifierKeys::empty(),
        );
        assert_eq!(node(ctx.doc, id).transform, Transform2D::scale(2.));
        assert_eq!(node(ctx.doc, id).data, original_data);
        assert_eq!(ctx.doc.components, components);
        assert_eq!(node(ctx.doc, source).data, original_source.data);
        assert_eq!(node(ctx.doc, source).transform, original_source.transform);
        assert!(ctx.doc.undo().expect("undo instance scale"));
        assert_eq!(node(ctx.doc, id).transform, Transform2D::IDENTITY);
    }

    #[test]
    fn scale_scene_replacement_cannot_restore_old_geometry_into_the_new_scene() {
        let mut doc = Doc::new();
        let id = insert(&mut doc, rectangle(100., 50.), None);
        doc.selection.select_only(id);
        doc.history = Default::default();
        let mut viewport = Viewport::default();
        let mut ctx = context(&mut doc, &mut viewport);
        let mut tool = ScaleTool::new();
        press(
            &mut tool,
            &mut ctx,
            DVec2::new(100., 50.),
            ModifierKeys::empty(),
        );
        move_to(
            &mut tool,
            &mut ctx,
            DVec2::new(200., 100.),
            ModifierKeys::empty(),
        );
        let mut replacement = Doc::new();
        let mut shape = rectangle(30., 20.);
        shape.id = id;
        insert(&mut replacement, shape, None);
        let replacement_data = node(&replacement, id).data.clone();
        ctx.doc.scene = replacement.scene;
        tool.deactivate(&mut ctx);
        assert_eq!(node(ctx.doc, id).data, replacement_data);
        assert_eq!(ctx.doc.history.undo_depth(), 0);
    }

    #[test]
    fn scale_every_handle_keeps_its_opposite_anchor_on_offset_geometry() {
        for handle in ResizeHandle::ALL {
            let mut doc = Doc::new();
            let id = insert(
                &mut doc,
                CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
                    10.,
                    20.,
                    100.,
                    50.,
                    Color::BLACK,
                ))),
                None,
            );
            doc.selection.select_only(id);
            doc.history = Default::default();
            let bounds = Bounds::from_xywh(10., 20., 100., 50.);
            let anchor = handle.anchor_world(bounds);
            let from = handle.handle_world(bounds);
            let mut viewport = Viewport::default();
            let mut ctx = context(&mut doc, &mut viewport);
            drag(
                &mut ScaleTool::new(),
                &mut ctx,
                from,
                anchor + (from - anchor) * 2.,
                ModifierKeys::empty(),
            );
            let scaled = ctx.doc.scene.world_bounds(id).expect("scaled bounds");
            near(scaled.width(), 200.);
            near(scaled.height(), 100.);
            near_point(handle.anchor_world(scaled), anchor);
        }
    }

    #[test]
    fn scale_returning_to_original_size_leaves_no_history_or_binding_changes() {
        let mut doc = Doc::new();
        let id = insert(&mut doc, rectangle(100., 50.), None);
        let original = node(&doc, id).data.clone();
        doc.selection.select_only(id);
        doc.history = Default::default();
        let mut viewport = Viewport::default();
        let mut ctx = context(&mut doc, &mut viewport);
        let mut tool = ScaleTool::new();
        let from = DVec2::new(100., 50.);
        press(&mut tool, &mut ctx, from, ModifierKeys::empty());
        move_to(&mut tool, &mut ctx, from * 2., ModifierKeys::empty());
        assert_ne!(node(ctx.doc, id).data, original);
        move_to(&mut tool, &mut ctx, from, ModifierKeys::empty());
        release(&mut tool, &mut ctx, from, ModifierKeys::empty());
        assert_eq!(node(ctx.doc, id).data, original);
        assert_eq!(node(ctx.doc, id).transform, Transform2D::IDENTITY);
        assert_eq!(ctx.doc.history.undo_depth(), 0);
    }

    #[test]
    fn scale_nonfinite_style_aborts_the_entire_preview_without_partial_geometry() {
        let mut doc = Doc::new();
        let first = insert(&mut doc, rectangle(100., 50.), None);
        let mut second = rectangle(100., 50.);
        second.transform = Transform2D::translation(200., 100.);
        second.blurs.push(Blur::layer(f64::MAX));
        let second = insert(&mut doc, second, None);
        let originals = [node(&doc, first).clone(), node(&doc, second).clone()];
        doc.selection.select_only(first);
        doc.selection.toggle(second);
        doc.history = Default::default();
        let mut viewport = Viewport::default();
        let mut ctx = context(&mut doc, &mut viewport);
        drag(
            &mut ScaleTool::new(),
            &mut ctx,
            DVec2::new(300., 150.),
            DVec2::new(600., 300.),
            ModifierKeys::empty(),
        );
        for original in originals {
            let unchanged = node(ctx.doc, original.id);
            assert_eq!(unchanged.data, original.data);
            assert_eq!(unchanged.transform, original.transform);
            assert_eq!(unchanged.blurs, original.blurs);
        }
        assert_eq!(ctx.doc.history.undo_depth(), 0);
    }

    #[test]
    fn scale_bound_frame_uses_visible_dimensions_and_active_mode_for_center_anchor() {
        let mut doc = Doc::new();
        let collection = VariableCollectionId::new();
        let default_mode = ModeId::new();
        let active_mode = ModeId::new();
        let width = VariableId::new();
        let height = VariableId::new();
        doc.variables.collections.insert(
            collection,
            VariableCollection {
                id: collection,
                name: "Frame dimensions".into(),
                modes: vec![
                    Mode {
                        id: default_mode,
                        name: "Small".into(),
                    },
                    Mode {
                        id: active_mode,
                        name: "Large".into(),
                    },
                ],
                default_mode,
                variable_order: vec![width, height],
            },
        );
        for (variable, name, small, large) in
            [(width, "Width", 100., 200.), (height, "Height", 50., 100.)]
        {
            doc.variables.variables.insert(
                variable,
                Variable {
                    id: variable,
                    collection,
                    name: name.into(),
                    ty: VariableType::Float,
                    values_by_mode: BTreeMap::from([
                        (default_mode, VarValue::Float { value: small }),
                        (active_mode, VarValue::Float { value: large }),
                    ]),
                    scopes: vec![],
                },
            );
        }
        doc.active_modes.insert(collection, active_mode);
        let mut frame = CanvasNode::new(NodeData::Group(GroupNode {
            clip_size: Some([100., 50.]),
            ..Default::default()
        }));
        frame.bindings.insert(BoundProp::ClipWidth, width);
        frame.bindings.insert(BoundProp::ClipHeight, height);
        let id = insert(&mut doc, frame, None);
        let original = node(&doc, id).clone();
        let variables = doc.variables.clone();
        doc.selection.select_only(id);
        doc.history = Default::default();
        let mut viewport = Viewport::default();
        let mut ctx = context(&mut doc, &mut viewport);
        drag(
            &mut ScaleTool::new(),
            &mut ctx,
            DVec2::new(200., 100.),
            DVec2::new(300., 150.),
            ModifierKeys::ALT,
        );
        let scaled = node(ctx.doc, id);
        assert_eq!(
            scaled.data.as_group().expect("frame").clip_size,
            Some([400., 200.])
        );
        assert_eq!(scaled.transform, Transform2D::translation(-100., -50.));
        assert!(scaled.bindings.is_empty());
        near_point(
            ctx.doc
                .scene
                .world_bounds(id)
                .expect("frame bounds")
                .center(),
            DVec2::new(100., 50.),
        );
        assert_eq!(ctx.doc.variables, variables);
        assert!(ctx.doc.undo().expect("undo bound frame"));
        assert_eq!(node(ctx.doc, id).data, original.data);
        assert_eq!(node(ctx.doc, id).bindings, original.bindings);
        assert_eq!(node(ctx.doc, id).transform, original.transform);
        assert!(ctx.doc.redo().expect("redo bound frame"));
        assert_eq!(
            node(ctx.doc, id).data.as_group().expect("frame").clip_size,
            Some([400., 200.])
        );
    }

    #[test]
    fn scale_release_uses_final_pointer_position_without_a_matching_move_event() {
        for intermediate_move in [false, true] {
            let mut doc = Doc::new();
            let id = insert(&mut doc, rectangle(100., 50.), None);
            doc.selection.select_only(id);
            doc.history = Default::default();
            let mut viewport = Viewport::default();
            let mut ctx = context(&mut doc, &mut viewport);
            let mut tool = ScaleTool::new();
            press(
                &mut tool,
                &mut ctx,
                DVec2::new(100., 50.),
                ModifierKeys::empty(),
            );
            if intermediate_move {
                move_to(
                    &mut tool,
                    &mut ctx,
                    DVec2::new(150., 75.),
                    ModifierKeys::empty(),
                );
            }
            release(
                &mut tool,
                &mut ctx,
                DVec2::new(200., 100.),
                ModifierKeys::empty(),
            );
            let bounds = ctx.doc.scene.world_bounds(id).expect("scaled bounds");
            near(bounds.width(), 200.);
            near(bounds.height(), 100.);
            assert_eq!(ctx.doc.history.undo_depth(), 1);
        }
    }
}
