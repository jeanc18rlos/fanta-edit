use std::collections::{HashSet, VecDeque};

use fanta_canvas::{HitPrecision, MarqueeMode, hit_test_deep, hit_test_within_screen};
use fanta_doc::{Bounds, Color, Fill, NodeData, NodeFlags, NodeId, Scene};
use glam::DVec2;

use super::rectangle::{RectangleSelectTool, RectangleSelectionOperation};
use crate::context::ToolContext;
use crate::event::{Button, LogicalKey, ModifierKeys, PointerEvent, ToolEvent};
use crate::tool::{CursorHint, Tool, ToolOverlay, ToolResponse, bounds_from_corners};

#[derive(Clone, Debug, PartialEq)]
pub struct DrawSelectionRegion {
    pub shape: DrawSelectionShape,
    pub targets: Vec<NodeId>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum DrawSelectionShape {
    Rectangle(Bounds),
    Ellipse(Bounds),
    Polygon(Vec<[f64; 2]>),
    RasterRuns {
        quads: Vec<[[f64; 2]; 4]>,
        bounds: Bounds,
    },
    Combined {
        operation: DrawShapeOperation,
        first: Box<DrawSelectionShape>,
        second: Box<DrawSelectionShape>,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DrawShapeOperation {
    Union,
    Subtract,
    Intersect,
}

impl DrawSelectionShape {
    pub fn bounds(&self) -> Option<Bounds> {
        match self {
            Self::Rectangle(bounds) | Self::Ellipse(bounds) => Some(*bounds),
            Self::Polygon(points) => {
                let first = points.first()?;
                let mut minimum = DVec2::from(*first);
                let mut maximum = minimum;
                for point in points.iter().skip(1) {
                    let point = DVec2::from(*point);
                    minimum = minimum.min(point);
                    maximum = maximum.max(point);
                }
                Some(Bounds::from_min_max(minimum, maximum))
            }
            Self::RasterRuns { bounds, .. } => Some(*bounds),
            Self::Combined {
                operation,
                first,
                second,
            } => {
                let first = first.bounds()?;
                let second = second.bounds()?;
                match operation {
                    DrawShapeOperation::Union => Some(first.union(&second)),
                    DrawShapeOperation::Subtract => Some(first),
                    DrawShapeOperation::Intersect => {
                        let minimum = DVec2::new(
                            first.min_x.max(second.min_x),
                            first.min_y.max(second.min_y),
                        );
                        let maximum = DVec2::new(
                            first.max_x.min(second.max_x),
                            first.max_y.min(second.max_y),
                        );
                        (minimum.x < maximum.x && minimum.y < maximum.y)
                            .then(|| Bounds::from_min_max(minimum, maximum))
                    }
                }
            }
        }
    }

    pub fn overlays(&self) -> Vec<ToolOverlay> {
        match self {
            Self::Rectangle(bounds) => vec![ToolOverlay::PreviewRect {
                world_rect: *bounds,
            }],
            Self::Ellipse(bounds) => vec![ToolOverlay::PreviewEllipse {
                world_rect: *bounds,
            }],
            Self::Polygon(points) => points
                .iter()
                .copied()
                .zip(points.iter().copied().cycle().skip(1))
                .take(points.len())
                .map(|(world_start, world_end)| ToolOverlay::PreviewLine {
                    world_start,
                    world_end,
                })
                .collect(),
            Self::RasterRuns { bounds, .. } => {
                vec![ToolOverlay::PreviewRect {
                    world_rect: *bounds,
                }]
            }
            Self::Combined { first, second, .. } => {
                let mut overlays = first.overlays();
                overlays.extend(second.overlays());
                overlays
            }
        }
    }
}

pub fn apply_draw_region(ctx: &mut ToolContext, shape: DrawSelectionShape, hits: Vec<NodeId>) {
    let operation = ctx.rectangle_selection_operation;
    let previous = ctx.draw_selection_region.take();
    if operation != RectangleSelectionOperation::Subtract || previous.is_none() {
        RectangleSelectTool::apply_hits(operation, &mut ctx.doc.selection, hits);
    }
    let shape = match (operation, previous) {
        (RectangleSelectionOperation::Replace, _) => Some(shape),
        (RectangleSelectionOperation::Add, Some(previous)) => Some(DrawSelectionShape::Combined {
            operation: DrawShapeOperation::Union,
            first: Box::new(previous.shape),
            second: Box::new(shape),
        }),
        (RectangleSelectionOperation::Subtract, Some(previous)) => {
            Some(DrawSelectionShape::Combined {
                operation: DrawShapeOperation::Subtract,
                first: Box::new(previous.shape),
                second: Box::new(shape),
            })
        }
        (RectangleSelectionOperation::Intersect, Some(previous)) => {
            Some(DrawSelectionShape::Combined {
                operation: DrawShapeOperation::Intersect,
                first: Box::new(previous.shape),
                second: Box::new(shape),
            })
        }
        (RectangleSelectionOperation::Add, None) => Some(shape),
        (RectangleSelectionOperation::Subtract | RectangleSelectionOperation::Intersect, None) => {
            None
        }
    };
    ctx.draw_selection_region = shape.map(|shape| DrawSelectionRegion {
        shape,
        targets: ctx.doc.selection.iter().copied().collect(),
    });
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RegionSelectionKind {
    Ellipse,
    Lasso,
    PolygonalLasso,
    MagicWand,
}

pub struct RegionSelectTool {
    kind: RegionSelectionKind,
    points: Vec<DVec2>,
    pointer: Option<DVec2>,
    dragging: bool,
}

impl RegionSelectTool {
    pub fn new(kind: RegionSelectionKind) -> Self {
        Self {
            kind,
            points: Vec::new(),
            pointer: None,
            dragging: false,
        }
    }

    fn apply_region(&mut self, ctx: &mut ToolContext, modifiers: ModifierKeys) {
        if self.points.len() < 2 {
            self.points.clear();
            return;
        }
        let Some(screen_bounds) = point_bounds(&self.points) else {
            return;
        };
        let world_points: Vec<_> = self
            .points
            .iter()
            .map(|point| ctx.screen_to_world(*point))
            .collect();
        let mode = if modifiers.contains(ModifierKeys::ALT) {
            MarqueeMode::Intersects
        } else {
            MarqueeMode::Contains
        };
        let Some(world_bounds) = point_bounds(&world_points) else {
            return;
        };
        let mut hits = Vec::new();
        let mut seen = HashSet::new();
        for leaf in hit_test_within_screen(
            &ctx.doc.scene,
            ctx.viewport,
            ctx.screen_size,
            screen_bounds,
            MarqueeMode::Intersects,
            ctx.scope(),
        ) {
            let Some(target) = content_target(&ctx.doc.scene, leaf, ctx.scope()) else {
                continue;
            };
            let Some(bounds) = ctx.doc.scene.world_bounds(target) else {
                continue;
            };
            let matches = match self.kind {
                RegionSelectionKind::Ellipse => ellipse_matches(bounds, world_bounds, mode),
                RegionSelectionKind::Lasso | RegionSelectionKind::PolygonalLasso => {
                    polygon_matches(bounds, &world_points, mode)
                }
                RegionSelectionKind::MagicWand => false,
            };
            if matches && seen.insert(target) {
                hits.push(target);
            }
        }
        let shape = match self.kind {
            RegionSelectionKind::Ellipse => DrawSelectionShape::Ellipse(world_bounds),
            RegionSelectionKind::Lasso | RegionSelectionKind::PolygonalLasso => {
                DrawSelectionShape::Polygon(
                    world_points.iter().map(|point| point.to_array()).collect(),
                )
            }
            RegionSelectionKind::MagicWand => return,
        };
        apply_draw_region(ctx, shape, hits);
        self.points.clear();
    }

    fn select_at(&self, ctx: &mut ToolContext, screen: DVec2) {
        ctx.draw_selection_region = None;
        let target = hit_test_deep(
            &ctx.doc.scene,
            ctx.screen_to_world(screen),
            HitPrecision::Path,
            ctx.scope(),
        )
        .into_iter()
        .find_map(|leaf| content_target(&ctx.doc.scene, leaf, ctx.scope()));
        RectangleSelectTool::apply_hits(
            ctx.rectangle_selection_operation,
            &mut ctx.doc.selection,
            target,
        );
    }

    fn magic_wand(&self, ctx: &mut ToolContext, screen: DVec2) {
        if let Some((target, shape)) = ctx.bitmap_wand_override.take() {
            apply_draw_region(ctx, shape, vec![target]);
            return;
        }
        let Some(seed) = hit_test_deep(
            &ctx.doc.scene,
            ctx.screen_to_world(screen),
            HitPrecision::Path,
            ctx.scope(),
        )
        .into_iter()
        .find_map(|leaf| content_target(&ctx.doc.scene, leaf, ctx.scope())) else {
            RectangleSelectTool::apply_hits(
                ctx.rectangle_selection_operation,
                &mut ctx.doc.selection,
                [],
            );
            return;
        };
        let Some(seed_node) = ctx.doc.scene.get(seed) else {
            return;
        };
        let seed_paint = paint_key(&seed_node.data);
        let candidates = draw_content_candidates(&ctx.doc.scene, ctx.scope());
        let mut matches: Vec<NodeId> = candidates
            .into_iter()
            .filter(|id| {
                if *id == seed {
                    return true;
                }
                ctx.doc.scene.get(*id).is_some_and(|node| {
                    paint_key(&node.data).is_some_and(|paint| {
                        paint_similar(paint, seed_paint, ctx.selection_tolerance)
                    })
                })
            })
            .collect();
        if ctx.selection_contiguous {
            let mut connected = HashSet::from([seed]);
            let matching: HashSet<_> = matches.iter().copied().collect();
            let mut queue = VecDeque::from([seed]);
            while let Some(selected) = queue.pop_front() {
                let Some(selected_bounds) = ctx.doc.scene.world_bounds(selected) else {
                    continue;
                };
                let search = Bounds::from_xywh(
                    selected_bounds.min_x - 1.0,
                    selected_bounds.min_y - 1.0,
                    selected_bounds.width() + 2.0,
                    selected_bounds.height() + 2.0,
                );
                for leaf in ctx.doc.scene.rect_query_where(search, |_, _| true) {
                    let Some(candidate) = content_target(&ctx.doc.scene, leaf, ctx.scope()) else {
                        continue;
                    };
                    if matching.contains(&candidate)
                        && !connected.contains(&candidate)
                        && ctx.doc.scene.world_bounds(candidate).is_some_and(|bounds| {
                            bounds_intersect_with_gap(selected_bounds, bounds, 1.0)
                        })
                    {
                        connected.insert(candidate);
                        queue.push_back(candidate);
                    }
                }
            }
            matches.retain(|id| connected.contains(id));
        }
        let bounds = matches
            .iter()
            .filter_map(|id| ctx.doc.scene.world_bounds(*id))
            .reduce(|first, second| first.union(&second));
        if let Some(bounds) = bounds {
            apply_draw_region(ctx, DrawSelectionShape::Rectangle(bounds), matches);
        }
    }

    fn overlay(&self, ctx: &ToolContext) -> ToolResponse {
        let mut response = ToolResponse::cursor(CursorHint::Crosshair);
        if self.points.is_empty() {
            return response;
        }
        match self.kind {
            RegionSelectionKind::Ellipse => {
                if let (Some(press), Some(pointer)) = (self.points.first(), self.pointer) {
                    response = response.with_overlay(ToolOverlay::PreviewEllipse {
                        world_rect: bounds_from_corners(
                            ctx.screen_to_world(*press),
                            ctx.screen_to_world(pointer),
                        ),
                    });
                }
            }
            RegionSelectionKind::Lasso | RegionSelectionKind::PolygonalLasso => {
                let mut points = self.points.clone();
                if let Some(pointer) = self.pointer {
                    if points.last() != Some(&pointer) {
                        points.push(pointer);
                    }
                }
                for edge in points.windows(2) {
                    response = response.with_overlay(ToolOverlay::PreviewLine {
                        world_start: ctx.screen_to_world(edge[0]).to_array(),
                        world_end: ctx.screen_to_world(edge[1]).to_array(),
                    });
                }
            }
            RegionSelectionKind::MagicWand => {}
        }
        response
    }
}

impl Tool for RegionSelectTool {
    fn name(&self) -> &'static str {
        match self.kind {
            RegionSelectionKind::Ellipse => "ellipse-select",
            RegionSelectionKind::Lasso => "lasso",
            RegionSelectionKind::PolygonalLasso => "polygonal-lasso",
            RegionSelectionKind::MagicWand => "magic-wand",
        }
    }

    fn handle_event(&mut self, ctx: &mut ToolContext, event: ToolEvent) -> ToolResponse {
        match event {
            ToolEvent::Pointer(PointerEvent::Press {
                screen,
                button: Button::Primary,
                modifiers,
                count,
            }) => {
                let screen = DVec2::from(screen);
                if self.kind == RegionSelectionKind::MagicWand {
                    self.magic_wand(ctx, screen);
                    return ToolResponse::cursor(CursorHint::Crosshair);
                }
                if self.kind == RegionSelectionKind::PolygonalLasso {
                    if self.points.is_empty()
                        && ctx.rectangle_selection_operation == RectangleSelectionOperation::Replace
                    {
                        ctx.draw_selection_region = None;
                    }
                    self.points.push(screen);
                    self.pointer = Some(screen);
                    if count >= 2 && self.points.len() >= 3 {
                        self.apply_region(ctx, modifiers);
                        self.pointer = None;
                    }
                    return self.overlay(ctx);
                }
                self.points.clear();
                if ctx.rectangle_selection_operation == RectangleSelectionOperation::Replace {
                    ctx.draw_selection_region = None;
                }
                self.points.push(screen);
                self.pointer = Some(screen);
                self.dragging = true;
                self.overlay(ctx)
            }
            ToolEvent::Pointer(PointerEvent::Move { screen, .. }) => {
                let screen = DVec2::from(screen);
                if !self.dragging && self.kind != RegionSelectionKind::PolygonalLasso {
                    return ToolResponse::cursor(CursorHint::Crosshair);
                }
                self.pointer = Some(screen);
                if self.kind == RegionSelectionKind::Lasso
                    && self
                        .points
                        .last()
                        .is_some_and(|last| (*last - screen).length() >= 2.0)
                {
                    self.points.push(screen);
                }
                self.overlay(ctx)
            }
            ToolEvent::Pointer(PointerEvent::Release {
                screen,
                button: Button::Primary,
                modifiers,
            }) => {
                if !self.dragging {
                    return self.overlay(ctx);
                }
                self.dragging = false;
                let screen = DVec2::from(screen);
                self.pointer = Some(screen);
                if self.kind == RegionSelectionKind::Ellipse {
                    if self
                        .points
                        .first()
                        .is_none_or(|press| (screen - *press).length() < 3.0)
                    {
                        self.select_at(ctx, screen);
                    } else {
                        self.points.push(screen);
                        self.apply_region(ctx, modifiers);
                    }
                } else {
                    if self.points.last() != Some(&screen) {
                        self.points.push(screen);
                    }
                    if self.points.len() < 3 {
                        self.select_at(ctx, screen);
                    } else {
                        self.apply_region(ctx, modifiers);
                    }
                }
                self.points.clear();
                self.pointer = None;
                ToolResponse::cursor(CursorHint::Crosshair)
            }
            ToolEvent::Key(key) if key.key == LogicalKey::Enter => {
                if self.kind == RegionSelectionKind::PolygonalLasso && self.points.len() >= 3 {
                    self.apply_region(ctx, key.modifiers);
                    self.pointer = None;
                }
                ToolResponse::cursor(CursorHint::Crosshair)
            }
            ToolEvent::Key(key) if key.key == LogicalKey::Escape => {
                if self.points.is_empty() {
                    ToolResponse::exit().with_cursor(CursorHint::Default)
                } else {
                    self.points.clear();
                    self.pointer = None;
                    self.dragging = false;
                    ToolResponse::cursor(CursorHint::Crosshair)
                }
            }
            _ => ToolResponse::empty(),
        }
    }

    fn activate(&mut self, _ctx: &mut ToolContext) {
        self.points.clear();
        self.pointer = None;
        self.dragging = false;
    }

    fn deactivate(&mut self, ctx: &mut ToolContext) {
        self.activate(ctx);
    }
}

pub fn draw_content_target(scene: &Scene, id: NodeId, scope: Option<NodeId>) -> Option<NodeId> {
    let mut target = id;
    let node = scene.get(id)?;
    if node.flags.intersects(NodeFlags::HIDDEN | NodeFlags::LOCKED) {
        return None;
    }
    for ancestor in scene.ancestors_of(id) {
        if ancestor
            .flags
            .intersects(NodeFlags::HIDDEN | NodeFlags::LOCKED)
        {
            return None;
        }
        if Some(ancestor.id) == scope {
            break;
        }
        if matches!(ancestor.data, NodeData::Boolean(_)) {
            target = ancestor.id;
        }
    }
    scene.get(target).and_then(|node| {
        matches!(
            node.data,
            NodeData::Vector(_) | NodeData::Bitmap(_) | NodeData::Boolean(_)
        )
        .then_some(target)
    })
}

pub(super) use draw_content_target as content_target;

pub fn draw_content_candidates(scene: &Scene, scope: Option<NodeId>) -> Vec<NodeId> {
    let roots = match scope {
        Some(root) => scene.children_of(Some(root)),
        None => scene.roots(),
    };
    let mut seen = HashSet::new();
    roots
        .iter()
        .flat_map(|root| scene.descendants_of(*root))
        .filter_map(|id| draw_content_target(scene, id, scope))
        .filter(|id| seen.insert(*id))
        .collect()
}

fn point_bounds(points: &[DVec2]) -> Option<Bounds> {
    let mut minimum = *points.first()?;
    let mut maximum = minimum;
    for point in points.iter().skip(1) {
        minimum = minimum.min(*point);
        maximum = maximum.max(*point);
    }
    Some(Bounds::from_min_max(minimum, maximum))
}

fn corners(bounds: Bounds) -> [DVec2; 4] {
    [
        DVec2::new(bounds.min_x, bounds.min_y),
        DVec2::new(bounds.max_x, bounds.min_y),
        DVec2::new(bounds.min_x, bounds.max_y),
        DVec2::new(bounds.max_x, bounds.max_y),
    ]
}

fn ellipse_contains(point: DVec2, bounds: Bounds) -> bool {
    let radius_x = bounds.width() * 0.5;
    let radius_y = bounds.height() * 0.5;
    if radius_x <= 0.0 || radius_y <= 0.0 {
        return false;
    }
    let center = bounds.center();
    let offset = point - center;
    (offset.x / radius_x).powi(2) + (offset.y / radius_y).powi(2) <= 1.0
}

fn ellipse_matches(bounds: Bounds, ellipse: Bounds, mode: MarqueeMode) -> bool {
    if mode == MarqueeMode::Contains {
        return corners(bounds)
            .into_iter()
            .all(|corner| ellipse_contains(corner, ellipse));
    }
    let center = ellipse.center();
    let radius_x = ellipse.width() * 0.5;
    let radius_y = ellipse.height() * 0.5;
    if radius_x <= 0.0 || radius_y <= 0.0 {
        return false;
    }
    let nearest_x = center.x.clamp(bounds.min_x, bounds.max_x);
    let nearest_y = center.y.clamp(bounds.min_y, bounds.max_y);
    ((nearest_x - center.x) / radius_x).powi(2) + ((nearest_y - center.y) / radius_y).powi(2) <= 1.0
}

fn polygon_contains(point: DVec2, polygon: &[DVec2]) -> bool {
    let mut inside = false;
    for (start, end) in polygon
        .iter()
        .copied()
        .zip(polygon.iter().copied().cycle().skip(1))
        .take(polygon.len())
    {
        if (start.y > point.y) != (end.y > point.y)
            && point.x < (end.x - start.x) * (point.y - start.y) / (end.y - start.y) + start.x
        {
            inside = !inside;
        }
    }
    inside
}

fn segment_intersects(first: DVec2, second: DVec2, third: DVec2, fourth: DVec2) -> bool {
    fn cross(first: DVec2, second: DVec2) -> f64 {
        first.x * second.y - first.y * second.x
    }
    let edge = second - first;
    let other = fourth - third;
    let denominator = cross(edge, other);
    if denominator.abs() < 1e-9 {
        return false;
    }
    let offset = third - first;
    let along_edge = cross(offset, other) / denominator;
    let along_other = cross(offset, edge) / denominator;
    (0.0..=1.0).contains(&along_edge) && (0.0..=1.0).contains(&along_other)
}

fn polygon_matches(bounds: Bounds, polygon: &[DVec2], mode: MarqueeMode) -> bool {
    if polygon.len() < 3 {
        return false;
    }
    let corners = corners(bounds);
    if mode == MarqueeMode::Contains {
        return corners
            .into_iter()
            .all(|corner| polygon_contains(corner, polygon));
    }
    if corners
        .into_iter()
        .any(|corner| polygon_contains(corner, polygon))
        || polygon.iter().any(|point| bounds.contains_point(*point))
    {
        return true;
    }
    let rectangle = [corners[0], corners[1], corners[3], corners[2]];
    polygon
        .iter()
        .copied()
        .zip(polygon.iter().copied().cycle().skip(1))
        .take(polygon.len())
        .any(|(start, end)| {
            rectangle
                .iter()
                .copied()
                .zip(rectangle.iter().copied().cycle().skip(1))
                .take(rectangle.len())
                .any(|(edge_start, edge_end)| segment_intersects(start, end, edge_start, edge_end))
        })
}

#[derive(Clone, Copy)]
enum PaintKey {
    Solid(Color),
    Bitmap(fanta_doc::AssetId),
}

fn paint_key(data: &NodeData) -> Option<PaintKey> {
    match data {
        NodeData::Vector(vector) => vector
            .fills
            .last()
            .and_then(Fill::solid_color)
            .or_else(|| {
                vector
                    .strokes
                    .last()
                    .and_then(|stroke| stroke.paint.solid_color())
            })
            .map(PaintKey::Solid),
        NodeData::Boolean(boolean) => boolean
            .fills
            .last()
            .and_then(Fill::solid_color)
            .or_else(|| {
                boolean
                    .strokes
                    .last()
                    .and_then(|stroke| stroke.paint.solid_color())
            })
            .map(PaintKey::Solid),
        NodeData::Bitmap(bitmap) => Some(PaintKey::Bitmap(bitmap.asset)),
        _ => None,
    }
}

fn paint_similar(candidate: PaintKey, seed: Option<PaintKey>, tolerance: u8) -> bool {
    match (candidate, seed) {
        (PaintKey::Solid(candidate), Some(PaintKey::Solid(seed))) => {
            candidate.r.abs_diff(seed.r) <= tolerance
                && candidate.g.abs_diff(seed.g) <= tolerance
                && candidate.b.abs_diff(seed.b) <= tolerance
                && candidate.a.abs_diff(seed.a) <= tolerance
        }
        (PaintKey::Bitmap(candidate), Some(PaintKey::Bitmap(seed))) => candidate == seed,
        _ => false,
    }
}

fn bounds_intersect_with_gap(first: Bounds, second: Bounds, gap: f64) -> bool {
    first.min_x <= second.max_x + gap
        && first.max_x + gap >= second.min_x
        && first.min_y <= second.max_y + gap
        && first.max_y + gap >= second.min_y
}
