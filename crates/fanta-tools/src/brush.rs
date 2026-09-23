use crate::context::ToolContext;
use crate::event::{Button, LogicalKey, PointerEvent, ToolEvent};
use crate::ink::{InkPoint, StrokeOptions, get_stroke};
use crate::tool::{CursorHint, Tool, ToolOverlay, ToolResponse};
use fanta_doc::{
    Blur, CanvasNode, Color, Fill, NodeData, Operation, PathData, Transform2D, VectorNode,
};
use glam::DVec2;
use smallvec::SmallVec;

const SAMPLE_PX: f64 = 2.0;
const MAX_SAMPLES: usize = 2048;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum BrushStyle {
    #[default]
    Round,
    Flat,
    Ink,
}

#[derive(Default)]
pub struct BrushTool {
    points: Vec<InkPoint>,
    preview_points: Vec<InkPoint>,
    last_sample_screen: Option<DVec2>,
    last_preview_screen: Option<DVec2>,
    sample_px: f64,
    active: bool,
}

impl BrushTool {
    pub fn new() -> Self {
        Self::default()
    }

    fn reset(&mut self) {
        self.points.clear();
        self.preview_points.clear();
        self.last_sample_screen = None;
        self.last_preview_screen = None;
        self.sample_px = SAMPLE_PX;
        self.active = false;
    }

    fn add_sample(&mut self, ctx: &ToolContext, screen: DVec2) {
        if self
            .last_sample_screen
            .is_some_and(|last| (screen - last).length() < SAMPLE_PX)
        {
            return;
        }
        let world = ctx.screen_to_world(screen);
        let point = InkPoint::new(world.x, world.y);
        self.points.push(point);
        self.last_sample_screen = Some(screen);
        if self
            .last_preview_screen
            .is_some_and(|last| (screen - last).length() < self.sample_px.max(SAMPLE_PX))
        {
            return;
        }
        self.preview_points.push(point);
        self.last_preview_screen = Some(screen);
        if self.preview_points.len() >= MAX_SAMPLES {
            let last = self.preview_points.len() - 1;
            self.preview_points = self
                .preview_points
                .drain(..)
                .enumerate()
                .filter_map(|(index, point)| (index % 2 == 0 || index == last).then_some(point))
                .collect();
            self.sample_px = self.sample_px.max(SAMPLE_PX) * 2.0;
        }
    }

    fn stroke_options(ctx: &ToolContext) -> StrokeOptions {
        let mut options = StrokeOptions {
            size: ctx.new_stroke_width.clamp(1.0, 5000.0),
            smoothing: 0.2,
            streamline: (ctx.stroke_smoothing / 6.0).clamp(0.0, 1.0),
            ..StrokeOptions::default()
        };
        match ctx.brush_style {
            BrushStyle::Round => {
                options.thinning = 0.0;
            }
            BrushStyle::Flat => {
                options.thinning = 0.0;
                options.cap_start = false;
                options.cap_end = false;
            }
            BrushStyle::Ink => {
                options.thinning = 0.75;
                options.smoothing = 0.45;
            }
        }
        options
    }

    fn stroke_color(ctx: &ToolContext) -> Color {
        let color = ctx.new_shape_fill;
        let alpha = u16::from(color.a) * u16::from(ctx.brush_flow.min(100)) / 100;
        Color::rgba(color.r, color.g, color.b, alpha as u8)
    }

    fn softness_radius(ctx: &ToolContext) -> f64 {
        ctx.new_stroke_width.clamp(1.0, 5000.0) * (100.0 - f64::from(ctx.brush_hardness.min(100)))
            / 100.0
            * 0.65
    }

    fn preview(&self, ctx: &ToolContext) -> ToolResponse {
        let mut response = ToolResponse::cursor(CursorHint::Crosshair);
        if self.preview_points.is_empty() {
            return response;
        }
        let mut preview_points = self.preview_points.clone();
        if let Some(latest) = self.points.last().copied()
            && preview_points.last() != Some(&latest)
        {
            preview_points.push(latest);
        }
        let options = Self::stroke_options(ctx);
        let outline = get_stroke(&preview_points, &options);
        if !outline.is_empty() {
            let color = Self::stroke_color(ctx);
            let softness_radius = Self::softness_radius(ctx);
            if softness_radius > 0.0 {
                let mut halo_options = options;
                halo_options.size = (halo_options.size + softness_radius * 2.0).min(5000.0);
                let halo = get_stroke(&preview_points, &halo_options);
                if !halo.is_empty() {
                    response.overlays.push(ToolOverlay::PreviewBrushStroke {
                        world_outline: halo,
                        color: Color::rgba(color.r, color.g, color.b, color.a / 4),
                    });
                }
            }
            response.overlays.push(ToolOverlay::PreviewBrushStroke {
                world_outline: outline,
                color: if softness_radius > 0.0 {
                    Color::rgba(
                        color.r,
                        color.g,
                        color.b,
                        (u16::from(color.a) * 3 / 4) as u8,
                    )
                } else {
                    color
                },
            });
        }
        response
    }

    fn commit(&mut self, ctx: &mut ToolContext) {
        let Some(first) = self.points.first() else {
            return;
        };
        let first = first.pt;
        let outline = get_stroke(&self.points, &Self::stroke_options(ctx));
        let Some(start) = outline.first() else {
            return;
        };
        let mut path = PathData::new();
        path.move_to(start[0] - first.x, start[1] - first.y);
        for point in outline.iter().skip(1) {
            path.line_to(point[0] - first.x, point[1] - first.y);
        }
        path.close();
        let mut fills = SmallVec::new();
        fills.push(Fill::solid(Self::stroke_color(ctx)));
        let mut node = CanvasNode::new(NodeData::Vector(VectorNode {
            path,
            fills,
            strokes: SmallVec::new(),
            corner_radius: None,
            corner_radii: None,
            corner_smoothing: 0.0,
            local_size: None,
            parametric: None,
        }));
        node.name = "Brush stroke".into();
        node.meta = serde_json::json!({ "fanta_draw_mark": "brush" });
        node.blend_mode = ctx.new_blend_mode;
        let softness_radius = Self::softness_radius(ctx);
        if softness_radius > 0.0 {
            node.blurs.push(Blur::layer(softness_radius));
        }
        node.transform = Transform2D::translation(first.x, first.y);
        ctx.place_new_node_on_active_page(&mut node);
        let id = node.id;
        if let Err(error) = ctx.doc.apply(Operation::create_node(node)) {
            tracing::warn!(target: "fanta-tools.brush", "create failed: {error}");
        } else {
            ctx.doc.selection.select_only(id);
        }
    }
}

impl Tool for BrushTool {
    fn name(&self) -> &'static str {
        "brush"
    }

    fn handle_event(&mut self, ctx: &mut ToolContext, event: ToolEvent) -> ToolResponse {
        match event {
            ToolEvent::Pointer(PointerEvent::Press {
                screen,
                button: Button::Primary,
                ..
            }) => {
                self.reset();
                self.active = true;
                self.add_sample(ctx, DVec2::from(screen));
                self.preview(ctx)
            }
            ToolEvent::Pointer(PointerEvent::Move { screen, .. }) if self.active => {
                self.add_sample(ctx, DVec2::from(screen));
                self.preview(ctx)
            }
            ToolEvent::Pointer(PointerEvent::Release {
                screen,
                button: Button::Primary,
                ..
            }) if self.active => {
                let release = DVec2::from(screen);
                if self.last_sample_screen != Some(release) {
                    let world = ctx.screen_to_world(release);
                    self.points.push(InkPoint::new(world.x, world.y));
                }
                self.commit(ctx);
                self.reset();
                ToolResponse::cursor(CursorHint::Crosshair)
            }
            ToolEvent::Key(key) if key.key == LogicalKey::Escape => {
                if self.active {
                    self.reset();
                    ToolResponse::cursor(CursorHint::Crosshair)
                } else {
                    ToolResponse::exit()
                }
            }
            _ => ToolResponse::empty(),
        }
    }

    fn activate(&mut self, _ctx: &mut ToolContext) {
        self.reset();
    }

    fn deactivate(&mut self, _ctx: &mut ToolContext) {
        self.reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::ModifierKeys;
    use fanta_canvas::SnapEngine;
    use fanta_doc::{BlendMode, Doc, Viewport};

    fn pointer(screen: [f64; 2], phase: &str) -> ToolEvent {
        let modifiers = ModifierKeys::empty();
        ToolEvent::Pointer(match phase {
            "press" => PointerEvent::Press {
                screen,
                button: Button::Primary,
                modifiers,
                count: 1,
            },
            "move" => PointerEvent::Move { screen, modifiers },
            _ => PointerEvent::Release {
                screen,
                button: Button::Primary,
                modifiers,
            },
        })
    }

    #[test]
    fn brush_creates_filled_undoable_ink_and_tip_changes_outline() {
        let mut doc = Doc::new();
        let mut viewport = Viewport::default();
        let snap = SnapEngine::default();
        let mut tool = BrushTool::new();
        let size = DVec2::new(800.0, 600.0);
        let mut round_segments = 0;
        let mut round_path = None;
        for style in [BrushStyle::Round, BrushStyle::Flat, BrushStyle::Ink] {
            let mut ctx = ToolContext::new(&mut doc, &mut viewport, snap.clone(), size);
            ctx.new_shape_fill = fanta_doc::Color::rgba(0x12, 0x34, 0x56, 0x80);
            ctx.new_stroke_width = 24.0;
            ctx.new_blend_mode = BlendMode::Multiply;
            ctx.brush_style = style;
            tool.handle_event(&mut ctx, pointer([400.0, 300.0], "press"));
            let preview = tool.handle_event(&mut ctx, pointer([430.0, 300.0], "move"));
            let expected_outline = get_stroke(&tool.points, &BrushTool::stroke_options(&ctx));
            assert!(matches!(
                preview.overlays.as_slice(),
                [ToolOverlay::PreviewBrushStroke { world_outline, color }]
                    if *world_outline == expected_outline && *color == ctx.new_shape_fill
            ));
            tool.handle_event(&mut ctx, pointer([460.0, 310.0], "release"));
            let node = doc
                .scene
                .get(*doc.scene.roots().last().expect("brush mark"))
                .expect("brush node");
            let NodeData::Vector(vector) = &node.data else {
                panic!("brush creates vector outline");
            };
            assert_eq!(vector.fills.len(), 1);
            assert!(vector.strokes.is_empty());
            assert_eq!(node.blend_mode, BlendMode::Multiply);
            assert_eq!(node.meta["fanta_draw_mark"], "brush");
            match style {
                BrushStyle::Round => {
                    round_segments = vector.path.segments.len();
                    round_path = Some(vector.path.clone());
                }
                BrushStyle::Flat => assert!(round_segments > vector.path.segments.len()),
                BrushStyle::Ink => assert_ne!(round_path.as_ref(), Some(&vector.path)),
            }
        }
        assert_eq!(doc.scene.len(), 3);
        assert!(doc.undo().expect("undo ink mark"));
        assert_eq!(doc.scene.len(), 2);
        assert!(doc.redo().expect("redo ink mark"));
        assert_eq!(doc.scene.len(), 3);
    }

    #[test]
    fn long_stroke_keeps_full_geometry_with_bounded_preview() {
        let mut doc = Doc::new();
        let mut viewport = Viewport::default();
        let ctx = ToolContext::new(
            &mut doc,
            &mut viewport,
            SnapEngine::default(),
            DVec2::new(800.0, 600.0),
        );
        let mut tool = BrushTool::new();
        for index in 0..5000 {
            tool.add_sample(&ctx, DVec2::new(400.0 + index as f64 * 3.0, 300.0));
        }
        assert_eq!(tool.points.len(), 5000);
        assert!(tool.preview_points.len() < MAX_SAMPLES);
    }

    #[test]
    fn brush_flow_and_hardness_affect_preview_and_committed_mark() {
        let mut doc = Doc::new();
        let mut viewport = Viewport::default();
        let mut context = ToolContext::new(
            &mut doc,
            &mut viewport,
            SnapEngine::default(),
            DVec2::new(800.0, 600.0),
        );
        context.new_shape_fill = Color::rgba(10, 20, 30, 200);
        context.new_stroke_width = 40.0;
        context.brush_flow = 50;
        context.brush_hardness = 0;
        let mut brush = BrushTool::new();
        brush.handle_event(&mut context, pointer([300.0, 300.0], "press"));
        let preview = brush.handle_event(&mut context, pointer([350.0, 300.0], "move"));
        assert!(matches!(
            preview.overlays.as_slice(),
            [ToolOverlay::PreviewBrushStroke { color: halo_color, .. },
             ToolOverlay::PreviewBrushStroke { color: core_color, .. }]
                if *halo_color == Color::rgba(10, 20, 30, 25)
                    && *core_color == Color::rgba(10, 20, 30, 75)
        ));
        brush.handle_event(&mut context, pointer([400.0, 300.0], "release"));
        let node = doc
            .scene
            .get(*doc.scene.roots().last().expect("brush mark"))
            .expect("brush node");
        let NodeData::Vector(vector) = &node.data else {
            panic!("brush mark must be a vector");
        };
        assert_eq!(
            vector.fills[0].solid_color(),
            Some(Color::rgba(10, 20, 30, 100))
        );
        assert_eq!(node.blurs.as_slice(), &[Blur::layer(26.0)]);
    }
}
