//! DesignPanel adapter: builds the controlled inspector read model from the
//! document snapshot library, maps `DesignPanelAction` intents onto the
//! existing `properties_ops` builders, and echoes fresh inspection contexts
//! from the host's `FigItemEvent` subscription.
//!
//! Wave-1 scope is deliberately honest rather than wide: sections the engine
//! cannot edit are gated off through `DesignPanelNodeCapabilities` instead of
//! half-wired. Live: position/size/rotation, appearance (visibility, opacity,
//! blend with Pass-through modeling), corners, solid fills and strokes,
//! stroke geometry, drop/inner shadows and layer/background blurs,
//! single-axis auto layout, whole-layer typography, instance props, and the
//! Page background. Gated off: layout grids, exports, style registries,
//! aspect-ratio lock, smart selection, constraints, arrange/transform
//! commands, text-path, and pattern/shader/media paint editing (gradient and
//! image paints are displayed read-only).

use std::collections::HashMap;

use fanta_doc::{
    BlendMode, Blur, BlurKind, BoundProp, Color as FantaColor, ComponentId, Doc, Fill, Gradient,
    LayoutMode, MaskType, NodeData, NodeFlags, NodeId, Operation, ParametricShape, Shadow,
    ShadowKind, StrokeAlign, StrokeCap, StrokeJoin, TextAlign, TextAutoResize,
    VAlign as TextVAlign, VarValue,
};
use fanta_gpui::design::{
    DesignAutoLayoutItem, DesignBlendMode, DesignColor, DesignComponentContext,
    DesignComponentProperty, DesignComponentPropertyValue, DesignComponentReference,
    DesignComponentRole, DesignCornerCapabilities, DesignEffect, DesignEffectKind,
    DesignEffectKindAvailability, DesignEffectSettings, DesignGradientStop, DesignLayout,
    DesignLayoutMode, DesignLetterSpacing, DesignLineHeight, DesignMaskType, DesignPageBackground,
    DesignPageViewData, DesignPaint, DesignPaintKind, DesignPaintProperty, DesignPaintValue,
    DesignPanel, DesignPanelAction, DesignPanelAutoLayoutDirection,
    DesignPanelAutoLayoutParticipation, DesignPanelAutoLayoutWrap, DesignPanelCollection,
    DesignPanelEditPhase, DesignPanelInspectionContext, DesignPanelMultipleSelection,
    DesignPanelNode, DesignPanelNodeCapabilities, DesignPanelNodeKind, DesignPanelParentLayout,
    DesignPanelPermissions, DesignPanelProperty, DesignPanelPropertyValueState, DesignPanelSection,
    DesignPanelTarget, DesignPanelValue, DesignSizingMode, DesignStroke, DesignStrokeAlign,
    DesignStrokeCap, DesignStrokeDashMode, DesignStrokeDashes, DesignStrokeJoin,
    DesignStrokeWeightMode, DesignStrokeWeights, DesignTextDecoration,
    DesignTextHorizontalAlignment, DesignTextResize, DesignTextVerticalAlignment, DesignTypography,
};
use gpui::{AppContext as _, Context, Entity, SharedString, Subscription, Window};

use crate::color_picker::GradientKind;
use crate::document::{DocChange, FigDocument};
use crate::properties_ops::{
    apply_preview_operation, blurs_operations, default_blur, default_shadow,
    detach_instance_operations, effects_operations, field_operations, finite_transform_operations,
    format_number, instance_prop_operations, layout_gap_operations, layout_limit_operations,
    layout_padding_operations, replace_data_operation, restore_snapshot,
    set_clip_content_meta_operation, set_corner_radius_corner, set_fill_color,
    shadow_field_operations, stroke_list_mut,
};
use crate::properties_snapshot::PaintKind as EnginePaintKind;
use crate::properties_snapshot::{
    CornerRadiusValue, InspectorField, NodeSection, NodeSnapshot, PaintSnapshot, PropValueSnapshot,
    TypographySnapshot, master_roots, multi_section, node_section, page_section,
};
use crate::view::FigView;

/// Per-surface kill switch: `FANTA_GPUI_DESIGN=0` keeps the legacy
/// `FantaPropertiesPanel` even when the process-wide runtime is enabled.
pub(crate) fn design_enabled() -> bool {
    !matches!(
        std::env::var("FANTA_GPUI_DESIGN").as_deref(),
        Ok("0") | Ok("false") | Ok("off")
    )
}

/// Parses a panel node id back to the engine id; a failure means a stale row.
fn node_id(id: &SharedString) -> Option<NodeId> {
    id.parse().ok()
}

// =============================================================================
// Read model: colors, blend, kinds
// =============================================================================

pub(crate) fn design_color(color: FantaColor) -> DesignColor {
    DesignColor::rgba(color.r, color.g, color.b, color.a)
}

pub(crate) fn fanta_color(color: DesignColor) -> FantaColor {
    FantaColor::rgba(color.red, color.green, color.blue, color.alpha)
}

/// Engine → panel blend mode. Total: every engine mode has a panel face.
pub(crate) fn design_blend_mode(mode: BlendMode) -> DesignBlendMode {
    match mode {
        BlendMode::Normal => DesignBlendMode::Normal,
        BlendMode::Multiply => DesignBlendMode::Multiply,
        BlendMode::Screen => DesignBlendMode::Screen,
        BlendMode::Overlay => DesignBlendMode::Overlay,
        BlendMode::Darken => DesignBlendMode::Darken,
        BlendMode::Lighten => DesignBlendMode::Lighten,
        BlendMode::ColorDodge => DesignBlendMode::ColorDodge,
        BlendMode::ColorBurn => DesignBlendMode::ColorBurn,
        BlendMode::HardLight => DesignBlendMode::HardLight,
        BlendMode::SoftLight => DesignBlendMode::SoftLight,
        BlendMode::Difference => DesignBlendMode::Difference,
        BlendMode::Exclusion => DesignBlendMode::Exclusion,
        BlendMode::Hue => DesignBlendMode::Hue,
        BlendMode::Saturation => DesignBlendMode::Saturation,
        BlendMode::Color => DesignBlendMode::Color,
        BlendMode::Luminosity => DesignBlendMode::Luminosity,
    }
}

/// Panel → engine blend mode. Partial: the engine has no Pass through (it is
/// modeled from `NodeFlags::ISOLATED_BLEND`) and no Linear burn/dodge.
pub(crate) fn engine_blend_mode(mode: DesignBlendMode) -> Option<BlendMode> {
    Some(match mode {
        DesignBlendMode::Normal => BlendMode::Normal,
        DesignBlendMode::Multiply => BlendMode::Multiply,
        DesignBlendMode::Screen => BlendMode::Screen,
        DesignBlendMode::Overlay => BlendMode::Overlay,
        DesignBlendMode::Darken => BlendMode::Darken,
        DesignBlendMode::Lighten => BlendMode::Lighten,
        DesignBlendMode::ColorDodge => BlendMode::ColorDodge,
        DesignBlendMode::ColorBurn => BlendMode::ColorBurn,
        DesignBlendMode::HardLight => BlendMode::HardLight,
        DesignBlendMode::SoftLight => BlendMode::SoftLight,
        DesignBlendMode::Difference => BlendMode::Difference,
        DesignBlendMode::Exclusion => BlendMode::Exclusion,
        DesignBlendMode::Hue => BlendMode::Hue,
        DesignBlendMode::Saturation => BlendMode::Saturation,
        DesignBlendMode::Color => BlendMode::Color,
        DesignBlendMode::Luminosity => BlendMode::Luminosity,
        DesignBlendMode::PassThrough
        | DesignBlendMode::LinearBurn
        | DesignBlendMode::LinearDodge => {
            return None;
        }
    })
}

/// The kind fold from engine node data to the panel's canonical taxonomy.
/// Vector provenance recovers Rectangle via `PathData::is_rect` and the
/// parametric shapes; component masters override their Group data; media
/// kinds the engine cannot edit fold to `Other` and are gated by an honest
/// capability snapshot. Mask state stays orthogonal (`DesignPanelNode::is_mask`).
pub(crate) fn design_kind(
    id: NodeId,
    data: &NodeData,
    masters: &HashMap<NodeId, ComponentId>,
) -> DesignPanelNodeKind {
    match data {
        NodeData::Group(group) => {
            if masters.contains_key(&id) {
                DesignPanelNodeKind::Component
            } else if group.is_frame_surface() {
                DesignPanelNodeKind::Frame
            } else {
                DesignPanelNodeKind::Group
            }
        }
        NodeData::Vector(vector) => match vector.parametric {
            Some(ParametricShape::Arc { .. }) => DesignPanelNodeKind::Ellipse,
            Some(ParametricShape::Star { .. }) => DesignPanelNodeKind::Star,
            Some(ParametricShape::Polygon { .. }) => DesignPanelNodeKind::Polygon,
            None if vector.path.is_rect() => DesignPanelNodeKind::Rectangle,
            None => DesignPanelNodeKind::Vector,
        },
        NodeData::Text(_) => DesignPanelNodeKind::Text,
        NodeData::Instance(_) => DesignPanelNodeKind::Instance,
        NodeData::Boolean(_) => DesignPanelNodeKind::BooleanOperation,
        NodeData::Bitmap(_) => DesignPanelNodeKind::Image,
        NodeData::Video(_) => DesignPanelNodeKind::Video,
        NodeData::Audio(_)
        | NodeData::NodeGraph(_)
        | NodeData::Model3d(_)
        | NodeData::AiArtifact(_)
        | NodeData::Embed(_) => DesignPanelNodeKind::Other,
    }
}

// =============================================================================
// Read model: paints, effects, layout, typography
// =============================================================================

/// One panel paint from a snapshot row. Paint ids are index-derived and
/// re-minted on every echo — the engine has no stable paint identity.
/// Gradients and media paints are displayed losslessly but read-only in
/// wave 1; only solid paints accept edits.
fn design_paint(
    node_id: NodeId,
    collection: &str,
    index: usize,
    snapshot: &PaintSnapshot,
) -> DesignPaint {
    let mut paint = match (&snapshot.kind, &snapshot.gradient) {
        (Some(EnginePaintKind::Solid), _) => {
            design_solid_paint(snapshot.color.unwrap_or(FantaColor::BLACK))
        }
        (Some(EnginePaintKind::Gradient(kind)), Some(gradient)) => {
            let mut paint = DesignPaint::gradient(
                match kind {
                    GradientKind::Linear => DesignPaintKind::LinearGradient,
                    GradientKind::Radial => DesignPaintKind::RadialGradient,
                    GradientKind::Angular => DesignPaintKind::AngularGradient,
                    GradientKind::Diamond => DesignPaintKind::DiamondGradient,
                },
                gradient_stops(gradient),
            );
            paint.read_only = true;
            paint
        }
        _ => {
            // An image/video fill (or a gradient whose payload went missing):
            // inspectable, never editable through this adapter yet.
            let mut paint = DesignPaint::image(fanta_gpui::design::DesignPaintSource::new(
                format!("{node_id}-{collection}-{index}-source"),
                snapshot.label.clone(),
            ));
            paint.read_only = true;
            paint
        }
    };
    paint.id = SharedString::from(format!("{node_id}-{collection}-{index}"));
    if let Some(opacity) = snapshot.opacity_percent {
        paint.opacity = opacity as f32;
    }
    paint.visible = snapshot.visible;
    if let Some(blend) = snapshot.blend {
        paint.blend_mode = design_blend_mode(blend);
    }
    paint
}

fn design_solid_paint(color: FantaColor) -> DesignPaint {
    let mut paint = DesignPaint::solid(design_color(color));
    // The engine folds a solid's opacity into its alpha channel; the panel
    // separates them. Presenting alpha as paint opacity keeps the round trip
    // lossless because `PaintOpacity` edits write the alpha channel back.
    paint.opacity = f32::from(color.a) / 255.0 * 100.0;
    paint
}

fn gradient_stops(gradient: &Gradient) -> Vec<DesignGradientStop> {
    let stops = match gradient {
        Gradient::Linear { stops, .. }
        | Gradient::Radial { stops, .. }
        | Gradient::Angular { stops, .. }
        | Gradient::Diamond { stops, .. } => stops,
    };
    stops
        .iter()
        .map(|stop| DesignGradientStop::new(stop.position, design_color(stop.color)))
        .collect()
}

/// The concatenated effect list: shadows first, then blurs, with ids that
/// address the exact engine list an edit must mutate.
fn design_effects(node: &fanta_doc::CanvasNode) -> Vec<DesignEffect> {
    let mut effects = Vec::with_capacity(node.effects.len() + node.blurs.len());
    for (index, shadow) in node.effects.iter().enumerate() {
        let effect = match shadow.kind {
            ShadowKind::Drop => {
                let mut effect = DesignEffect::drop_shadow(
                    design_color(shadow.color),
                    shadow.blur as f32,
                    shadow.spread as f32,
                    shadow.offset[0] as f32,
                    shadow.offset[1] as f32,
                );
                if let DesignEffectSettings::DropShadow(settings) = &mut effect.settings {
                    settings.show_behind_node = shadow.show_behind_node;
                }
                effect
            }
            ShadowKind::Inner => {
                let mut effect = DesignEffect::new(DesignEffectKind::InnerShadow);
                if let DesignEffectSettings::InnerShadow(settings) = &mut effect.settings {
                    settings.color = design_color(shadow.color);
                    settings.offset.x = shadow.offset[0] as f32;
                    settings.offset.y = shadow.offset[1] as f32;
                    settings.radius = shadow.blur as f32;
                    settings.spread = shadow.spread as f32;
                }
                effect.sync_compatibility_summary();
                effect
            }
        };
        effects.push(effect.with_id(format!("shadow-{index}")));
    }
    for (index, blur) in node.blurs.iter().enumerate() {
        let kind = match blur.kind {
            BlurKind::Layer => DesignEffectKind::LayerBlur,
            BlurKind::Background => DesignEffectKind::BackgroundBlur,
        };
        let mut effect = DesignEffect::new(kind);
        match &mut effect.settings {
            DesignEffectSettings::LayerBlur(settings)
            | DesignEffectSettings::BackgroundBlur(settings) => {
                settings.set_end_radius(blur.radius as f32);
            }
            _ => {}
        }
        effect.sync_compatibility_summary();
        effects.push(effect.with_id(format!("blur-{index}")));
    }
    effects
}

/// Which engine effect list an effect id addresses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EffectRef {
    Shadow(usize),
    Blur(usize),
}

fn effect_ref(effect_id: &str) -> Option<EffectRef> {
    if let Some(index) = effect_id.strip_prefix("shadow-") {
        return index.parse().ok().map(EffectRef::Shadow);
    }
    if let Some(index) = effect_id.strip_prefix("blur-") {
        return index.parse().ok().map(EffectRef::Blur);
    }
    None
}

fn design_layout(section: &NodeSection) -> Option<DesignLayout> {
    let layout = section.layout.as_ref()?;
    let mut out = DesignLayout {
        clip_content: layout.clip,
        ..DesignLayout::default()
    };
    out.mode = DesignLayoutMode::None;
    out.item = DesignAutoLayoutItem::default();
    let Some(auto) = layout.auto_layout.as_ref() else {
        return Some(out);
    };
    out.mode = match auto.mode {
        LayoutMode::Horizontal => DesignLayoutMode::Horizontal,
        LayoutMode::Vertical => DesignLayoutMode::Vertical,
    };
    let (primary_gap, counter_gap) = match auto.mode {
        LayoutMode::Horizontal => (auto.gap_h, auto.gap_v),
        LayoutMode::Vertical => (auto.gap_v, auto.gap_h),
    };
    out.gap = primary_gap as f32;
    out.counter_axis_gap = auto.wrap.then_some(counter_gap as f32);
    out.wrap = auto.wrap;
    let sizing = |axis: fanta_doc::AxisSizing| match axis {
        fanta_doc::AxisSizing::Fixed => DesignSizingMode::Fixed,
        fanta_doc::AxisSizing::Hug => DesignSizingMode::Hug,
    };
    let (horizontal, vertical) = match auto.mode {
        LayoutMode::Horizontal => (auto.primary_sizing, auto.counter_sizing),
        LayoutMode::Vertical => (auto.counter_sizing, auto.primary_sizing),
    };
    out.horizontal_sizing = sizing(horizontal);
    out.vertical_sizing = sizing(vertical);
    // The snapshot collapses per-side padding to H/V pairs and reads `None`
    // for a mixed pair; keep the readout mechanical by falling back to zero
    // (the panel shows mixed states through property value states, which
    // wave 1 does not supply for padding).
    let pad_h = auto.pad_h.unwrap_or(0.0) as f32;
    let pad_v = auto.pad_v.unwrap_or(0.0) as f32;
    out.padding = [pad_v, pad_h, pad_v, pad_h];
    out.item.min_width = auto.min_width.map(|value| value as f32);
    out.item.max_width = auto.max_width.map(|value| value as f32);
    out.item.min_height = auto.min_height.map(|value| value as f32);
    out.item.max_height = auto.max_height.map(|value| value as f32);
    Some(out)
}

fn design_typography(snapshot: &TypographySnapshot) -> DesignTypography {
    DesignTypography {
        family: SharedString::from(snapshot.font_family.clone()),
        style: if snapshot.italic {
            "Italic".into()
        } else {
            "Regular".into()
        },
        weight: f32::from(snapshot.weight),
        size: snapshot.size_px as f32,
        line_height: DesignLineHeight::Percent((snapshot.line_height * 100.0) as f32),
        letter_spacing: DesignLetterSpacing::Pixels(snapshot.letter_spacing as f32),
        horizontal_alignment: match snapshot.align {
            TextAlign::Left => DesignTextHorizontalAlignment::Left,
            TextAlign::Center => DesignTextHorizontalAlignment::Center,
            TextAlign::Right => DesignTextHorizontalAlignment::Right,
            TextAlign::Justify => DesignTextHorizontalAlignment::Justified,
        },
        vertical_alignment: match snapshot.vertical_align {
            TextVAlign::Top => DesignTextVerticalAlignment::Top,
            TextVAlign::Center => DesignTextVerticalAlignment::Center,
            TextVAlign::Bottom => DesignTextVerticalAlignment::Bottom,
        },
        resize: match snapshot.auto_resize {
            TextAutoResize::None => DesignTextResize::Fixed,
            TextAutoResize::WidthAndHeight => DesignTextResize::AutoWidth,
            TextAutoResize::Height => DesignTextResize::AutoHeight,
        },
        decoration: if snapshot.underline {
            DesignTextDecoration::Underline
        } else if snapshot.strikethrough {
            DesignTextDecoration::Strikethrough
        } else {
            DesignTextDecoration::None
        },
        ..DesignTypography::default()
    }
}

fn design_stroke(
    id: NodeId,
    kind: DesignPanelNodeKind,
    node: &fanta_doc::CanvasNode,
    snapshots: &[PaintSnapshot],
) -> Option<DesignStroke> {
    let strokes = match &node.data {
        NodeData::Vector(vector) => &vector.strokes,
        NodeData::Group(group) => &group.strokes,
        _ => return None,
    };
    if strokes.is_empty() {
        return None;
    }
    let first = strokes.first()?;
    let paints = snapshots
        .iter()
        .enumerate()
        .map(|(index, snapshot)| design_paint(id, "stroke", index, snapshot))
        .collect();
    let mut stroke = DesignStroke::for_node(
        kind,
        DesignPaint::solid(DesignColor::BLACK),
        first.width as f32,
        match first.align {
            StrokeAlign::Center => DesignStrokeAlign::Center,
            StrokeAlign::Inside => DesignStrokeAlign::Inside,
            StrokeAlign::Outside => DesignStrokeAlign::Outside,
        },
    );
    stroke.paints = paints;
    if let Some([top, right, bottom, left]) = first.per_side {
        stroke.weights = DesignStrokeWeights {
            mode: DesignStrokeWeightMode::Custom,
            top: top as f32,
            right: right as f32,
            bottom: bottom as f32,
            left: left as f32,
        };
    }
    let cap = match first.cap {
        StrokeCap::Butt => DesignStrokeCap::None,
        StrokeCap::Round => DesignStrokeCap::Round,
        StrokeCap::Square => DesignStrokeCap::Square,
    };
    stroke.start_cap = cap;
    stroke.end_cap = cap;
    stroke.endpoint_cap = cap;
    stroke.join = match first.join {
        StrokeJoin::Miter => DesignStrokeJoin::Miter,
        StrokeJoin::Round => DesignStrokeJoin::Round,
        StrokeJoin::Bevel => DesignStrokeJoin::Bevel,
    };
    stroke.dashes = if first.dash.is_empty() {
        DesignStrokeDashes::solid()
    } else {
        DesignStrokeDashes {
            mode: DesignStrokeDashMode::Custom,
            pattern: first.dash.iter().map(|value| *value as f32).collect(),
        }
    };
    Some(stroke)
}

/// Honest capability gates for wave 1: what the adapter has not wired stays
/// off even when the coarse kind preset would advertise it.
fn gate_capabilities(
    kind: DesignPanelNodeKind,
    is_mask: bool,
    corner_capabilities: DesignCornerCapabilities,
) -> DesignPanelNodeCapabilities {
    let mut capabilities = DesignPanelNodeCapabilities::for_node_kind(kind);
    capabilities.sections.retain(|section| {
        !matches!(
            section,
            DesignPanelSection::LayoutGrid
                | DesignPanelSection::Export
                | DesignPanelSection::Selection
        )
    });
    if is_mask && !capabilities.sections.contains(&DesignPanelSection::Mask) {
        capabilities.sections.push(DesignPanelSection::Mask);
    }
    capabilities.aspect_ratio_lock = false;
    capabilities.constraints = false;
    capabilities.arrange = false;
    capabilities.transforms = false;
    capabilities.resize_to_fit = false;
    capabilities.add_auto_layout = false;
    capabilities.grid_auto_layout = false;
    capabilities.layout_guides = false;
    capabilities.pass_through_blend = matches!(
        kind,
        DesignPanelNodeKind::Frame | DesignPanelNodeKind::Group | DesignPanelNodeKind::Component
    );
    capabilities.clip_content = matches!(
        kind,
        DesignPanelNodeKind::Frame | DesignPanelNodeKind::Group | DesignPanelNodeKind::Component
    );
    if kind == DesignPanelNodeKind::Other {
        // The engine can move, hide, and re-composite these nodes but cannot
        // edit their content: wrapper-level surfaces only.
        capabilities.sections = vec![DesignPanelSection::Position, DesignPanelSection::Layer];
        capabilities.fill = false;
        capabilities.stroke = false;
        capabilities.effects = false;
        capabilities.auto_layout_container = false;
        capabilities.layer_appearance = true;
        capabilities.visibility = true;
    }
    let _ = corner_capabilities;
    capabilities
}

/// The complete controlled read model for one selected node.
pub(crate) fn design_node(
    document: &FigDocument,
    id: NodeId,
    masters: &HashMap<NodeId, ComponentId>,
) -> Option<DesignPanelNode> {
    let doc = &document.doc;
    let section = node_section(doc, id, masters)?;
    let node = doc.scene.get(id)?;
    let kind = design_kind(id, &node.data, masters);
    let mut out = DesignPanelNode::new(
        SharedString::from(id.to_string()),
        SharedString::from(section.name.clone()),
        kind,
    );
    // Scrub the representative preset: every field below is host-authored.
    out.fills.clear();
    out.stroke = None;
    out.effects.clear();
    out.layout_grids.clear();
    out.export_settings.clear();
    out.component_context = None;
    out.component_properties.clear();
    out.transform_modifiers.clear();
    out.typography = None;
    out.layout = None;
    out.fill_shows_in_exports = None;

    out.visible = section.visible;
    out.x = section.x as f32;
    out.y = section.y as f32;
    out.width = section.width as f32;
    out.height = section.height as f32;
    out.rotation = section.rotation_degrees as f32;
    out.lock_aspect_ratio = false;
    out.opacity = section.opacity_percent as f32;
    out.blend_mode = display_blend_mode(kind, node);

    match section.corner_radius {
        CornerRadiusValue::NotApplicable => {
            out.corner_radii = [0.0; 4];
            out.independent_corners = false;
        }
        CornerRadiusValue::Uniform(radius) => {
            out.corner_radii = [radius as f32; 4];
            out.independent_corners = false;
        }
        CornerRadiusValue::PerCorner(radii) => {
            out.corner_radii = radii.map(|radius| radius as f32);
            out.independent_corners = true;
        }
    }
    out.corner_smoothing = section
        .corner_smoothing
        .map(|percent| (percent / 100.0) as f32)
        .unwrap_or(0.0);
    out.corner_capabilities = DesignCornerCapabilities {
        uniform_radius: !matches!(section.corner_radius, CornerRadiusValue::NotApplicable),
        independent_radii: !matches!(section.corner_radius, CornerRadiusValue::NotApplicable),
        smoothing: section.corner_smoothing.is_some(),
    };

    if let Some(fills) = &section.fills {
        out.fills = fills
            .iter()
            .enumerate()
            .map(|(index, snapshot)| design_paint(id, "fill", index, snapshot))
            .collect();
    } else if let Some(typography) = &section.typography {
        // A text node's Fill section is its glyph color.
        let mut paint = design_solid_paint(typography.color);
        paint.id = SharedString::from(format!("{id}-fill-0"));
        out.fills = vec![paint];
    }
    if let Some(strokes) = &section.strokes {
        out.stroke = design_stroke(id, kind, node, strokes);
    }
    out.effects = design_effects(node);
    out.effect_capabilities.kind_availability = DesignEffectKind::ALL
        .into_iter()
        .map(|kind| {
            if matches!(
                kind,
                DesignEffectKind::DropShadow
                    | DesignEffectKind::InnerShadow
                    | DesignEffectKind::LayerBlur
                    | DesignEffectKind::BackgroundBlur
            ) {
                DesignEffectKindAvailability::available(kind)
            } else {
                DesignEffectKindAvailability::unavailable(
                    kind,
                    "Not supported by this document engine",
                )
            }
        })
        .collect();
    out.layout = design_layout(&section);
    out.typography = section.typography.as_ref().map(design_typography);
    out.is_mask = node.is_mask;
    out.mask_mode = match node.mask_type {
        MaskType::Alpha => DesignMaskType::Alpha,
        MaskType::Luminance => DesignMaskType::Luminance,
    };
    out.mask_type = node
        .is_mask
        .then(|| SharedString::from(out.mask_mode.label()));

    if let Some(instance) = &section.instance {
        out.component_context = Some(DesignComponentContext {
            role: DesignComponentRole::Instance,
            main_component: Some(match instance.main_root {
                Some(root) => DesignComponentReference::local(
                    root.to_string(),
                    instance.component_name.clone(),
                ),
                None => DesignComponentReference::local("", instance.component_name.clone()),
            }),
            description: None,
            documentation_links: Vec::new(),
            overrides: Default::default(),
            authoring: None,
        });
        out.component_properties = instance
            .props
            .iter()
            .map(|prop| {
                let prop_id = SharedString::from(prop.id.to_string());
                match &prop.value {
                    PropValueSnapshot::Bool(value) => {
                        DesignComponentProperty::boolean(prop_id, prop.name.clone(), *value, *value)
                    }
                    PropValueSnapshot::Text(value) => DesignComponentProperty::text(
                        prop_id,
                        prop.name.clone(),
                        value.clone(),
                        value.clone(),
                    ),
                    PropValueSnapshot::Number(value) => DesignComponentProperty::text(
                        prop_id,
                        prop.name.clone(),
                        format_number(*value),
                        format_number(*value),
                    ),
                    // Color and display-only props are inspectable; their
                    // edits are rejected by the strict kind guard in
                    // `handle_design_component_prop`.
                    PropValueSnapshot::Color(color) => DesignComponentProperty::text(
                        prop_id,
                        prop.name.clone(),
                        design_color(*color).hex(),
                        design_color(*color).hex(),
                    ),
                    PropValueSnapshot::Display(value) => DesignComponentProperty::text(
                        prop_id,
                        prop.name.clone(),
                        value.clone(),
                        value.clone(),
                    ),
                }
            })
            .collect();
    }

    out.capabilities = Some(gate_capabilities(
        kind,
        node.is_mask,
        out.corner_capabilities,
    ));
    Some(out)
}

/// Pass through is modeled from `ISOLATED_BLEND` inverted: a Normal-blend
/// container that does NOT isolate presents as Figma's Pass through.
fn display_blend_mode(kind: DesignPanelNodeKind, node: &fanta_doc::CanvasNode) -> DesignBlendMode {
    let container_pass_through = matches!(
        kind,
        DesignPanelNodeKind::Frame | DesignPanelNodeKind::Group | DesignPanelNodeKind::Component
    ) && node.blend_mode == BlendMode::Normal
        && !node.flags.contains(NodeFlags::ISOLATED_BLEND);
    if container_pass_through {
        DesignBlendMode::PassThrough
    } else {
        design_blend_mode(node.blend_mode)
    }
}

fn parent_layout_for(doc: &Doc, id: NodeId) -> DesignPanelParentLayout {
    let Some(node) = doc.scene.get(id) else {
        return DesignPanelParentLayout::Canvas;
    };
    let Some(parent) = node.parent.and_then(|parent| doc.scene.get(parent)) else {
        return DesignPanelParentLayout::Canvas;
    };
    let NodeData::Group(group) = &parent.data else {
        return DesignPanelParentLayout::Freeform;
    };
    let Some(layout) = &group.auto_layout else {
        return DesignPanelParentLayout::Freeform;
    };
    let direction = match layout.mode {
        LayoutMode::Horizontal => DesignPanelAutoLayoutDirection::Horizontal,
        LayoutMode::Vertical => DesignPanelAutoLayoutDirection::Vertical,
    };
    let wrap = if layout.wrap {
        DesignPanelAutoLayoutWrap::Wrap
    } else {
        DesignPanelAutoLayoutWrap::NoWrap
    };
    let participation = if node
        .layout_child
        .as_ref()
        .is_some_and(|child| child.absolute)
    {
        DesignPanelAutoLayoutParticipation::Ignored
    } else {
        DesignPanelAutoLayoutParticipation::InFlow
    };
    DesignPanelParentLayout::auto_layout(direction, wrap, participation)
}

/// A minimal member node for the non-aggregate tail of a multiple selection:
/// only its identity participates in target validation.
fn member_node(doc: &Doc, id: NodeId, masters: &HashMap<NodeId, ComponentId>) -> DesignPanelNode {
    let (name, kind) = doc
        .scene
        .get(id)
        .map(|node| (node.name.clone(), design_kind(id, &node.data, masters)))
        .unwrap_or_else(|| (String::new(), DesignPanelNodeKind::Other));
    let mut node = DesignPanelNode::new(SharedString::from(id.to_string()), name, kind);
    node.fills.clear();
    node.stroke = None;
    node.effects.clear();
    node.layout_grids.clear();
    node.export_settings.clear();
    node
}

type PropertyStates = Vec<(DesignPanelProperty, DesignPanelPropertyValueState)>;

/// Aggregate visual model + mixed/uniform states for a multiple selection.
fn aggregate_selection(
    document: &FigDocument,
    ids: &[NodeId],
    masters: &HashMap<NodeId, ComponentId>,
) -> (DesignPanelNode, PropertyStates) {
    let doc = &document.doc;
    let multi = multi_section(doc, ids, masters);
    let first = ids.first().copied().unwrap_or_else(NodeId::new);
    let mut node = DesignPanelNode::new(
        SharedString::from(first.to_string()),
        format!("{} selected", multi.count),
        DesignPanelNodeKind::MultipleSelection,
    );
    node.fills.clear();
    node.stroke = None;
    node.effects.clear();
    node.layout_grids.clear();
    node.export_settings.clear();
    node.component_context = None;
    node.component_properties.clear();
    node.typography = None;
    node.layout = None;
    node.x = multi.x.unwrap_or(0.0) as f32;
    node.y = multi.y.unwrap_or(0.0) as f32;
    node.width = multi.width.unwrap_or(0.0) as f32;
    node.height = multi.height.unwrap_or(0.0) as f32;
    node.rotation = multi.rotation_degrees.unwrap_or(0.0) as f32;
    let mut capabilities =
        DesignPanelNodeCapabilities::for_node_kind(DesignPanelNodeKind::MultipleSelection);
    capabilities.sections = vec![DesignPanelSection::Position, DesignPanelSection::Layer];
    capabilities.aspect_ratio_lock = false;
    capabilities.constraints = false;
    capabilities.arrange = false;
    capabilities.transforms = false;
    capabilities.resize_to_fit = false;
    capabilities.add_auto_layout = false;
    capabilities.grid_auto_layout = false;
    capabilities.layout_guides = false;
    capabilities.pass_through_blend = false;
    capabilities.fill = false;
    capabilities.stroke = false;
    capabilities.effects = false;
    capabilities.auto_layout_container = false;
    node.capabilities = Some(capabilities);

    let state = |value: Option<f64>| match value {
        Some(value) => {
            DesignPanelPropertyValueState::Uniform(DesignPanelValue::Number(value as f32))
        }
        None => DesignPanelPropertyValueState::Mixed,
    };
    let states = vec![
        (DesignPanelProperty::X, state(multi.x)),
        (DesignPanelProperty::Y, state(multi.y)),
        (DesignPanelProperty::Width, state(multi.width)),
        (DesignPanelProperty::Height, state(multi.height)),
        (DesignPanelProperty::Rotation, state(multi.rotation_degrees)),
    ];
    (node, states)
}

/// Bound-state projection for the whole-node `BoundProp`s the panel displays.
fn bound_states(doc: &Doc, id: NodeId) -> PropertyStates {
    let Some(node) = doc.scene.get(id) else {
        return Vec::new();
    };
    let mut states = Vec::new();
    for (prop, variable_id) in &node.bindings {
        let property = match prop {
            BoundProp::Opacity => DesignPanelProperty::Opacity,
            BoundProp::Visible => DesignPanelProperty::Visible,
            BoundProp::CornerRadius => DesignPanelProperty::CornerRadius,
            _ => continue,
        };
        let name = doc
            .variables
            .variable(*variable_id)
            .map(|variable| variable.name.clone())
            .unwrap_or_else(|| "Missing variable".to_string());
        let resolved = match property {
            DesignPanelProperty::Opacity => DesignPanelValue::Number(node.opacity.get() * 100.0),
            DesignPanelProperty::Visible => {
                DesignPanelValue::Bool(!node.flags.contains(NodeFlags::HIDDEN))
            }
            _ => DesignPanelValue::Number(0.0),
        };
        states.push((
            property,
            DesignPanelPropertyValueState::bound(
                fanta_gpui::design::DesignPanelPropertyBinding::new(
                    variable_id.to_string(),
                    name,
                    fanta_gpui::design::DesignPanelBindingKind::Variable,
                    resolved,
                ),
            ),
        ));
    }
    states
}

// =============================================================================
// Adapter state
// =============================================================================

/// The last state echoed into the panel; a matching key skips the rebuild.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct DesignEchoKey {
    selection: Vec<NodeId>,
    generation: u64,
    editable: bool,
    page_index: Option<usize>,
}

/// An in-flight Begin/Preview/Commit/Cancel property gesture: the pre-gesture
/// node state, restored before commit so one gesture is one undo step.
pub(crate) struct DesignEditSession {
    pub node: NodeId,
    pub snapshot: NodeSnapshot,
}

pub(crate) struct DesignAdapter {
    pub panel: Entity<DesignPanel>,
    pub(crate) last_echo: Option<DesignEchoKey>,
    pub(crate) session: Option<DesignEditSession>,
    _subscription: Subscription,
}

impl DesignAdapter {
    pub(crate) fn new(window: &mut Window, cx: &mut Context<FigView>) -> Self {
        let panel = cx.new(|cx| {
            DesignPanel::new(
                "fig-gpui-design",
                DesignPanelNode::new("fig-gpui-design-empty", "Page", DesignPanelNodeKind::Frame),
                window,
                cx,
            )
        });
        let subscription = cx.subscribe_in(&panel, window, FigView::handle_design_action);
        Self {
            panel,
            last_echo: None,
            session: None,
            _subscription: subscription,
        }
    }
}

// =============================================================================
// Host integration: echo, finish hook, and intents
// =============================================================================

impl FigView {
    /// Echo document state into the DesignPanel: inspection context, property
    /// value states, and Page view data, memoized on (selection identity,
    /// render generation, editability, page).
    pub(crate) fn refresh_gpui_design(&mut self, cx: &mut Context<Self>) {
        if self.gpui_design.is_none() {
            return;
        }
        let item = self.item().clone();
        let page_index = self.selected_page_index();
        let built = {
            let fig_item = item.read(cx);
            let Some(document) = fig_item.document() else {
                return;
            };
            let editable = fig_item.is_editable();
            let doc = &document.doc;
            let selection: Vec<NodeId> = doc
                .selection
                .iter()
                .copied()
                .filter(|id| doc.scene.contains(*id))
                .collect();
            let key = DesignEchoKey {
                selection: selection.clone(),
                generation: document.render_generation(),
                editable,
                page_index,
            };
            if self
                .gpui_design
                .as_ref()
                .is_some_and(|adapter| adapter.last_echo.as_ref() == Some(&key))
            {
                return;
            }
            let masters = master_roots(&doc.components);
            let permissions = if editable {
                DesignPanelPermissions::editor()
            } else {
                DesignPanelPermissions::viewer()
            };
            let (context, states) = match selection.as_slice() {
                [] => (DesignPanelInspectionContext::page(permissions), Vec::new()),
                [id] => match design_node(document, *id, &masters) {
                    Some(node) => {
                        let states = bound_states(doc, *id);
                        (
                            DesignPanelInspectionContext::single(
                                node,
                                parent_layout_for(doc, *id),
                                permissions,
                            ),
                            states,
                        )
                    }
                    None => (DesignPanelInspectionContext::page(permissions), Vec::new()),
                },
                ids => {
                    let (aggregate, states) = aggregate_selection(document, ids, &masters);
                    let mut members = ids.iter().skip(1).map(|id| member_node(doc, *id, &masters));
                    let second = members
                        .next()
                        .unwrap_or_else(|| member_node(doc, ids[0], &masters));
                    // Every member's parent layout must agree or the context
                    // degrades to Mixed, matching the panel's contract.
                    let mut layouts = ids.iter().map(|id| parent_layout_for(doc, *id));
                    let first_layout = layouts.next().unwrap_or(DesignPanelParentLayout::Canvas);
                    let parent_layout = if layouts.all(|layout| layout == first_layout) {
                        first_layout
                    } else {
                        DesignPanelParentLayout::Mixed
                    };
                    (
                        DesignPanelInspectionContext::multiple(
                            DesignPanelMultipleSelection::with_remaining(
                                aggregate, second, members,
                            ),
                            parent_layout,
                            permissions,
                        ),
                        states,
                    )
                }
            };
            let page_data = selection.is_empty().then(|| {
                let page = page_section(document, page_index);
                let background = match page.background {
                    Some(crate::properties_snapshot::PageBackgroundValue::Solid(color)) => {
                        DesignPageBackground::new(design_color(color))
                    }
                    Some(_) => DesignPageBackground::new(DesignColor::WHITE)
                        .read_only("Non-solid page backgrounds are edited on canvas"),
                    None => DesignPageBackground::new(design_color(
                        crate::properties_ops::DEFAULT_PAGE_BACKGROUND,
                    )),
                };
                let page_id = page
                    .id
                    .map(|root| root.to_string())
                    .unwrap_or_else(|| format!("page-{}", page_index.unwrap_or(0)));
                DesignPageViewData::canonical(page_id, background)
            });
            Some((key, context, states, page_data))
        };
        let Some((key, context, states, page_data)) = built else {
            return;
        };
        let Some(adapter) = self.gpui_design.as_mut() else {
            return;
        };
        adapter.last_echo = Some(key);
        adapter.panel.update(cx, |panel, cx| {
            if let Some(page_data) = page_data {
                panel.set_page_view_data(page_data, cx);
            }
            panel.set_inspection_context(context, cx);
            panel.set_property_value_states(states, cx);
        });
    }

    /// Cancels any in-flight DesignPanel preview gesture, restoring the
    /// Begin snapshot — the transaction boundary `finish_panel_edits` needs
    /// before an external mutation lands.
    pub(crate) fn finish_gpui_design_edits(&mut self, cx: &mut Context<Self>) {
        let Some(session) = self
            .gpui_design
            .as_mut()
            .and_then(|adapter| adapter.session.take())
        else {
            return;
        };
        let item = self.item().clone();
        item.update(cx, |item, cx| {
            item.with_document(cx, |document| {
                restore_snapshot(&mut document.doc, &session.snapshot);
                ((), DocChange::ContentPreview)
            });
            item.finish_content_preview(false, cx);
        });
    }

    /// Applies committed operations through the item: one op directly, many
    /// as a single history transaction (one undo step).
    fn design_apply_ops(&mut self, operations: Vec<Operation>, cx: &mut Context<Self>) -> bool {
        let operations = finite_transform_operations(operations);
        if operations.is_empty() {
            return false;
        }
        let item = self.item().clone();
        item.update(cx, |item, cx| {
            if !item.is_editable() {
                return false;
            }
            if operations.len() == 1 {
                let mut applied = false;
                for operation in operations {
                    match item.apply(operation, cx) {
                        Ok(()) => applied = true,
                        Err(error) => {
                            log::error!("fig design adapter failed to apply operation: {error:#}")
                        }
                    }
                }
                applied
            } else {
                let label = operations
                    .first()
                    .map(|operation| operation.label().to_string())
                    .unwrap_or_else(|| "Edit".to_string());
                let applied = item.with_document(cx, |document| {
                    let doc = &mut document.doc;
                    doc.history.begin(label, &mut doc.scene);
                    for operation in operations {
                        if let Err(error) = doc.apply(operation) {
                            log::error!("fig design adapter failed to apply operation: {error:#}");
                            break;
                        }
                    }
                    doc.history.commit(&mut doc.scene);
                    ((), DocChange::Content)
                });
                applied.is_some()
            }
        })
    }

    /// Builds committed operations against the current document.
    fn design_ops(
        &mut self,
        cx: &mut Context<Self>,
        build: impl FnOnce(&Doc) -> Vec<Operation>,
    ) -> Vec<Operation> {
        let item = self.item().clone();
        let item = item.read(cx);
        if !item.is_editable() {
            return Vec::new();
        }
        let Some(document) = item.document() else {
            return Vec::new();
        };
        build(&document.doc)
    }

    /// The typed intent seam: maps panel actions onto document operations.
    #[allow(deprecated)]
    pub(crate) fn handle_design_action(
        &mut self,
        _panel: &Entity<DesignPanel>,
        action: &DesignPanelAction,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match action {
            DesignPanelAction::PropertyChangeRequested {
                node_id: id,
                property,
                value,
            } => {
                let Some(id) = node_id(id) else {
                    return;
                };
                self.finish_document_edits_for_external_change(cx);
                let ops = self.design_ops(cx, |doc| {
                    property_operations(doc, id, *property, value).unwrap_or_else(|| {
                        log::debug!("fig design adapter: unhandled property {property:?}");
                        Vec::new()
                    })
                });
                self.design_apply_ops(ops, cx);
            }
            DesignPanelAction::PropertyEditRequested {
                node_id: id,
                property,
                value,
                phase,
            } => {
                let Some(id) = node_id(id) else {
                    return;
                };
                self.handle_design_phased_edit(id, *property, value, *phase, cx);
            }
            DesignPanelAction::TargetedNodeActionRequested { target, action } => {
                self.handle_design_targeted_action(target, action, cx);
            }
            DesignPanelAction::PaintEditRequested {
                node_id: id,
                collection,
                paint_id: _,
                index,
                edit,
                phase,
                ..
            } => {
                let Some(id) = node_id(id) else {
                    return;
                };
                self.handle_design_paint_edit(id, *collection, *index, edit, *phase, cx);
            }
            DesignPanelAction::PaintChangeRequested {
                node_id: id,
                collection,
                index,
                paint,
                ..
            } => {
                // Compatibility whole-paint path: only the solid color case.
                let Some(id) = node_id(id) else {
                    return;
                };
                let is_stroke = *collection == DesignPanelCollection::Stroke;
                let color = fanta_color(paint.color);
                self.finish_document_edits_for_external_change(cx);
                let index = *index;
                let ops = self.design_ops(cx, |doc| {
                    solid_paint_color_operations(doc, id, is_stroke, index, color)
                });
                self.design_apply_ops(ops, cx);
            }
            DesignPanelAction::PaintReorderRequested {
                node_id: id,
                collection,
                from_index,
                to_index,
                ..
            } => {
                let Some(id) = node_id(id) else {
                    return;
                };
                let is_stroke = *collection == DesignPanelCollection::Stroke;
                let (from, to) = (*from_index, *to_index);
                self.finish_document_edits_for_external_change(cx);
                let ops = self.design_ops(cx, |doc| {
                    replace_data_operation(doc, id, |data| {
                        if is_stroke {
                            if let Some(strokes) = stroke_list_mut(data)
                                && from < strokes.len()
                                && to < strokes.len()
                            {
                                let stroke = strokes.remove(from);
                                strokes.insert(to, stroke);
                            }
                        } else if let NodeData::Vector(vector) = data
                            && from < vector.fills.len()
                            && to < vector.fills.len()
                        {
                            let fill = vector.fills.remove(from);
                            vector.fills.insert(to, fill);
                        }
                    })
                });
                self.design_apply_ops(ops, cx);
            }
            DesignPanelAction::CollectionItemAddRequested {
                node_id: id,
                collection,
                ..
            } => {
                let Some(id) = node_id(id) else {
                    return;
                };
                let is_stroke = *collection == DesignPanelCollection::Stroke;
                if *collection != DesignPanelCollection::Fill && !is_stroke {
                    return;
                }
                self.finish_document_edits_for_external_change(cx);
                let ops = self.design_ops(cx, |doc| {
                    replace_data_operation(doc, id, |data| {
                        if is_stroke {
                            if let Some(strokes) = stroke_list_mut(data) {
                                strokes.push(fanta_doc::Stroke::solid(FantaColor::BLACK, 1.0));
                            }
                        } else {
                            crate::properties_ops::add_fill(data);
                        }
                    })
                });
                self.design_apply_ops(ops, cx);
            }
            DesignPanelAction::CollectionItemRemoveRequested {
                node_id: id,
                collection,
                index,
                ..
            } => {
                let Some(id) = node_id(id) else {
                    return;
                };
                let is_stroke = *collection == DesignPanelCollection::Stroke;
                if *collection != DesignPanelCollection::Fill && !is_stroke {
                    return;
                }
                let index = *index;
                self.finish_document_edits_for_external_change(cx);
                let ops = self.design_ops(cx, |doc| {
                    replace_data_operation(doc, id, |data| {
                        if is_stroke {
                            if let Some(strokes) = stroke_list_mut(data)
                                && index < strokes.len()
                            {
                                strokes.remove(index);
                            }
                        } else {
                            crate::properties_ops::remove_fill(data, index);
                        }
                    })
                });
                self.design_apply_ops(ops, cx);
            }
            DesignPanelAction::EffectAddRequested { node_id: id, kind } => {
                let Some(id) = node_id(id) else {
                    return;
                };
                let kind = *kind;
                self.finish_document_edits_for_external_change(cx);
                let ops = self.design_ops(cx, |doc| match kind {
                    DesignEffectKind::DropShadow => effects_operations(doc, id, |effects| {
                        effects.push(default_shadow());
                    }),
                    DesignEffectKind::InnerShadow => effects_operations(doc, id, |effects| {
                        effects.push(Shadow {
                            kind: ShadowKind::Inner,
                            ..default_shadow()
                        });
                    }),
                    DesignEffectKind::LayerBlur => blurs_operations(doc, id, |blurs| {
                        blurs.push(default_blur(BlurKind::Layer));
                    }),
                    DesignEffectKind::BackgroundBlur => blurs_operations(doc, id, |blurs| {
                        blurs.push(default_blur(BlurKind::Background));
                    }),
                    _ => Vec::new(),
                });
                self.design_apply_ops(ops, cx);
            }
            DesignPanelAction::EffectRemoveRequested {
                node_id: id,
                effect_id,
                ..
            } => {
                let Some(id) = node_id(id) else {
                    return;
                };
                let Some(reference) = effect_ref(effect_id) else {
                    return;
                };
                self.finish_document_edits_for_external_change(cx);
                let ops = self.design_ops(cx, |doc| match reference {
                    EffectRef::Shadow(index) => effects_operations(doc, id, |effects| {
                        if index < effects.len() {
                            effects.remove(index);
                        }
                    }),
                    EffectRef::Blur(index) => blurs_operations(doc, id, |blurs| {
                        if index < blurs.len() {
                            blurs.remove(index);
                        }
                    }),
                });
                self.design_apply_ops(ops, cx);
            }
            DesignPanelAction::EffectEditRequested {
                node_id: id,
                effect_id,
                property,
                value,
                phase,
                ..
            } => {
                let Some(id) = node_id(id) else {
                    return;
                };
                let Some(reference) = effect_ref(effect_id) else {
                    return;
                };
                self.handle_design_effect_edit(id, reference, *property, value, *phase, cx);
            }
            DesignPanelAction::ComponentPropertyChangeRequested {
                node_id: id,
                property_id,
                value,
            } => {
                self.handle_design_component_prop(id, property_id, Some(value), cx);
            }
            DesignPanelAction::ComponentPropertyEditRequested {
                node_id: id,
                property_id,
                value,
                phase,
            } => {
                if *phase == DesignPanelEditPhase::Commit {
                    self.handle_design_component_prop(id, property_id, Some(value), cx);
                }
            }
            DesignPanelAction::ComponentPropertyResetRequested {
                node_id: id,
                property_id,
            } => {
                self.handle_design_component_prop(id, property_id, None, cx);
            }
            DesignPanelAction::DetachInstanceRequested { node_id: id } => {
                let Some(id) = node_id(id) else {
                    return;
                };
                self.finish_document_edits_for_external_change(cx);
                let ops = self.design_ops(cx, |doc| detach_instance_operations(doc, id));
                self.design_apply_ops(ops, cx);
            }
            DesignPanelAction::GoToMainComponentRequested { node_id: id } => {
                let Some(id) = node_id(id) else {
                    return;
                };
                let item = self.item().clone();
                item.update(cx, |item, cx| {
                    item.with_document(cx, |document| {
                        let main_root = match document.doc.scene.get(id).map(|node| &node.data) {
                            Some(NodeData::Instance(instance)) => document
                                .doc
                                .components
                                .def(instance.component)
                                .map(|def| def.root),
                            _ => None,
                        };
                        let change = match main_root {
                            Some(root) if document.doc.scene.contains(root) => {
                                document.doc.selection.replace_with([root]);
                                DocChange::Selection
                            }
                            _ => DocChange::None,
                        };
                        ((), change)
                    });
                });
            }
            DesignPanelAction::PageBackgroundChangeRequested { page_id, color } => {
                self.handle_design_page_background(
                    page_id,
                    *color,
                    DesignPanelEditPhase::Commit,
                    cx,
                );
            }
            DesignPanelAction::PageBackgroundEditRequested {
                page_id,
                color,
                phase,
            } => {
                self.handle_design_page_background(page_id, *color, *phase, cx);
            }
            DesignPanelAction::PropertyVariableDetachRequested {
                node_id: id,
                target,
                ..
            } => {
                let Some(id) = node_id(id) else {
                    return;
                };
                let prop = match target.property {
                    DesignPanelProperty::Opacity => BoundProp::Opacity,
                    DesignPanelProperty::Visible => BoundProp::Visible,
                    DesignPanelProperty::CornerRadius => BoundProp::CornerRadius,
                    property => {
                        log::debug!(
                            "fig design adapter: unhandled variable detach for {property:?}"
                        );
                        return;
                    }
                };
                self.finish_document_edits_for_external_change(cx);
                let ops = self.design_ops(cx, |doc| {
                    crate::variable_binding::unbind_property_operation(doc, id, prop)
                        .ok()
                        .flatten()
                        .into_iter()
                        .collect()
                });
                self.design_apply_ops(ops, cx);
            }
            DesignPanelAction::SurfaceChangeRequested { requested, .. } => {
                // Design is the only wired surface; echo the accepted value
                // straight back for the editable set.
                let Some(adapter) = self.gpui_design.as_ref() else {
                    return;
                };
                let requested = *requested;
                adapter.panel.update(cx, |panel, cx| {
                    panel.set_active_surface(requested, cx);
                });
            }
            _ => {
                log::debug!("fig design adapter: unhandled action {action:?}");
            }
        }
    }

    /// Validates the exact ordered multi-node target against the document
    /// selection, then applies the nested leaf per member as one transaction.
    fn handle_design_targeted_action(
        &mut self,
        target: &DesignPanelTarget,
        action: &DesignPanelAction,
        cx: &mut Context<Self>,
    ) {
        if !self.design_target_matches_selection(target, cx) {
            log::warn!("fig design adapter: rejecting stale multi-node target");
            return;
        }
        let DesignPanelTarget::Nodes { node_ids } = target else {
            return;
        };
        let ids: Vec<NodeId> = node_ids.iter().filter_map(node_id).collect();
        let (property, value) = match action {
            DesignPanelAction::PropertyChangeRequested {
                property, value, ..
            } => (*property, value.clone()),
            DesignPanelAction::PropertyEditRequested {
                property,
                value,
                phase,
                ..
            } => {
                if *phase != DesignPanelEditPhase::Commit {
                    // Multi-node gestures are commit-only in wave 1: the
                    // document changes once, atomically.
                    return;
                }
                (*property, value.clone())
            }
            _ => {
                log::debug!("fig design adapter: unhandled targeted leaf {action:?}");
                return;
            }
        };
        self.finish_document_edits_for_external_change(cx);
        let ops = self.design_ops(cx, |doc| {
            ids.iter()
                .flat_map(|id| property_operations(doc, *id, property, &value).unwrap_or_default())
                .collect()
        });
        self.design_apply_ops(ops, cx);
    }

    fn design_target_matches_selection(
        &self,
        target: &DesignPanelTarget,
        cx: &Context<Self>,
    ) -> bool {
        let DesignPanelTarget::Nodes { node_ids } = target else {
            return false;
        };
        let item = self.item().read(cx);
        let Some(document) = item.document() else {
            return false;
        };
        let selection: Vec<NodeId> = document.doc.selection.iter().copied().collect();
        let target_ids: Vec<NodeId> = node_ids.iter().filter_map(node_id).collect();
        target_ids.len() == node_ids.len() && target_ids == selection
    }

    /// One Begin/Preview/Commit/Cancel gesture over a single node property,
    /// using the same snapshot-restore staging as the legacy panel scrubs.
    fn handle_design_phased_edit(
        &mut self,
        id: NodeId,
        property: DesignPanelProperty,
        value: &DesignPanelValue,
        phase: DesignPanelEditPhase,
        cx: &mut Context<Self>,
    ) {
        match phase {
            DesignPanelEditPhase::Begin => {
                self.finish_document_edits_for_external_change(cx);
                let item = self.item().clone();
                let snapshot = {
                    let item = item.read(cx);
                    if !item.is_editable() {
                        return;
                    }
                    let Some(node) = item
                        .document()
                        .and_then(|document| document.doc.scene.get(id))
                    else {
                        return;
                    };
                    NodeSnapshot {
                        id,
                        transform: node.transform,
                        opacity: node.opacity,
                        data: Box::new(node.data.clone()),
                        effects: node.effects.clone(),
                        blurs: node.blurs.clone(),
                    }
                };
                if let Some(adapter) = self.gpui_design.as_mut() {
                    adapter.session = Some(DesignEditSession { node: id, snapshot });
                }
            }
            DesignPanelEditPhase::Preview => {
                let Some(snapshot) = self
                    .gpui_design
                    .as_ref()
                    .and_then(|adapter| adapter.session.as_ref())
                    .filter(|session| session.node == id)
                    .map(|session| session.snapshot.clone())
                else {
                    return;
                };
                let item = self.item().clone();
                item.update(cx, |item, cx| {
                    if !item.is_editable() {
                        return;
                    }
                    item.with_document(cx, |document| {
                        restore_snapshot(&mut document.doc, &snapshot);
                        let operations = finite_transform_operations(
                            property_operations(&document.doc, id, property, value)
                                .unwrap_or_default(),
                        );
                        for operation in &operations {
                            apply_preview_operation(&mut document.doc, operation);
                        }
                        ((), DocChange::ContentPreview)
                    });
                });
            }
            DesignPanelEditPhase::Commit => {
                let session = self
                    .gpui_design
                    .as_mut()
                    .and_then(|adapter| adapter.session.take())
                    .filter(|session| session.node == id);
                let item = self.item().clone();
                if let Some(session) = &session {
                    item.update(cx, |item, cx| {
                        item.with_document(cx, |document| {
                            restore_snapshot(&mut document.doc, &session.snapshot);
                            ((), DocChange::ContentPreview)
                        });
                    });
                } else {
                    self.finish_document_edits_for_external_change(cx);
                }
                let ops = self.design_ops(cx, |doc| {
                    property_operations(doc, id, property, value).unwrap_or_else(|| {
                        log::debug!("fig design adapter: unhandled property {property:?}");
                        Vec::new()
                    })
                });
                let committed = self.design_apply_ops(ops, cx);
                if session.is_some() {
                    item.update(cx, |item, cx| item.finish_content_preview(committed, cx));
                }
            }
            DesignPanelEditPhase::Cancel => {
                let session = self
                    .gpui_design
                    .as_mut()
                    .and_then(|adapter| adapter.session.take())
                    .filter(|session| session.node == id);
                let Some(session) = session else {
                    return;
                };
                let item = self.item().clone();
                item.update(cx, |item, cx| {
                    item.with_document(cx, |document| {
                        restore_snapshot(&mut document.doc, &session.snapshot);
                        ((), DocChange::ContentPreview)
                    });
                    item.finish_content_preview(false, cx);
                });
            }
        }
    }

    fn handle_design_paint_edit(
        &mut self,
        id: NodeId,
        collection: DesignPanelCollection,
        index: usize,
        edit: &fanta_gpui::design::DesignPaintEdit,
        phase: DesignPanelEditPhase,
        cx: &mut Context<Self>,
    ) {
        let is_stroke = match collection {
            DesignPanelCollection::Fill => false,
            DesignPanelCollection::Stroke => true,
            _ => return,
        };
        let value = match (&edit.property, &edit.value) {
            (DesignPaintProperty::Color, DesignPaintValue::Color(color)) => {
                PaintEditValue::Color(fanta_color(*color))
            }
            (DesignPaintProperty::Opacity, DesignPaintValue::Number(percent)) => {
                PaintEditValue::Opacity(f64::from(*percent))
            }
            _ => {
                log::debug!(
                    "fig design adapter: unhandled paint edit {:?}",
                    edit.property
                );
                return;
            }
        };
        // Route through the shared phased machinery by synthesizing the
        // property key: paint edits reuse a whole-node snapshot, so Begin /
        // Preview / Cancel behave exactly like ordinary property gestures.
        let build = move |doc: &Doc| paint_edit_operations(doc, id, is_stroke, index, &value);
        match phase {
            DesignPanelEditPhase::Begin => {
                self.handle_design_phased_edit(
                    id,
                    DesignPanelProperty::Opacity,
                    &DesignPanelValue::Number(0.0),
                    DesignPanelEditPhase::Begin,
                    cx,
                );
                // Begin captured the snapshot; nothing document-facing yet.
            }
            DesignPanelEditPhase::Preview => {
                let Some(snapshot) = self
                    .gpui_design
                    .as_ref()
                    .and_then(|adapter| adapter.session.as_ref())
                    .filter(|session| session.node == id)
                    .map(|session| session.snapshot.clone())
                else {
                    return;
                };
                let item = self.item().clone();
                item.update(cx, |item, cx| {
                    if !item.is_editable() {
                        return;
                    }
                    item.with_document(cx, |document| {
                        restore_snapshot(&mut document.doc, &snapshot);
                        let operations = build(&document.doc);
                        for operation in &operations {
                            apply_preview_operation(&mut document.doc, operation);
                        }
                        ((), DocChange::ContentPreview)
                    });
                });
            }
            DesignPanelEditPhase::Commit => {
                let session = self
                    .gpui_design
                    .as_mut()
                    .and_then(|adapter| adapter.session.take())
                    .filter(|session| session.node == id);
                let item = self.item().clone();
                if let Some(session) = &session {
                    item.update(cx, |item, cx| {
                        item.with_document(cx, |document| {
                            restore_snapshot(&mut document.doc, &session.snapshot);
                            ((), DocChange::ContentPreview)
                        });
                    });
                } else {
                    self.finish_document_edits_for_external_change(cx);
                }
                let ops = self.design_ops(cx, build);
                let committed = self.design_apply_ops(ops, cx);
                if session.is_some() {
                    item.update(cx, |item, cx| item.finish_content_preview(committed, cx));
                }
            }
            DesignPanelEditPhase::Cancel => {
                self.handle_design_phased_edit(
                    id,
                    DesignPanelProperty::Opacity,
                    &DesignPanelValue::Number(0.0),
                    DesignPanelEditPhase::Cancel,
                    cx,
                );
            }
        }
    }

    fn handle_design_effect_edit(
        &mut self,
        id: NodeId,
        reference: EffectRef,
        property: DesignPanelProperty,
        value: &DesignPanelValue,
        phase: DesignPanelEditPhase,
        cx: &mut Context<Self>,
    ) {
        if phase != DesignPanelEditPhase::Commit && phase != DesignPanelEditPhase::Begin {
            // Effect scrubs preview through the shared property session when
            // one exists; discrete commits are the wave-1 contract otherwise.
            if phase == DesignPanelEditPhase::Cancel {
                self.finish_gpui_design_edits(cx);
            }
            return;
        }
        if phase == DesignPanelEditPhase::Begin {
            self.handle_design_phased_edit(id, property, value, DesignPanelEditPhase::Begin, cx);
            return;
        }
        let session = self
            .gpui_design
            .as_mut()
            .and_then(|adapter| adapter.session.take())
            .filter(|session| session.node == id);
        let item = self.item().clone();
        if let Some(session) = &session {
            item.update(cx, |item, cx| {
                item.with_document(cx, |document| {
                    restore_snapshot(&mut document.doc, &session.snapshot);
                    ((), DocChange::ContentPreview)
                });
            });
        } else {
            self.finish_document_edits_for_external_change(cx);
        }
        let ops = self.design_ops(cx, |doc| {
            effect_edit_operations(doc, id, reference, property, value).unwrap_or_else(|| {
                log::debug!("fig design adapter: unhandled effect edit {property:?}");
                Vec::new()
            })
        });
        let committed = self.design_apply_ops(ops, cx);
        if session.is_some() {
            item.update(cx, |item, cx| item.finish_content_preview(committed, cx));
        }
    }

    fn handle_design_component_prop(
        &mut self,
        id: &SharedString,
        property_id: &SharedString,
        value: Option<&DesignComponentPropertyValue>,
        cx: &mut Context<Self>,
    ) {
        let Some(id) = node_id(id) else {
            return;
        };
        let Ok(prop) = property_id.parse() else {
            return;
        };
        self.finish_document_edits_for_external_change(cx);
        let value = value.cloned();
        let ops = self.design_ops(cx, |doc| {
            // Strict kind guard: an edit only applies when the incoming panel
            // value matches the schema's default value kind, so a text draft
            // can never corrupt a Color or unsupported prop.
            let kind = current_prop_kind(doc, id, prop);
            let new = match &value {
                None => None,
                Some(DesignComponentPropertyValue::Boolean(value)) => {
                    if !matches!(kind, Some(EnginePropKind::Boolean)) {
                        return Vec::new();
                    }
                    Some(VarValue::Boolean { value: *value })
                }
                Some(DesignComponentPropertyValue::Text(text)) => match kind {
                    Some(EnginePropKind::Number) => match text.trim().parse::<f64>() {
                        Ok(value) if value.is_finite() => Some(VarValue::Float { value }),
                        _ => return Vec::new(),
                    },
                    Some(EnginePropKind::Text) => Some(VarValue::String {
                        value: text.to_string(),
                    }),
                    _ => return Vec::new(),
                },
                Some(other) => {
                    log::debug!("fig design adapter: unhandled component prop value {other:?}");
                    return Vec::new();
                }
            };
            instance_prop_operations(doc, id, prop, new)
        });
        self.design_apply_ops(ops, cx);
    }

    fn handle_design_page_background(
        &mut self,
        page_id: &SharedString,
        color: DesignColor,
        phase: DesignPanelEditPhase,
        cx: &mut Context<Self>,
    ) {
        if phase != DesignPanelEditPhase::Commit {
            return;
        }
        let Some(root) = node_id(page_id) else {
            return;
        };
        let color = fanta_color(color);
        self.finish_document_edits_for_external_change(cx);
        let ops = self.design_ops(cx, |doc| {
            replace_data_operation(doc, root, |data| {
                if let NodeData::Group(group) = data {
                    group.background = Some(Fill::solid(color));
                }
            })
        });
        self.design_apply_ops(ops, cx);
    }
}

// =============================================================================
// Intent → operation builders
// =============================================================================

enum PaintEditValue {
    Color(FantaColor),
    Opacity(f64),
}

fn paint_edit_operations(
    doc: &Doc,
    id: NodeId,
    is_stroke: bool,
    index: usize,
    value: &PaintEditValue,
) -> Vec<Operation> {
    // A text node's single Fill row edits the glyph color.
    let is_text = matches!(
        doc.scene.get(id).map(|node| &node.data),
        Some(NodeData::Text(_))
    );
    match value {
        PaintEditValue::Color(color) => {
            if is_text && !is_stroke {
                return field_operations(
                    doc,
                    &InspectorField::TextColor(id),
                    &design_color(*color).hex(),
                );
            }
            solid_paint_color_operations(doc, id, is_stroke, index, *color)
        }
        PaintEditValue::Opacity(percent) => field_operations(
            doc,
            &InspectorField::PaintOpacity {
                id,
                index,
                is_stroke,
            },
            &format_number(*percent),
        ),
    }
}

fn solid_paint_color_operations(
    doc: &Doc,
    id: NodeId,
    is_stroke: bool,
    index: usize,
    color: FantaColor,
) -> Vec<Operation> {
    replace_data_operation(doc, id, |data| {
        if is_stroke {
            if let Some(strokes) = stroke_list_mut(data)
                && let Some(stroke) = strokes.get_mut(index)
            {
                let blend = crate::properties_ops::paint_blend(&stroke.paint);
                stroke.paint = crate::properties_ops::solid_fill_with_blend(color, blend);
            }
        } else {
            set_fill_color(data, index, color);
        }
    })
}

#[allow(deprecated)]
fn effect_edit_operations(
    doc: &Doc,
    id: NodeId,
    reference: EffectRef,
    property: DesignPanelProperty,
    value: &DesignPanelValue,
) -> Option<Vec<Operation>> {
    match reference {
        EffectRef::Shadow(index) => {
            let number = |value: &DesignPanelValue| match value {
                DesignPanelValue::Number(value) => Some(f64::from(*value)),
                _ => None,
            };
            Some(match property {
                DesignPanelProperty::EffectShadowColor(_) => {
                    let DesignPanelValue::Color(color) = value else {
                        return None;
                    };
                    let color = fanta_color(*color);
                    shadow_field_operations(doc, id, index, |shadow| shadow.color = color)
                }
                DesignPanelProperty::EffectShadowBlur(_) | DesignPanelProperty::EffectBlur(_) => {
                    let blur = number(value)?.max(0.0);
                    shadow_field_operations(doc, id, index, |shadow| shadow.blur = blur)
                }
                DesignPanelProperty::EffectShadowSpread(_)
                | DesignPanelProperty::EffectSpread(_) => {
                    let spread = number(value)?;
                    shadow_field_operations(doc, id, index, |shadow| shadow.spread = spread)
                }
                DesignPanelProperty::EffectShadowOffsetX(_)
                | DesignPanelProperty::EffectOffsetX(_) => {
                    let offset = number(value)?;
                    shadow_field_operations(doc, id, index, |shadow| shadow.offset[0] = offset)
                }
                DesignPanelProperty::EffectShadowOffsetY(_)
                | DesignPanelProperty::EffectOffsetY(_) => {
                    let offset = number(value)?;
                    shadow_field_operations(doc, id, index, |shadow| shadow.offset[1] = offset)
                }
                DesignPanelProperty::EffectDropShadowShowBehindNode(_) => {
                    let DesignPanelValue::Bool(show) = value else {
                        return None;
                    };
                    let show = *show;
                    shadow_field_operations(doc, id, index, |shadow| shadow.show_behind_node = show)
                }
                DesignPanelProperty::EffectKind(_) => {
                    let DesignPanelValue::EffectKind(kind) = value else {
                        return None;
                    };
                    return effect_kind_change_operations(doc, id, reference, *kind);
                }
                _ => return None,
            })
        }
        EffectRef::Blur(index) => Some(match property {
            DesignPanelProperty::EffectBlurRadius(_) | DesignPanelProperty::EffectBlur(_) => {
                let DesignPanelValue::Number(radius) = value else {
                    return None;
                };
                let radius = f64::from(*radius).max(0.0);
                blurs_operations(doc, id, |blurs| {
                    if let Some(blur) = blurs.get_mut(index) {
                        blur.radius = radius;
                    }
                })
            }
            DesignPanelProperty::EffectKind(_) => {
                let DesignPanelValue::EffectKind(kind) = value else {
                    return None;
                };
                return effect_kind_change_operations(doc, id, reference, *kind);
            }
            _ => return None,
        }),
    }
}

/// Change an effect row's kind, crossing the engine's shadow/blur lists when
/// needed; the caller batches the two list writes into one transaction.
fn effect_kind_change_operations(
    doc: &Doc,
    id: NodeId,
    reference: EffectRef,
    kind: DesignEffectKind,
) -> Option<Vec<Operation>> {
    let node = doc.scene.get(id)?;
    match (reference, kind) {
        (
            EffectRef::Shadow(index),
            DesignEffectKind::DropShadow | DesignEffectKind::InnerShadow,
        ) => {
            let target = if kind == DesignEffectKind::DropShadow {
                ShadowKind::Drop
            } else {
                ShadowKind::Inner
            };
            Some(shadow_field_operations(doc, id, index, |shadow| {
                shadow.kind = target
            }))
        }
        (
            EffectRef::Blur(index),
            DesignEffectKind::LayerBlur | DesignEffectKind::BackgroundBlur,
        ) => {
            let target = if kind == DesignEffectKind::LayerBlur {
                BlurKind::Layer
            } else {
                BlurKind::Background
            };
            Some(blurs_operations(doc, id, |blurs| {
                if let Some(blur) = blurs.get_mut(index) {
                    blur.kind = target;
                }
            }))
        }
        (
            EffectRef::Shadow(index),
            DesignEffectKind::LayerBlur | DesignEffectKind::BackgroundBlur,
        ) => {
            let shadow = node.effects.get(index)?.clone();
            let blur_kind = if kind == DesignEffectKind::LayerBlur {
                BlurKind::Layer
            } else {
                BlurKind::Background
            };
            let mut operations = effects_operations(doc, id, |effects| {
                if index < effects.len() {
                    effects.remove(index);
                }
            });
            operations.extend(blurs_operations(doc, id, |blurs| {
                blurs.push(Blur {
                    kind: blur_kind,
                    radius: shadow.blur,
                });
            }));
            Some(operations)
        }
        (EffectRef::Blur(index), DesignEffectKind::DropShadow | DesignEffectKind::InnerShadow) => {
            let blur = *node.blurs.get(index)?;
            let shadow_kind = if kind == DesignEffectKind::DropShadow {
                ShadowKind::Drop
            } else {
                ShadowKind::Inner
            };
            let mut operations = blurs_operations(doc, id, |blurs| {
                if index < blurs.len() {
                    blurs.remove(index);
                }
            });
            operations.extend(effects_operations(doc, id, |effects| {
                effects.push(Shadow {
                    kind: shadow_kind,
                    blur: blur.radius,
                    ..default_shadow()
                });
            }));
            Some(operations)
        }
        _ => None,
    }
}

enum EnginePropKind {
    Boolean,
    Number,
    Text,
    Other,
}

fn current_prop_kind(
    doc: &Doc,
    id: NodeId,
    prop: fanta_doc::ComponentPropId,
) -> Option<EnginePropKind> {
    let node = doc.scene.get(id)?;
    let NodeData::Instance(instance) = &node.data else {
        return None;
    };
    let definition = crate::properties_snapshot::resolved_instance_def(&doc.components, instance)?;
    let schema = definition.props.iter().find(|schema| schema.id == prop)?;
    Some(match schema.default {
        VarValue::Boolean { .. } => EnginePropKind::Boolean,
        VarValue::Float { .. } => EnginePropKind::Number,
        VarValue::String { .. } => EnginePropKind::Text,
        _ => EnginePropKind::Other,
    })
}

/// Committed operations for one node property edit. `None` marks a property
/// this adapter has not wired (as opposed to a valid no-op).
#[allow(deprecated)]
fn property_operations(
    doc: &Doc,
    id: NodeId,
    property: DesignPanelProperty,
    value: &DesignPanelValue,
) -> Option<Vec<Operation>> {
    use DesignPanelProperty as P;
    use DesignPanelValue as V;

    let number = |value: &V| match value {
        V::Number(value) => Some(f64::from(*value)),
        V::Integer(value) => Some(*value as f64),
        _ => None,
    };
    let field = |field: InspectorField, value: f64| {
        Some(field_operations(doc, &field, &format_number(value)))
    };

    match (property, value) {
        (P::X, _) => field(InspectorField::X(id), number(value)?),
        (P::Y, _) => field(InspectorField::Y(id), number(value)?),
        (P::Width, _) => field(InspectorField::Width(id), number(value)?),
        (P::Height, _) => field(InspectorField::Height(id), number(value)?),
        (P::Rotation, V::AngleRadians(radians)) => field(
            InspectorField::Rotation(id),
            f64::from(*radians).to_degrees(),
        ),
        (P::Rotation, _) => field(InspectorField::Rotation(id), number(value)?),
        (P::Opacity, _) => field(InspectorField::Opacity(id), number(value)?),
        (P::CornerRadius, _) => field(InspectorField::CornerRadius(id), number(value)?),
        (P::CornerRadiusTopLeft, _)
        | (P::CornerRadiusTopRight, _)
        | (P::CornerRadiusBottomRight, _)
        | (P::CornerRadiusBottomLeft, _) => {
            let corner = match property {
                P::CornerRadiusTopLeft => 0,
                P::CornerRadiusTopRight => 1,
                P::CornerRadiusBottomRight => 2,
                _ => 3,
            };
            let radius = number(value)?.max(0.0);
            Some(replace_data_operation(doc, id, |data| {
                set_corner_radius_corner(data, corner, radius);
            }))
        }
        (P::CornerSmoothing, V::Ratio(ratio)) => field(
            InspectorField::CornerSmoothing(id),
            f64::from(*ratio) * 100.0,
        ),
        (P::CornerSmoothing, _) => field(InspectorField::CornerSmoothing(id), number(value)?),
        (P::IndependentCorners, V::Bool(independent)) => {
            let independent = *independent;
            Some(replace_data_operation(doc, id, |data| {
                set_independent_corners(data, independent);
            }))
        }
        (P::Visible, V::Bool(visible)) => {
            let node = doc.scene.get(id)?;
            let currently_visible = !node.flags.contains(NodeFlags::HIDDEN);
            if currently_visible == *visible {
                return Some(Vec::new());
            }
            Some(vec![Operation::SetFlags {
                id,
                old: node.flags,
                new: node.flags ^ NodeFlags::HIDDEN,
            }])
        }
        (P::BlendMode, V::BlendMode(mode)) => Some(blend_mode_operations(doc, id, *mode)),
        (P::ClipContent, V::Bool(enabled)) => {
            Some(set_clip_content_meta_operation(doc, id, *enabled))
        }
        (P::StrokeWeight, _) => {
            let weight = number(value)?.max(0.0);
            Some(replace_data_operation(doc, id, |data| {
                if let Some(strokes) = stroke_list_mut(data) {
                    for stroke in strokes.iter_mut() {
                        stroke.width = weight;
                        stroke.per_side = None;
                    }
                }
            }))
        }
        (P::StrokeWeightTop, _)
        | (P::StrokeWeightRight, _)
        | (P::StrokeWeightBottom, _)
        | (P::StrokeWeightLeft, _) => {
            let side = match property {
                P::StrokeWeightTop => 0,
                P::StrokeWeightRight => 1,
                P::StrokeWeightBottom => 2,
                _ => 3,
            };
            let weight = number(value)?.max(0.0);
            Some(replace_data_operation(doc, id, |data| {
                if let Some(strokes) = stroke_list_mut(data) {
                    for stroke in strokes.iter_mut() {
                        let mut per_side = stroke.per_side.unwrap_or([stroke.width; 4]);
                        per_side[side] = weight;
                        stroke.per_side = Some(per_side);
                    }
                }
            }))
        }
        (P::StrokeAlign, V::StrokeAlign(align)) => {
            let align = match align {
                DesignStrokeAlign::Center => StrokeAlign::Center,
                DesignStrokeAlign::Inside => StrokeAlign::Inside,
                DesignStrokeAlign::Outside => StrokeAlign::Outside,
            };
            Some(replace_data_operation(doc, id, |data| {
                if let Some(strokes) = stroke_list_mut(data) {
                    for stroke in strokes.iter_mut() {
                        stroke.align = align;
                    }
                }
            }))
        }
        (P::StrokeStartCap, V::StrokeCap(cap))
        | (P::StrokeEndCap, V::StrokeCap(cap))
        | (P::StrokeEndpointCap, V::StrokeCap(cap)) => {
            // The engine has one cap per stroke; every endpoint leaf writes it.
            let cap = match cap {
                DesignStrokeCap::Round => StrokeCap::Round,
                DesignStrokeCap::Square => StrokeCap::Square,
                _ => StrokeCap::Butt,
            };
            Some(replace_data_operation(doc, id, |data| {
                if let Some(strokes) = stroke_list_mut(data) {
                    for stroke in strokes.iter_mut() {
                        stroke.cap = cap;
                    }
                }
            }))
        }
        (P::StrokeJoin, V::StrokeJoin(join)) => {
            let join = match join {
                DesignStrokeJoin::Miter => StrokeJoin::Miter,
                DesignStrokeJoin::Round => StrokeJoin::Round,
                DesignStrokeJoin::Bevel => StrokeJoin::Bevel,
            };
            Some(replace_data_operation(doc, id, |data| {
                if let Some(strokes) = stroke_list_mut(data) {
                    for stroke in strokes.iter_mut() {
                        stroke.join = join;
                    }
                }
            }))
        }
        (P::StrokeDashMode, V::StrokeDashMode(mode)) => {
            let dash: Vec<f64> = match mode {
                DesignStrokeDashMode::Solid => Vec::new(),
                DesignStrokeDashMode::Dashed | DesignStrokeDashMode::Custom => vec![4.0, 4.0],
            };
            let clear = dash.is_empty();
            Some(replace_data_operation(doc, id, |data| {
                if let Some(strokes) = stroke_list_mut(data) {
                    for stroke in strokes.iter_mut() {
                        if clear {
                            stroke.dash.clear();
                        } else if stroke.dash.is_empty() {
                            stroke.dash = dash.clone();
                        }
                    }
                }
            }))
        }
        (P::StrokeDashPattern, V::NumberList(pattern)) => {
            let dash: Vec<f64> = pattern
                .iter()
                .map(|value| f64::from(*value).max(0.0))
                .collect();
            Some(replace_data_operation(doc, id, |data| {
                if let Some(strokes) = stroke_list_mut(data) {
                    for stroke in strokes.iter_mut() {
                        stroke.dash = dash.clone();
                    }
                }
            }))
        }
        (P::LayoutMode, V::LayoutMode(mode)) => Some(layout_mode_operations(doc, id, *mode)),
        (P::Gap, _) => {
            let gap = number(value)?;
            let horizontal = primary_axis_is_horizontal(doc, id)?;
            Some(layout_gap_operations(
                doc,
                id,
                &format_number(gap),
                horizontal,
            ))
        }
        (P::CounterAxisGap, V::OptionalNumber(gap)) => {
            let gap = gap.map(f64::from).unwrap_or(0.0);
            let horizontal = !primary_axis_is_horizontal(doc, id)?;
            Some(layout_gap_operations(
                doc,
                id,
                &format_number(gap),
                horizontal,
            ))
        }
        (P::CounterAxisGap, _) => {
            let gap = number(value)?;
            let horizontal = !primary_axis_is_horizontal(doc, id)?;
            Some(layout_gap_operations(
                doc,
                id,
                &format_number(gap),
                horizontal,
            ))
        }
        (P::PaddingHorizontal, _) => Some(layout_padding_operations(
            doc,
            id,
            &format_number(number(value)?),
            true,
        )),
        (P::PaddingVertical, _) => Some(layout_padding_operations(
            doc,
            id,
            &format_number(number(value)?),
            false,
        )),
        (P::PaddingShorthand, _) => {
            let padding = number(value)?.max(0.0);
            Some(replace_data_operation(doc, id, |data| {
                if let NodeData::Group(group) = data
                    && let Some(layout) = group.auto_layout.as_mut()
                {
                    layout.padding = [padding; 4];
                }
            }))
        }
        (P::PaddingTop, _) | (P::PaddingRight, _) | (P::PaddingBottom, _) | (P::PaddingLeft, _) => {
            let side = match property {
                P::PaddingTop => 0,
                P::PaddingRight => 1,
                P::PaddingBottom => 2,
                _ => 3,
            };
            let padding = number(value)?.max(0.0);
            Some(replace_data_operation(doc, id, |data| {
                if let NodeData::Group(group) = data
                    && let Some(layout) = group.auto_layout.as_mut()
                {
                    layout.padding[side] = padding;
                }
            }))
        }
        (P::Wrap, V::Bool(wrap)) => {
            let wrap = *wrap;
            Some(replace_data_operation(doc, id, |data| {
                if let NodeData::Group(group) = data
                    && let Some(layout) = group.auto_layout.as_mut()
                {
                    layout.wrap = wrap;
                }
            }))
        }
        (P::HorizontalSizing, V::SizingMode(mode)) | (P::VerticalSizing, V::SizingMode(mode)) => {
            let sizing = match mode {
                DesignSizingMode::Fixed => fanta_doc::AxisSizing::Fixed,
                DesignSizingMode::Hug => fanta_doc::AxisSizing::Hug,
                _ => return None,
            };
            let horizontal = property == P::HorizontalSizing;
            Some(replace_data_operation(doc, id, |data| {
                if let NodeData::Group(group) = data
                    && let Some(layout) = group.auto_layout.as_mut()
                {
                    let primary = horizontal == (layout.mode == LayoutMode::Horizontal);
                    if primary {
                        layout.primary_sizing = sizing;
                    } else {
                        layout.counter_sizing = sizing;
                    }
                }
            }))
        }
        (P::MinWidth, value)
        | (P::MaxWidth, value)
        | (P::MinHeight, value)
        | (P::MaxHeight, value) => {
            let text = match value {
                V::OptionalNumber(None) => String::new(),
                V::OptionalNumber(Some(limit)) => format_number(f64::from(*limit)),
                _ => format_number(number(value)?),
            };
            let horizontal = matches!(property, P::MinWidth | P::MaxWidth);
            let minimum = matches!(property, P::MinWidth | P::MinHeight);
            Some(layout_limit_operations(doc, id, &text, horizontal, minimum))
        }
        (P::FontFamily, V::Text(family)) => Some(field_operations(
            doc,
            &InspectorField::FontFamily(id),
            family,
        )),
        (P::FontSize, _) => field(InspectorField::FontSize(id), number(value)?),
        (P::FontWeight, _) => {
            let weight = number(value)?.clamp(1.0, 1000.0) as u16;
            Some(replace_data_operation(doc, id, |data| {
                if let NodeData::Text(text) = data {
                    text.style.weight = weight;
                }
            }))
        }
        (P::LineHeight, V::LineHeight(DesignLineHeight::Percent(percent))) => {
            field(InspectorField::LineHeight(id), f64::from(*percent) / 100.0)
        }
        (P::LineHeight, _) => None,
        (P::LetterSpacing, V::LetterSpacing(DesignLetterSpacing::Pixels(pixels))) => {
            field(InspectorField::LetterSpacing(id), f64::from(*pixels))
        }
        (P::LetterSpacing, _) => None,
        (P::HorizontalTextAlignment, V::TextHorizontalAlignment(align)) => {
            let align = match align {
                DesignTextHorizontalAlignment::Left => TextAlign::Left,
                DesignTextHorizontalAlignment::Center => TextAlign::Center,
                DesignTextHorizontalAlignment::Right => TextAlign::Right,
                DesignTextHorizontalAlignment::Justified => TextAlign::Justify,
            };
            Some(replace_data_operation(doc, id, |data| {
                if let NodeData::Text(text) = data {
                    text.align = align;
                }
            }))
        }
        (P::VerticalTextAlignment, V::TextVerticalAlignment(align)) => {
            let align = match align {
                DesignTextVerticalAlignment::Top => TextVAlign::Top,
                DesignTextVerticalAlignment::Center => TextVAlign::Center,
                DesignTextVerticalAlignment::Bottom => TextVAlign::Bottom,
            };
            Some(replace_data_operation(doc, id, |data| {
                if let NodeData::Text(text) = data {
                    text.vertical_align = align;
                }
            }))
        }
        (P::TextResize, V::TextResize(resize)) => {
            let resize = match resize {
                DesignTextResize::Fixed => TextAutoResize::None,
                DesignTextResize::AutoWidth => TextAutoResize::WidthAndHeight,
                DesignTextResize::AutoHeight => TextAutoResize::Height,
            };
            Some(replace_data_operation(doc, id, |data| {
                if let NodeData::Text(text) = data {
                    text.auto_resize = resize;
                }
            }))
        }
        _ => None,
    }
}

/// Pass through clears `ISOLATED_BLEND`; Normal sets it; every concrete
/// engine mode writes `SetBlendMode`. Two writes batch into one transaction.
fn blend_mode_operations(doc: &Doc, id: NodeId, mode: DesignBlendMode) -> Vec<Operation> {
    let Some(node) = doc.scene.get(id) else {
        return Vec::new();
    };
    let mut operations = Vec::new();
    match mode {
        DesignBlendMode::PassThrough => {
            if node.flags.contains(NodeFlags::ISOLATED_BLEND) {
                operations.push(Operation::SetFlags {
                    id,
                    old: node.flags,
                    new: node.flags & !NodeFlags::ISOLATED_BLEND,
                });
            }
            if node.blend_mode != BlendMode::Normal {
                operations.push(Operation::SetBlendMode {
                    id,
                    old: node.blend_mode,
                    new: BlendMode::Normal,
                });
            }
        }
        DesignBlendMode::Normal => {
            if !node.flags.contains(NodeFlags::ISOLATED_BLEND) {
                operations.push(Operation::SetFlags {
                    id,
                    old: node.flags,
                    new: node.flags | NodeFlags::ISOLATED_BLEND,
                });
            }
            if node.blend_mode != BlendMode::Normal {
                operations.push(Operation::SetBlendMode {
                    id,
                    old: node.blend_mode,
                    new: BlendMode::Normal,
                });
            }
        }
        other => {
            let Some(target) = engine_blend_mode(other) else {
                log::debug!("fig design adapter: unsupported blend mode {other:?}");
                return Vec::new();
            };
            if node.blend_mode != target {
                operations.push(Operation::SetBlendMode {
                    id,
                    old: node.blend_mode,
                    new: target,
                });
            }
        }
    }
    operations
}

fn set_independent_corners(data: &mut NodeData, independent: bool) {
    let (corner_radius, corner_radii) = match data {
        NodeData::Vector(vector) => (&mut vector.corner_radius, &mut vector.corner_radii),
        NodeData::Group(group) => (&mut group.corner_radius, &mut group.corner_radii),
        _ => return,
    };
    if independent {
        if corner_radii.is_none() {
            let uniform = corner_radius.unwrap_or(0.0);
            *corner_radii = Some([uniform; 4]);
        }
    } else {
        if let Some(radii) = corner_radii.take() {
            *corner_radius = (radii[0] > 0.0).then_some(radii[0]);
        }
    }
}

fn layout_mode_operations(doc: &Doc, id: NodeId, mode: DesignLayoutMode) -> Vec<Operation> {
    let target = match mode {
        DesignLayoutMode::None => None,
        DesignLayoutMode::Horizontal => Some(LayoutMode::Horizontal),
        DesignLayoutMode::Vertical => Some(LayoutMode::Vertical),
        DesignLayoutMode::Grid => {
            log::debug!("fig design adapter: grid auto layout is not supported by the engine");
            return Vec::new();
        }
    };
    replace_data_operation(doc, id, |data| {
        let NodeData::Group(group) = data else {
            return;
        };
        match target {
            None => group.auto_layout = None,
            Some(mode) => match group.auto_layout.as_mut() {
                Some(layout) => layout.mode = mode,
                None => {
                    group.auto_layout = Some(fanta_doc::AutoLayout {
                        mode,
                        ..fanta_doc::AutoLayout::default()
                    });
                }
            },
        }
    })
}

fn primary_axis_is_horizontal(doc: &Doc, id: NodeId) -> Option<bool> {
    match doc.scene.get(id).map(|node| &node.data) {
        Some(NodeData::Group(group)) => group
            .auto_layout
            .as_ref()
            .map(|layout| layout.mode == LayoutMode::Horizontal),
        _ => None,
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use fanta_doc::{CanvasNode, Doc, GroupNode, VectorNode};
    use gpui::{Entity, TestAppContext, VisualTestContext};
    use project::{FakeFs, Project};

    use super::*;
    use crate::view::FigView;

    fn init_test(cx: &mut TestAppContext) {
        cx.update(|cx| {
            zlog::init_test();
            assets::Assets.load_test_fonts(cx);
            let store = settings::SettingsStore::test(cx);
            cx.set_global(store);
            theme_settings::init(theme::LoadThemes::JustBase, cx);
            release_channel::init(semver::Version::new(0, 0, 0), cx);
            editor::init(cx);
            gpui_component::init(cx);
            fanta_gpui::init(cx);
            crate::theme_bridge::init(cx);
        });
    }

    /// One page holding one 200×100 red rectangle at (10, 20). Built with
    /// direct scene inserts so the undo history starts empty — the undo-step
    /// assertions below must observe only panel-authored operations.
    fn doc_with_rect() -> (Doc, NodeId, NodeId) {
        let mut doc = Doc::new();
        let mut page = CanvasNode::new(NodeData::Group(GroupNode::default()));
        page.name = "Page 1".to_owned();
        let page_id = page.id;
        doc.scene.insert(page).expect("insert page");
        doc.add_page(page_id);
        doc.set_active_page(Some(page_id));
        let mut rect = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
            0.0,
            0.0,
            200.0,
            100.0,
            FantaColor::rgb(0xe0, 0x30, 0x30),
        )));
        rect.name = "Hero".to_owned();
        rect.transform = fanta_doc::Transform2D::translation(10.0, 20.0);
        rect.parent = Some(page_id);
        let rect_id = rect.id;
        doc.scene.insert(rect).expect("insert rect");
        (doc, page_id, rect_id)
    }

    async fn setup_view(
        doc: Doc,
        cx: &mut TestAppContext,
    ) -> (Entity<FigView>, Entity<DesignPanel>, VisualTestContext) {
        init_test(cx);
        let fs = FakeFs::new(cx.executor());
        let project = Project::test(fs, [], cx).await;
        let item = crate::document::ready_item_for_test(
            &project,
            PathBuf::from("/tmp/Design.fig"),
            doc,
            cx,
        );
        let (view, cx) =
            cx.add_window_view(move |window, cx| FigView::new(item, project, window, cx));
        cx.run_until_parked();
        let panel = view.read_with(cx, |view, _| {
            view.gpui_design
                .as_ref()
                .expect("the design adapter should mount in a themed window")
                .panel
                .clone()
        });
        let cx = cx.clone();
        (view, panel, cx)
    }

    #[gpui::test]
    async fn mounting_under_the_flag_echoes_the_selected_rectangle(cx: &mut TestAppContext) {
        let (mut doc, _page, rect) = doc_with_rect();
        doc.selection.replace_with([rect]);
        let (view, panel, mut cx) = setup_view(doc, cx).await;
        let cx = &mut cx;
        view.update_in(cx, |view, _, cx| view.refresh_gpui_design(cx));
        cx.run_until_parked();

        panel.read_with(cx, |panel, _| {
            let node = panel.node();
            assert_eq!(node.id.as_ref(), rect.to_string());
            assert_eq!(node.kind, DesignPanelNodeKind::Rectangle);
            assert_eq!(node.x, 10.0);
            assert_eq!(node.y, 20.0);
            assert_eq!(node.width, 200.0);
            assert_eq!(node.height, 100.0);
            assert_eq!(node.fills.len(), 1);
            assert_eq!(
                node.fills[0].color,
                DesignColor::rgb(0xe0, 0x30, 0x30),
                "the real document fill should reach the panel"
            );
            let capabilities = node.capabilities.as_ref().expect("capabilities are honest");
            assert!(
                !capabilities
                    .sections
                    .contains(&DesignPanelSection::LayoutGrid),
                "layout grids are gated off"
            );
            assert!(
                !capabilities.sections.contains(&DesignPanelSection::Export),
                "exports are gated off"
            );
            assert!(!capabilities.aspect_ratio_lock);
        });
    }

    #[gpui::test]
    async fn opacity_edit_lands_as_one_undoable_operation_and_echoes(cx: &mut TestAppContext) {
        let (mut doc, _page, rect) = doc_with_rect();
        doc.selection.replace_with([rect]);
        let (view, panel, mut cx) = setup_view(doc, cx).await;
        let cx = &mut cx;
        view.update_in(cx, |view, _, cx| view.refresh_gpui_design(cx));
        cx.run_until_parked();

        panel.update_in(cx, |_, _, cx| {
            cx.emit(DesignPanelAction::PropertyChangeRequested {
                node_id: SharedString::from(rect.to_string()),
                property: DesignPanelProperty::Opacity,
                value: DesignPanelValue::Number(50.0),
            });
        });
        cx.run_until_parked();

        let item = view.read_with(cx, |view, _| view.item().clone());
        item.read_with(cx, |item, _| {
            let doc = &item.document().expect("document ready").doc;
            let node = doc.scene.get(rect).expect("rect exists");
            assert!((node.opacity.get() - 0.5).abs() < 1e-6, "the edit landed");
        });
        panel.read_with(cx, |panel, _| {
            assert!(
                (panel.node().opacity - 50.0).abs() < 1e-3,
                "the host echo refreshed the panel snapshot"
            );
        });

        // Exactly one undo step: a single undo restores full opacity.
        item.update(cx, |item, cx| {
            item.undo(cx).expect("undo applies");
        });
        cx.run_until_parked();
        item.read_with(cx, |item, _| {
            let doc = &item.document().expect("document ready").doc;
            let node = doc.scene.get(rect).expect("rect exists");
            assert!(
                (node.opacity.get() - 1.0).abs() < 1e-6,
                "one undo restores the pre-edit opacity"
            );
        });
        panel.read_with(cx, |panel, _| {
            assert!(
                (panel.node().opacity - 100.0).abs() < 1e-3,
                "the undo echoes back into the panel"
            );
        });
    }

    #[gpui::test]
    async fn scrub_begin_preview_cancel_leaves_the_document_untouched(cx: &mut TestAppContext) {
        let (mut doc, _page, rect) = doc_with_rect();
        doc.selection.replace_with([rect]);
        let (view, panel, mut cx) = setup_view(doc, cx).await;
        let cx = &mut cx;
        view.update_in(cx, |view, _, cx| view.refresh_gpui_design(cx));
        cx.run_until_parked();

        let emit = |cx: &mut VisualTestContext, value: f32, phase: DesignPanelEditPhase| {
            panel.update_in(cx, |_, _, cx| {
                cx.emit(DesignPanelAction::PropertyEditRequested {
                    node_id: SharedString::from(rect.to_string()),
                    property: DesignPanelProperty::Opacity,
                    value: DesignPanelValue::Number(value),
                    phase,
                });
            });
            cx.run_until_parked();
        };
        emit(cx, 100.0, DesignPanelEditPhase::Begin);
        emit(cx, 25.0, DesignPanelEditPhase::Preview);

        let item = view.read_with(cx, |view, _| view.item().clone());
        item.read_with(cx, |item, _| {
            let doc = &item.document().expect("document ready").doc;
            let node = doc.scene.get(rect).expect("rect exists");
            assert!(
                (node.opacity.get() - 0.25).abs() < 1e-6,
                "the preview frame shows the candidate value"
            );
        });

        emit(cx, 25.0, DesignPanelEditPhase::Cancel);
        item.read_with(cx, |item, _| {
            assert!(
                !item.is_dirty(),
                "a cancelled gesture leaves the item clean"
            );
            let doc = &item.document().expect("document ready").doc;
            let node = doc.scene.get(rect).expect("rect exists");
            assert!(
                (node.opacity.get() - 1.0).abs() < 1e-6,
                "cancel restores the Begin snapshot"
            );
        });
        // Nothing to undo: the gesture never became a history entry.
        item.update(cx, |item, cx| {
            assert!(
                !item.undo(cx).expect("undo call succeeds"),
                "a cancelled preview must not create an undo step"
            );
        });
    }

    #[gpui::test]
    async fn stale_multi_node_targets_are_rejected_wholesale(cx: &mut TestAppContext) {
        let (mut doc, page, rect) = doc_with_rect();
        let mut second = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
            0.0,
            0.0,
            50.0,
            50.0,
            FantaColor::rgb(0x30, 0x30, 0xe0),
        )));
        second.name = "Chip".to_owned();
        second.parent = Some(page);
        let second_id = second.id;
        doc.scene.insert(second).expect("insert second");
        doc.selection.replace_with([rect, second_id]);
        let (view, panel, mut cx) = setup_view(doc, cx).await;
        let cx = &mut cx;
        view.update_in(cx, |view, _, cx| view.refresh_gpui_design(cx));
        cx.run_until_parked();

        let leaf = |target_ids: Vec<SharedString>| DesignPanelAction::TargetedNodeActionRequested {
            target: DesignPanelTarget::Nodes {
                node_ids: target_ids,
            },
            action: Box::new(DesignPanelAction::PropertyChangeRequested {
                node_id: SharedString::from(rect.to_string()),
                property: DesignPanelProperty::Opacity,
                value: DesignPanelValue::Number(40.0),
            }),
        };

        // A reordered target no longer matches the exact ordered selection.
        panel.update_in(cx, |_, _, cx| {
            cx.emit(leaf(vec![
                SharedString::from(second_id.to_string()),
                SharedString::from(rect.to_string()),
            ]));
        });
        cx.run_until_parked();
        let item = view.read_with(cx, |view, _| view.item().clone());
        item.read_with(cx, |item, _| {
            let doc = &item.document().expect("document ready").doc;
            for id in [rect, second_id] {
                let node = doc.scene.get(id).expect("node exists");
                assert!(
                    (node.opacity.get() - 1.0).abs() < 1e-6,
                    "a stale target must not change any member"
                );
            }
        });

        // The exact ordered target applies to every member as one undo step.
        panel.update_in(cx, |_, _, cx| {
            cx.emit(leaf(vec![
                SharedString::from(rect.to_string()),
                SharedString::from(second_id.to_string()),
            ]));
        });
        cx.run_until_parked();
        item.read_with(cx, |item, _| {
            let doc = &item.document().expect("document ready").doc;
            for id in [rect, second_id] {
                let node = doc.scene.get(id).expect("node exists");
                assert!(
                    (node.opacity.get() - 0.4).abs() < 1e-6,
                    "a valid target applies to every member"
                );
            }
        });
        item.update(cx, |item, cx| {
            item.undo(cx).expect("undo applies");
        });
        item.read_with(cx, |item, _| {
            let doc = &item.document().expect("document ready").doc;
            for id in [rect, second_id] {
                let node = doc.scene.get(id).expect("node exists");
                assert!(
                    (node.opacity.get() - 1.0).abs() < 1e-6,
                    "one undo restores the whole multi-node edit"
                );
            }
        });
    }
}
