//! Connects the document model to the Design inspector read model and actions.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context as _;

use fanta_doc::{
    AssetId, BlendMode, Blur, BlurKind, BooleanOp, BoundProp, Color as FantaColor, ComponentId,
    ComponentPropKind, CounterAlign, Doc, Fill, Gradient, ImageFitMode, LayoutChild, LayoutMode,
    MaskType, NodeData, NodeFlags, NodeId, Operation, ParametricShape, PatternFill,
    PatternHorizontalAlignment, PatternSpacing, PatternTileType, PrimaryAlign, Shadow, ShadowKind,
    StrokeAlign, StrokeCap, StrokeJoin, TextAlign, TextAutoResize, Transform2D,
    VAlign as TextVAlign, VarValue,
};
use fanta_gpui::design::{
    DesignArcData, DesignArrangeOperation, DesignAutoLayoutItem, DesignBaselineAlignment,
    DesignBlendMode, DesignBooleanOperation, DesignColor, DesignComponentContext,
    DesignComponentProperty, DesignComponentPropertyValue, DesignComponentReference,
    DesignComponentRole, DesignCornerCapabilities, DesignCounterAxisAlignContent, DesignEffect,
    DesignEffectKind, DesignEffectKindAvailability, DesignEffectSettings,
    DesignEffectStyleViewData, DesignEffectVector, DesignExportConfiguration, DesignExportFormat,
    DesignExportMode, DesignExportSizing as DesignPanelExportSizing, DesignExportViewData,
    DesignFontFamily, DesignFontSource, DesignFontStyle, DesignFontViewData, DesignGradientStop,
    DesignImageFilters, DesignItemSpacingMode, DesignLayout, DesignLayoutAlignSelf,
    DesignLayoutMode, DesignLayoutPositioning, DesignLetterSpacing, DesignLineHeight,
    DesignMaskType, DesignMediaCropAction, DesignMediaCropToolState, DesignMediaPaintCapabilities,
    DesignMediaPaintPlacement, DesignMediaPaintView, DesignMediaPaintViewData,
    DesignMediaQuarterTurn, DesignPaint, DesignPaintKind, DesignPaintPayload, DesignPaintProperty,
    DesignPaintSource, DesignPaintStyleViewData, DesignPaintTransform, DesignPaintType,
    DesignPaintValue, DesignPanel, DesignPanelAction, DesignPanelAutoLayoutDirection,
    DesignPanelAutoLayoutParticipation, DesignPanelAutoLayoutWrap, DesignPanelCollection,
    DesignPanelEditPhase, DesignPanelNode, DesignPanelNodeCapabilities, DesignPanelNodeKind,
    DesignPanelParentLayout, DesignPanelProperty, DesignPanelPropertyValueState,
    DesignPanelSection, DesignPanelSelection, DesignPanelTarget, DesignPanelValue,
    DesignPatternHorizontalAlignment, DesignPatternPaint, DesignPatternSource,
    DesignPatternTileType, DesignPolygonGeometry, DesignSelectionHeaderCommand,
    DesignSelectionHeaderControl, DesignSelectionHeaderControlKind, DesignSelectionHeaderMenu,
    DesignSelectionHeaderMenuItem, DesignSelectionHeaderViewData, DesignShaderDefinition,
    DesignShaderGradientStop, DesignShaderPaint, DesignShaderPropertyAssignment,
    DesignShaderPropertyDefinition, DesignShaderPropertyKind, DesignShaderPropertyValue,
    DesignShaderViewData, DesignShapeGeometry, DesignSizingMode, DesignStackingOrder,
    DesignStarGeometry, DesignStroke, DesignStrokeAlign, DesignStrokeCap, DesignStrokeDashMode,
    DesignStrokeDashes, DesignStrokeEditContext, DesignStrokeJoin, DesignStrokeWeightMode,
    DesignStrokeWeights, DesignTextDecoration, DesignTextHorizontalAlignment, DesignTextResize,
    DesignTextVerticalAlignment, DesignTransformOperation, DesignTypography,
    DesignVideoPreviewAction, DesignVideoPreviewState,
};
use gpui::{AppContext as _, Context, Entity, SharedString, Subscription, TaskExt as _, Window};

use super::design_snapshot::build_design_view_data;
use crate::color_picker::GradientKind;
use crate::document::{AssetStores, DocChange, FigDocument, MAX_IMAGE_SOURCE_BYTES, PreparedImage};
use crate::export::{
    ExportFormat, ExportRequest, ExportSizing, prepare_export_requests, run_export_jobs,
};
use crate::properties_ops::{
    apply_preview_operation, blurs_operations, default_blur, default_shadow,
    detach_instance_operations, effects_operations, field_operations, finite_transform_operations,
    format_number, instance_prop_operations, layout_gap_operations, layout_limit_operations,
    layout_padding_operations, parse_color, parse_number, read_field_text, replace_data_operation,
    restore_snapshot, set_clip_content_meta_operation, set_corner_radius_corner, set_fill_color,
    shadow_field_operations, stroke_list_mut,
};
use crate::properties_snapshot::PaintKind as EnginePaintKind;
use crate::properties_snapshot::{
    CornerRadiusValue, InspectorField, NodeSection, NodeSnapshot, PaintSnapshot, PropValueSnapshot,
    TypographySnapshot, multi_section, node_section,
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

fn supported_paint_types(text_selection: bool) -> &'static [DesignPaintType] {
    if text_selection {
        &[DesignPaintType::Solid]
    } else if cfg!(target_os = "macos") {
        &[
            DesignPaintType::Solid,
            DesignPaintType::Gradient,
            DesignPaintType::Pattern,
            DesignPaintType::Image,
            DesignPaintType::Video,
            DesignPaintType::Shader,
        ]
    } else {
        &[
            DesignPaintType::Solid,
            DesignPaintType::Gradient,
            DesignPaintType::Pattern,
            DesignPaintType::Image,
            DesignPaintType::Shader,
        ]
    }
}

/// Parses a panel node id back to the engine id; a failure means a stale row.
fn node_id(id: &SharedString) -> Option<NodeId> {
    id.parse().ok()
}

/// What a click-driven inspector control the adapter has not wired is called
/// in the notice it raises. `notify_unavailable` appends the rest of the
/// sentence.
const UNWIRED_CONTROL: &str = "This inspector control";

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

fn matching_design_layers<'a>(
    doc: &'a Doc,
    id: NodeId,
    page: Option<NodeId>,
) -> impl Iterator<Item = NodeId> + 'a {
    let masters = crate::properties_snapshot::master_roots(&doc.components);
    let kind = doc
        .scene
        .get(id)
        .map(|node| design_kind(id, &node.data, &masters));
    let roots = page
        .map(|page| doc.scene.children_of(Some(page)))
        .unwrap_or_else(|| doc.scene.children_of(None));
    roots
        .iter()
        .copied()
        .flat_map(|root| doc.scene.descendants_of(root))
        .filter(move |candidate| {
            kind.as_ref().is_some_and(|kind| {
                doc.scene.get(*candidate).is_some_and(|candidate_node| {
                    design_kind(*candidate, &candidate_node.data, &masters) == *kind
                })
            })
        })
}

pub(crate) fn selection_header_for_doc(
    doc: &Doc,
    selection: &[NodeId],
    page: Option<NodeId>,
    editable: bool,
) -> Option<DesignSelectionHeaderViewData> {
    use DesignSelectionHeaderControlKind as Kind;
    use fanta_gpui::layers::LayersPanelContextAction as LayerAction;

    let first = *selection.first()?;
    let node = doc.scene.get(first)?;
    let masters = crate::properties_snapshot::master_roots(&doc.components);
    let kind = design_kind(first, &node.data, &masters);
    let can_edit = editable
        && selection
            .iter()
            .all(|id| crate::layer_context_ops::editable(doc, *id));
    let mut controls = Vec::new();
    if selection.len() == 1 && matching_design_layers(doc, first, page).take(2).count() > 1 {
        controls.push(DesignSelectionHeaderControl::direct(
            Kind::SelectMatchingLayers,
        ));
    }
    if can_edit {
        let actions = crate::gpui_adapters::layers::context_actions(doc, first);
        if selection.len() > 1
            || (!doc.is_component_root(first) && actions.contains(&LayerAction::CreateComponent))
        {
            controls.push(DesignSelectionHeaderControl::direct(Kind::CreateComponent));
        }
        if selection.len() == 1 && actions.contains(&LayerAction::UseAsMask) {
            controls.push(DesignSelectionHeaderControl::direct(Kind::UseAsMask));
        }
        let can_boolean = selection.len() > 1
            && selection.iter().all(|id| {
                doc.scene.get(*id).is_some_and(|member| {
                    member.parent == node.parent && matches!(member.data, NodeData::Vector(_))
                })
            });
        let can_flatten = actions.contains(&LayerAction::Flatten);
        if can_boolean || can_flatten {
            let mut menu = DesignSelectionHeaderControl::boolean_flatten_menu();
            menu.menu_items.retain(|item| match item.command {
                DesignSelectionHeaderCommand::Boolean(_) => can_boolean,
                DesignSelectionHeaderCommand::Flatten => can_flatten,
                _ => false,
            });
            if !can_boolean {
                menu = menu.with_tooltip("Flatten");
            }
            controls.push(menu);
        }
        if selection.len() == 1 && matches!(node.data, NodeData::Vector(_) | NodeData::Text(_)) {
            controls.push(DesignSelectionHeaderControl::direct(Kind::EditObject));
        }
    }
    let title = if selection.len() > 1 {
        format!("{} layers", selection.len())
    } else if crate::layer_context_ops::is_section(node) {
        "Section".to_string()
    } else {
        kind.label().to_string()
    };
    let mut header = DesignSelectionHeaderViewData::new(title, controls);
    if can_edit && selection.len() == 1 && matches!(node.data, NodeData::Group(_)) {
        let actions = crate::gpui_adapters::layers::context_actions(doc, first);
        let menu_items = [
            ("frame", "Frame", LayerAction::ConvertToFrame),
            ("section", "Section", LayerAction::ConvertToSection),
        ]
        .into_iter()
        .filter(|(_, _, action)| actions.contains(action))
        .map(|(id, label, _)| {
            DesignSelectionHeaderMenuItem::new(
                id,
                label,
                DesignSelectionHeaderCommand::TitleMenuItem { item_id: id.into() },
            )
        })
        .collect::<Vec<_>>();
        if !menu_items.is_empty() {
            header =
                header.with_title_menu(DesignSelectionHeaderMenu::new("layer-type", menu_items));
        }
    }
    Some(header)
}

// =============================================================================
// Read model: paints, effects, layout, typography
// =============================================================================

/// One panel paint from a snapshot row. Paint ids are index-derived and
/// re-minted on every echo — the engine has no stable paint identity.
fn design_paint(
    node_id: NodeId,
    collection: &str,
    index: usize,
    snapshot: &PaintSnapshot,
    fill: Option<&Fill>,
) -> DesignPaint {
    let mut paint = match fill {
        Some(Fill::Pattern { pattern, .. }) => {
            let mut paint = DesignPaint::pattern(pattern.source_node_id.to_string());
            if let DesignPaintPayload::Pattern(payload) = &mut paint.payload {
                payload.tile_type = match pattern.tile_type {
                    PatternTileType::Rectangular => DesignPatternTileType::Rectangular,
                    PatternTileType::HorizontalHexagonal => {
                        DesignPatternTileType::HorizontalHexagonal
                    }
                    PatternTileType::VerticalHexagonal => DesignPatternTileType::VerticalHexagonal,
                };
                payload.scaling_factor = pattern.scaling_factor;
                payload.spacing = fanta_gpui::design::DesignPatternSpacing::new(
                    pattern.spacing.x,
                    pattern.spacing.y,
                );
                payload.horizontal_alignment = match pattern.horizontal_alignment {
                    PatternHorizontalAlignment::Start => DesignPatternHorizontalAlignment::Start,
                    PatternHorizontalAlignment::Center => DesignPatternHorizontalAlignment::Center,
                    PatternHorizontalAlignment::End => DesignPatternHorizontalAlignment::End,
                };
            }
            paint
        }
        Some(Fill::Image {
            asset,
            mode,
            crop,
            scale,
            rotation,
            adjust,
            ..
        }) => {
            let source_id = asset.to_string();
            let mut source = DesignPaintSource::new(source_id.clone(), "Image");
            source.reference = Some(source_id.into());
            let mut paint = DesignPaint::image(source);
            if let DesignPaintPayload::Image(payload) = &mut paint.payload {
                let degrees = rotation.unwrap_or(0.0).rem_euclid(360.0);
                let quarter_turn = match degrees {
                    degrees if (degrees - 90.0).abs() < 0.001 => {
                        DesignMediaQuarterTurn::Clockwise90
                    }
                    degrees if (degrees - 180.0).abs() < 0.001 => {
                        DesignMediaQuarterTurn::Clockwise180
                    }
                    degrees if (degrees - 270.0).abs() < 0.001 => {
                        DesignMediaQuarterTurn::Clockwise270
                    }
                    _ => DesignMediaQuarterTurn::None,
                };
                payload.placement = if let Some(crop) = crop {
                    DesignMediaPaintPlacement::Crop {
                        transform: DesignPaintTransform {
                            m11: crop[2],
                            m12: 0.0,
                            m21: 0.0,
                            m22: crop[3],
                            tx: crop[0],
                            ty: crop[1],
                        },
                    }
                } else {
                    match mode {
                        ImageFitMode::Fill | ImageFitMode::Stretch => {
                            DesignMediaPaintPlacement::Fill {
                                rotation: quarter_turn,
                            }
                        }
                        ImageFitMode::Fit => DesignMediaPaintPlacement::Fit {
                            rotation: quarter_turn,
                        },
                        ImageFitMode::Tile => DesignMediaPaintPlacement::Tile {
                            scaling_factor: scale.unwrap_or(1.0),
                            rotation: quarter_turn,
                        },
                    }
                };
                payload.filters = DesignImageFilters {
                    exposure: adjust.exposure,
                    contrast: adjust.contrast,
                    saturation: adjust.saturation,
                    temperature: adjust.temperature,
                    tint: adjust.tint,
                    highlights: adjust.highlights,
                    shadows: adjust.shadows,
                };
                if *mode == ImageFitMode::Stretch && crop.is_none()
                    || degrees != 0.0 && quarter_turn == DesignMediaQuarterTurn::None
                {
                    paint.read_only = true;
                }
            }
            paint
        }
        Some(Fill::Video { video, .. }) => {
            let source_id = video.asset.to_string();
            let mut source = DesignPaintSource::new(source_id.clone(), "Video");
            source.reference = Some(source_id.into());
            let mut paint = DesignPaint::video(source);
            if let DesignPaintPayload::Video(payload) = &mut paint.payload {
                payload.placement = design_media_placement(
                    video.mode,
                    video.crop.as_deref(),
                    video.scale,
                    video.rotation,
                );
                payload.filters = design_image_filters(&video.adjust);
            }
            let degrees = video.rotation.unwrap_or(0.0).rem_euclid(360.0);
            let quarter_turn = [0.0_f32, 90.0, 180.0, 270.0]
                .into_iter()
                .any(|quarter| (degrees - quarter).abs() < 0.001);
            if video.mode == ImageFitMode::Stretch && video.crop.is_none() || !quarter_turn {
                paint.read_only = true;
            }
            paint
        }
        Some(Fill::Shader { shader, .. }) => DesignPaint::from_payload(DesignPaintPayload::Shader(
            DesignShaderPaint::new(&shader.shader_id, &shader.name).with_properties(
                shader.properties.iter().filter_map(|property| {
                    design_shader_value(&property.value).map(|value| {
                        DesignShaderPropertyAssignment::new(&property.definition_id, value)
                    })
                }),
            ),
        )),
        _ => match (&snapshot.kind, &snapshot.gradient) {
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
                if let DesignPaintPayload::Gradient(payload) = &mut paint.payload {
                    payload.transform = design_gradient_transform(gradient);
                }
                paint.opacity = crate::color_picker::gradient_stops(gradient)
                    .iter()
                    .map(|stop| stop.color.a)
                    .max()
                    .map_or(0.0, |alpha| f32::from(alpha) / 255.0 * 100.0);
                paint
            }
            _ => {
                // An unknown fill (or a gradient whose payload went missing):
                // inspectable, never editable through this adapter yet.
                let mut paint = DesignPaint::image(fanta_gpui::design::DesignPaintSource::new(
                    format!("{node_id}-{collection}-{index}-source"),
                    snapshot.label.clone(),
                ));
                paint.read_only = true;
                paint
            }
        },
    };
    paint.id = SharedString::from(format!("{node_id}-{collection}-{index}"));
    if let Some(opacity) = snapshot.opacity_percent
        && !matches!(snapshot.kind, Some(EnginePaintKind::Gradient(_)))
    {
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

fn design_media_placement(
    mode: ImageFitMode,
    crop: Option<&[f32; 4]>,
    scale: Option<f32>,
    rotation: Option<f32>,
) -> DesignMediaPaintPlacement {
    let degrees = rotation.unwrap_or(0.0).rem_euclid(360.0);
    let quarter_turn = match degrees {
        degrees if (degrees - 90.0).abs() < 0.001 => DesignMediaQuarterTurn::Clockwise90,
        degrees if (degrees - 180.0).abs() < 0.001 => DesignMediaQuarterTurn::Clockwise180,
        degrees if (degrees - 270.0).abs() < 0.001 => DesignMediaQuarterTurn::Clockwise270,
        _ => DesignMediaQuarterTurn::None,
    };
    if let Some(crop) = crop {
        DesignMediaPaintPlacement::Crop {
            transform: DesignPaintTransform {
                m11: crop[2],
                m12: 0.0,
                m21: 0.0,
                m22: crop[3],
                tx: crop[0],
                ty: crop[1],
            },
        }
    } else {
        match mode {
            ImageFitMode::Fill | ImageFitMode::Stretch => DesignMediaPaintPlacement::Fill {
                rotation: quarter_turn,
            },
            ImageFitMode::Fit => DesignMediaPaintPlacement::Fit {
                rotation: quarter_turn,
            },
            ImageFitMode::Tile => DesignMediaPaintPlacement::Tile {
                scaling_factor: scale.unwrap_or(1.0),
                rotation: quarter_turn,
            },
        }
    }
}

fn design_image_filters(adjust: &fanta_doc::ImageAdjust) -> DesignImageFilters {
    DesignImageFilters {
        exposure: adjust.exposure,
        contrast: adjust.contrast,
        saturation: adjust.saturation,
        temperature: adjust.temperature,
        tint: adjust.tint,
        highlights: adjust.highlights,
        shadows: adjust.shadows,
    }
}

fn design_shader_value(
    value: &fanta_doc::ShaderPropertyValue,
) -> Option<DesignShaderPropertyValue> {
    use fanta_doc::ShaderPropertyValue as Value;
    Some(match value {
        Value::Boolean(value) => DesignShaderPropertyValue::Boolean(*value),
        Value::Text(value) => DesignShaderPropertyValue::Text(value.clone().into()),
        Value::Number(value) if value.is_finite() => DesignShaderPropertyValue::Number(*value),
        Value::Number(_) => return None,
        Value::AssetId(value) => DesignShaderPropertyValue::AssetId(value.clone().into()),
        Value::Color(value) => DesignShaderPropertyValue::Color(design_color(*value)),
        Value::Point(value) => {
            DesignShaderPropertyValue::Point(DesignEffectVector::new(value[0], value[1]))
        }
        Value::Line { start, end } => DesignShaderPropertyValue::Line {
            start: DesignEffectVector::new(start[0], start[1]),
            end: DesignEffectVector::new(end[0], end[1]),
        },
        Value::Circle { center, radius } => DesignShaderPropertyValue::Circle {
            center: DesignEffectVector::new(center[0], center[1]),
            radius: *radius,
        },
        Value::CirclePoint {
            center,
            radius,
            angle,
        } => DesignShaderPropertyValue::CirclePoint {
            center: DesignEffectVector::new(center[0], center[1]),
            radius: *radius,
            angle: *angle,
        },
        Value::ColorPoint {
            point,
            color,
            variable_id,
        } => DesignShaderPropertyValue::ColorPoint {
            point: DesignEffectVector::new(point[0], point[1]),
            color: design_color(*color),
            variable_id: variable_id.clone().map(Into::into),
        },
        Value::Gradient(stops) => DesignShaderPropertyValue::Gradient(
            stops
                .iter()
                .map(|stop| DesignShaderGradientStop {
                    position: stop.position,
                    color: design_color(stop.color),
                    variable_id: stop.variable_id.clone().map(Into::into),
                })
                .collect(),
        ),
        Value::VariableAlias { variable_id } => DesignShaderPropertyValue::VariableAlias {
            variable_id: variable_id.clone().into(),
        },
        Value::Opaque { type_name, payload } => DesignShaderPropertyValue::Opaque {
            type_name: type_name.clone().into(),
            payload: payload.clone().into(),
        },
    })
}

fn engine_shader_value(
    value: &DesignShaderPropertyValue,
) -> Option<fanta_doc::ShaderPropertyValue> {
    use fanta_doc::ShaderPropertyValue as Value;
    Some(match value {
        DesignShaderPropertyValue::Boolean(value) => Value::Boolean(*value),
        DesignShaderPropertyValue::Text(value) => Value::Text(value.to_string()),
        DesignShaderPropertyValue::Number(value) if value.is_finite() => Value::Number(*value),
        DesignShaderPropertyValue::Number(_) => return None,
        DesignShaderPropertyValue::AssetId(value) => Value::AssetId(value.to_string()),
        DesignShaderPropertyValue::Color(value) => Value::Color(fanta_color(*value)),
        DesignShaderPropertyValue::Point(value) => Value::Point([value.x, value.y]),
        DesignShaderPropertyValue::Line { start, end } => Value::Line {
            start: [start.x, start.y],
            end: [end.x, end.y],
        },
        DesignShaderPropertyValue::Circle { center, radius } => Value::Circle {
            center: [center.x, center.y],
            radius: *radius,
        },
        DesignShaderPropertyValue::CirclePoint {
            center,
            radius,
            angle,
        } => Value::CirclePoint {
            center: [center.x, center.y],
            radius: *radius,
            angle: *angle,
        },
        DesignShaderPropertyValue::ColorPoint {
            point,
            color,
            variable_id,
        } => Value::ColorPoint {
            point: [point.x, point.y],
            color: fanta_color(*color),
            variable_id: variable_id.as_ref().map(ToString::to_string),
        },
        DesignShaderPropertyValue::Gradient(stops) => Value::Gradient(
            stops
                .iter()
                .map(|stop| fanta_doc::ShaderGradientStop {
                    position: stop.position,
                    color: fanta_color(stop.color),
                    variable_id: stop.variable_id.as_ref().map(ToString::to_string),
                })
                .collect(),
        ),
        DesignShaderPropertyValue::VariableAlias { variable_id } => Value::VariableAlias {
            variable_id: variable_id.to_string(),
        },
        DesignShaderPropertyValue::Opaque { type_name, payload } => Value::Opaque {
            type_name: type_name.to_string(),
            payload: payload.to_string(),
        },
    })
}

fn bundled_shader_catalog() -> DesignShaderViewData {
    use DesignShaderPropertyKind::{Color, Number};
    use DesignShaderPropertyValue::{Color as ColorValue, Number as NumberValue};
    DesignShaderViewData::new(
        [
            DesignShaderDefinition::new(
                "fanta:shader:fractal-noise",
                "Fractal noise",
                true,
                [
                    DesignShaderPropertyDefinition::new("frequency", "Frequency", Number)
                        .with_default(NumberValue(4.0)),
                    DesignShaderPropertyDefinition::new("octaves", "Octaves", Number)
                        .with_default(NumberValue(4.0)),
                    DesignShaderPropertyDefinition::new("color_a", "First color", Color)
                        .with_default(ColorValue(DesignColor::rgb(0x25, 0x1f, 0x48))),
                    DesignShaderPropertyDefinition::new("color_b", "Second color", Color)
                        .with_default(ColorValue(DesignColor::rgb(0x7d, 0xe3, 0xd6))),
                ],
            ),
            DesignShaderDefinition::new(
                "fanta:shader:halftone",
                "Halftone",
                true,
                [
                    DesignShaderPropertyDefinition::new("columns", "Columns", Number)
                        .with_default(NumberValue(18.0)),
                    DesignShaderPropertyDefinition::new("radius", "Dot radius", Number)
                        .with_default(NumberValue(1.0)),
                    DesignShaderPropertyDefinition::new("ink_color", "Ink", Color)
                        .with_default(ColorValue(DesignColor::rgb(0x1e, 0x29, 0x3b))),
                    DesignShaderPropertyDefinition::new("paper_color", "Paper", Color)
                        .with_default(ColorValue(DesignColor::rgb(0xf8, 0xfa, 0xfc))),
                ],
            ),
        ],
        [],
    )
}

fn media_paint_view_data(
    context: &fanta_gpui::design::DesignPanelInspectionContext,
    crop_session: Option<&DesignCropSession>,
    pattern_sources: Vec<DesignPatternSource>,
    video_previews: &HashMap<String, DesignVideoPreviewState>,
) -> DesignMediaPaintViewData {
    let DesignPanelSelection::Single(node) = context.selection() else {
        return DesignMediaPaintViewData::default();
    };
    let capabilities = DesignMediaPaintCapabilities::editor()
        .with_source_actions(true, false, false)
        .with_crop_rotation(false);
    let fills = node.fills.iter().enumerate().filter_map(|(index, paint)| {
        matches!(
            paint.payload,
            DesignPaintPayload::Image(_) | DesignPaintPayload::Video(_)
        )
        .then(|| {
            let mut view =
                DesignMediaPaintView::new(DesignPanelCollection::Fill, paint.id.clone(), index)
                    .with_capabilities(capabilities);
            if let DesignPaintPayload::Video(video) = &paint.payload {
                view = view.with_video_preview(
                    video_previews
                        .get(video.source.id.as_ref())
                        .cloned()
                        .unwrap_or_else(DesignVideoPreviewState::loading),
                );
            }
            if let Some(session) = crop_session
                && node_id(&node.id) == Some(session.node)
                && !session.is_stroke
                && session.index == index
            {
                view.crop_tool = session.state;
            }
            view
        })
    });
    let strokes = node
        .stroke
        .as_ref()
        .into_iter()
        .flat_map(|stroke| stroke.paints.iter().enumerate())
        .filter_map(|(index, paint)| {
            matches!(
                paint.payload,
                DesignPaintPayload::Image(_) | DesignPaintPayload::Video(_)
            )
            .then(|| {
                let mut view = DesignMediaPaintView::new(
                    DesignPanelCollection::Stroke,
                    paint.id.clone(),
                    index,
                )
                .with_capabilities(capabilities);
                if let DesignPaintPayload::Video(video) = &paint.payload {
                    view = view.with_video_preview(
                        video_previews
                            .get(video.source.id.as_ref())
                            .cloned()
                            .unwrap_or_else(DesignVideoPreviewState::loading),
                    );
                }
                if let Some(session) = crop_session
                    && node_id(&node.id) == Some(session.node)
                    && session.is_stroke
                    && session.index == index
                {
                    view.crop_tool = session.state;
                }
                view
            })
        });
    DesignMediaPaintViewData::new(fills.chain(strokes)).with_pattern_sources(pattern_sources)
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

fn design_gradient_transform(gradient: &Gradient) -> DesignPaintTransform {
    let (primary, secondary, center) = match gradient {
        Gradient::Linear { start, end, .. } => {
            let axis = [end[0] - start[0], end[1] - start[1]];
            let other = [axis[1], -axis[0]];
            let center = [(start[0] + end[0]) / 2.0, (start[1] + end[1]) / 2.0];
            (other, axis, center)
        }
        Gradient::Radial {
            center,
            radius,
            handles,
            ..
        }
        | Gradient::Diamond {
            center,
            radius,
            handles,
            ..
        } => {
            let handles = handles.unwrap_or([
                [center[0] + radius, center[1]],
                [center[0], center[1] + radius],
            ]);
            (
                [
                    2.0 * (handles[0][0] - center[0]),
                    2.0 * (handles[0][1] - center[1]),
                ],
                [
                    2.0 * (handles[1][0] - center[0]),
                    2.0 * (handles[1][1] - center[1]),
                ],
                *center,
            )
        }
        Gradient::Angular {
            center,
            start_angle,
            ..
        } => {
            let (sine, cosine) = start_angle.sin_cos();
            ([cosine, sine], [-sine, cosine], *center)
        }
    };
    DesignPaintTransform {
        m11: primary[0],
        m12: primary[1],
        m21: secondary[0],
        m22: secondary[1],
        tx: center[0] - (primary[0] + secondary[0]) * 0.5,
        ty: center[1] - (primary[1] + secondary[1]) * 0.5,
    }
}

fn gradient_from_design(
    kind: DesignPaintKind,
    stops: &[DesignGradientStop],
    transform: DesignPaintTransform,
) -> Option<Gradient> {
    let components = [
        transform.m11,
        transform.m12,
        transform.m21,
        transform.m22,
        transform.tx,
        transform.ty,
    ];
    if !components.into_iter().all(f32::is_finite) || stops.len() < 2 {
        return None;
    }
    let stops: Vec<fanta_doc::GradientStop> = stops
        .iter()
        .map(|stop| {
            Some(fanta_doc::GradientStop {
                position: stop
                    .position
                    .is_finite()
                    .then_some(stop.position.clamp(0.0, 1.0))?,
                color: fanta_color(stop.color),
            })
        })
        .collect::<Option<_>>()?;
    let point = |x: f32, y: f32| {
        [
            transform.m11 * x + transform.m21 * y + transform.tx,
            transform.m12 * x + transform.m22 * y + transform.ty,
        ]
    };
    let center = point(0.5, 0.5);
    let primary = point(1.0, 0.5);
    let secondary = point(0.5, 1.0);
    let radius = (primary[0] - center[0]).hypot(primary[1] - center[1]);
    Some(match kind {
        DesignPaintKind::LinearGradient => Gradient::Linear {
            start: point(0.5, 0.0),
            end: point(0.5, 1.0),
            stops,
        },
        DesignPaintKind::RadialGradient => Gradient::Radial {
            center,
            radius,
            handles: Some([primary, secondary]),
            stops,
        },
        DesignPaintKind::AngularGradient => {
            let mut stops = stops;
            if transform.m11 * transform.m22 - transform.m21 * transform.m12 < 0.0 {
                for stop in &mut stops {
                    stop.position = 1.0 - stop.position;
                }
                stops.sort_by(|left, right| left.position.total_cmp(&right.position));
            }
            Gradient::Angular {
                center,
                start_angle: transform.m12.atan2(transform.m11),
                stops,
            }
        }
        DesignPaintKind::DiamondGradient => Gradient::Diamond {
            center,
            radius,
            handles: Some([primary, secondary]),
            stops,
        },
        _ => return None,
    })
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

fn design_layout(
    doc: &Doc,
    node: &fanta_doc::CanvasNode,
    section: &NodeSection,
) -> Option<DesignLayout> {
    if section.layout.is_none() && section.layout_child.is_none() {
        return None;
    }
    let mut out = DesignLayout::default();
    if let Some(layout) = section.layout.as_ref() {
        out.clip_content = layout.clip;
    }
    out.mode = DesignLayoutMode::None;
    out.item = DesignAutoLayoutItem::default();
    if let Some(auto) = section
        .layout
        .as_ref()
        .and_then(|layout| layout.auto_layout.as_ref())
    {
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
        let primary_cell = match auto.primary_align {
            PrimaryAlign::Start | PrimaryAlign::SpaceBetween | PrimaryAlign::SpaceEvenly => 0,
            PrimaryAlign::Center => 1,
            PrimaryAlign::End => 2,
        };
        let counter_cell = match auto.counter_align {
            CounterAlign::Start | CounterAlign::Stretch | CounterAlign::Baseline => 0,
            CounterAlign::Center => 1,
            CounterAlign::End => 2,
        };
        (out.alignment_x, out.alignment_y) = match auto.mode {
            LayoutMode::Horizontal => (primary_cell, counter_cell),
            LayoutMode::Vertical => (counter_cell, primary_cell),
        };
        out.item_spacing_mode = if matches!(
            auto.primary_align,
            PrimaryAlign::SpaceBetween | PrimaryAlign::SpaceEvenly
        ) {
            DesignItemSpacingMode::Auto
        } else {
            DesignItemSpacingMode::Fixed
        };
        out.stacking_order = if auto.reverse_z {
            DesignStackingOrder::FirstOnTop
        } else {
            DesignStackingOrder::LastOnTop
        };
        out.baseline_alignment = if auto.counter_align == CounterAlign::Baseline {
            DesignBaselineAlignment::Baseline
        } else {
            DesignBaselineAlignment::Bounds
        };
        out.padding = match &node.data {
            NodeData::Group(group) => group
                .auto_layout
                .map(|layout| layout.padding.map(|value| value as f32))
                .unwrap_or_default(),
            _ => [0.; 4],
        };
        out.item.min_width = auto.min_width.map(|value| value as f32);
        out.item.max_width = auto.max_width.map(|value| value as f32);
        out.item.min_height = auto.min_height.map(|value| value as f32);
        out.item.max_height = auto.max_height.map(|value| value as f32);
        if let NodeData::Group(group) = &node.data
            && let Some(layout) = group.auto_layout
        {
            out.include_strokes = layout.include_strokes;
            out.counter_axis_align_content = if layout.counter_auto_spacing {
                DesignCounterAxisAlignContent::SpaceBetween
            } else {
                DesignCounterAxisAlignContent::Auto
            };
            if layout.counter_auto_spacing {
                out.counter_axis_gap = None;
            }
        }
    }
    if section.layout_child.is_some() {
        let child = node.layout_child.unwrap_or(LayoutChild {
            grow: 0.0,
            absolute: false,
            align_self: None,
        });
        out.item.positioning = if child.absolute {
            DesignLayoutPositioning::Absolute
        } else {
            DesignLayoutPositioning::InFlow
        };
        out.item.align_self = if child.align_self == Some(CounterAlign::Stretch) {
            DesignLayoutAlignSelf::Stretch
        } else {
            DesignLayoutAlignSelf::Inherit
        };
        out.item.layout_grow = child.grow;
        if let Some(parent_mode) = node
            .parent
            .and_then(|id| doc.scene.get(id))
            .and_then(|parent| match &parent.data {
                NodeData::Group(group) => group.auto_layout.map(|layout| layout.mode),
                _ => None,
            })
        {
            let (primary, counter) = if parent_mode == LayoutMode::Horizontal {
                (&mut out.horizontal_sizing, &mut out.vertical_sizing)
            } else {
                (&mut out.vertical_sizing, &mut out.horizontal_sizing)
            };
            if child.grow > 0.0 {
                *primary = DesignSizingMode::Fill;
            }
            if child.align_self == Some(CounterAlign::Stretch) {
                *counter = DesignSizingMode::Fill;
            }
        }
    }
    if let NodeData::Text(text) = &node.data {
        match text.auto_resize {
            TextAutoResize::None => {}
            TextAutoResize::Height => out.vertical_sizing = DesignSizingMode::Hug,
            TextAutoResize::WidthAndHeight => {
                out.horizontal_sizing = DesignSizingMode::Hug;
                out.vertical_sizing = DesignSizingMode::Hug;
            }
        }
    }
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
    .with_advanced_type_settings_enabled(false)
}

fn design_font_catalog(
    system_families: impl IntoIterator<Item = SharedString>,
) -> DesignFontViewData {
    let mut seen = HashSet::new();
    let mut names = system_families
        .into_iter()
        .map(|name| name.to_string())
        .chain(fanta_text::bundled_family_names().map(str::to_owned))
        .filter(|name| !name.is_empty() && !name.starts_with('.'))
        .filter(|name| seen.insert(name.to_lowercase()))
        .collect::<Vec<_>>();
    names.sort_unstable_by_key(|name| name.to_lowercase());
    DesignFontViewData::ready(names.into_iter().map(|name| {
        let regular = DesignFontStyle::imported("regular", "Regular");
        let mut italic = DesignFontStyle::imported("italic", "Italic");
        italic.italic = true;
        DesignFontFamily::local(name.clone(), name, [regular, italic])
    }))
}

fn font_apply_operations(
    doc: &Doc,
    id: NodeId,
    family: &str,
    style: &DesignFontStyle,
) -> Vec<Operation> {
    replace_data_operation(doc, id, |data| {
        if let NodeData::Text(text) = data {
            text.style.font_family = family.to_owned();
            text.style.italic = style.italic;
            if let Some(weight) = style.weight {
                text.style.weight = weight;
            }
            for run in &mut text.style_runs {
                run.style.font_family = family.to_owned();
                run.style.italic = style.italic;
                if let Some(weight) = style.weight {
                    run.style.weight = weight;
                }
            }
        }
    })
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
        .map(|(index, snapshot)| {
            design_paint(
                id,
                "stroke",
                index,
                snapshot,
                strokes.get(index).map(|stroke| &stroke.paint),
            )
        })
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
    stroke.capabilities.variable_width = false;
    stroke.capabilities.complex_stroke = false;
    if matches!(
        kind,
        DesignPanelNodeKind::Line
            | DesignPanelNodeKind::Arrow
            | DesignPanelNodeKind::Vector
            | DesignPanelNodeKind::Pen
            | DesignPanelNodeKind::Pencil
    ) {
        // The engine stores one cap for every endpoint. The aggregate
        // control exposes that cap without offering an ineffective swap.
        stroke.edit_context = DesignStrokeEditContext::branching(2);
    }
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
    stroke.dash_cap = cap;
    stroke.join = match first.join {
        StrokeJoin::Miter => DesignStrokeJoin::Miter,
        StrokeJoin::Round => DesignStrokeJoin::Round,
        StrokeJoin::Bevel => DesignStrokeJoin::Bevel,
    };
    stroke.miter_angle = ((1.0 / first.miter_limit.max(1.0)).asin().to_degrees() * 2.0) as f32;
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

/// Honest capability gates: what the adapter has not wired stays off even
/// when the coarse kind preset would advertise it. `transforms` is wired
/// (rotate 90°, flip H/V); `arrange` is not, because aligning a lone node to
/// its own bounds is a no-op — align/distribute only turn on for the
/// multi-selection context built by `aggregate_selection`.
fn gate_capabilities(
    kind: DesignPanelNodeKind,
    data: &NodeData,
    is_mask: bool,
    corner_capabilities: DesignCornerCapabilities,
) -> DesignPanelNodeCapabilities {
    let mut capabilities = DesignPanelNodeCapabilities::for_node_kind(kind);
    capabilities.sections.retain(|section| {
        !matches!(
            section,
            DesignPanelSection::LayoutGrid | DesignPanelSection::Selection
        )
    });
    if is_mask && !capabilities.sections.contains(&DesignPanelSection::Mask) {
        capabilities.sections.push(DesignPanelSection::Mask);
    }
    capabilities.aspect_ratio_lock = false;
    capabilities.constraints = false;
    capabilities.arrange = false;
    capabilities.transforms = true;
    capabilities.resize_to_fit = false;
    capabilities.add_auto_layout = kind != DesignPanelNodeKind::Other;
    capabilities.auto_layout_container = matches!(
        kind,
        DesignPanelNodeKind::Frame | DesignPanelNodeKind::Group | DesignPanelNodeKind::Component
    );
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
    capabilities.fill &= matches!(
        data,
        NodeData::Vector(_) | NodeData::Group(_) | NodeData::Text(_)
    );
    capabilities.stroke &= matches!(data, NodeData::Vector(_) | NodeData::Group(_));
    let fill = capabilities.fill;
    let stroke = capabilities.stroke;
    capabilities.sections.retain(|section| match section {
        DesignPanelSection::Fill => fill,
        DesignPanelSection::Stroke => stroke,
        _ => true,
    });
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

/// The X/Y/W/H states for a node the engine can derive no box for: a group with
/// neither a stored box nor bounded content, an empty page root being the one
/// that shows up in practice. Every panel field needs a number, so the read
/// model carries zeros there — and "0, 0, 0×0" reads as a real layer sitting at
/// the origin. `Unset` renders as "—" instead, and the read-only wrapper stops
/// an edit whose base value would be a fiction.
fn boxless_geometry_states(section: &NodeSection) -> PropertyStates {
    if section.geometry.is_some() {
        return Vec::new();
    }
    [
        DesignPanelProperty::X,
        DesignPanelProperty::Y,
        DesignPanelProperty::Width,
        DesignPanelProperty::Height,
    ]
    .into_iter()
    .map(|property| {
        (
            property,
            DesignPanelPropertyValueState::Unset
                .read_only_with_reason("This layer has no size of its own"),
        )
    })
    .collect()
}

/// The complete controlled read model for one selected node, paired with the
/// property states that come from the node's geometry rather than a binding.
pub(crate) fn design_node(
    document: &FigDocument,
    id: NodeId,
    masters: &HashMap<NodeId, ComponentId>,
) -> Option<(DesignPanelNode, PropertyStates)> {
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
    out.shape_geometry = DesignShapeGeometry::None;

    out.visible = section.visible;
    out.x = section.x as f32;
    out.y = section.y as f32;
    out.width = section.width as f32;
    out.height = section.height as f32;
    out.rotation = section.rotation_degrees as f32;
    out.lock_aspect_ratio = false;
    out.opacity = section.opacity_percent as f32;
    out.blend_mode = display_blend_mode(kind, node);
    out.shape_geometry = match &node.data {
        NodeData::Vector(vector) => match vector.parametric {
            Some(ParametricShape::Polygon { points }) => {
                DesignShapeGeometry::Polygon(DesignPolygonGeometry::new(points.clamp(3, 60) as u16))
            }
            Some(ParametricShape::Star {
                points,
                inner_ratio,
            }) => DesignShapeGeometry::Star(DesignStarGeometry::new(
                points.clamp(3, 60) as u16,
                inner_ratio as f32,
            )),
            Some(ParametricShape::Arc {
                start_rad,
                sweep_rad,
                inner_ratio,
            }) => DesignShapeGeometry::Ellipse(DesignArcData::new(
                start_rad as f32,
                (start_rad + sweep_rad) as f32,
                inner_ratio as f32,
            )),
            None => DesignShapeGeometry::None,
        },
        NodeData::Boolean(boolean) => DesignShapeGeometry::Boolean(match boolean.op {
            BooleanOp::Union => DesignBooleanOperation::Union,
            BooleanOp::Subtract => DesignBooleanOperation::Subtract,
            BooleanOp::Intersect => DesignBooleanOperation::Intersect,
            BooleanOp::Exclude => DesignBooleanOperation::Exclude,
        }),
        _ => DesignShapeGeometry::None,
    };

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
            .map(|(index, snapshot)| {
                let fill = match &node.data {
                    NodeData::Vector(vector) => vector.fills.get(index),
                    NodeData::Group(group) => group
                        .background
                        .iter()
                        .chain(group.background_fills.iter())
                        .nth(index),
                    _ => None,
                };
                design_paint(id, "fill", index, snapshot, fill)
            })
            .collect();
    } else if let Some(typography) = &section.typography {
        // A text node's Fill section is its glyph color.
        let mut paint = design_solid_paint(typography.color);
        paint.id = SharedString::from(format!("{id}-fill-0"));
        paint.blend_mode = design_blend_mode(node.blend_mode);
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
    out.effect_capabilities.progressive_blur = false;
    out.effect_capabilities.shadow_blend_mode = false;
    out.layout = design_layout(doc, node, &section);
    out.typography = section.typography.as_ref().map(design_typography);
    out.is_mask = node.is_mask;
    out.mask_mode = match node.mask_type {
        MaskType::Alpha => DesignMaskType::Alpha,
        MaskType::Vector => DesignMaskType::Vector,
        MaskType::Luminance => DesignMaskType::Luminance,
    };
    out.mask_type = node
        .is_mask
        .then(|| SharedString::from(out.mask_mode.label()));

    if let Some(instance) = &section.instance {
        out.component_context = Some(DesignComponentContext {
            role: DesignComponentRole::Instance,
            main_component: instance.main_root.map(|root| {
                DesignComponentReference::local(root.to_string(), instance.component_name.clone())
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
        &node.data,
        node.is_mask,
        out.corner_capabilities,
    ));
    let mut states = boxless_geometry_states(&section);
    if let Some(instance) = &section.instance {
        states.extend(
            instance
                .props
                .iter()
                .enumerate()
                .filter_map(|(index, prop)| {
                    let PropValueSnapshot::Display(value) = &prop.value else {
                        return None;
                    };
                    Some((
                        DesignPanelProperty::ComponentProperty(index),
                        DesignPanelPropertyValueState::Uniform(DesignPanelValue::Text(
                            value.clone(),
                        ))
                        .read_only_with_reason("This component property cannot be edited here"),
                    ))
                }),
        );
    }
    Some((out, states))
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

pub(crate) fn parent_layout_for(doc: &Doc, id: NodeId) -> DesignPanelParentLayout {
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
pub(crate) fn member_node(
    doc: &Doc,
    id: NodeId,
    masters: &HashMap<NodeId, ComponentId>,
) -> DesignPanelNode {
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

pub(crate) type PropertyStates = Vec<(DesignPanelProperty, DesignPanelPropertyValueState)>;

/// Aggregate visual model + mixed/uniform states for a multiple selection.
pub(crate) fn aggregate_selection(
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
    capabilities.arrange = true;
    capabilities.transforms = true;
    capabilities.resize_to_fit = false;
    capabilities.add_auto_layout = true;
    capabilities.sections.push(DesignPanelSection::Layout);
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
pub(crate) fn bound_states(doc: &Doc, id: NodeId) -> PropertyStates {
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
    video_previews: Vec<(
        AssetId,
        u32,
        u32,
        bool,
        fanta_gpui::design::DesignVideoPreviewStatus,
    )>,
}

/// An in-flight Begin/Preview/Commit/Cancel property gesture: the pre-gesture
/// node state, restored before commit so one gesture is one undo step.
pub(crate) struct DesignEditSession {
    pub node: NodeId,
    pub snapshot: NodeSnapshot,
}

struct DesignCropSession {
    node: NodeId,
    is_stroke: bool,
    index: usize,
    state: DesignMediaCropToolState,
}

pub(crate) struct DesignAdapter {
    pub panel: Entity<DesignPanel>,
    pub(crate) last_echo: Option<DesignEchoKey>,
    pub(crate) session: Option<DesignEditSession>,
    crop_session: Option<DesignCropSession>,
    font_catalog: DesignFontViewData,
    font_catalog_dirty: bool,
    export_configurations: Vec<DesignExportConfiguration>,
    export_edit_snapshot: Option<Vec<DesignExportConfiguration>>,
    next_export_id: u64,
    _subscription: Subscription,
}

impl DesignAdapter {
    pub(crate) fn new(window: &mut Window, cx: &mut Context<FigView>) -> Self {
        theme::FontFamilyCache::init_global(cx);
        let font_cache = theme::FontFamilyCache::global(cx);
        let font_catalog =
            design_font_catalog(font_cache.try_list_font_families().unwrap_or_default());
        cx.spawn({
            let font_cache = font_cache.clone();
            async move |this, cx| {
                let families = loop {
                    if let Some(families) = font_cache.try_list_font_families() {
                        break families;
                    }
                    font_cache.prefetch(cx).await;
                    cx.background_executor()
                        .timer(Duration::from_millis(50))
                        .await;
                };
                this.update(cx, |view, cx| {
                    if let Some(adapter) = view.gpui_design.as_mut() {
                        adapter.font_catalog = design_font_catalog(families);
                        adapter.font_catalog_dirty = true;
                        adapter.last_echo = None;
                    }
                    view.refresh_gpui_design(cx);
                })?;
                Ok::<(), anyhow::Error>(())
            }
        })
        .detach_and_log_err(cx);
        let panel = cx.new(|cx| {
            DesignPanel::new(
                "fig-gpui-design",
                DesignPanelNode::new("fig-gpui-design-empty", "Page", DesignPanelNodeKind::Frame),
                window,
                cx,
            )
        });
        panel.update(cx, |panel, cx| {
            panel.set_supported_paint_types(supported_paint_types(false), cx);
            panel.set_export_advanced_settings_enabled(false, cx);
            panel.set_paint_visibility_supported(false, cx);
            panel.set_shader_view_data(bundled_shader_catalog(), cx);
            panel.set_shader_variable_binding_enabled(false, cx);
            panel.set_eyedropper_enabled(false, cx);
            panel.set_color_variable_creation_enabled(false, cx);
            panel.set_paint_style_view_data(
                DesignPaintStyleViewData::default().with_enabled(false),
                cx,
            );
            panel.set_effect_style_view_data(
                DesignEffectStyleViewData::default().with_enabled(false),
                cx,
            );
            let supported_blend_modes: Vec<_> = DesignBlendMode::ALL
                .into_iter()
                .filter(|mode| {
                    !matches!(
                        mode,
                        DesignBlendMode::LinearBurn | DesignBlendMode::LinearDodge
                    )
                })
                .collect();
            panel.set_supported_blend_modes(&supported_blend_modes, cx);
        });
        let subscription = cx.subscribe_in(&panel, window, FigView::handle_design_action);
        Self {
            panel,
            last_echo: None,
            session: None,
            crop_session: None,
            font_catalog,
            font_catalog_dirty: true,
            export_configurations: vec![DesignExportConfiguration::new(
                "fanta-export-0",
                DesignExportFormat::Png,
            )],
            export_edit_snapshot: None,
            next_export_id: 1,
            _subscription: subscription,
        }
    }
}

// =============================================================================
// Host integration: echo, finish hook, and intents
// =============================================================================

impl FigView {
    #[cfg(target_os = "macos")]
    fn design_video_preview_state(
        &mut self,
        asset: AssetId,
        cx: &mut Context<Self>,
    ) -> DesignVideoPreviewState {
        self.ensure_video_fill_playback(asset, cx);
        if let Some(error) = self.video_fill_error(asset) {
            return DesignVideoPreviewState::error(error);
        }
        if self.video_fill_loading(asset) {
            return DesignVideoPreviewState::loading();
        }
        let Some(playback) = self.video_fill_playback(asset) else {
            return DesignVideoPreviewState::loading();
        };
        let playback = playback.read(cx);
        if let Some(error) = playback.error() {
            return DesignVideoPreviewState::error(error.clone());
        }
        let status = playback.status();
        if status.duration_us == 0 {
            return DesignVideoPreviewState::loading();
        }
        DesignVideoPreviewState::ready(
            status.duration_us as f32 / 1_000_000.0,
            status.current_time_us as f32 / 1_000_000.0,
            status.state == media::video::VideoPlaybackState::Playing,
        )
    }

    /// Echo document state into the DesignPanel — inspection context, property
    /// value states, and Page view data — built as one snapshot by
    /// [`build_design_view_data`] and memoized on (selection identity, render
    /// generation, editability, page).
    ///
    /// The snapshot is *applied* through the granular setters rather than
    /// `DesignPanel::set_view_data`, because the complete-snapshot path is not
    /// yet echo-safe for this host: it routes every `None` projection through
    /// the matching `apply_clear_*`, which can discard transient dropdown
    /// state. The granular setters touch only what this adapter owns.
    pub(crate) fn refresh_gpui_design(&mut self, cx: &mut Context<Self>) {
        if self.gpui_design.is_none() {
            return;
        }
        let mut video_previews = HashMap::new();
        #[cfg(target_os = "macos")]
        for asset in self.selected_canvas_video_fill_assets(cx) {
            video_previews.insert(
                asset.to_string(),
                self.design_video_preview_state(asset, cx),
            );
        }
        let mut video_preview_signature = video_previews
            .iter()
            .filter_map(|(source, preview)| {
                let asset = source.parse::<AssetId>().ok()?;
                Some((
                    asset,
                    (preview.duration_seconds * 10.0) as u32,
                    (preview.current_seconds * 10.0) as u32,
                    preview.playing,
                    preview.status.clone(),
                ))
            })
            .collect::<Vec<_>>();
        video_preview_signature.sort_by_key(|(asset, ..)| *asset);
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
            let text_selection = selection.len() == 1
                && selection.first().is_some_and(|id| {
                    matches!(
                        doc.scene.get(*id).map(|node| &node.data),
                        Some(NodeData::Text(_))
                    )
                });
            let key = DesignEchoKey {
                selection: selection.clone(),
                generation: document.render_generation(),
                editable,
                page_index,
                video_previews: video_preview_signature,
            };
            if self
                .gpui_design
                .as_ref()
                .is_some_and(|adapter| adapter.last_echo.as_ref() == Some(&key))
            {
                return;
            }
            // The document speaks for the inspection context, the property
            // states, and — only for an empty selection — the Page
            // projection. The panel keeps the Page projection it already has
            // while a node is selected, so thread the live value back in.
            let previous_page = self
                .gpui_design
                .as_ref()
                .and_then(|adapter| adapter.panel.read(cx).page_view_data().cloned());
            let view_data =
                build_design_view_data(document, &selection, page_index, editable, previous_page);
            let pattern_sources = selection
                .first()
                .filter(|_| selection.len() == 1)
                .map(|selected| {
                    pattern_source_candidates(doc, *selected)
                        .into_iter()
                        .map(|(id, name)| DesignPatternSource::new(id.to_string(), name))
                        .collect()
                })
                .unwrap_or_default();
            Some((key, view_data, selection, text_selection, pattern_sources))
        };
        let Some((key, mut view_data, selection, text_selection, pattern_sources)) = built else {
            return;
        };
        // Granular, not `set_view_data` — see this method's docs. Page first,
        // then context, then states: the order the panel's own setters were
        // called in before the snapshot builder existed. Re-applying an
        // unchanged Page projection is a no-op inside `apply_page_view_data`,
        // so carrying the retained value forward costs nothing.
        let page_view_data = view_data.projections.page.take();
        let export_target = if selection.is_empty() {
            DesignPanelTarget::Page {
                page_id: page_view_data
                    .as_ref()
                    .map(|page| page.page_id.clone())
                    .unwrap_or_else(|| format!("page-{}", page_index.unwrap_or(0)).into()),
            }
        } else {
            DesignPanelTarget::Nodes {
                node_ids: selection.iter().map(|id| id.to_string().into()).collect(),
            }
        };
        let add_auto_layout = view_data.projections.add_auto_layout.take();
        let selection_header = view_data.projections.selection_header.take();
        let media_paints = media_paint_view_data(
            &view_data.inspection_context,
            self.gpui_design
                .as_ref()
                .and_then(|adapter| adapter.crop_session.as_ref()),
            pattern_sources,
            &video_previews,
        );
        let inspection_context = view_data.inspection_context;
        let property_states = view_data.property_states;
        let Some(adapter) = self.gpui_design.as_mut() else {
            return;
        };
        let export_view_data = DesignExportViewData {
            target: export_target,
            configurations: adapter.export_configurations.clone(),
            mode: DesignExportMode::Static,
            static_capabilities: Default::default(),
            preview: None,
            animated: None,
        };
        let font_catalog = adapter
            .font_catalog_dirty
            .then(|| adapter.font_catalog.clone());
        adapter.font_catalog_dirty = false;
        adapter.last_echo = Some(key);
        adapter.panel.update(cx, |panel, cx| {
            panel.set_paint_collection_item_actions_enabled(
                DesignPanelCollection::Fill,
                !text_selection,
                cx,
            );
            if let Some(font_catalog) = font_catalog {
                panel.set_font_view_data(font_catalog, cx);
            }
            panel.set_supported_paint_types(supported_paint_types(text_selection), cx);
            if let Some(page_view_data) = page_view_data {
                panel.set_page_view_data(page_view_data, cx);
            }
            panel.set_inspection_context(inspection_context, cx);
            panel.set_export_view_data(export_view_data, cx);
            panel.set_media_paint_view_data(media_paints, cx);
            if let Some(add_auto_layout) = add_auto_layout {
                panel.set_add_auto_layout_view_data(add_auto_layout, cx);
            } else {
                panel.clear_add_auto_layout_view_data(cx);
            }
            if let Some(selection_header) = selection_header {
                panel.set_selection_header_view_data_for_target(
                    selection_header.target,
                    selection_header.view_data,
                    cx,
                );
            } else {
                panel.clear_selection_header_view_data(cx);
            }
            panel.set_property_value_states(property_states, cx);
        });
    }

    /// Cancels any in-flight DesignPanel preview gesture, restoring the
    /// Begin snapshot — the transaction boundary `finish_panel_edits` needs
    /// before an external mutation lands.
    pub(crate) fn finish_gpui_design_edits(&mut self, cx: &mut Context<Self>) {
        let Some(adapter) = self.gpui_design.as_mut() else {
            return;
        };
        adapter.crop_session = None;
        adapter.last_echo = None;
        if let Some(configurations) = adapter.export_edit_snapshot.take() {
            adapter.export_configurations = configurations;
        }
        let Some(session) = adapter.session.take() else {
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

    fn design_export_target_matches(&self, target: &DesignPanelTarget, cx: &Context<Self>) -> bool {
        match target {
            DesignPanelTarget::Nodes { .. } => self.design_target_matches_selection(target, cx),
            DesignPanelTarget::Page { page_id } => {
                let item = self.item().read(cx);
                let Some(document) = item.document() else {
                    return false;
                };
                if !document.doc.selection.is_empty() {
                    return false;
                }
                let page =
                    crate::properties_snapshot::page_section(document, self.selected_page_index());
                let current_page_id = page
                    .id
                    .map(|id| id.to_string())
                    .unwrap_or_else(|| format!("page-{}", self.selected_page_index().unwrap_or(0)));
                page_id.as_ref() == current_page_id
            }
        }
    }

    pub(crate) fn export_design_selection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(adapter) = self.gpui_design.as_ref() else {
            crate::view::show_canvas_notice(
                "The design exporter is unavailable.".into(),
                window,
                cx,
            );
            return;
        };
        let requests = adapter
            .export_configurations
            .iter()
            .map(|configuration| ExportRequest {
                format: match configuration.format() {
                    DesignExportFormat::Png => ExportFormat::Png,
                    DesignExportFormat::Jpg => ExportFormat::Jpeg,
                    DesignExportFormat::Svg => ExportFormat::Svg,
                    DesignExportFormat::Pdf => ExportFormat::Pdf,
                },
                sizing: match configuration.sizing {
                    DesignPanelExportSizing::Scale(value) => ExportSizing::Scale(f64::from(value)),
                    DesignPanelExportSizing::Width(value) => ExportSizing::Width(f64::from(value)),
                    DesignPanelExportSizing::Height(value) => {
                        ExportSizing::Height(f64::from(value))
                    }
                },
                suffix: configuration.common.suffix.to_string(),
            })
            .collect::<Vec<_>>();
        let item = self.item().read(cx);
        let Some(document) = item.document() else {
            crate::view::show_canvas_notice(
                "The document is not ready to export.".into(),
                window,
                cx,
            );
            return;
        };
        let project_root = item.project_root().map(std::path::Path::to_path_buf);
        let output_directory = project_root
            .as_ref()
            .map(|root| root.join("exports"))
            .unwrap_or_default();
        let jobs = prepare_export_requests(
            &document.doc,
            document.asset_resolver.clone(),
            document.page(self.selected_page_index()),
            output_directory,
            &requests,
        );
        let jobs = match jobs {
            Ok(jobs) => jobs,
            Err(error) => {
                crate::view::show_canvas_notice(format!("Export failed: {error:#}"), window, cx);
                return;
            }
        };
        let chooser = project_root.is_none().then(|| {
            cx.prompt_for_paths(gpui::PathPromptOptions {
                files: false,
                directories: true,
                multiple: false,
                prompt: Some("Choose export folder".into()),
            })
        });
        cx.spawn_in(window, async move |this, cx| {
            let output_directory = match chooser {
                Some(chooser) => {
                    let selected: anyhow::Result<Option<std::path::PathBuf>> =
                        async { Ok(chooser.await??.and_then(|paths| paths.into_iter().next())) }
                            .await;
                    match selected {
                        Ok(Some(directory)) => {
                            #[cfg(all(target_os = "macos", feature = "mac_app_store"))]
                            workspace::remember_user_selected_paths(std::slice::from_ref(
                                &directory,
                            ))?;
                            Some(directory)
                        }
                        Ok(None) => return Ok::<(), anyhow::Error>(()),
                        Err(error) => {
                            this.update_in(cx, |_, window, cx| {
                                crate::view::show_canvas_notice(
                                    format!("Export folder could not be chosen: {error:#}"),
                                    window,
                                    cx,
                                );
                            })?;
                            return Ok(());
                        }
                    }
                }
                None => None,
            };
            this.update_in(cx, |_, window, cx| {
                crate::view::show_canvas_notice("Exporting…".into(), window, cx);
            })?;
            let jobs = if let Some(directory) = output_directory {
                jobs.with_output_directory(directory)
            } else {
                jobs
            };
            let result = cx
                .background_spawn(async move { run_export_jobs(jobs) })
                .await;
            this.update_in(cx, |_, window, cx| {
                let message = match result {
                    Ok(paths) => {
                        for path in &paths {
                            log::info!("Fanta export written to {}", path.display());
                        }
                        match paths.as_slice() {
                            [path] => format!("Exported {}", path.display()),
                            [first, ..] => format!(
                                "Exported {} files to {}",
                                paths.len(),
                                first.parent().map_or_else(
                                    || "export folder".to_string(),
                                    |path| path.display().to_string()
                                )
                            ),
                            [] => "Nothing was exported.".to_string(),
                        }
                    }
                    Err(error) => {
                        log::error!("Fanta export failed: {error:#}");
                        format!("Export failed: {error:#}")
                    }
                };
                crate::view::show_canvas_notice(message, window, cx);
            })?;
            Ok(())
        })
        .detach_and_log_err(cx);
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
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match action {
            DesignPanelAction::ExportConfigurationAddRequested { target } => {
                if !self.design_export_target_matches(target, cx) {
                    return;
                }
                if let Some(adapter) = self.gpui_design.as_mut() {
                    let id = adapter.next_export_id;
                    adapter.next_export_id = adapter.next_export_id.wrapping_add(1);
                    adapter
                        .export_configurations
                        .push(DesignExportConfiguration::new(
                            format!("fanta-export-{id}"),
                            DesignExportFormat::Png,
                        ));
                    adapter.last_echo = None;
                }
                self.refresh_gpui_design(cx);
            }
            DesignPanelAction::ExportConfigurationRemoveRequested {
                target,
                configuration_id,
            } => {
                if !self.design_export_target_matches(target, cx) {
                    return;
                }
                if let Some(adapter) = self.gpui_design.as_mut() {
                    adapter
                        .export_configurations
                        .retain(|configuration| configuration.id != *configuration_id);
                    adapter.last_echo = None;
                }
                self.refresh_gpui_design(cx);
            }
            DesignPanelAction::ExportConfigurationChangeRequested {
                target,
                configuration_id,
                change,
                phase,
            } => {
                if !self.design_export_target_matches(target, cx) {
                    return;
                }
                let Some(adapter) = self.gpui_design.as_mut() else {
                    return;
                };
                match phase {
                    DesignPanelEditPhase::Begin => {
                        adapter.export_edit_snapshot = Some(adapter.export_configurations.clone());
                    }
                    DesignPanelEditPhase::Cancel => {
                        if let Some(snapshot) = adapter.export_edit_snapshot.take() {
                            adapter.export_configurations = snapshot;
                        }
                    }
                    DesignPanelEditPhase::Preview | DesignPanelEditPhase::Commit => {
                        if let Some(configuration) = adapter
                            .export_configurations
                            .iter_mut()
                            .find(|configuration| configuration.id == *configuration_id)
                        {
                            configuration.apply_change(change.clone());
                            configuration.apply_target_defaults(Default::default());
                        }
                        if *phase == DesignPanelEditPhase::Commit {
                            adapter.export_edit_snapshot = None;
                        }
                    }
                }
                adapter.last_echo = None;
                self.refresh_gpui_design(cx);
            }
            DesignPanelAction::ExportAllRequested { target } => {
                if self.design_export_target_matches(target, cx) {
                    self.export_design_selection(window, cx);
                }
            }
            DesignPanelAction::ExportRequested { .. } => {
                self.export_design_selection(window, cx);
            }
            DesignPanelAction::PropertyChangeRequested {
                node_id: id,
                property,
                value,
            } => {
                let Some(id) = node_id(id) else {
                    return;
                };
                self.finish_document_edits_for_external_change(cx);
                let mut unhandled = false;
                let ops = self.design_ops(cx, |doc| {
                    property_operations(doc, id, *property, value).unwrap_or_else(|| {
                        unhandled = true;
                        Vec::new()
                    })
                });
                if unhandled {
                    log::debug!("fig design adapter: unhandled property {property:?}");
                    crate::view::notify_unavailable(UNWIRED_CONTROL, window, cx);
                    return;
                }
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
                self.handle_design_phased_edit(id, *property, value, *phase, window, cx);
            }
            DesignPanelAction::PropertyCopyRequested {
                target,
                displayed_value,
                ..
            } => {
                if self.design_target_matches_selection(target, cx) {
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(
                        displayed_value.to_string(),
                    ));
                }
            }
            DesignPanelAction::TargetedNodeActionRequested { target, action } => {
                self.handle_design_targeted_action(target, action, window, cx);
            }
            DesignPanelAction::SelectionHeaderCommandRequested { target, command } => {
                self.handle_design_selection_header_command(target, command, window, cx);
            }
            DesignPanelAction::AddAutoLayoutRequested { target } => {
                self.handle_design_add_auto_layout(target, window, cx);
            }
            DesignPanelAction::ArrangeRequested { target, operation } => {
                self.handle_design_arrange(target, *operation, window, cx);
            }
            DesignPanelAction::TransformRequested { target, operation } => {
                self.handle_design_transform(target, *operation, cx);
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
                self.handle_design_paint_edit(id, *collection, *index, edit, *phase, window, cx);
            }
            DesignPanelAction::PaintMediaSourceActionRequested {
                node_id: id,
                collection,
                index,
                source_id,
                action,
                ..
            } => {
                if *action != fanta_gpui::design::DesignMediaSourceAction::Upload {
                    return;
                }
                let Some(id) = node_id(id) else {
                    return;
                };
                let is_stroke = match collection {
                    DesignPanelCollection::Fill => false,
                    DesignPanelCollection::Stroke => true,
                    _ => return,
                };
                let is_video = self.item().read(cx).document().is_some_and(|document| {
                    matches!(
                        current_paint(&document.doc, id, is_stroke, *index),
                        Some(Fill::Video { .. })
                    )
                });
                if is_video {
                    self.handle_design_video_upload(
                        id,
                        is_stroke,
                        *index,
                        Some(source_id.clone()),
                        None,
                        window,
                        cx,
                    );
                } else {
                    self.handle_design_image_upload(
                        id,
                        is_stroke,
                        *index,
                        Some(source_id.clone()),
                        None,
                        window,
                        cx,
                    );
                }
            }
            DesignPanelAction::PaintMediaSourceDropRequested {
                node_id: id,
                collection,
                index,
                expected_source_id,
                expected_media_kind,
                file,
                ..
            } => {
                let Some(id) = node_id(id) else {
                    return;
                };
                let is_stroke = match collection {
                    DesignPanelCollection::Fill => false,
                    DesignPanelCollection::Stroke => true,
                    _ => return,
                };
                match expected_media_kind {
                    fanta_gpui::design::DesignMediaKind::Image => self.handle_design_image_upload(
                        id,
                        is_stroke,
                        *index,
                        Some(expected_source_id.clone()),
                        Some(file.path.clone()),
                        window,
                        cx,
                    ),
                    fanta_gpui::design::DesignMediaKind::Video => self.handle_design_video_upload(
                        id,
                        is_stroke,
                        *index,
                        Some(expected_source_id.clone()),
                        Some(file.path.clone()),
                        window,
                        cx,
                    ),
                }
            }
            DesignPanelAction::PaintMediaCropActionRequested {
                node_id: id,
                collection,
                index,
                action,
                ..
            } => {
                let Some(id) = node_id(id) else {
                    return;
                };
                let is_stroke = match collection {
                    DesignPanelCollection::Fill => false,
                    DesignPanelCollection::Stroke => true,
                    _ => return,
                };
                self.handle_design_crop_action(id, is_stroke, *index, action, window, cx);
            }
            DesignPanelAction::PaintVideoPreviewActionRequested {
                node_id: id,
                collection,
                paint_id,
                index,
                action,
                ..
            } => {
                let Some(id) = node_id(id) else {
                    return;
                };
                let collection_name = match collection {
                    DesignPanelCollection::Fill => "fill",
                    DesignPanelCollection::Stroke => "stroke",
                    _ => return,
                };
                if paint_id.as_ref() != format!("{id}-{collection_name}-{index}") {
                    return;
                }
                let asset =
                    self.item().read(cx).document().and_then(|document| {
                        match current_paint(
                            &document.doc,
                            id,
                            *collection == DesignPanelCollection::Stroke,
                            *index,
                        ) {
                            Some(Fill::Video { video, .. }) => Some(video.asset),
                            _ => None,
                        }
                    });
                let Some(asset) = asset else {
                    return;
                };
                #[cfg(target_os = "macos")]
                {
                    self.ensure_video_fill_playback(asset, cx);
                    let Some(playback) = self.video_fill_playback(asset) else {
                        return;
                    };
                    playback.update(cx, |playback, cx| {
                        match action {
                            DesignVideoPreviewAction::Play => playback.play(cx),
                            DesignVideoPreviewAction::Pause => playback.pause(cx),
                            DesignVideoPreviewAction::Seek { seconds }
                            | DesignVideoPreviewAction::Scrub {
                                seconds,
                                phase: DesignPanelEditPhase::Preview | DesignPanelEditPhase::Commit,
                            } => {
                                if seconds.is_finite() && *seconds >= 0.0 {
                                    playback.seek((*seconds as f64 * 1_000_000.0) as u64, cx);
                                }
                            }
                            DesignVideoPreviewAction::Scrub { .. } => {}
                        }
                        playback.tick(window, cx);
                    });
                    if let Some(adapter) = self.gpui_design.as_mut() {
                        adapter.last_echo = None;
                    }
                    cx.notify();
                }
                #[cfg(not(target_os = "macos"))]
                {
                    let _ = (asset, action);
                    crate::view::notify_unavailable("Video preview", window, cx);
                }
            }
            DesignPanelAction::PaintShaderApplyRequested {
                node_id: id,
                collection,
                index,
                shader,
                ..
            } => {
                let Some(id) = node_id(id) else {
                    return;
                };
                let catalog = bundled_shader_catalog();
                let Some(payload) = catalog
                    .shader(shader)
                    .and_then(DesignShaderPaint::from_definition)
                else {
                    return;
                };
                let edit = fanta_gpui::design::DesignPaintEdit {
                    property: DesignPaintProperty::Payload,
                    value: DesignPaintValue::Payload(DesignPaintPayload::Shader(payload)),
                };
                self.handle_design_paint_edit(
                    id,
                    *collection,
                    *index,
                    &edit,
                    DesignPanelEditPhase::Commit,
                    window,
                    cx,
                );
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
                        } else if let NodeData::Group(group) = data {
                            let count = usize::from(group.background.is_some())
                                + group.background_fills.len();
                            if from < count && to < count {
                                let has_background = group.background.is_some();
                                let mut fills: Vec<Fill> = group
                                    .background
                                    .take()
                                    .into_iter()
                                    .chain(group.background_fills.drain(..))
                                    .collect();
                                let fill = fills.remove(from);
                                fills.insert(to, fill);
                                if has_background {
                                    group.background = Some(fills.remove(0));
                                }
                                group.background_fills.extend(fills);
                            }
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
                self.handle_design_effect_edit(id, reference, *property, value, *phase, window, cx);
            }
            DesignPanelAction::TypographyPropertyChangeRequested {
                node_id: id,
                target,
                property,
                value,
            } => {
                if target.is_selected_text_range() {
                    crate::view::notify_unavailable("Selected text formatting", window, cx);
                    return;
                }
                let Some(id) = node_id(id) else {
                    return;
                };
                self.finish_document_edits_for_external_change(cx);
                let mut unhandled = false;
                let ops = self.design_ops(cx, |doc| {
                    property_operations(doc, id, *property, value).unwrap_or_else(|| {
                        unhandled = true;
                        Vec::new()
                    })
                });
                if unhandled {
                    log::debug!("fig design adapter: unhandled typography property {property:?}");
                    crate::view::notify_unavailable(UNWIRED_CONTROL, window, cx);
                    return;
                }
                self.design_apply_ops(ops, cx);
            }
            DesignPanelAction::TypographyPropertyEditRequested {
                node_id: id,
                target,
                property,
                value,
                phase,
            } => {
                if target.is_selected_text_range() {
                    if *phase == DesignPanelEditPhase::Commit {
                        crate::view::notify_unavailable("Selected text formatting", window, cx);
                    }
                    return;
                }
                let Some(id) = node_id(id) else {
                    return;
                };
                self.handle_design_phased_edit(id, *property, value, *phase, window, cx);
            }
            DesignPanelAction::TypographyFontApplyRequested {
                node_id: id,
                target,
                font,
            } => {
                let Some(id) = node_id(id) else {
                    return;
                };
                if !matches!(font.source, DesignFontSource::Local) {
                    return;
                }
                let Some((family, style)) = self
                    .gpui_design
                    .as_ref()
                    .and_then(|adapter| adapter.font_catalog.font(font))
                    .filter(|(_, style)| style.availability.can_apply())
                else {
                    return;
                };
                let family = family.name.to_string();
                let style = style.clone();
                if target.is_selected_text_range() {
                    if self.text_selection_typography(id).is_none() {
                        return;
                    }
                    self.with_text_selection_style(cx, |text_style| {
                        text_style.font_family.clone_from(&family);
                        text_style.italic = style.italic;
                        if let Some(weight) = style.weight {
                            text_style.weight = weight;
                        }
                    });
                } else {
                    self.finish_document_edits_for_external_change(cx);
                    let ops =
                        self.design_ops(cx, |doc| font_apply_operations(doc, id, &family, &style));
                    self.design_apply_ops(ops, cx);
                }
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
                    window,
                    cx,
                );
            }
            DesignPanelAction::PageBackgroundEditRequested {
                page_id,
                color,
                phase,
            } => {
                self.handle_design_page_background(page_id, *color, *phase, window, cx);
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
        window: &mut Window,
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
            // A wrapped leaf that is still mid-gesture must stay silent: its
            // Preview frames arrive continuously while a slider is dragged,
            // so one notice per frame would bury the canvas. Multi-selection
            // capabilities render neither paints nor effects today, but these
            // are the first phased leaves a widened capability would emit.
            DesignPanelAction::PaintEditRequested { phase, .. }
            | DesignPanelAction::EffectEditRequested { phase, .. }
                if *phase != DesignPanelEditPhase::Commit =>
            {
                log::debug!("fig design adapter: unhandled targeted leaf {action:?}");
                return;
            }
            _ => {
                log::debug!("fig design adapter: unhandled targeted leaf {action:?}");
                crate::view::notify_unavailable(UNWIRED_CONTROL, window, cx);
                return;
            }
        };
        self.finish_document_edits_for_external_change(cx);
        let mut unsupported = false;
        let ops = self.design_ops(cx, |doc| {
            targeted_property_operations(doc, &ids, property, &value).unwrap_or_else(|| {
                unsupported = true;
                Vec::new()
            })
        });
        if unsupported {
            crate::view::notify_unavailable(UNWIRED_CONTROL, window, cx);
            return;
        }
        self.design_apply_ops(ops, cx);
    }

    fn handle_design_add_auto_layout(
        &mut self,
        target: &DesignPanelTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.design_target_matches_selection(target, cx) {
            log::warn!("fig design adapter: rejecting stale auto layout target");
            return;
        }
        let DesignPanelTarget::Nodes { node_ids } = target else {
            return;
        };
        let ids: Vec<NodeId> = node_ids.iter().filter_map(node_id).collect();
        if ids.is_empty() {
            return;
        }
        self.apply_structure_edit(
            "Add auto layout",
            move |doc| {
                anyhow::ensure!(
                    ids.iter()
                        .all(|id| crate::layer_context_ops::editable(doc, *id)),
                    "A selected layer is locked or is a page"
                );
                if ids.len() == 1 {
                    let id = ids[0];
                    let operations = crate::layer_context_ops::auto_layout(
                        doc,
                        id,
                        Some(LayoutMode::Horizontal),
                    )?;
                    let created = crate::layer_context_ops::created_roots(&operations);
                    return Ok((
                        operations,
                        if created.is_empty() {
                            vec![id]
                        } else {
                            created
                        },
                    ));
                }
                let grouped = crate::structure::frame_selection_operations(doc, &ids, None)?;
                let mut scratch = doc.clone();
                for operation in &grouped.operations {
                    scratch.apply(operation.clone())?;
                }
                let mut operations = grouped.operations;
                operations.extend(crate::layer_context_ops::auto_layout(
                    &scratch,
                    grouped.group,
                    Some(LayoutMode::Horizontal),
                )?);
                Ok((operations, vec![grouped.group]))
            },
            window,
            cx,
        );
    }

    /// Align or distribute the exact ordered target as one history entry.
    fn handle_design_arrange(
        &mut self,
        target: &DesignPanelTarget,
        operation: DesignArrangeOperation,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.design_target_matches_selection(target, cx) {
            log::warn!("fig design adapter: rejecting stale arrange target");
            return;
        }
        let DesignPanelTarget::Nodes { node_ids } = target else {
            return;
        };
        let ids: Vec<NodeId> = node_ids.iter().filter_map(node_id).collect();
        self.finish_document_edits_for_external_change(cx);
        let mut unsupported = false;
        let ops = self.design_ops(cx, |doc| {
            arrange_operations(doc, &ids, operation).unwrap_or_else(|| {
                unsupported = true;
                Vec::new()
            })
        });
        if unsupported {
            crate::view::notify_unavailable("Tidy up", window, cx);
            return;
        }
        self.design_apply_ops(ops, cx);
    }

    /// Rotate or flip the exact ordered target as one history entry.
    fn handle_design_transform(
        &mut self,
        target: &DesignPanelTarget,
        operation: DesignTransformOperation,
        cx: &mut Context<Self>,
    ) {
        if !self.design_target_matches_selection(target, cx) {
            log::warn!("fig design adapter: rejecting stale transform target");
            return;
        }
        let DesignPanelTarget::Nodes { node_ids } = target else {
            return;
        };
        let ids: Vec<NodeId> = node_ids.iter().filter_map(node_id).collect();
        self.finish_document_edits_for_external_change(cx);
        let ops = self.design_ops(cx, |doc| match operation {
            DesignTransformOperation::RotateClockwise90 => rotate_clockwise_operations(doc, &ids),
            DesignTransformOperation::FlipHorizontal => flip_operations(doc, &ids, true),
            DesignTransformOperation::FlipVertical => flip_operations(doc, &ids, false),
        });
        self.design_apply_ops(ops, cx);
    }

    fn handle_design_selection_header_command(
        &mut self,
        target: &DesignPanelTarget,
        command: &DesignSelectionHeaderCommand,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use fanta_gpui::layers::LayersPanelContextAction as LayerAction;

        if !self.design_target_matches_selection(target, cx) {
            log::warn!("fig design adapter: rejecting stale selection-header target");
            return;
        }
        let DesignPanelTarget::Nodes { node_ids } = target else {
            return;
        };
        let ids: Vec<NodeId> = node_ids.iter().filter_map(node_id).collect();
        let Some(first) = ids.first().copied() else {
            return;
        };
        let page = self
            .selected_page_index()
            .and_then(|index| {
                self.item()
                    .read(cx)
                    .document()
                    .and_then(|document| document.doc.pages().get(index).copied())
            })
            .or_else(|| {
                self.item()
                    .read(cx)
                    .document()
                    .and_then(|document| document.doc.active_page())
            });
        let supported = self.item().read(cx).document().is_some_and(|document| {
            let editable = self.is_editable(cx);
            selection_header_for_doc(&document.doc, &ids, page, editable).is_some_and(|header| {
                header
                    .primary_controls
                    .iter()
                    .chain(header.overflow_controls.iter())
                    .any(|control| {
                        control.command().as_ref() == Some(command)
                            || control
                                .menu_items
                                .iter()
                                .any(|item| &item.command == command)
                    })
                    || header
                        .title_menu
                        .as_ref()
                        .is_some_and(|menu| menu.items.iter().any(|item| &item.command == command))
            })
        });
        if !supported {
            log::warn!(
                "fig design adapter: rejecting unavailable selection-header command {command:?}"
            );
            return;
        }
        match command {
            DesignSelectionHeaderCommand::SelectMatchingLayers => {
                let item = self.item().clone();
                item.update(cx, |item, cx| {
                    item.with_document(cx, |document| {
                        let matching =
                            matching_design_layers(&document.doc, first, page).collect::<Vec<_>>();
                        if matching.is_empty() {
                            return ((), DocChange::None);
                        }
                        document.doc.selection.replace_with(matching);
                        ((), DocChange::Selection)
                    });
                });
            }
            DesignSelectionHeaderCommand::CreateComponent => {
                self.apply_structure_edit(
                    "Create component",
                    move |doc| {
                        if ids.len() == 1 {
                            let operations = crate::layer_context_ops::simple(
                                doc,
                                first,
                                LayerAction::CreateComponent,
                            )?;
                            anyhow::ensure!(
                                !operations.is_empty(),
                                "The layer is already a component"
                            );
                            let roots = crate::layer_context_ops::created_roots(&operations);
                            return Ok((
                                operations,
                                if roots.is_empty() { vec![first] } else { roots },
                            ));
                        }
                        let grouped =
                            crate::structure::frame_selection_operations(doc, &ids, None)?;
                        let mut scratch = doc.clone();
                        for operation in &grouped.operations {
                            scratch.apply(operation.clone())?;
                        }
                        let component = crate::properties_ops::create_component_operations(
                            &scratch,
                            grouped.group,
                        );
                        anyhow::ensure!(
                            !component.is_empty(),
                            "Could not create a component from this selection"
                        );
                        let mut operations = grouped.operations;
                        operations.extend(component);
                        Ok((operations, vec![grouped.group]))
                    },
                    window,
                    cx,
                );
            }
            DesignSelectionHeaderCommand::UseAsMask
            | DesignSelectionHeaderCommand::Flatten
            | DesignSelectionHeaderCommand::TitleMenuItem { .. } => {
                let action = match command {
                    DesignSelectionHeaderCommand::UseAsMask => LayerAction::UseAsMask,
                    DesignSelectionHeaderCommand::Flatten => LayerAction::Flatten,
                    DesignSelectionHeaderCommand::TitleMenuItem { item_id }
                        if item_id.as_ref() == "frame" =>
                    {
                        LayerAction::ConvertToFrame
                    }
                    DesignSelectionHeaderCommand::TitleMenuItem { item_id }
                        if item_id.as_ref() == "section" =>
                    {
                        LayerAction::ConvertToSection
                    }
                    _ => return,
                };
                self.apply_structure_edit(
                    action.label(),
                    move |doc| {
                        let operations = crate::layer_context_ops::simple(doc, first, action)?;
                        let roots = crate::layer_context_ops::created_roots(&operations);
                        let selection = if roots.is_empty() { ids } else { roots };
                        Ok((operations, selection))
                    },
                    window,
                    cx,
                );
            }
            DesignSelectionHeaderCommand::Boolean(operation) => {
                let operation = match operation {
                    fanta_gpui::design::DesignBooleanOperation::Union => {
                        fanta_doc::BooleanOp::Union
                    }
                    fanta_gpui::design::DesignBooleanOperation::Subtract => {
                        fanta_doc::BooleanOp::Subtract
                    }
                    fanta_gpui::design::DesignBooleanOperation::Intersect => {
                        fanta_doc::BooleanOp::Intersect
                    }
                    fanta_gpui::design::DesignBooleanOperation::Exclude => {
                        fanta_doc::BooleanOp::Exclude
                    }
                };
                self.finish_document_edits_for_external_change(cx);
                let item = self.item().clone();
                let result = item.update(cx, |item, cx| {
                    item.with_document(cx, |document| {
                        let doc = &mut document.doc;
                        let original = doc.clone();
                        let mut operands = ids;
                        operands.sort_by_key(|id| doc.scene.get(*id).map(|node| node.index));
                        doc.history.begin("Boolean selection", &mut doc.scene);
                        let result = fanta_tools::make_boolean(doc, &operands, operation).filter(
                            |created| doc.scene.children_of(Some(*created)).len() == operands.len(),
                        );
                        match result {
                            Some(created) => {
                                doc.selection.replace_with([created]);
                                doc.history.commit(&mut doc.scene);
                                (Ok(()), DocChange::Content)
                            }
                            None => {
                                *doc = original;
                                (
                                    Err(anyhow::anyhow!("Could not combine these layers")),
                                    DocChange::None,
                                )
                            }
                        }
                    })
                    .unwrap_or_else(|| Err(anyhow::anyhow!("The document is no longer available")))
                });
                if let Err(error) = result {
                    log::warn!("boolean selection failed: {error:#}");
                    crate::view::notify_unavailable("Boolean selection", window, cx);
                }
            }
            DesignSelectionHeaderCommand::EditObject => {
                let kind = self.item().read(cx).document().and_then(|document| {
                    document
                        .doc
                        .scene
                        .get(first)
                        .map(|node| matches!(node.data, NodeData::Text(_)))
                });
                match kind {
                    Some(true) => {
                        self.open_text_edit(first, crate::view::TextEditSeed::SelectAll, window, cx)
                    }
                    Some(false) => self.activate_tool(crate::tools::ToolKind::NodeEdit, cx),
                    None => {}
                }
            }
            DesignSelectionHeaderCommand::CreateLink
            | DesignSelectionHeaderCommand::ApplyTextContentVariable
            | DesignSelectionHeaderCommand::HostDefined { .. } => {}
        }
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
        window: &mut Window,
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
                let mut unhandled = false;
                let ops = self.design_ops(cx, |doc| {
                    property_operations(doc, id, property, value).unwrap_or_else(|| {
                        unhandled = true;
                        Vec::new()
                    })
                });
                let committed = self.design_apply_ops(ops, cx);
                if session.is_some() {
                    item.update(cx, |item, cx| item.finish_content_preview(committed, cx));
                }
                // Only the commit end of a gesture may speak: Preview runs on
                // every frame of a slider drag, so toasting there would fire
                // dozens of notices for one drag.
                if unhandled {
                    log::debug!("fig design adapter: unhandled property {property:?}");
                    crate::view::notify_unavailable(UNWIRED_CONTROL, window, cx);
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

    fn handle_design_crop_action(
        &mut self,
        id: NodeId,
        is_stroke: bool,
        index: usize,
        action: &DesignMediaCropAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let collection = if is_stroke {
            DesignPanelCollection::Stroke
        } else {
            DesignPanelCollection::Fill
        };
        let media = self
            .item()
            .read(cx)
            .document()
            .and_then(|document| media_payload_for(&document.doc, id, is_stroke, index));
        let Some(mut media) = media else {
            return;
        };
        match action {
            DesignMediaCropAction::Begin => {
                let DesignMediaPaintPlacement::Crop { transform } = media_placement(&media) else {
                    return;
                };
                self.handle_design_phased_edit(
                    id,
                    DesignPanelProperty::Opacity,
                    &DesignPanelValue::Number(0.0),
                    DesignPanelEditPhase::Begin,
                    window,
                    cx,
                );
                if let Some(adapter) = self.gpui_design.as_mut() {
                    adapter.crop_session = Some(DesignCropSession {
                        node: id,
                        is_stroke,
                        index,
                        state: DesignMediaCropToolState {
                            active: true,
                            transform,
                            ..DesignMediaCropToolState::default()
                        },
                    });
                    adapter.last_echo = None;
                }
                self.refresh_gpui_design(cx);
            }
            DesignMediaCropAction::Preview {
                transform,
                zoom,
                aspect_ratio,
            }
            | DesignMediaCropAction::Commit {
                transform,
                zoom,
                aspect_ratio,
            } => {
                let Some(adapter) = self.gpui_design.as_mut() else {
                    return;
                };
                let Some(session) = adapter.crop_session.as_mut().filter(|session| {
                    session.node == id && session.is_stroke == is_stroke && session.index == index
                }) else {
                    return;
                };
                session.state.transform = *transform;
                session.state.zoom = *zoom;
                session.state.aspect_ratio = *aspect_ratio;
                let state = session.state;
                adapter.last_echo = None;
                let source_aspect = self.item().read(cx).document().and_then(|document| {
                    let source = match current_paint(&document.doc, id, is_stroke, index)? {
                        Fill::Image { asset, .. } => *asset,
                        Fill::Video { video, .. } => video.poster?,
                        _ => return None,
                    };
                    let image = document.asset_resolver.as_ref()?.resolve(source)?;
                    Some(image.width as f32 / image.height.max(1) as f32)
                });
                let Some(transform) =
                    source_aspect.and_then(|aspect| crop_transform_for_state(state, aspect))
                else {
                    crate::view::show_canvas_notice(
                        "This crop transform cannot be applied.".into(),
                        window,
                        cx,
                    );
                    return;
                };
                set_media_placement(&mut media, DesignMediaPaintPlacement::Crop { transform });
                let edit = fanta_gpui::design::DesignPaintEdit {
                    property: DesignPaintProperty::Payload,
                    value: DesignPaintValue::Payload(media),
                };
                let phase = if matches!(action, DesignMediaCropAction::Commit { .. }) {
                    DesignPanelEditPhase::Commit
                } else {
                    DesignPanelEditPhase::Preview
                };
                self.handle_design_paint_edit(id, collection, index, &edit, phase, window, cx);
                if phase == DesignPanelEditPhase::Commit
                    && let Some(adapter) = self.gpui_design.as_mut()
                {
                    adapter.crop_session = None;
                    adapter.last_echo = None;
                }
                self.refresh_gpui_design(cx);
            }
            DesignMediaCropAction::Cancel => {
                self.handle_design_phased_edit(
                    id,
                    DesignPanelProperty::Opacity,
                    &DesignPanelValue::Number(0.0),
                    DesignPanelEditPhase::Cancel,
                    window,
                    cx,
                );
                if let Some(adapter) = self.gpui_design.as_mut() {
                    adapter.crop_session = None;
                    adapter.last_echo = None;
                }
                self.refresh_gpui_design(cx);
            }
            DesignMediaCropAction::ResizeToFit => {
                set_media_placement(
                    &mut media,
                    DesignMediaPaintPlacement::Fit {
                        rotation: DesignMediaQuarterTurn::None,
                    },
                );
                let edit = fanta_gpui::design::DesignPaintEdit {
                    property: DesignPaintProperty::Payload,
                    value: DesignPaintValue::Payload(media),
                };
                self.handle_design_paint_edit(
                    id,
                    collection,
                    index,
                    &edit,
                    DesignPanelEditPhase::Commit,
                    window,
                    cx,
                );
                if let Some(adapter) = self.gpui_design.as_mut() {
                    adapter.crop_session = None;
                    adapter.last_echo = None;
                }
                self.refresh_gpui_design(cx);
            }
        }
    }

    fn handle_design_image_upload(
        &mut self,
        id: NodeId,
        is_stroke: bool,
        index: usize,
        expected_source_id: Option<SharedString>,
        path: Option<std::path::PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.finish_document_edits_for_external_change(cx);
        let item = self.item().clone();
        let expected = item.read(cx).document().and_then(|document| {
            Some((
                document.doc.id,
                current_paint(&document.doc, id, is_stroke, index)?.clone(),
            ))
        });
        let Some((document_id, expected_fill)) = expected else {
            return;
        };
        if let Some(source_id) = expected_source_id
            && !matches!(&expected_fill, Fill::Image { asset, .. } if asset.to_string() == source_id.as_ref())
        {
            return;
        }
        let chooser = path.is_none().then(|| {
            cx.prompt_for_paths(gpui::PathPromptOptions {
                files: true,
                directories: false,
                multiple: false,
                prompt: Some("Choose image fill".into()),
            })
        });
        cx.spawn_in(window, async move |this, cx| {
            let result: anyhow::Result<()> = async {
                let path = match (path, chooser) {
                    (Some(path), _) => path,
                    (None, Some(chooser)) => {
                        let Some(path) = chooser.await??.and_then(|paths| paths.into_iter().next())
                        else {
                            return Ok(());
                        };
                        #[cfg(all(target_os = "macos", feature = "mac_app_store"))]
                        workspace::remember_user_selected_paths(std::slice::from_ref(&path))?;
                        path
                    }
                    (None, None) => return Ok(()),
                };
                let prepared = cx
                    .background_spawn(async move {
                        let length = std::fs::metadata(&path)
                            .with_context(|| format!("reading {}", path.display()))?
                            .len();
                        anyhow::ensure!(
                            usize::try_from(length)
                                .ok()
                                .is_some_and(|length| length <= MAX_IMAGE_SOURCE_BYTES),
                            "image must be smaller than {MAX_IMAGE_SOURCE_BYTES} bytes"
                        );
                        let bytes = std::fs::read(&path)
                            .with_context(|| format!("reading {}", path.display()))?;
                        PreparedImage::new(bytes)
                    })
                    .await?;
                item.update(cx, |item, cx| {
                    anyhow::ensure!(item.is_editable(), "this document is read-only");
                    item.with_document(cx, |document| {
                        let result: anyhow::Result<bool> = (|| {
                            let (doc, mut assets) = document.doc_and_assets();
                            anyhow::ensure!(doc.id == document_id, "the document changed");
                            replace_image_paint_with_asset(
                                doc,
                                &mut assets,
                                id,
                                is_stroke,
                                index,
                                &expected_fill,
                                prepared,
                            )
                        })();
                        let change = if result.as_ref().is_ok_and(|changed| *changed) {
                            DocChange::Content
                        } else {
                            DocChange::None
                        };
                        (result, change)
                    })
                    .context("the document is closed")??;
                    Ok(())
                })?;
                Ok(())
            }
            .await;
            if let Err(error) = result {
                this.update_in(cx, |_, window, cx| {
                    crate::view::show_canvas_notice(format!("Image fill: {error:#}"), window, cx)
                })?;
            }
            Ok::<(), anyhow::Error>(())
        })
        .detach_and_log_err(cx);
    }

    fn handle_design_video_upload(
        &mut self,
        id: NodeId,
        is_stroke: bool,
        index: usize,
        expected_source_id: Option<SharedString>,
        path: Option<std::path::PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.finish_document_edits_for_external_change(cx);
        let item = self.item().clone();
        let expected = item.read(cx).document().and_then(|document| {
            Some((
                document.doc.id,
                current_paint(&document.doc, id, is_stroke, index)?.clone(),
            ))
        });
        let Some((document_id, expected_fill)) = expected else {
            return;
        };
        if let Some(source_id) = expected_source_id
            && !matches!(&expected_fill, Fill::Video { video, .. } if video.asset.to_string() == source_id.as_ref())
        {
            return;
        }
        let chooser = path.is_none().then(|| {
            cx.prompt_for_paths(gpui::PathPromptOptions {
                files: true,
                directories: false,
                multiple: false,
                prompt: Some("Choose video fill".into()),
            })
        });
        cx.spawn_in(window, async move |this, cx| {
            let result: anyhow::Result<()> = async {
                let path = match (path, chooser) {
                    (Some(path), _) => path,
                    (None, Some(chooser)) => {
                        let Some(path) = chooser.await??.and_then(|paths| paths.into_iter().next())
                        else {
                            return Ok(());
                        };
                        #[cfg(all(target_os = "macos", feature = "mac_app_store"))]
                        workspace::remember_user_selected_paths(std::slice::from_ref(&path))?;
                        path
                    }
                    (None, None) => return Ok(()),
                };
                let prepared = cx
                    .background_spawn(async move {
                        let length = std::fs::metadata(&path)
                            .with_context(|| format!("reading {}", path.display()))?
                            .len();
                        anyhow::ensure!(
                            length > 0 && length <= 100 * 1024 * 1024,
                            "video must be nonempty and no larger than 100 MiB"
                        );
                        let bytes = std::fs::read(&path)
                            .with_context(|| format!("reading {}", path.display()))?;
                        crate::generation_media::prepare_video(Arc::from(bytes)).await
                    })
                    .await?;
                item.update(cx, |item, cx| {
                    anyhow::ensure!(item.is_editable(), "this document is read-only");
                    item.with_document(cx, |document| {
                        let result = replace_video_paint_with_asset(
                            document,
                            id,
                            is_stroke,
                            index,
                            document_id,
                            &expected_fill,
                            prepared,
                        );
                        let change = if result.as_ref().is_ok_and(|changed| *changed) {
                            DocChange::Content
                        } else {
                            DocChange::None
                        };
                        (result, change)
                    })
                    .context("the document is closed")??;
                    Ok(())
                })?;
                Ok(())
            }
            .await;
            if let Err(error) = result {
                this.update_in(cx, |_, window, cx| {
                    crate::view::show_canvas_notice(format!("Video fill: {error:#}"), window, cx)
                })?;
            }
            Ok::<(), anyhow::Error>(())
        })
        .detach_and_log_err(cx);
    }

    fn handle_design_paint_edit(
        &mut self,
        id: NodeId,
        collection: DesignPanelCollection,
        index: usize,
        edit: &fanta_gpui::design::DesignPaintEdit,
        phase: DesignPanelEditPhase,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let is_stroke = match collection {
            DesignPanelCollection::Fill => false,
            DesignPanelCollection::Stroke => true,
            _ => return,
        };
        if phase == DesignPanelEditPhase::Commit
            && let (
                DesignPaintProperty::Payload,
                DesignPaintValue::Payload(DesignPaintPayload::Image(image)),
            ) = (&edit.property, &edit.value)
            && image.source.id.is_empty()
        {
            self.handle_design_image_upload(id, is_stroke, index, None, None, window, cx);
            return;
        }
        if phase == DesignPanelEditPhase::Commit
            && let (
                DesignPaintProperty::Payload,
                DesignPaintValue::Payload(DesignPaintPayload::Video(video)),
            ) = (&edit.property, &edit.value)
            && video.source.id.is_empty()
        {
            self.handle_design_video_upload(id, is_stroke, index, None, None, window, cx);
            return;
        }
        let mut value = match (&edit.property, &edit.value) {
            (DesignPaintProperty::Color, DesignPaintValue::Color(color)) => {
                PaintEditValue::Color(fanta_color(*color))
            }
            (DesignPaintProperty::Opacity, DesignPaintValue::Number(percent))
                if percent.is_finite() =>
            {
                PaintEditValue::Opacity(f64::from(*percent))
            }
            (DesignPaintProperty::BlendMode, DesignPaintValue::BlendMode(mode)) => {
                let Some(mode) = engine_blend_mode(*mode) else {
                    if phase == DesignPanelEditPhase::Commit {
                        crate::view::notify_unavailable("This blend mode", window, cx);
                    }
                    return;
                };
                PaintEditValue::BlendMode(mode)
            }
            (DesignPaintProperty::Payload, DesignPaintValue::Payload(payload)) => {
                PaintEditValue::Payload(payload.clone())
            }
            (DesignPaintProperty::GradientKind, DesignPaintValue::PaintKind(kind))
                if kind.is_gradient() =>
            {
                PaintEditValue::GradientKind(*kind)
            }
            (DesignPaintProperty::GradientTransform, DesignPaintValue::Transform(transform)) => {
                PaintEditValue::GradientTransform(*transform)
            }
            (
                DesignPaintProperty::GradientStopColor { index, .. },
                DesignPaintValue::Color(color),
            ) => PaintEditValue::GradientStopColor(*index, fanta_color(*color)),
            (
                DesignPaintProperty::GradientStopPosition { index, .. },
                DesignPaintValue::Number(position),
            ) if position.is_finite() => PaintEditValue::GradientStopPosition(*index, *position),
            (DesignPaintProperty::GradientStopAdd, DesignPaintValue::GradientStop(stop)) => {
                PaintEditValue::GradientStopAdd(stop.position, fanta_color(stop.color))
            }
            (DesignPaintProperty::GradientStopRemove { index, .. }, DesignPaintValue::None) => {
                PaintEditValue::GradientStopRemove(*index)
            }
            (
                DesignPaintProperty::PatternSourceNode
                | DesignPaintProperty::PatternTileType
                | DesignPaintProperty::PatternScalingFactor
                | DesignPaintProperty::PatternSpacing
                | DesignPaintProperty::PatternHorizontalAlignment
                | DesignPaintProperty::MediaScaleMode
                | DesignPaintProperty::MediaCropTransform
                | DesignPaintProperty::MediaTileScalingFactor
                | DesignPaintProperty::MediaQuarterTurn
                | DesignPaintProperty::MediaFilter(_)
                | DesignPaintProperty::ShaderProperty { .. },
                _,
            ) => PaintEditValue::ModelEdit(edit.clone()),
            _ => {
                log::debug!(
                    "fig design adapter: unhandled paint edit {:?}",
                    edit.property
                );
                return;
            }
        };
        if phase == DesignPanelEditPhase::Commit
            && let PaintEditValue::Payload(DesignPaintPayload::Pattern(pattern)) = &mut value
            && pattern.source_node_id.is_empty()
        {
            let source = self.item().read(cx).document().and_then(|document| {
                pattern_source_candidates(&document.doc, id)
                    .first()
                    .map(|(source, _)| *source)
            });
            let Some(source) = source else {
                crate::view::show_canvas_notice(
                    "Add another vector or frame to use as a pattern source.".into(),
                    window,
                    cx,
                );
                return;
            };
            pattern.source_node_id = source.to_string().into();
        }
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
                    window,
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
                    window,
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
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match phase {
            DesignPanelEditPhase::Begin => {
                self.handle_design_phased_edit(
                    id,
                    property,
                    value,
                    DesignPanelEditPhase::Begin,
                    window,
                    cx,
                );
                return;
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
                        if let Some(operations) =
                            effect_edit_operations(&document.doc, id, reference, property, value)
                        {
                            for operation in &operations {
                                apply_preview_operation(&mut document.doc, operation);
                            }
                        }
                        ((), DocChange::ContentPreview)
                    });
                });
                return;
            }
            DesignPanelEditPhase::Cancel => {
                self.finish_gpui_design_edits(cx);
                return;
            }
            DesignPanelEditPhase::Commit => {}
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
            let kind = current_prop_kind(doc, id, prop);
            if !matches!(
                kind,
                Some(
                    EnginePropKind::Boolean
                        | EnginePropKind::Number
                        | EnginePropKind::Text
                        | EnginePropKind::Color
                )
            ) {
                return Vec::new();
            }
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
                    Some(EnginePropKind::Color) => match parse_color(text) {
                        Some(value) => Some(VarValue::Color { value }),
                        None => return Vec::new(),
                    },
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
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(root) = node_id(page_id) else {
            return;
        };
        let item = self.item().clone();
        if item
            .read(cx)
            .document()
            .is_none_or(|document| document.doc.active_page() != Some(root))
        {
            return;
        }
        let color = fanta_color(color);
        let build = |doc: &Doc| {
            replace_data_operation(doc, root, |data| {
                if let NodeData::Group(group) = data {
                    group.background = Some(Fill::solid(color));
                }
            })
        };
        match phase {
            DesignPanelEditPhase::Begin => self.handle_design_phased_edit(
                root,
                DesignPanelProperty::Opacity,
                &DesignPanelValue::Number(0.0),
                DesignPanelEditPhase::Begin,
                window,
                cx,
            ),
            DesignPanelEditPhase::Preview => {
                let Some(snapshot) = self
                    .gpui_design
                    .as_ref()
                    .and_then(|adapter| adapter.session.as_ref())
                    .filter(|session| session.node == root)
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
                        for operation in build(&document.doc) {
                            apply_preview_operation(&mut document.doc, &operation);
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
                    .filter(|session| session.node == root);
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
            DesignPanelEditPhase::Cancel => self.finish_gpui_design_edits(cx),
        }
    }
}

// =============================================================================
// Intent → operation builders
// =============================================================================

fn targeted_property_operations(
    doc: &Doc,
    ids: &[NodeId],
    property: DesignPanelProperty,
    value: &DesignPanelValue,
) -> Option<Vec<Operation>> {
    if matches!(
        property,
        DesignPanelProperty::X
            | DesignPanelProperty::Y
            | DesignPanelProperty::Width
            | DesignPanelProperty::Height
    ) {
        let coordinate = match value {
            DesignPanelValue::Number(value) => f64::from(*value),
            DesignPanelValue::Integer(value) => *value as f64,
            _ => return None,
        };
        if !coordinate.is_finite() {
            return None;
        }
        let bounds = ids
            .iter()
            .try_fold(None, |bounds: Option<fanta_doc::Bounds>, id| {
                let current = doc.scene.world_bounds(*id)?;
                Some(Some(
                    bounds.map_or(current, |bounds| bounds.union(&current)),
                ))
            })??;
        let world_change = match property {
            DesignPanelProperty::X => {
                if coordinate == bounds.min_x {
                    return Some(Vec::new());
                }
                Transform2D::translation(coordinate - bounds.min_x, 0.0)
            }
            DesignPanelProperty::Y => {
                if coordinate == bounds.min_y {
                    return Some(Vec::new());
                }
                Transform2D::translation(0.0, coordinate - bounds.min_y)
            }
            DesignPanelProperty::Width => {
                let width = bounds.max_x - bounds.min_x;
                if coordinate <= 0.0 || width <= 0.0 {
                    return None;
                }
                if coordinate == width {
                    return Some(Vec::new());
                }
                Transform2D::translation(-bounds.min_x, 0.0)
                    .then(&Transform2D::scale_xy(coordinate / width, 1.0))
                    .then(&Transform2D::translation(bounds.min_x, 0.0))
            }
            DesignPanelProperty::Height => {
                let height = bounds.max_y - bounds.min_y;
                if coordinate <= 0.0 || height <= 0.0 {
                    return None;
                }
                if coordinate == height {
                    return Some(Vec::new());
                }
                Transform2D::translation(0.0, -bounds.min_y)
                    .then(&Transform2D::scale_xy(1.0, coordinate / height))
                    .then(&Transform2D::translation(0.0, bounds.min_y))
            }
            _ => return None,
        };
        return ids
            .iter()
            .map(|id| {
                let node = doc.scene.get(*id)?;
                let parent_world = node
                    .parent
                    .and_then(|parent| doc.scene.world_transform(parent))
                    .unwrap_or(Transform2D::IDENTITY);
                let determinant = parent_world.0.matrix2.determinant();
                if !determinant.is_finite() || determinant.abs() <= f64::EPSILON {
                    return None;
                }
                Some(Operation::SetTransform {
                    id: *id,
                    old: node.transform,
                    new: node
                        .transform
                        .then(&parent_world)
                        .then(&world_change)
                        .then(&parent_world.inverse()),
                })
            })
            .collect();
    }

    ids.iter()
        .map(|id| property_operations(doc, *id, property, value))
        .collect::<Option<Vec<_>>>()
        .map(|operations| operations.into_iter().flatten().collect())
}

/// Maps one panel arrange command onto engine operations.
///
/// `distribute` deliberately returns nothing for fewer than three nodes:
/// two nodes are already as far apart as they can be, so there is no gap to
/// equalize. That is engine behavior, not a missing case.
fn arrange_operations(
    doc: &Doc,
    ids: &[NodeId],
    operation: DesignArrangeOperation,
) -> Option<Vec<Operation>> {
    use fanta_canvas::{Axis, HAlign, VAlign, align_horizontal, align_vertical, distribute};

    let scene = &doc.scene;
    Some(match operation {
        DesignArrangeOperation::AlignLeft => align_horizontal(scene, ids, HAlign::Left),
        DesignArrangeOperation::AlignHorizontalCenter => {
            align_horizontal(scene, ids, HAlign::Center)
        }
        DesignArrangeOperation::AlignRight => align_horizontal(scene, ids, HAlign::Right),
        DesignArrangeOperation::AlignTop => align_vertical(scene, ids, VAlign::Top),
        DesignArrangeOperation::AlignVerticalCenter => align_vertical(scene, ids, VAlign::Middle),
        DesignArrangeOperation::AlignBottom => align_vertical(scene, ids, VAlign::Bottom),
        DesignArrangeOperation::DistributeHorizontal => distribute(scene, ids, Axis::X),
        DesignArrangeOperation::DistributeVertical => distribute(scene, ids, Axis::Y),
        DesignArrangeOperation::TidyUp => {
            if ids.len() < 2 {
                return Some(Vec::new());
            }
            let bounds = ids
                .iter()
                .map(|id| scene.world_bounds(*id))
                .collect::<Option<Vec<_>>>()?;
            let centers_x = bounds.iter().map(|bounds| bounds.center().x);
            let centers_y = bounds.iter().map(|bounds| bounds.center().y);
            let horizontal_span = centers_x.clone().fold(f64::NEG_INFINITY, f64::max)
                - centers_x.fold(f64::INFINITY, f64::min);
            let vertical_span = centers_y.clone().fold(f64::NEG_INFINITY, f64::max)
                - centers_y.fold(f64::INFINITY, f64::min);
            let horizontal = horizontal_span >= vertical_span;
            let mut operations = if horizontal {
                align_vertical(scene, ids, VAlign::Middle)
            } else {
                align_horizontal(scene, ids, HAlign::Center)
            };
            let mut scratch = doc.clone();
            for operation in &operations {
                if let Err(error) = scratch.apply(operation.clone()) {
                    log::warn!("tidy up alignment failed: {error:#}");
                    return None;
                }
            }
            operations.extend(distribute(
                &scratch.scene,
                ids,
                if horizontal { Axis::X } else { Axis::Y },
            ));
            operations
        }
    })
}

/// Turns every target a further 90° clockwise.
///
/// Each member turns about its own center rather than orbiting the selection
/// box: the rotation goes through the same `InspectorField::Rotation` writer
/// the numeric field uses, so the value the panel then echoes back is exactly
/// the one the button produced.
fn rotate_clockwise_operations(doc: &Doc, ids: &[NodeId]) -> Vec<Operation> {
    ids.iter()
        .flat_map(|id| {
            let field = InspectorField::Rotation(*id);
            let Some(degrees) = read_field_text(doc, &field)
                .as_deref()
                .and_then(|text| parse_number(text.trim_end_matches('°')))
            else {
                return Vec::new();
            };
            field_operations(doc, &field, &format_number(degrees + 90.0))
        })
        .collect()
}

/// Mirrors every target about the selection's world-bounds center axis.
///
/// Mirroring the whole selection box — rather than each node about its own
/// center — is Figma's behavior and is what makes a multi-node flip reverse
/// the members' order; for a lone node the two are the same thing.
fn flip_operations(doc: &Doc, ids: &[NodeId], horizontal: bool) -> Vec<Operation> {
    let scene = &doc.scene;
    let mut selection_bounds: Option<fanta_doc::Bounds> = None;
    for id in ids {
        if let Some(bounds) = scene.world_bounds(*id) {
            selection_bounds = Some(match selection_bounds {
                Some(accumulated) => accumulated.union(&bounds),
                None => bounds,
            });
        }
    }
    let Some(selection_bounds) = selection_bounds else {
        return Vec::new();
    };
    let center = selection_bounds.center();
    let (scale_x, scale_y) = if horizontal { (-1.0, 1.0) } else { (1.0, -1.0) };
    let mirror = Transform2D::translation(-center.x, -center.y)
        .then(&Transform2D::scale_xy(scale_x, scale_y))
        .then(&Transform2D::translation(center.x, center.y));

    let mut operations = Vec::with_capacity(ids.len());
    for id in ids {
        let Some(node) = scene.get(*id) else {
            continue;
        };
        let parent_world = node
            .parent
            .and_then(|parent| scene.world_transform(parent))
            .unwrap_or(Transform2D::IDENTITY);
        let determinant = parent_world.0.matrix2.determinant();
        if !determinant.is_finite() || determinant.abs() <= f64::EPSILON {
            continue;
        }
        // The mirror is a world-space map, so it wraps the node's world
        // transform and the result is pulled back into the parent's frame.
        let new = node
            .transform
            .then(&parent_world)
            .then(&mirror)
            .then(&parent_world.inverse());
        operations.push(Operation::SetTransform {
            id: *id,
            old: node.transform,
            new,
        });
    }
    operations
}

enum PaintEditValue {
    Color(FantaColor),
    Opacity(f64),
    BlendMode(BlendMode),
    Payload(DesignPaintPayload),
    ModelEdit(fanta_gpui::design::DesignPaintEdit),
    GradientKind(DesignPaintKind),
    GradientTransform(DesignPaintTransform),
    GradientStopColor(usize, FantaColor),
    GradientStopPosition(usize, f32),
    GradientStopAdd(f32, FantaColor),
    GradientStopRemove(usize),
}

fn pattern_source_is_valid(doc: &Doc, target: NodeId, source: NodeId) -> bool {
    source != target
        && doc
            .scene
            .get(source)
            .is_some_and(|node| matches!(&node.data, NodeData::Vector(_) | NodeData::Group(_)))
        && !doc.scene.ancestors_of(target).any(|node| node.id == source)
}

fn pattern_source_candidates(doc: &Doc, target: NodeId) -> Vec<(NodeId, String)> {
    let ancestors = doc
        .scene
        .ancestors_of(target)
        .map(|node| node.id)
        .collect::<HashSet<_>>();
    let page_root = doc
        .scene
        .ancestors_of(target)
        .last()
        .map_or(target, |node| node.id);
    doc.scene
        .descendants_of(page_root)
        .filter(|candidate| *candidate != target && !ancestors.contains(candidate))
        .filter_map(|candidate| {
            doc.scene.get(candidate).and_then(|node| {
                matches!(&node.data, NodeData::Vector(_) | NodeData::Group(_))
                    .then(|| (candidate, node.name.clone()))
            })
        })
        .collect()
}

fn pattern_fill_from_design(
    doc: &Doc,
    target: NodeId,
    payload: &DesignPatternPaint,
) -> Option<PatternFill> {
    let source_node_id = payload.source_node_id.parse().ok()?;
    if !pattern_source_is_valid(doc, target, source_node_id)
        || !payload.scaling_factor.is_finite()
        || payload.scaling_factor <= 0.0
        || !payload.spacing.x.is_finite()
        || !payload.spacing.y.is_finite()
    {
        return None;
    }
    Some(PatternFill {
        source_node_id,
        tile_type: match payload.tile_type {
            DesignPatternTileType::Rectangular => PatternTileType::Rectangular,
            DesignPatternTileType::HorizontalHexagonal => PatternTileType::HorizontalHexagonal,
            DesignPatternTileType::VerticalHexagonal => PatternTileType::VerticalHexagonal,
        },
        scaling_factor: payload.scaling_factor,
        spacing: PatternSpacing {
            x: payload.spacing.x,
            y: payload.spacing.y,
        },
        horizontal_alignment: match payload.horizontal_alignment {
            DesignPatternHorizontalAlignment::Start => PatternHorizontalAlignment::Start,
            DesignPatternHorizontalAlignment::Center => PatternHorizontalAlignment::Center,
            DesignPatternHorizontalAlignment::End => PatternHorizontalAlignment::End,
        },
    })
}

fn image_fill_from_design(
    payload: &fanta_gpui::design::DesignImagePaint,
    opacity: f32,
    blend: BlendMode,
) -> Option<Fill> {
    let asset = payload.source.id.parse().ok()?;
    let (mode, crop, scale, rotation) = engine_media_placement(payload.placement)?;
    let filters = payload.filters.normalized()?;
    Some(Fill::Image {
        asset,
        mode,
        opacity,
        crop,
        scale,
        rotation,
        blend,
        adjust: engine_image_filters(filters),
    })
}

fn engine_media_placement(
    placement: DesignMediaPaintPlacement,
) -> Option<(
    ImageFitMode,
    Option<Box<[f32; 4]>>,
    Option<f32>,
    Option<f32>,
)> {
    if !placement.is_valid() {
        return None;
    }
    Some(match placement {
        DesignMediaPaintPlacement::Fill { rotation } => (
            ImageFitMode::Fill,
            None,
            None,
            Some(f32::from(rotation.degrees())),
        ),
        DesignMediaPaintPlacement::Fit { rotation } => (
            ImageFitMode::Fit,
            None,
            None,
            Some(f32::from(rotation.degrees())),
        ),
        DesignMediaPaintPlacement::Crop { transform } => {
            if transform.m12.abs() > 1e-6
                || transform.m21.abs() > 1e-6
                || transform.m11 <= 0.0
                || transform.m22 <= 0.0
            {
                return None;
            }
            (
                ImageFitMode::Fill,
                Some(Box::new([
                    transform.tx,
                    transform.ty,
                    transform.m11,
                    transform.m22,
                ])),
                None,
                None,
            )
        }
        DesignMediaPaintPlacement::Tile {
            scaling_factor,
            rotation,
        } => (
            ImageFitMode::Tile,
            None,
            Some(scaling_factor),
            Some(f32::from(rotation.degrees())),
        ),
    })
}

fn engine_image_filters(filters: DesignImageFilters) -> fanta_doc::ImageAdjust {
    fanta_doc::ImageAdjust {
        exposure: filters.exposure,
        contrast: filters.contrast,
        saturation: filters.saturation,
        temperature: filters.temperature,
        tint: filters.tint,
        highlights: filters.highlights,
        shadows: filters.shadows,
    }
}

fn video_fill_from_design(
    payload: &fanta_gpui::design::DesignVideoPaint,
    previous: Option<&fanta_doc::VideoFill>,
    opacity: f32,
    blend: BlendMode,
) -> Option<Fill> {
    let asset = payload.source.id.parse().ok()?;
    let (mode, crop, scale, rotation) = engine_media_placement(payload.placement)?;
    let filters = payload.filters.normalized()?;
    Some(Fill::Video {
        video: Box::new(fanta_doc::VideoFill {
            asset,
            poster: previous
                .filter(|previous| previous.asset == asset)
                .and_then(|previous| previous.poster),
            mode,
            crop,
            scale,
            rotation,
            adjust: engine_image_filters(filters),
        }),
        opacity,
        blend,
    })
}

fn shader_fill_from_design(
    payload: &DesignShaderPaint,
    previous: Option<&fanta_doc::ShaderFill>,
    opacity: f32,
    blend: BlendMode,
) -> Option<Fill> {
    let catalog = bundled_shader_catalog();
    let definition = catalog.definition(&payload.shader_id)?;
    let mut properties: Vec<fanta_doc::ShaderPropertyAssignment> = payload
        .properties
        .iter()
        .filter_map(|assignment| {
            if definition
                .property(&assignment.definition_id)
                .is_some_and(|property| !assignment.value.is_compatible_with(property.kind))
            {
                return None;
            }
            engine_shader_value(&assignment.value).map(|value| {
                fanta_doc::ShaderPropertyAssignment {
                    definition_id: assignment.definition_id.to_string(),
                    value,
                }
            })
        })
        .collect();
    if let Some(previous) = previous.filter(|previous| previous.shader_id == payload.shader_id) {
        for assignment in &previous.properties {
            let unknown_or_opaque = definition.property(&assignment.definition_id).is_none()
                || matches!(
                    &assignment.value,
                    fanta_doc::ShaderPropertyValue::Opaque { .. }
                );
            if unknown_or_opaque
                && !properties
                    .iter()
                    .any(|property| property.definition_id == assignment.definition_id)
            {
                properties.push(assignment.clone());
            }
        }
    }
    Some(Fill::Shader {
        shader: Box::new(fanta_doc::ShaderFill {
            shader_id: definition.id.to_string(),
            name: definition.name.to_string(),
            properties,
        }),
        opacity,
        blend,
    })
}

fn paint_opacity_fraction(fill: &Fill) -> f32 {
    match fill {
        Fill::Solid { color, .. } => f32::from(color.a) / 255.0,
        Fill::Gradient { gradient, .. } => crate::color_picker::gradient_stops(gradient)
            .iter()
            .map(|stop| f32::from(stop.color.a) / 255.0)
            .reduce(f32::max)
            .unwrap_or(1.0),
        Fill::Image { opacity, .. }
        | Fill::Pattern { opacity, .. }
        | Fill::Video { opacity, .. }
        | Fill::Shader { opacity, .. } => *opacity,
    }
}

fn current_paint(doc: &Doc, id: NodeId, is_stroke: bool, index: usize) -> Option<&Fill> {
    let node = doc.scene.get(id)?;
    match &node.data {
        NodeData::Vector(vector) => {
            if is_stroke {
                vector.strokes.get(index).map(|stroke| &stroke.paint)
            } else {
                vector.fills.get(index)
            }
        }
        NodeData::Group(group) => {
            if is_stroke {
                group.strokes.get(index).map(|stroke| &stroke.paint)
            } else {
                group
                    .background
                    .iter()
                    .chain(group.background_fills.iter())
                    .nth(index)
            }
        }
        _ => None,
    }
}

fn replace_image_paint_with_asset(
    doc: &mut Doc,
    assets: &mut AssetStores<'_>,
    id: NodeId,
    is_stroke: bool,
    index: usize,
    expected_fill: &Fill,
    prepared: PreparedImage,
) -> anyhow::Result<bool> {
    anyhow::ensure!(
        current_paint(doc, id, is_stroke, index) == Some(expected_fill),
        "the paint changed while choosing an image"
    );
    let (asset, _, inserted) = assets.add_prepared_image_tracked(prepared)?;
    let mut new_fill = match expected_fill {
        Fill::Image { .. } => expected_fill.clone(),
        other => Fill::Image {
            asset,
            mode: ImageFitMode::Fill,
            opacity: paint_opacity_fraction(other),
            crop: None,
            scale: None,
            rotation: None,
            blend: crate::properties_ops::paint_blend(other),
            adjust: fanta_doc::ImageAdjust::default(),
        },
    };
    if let Fill::Image { asset: source, .. } = &mut new_fill {
        *source = asset;
    }
    let operations = replace_data_operation(doc, id, |data| {
        if let Some(paint) = crate::properties_ops::paint_slot_mut(data, index, is_stroke) {
            *paint = new_fill.clone();
        }
    });
    let mut changed = false;
    for operation in operations {
        if let Err(error) = doc.apply(operation) {
            if inserted {
                assets.remove(asset);
            }
            return Err(error.into());
        }
        changed = true;
    }
    if !changed && inserted {
        assets.remove(asset);
    }
    Ok(changed)
}

fn replace_video_paint_with_asset(
    document: &mut FigDocument,
    id: NodeId,
    is_stroke: bool,
    index: usize,
    document_id: fanta_doc::DocId,
    expected_fill: &Fill,
    prepared: crate::generation_media::PreparedVideo,
) -> anyhow::Result<bool> {
    anyhow::ensure!(document.doc.id == document_id, "the document changed");
    anyhow::ensure!(
        current_paint(&document.doc, id, is_stroke, index) == Some(expected_fill),
        "the paint changed while choosing a video"
    );
    let crate::generation_media::PreparedVideo {
        bytes,
        asset,
        poster,
        ..
    } = prepared;
    if let Some(existing) = document.raw_assets.get(&asset) {
        anyhow::ensure!(
            existing.as_slice() == bytes.as_ref(),
            "video asset hash collision"
        );
    }
    let poster_asset = match poster {
        Some(poster) => Some(
            document
                .doc_and_assets()
                .1
                .add_image_tracked(poster.png.to_vec())?,
        ),
        None => None,
    };
    let mut new_fill = match expected_fill {
        Fill::Video { .. } => expected_fill.clone(),
        other => Fill::Video {
            video: Box::new(fanta_doc::VideoFill {
                asset,
                poster: None,
                mode: ImageFitMode::Fill,
                crop: None,
                scale: None,
                rotation: None,
                adjust: fanta_doc::ImageAdjust::default(),
            }),
            opacity: paint_opacity_fraction(other),
            blend: crate::properties_ops::paint_blend(other),
        },
    };
    if let Fill::Video { video, .. } = &mut new_fill {
        video.asset = asset;
        video.poster = poster_asset.map(|(id, _, _)| id);
    }
    let operations = replace_data_operation(&document.doc, id, |data| {
        if let Some(paint) = crate::properties_ops::paint_slot_mut(data, index, is_stroke) {
            *paint = new_fill.clone();
        }
    });
    let mut changed = false;
    for operation in operations {
        if let Err(error) = document.doc.apply(operation) {
            if let Some((poster, _, true)) = poster_asset {
                document.doc_and_assets().1.remove(poster);
            }
            return Err(error.into());
        }
        changed = true;
    }
    if changed {
        Arc::make_mut(&mut document.raw_assets).insert(asset, bytes.to_vec());
    } else if let Some((poster, _, true)) = poster_asset {
        document.doc_and_assets().1.remove(poster);
    }
    Ok(changed)
}

fn media_payload_for(
    doc: &Doc,
    id: NodeId,
    is_stroke: bool,
    index: usize,
) -> Option<DesignPaintPayload> {
    let fill = current_paint(doc, id, is_stroke, index)?;
    let snapshot = crate::properties_snapshot::paint_snapshot(fill, None);
    let paint = design_paint(
        id,
        if is_stroke { "stroke" } else { "fill" },
        index,
        &snapshot,
        Some(fill),
    );
    (!paint.read_only
        && matches!(
            paint.payload,
            DesignPaintPayload::Image(_) | DesignPaintPayload::Video(_)
        ))
    .then_some(paint.payload)
}

fn media_placement(payload: &DesignPaintPayload) -> DesignMediaPaintPlacement {
    match payload {
        DesignPaintPayload::Image(image) => image.placement,
        DesignPaintPayload::Video(video) => video.placement,
        _ => DesignMediaPaintPlacement::default(),
    }
}

fn set_media_placement(payload: &mut DesignPaintPayload, placement: DesignMediaPaintPlacement) {
    match payload {
        DesignPaintPayload::Image(image) => image.placement = placement,
        DesignPaintPayload::Video(video) => video.placement = placement,
        _ => {}
    }
}

fn crop_transform_for_state(
    state: DesignMediaCropToolState,
    source_aspect: f32,
) -> Option<DesignPaintTransform> {
    let mut transform = state.transform;
    if !state.zoom.is_finite()
        || state.zoom <= 0.0
        || !source_aspect.is_finite()
        || source_aspect <= 0.0
        || !transform.m11.is_finite()
        || !transform.m22.is_finite()
        || !transform.tx.is_finite()
        || !transform.ty.is_finite()
        || transform.m12.abs() > 1e-6
        || transform.m21.abs() > 1e-6
        || transform.m11 <= 0.0
        || transform.m22 <= 0.0
    {
        return None;
    }
    let width = transform.m11 / state.zoom;
    let mut height = transform.m22 / state.zoom;
    let ratio = match state.aspect_ratio {
        fanta_gpui::design::DesignMediaCropAspectRatio::Free => None,
        fanta_gpui::design::DesignMediaCropAspectRatio::Original => Some(source_aspect),
        fanta_gpui::design::DesignMediaCropAspectRatio::Square => Some(1.0),
        fanta_gpui::design::DesignMediaCropAspectRatio::FourByThree => Some(4.0 / 3.0),
        fanta_gpui::design::DesignMediaCropAspectRatio::SixteenByNine => Some(16.0 / 9.0),
    };
    if let Some(ratio) = ratio {
        height = width * source_aspect / ratio;
    }
    if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
        return None;
    }
    let center_x = transform.tx + transform.m11 * 0.5;
    let center_y = transform.ty + transform.m22 * 0.5;
    transform.m11 = width.min(1.0);
    transform.m22 = height.min(1.0);
    transform.tx = (center_x - transform.m11 * 0.5).clamp(0.0, 1.0 - transform.m11);
    transform.ty = (center_y - transform.m22 * 0.5).clamp(0.0, 1.0 - transform.m22);
    Some(transform)
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
                return text_paint_color_operations(doc, id, *color);
            }
            solid_paint_color_operations(doc, id, is_stroke, index, *color)
        }
        PaintEditValue::Opacity(percent) => {
            if is_text && !is_stroke {
                let alpha = (percent.clamp(0.0, 100.0) / 100.0 * 255.0).round() as u8;
                replace_data_operation(doc, id, |data| {
                    if let NodeData::Text(text) = data {
                        text.style.color.a = alpha;
                        for run in &mut text.style_runs {
                            run.style.color.a = alpha;
                        }
                    }
                })
            } else if paint_gradient(doc, id, is_stroke, index).is_none() {
                field_operations(
                    doc,
                    &InspectorField::PaintOpacity {
                        id,
                        index,
                        is_stroke,
                    },
                    &format_number(*percent),
                )
            } else {
                gradient_paint_operations(doc, id, is_stroke, index, |gradient| {
                    let stops = crate::color_picker::gradient_stops_mut(gradient);
                    let old_max = stops.iter().map(|stop| stop.color.a).max().unwrap_or(0);
                    let target_max = (percent.clamp(0.0, 100.0) / 100.0 * 255.0).round();
                    for stop in stops {
                        stop.color.a = if old_max == 0 {
                            target_max as u8
                        } else {
                            (f64::from(stop.color.a) / f64::from(old_max) * target_max)
                                .round()
                                .clamp(0.0, 255.0) as u8
                        };
                    }
                    Some(gradient.clone())
                })
            }
        }
        PaintEditValue::BlendMode(blend) => {
            if is_text && !is_stroke {
                blend_mode_operations(doc, id, design_blend_mode(*blend))
            } else {
                replace_data_operation(doc, id, |data| {
                    if let Some(paint) =
                        crate::properties_ops::paint_slot_mut(data, index, is_stroke)
                    {
                        match paint {
                            Fill::Solid { blend: current, .. }
                            | Fill::Gradient { blend: current, .. }
                            | Fill::Image { blend: current, .. }
                            | Fill::Pattern { blend: current, .. }
                            | Fill::Video { blend: current, .. }
                            | Fill::Shader { blend: current, .. } => *current = *blend,
                        }
                    }
                })
            }
        }
        PaintEditValue::Payload(payload) => match payload {
            DesignPaintPayload::Solid(solid) => {
                let color = fanta_color(solid.color);
                if is_text && !is_stroke {
                    text_paint_color_operations(doc, id, color)
                } else {
                    solid_paint_color_operations(doc, id, is_stroke, index, color)
                }
            }
            DesignPaintPayload::Gradient(gradient) if !is_text => {
                let Some(engine_gradient) =
                    gradient_from_design(gradient.kind, &gradient.stops, gradient.transform)
                else {
                    return Vec::new();
                };
                replace_data_operation(doc, id, |data| {
                    crate::properties_ops::set_paint_gradient(
                        data,
                        index,
                        is_stroke,
                        engine_gradient.clone(),
                    );
                })
            }
            DesignPaintPayload::Pattern(pattern) if !is_text => {
                let Some(pattern) = pattern_fill_from_design(doc, id, pattern) else {
                    return Vec::new();
                };
                replace_data_operation(doc, id, |data| {
                    if let Some(paint) =
                        crate::properties_ops::paint_slot_mut(data, index, is_stroke)
                    {
                        let blend = crate::properties_ops::paint_blend(paint);
                        let opacity = match paint {
                            Fill::Pattern { opacity, .. }
                            | Fill::Image { opacity, .. }
                            | Fill::Video { opacity, .. }
                            | Fill::Shader { opacity, .. } => *opacity,
                            _ => 1.0,
                        };
                        *paint = Fill::Pattern {
                            pattern: Box::new(pattern.clone()),
                            opacity,
                            blend,
                        };
                    }
                })
            }
            DesignPaintPayload::Image(image) if !is_text => {
                replace_data_operation(doc, id, |data| {
                    if let Some(paint) =
                        crate::properties_ops::paint_slot_mut(data, index, is_stroke)
                    {
                        let blend = crate::properties_ops::paint_blend(paint);
                        let opacity = match paint {
                            Fill::Image { opacity, .. }
                            | Fill::Pattern { opacity, .. }
                            | Fill::Video { opacity, .. }
                            | Fill::Shader { opacity, .. } => *opacity,
                            _ => 1.0,
                        };
                        if let Some(image_fill) = image_fill_from_design(image, opacity, blend) {
                            *paint = image_fill;
                        }
                    }
                })
            }
            DesignPaintPayload::Video(video) if !is_text => {
                let previous = current_paint(doc, id, is_stroke, index).and_then(|paint| {
                    if let Fill::Video { video, .. } = paint {
                        Some(video.as_ref())
                    } else {
                        None
                    }
                });
                let Some(new_fill) = video_fill_from_design(
                    video,
                    previous,
                    current_paint(doc, id, is_stroke, index).map_or(1.0, paint_opacity_fraction),
                    current_paint(doc, id, is_stroke, index)
                        .map_or(BlendMode::Normal, crate::properties_ops::paint_blend),
                ) else {
                    return Vec::new();
                };
                replace_data_operation(doc, id, |data| {
                    if let Some(paint) =
                        crate::properties_ops::paint_slot_mut(data, index, is_stroke)
                    {
                        *paint = new_fill.clone();
                    }
                })
            }
            DesignPaintPayload::Shader(shader) if !is_text => {
                let previous = current_paint(doc, id, is_stroke, index).and_then(|paint| {
                    if let Fill::Shader { shader, .. } = paint {
                        Some(shader.as_ref())
                    } else {
                        None
                    }
                });
                let Some(new_fill) = shader_fill_from_design(
                    shader,
                    previous,
                    current_paint(doc, id, is_stroke, index).map_or(1.0, paint_opacity_fraction),
                    current_paint(doc, id, is_stroke, index)
                        .map_or(BlendMode::Normal, crate::properties_ops::paint_blend),
                ) else {
                    return Vec::new();
                };
                replace_data_operation(doc, id, |data| {
                    if let Some(paint) =
                        crate::properties_ops::paint_slot_mut(data, index, is_stroke)
                    {
                        *paint = new_fill.clone();
                    }
                })
            }
            _ => Vec::new(),
        },
        PaintEditValue::ModelEdit(edit) => {
            let Some(fill) = current_paint(doc, id, is_stroke, index) else {
                return Vec::new();
            };
            let snapshot = crate::properties_snapshot::paint_snapshot(fill, None);
            let mut paint = design_paint(
                id,
                if is_stroke { "stroke" } else { "fill" },
                index,
                &snapshot,
                Some(fill),
            );
            if !paint.apply_edit(edit) {
                return Vec::new();
            }
            paint_edit_operations(
                doc,
                id,
                is_stroke,
                index,
                &PaintEditValue::Payload(paint.payload),
            )
        }
        PaintEditValue::GradientKind(kind) => {
            gradient_paint_operations(doc, id, is_stroke, index, |gradient| {
                let design_transform = design_gradient_transform(gradient);
                gradient_from_design(*kind, &gradient_stops(gradient), design_transform)
            })
        }
        PaintEditValue::GradientTransform(transform) => {
            gradient_paint_operations(doc, id, is_stroke, index, |gradient| {
                let kind = design_gradient_kind(gradient);
                gradient_from_design(kind, &gradient_stops(gradient), *transform)
            })
        }
        PaintEditValue::GradientStopColor(stop_index, color) => {
            gradient_paint_operations(doc, id, is_stroke, index, |gradient| {
                crate::color_picker::gradient_stops_mut(gradient)
                    .get_mut(*stop_index)?
                    .color = *color;
                Some(gradient.clone())
            })
        }
        PaintEditValue::GradientStopPosition(stop_index, position) => {
            gradient_paint_operations(doc, id, is_stroke, index, |gradient| {
                let stop =
                    crate::color_picker::gradient_stops_mut(gradient).get_mut(*stop_index)?;
                stop.position = position.clamp(0.0, 1.0);
                crate::color_picker::gradient_stops_mut(gradient)
                    .sort_by(|left, right| left.position.total_cmp(&right.position));
                Some(gradient.clone())
            })
        }
        PaintEditValue::GradientStopAdd(position, color) => {
            gradient_paint_operations(doc, id, is_stroke, index, |gradient| {
                if !position.is_finite() {
                    return None;
                }
                let stops = crate::color_picker::gradient_stops_mut(gradient);
                stops.push(fanta_doc::GradientStop {
                    position: position.clamp(0.0, 1.0),
                    color: *color,
                });
                stops.sort_by(|left, right| left.position.total_cmp(&right.position));
                Some(gradient.clone())
            })
        }
        PaintEditValue::GradientStopRemove(stop_index) => {
            gradient_paint_operations(doc, id, is_stroke, index, |gradient| {
                let stops = crate::color_picker::gradient_stops_mut(gradient);
                if stops.len() <= 2 || *stop_index >= stops.len() {
                    return None;
                }
                stops.remove(*stop_index);
                Some(gradient.clone())
            })
        }
    }
}

fn design_gradient_kind(gradient: &Gradient) -> DesignPaintKind {
    match gradient {
        Gradient::Linear { .. } => DesignPaintKind::LinearGradient,
        Gradient::Radial { .. } => DesignPaintKind::RadialGradient,
        Gradient::Angular { .. } => DesignPaintKind::AngularGradient,
        Gradient::Diamond { .. } => DesignPaintKind::DiamondGradient,
    }
}

fn gradient_paint_operations(
    doc: &Doc,
    id: NodeId,
    is_stroke: bool,
    index: usize,
    edit: impl FnOnce(&mut Gradient) -> Option<Gradient>,
) -> Vec<Operation> {
    let Some(mut gradient) = paint_gradient(doc, id, is_stroke, index) else {
        return Vec::new();
    };
    let Some(edited) = edit(&mut gradient) else {
        return Vec::new();
    };
    replace_data_operation(doc, id, |data| {
        crate::properties_ops::set_paint_gradient(data, index, is_stroke, edited.clone());
    })
}

fn paint_gradient(doc: &Doc, id: NodeId, is_stroke: bool, index: usize) -> Option<Gradient> {
    let node = doc.scene.get(id)?;
    let mut data = node.data.clone();
    match crate::properties_ops::paint_slot_mut(&mut data, index, is_stroke)? {
        Fill::Gradient { gradient, .. } => Some(gradient.clone()),
        _ => None,
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

fn text_paint_color_operations(doc: &Doc, id: NodeId, color: FantaColor) -> Vec<Operation> {
    replace_data_operation(doc, id, |data| {
        if let NodeData::Text(text) = data {
            text.set_glyph_color(color);
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
                DesignPanelValue::Number(value) if value.is_finite() => Some(f64::from(*value)),
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
                if !radius.is_finite() {
                    return None;
                }
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
    Color,
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
    Some(match schema.kind {
        ComponentPropKind::Bool => EnginePropKind::Boolean,
        ComponentPropKind::Number => EnginePropKind::Number,
        ComponentPropKind::Text => EnginePropKind::Text,
        ComponentPropKind::Color => EnginePropKind::Color,
        ComponentPropKind::InstanceSwap | ComponentPropKind::Variant { .. } => {
            EnginePropKind::Other
        }
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
        (P::IsMask, V::Bool(masked)) => {
            let node = doc.scene.get(id)?;
            Some(
                (node.is_mask != *masked)
                    .then(|| Operation::SetMask {
                        id,
                        old: node.is_mask,
                        new: *masked,
                    })
                    .into_iter()
                    .collect(),
            )
        }
        (P::MaskType, V::MaskType(kind)) => {
            let node = doc.scene.get(id)?;
            let target = match kind {
                DesignMaskType::Luminance => MaskType::Luminance,
                DesignMaskType::Alpha => MaskType::Alpha,
                DesignMaskType::Vector => MaskType::Vector,
            };
            Some(
                (node.mask_type != target)
                    .then(|| Operation::SetMaskType {
                        id,
                        old: node.mask_type,
                        new: target,
                    })
                    .into_iter()
                    .collect(),
            )
        }
        (P::PolygonCount, _) => {
            let points = number(value)?.clamp(3.0, 60.0) as u32;
            Some(parametric_shape_operations(doc, id, move |shape| {
                if let ParametricShape::Polygon { points: current } = shape {
                    *current = points;
                }
            }))
        }
        (P::StarPointCount, _) => {
            let points = number(value)?.clamp(3.0, 60.0) as u32;
            Some(parametric_shape_operations(doc, id, move |shape| {
                if let ParametricShape::Star {
                    points: current, ..
                } = shape
                {
                    *current = points;
                }
            }))
        }
        (P::StarInnerRadius, V::Ratio(ratio)) => {
            let ratio = f64::from(*ratio).clamp(0.0, 1.0);
            Some(parametric_shape_operations(doc, id, move |shape| {
                if let ParametricShape::Star { inner_ratio, .. } = shape {
                    *inner_ratio = ratio;
                }
            }))
        }
        (P::ArcStartingAngle, V::AngleRadians(angle)) => {
            let angle = f64::from(*angle);
            Some(parametric_shape_operations(doc, id, move |shape| {
                if let ParametricShape::Arc { start_rad, .. } = shape {
                    *start_rad = angle;
                }
            }))
        }
        (P::ArcSweep, V::AngleRadians(angle)) => {
            let angle = f64::from(*angle);
            Some(parametric_shape_operations(doc, id, move |shape| {
                if let ParametricShape::Arc { sweep_rad, .. } = shape {
                    *sweep_rad = angle;
                }
            }))
        }
        (P::ArcEndingAngle, V::AngleRadians(angle)) => {
            let angle = f64::from(*angle);
            Some(parametric_shape_operations(doc, id, move |shape| {
                if let ParametricShape::Arc {
                    start_rad,
                    sweep_rad,
                    ..
                } = shape
                {
                    *sweep_rad = angle - *start_rad;
                }
            }))
        }
        (P::ArcInnerRadius, V::Ratio(ratio)) => {
            let ratio = f64::from(*ratio).clamp(0.0, 1.0);
            Some(parametric_shape_operations(doc, id, move |shape| {
                if let ParametricShape::Arc { inner_ratio, .. } = shape {
                    *inner_ratio = ratio;
                }
            }))
        }
        (P::BooleanOperation, V::BooleanOperation(operation)) => {
            let target = match operation {
                DesignBooleanOperation::Union => BooleanOp::Union,
                DesignBooleanOperation::Subtract => BooleanOp::Subtract,
                DesignBooleanOperation::Intersect => BooleanOp::Intersect,
                DesignBooleanOperation::Exclude => BooleanOp::Exclude,
            };
            Some(replace_data_operation(doc, id, |data| {
                if let NodeData::Boolean(boolean) = data {
                    boolean.op = target;
                }
            }))
        }
        (P::BlendMode, V::BlendMode(mode)) => {
            if !matches!(mode, DesignBlendMode::PassThrough) && engine_blend_mode(*mode).is_none() {
                return None;
            }
            Some(blend_mode_operations(doc, id, *mode))
        }
        (P::ClipContent, V::Bool(enabled)) => {
            Some(set_clip_content_meta_operation(doc, id, *enabled))
        }
        (P::StrokeWeight, _) => {
            let weight = number(value)?.max(0.0);
            Some(replace_data_operation(doc, id, |data| {
                if let Some(strokes) = stroke_list_mut(data) {
                    for stroke in strokes.iter_mut() {
                        if let Some(per_side) = stroke.per_side.as_mut() {
                            per_side[0] = weight;
                        } else {
                            stroke.width = weight;
                        }
                    }
                }
            }))
        }
        (P::StrokeWeightMode, V::StrokeWeightMode(mode)) => {
            let custom = *mode != DesignStrokeWeightMode::All;
            Some(replace_data_operation(doc, id, |data| {
                if let Some(strokes) = stroke_list_mut(data) {
                    for stroke in strokes.iter_mut() {
                        if custom {
                            stroke.per_side.get_or_insert([stroke.width; 4]);
                        } else if let Some(per_side) = stroke.per_side.take() {
                            stroke.width = per_side[0];
                        }
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
        | (P::StrokeEndpointCap, V::StrokeCap(cap))
        | (P::StrokeDashCap, V::StrokeCap(cap)) => {
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
        (P::StrokeMiterAngle, _) => {
            let angle = number(value)?.clamp(1.0, 180.0).to_radians();
            let miter_limit = (1.0 / (angle * 0.5).sin()).clamp(1.0, 1000.0);
            Some(replace_data_operation(doc, id, |data| {
                if let Some(strokes) = stroke_list_mut(data) {
                    for stroke in strokes.iter_mut() {
                        stroke.miter_limit = miter_limit;
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
        (P::LayoutMode, V::LayoutMode(mode)) => layout_mode_operations(doc, id, *mode),
        (P::AutoLayoutAlignment, V::AutoLayoutAlignment(alignment)) => {
            let (x, y) = (alignment.x, alignment.y);
            Some(auto_layout_operations(doc, id, move |layout| {
                set_auto_layout_alignment(layout, x, y);
            }))
        }
        (P::AlignmentX, _) | (P::AlignmentY, _) => {
            let cell = number(value)?.clamp(0.0, 2.0) as u8;
            let node = doc.scene.get(id)?;
            let NodeData::Group(group) = &node.data else {
                return None;
            };
            let layout = group.auto_layout?;
            let (mut x, mut y) = auto_layout_alignment(layout);
            if property == P::AlignmentX {
                x = cell;
            } else {
                y = cell;
            }
            Some(auto_layout_operations(doc, id, move |layout| {
                set_auto_layout_alignment(layout, x, y);
            }))
        }
        (P::ItemSpacingMode, V::ItemSpacingMode(mode)) => {
            let auto = *mode == DesignItemSpacingMode::Auto;
            Some(auto_layout_operations(doc, id, move |layout| {
                if auto {
                    layout.primary_align = PrimaryAlign::SpaceBetween;
                } else if matches!(
                    layout.primary_align,
                    PrimaryAlign::SpaceBetween | PrimaryAlign::SpaceEvenly
                ) {
                    layout.primary_align = PrimaryAlign::Start;
                }
            }))
        }
        (P::CounterAxisAlignContent, V::CounterAxisAlignContent(alignment)) => {
            let space_between = *alignment == DesignCounterAxisAlignContent::SpaceBetween;
            Some(auto_layout_operations(doc, id, move |layout| {
                layout.counter_auto_spacing = space_between;
            }))
        }
        (P::StackingOrder, V::StackingOrder(order)) => {
            let first_on_top = *order == DesignStackingOrder::FirstOnTop;
            Some(auto_layout_operations(doc, id, move |layout| {
                layout.reverse_z = first_on_top;
            }))
        }
        (P::IncludeStrokes, V::Bool(include)) => {
            let include = *include;
            Some(auto_layout_operations(doc, id, move |layout| {
                layout.include_strokes = include;
            }))
        }
        (P::BaselineAlignment, V::BaselineAlignment(alignment)) => {
            let baseline = *alignment == DesignBaselineAlignment::Baseline;
            Some(auto_layout_operations(doc, id, move |layout| {
                layout.counter_align = if baseline {
                    CounterAlign::Baseline
                } else {
                    CounterAlign::Start
                };
            }))
        }
        (P::LayoutPositioning, V::LayoutPositioning(positioning)) => {
            let absolute = *positioning == DesignLayoutPositioning::Absolute;
            Some(layout_child_operations(doc, id, move |child| {
                child.absolute = absolute;
            }))
        }
        (P::LayoutAlignSelf, V::LayoutAlignSelf(alignment)) => {
            let stretch = *alignment == DesignLayoutAlignSelf::Stretch;
            Some(layout_child_operations(doc, id, move |child| {
                child.align_self = stretch.then_some(CounterAlign::Stretch);
            }))
        }
        (P::LayoutGrow, _) => {
            let grow = number(value)?.clamp(0.0, 1.0) as f32;
            Some(layout_child_operations(doc, id, move |child| {
                child.grow = grow;
            }))
        }
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
        (P::PaddingShorthand, V::NumberList(values)) if (1..=4).contains(&values.len()) => {
            let values: Vec<f64> = values
                .iter()
                .map(|value| f64::from(*value).max(0.0))
                .collect();
            let padding = match values.as_slice() {
                [all] => [*all; 4],
                [vertical, horizontal] => [*vertical, *horizontal, *vertical, *horizontal],
                [top, horizontal, bottom] => [*top, *horizontal, *bottom, *horizontal],
                [top, right, bottom, left] => [*top, *right, *bottom, *left],
                _ => return None,
            };
            Some(auto_layout_operations(doc, id, move |layout| {
                layout.padding = padding;
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
            let horizontal = property == P::HorizontalSizing;
            let node = doc.scene.get(id)?;
            let parent_layout = node
                .parent
                .and_then(|parent| doc.scene.get(parent))
                .and_then(|parent| match &parent.data {
                    NodeData::Group(group) => group.auto_layout,
                    _ => None,
                });
            let mut operations = Vec::new();
            if let Some(parent_layout) = parent_layout {
                let primary = horizontal == (parent_layout.mode == LayoutMode::Horizontal);
                match mode {
                    DesignSizingMode::Fill => {
                        return Some(layout_child_operations(doc, id, move |child| {
                            if primary {
                                child.grow = 1.0;
                            } else {
                                child.align_self = Some(CounterAlign::Stretch);
                            }
                        }));
                    }
                    DesignSizingMode::Fixed | DesignSizingMode::Hug => {
                        operations.extend(layout_child_operations(doc, id, move |child| {
                            if primary {
                                child.grow = 0.0;
                            } else if child.align_self == Some(CounterAlign::Stretch) {
                                child.align_self = None;
                            }
                        }));
                    }
                }
            } else if *mode == DesignSizingMode::Fill {
                return None;
            }
            if matches!(node.data, NodeData::Group(_)) {
                let sizing = match mode {
                    DesignSizingMode::Fixed | DesignSizingMode::Fill => {
                        fanta_doc::AxisSizing::Fixed
                    }
                    DesignSizingMode::Hug => fanta_doc::AxisSizing::Hug,
                };
                operations.extend(replace_data_operation(doc, id, |data| {
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
                }));
            } else if matches!(node.data, NodeData::Text(_)) {
                operations.extend(replace_data_operation(doc, id, |data| {
                    if let NodeData::Text(text) = data {
                        text.auto_resize = match (horizontal, mode) {
                            (true, DesignSizingMode::Hug) => TextAutoResize::WidthAndHeight,
                            (false, DesignSizingMode::Hug)
                                if text.auto_resize == TextAutoResize::WidthAndHeight =>
                            {
                                TextAutoResize::WidthAndHeight
                            }
                            (false, DesignSizingMode::Hug) => TextAutoResize::Height,
                            (true, DesignSizingMode::Fixed)
                                if text.auto_resize == TextAutoResize::WidthAndHeight =>
                            {
                                TextAutoResize::Height
                            }
                            (_, DesignSizingMode::Fixed) => TextAutoResize::None,
                            (_, DesignSizingMode::Fill) => text.auto_resize,
                        };
                    }
                }));
            }
            Some(operations)
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
        (P::FontFamily, V::Text(family)) if !family.is_empty() => {
            Some(replace_data_operation(doc, id, |data| {
                if let NodeData::Text(text) = data {
                    text.style.font_family = family.to_string();
                    for run in &mut text.style_runs {
                        run.style.font_family = family.to_string();
                    }
                }
            }))
        }
        (P::FontSize, _) => {
            let size = number(value)?;
            if !size.is_finite() || size <= 0.0 {
                return None;
            }
            Some(replace_data_operation(doc, id, |data| {
                if let NodeData::Text(text) = data {
                    text.style.size_px = size;
                    for run in &mut text.style_runs {
                        run.style.size_px = size;
                    }
                }
            }))
        }
        (P::FontWeight, _) => {
            let weight = number(value)?.clamp(1.0, 1000.0) as u16;
            Some(replace_data_operation(doc, id, |data| {
                if let NodeData::Text(text) = data {
                    text.style.weight = weight;
                    for run in &mut text.style_runs {
                        run.style.weight = weight;
                    }
                }
            }))
        }
        (P::LineHeight, V::LineHeight(DesignLineHeight::Percent(percent))) => {
            let line_height = f64::from(*percent) / 100.0;
            if !line_height.is_finite() || line_height <= 0.0 {
                return None;
            }
            Some(replace_data_operation(doc, id, |data| {
                if let NodeData::Text(text) = data {
                    text.style.line_height = line_height;
                    text.style.line_height_auto_percent = None;
                    for run in &mut text.style_runs {
                        run.style.line_height = line_height;
                        run.style.line_height_auto_percent = None;
                    }
                }
            }))
        }
        (P::LineHeight, _) => None,
        (P::LetterSpacing, V::LetterSpacing(DesignLetterSpacing::Pixels(pixels))) => {
            let spacing = f64::from(*pixels);
            if !spacing.is_finite() {
                return None;
            }
            Some(replace_data_operation(doc, id, |data| {
                if let NodeData::Text(text) = data {
                    text.style.letter_spacing = spacing;
                    for run in &mut text.style_runs {
                        run.style.letter_spacing = spacing;
                    }
                }
            }))
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
        (P::TextDecoration, V::TextDecoration(decoration)) => {
            let underline = *decoration == DesignTextDecoration::Underline;
            let strikethrough = *decoration == DesignTextDecoration::Strikethrough;
            Some(replace_data_operation(doc, id, |data| {
                if let NodeData::Text(text) = data {
                    text.style.underline = underline;
                    text.style.strikethrough = strikethrough;
                    for run in &mut text.style_runs {
                        run.style.underline = underline;
                        run.style.strikethrough = strikethrough;
                    }
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

fn parametric_shape_operations(
    doc: &Doc,
    id: NodeId,
    change: impl FnOnce(&mut ParametricShape),
) -> Vec<Operation> {
    replace_data_operation(doc, id, |data| {
        let NodeData::Vector(vector) = data else {
            return;
        };
        let Some(shape) = vector.parametric.as_mut() else {
            return;
        };
        let size = vector.local_size.or_else(|| {
            vector
                .path
                .rough_bounds()
                .map(|bounds| [bounds.width(), bounds.height()])
        });
        let Some([width, height]) = size else {
            return;
        };
        change(shape);
        vector.path = shape.to_path(width, height);
    })
}

fn auto_layout_alignment(layout: fanta_doc::AutoLayout) -> (u8, u8) {
    let primary = match layout.primary_align {
        PrimaryAlign::Start | PrimaryAlign::SpaceBetween | PrimaryAlign::SpaceEvenly => 0,
        PrimaryAlign::Center => 1,
        PrimaryAlign::End => 2,
    };
    let counter = match layout.counter_align {
        CounterAlign::Start | CounterAlign::Stretch | CounterAlign::Baseline => 0,
        CounterAlign::Center => 1,
        CounterAlign::End => 2,
    };
    match layout.mode {
        LayoutMode::Horizontal => (primary, counter),
        LayoutMode::Vertical => (counter, primary),
    }
}

fn set_auto_layout_alignment(layout: &mut fanta_doc::AutoLayout, x: u8, y: u8) {
    let (primary, counter) = if layout.mode == LayoutMode::Horizontal {
        (x, y)
    } else {
        (y, x)
    };
    if !matches!(
        layout.primary_align,
        PrimaryAlign::SpaceBetween | PrimaryAlign::SpaceEvenly
    ) {
        layout.primary_align = match primary {
            0 => PrimaryAlign::Start,
            1 => PrimaryAlign::Center,
            _ => PrimaryAlign::End,
        };
    }
    layout.counter_align = match counter {
        0 => CounterAlign::Start,
        1 => CounterAlign::Center,
        _ => CounterAlign::End,
    };
}

fn auto_layout_operations(
    doc: &Doc,
    id: NodeId,
    change: impl FnOnce(&mut fanta_doc::AutoLayout),
) -> Vec<Operation> {
    replace_data_operation(doc, id, |data| {
        if let NodeData::Group(group) = data
            && let Some(layout) = group.auto_layout.as_mut()
        {
            change(layout);
        }
    })
}

fn layout_child_operations(
    doc: &Doc,
    id: NodeId,
    change: impl FnOnce(&mut LayoutChild),
) -> Vec<Operation> {
    let Some(node) = doc.scene.get(id) else {
        return Vec::new();
    };
    let Some(parent_layout) = node
        .parent
        .and_then(|parent| doc.scene.get(parent))
        .and_then(|parent| match &parent.data {
            NodeData::Group(group) => group.auto_layout,
            _ => None,
        })
    else {
        return Vec::new();
    };
    if !parent_layout.child_layout {
        return Vec::new();
    }
    let old = node.layout_child;
    let mut child = old.unwrap_or(LayoutChild {
        grow: 0.0,
        absolute: false,
        align_self: None,
    });
    change(&mut child);
    let new = (!child.is_trivial()).then_some(child);
    if new == old {
        Vec::new()
    } else {
        vec![Operation::SetLayoutChild { id, old, new }]
    }
}

fn layout_mode_operations(doc: &Doc, id: NodeId, mode: DesignLayoutMode) -> Option<Vec<Operation>> {
    let target = match mode {
        DesignLayoutMode::None => None,
        DesignLayoutMode::Horizontal => Some(LayoutMode::Horizontal),
        DesignLayoutMode::Vertical => Some(LayoutMode::Vertical),
        DesignLayoutMode::Grid => {
            log::debug!("fig design adapter: grid auto layout is not supported by the engine");
            return None;
        }
    };
    let node = doc.scene.get(id)?;
    if !matches!(node.data, NodeData::Group(_)) {
        return None;
    }
    let size = doc
        .scene
        .local_bounds(id)
        .map(|bounds| [bounds.width(), bounds.height()]);
    Some(replace_data_operation(doc, id, |data| {
        let NodeData::Group(group) = data else {
            return;
        };
        match target {
            None => group.auto_layout = None,
            Some(mode) => match group.auto_layout.as_mut() {
                Some(layout) => layout.mode = mode,
                None => {
                    if group.local_size.is_none() && group.clip_size.is_none() {
                        group.local_size = size;
                    }
                    group.auto_layout = Some(fanta_doc::AutoLayout {
                        mode,
                        ..fanta_doc::AutoLayout::default()
                    });
                }
            },
        }
    }))
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
    use std::cell::RefCell;
    use std::collections::BTreeMap;
    use std::path::PathBuf;
    use std::rc::Rc;

    use fanta_doc::{
        BooleanNode, CanvasNode, ComponentDef, ComponentPropDef, ComponentPropId, Doc, GroupNode,
        InstanceNode, VectorNode,
    };
    use fanta_gpui::design::{
        DesignFontCatalogState, DesignFontSelection, DesignPageBackground, DesignPageViewData,
        DesignPaintEdit, DesignPaintTarget, DesignPanelInspectionContext, DesignPanelPermissions,
        DesignPanelSurface, DesignTypographyTarget,
    };
    use gpui::{Entity, Modifiers, TestAppContext, VisualTestContext, point, px, size};
    use image::GenericImageView as _;
    use project::{FakeFs, Project};

    use super::*;
    use crate::properties_snapshot::master_roots;
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

    /// One page holding three 20×20 squares on a row at x = 10, 50 and 120.
    /// Left-aligning them must move two; distributing them must move only the
    /// middle one.
    fn doc_with_three_squares() -> (Doc, NodeId, [NodeId; 3]) {
        let mut doc = Doc::new();
        let mut page = CanvasNode::new(NodeData::Group(GroupNode::default()));
        page.name = "Page 1".to_owned();
        let page_id = page.id;
        doc.scene.insert(page).expect("insert page");
        doc.add_page(page_id);
        doc.set_active_page(Some(page_id));
        let mut ids = Vec::with_capacity(3);
        for (index, x) in [10.0_f64, 50.0, 120.0].into_iter().enumerate() {
            let mut square = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
                0.0,
                0.0,
                20.0,
                20.0,
                FantaColor::rgb(0xe0, 0x30, 0x30),
            )));
            square.name = format!("Square {index}");
            square.transform = Transform2D::translation(x, 0.0);
            square.parent = Some(page_id);
            ids.push(square.id);
            doc.scene.insert(square).expect("insert square");
        }
        let ids = [ids[0], ids[1], ids[2]];
        (doc, page_id, ids)
    }

    #[test]
    fn pattern_picker_settings_write_and_echo_into_the_scene() {
        let (mut doc, page, [target, source, replacement_source]) = doc_with_three_squares();
        let candidates = pattern_source_candidates(&doc, target);
        assert!(candidates.iter().any(|(id, _)| *id == source));
        assert!(candidates.iter().any(|(id, _)| *id == replacement_source));
        assert!(
            candidates
                .iter()
                .all(|(id, _)| *id != target && *id != page)
        );
        let mut paint = DesignPaint::pattern(source.to_string());
        let DesignPaintPayload::Pattern(pattern) = &mut paint.payload else {
            panic!("pattern constructor must create a pattern payload");
        };
        pattern.tile_type = DesignPatternTileType::HorizontalHexagonal;
        pattern.scaling_factor = 0.75;
        pattern.spacing = fanta_gpui::design::DesignPatternSpacing::new(0.2, 0.35);
        pattern.horizontal_alignment = DesignPatternHorizontalAlignment::End;
        let operations = paint_edit_operations(
            &doc,
            target,
            false,
            0,
            &PaintEditValue::Payload(paint.payload.clone()),
        );
        assert_eq!(operations.len(), 1);
        doc.apply(operations.into_iter().next().expect("one paint operation"))
            .expect("apply pattern");
        let fill = current_paint(&doc, target, false, 0).expect("target fill");
        let snapshot = crate::properties_snapshot::paint_snapshot(fill, None);
        let echoed = design_paint(target, "fill", 0, &snapshot, Some(fill));
        assert_eq!(echoed.payload, paint.payload);
        let operations = paint_edit_operations(
            &doc,
            target,
            false,
            0,
            &PaintEditValue::ModelEdit(DesignPaintEdit {
                property: DesignPaintProperty::PatternSourceNode,
                value: DesignPaintValue::PatternSourceNode(replacement_source.to_string().into()),
            }),
        );
        assert_eq!(operations.len(), 1);
        doc.apply(operations.into_iter().next().expect("one source operation"))
            .expect("replace source");
        assert!(matches!(
            current_paint(&doc, target, false, 0),
            Some(Fill::Pattern { pattern, .. }) if pattern.source_node_id == replacement_source
        ));
        assert!(
            paint_edit_operations(
                &doc,
                target,
                false,
                0,
                &PaintEditValue::ModelEdit(DesignPaintEdit {
                    property: DesignPaintProperty::PatternSourceNode,
                    value: DesignPaintValue::PatternSourceNode(target.to_string().into()),
                }),
            )
            .is_empty(),
            "a pattern cannot source itself"
        );
    }

    #[test]
    fn image_upload_creates_an_asset_and_media_controls_edit_the_fill() {
        let (mut doc, _page, target) = doc_with_rect();
        let before = current_paint(&doc, target, false, 0)
            .expect("rectangle fill")
            .clone();
        let mut bytes = Vec::new();
        image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            2,
            2,
            image::Rgba([10, 20, 30, 255]),
        ))
        .write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Png,
        )
        .expect("encode image");
        let prepared = PreparedImage::new(bytes).expect("prepare image");
        let mut stores = crate::document::TestAssetStores::default();
        assert!(
            replace_image_paint_with_asset(
                &mut doc,
                &mut stores.stores(),
                target,
                false,
                0,
                &before,
                prepared,
            )
            .expect("apply image fill")
        );
        let fill = current_paint(&doc, target, false, 0).expect("image fill");
        let Fill::Image { asset, .. } = fill else {
            panic!("upload should create an image fill");
        };
        assert!(stores.raw_assets().contains_key(asset));
        let filter_edit = DesignPaintEdit {
            property: DesignPaintProperty::MediaFilter(
                fanta_gpui::design::DesignImageFilter::Contrast,
            ),
            value: DesignPaintValue::Number(0.4),
        };
        let operations = paint_edit_operations(
            &doc,
            target,
            false,
            0,
            &PaintEditValue::ModelEdit(filter_edit),
        );
        assert_eq!(operations.len(), 1);
        doc.apply(operations.into_iter().next().expect("one filter operation"))
            .expect("apply image filter");
        let fill = current_paint(&doc, target, false, 0).expect("image fill");
        let Fill::Image { adjust, .. } = fill else {
            panic!("filter edit must retain image fill");
        };
        assert!((adjust.contrast - 0.4).abs() < 1e-5);
    }

    #[test]
    fn video_upload_keeps_source_and_poster_and_edits_filters() {
        let (doc, _page, target) = doc_with_rect();
        let mut document = FigDocument::from_doc(doc, BTreeMap::new());
        let before = current_paint(&document.doc, target, false, 0)
            .expect("rectangle fill")
            .clone();
        let mut poster = Vec::new();
        image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            2,
            2,
            image::Rgba([10, 20, 30, 255]),
        ))
        .write_to(
            &mut std::io::Cursor::new(&mut poster),
            image::ImageFormat::Png,
        )
        .expect("encode poster");
        let bytes = Arc::<[u8]>::from(&b"mp4 fixture"[..]);
        let asset = fanta_format::asset_id_for_bytes(&bytes);
        let prepared = crate::generation_media::PreparedVideo {
            bytes: bytes.clone(),
            asset,
            metadata: crate::generation_media::VideoMetadata {
                width: 2,
                height: 2,
                duration_us: 1_000_000,
            },
            poster: Some(crate::generation_media::VideoPoster {
                png: Arc::from(poster),
                time_us: 0,
            }),
        };
        let document_id = document.doc.id;
        assert!(
            replace_video_paint_with_asset(
                &mut document,
                target,
                false,
                0,
                document_id,
                &before,
                prepared,
            )
            .expect("apply video fill")
        );
        assert_eq!(document.raw_assets.get(&asset), Some(&bytes.to_vec()));
        let fill = current_paint(&document.doc, target, false, 0).expect("video fill");
        let Fill::Video { video, .. } = fill else {
            panic!("upload should create a video fill")
        };
        assert_eq!(video.asset, asset);
        assert!(
            video
                .poster
                .is_some_and(|poster| document.raw_assets.contains_key(&poster))
        );
        let snapshot = crate::properties_snapshot::paint_snapshot(fill, None);
        let echoed = design_paint(target, "fill", 0, &snapshot, Some(fill));
        assert!(matches!(echoed.payload, DesignPaintPayload::Video(_)));
        let (node, _) = design_node(&document, target, &master_roots(&document.doc.components))
            .expect("selected node projects");
        let context = DesignPanelInspectionContext::single(
            node,
            parent_layout_for(&document.doc, target),
            DesignPanelPermissions::editor(),
        );
        let preview = DesignVideoPreviewState::ready(4.0, 1.5, true);
        let media = media_paint_view_data(
            &context,
            None,
            Vec::new(),
            &HashMap::from([(asset.to_string(), preview.clone())]),
        );
        assert_eq!(
            media
                .paint(DesignPanelCollection::Fill, &echoed.id, 0)
                .and_then(|view| view.video_preview.as_ref()),
            Some(&preview)
        );
        let operations = paint_edit_operations(
            &document.doc,
            target,
            false,
            0,
            &PaintEditValue::ModelEdit(DesignPaintEdit {
                property: DesignPaintProperty::MediaFilter(
                    fanta_gpui::design::DesignImageFilter::Contrast,
                ),
                value: DesignPaintValue::Number(0.4),
            }),
        );
        assert_eq!(operations.len(), 1);
        document
            .doc
            .apply(
                operations
                    .into_iter()
                    .next()
                    .expect("one video filter operation"),
            )
            .expect("apply video filter");
        let Some(Fill::Video { video, .. }) = current_paint(&document.doc, target, false, 0) else {
            panic!("filter edit must retain video fill");
        };
        assert!((video.adjust.contrast - 0.4).abs() < 1e-5);
    }

    #[test]
    fn bundled_shader_selection_and_property_edit_roundtrip() {
        let (mut doc, _page, target) = doc_with_rect();
        let catalog = bundled_shader_catalog();
        assert_eq!(catalog.page_shaders.len(), 2);
        let definition = catalog
            .definition("fanta:shader:halftone")
            .expect("halftone shader");
        let payload = DesignShaderPaint::from_definition(definition).expect("imported shader");
        let operations = paint_edit_operations(
            &doc,
            target,
            false,
            0,
            &PaintEditValue::Payload(DesignPaintPayload::Shader(payload.clone())),
        );
        assert_eq!(operations.len(), 1);
        doc.apply(operations.into_iter().next().expect("one shader operation"))
            .expect("apply shader");
        let fill = current_paint(&doc, target, false, 0).expect("shader fill");
        let snapshot = crate::properties_snapshot::paint_snapshot(fill, None);
        let echoed = design_paint(target, "fill", 0, &snapshot, Some(fill));
        assert_eq!(echoed.payload, DesignPaintPayload::Shader(payload.clone()));
        let future_property = fanta_doc::ShaderPropertyAssignment {
            definition_id: "future_property".into(),
            value: fanta_doc::ShaderPropertyValue::Opaque {
                type_name: "future_type".into(),
                payload: "preserve this value".into(),
            },
        };
        for operation in replace_data_operation(&doc, target, |data| {
            if let NodeData::Vector(vector) = data
                && let Some(Fill::Shader { shader, .. }) = vector.fills.first_mut()
            {
                shader.properties.push(future_property.clone());
            }
        }) {
            doc.apply(operation).expect("add future shader property");
        }
        let mut edited_payload = payload;
        edited_payload
            .properties
            .iter_mut()
            .find(|property| property.definition_id == "radius")
            .expect("halftone radius property")
            .value = DesignShaderPropertyValue::Number(1.25);
        let operations = paint_edit_operations(
            &doc,
            target,
            false,
            0,
            &PaintEditValue::Payload(DesignPaintPayload::Shader(edited_payload)),
        );
        assert_eq!(operations.len(), 1);
        doc.apply(operations.into_iter().next().expect("one shader operation"))
            .expect("apply shader catalog defaults");
        assert!(matches!(
            current_paint(&doc, target, false, 0),
            Some(Fill::Shader { shader, .. }) if shader.properties.contains(&future_property)
        ));
        let operations = paint_edit_operations(
            &doc,
            target,
            false,
            0,
            &PaintEditValue::ModelEdit(DesignPaintEdit {
                property: DesignPaintProperty::ShaderProperty {
                    definition_id: "radius".into(),
                },
                value: DesignPaintValue::ShaderProperty(DesignShaderPropertyValue::Number(1.5)),
            }),
        );
        assert_eq!(operations.len(), 1);
        doc.apply(operations.into_iter().next().expect("one radius operation"))
            .expect("edit radius");
        let Some(Fill::Shader { shader, .. }) = current_paint(&doc, target, false, 0) else {
            panic!("property edit must retain shader fill");
        };
        assert!(
            shader
                .properties
                .iter()
                .any(|property| property.definition_id == "radius"
                    && property.value == fanta_doc::ShaderPropertyValue::Number(1.5))
        );
        assert!(shader.properties.contains(&future_property));
    }

    #[test]
    fn crop_zoom_and_ratio_produce_a_bounded_window() {
        let state = DesignMediaCropToolState {
            active: true,
            transform: DesignPaintTransform::IDENTITY,
            zoom: 2.0,
            aspect_ratio: fanta_gpui::design::DesignMediaCropAspectRatio::Square,
        };
        let crop = crop_transform_for_state(state, 2.0).expect("valid crop");
        assert!((crop.m11 - 0.5).abs() < 1e-6);
        assert!((crop.m22 - 1.0).abs() < 1e-6);
        assert!(crop.tx >= 0.0 && crop.ty >= 0.0);
        assert!(crop.tx + crop.m11 <= 1.0 && crop.ty + crop.m22 <= 1.0);
    }

    #[test]
    fn selection_header_only_exposes_commands_supported_by_the_current_selection() {
        use DesignSelectionHeaderControlKind as Kind;

        let (doc, page, ids) = doc_with_three_squares();
        let single = selection_header_for_doc(&doc, &ids[..1], Some(page), true)
            .expect("selected layer has a header");
        assert!(
            single
                .primary_controls
                .iter()
                .any(|control| control.kind == Kind::SelectMatchingLayers)
        );
        assert!(
            single
                .primary_controls
                .iter()
                .any(|control| control.kind == Kind::CreateComponent)
        );
        assert!(
            single
                .primary_controls
                .iter()
                .any(|control| control.kind == Kind::EditObject)
        );
        assert!(single.primary_controls.iter().all(|control| {
            control.kind != Kind::CreateLink && control.kind != Kind::ApplyTextContentVariable
        }));

        let multiple = selection_header_for_doc(&doc, &ids[..2], Some(page), true)
            .expect("multiple selection has a header");
        let boolean_menu = multiple
            .primary_controls
            .iter()
            .find(|control| control.kind == Kind::BooleanFlattenMenu)
            .expect("two vector layers support boolean operations");
        assert_eq!(
            boolean_menu
                .menu_items
                .iter()
                .filter(|item| matches!(item.command, DesignSelectionHeaderCommand::Boolean(_)))
                .count(),
            4
        );
        assert!(
            !multiple
                .primary_controls
                .iter()
                .any(|control| control.kind == Kind::UseAsMask)
        );

        let viewer = selection_header_for_doc(&doc, &ids[..1], Some(page), false)
            .expect("viewers can inspect the selection");
        assert_eq!(viewer.primary_controls.len(), 1);
        assert_eq!(viewer.primary_controls[0].kind, Kind::SelectMatchingLayers);
    }

    fn min_x(doc: &Doc, id: NodeId) -> f64 {
        doc.scene
            .world_bounds(id)
            .expect("the square has world bounds")
            .min_x
    }

    #[test]
    fn multi_selection_position_moves_the_selection_box_without_collapsing_spacing() {
        let (mut doc, _page, ids) = doc_with_three_squares();
        let operations = targeted_property_operations(
            &doc,
            &ids,
            DesignPanelProperty::X,
            &DesignPanelValue::Number(40.0),
        )
        .expect("selection X is supported");
        assert_eq!(operations.len(), 3);
        for operation in operations {
            doc.apply(operation).expect("move selection");
        }
        for (id, expected_x) in ids.into_iter().zip([40.0, 80.0, 150.0]) {
            assert!((min_x(&doc, id) - expected_x).abs() < 1e-9);
        }

        let operations = targeted_property_operations(
            &doc,
            &ids,
            DesignPanelProperty::Width,
            &DesignPanelValue::Number(260.0),
        )
        .expect("selection width is supported");
        for operation in operations {
            doc.apply(operation).expect("resize selection");
        }
        let first = doc.scene.world_bounds(ids[0]).expect("first bounds");
        let last = doc.scene.world_bounds(ids[2]).expect("last bounds");
        assert!((first.min_x - 40.0).abs() < 1e-9);
        assert!((last.max_x - 300.0).abs() < 1e-9);

        assert!(
            targeted_property_operations(
                &doc,
                &ids,
                DesignPanelProperty::LockAspectRatio,
                &DesignPanelValue::Bool(true),
            )
            .is_none(),
            "an unsupported aggregate leaf cannot partially edit members"
        );
    }

    #[test]
    fn arrange_maps_align_distribute_and_tidy_up() {
        let (mut doc, _page, ids) = doc_with_three_squares();

        // Align Left leaves the already-leftmost square alone: the engine
        // emits no operation for a zero delta.
        let ops = arrange_operations(&doc, &ids, DesignArrangeOperation::AlignLeft)
            .expect("align left is wired");
        assert_eq!(ops.len(), 2, "only the two trailing squares move");
        assert!(
            ops.iter()
                .all(|op| matches!(op, Operation::SetTransform { .. })),
            "aligning is expressed as transform writes"
        );
        for op in ops {
            doc.apply(op).expect("apply align");
        }
        for id in ids {
            assert!(
                (min_x(&doc, id) - 10.0).abs() < 1e-9,
                "every square aligned"
            );
        }

        // Distributing needs three nodes; two are already as far apart as
        // they can be, so the engine returns nothing.
        let (doc, _page, ids) = doc_with_three_squares();
        assert!(
            arrange_operations(
                &doc,
                &ids[..2],
                DesignArrangeOperation::DistributeHorizontal
            )
            .expect("distribute is wired")
            .is_empty(),
            "a two-node distribute is a no-op"
        );
        let ops = arrange_operations(&doc, &ids, DesignArrangeOperation::DistributeHorizontal)
            .expect("distribute is wired");
        assert_eq!(ops.len(), 1, "the outermost squares stay anchored");

        let mut tidy = doc.clone();
        let ops = arrange_operations(&tidy, &ids, DesignArrangeOperation::TidyUp)
            .expect("tidy up is wired");
        assert!(!ops.is_empty());
        for op in ops {
            tidy.apply(op).expect("apply tidy up");
        }
        assert!((min_x(&tidy, ids[1]) - 65.0).abs() < 1e-9);
    }

    #[test]
    fn auto_layout_controls_write_and_echo_container_and_child_values() {
        let (mut doc, _page, rect) = doc_with_rect();
        doc.selection.replace_with([rect]);
        let create =
            crate::layer_context_ops::auto_layout(&doc, rect, Some(LayoutMode::Horizontal))
                .expect("wrap the rectangle in auto layout");
        let group = crate::layer_context_ops::created_roots(&create)[0];
        for operation in create {
            doc.apply(operation).expect("create layout frame");
        }

        let mut change = |id, property, value| {
            let operations = property_operations(&doc, id, property, &value)
                .expect("the exposed layout control is backed");
            for operation in operations {
                doc.apply(operation).expect("apply layout control");
            }
        };
        change(
            group,
            DesignPanelProperty::LayoutMode,
            DesignPanelValue::LayoutMode(DesignLayoutMode::Vertical),
        );
        change(
            group,
            DesignPanelProperty::AutoLayoutAlignment,
            DesignPanelValue::AutoLayoutAlignment(
                fanta_gpui::design::DesignAutoLayoutAlignment::new(2, 1),
            ),
        );
        change(
            group,
            DesignPanelProperty::ItemSpacingMode,
            DesignPanelValue::ItemSpacingMode(DesignItemSpacingMode::Auto),
        );
        change(
            group,
            DesignPanelProperty::PaddingShorthand,
            DesignPanelValue::NumberList(vec![3.0, 5.0, 7.0, 11.0]),
        );
        change(
            group,
            DesignPanelProperty::StackingOrder,
            DesignPanelValue::StackingOrder(DesignStackingOrder::FirstOnTop),
        );
        change(
            group,
            DesignPanelProperty::IncludeStrokes,
            DesignPanelValue::Bool(true),
        );
        change(
            rect,
            DesignPanelProperty::LayoutPositioning,
            DesignPanelValue::LayoutPositioning(DesignLayoutPositioning::Absolute),
        );
        change(
            rect,
            DesignPanelProperty::LayoutAlignSelf,
            DesignPanelValue::LayoutAlignSelf(DesignLayoutAlignSelf::Stretch),
        );
        change(
            rect,
            DesignPanelProperty::HorizontalSizing,
            DesignPanelValue::SizingMode(DesignSizingMode::Fill),
        );

        let node = doc.scene.get(group).expect("layout frame exists");
        let NodeData::Group(frame) = &node.data else {
            panic!("the wrapper is a group");
        };
        let layout = frame.auto_layout.expect("layout is active");
        assert_eq!(layout.mode, LayoutMode::Vertical);
        assert_eq!(layout.primary_align, PrimaryAlign::SpaceBetween);
        assert_eq!(layout.counter_align, CounterAlign::End);
        assert_eq!(layout.padding, [3.0, 5.0, 7.0, 11.0]);
        assert!(layout.reverse_z);
        assert!(layout.include_strokes);
        let section = node_section(&doc, group, &master_roots(&doc.components))
            .expect("layout frame snapshot");
        let echoed = design_layout(&doc, node, &section).expect("layout controls are visible");
        assert_eq!(echoed.mode, DesignLayoutMode::Vertical);
        assert_eq!(echoed.item_spacing_mode, DesignItemSpacingMode::Auto);
        assert_eq!(echoed.padding, [3.0, 5.0, 7.0, 11.0]);
        assert_eq!(echoed.stacking_order, DesignStackingOrder::FirstOnTop);
        assert!(echoed.include_strokes);

        let child = doc.scene.get(rect).expect("rectangle still exists");
        let child_settings = child.layout_child.expect("child layout settings");
        assert!(child_settings.absolute);
        assert_eq!(child_settings.align_self, Some(CounterAlign::Stretch));
        assert_eq!(child_settings.grow, 0.0);
        let section =
            node_section(&doc, rect, &master_roots(&doc.components)).expect("child snapshot");
        let echoed = design_layout(&doc, child, &section).expect("child controls are visible");
        assert_eq!(echoed.item.positioning, DesignLayoutPositioning::Absolute);
        assert_eq!(echoed.item.align_self, DesignLayoutAlignSelf::Stretch);
        assert_eq!(echoed.horizontal_sizing, DesignSizingMode::Fill);
    }

    #[test]
    fn mask_type_and_shape_geometry_changes_echo_into_the_inspector() {
        let (mut doc, page, rect) = doc_with_rect();
        let mask_operations = property_operations(
            &doc,
            rect,
            DesignPanelProperty::IsMask,
            &DesignPanelValue::Bool(true),
        )
        .expect("mask toggle is backed");
        for operation in mask_operations {
            doc.apply(operation).expect("enable mask");
        }
        let mask_type_operations = property_operations(
            &doc,
            rect,
            DesignPanelProperty::MaskType,
            &DesignPanelValue::MaskType(DesignMaskType::Vector),
        )
        .expect("vector mask type is backed");
        for operation in mask_type_operations {
            doc.apply(operation).expect("set vector mask");
        }

        let node = doc.scene.get_mut(rect).expect("rectangle exists");
        let NodeData::Vector(vector) = &mut node.data else {
            panic!("rectangle is a vector");
        };
        let shape = ParametricShape::Star {
            points: 5,
            inner_ratio: 0.5,
        };
        vector.path = shape.to_path(200.0, 100.0);
        vector.local_size = Some([200.0, 100.0]);
        vector.parametric = Some(shape);
        let operations = property_operations(
            &doc,
            rect,
            DesignPanelProperty::StarPointCount,
            &DesignPanelValue::Number(8.0),
        )
        .expect("star point count is backed");
        for operation in operations {
            doc.apply(operation).expect("change star points");
        }

        let mut boolean = CanvasNode::new(NodeData::Boolean(BooleanNode::default()));
        boolean.parent = Some(page);
        let boolean_id = boolean.id;
        doc.scene.insert(boolean).expect("insert boolean");
        let operations = property_operations(
            &doc,
            boolean_id,
            DesignPanelProperty::BooleanOperation,
            &DesignPanelValue::BooleanOperation(DesignBooleanOperation::Intersect),
        )
        .expect("boolean operation is backed");
        for operation in operations {
            doc.apply(operation).expect("change boolean operation");
        }

        let document = FigDocument::from_doc(doc, BTreeMap::new());
        let masters = master_roots(&document.doc.components);
        let (star, _) = design_node(&document, rect, &masters).expect("star is projected");
        assert!(star.is_mask);
        assert_eq!(star.mask_mode, DesignMaskType::Vector);
        assert!(matches!(
            star.shape_geometry,
            DesignShapeGeometry::Star(DesignStarGeometry { point_count: 8, .. })
        ));
        let (boolean, _) =
            design_node(&document, boolean_id, &masters).expect("boolean is projected");
        assert!(matches!(
            boolean.shape_geometry,
            DesignShapeGeometry::Boolean(DesignBooleanOperation::Intersect)
        ));
    }

    #[gpui::test]
    async fn add_auto_layout_and_direction_are_undoable_from_design_actions(
        cx: &mut TestAppContext,
    ) {
        let (mut doc, page, rect) = doc_with_rect();
        doc.selection.replace_with([rect]);
        let (view, panel, mut cx) = setup_view(doc, cx).await;
        let cx = &mut cx;
        view.update_in(cx, |view, _, cx| view.refresh_gpui_design(cx));
        cx.run_until_parked();
        panel.read_with(cx, |panel, _| {
            assert!(panel.node().supports_add_auto_layout());
            assert!(panel.view_data().projections.add_auto_layout.is_some());
        });
        panel.update_in(cx, |_, _, cx| {
            cx.emit(DesignPanelAction::AddAutoLayoutRequested {
                target: DesignPanelTarget::Nodes {
                    node_ids: vec![rect.to_string().into()],
                },
            });
        });
        cx.run_until_parked();

        let item = view.read_with(cx, |view, _| view.item().clone());
        let group = item.read_with(cx, |item, _| {
            let doc = &item.document().expect("document ready").doc;
            let group = *doc.selection.iter().next().expect("wrapper selected");
            assert_ne!(group, rect);
            assert_eq!(
                doc.scene.get(group).expect("group exists").parent,
                Some(page)
            );
            assert_eq!(
                doc.scene.get(rect).expect("rect exists").parent,
                Some(group)
            );
            let NodeData::Group(frame) = &doc.scene.get(group).expect("group exists").data else {
                panic!("wrapper is a group");
            };
            assert_eq!(
                frame.auto_layout.expect("layout active").mode,
                LayoutMode::Horizontal
            );
            group
        });
        panel.update_in(cx, |_, _, cx| {
            cx.emit(DesignPanelAction::PropertyChangeRequested {
                node_id: group.to_string().into(),
                property: DesignPanelProperty::LayoutMode,
                value: DesignPanelValue::LayoutMode(DesignLayoutMode::Vertical),
            });
        });
        cx.run_until_parked();
        item.read_with(cx, |item, _| {
            let doc = &item.document().expect("document ready").doc;
            let NodeData::Group(frame) = &doc.scene.get(group).expect("group exists").data else {
                panic!("wrapper is a group");
            };
            assert_eq!(
                frame.auto_layout.expect("layout active").mode,
                LayoutMode::Vertical
            );
        });
        panel.read_with(cx, |panel, _| {
            assert_eq!(
                panel.node().layout.as_ref().expect("layout echo").mode,
                DesignLayoutMode::Vertical
            );
        });
        item.update(cx, |item, cx| item.undo(cx).expect("undo direction"));
        cx.run_until_parked();
        item.read_with(cx, |item, _| {
            let doc = &item.document().expect("document ready").doc;
            let NodeData::Group(frame) = &doc.scene.get(group).expect("group exists").data else {
                panic!("wrapper is a group");
            };
            assert_eq!(
                frame.auto_layout.expect("layout active").mode,
                LayoutMode::Horizontal
            );
        });
        item.update(cx, |item, cx| {
            item.undo(cx).expect("undo auto layout wrapper")
        });
        cx.run_until_parked();
        item.read_with(cx, |item, _| {
            let doc = &item.document().expect("document ready").doc;
            assert!(doc.scene.get(group).is_none());
            assert_eq!(doc.scene.get(rect).expect("rect exists").parent, Some(page));
        });
    }

    #[test]
    fn flipping_mirrors_the_selection_box_and_rotating_adds_ninety_degrees() {
        let (mut doc, _page, ids) = doc_with_three_squares();

        // The selection spans x = 10..140, so a horizontal flip swaps the
        // outer squares and leaves the middle one where it is.
        let ops = flip_operations(&doc, &ids, true);
        assert_eq!(ops.len(), 3, "every member is remapped");
        for op in ops {
            doc.apply(op).expect("apply flip");
        }
        assert!((min_x(&doc, ids[0]) - 120.0).abs() < 1e-9);
        assert!((min_x(&doc, ids[1]) - 80.0).abs() < 1e-9);
        assert!((min_x(&doc, ids[2]) - 10.0).abs() < 1e-9);

        let (doc, _page, ids) = doc_with_three_squares();
        let ops = rotate_clockwise_operations(&doc, &ids[..1]);
        assert_eq!(ops.len(), 1, "one square, one transform write");
        let mut doc = doc;
        for op in ops {
            doc.apply(op).expect("apply rotation");
        }
        let rotated = read_field_text(&doc, &InspectorField::Rotation(ids[0]))
            .and_then(|text| parse_number(text.trim_end_matches('°')))
            .expect("the square reports a rotation");
        assert!(
            (rotated - 90.0).abs() < 0.01,
            "a square at 0° turns to 90°, got {rotated}"
        );
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
                capabilities.sections.contains(&DesignPanelSection::Export),
                "the selected rectangle can export"
            );
            let export = panel
                .view_data()
                .projections
                .export
                .expect("the active inspector has export settings");
            assert_eq!(export.configurations.len(), 1);
            assert_eq!(export.configurations[0].format(), DesignExportFormat::Png);
            assert!(!capabilities.aspect_ratio_lock);
            assert!(!panel.paint_style_view_data().enabled);
        });
    }

    #[gpui::test]
    async fn design_export_rows_echo_add_change_suffix_and_remove(cx: &mut TestAppContext) {
        let (mut doc, _page, rect) = doc_with_rect();
        doc.selection.replace_with([rect]);
        let (view, panel, mut cx) = setup_view(doc, cx).await;
        let cx = &mut cx;
        view.update_in(cx, |view, _, cx| view.refresh_gpui_design(cx));
        cx.run_until_parked();
        let target = DesignPanelTarget::Nodes {
            node_ids: vec![rect.to_string().into()],
        };
        panel.update_in(cx, |_, _, cx| {
            cx.emit(DesignPanelAction::ExportConfigurationAddRequested {
                target: target.clone(),
            });
        });
        cx.run_until_parked();
        panel.read_with(cx, |panel, _| {
            assert_eq!(
                panel
                    .view_data()
                    .projections
                    .export
                    .expect("export projection")
                    .configurations
                    .len(),
                2
            );
        });
        for change in [
            fanta_gpui::design::DesignExportConfigurationChange::Format(DesignExportFormat::Jpg),
            fanta_gpui::design::DesignExportConfigurationChange::Sizing(
                DesignPanelExportSizing::Width(480.0),
            ),
            fanta_gpui::design::DesignExportConfigurationChange::Suffix("-large".into()),
        ] {
            panel.update_in(cx, |_, _, cx| {
                cx.emit(DesignPanelAction::ExportConfigurationChangeRequested {
                    target: target.clone(),
                    configuration_id: "fanta-export-1".into(),
                    change,
                    phase: DesignPanelEditPhase::Commit,
                });
            });
            cx.run_until_parked();
        }
        panel.read_with(cx, |panel, _| {
            let export = panel
                .view_data()
                .projections
                .export
                .expect("export projection");
            let changed = &export.configurations[1];
            assert_eq!(changed.format(), DesignExportFormat::Jpg);
            assert_eq!(changed.sizing, DesignPanelExportSizing::Width(480.0));
            assert_eq!(changed.common.suffix.as_ref(), "-large");
        });
        for phase in [
            DesignPanelEditPhase::Begin,
            DesignPanelEditPhase::Preview,
            DesignPanelEditPhase::Cancel,
        ] {
            panel.update_in(cx, |_, _, cx| {
                cx.emit(DesignPanelAction::ExportConfigurationChangeRequested {
                    target: target.clone(),
                    configuration_id: "fanta-export-1".into(),
                    change: fanta_gpui::design::DesignExportConfigurationChange::Suffix(
                        "-temporary".into(),
                    ),
                    phase,
                });
            });
            cx.run_until_parked();
        }
        panel.read_with(cx, |panel, _| {
            let export = panel
                .view_data()
                .projections
                .export
                .expect("export projection");
            assert_eq!(export.configurations[1].common.suffix.as_ref(), "-large");
        });
        panel.update_in(cx, |_, _, cx| {
            cx.emit(DesignPanelAction::ExportConfigurationRemoveRequested {
                target,
                configuration_id: "fanta-export-1".into(),
            });
        });
        cx.run_until_parked();
        panel.read_with(cx, |panel, _| {
            assert_eq!(
                panel
                    .view_data()
                    .projections
                    .export
                    .expect("export projection")
                    .configurations
                    .len(),
                1
            );
        });
    }

    #[gpui::test]
    async fn design_export_action_writes_png_from_an_unsaved_design(cx: &mut TestAppContext) {
        let (mut doc, _page, rect) = doc_with_rect();
        doc.selection.replace_with([rect]);
        let (view, panel, mut cx) = setup_view(doc, cx).await;
        let cx = &mut cx;
        view.update_in(cx, |view, _, cx| view.refresh_gpui_design(cx));
        cx.run_until_parked();
        panel.update_in(cx, |_, _, cx| {
            cx.emit(DesignPanelAction::ExportAllRequested {
                target: DesignPanelTarget::Nodes {
                    node_ids: vec![rect.to_string().into()],
                },
            });
        });
        assert!(
            cx.did_prompt_for_paths(),
            "unsaved designs ask for an export folder"
        );
        let output = tempfile::tempdir().expect("export destination");
        let directory = output.path().to_path_buf();
        cx.simulate_path_prompt_response(move |options| {
            assert!(options.directories);
            assert!(!options.files);
            Some(vec![directory])
        });
        cx.run_until_parked();
        let exported = output.path().join("Hero.png");
        assert!(exported.exists(), "the export action writes a real PNG");
        let image = image::open(exported).expect("exported PNG can be decoded");
        assert_eq!(image.dimensions(), (200, 100));
        let center = image.get_pixel(100, 50);
        assert!(
            center[0] > center[1] && center[3] > 0,
            "the red layer rendered"
        );
    }

    #[gpui::test]
    async fn design_export_action_writes_the_current_page(cx: &mut TestAppContext) {
        let (doc, page, _rect) = doc_with_rect();
        let (view, panel, mut cx) = setup_view(doc, cx).await;
        let cx = &mut cx;
        view.update_in(cx, |view, _, cx| view.refresh_gpui_design(cx));
        cx.run_until_parked();
        panel.update_in(cx, |_, _, cx| {
            cx.emit(DesignPanelAction::ExportAllRequested {
                target: DesignPanelTarget::Page {
                    page_id: page.to_string().into(),
                },
            });
        });
        assert!(
            cx.did_prompt_for_paths(),
            "page export chooses a destination"
        );
        let output = tempfile::tempdir().expect("export destination");
        let directory = output.path().to_path_buf();
        cx.simulate_path_prompt_response(move |_| Some(vec![directory]));
        cx.run_until_parked();
        let exported = output.path().join("Page 1.png");
        assert!(exported.exists());
        let image = image::open(exported).expect("page PNG can be decoded");
        assert_eq!(image.dimensions(), (200, 100));
        let center = image.get_pixel(100, 50);
        assert!(
            center[0] > center[1] && center[3] > 0,
            "the red page content rendered"
        );
    }

    #[gpui::test]
    async fn font_browser_catalog_and_apply_action_update_the_text_layer(cx: &mut TestAppContext) {
        let (mut doc, page, _) = doc_with_rect();
        let mut text = CanvasNode::new(NodeData::Text(fanta_doc::TextNode::new(
            "Hello", 100.0, 30.0,
        )));
        text.parent = Some(page);
        let id = text.id;
        doc.scene.insert(text).expect("insert text");
        doc.selection.replace_with([id]);
        let (view, panel, mut cx) = setup_view(doc, cx).await;
        let cx = &mut cx;
        view.update_in(cx, |view, _, cx| view.refresh_gpui_design(cx));
        cx.run_until_parked();

        panel.read_with(cx, |panel, _| {
            assert!(
                !panel
                    .node()
                    .typography
                    .as_ref()
                    .expect("text typography")
                    .advanced_type_settings_enabled
            );
            let fonts = &panel.view_data().resources.fonts;
            assert_eq!(fonts.state, DesignFontCatalogState::Ready);
            assert!(
                fonts
                    .font(&DesignFontSelection::local("Source Serif 4", "italic"))
                    .is_some()
            );
        });
        panel.update_in(cx, |_, _, cx| {
            cx.emit(DesignPanelAction::TypographyFontApplyRequested {
                node_id: id.to_string().into(),
                target: DesignTypographyTarget::WholeLayer,
                font: DesignFontSelection::local("Source Serif 4", "italic"),
            });
        });
        cx.run_until_parked();

        let item = view.read_with(cx, |view, _| view.item().clone());
        item.read_with(cx, |item, _| {
            let doc = &item.document().expect("document ready").doc;
            let NodeData::Text(text) = &doc.scene.get(id).expect("text exists").data else {
                panic!("text node");
            };
            assert_eq!(text.style.font_family, "Source Serif 4");
            assert!(text.style.italic);
            assert!(doc.history.can_undo());
        });
    }

    #[gpui::test]
    async fn typography_property_actions_update_the_text_layer(cx: &mut TestAppContext) {
        let (mut doc, page, _) = doc_with_rect();
        let mut text_data = fanta_doc::TextNode::new("Hello", 100.0, 30.0);
        text_data.style_runs.push(fanta_doc::TextStyleRun {
            start: 0,
            end: 5,
            style: text_data.style.clone(),
        });
        let mut text = CanvasNode::new(NodeData::Text(text_data));
        text.parent = Some(page);
        let id = text.id;
        doc.scene.insert(text).expect("insert text");
        doc.selection.replace_with([id]);
        let (view, panel, mut cx) = setup_view(doc, cx).await;

        panel.update_in(&mut cx, |_, _, cx| {
            cx.emit(DesignPanelAction::TypographyPropertyChangeRequested {
                node_id: id.to_string().into(),
                target: DesignTypographyTarget::WholeLayer,
                property: DesignPanelProperty::FontSize,
                value: DesignPanelValue::Number(28.0),
            });
        });
        cx.run_until_parked();
        panel.update_in(&mut cx, |_, _, cx| {
            cx.emit(DesignPanelAction::TypographyPropertyChangeRequested {
                node_id: id.to_string().into(),
                target: DesignTypographyTarget::WholeLayer,
                property: DesignPanelProperty::FontWeight,
                value: DesignPanelValue::Number(600.0),
            });
        });
        cx.run_until_parked();
        panel.update_in(&mut cx, |_, _, cx| {
            cx.emit(DesignPanelAction::TypographyPropertyEditRequested {
                node_id: id.to_string().into(),
                target: DesignTypographyTarget::WholeLayer,
                property: DesignPanelProperty::LineHeight,
                value: DesignPanelValue::LineHeight(DesignLineHeight::Percent(150.0)),
                phase: DesignPanelEditPhase::Commit,
            });
        });
        cx.run_until_parked();
        panel.update_in(&mut cx, |_, _, cx| {
            cx.emit(DesignPanelAction::TypographyPropertyEditRequested {
                node_id: id.to_string().into(),
                target: DesignTypographyTarget::WholeLayer,
                property: DesignPanelProperty::LetterSpacing,
                value: DesignPanelValue::LetterSpacing(DesignLetterSpacing::Pixels(2.0)),
                phase: DesignPanelEditPhase::Commit,
            });
        });
        cx.run_until_parked();
        panel.update_in(&mut cx, |_, _, cx| {
            cx.emit(DesignPanelAction::TypographyPropertyEditRequested {
                node_id: id.to_string().into(),
                target: DesignTypographyTarget::WholeLayer,
                property: DesignPanelProperty::TextDecoration,
                value: DesignPanelValue::TextDecoration(DesignTextDecoration::Underline),
                phase: DesignPanelEditPhase::Commit,
            });
        });
        cx.run_until_parked();
        panel.update_in(&mut cx, |_, _, cx| {
            cx.emit(DesignPanelAction::TypographyPropertyChangeRequested {
                node_id: id.to_string().into(),
                target: DesignTypographyTarget::SelectedTextRange,
                property: DesignPanelProperty::FontSize,
                value: DesignPanelValue::Number(99.0),
            });
        });
        cx.run_until_parked();

        let item = view.read_with(&cx, |view, _| view.item().clone());
        item.read_with(&cx, |item, _| {
            let doc = &item.document().expect("document ready").doc;
            let NodeData::Text(text) = &doc.scene.get(id).expect("text exists").data else {
                panic!("text node");
            };
            assert_eq!(text.style.size_px, 28.0);
            assert_eq!(text.style.weight, 600);
            assert_eq!(text.style.line_height, 1.5);
            assert_eq!(text.style.letter_spacing, 2.0);
            assert!(text.style.underline);
            assert_eq!(text.style_runs.len(), 1);
            assert_eq!(text.style_runs[0].style.size_px, 28.0);
            assert_eq!(text.style_runs[0].style.weight, 600);
            assert_eq!(text.style_runs[0].style.line_height, 1.5);
            assert_eq!(text.style_runs[0].style.letter_spacing, 2.0);
            assert!(text.style_runs[0].style.underline);
            assert!(doc.history.can_undo());
        });
    }

    #[test]
    fn unsupported_node_paint_sections_are_hidden() {
        let text = NodeData::Text(fanta_doc::TextNode::new("Hello", 100.0, 30.0));
        let text_caps = gate_capabilities(
            DesignPanelNodeKind::Text,
            &text,
            false,
            DesignCornerCapabilities::NONE,
        );
        assert!(text_caps.fill);
        assert!(!text_caps.stroke);
        assert!(!text_caps.sections.contains(&DesignPanelSection::Stroke));

        let boolean = NodeData::Boolean(BooleanNode::default());
        let boolean_caps = gate_capabilities(
            DesignPanelNodeKind::BooleanOperation,
            &boolean,
            false,
            DesignCornerCapabilities::NONE,
        );
        assert!(!boolean_caps.fill && !boolean_caps.stroke);
        assert!(!boolean_caps.sections.contains(&DesignPanelSection::Fill));
        assert!(!boolean_caps.sections.contains(&DesignPanelSection::Stroke));

        let vector = NodeData::Vector(VectorNode::rect_solid(
            0.0,
            0.0,
            20.0,
            20.0,
            FantaColor::BLACK,
        ));
        let vector_caps = gate_capabilities(
            DesignPanelNodeKind::Rectangle,
            &vector,
            false,
            DesignCornerCapabilities::NONE,
        );
        assert!(vector_caps.fill && vector_caps.stroke);
    }

    #[gpui::test]
    async fn component_color_edit_applies_and_unsupported_value_is_read_only(
        cx: &mut TestAppContext,
    ) {
        let (mut doc, page, master_root) = doc_with_rect();
        let component = ComponentId::new();
        let color_prop = ComponentPropId::new();
        let unsupported_prop = ComponentPropId::new();
        let mut definition = ComponentDef::new(component, master_root, "Badge");
        definition.props = vec![
            ComponentPropDef {
                id: color_prop,
                name: "Tint".into(),
                kind: ComponentPropKind::Color,
                formatter: Default::default(),
                default: VarValue::Color {
                    value: FantaColor::BLACK,
                },
                bindings: Vec::new(),
            },
            ComponentPropDef {
                id: unsupported_prop,
                name: "Nested component".into(),
                kind: ComponentPropKind::InstanceSwap,
                formatter: Default::default(),
                default: VarValue::String {
                    value: "Original".into(),
                },
                bindings: Vec::new(),
            },
        ];
        doc.components.defs.insert(component, definition);
        let mut instance = CanvasNode::new(NodeData::Instance(InstanceNode {
            component,
            overrides: Vec::new(),
            prop_values: BTreeMap::new(),
            derived: Vec::new(),
            local_size: [120.0, 60.0],
        }));
        instance.parent = Some(page);
        let id = instance.id;
        doc.scene.insert(instance).expect("insert instance");
        doc.selection.replace_with([id]);
        let (view, panel, mut cx) = setup_view(doc, cx).await;
        let cx = &mut cx;
        view.update_in(cx, |view, _, cx| view.refresh_gpui_design(cx));
        cx.run_until_parked();

        panel.read_with(cx, |panel, _| {
            assert_eq!(panel.node().component_properties.len(), 2);
            assert!(
                !panel
                    .view_data()
                    .property_states
                    .get(&DesignPanelProperty::ComponentProperty(0))
                    .is_some_and(DesignPanelPropertyValueState::is_read_only)
            );
            assert!(
                panel
                    .view_data()
                    .property_states
                    .get(&DesignPanelProperty::ComponentProperty(1))
                    .is_some_and(DesignPanelPropertyValueState::is_read_only)
            );
            assert_eq!(
                panel
                    .view_data()
                    .property_states
                    .get(&DesignPanelProperty::ComponentProperty(1))
                    .and_then(DesignPanelPropertyValueState::resolved),
                Some(&DesignPanelValue::Text("Original".into()))
            );
        });

        panel.update_in(cx, |_, _, cx| {
            cx.emit(DesignPanelAction::ComponentPropertyChangeRequested {
                node_id: id.to_string().into(),
                property_id: color_prop.to_string().into(),
                value: DesignComponentPropertyValue::Text("#33669980".into()),
            });
            cx.emit(DesignPanelAction::ComponentPropertyChangeRequested {
                node_id: id.to_string().into(),
                property_id: unsupported_prop.to_string().into(),
                value: DesignComponentPropertyValue::Text("Changed".into()),
            });
            cx.emit(DesignPanelAction::ComponentPropertyChangeRequested {
                node_id: id.to_string().into(),
                property_id: color_prop.to_string().into(),
                value: DesignComponentPropertyValue::Text("invalid hex".into()),
            });
        });
        cx.run_until_parked();

        let item = view.read_with(cx, |view, _| view.item().clone());
        item.read_with(cx, |item, _| {
            let doc = &item.document().expect("document ready").doc;
            let NodeData::Instance(instance) = &doc.scene.get(id).expect("instance exists").data
            else {
                panic!("instance node");
            };
            assert_eq!(
                instance.prop_values.get(&color_prop),
                Some(&VarValue::Color {
                    value: FantaColor::rgba(0x33, 0x66, 0x99, 0x80),
                })
            );
            assert!(!instance.prop_values.contains_key(&unsupported_prop));
            assert!(doc.history.can_undo());
        });
    }

    #[gpui::test]
    async fn property_copy_action_writes_the_displayed_value(cx: &mut TestAppContext) {
        let (mut doc, _page, rect) = doc_with_rect();
        doc.selection.replace_with([rect]);
        let (_view, panel, mut cx) = setup_view(doc, cx).await;
        let cx = &mut cx;
        panel.update_in(cx, |_, _, cx| {
            cx.emit(DesignPanelAction::PropertyCopyRequested {
                target: DesignPanelTarget::Nodes {
                    node_ids: vec![rect.to_string().into()],
                },
                property: DesignPanelProperty::Width,
                displayed_value: "200 px".into(),
            });
        });
        cx.run_until_parked();
        assert_eq!(
            cx.read_from_clipboard().and_then(|item| item.text()),
            Some("200 px".to_string())
        );
    }

    #[gpui::test]
    async fn finishing_external_edit_clears_a_stale_crop_session(cx: &mut TestAppContext) {
        let (mut doc, _page, rect) = doc_with_rect();
        doc.selection.replace_with([rect]);
        let (view, _panel, mut cx) = setup_view(doc, cx).await;
        let cx = &mut cx;
        view.update_in(cx, |view, _, cx| {
            view.refresh_gpui_design(cx);
            let adapter = view.gpui_design.as_mut().expect("design adapter mounted");
            assert!(adapter.last_echo.is_some());
            adapter.crop_session = Some(DesignCropSession {
                node: rect,
                is_stroke: false,
                index: 0,
                state: DesignMediaCropToolState {
                    active: true,
                    ..DesignMediaCropToolState::default()
                },
            });
            view.finish_gpui_design_edits(cx);
            let adapter = view.gpui_design.as_ref().expect("design adapter mounted");
            assert!(adapter.crop_session.is_none());
            assert!(adapter.last_echo.is_none());
        });
    }

    #[gpui::test]
    async fn adding_drop_shadow_is_undoable_and_echoes_to_inspector(cx: &mut TestAppContext) {
        let (mut doc, _page, rect) = doc_with_rect();
        doc.selection.replace_with([rect]);
        let (view, panel, mut cx) = setup_view(doc, cx).await;
        let cx = &mut cx;
        cx.simulate_resize(size(px(1200.), px(900.)));
        view.update_in(cx, |view, _, cx| view.refresh_gpui_design(cx));
        cx.run_until_parked();

        panel.update_in(cx, |_, _, cx| {
            cx.emit(DesignPanelAction::EffectAddRequested {
                node_id: SharedString::from(rect.to_string()),
                kind: DesignEffectKind::DropShadow,
            });
        });
        cx.run_until_parked();

        let item = view.read_with(cx, |view, _| view.item().clone());
        item.read_with(cx, |item, _| {
            let doc = &item.document().expect("document ready").doc;
            let node = doc.scene.get(rect).expect("rect exists");
            assert_eq!(node.effects.len(), 1);
            assert_eq!(node.effects[0].kind, ShadowKind::Drop);
            assert!(doc.history.can_undo());
        });
        panel.read_with(cx, |panel, _| {
            assert_eq!(panel.node().effects.len(), 1);
        });

        for phase in [
            DesignPanelEditPhase::Begin,
            DesignPanelEditPhase::Preview,
            DesignPanelEditPhase::Cancel,
        ] {
            panel.update_in(cx, |_, _, cx| {
                cx.emit(DesignPanelAction::EffectEditRequested {
                    node_id: SharedString::from(rect.to_string()),
                    effect_id: "shadow-0".into(),
                    index: 0,
                    property: DesignPanelProperty::EffectShadowBlur(0),
                    shader_property_id: None,
                    value: DesignPanelValue::Number(24.0),
                    phase,
                });
            });
            cx.run_until_parked();
            let expected = if phase == DesignPanelEditPhase::Preview {
                24.0
            } else {
                default_shadow().blur
            };
            item.read_with(cx, |item, _| {
                let doc = &item.document().expect("document ready").doc;
                assert_eq!(
                    doc.scene.get(rect).expect("rect exists").effects[0].blur,
                    expected
                );
            });
        }

        let shadow_color = DesignColor::rgb(0x36, 0x80, 0xd4);
        for (property, value) in [
            (
                DesignPanelProperty::EffectShadowBlur(0),
                DesignPanelValue::Number(18.0),
            ),
            (
                DesignPanelProperty::EffectShadowColor(0),
                DesignPanelValue::Color(shadow_color),
            ),
        ] {
            panel.update_in(cx, |_, _, cx| {
                cx.emit(DesignPanelAction::EffectEditRequested {
                    node_id: SharedString::from(rect.to_string()),
                    effect_id: "shadow-0".into(),
                    index: 0,
                    property,
                    shader_property_id: None,
                    value,
                    phase: DesignPanelEditPhase::Commit,
                });
            });
            cx.run_until_parked();
        }
        item.update(cx, |item, cx| {
            item.with_document(cx, |document| {
                let doc = &mut document.doc;
                let shadow = &doc.scene.get(rect).expect("rect exists").effects[0];
                assert_eq!(shadow.blur, 18.0);
                assert_eq!(shadow.color, fanta_color(shadow_color));
                assert!(doc.undo().expect("undo shadow color"));
                assert_eq!(
                    doc.scene.get(rect).expect("rect exists").effects[0].color,
                    default_shadow().color
                );
                assert!(doc.undo().expect("undo shadow blur"));
                assert_eq!(
                    doc.scene.get(rect).expect("rect exists").effects[0].blur,
                    default_shadow().blur
                );
                assert!(doc.undo().expect("undo shadow creation"));
                assert!(doc.scene.get(rect).expect("rect exists").effects.is_empty());
                ((), DocChange::Content)
            });
        });
    }

    #[gpui::test]
    async fn adding_drop_shadow_by_click_renders_in_the_mounted_inspector(cx: &mut TestAppContext) {
        let (mut doc, _page, rect) = doc_with_rect();
        doc.selection.replace_with([rect]);
        let (view, panel, mut cx) = setup_view(doc, cx).await;
        let cx = &mut cx;
        cx.simulate_resize(size(px(1200.), px(1600.)));
        view.update_in(cx, |view, _, cx| view.refresh_gpui_design(cx));
        cx.run_until_parked();

        let add = cx
            .debug_bounds("fig-gpui-design-add-effect")
            .expect("the selected rectangle exposes Add Effect");
        cx.simulate_click(add.center(), Modifiers::none());
        cx.run_until_parked();
        panel.read_with(cx, |panel, _| {
            assert_eq!(panel.node().effects.len(), 1);
            assert_eq!(panel.node().effects[0].kind, DesignEffectKind::DropShadow);
        });
        let item = view.read_with(cx, |view, _| view.item().clone());
        item.read_with(cx, |item, _| {
            let doc = &item.document().expect("document ready").doc;
            let shadow = &doc.scene.get(rect).expect("rect exists").effects[0];
            assert_eq!(shadow.color, default_shadow().color);
            assert!(doc.history.can_undo());
        });
    }

    #[test]
    fn effect_edits_reject_nonfinite_filter_values() {
        let (mut doc, _page, rect) = doc_with_rect();
        let node = doc.scene.get_mut(rect).expect("rect exists");
        node.effects.push(default_shadow());
        node.blurs.push(default_blur(BlurKind::Layer));

        for value in [f32::INFINITY, f32::NEG_INFINITY, f32::NAN] {
            for property in [
                DesignPanelProperty::EffectShadowBlur(0),
                DesignPanelProperty::EffectShadowSpread(0),
                DesignPanelProperty::EffectShadowOffsetX(0),
                DesignPanelProperty::EffectShadowOffsetY(0),
            ] {
                assert!(
                    effect_edit_operations(
                        &doc,
                        rect,
                        EffectRef::Shadow(0),
                        property,
                        &DesignPanelValue::Number(value),
                    )
                    .is_none(),
                    "nonfinite shadow value must not reach the renderer"
                );
            }
            assert!(
                effect_edit_operations(
                    &doc,
                    rect,
                    EffectRef::Blur(0),
                    DesignPanelProperty::EffectBlurRadius(0),
                    &DesignPanelValue::Number(value),
                )
                .is_none(),
                "nonfinite blur value must not reach the renderer"
            );
        }
    }

    #[gpui::test]
    async fn gradient_picker_edits_round_trip_and_undo(cx: &mut TestAppContext) {
        let (mut doc, _page, rect) = doc_with_rect();
        if let Some(node) = doc.scene.get_mut(rect)
            && let NodeData::Vector(vector) = &mut node.data
            && let Some(Fill::Solid { blend, .. }) = vector.fills.first_mut()
        {
            *blend = BlendMode::Multiply;
        }
        doc.selection.replace_with([rect]);
        let (view, panel, mut cx) = setup_view(doc, cx).await;
        let cx = &mut cx;
        view.update_in(cx, |view, _, cx| view.refresh_gpui_design(cx));
        cx.run_until_parked();

        let source = DesignPaint::gradient(
            DesignPaintKind::LinearGradient,
            vec![
                DesignGradientStop::new(0.0, DesignColor::rgb(0xe0, 0x30, 0x30)),
                DesignGradientStop::new(1.0, DesignColor::rgb(0x30, 0x30, 0xe0)),
            ],
        );
        let emit = |panel: &Entity<DesignPanel>,
                    cx: &mut VisualTestContext,
                    property: DesignPaintProperty,
                    value: DesignPaintValue| {
            panel.update_in(cx, |_, _, cx| {
                cx.emit(DesignPanelAction::PaintEditRequested {
                    node_id: SharedString::from(rect.to_string()),
                    collection: DesignPanelCollection::Fill,
                    target: DesignPaintTarget::WholeLayer,
                    paint_id: SharedString::from(format!("{rect}-fill-0")),
                    index: 0,
                    edit: DesignPaintEdit { property, value },
                    phase: DesignPanelEditPhase::Commit,
                });
            });
            cx.run_until_parked();
        };
        emit(
            &panel,
            cx,
            DesignPaintProperty::Payload,
            DesignPaintValue::Payload(source.payload),
        );
        let item = view.read_with(cx, |view, _| view.item().clone());
        item.read_with(cx, |item, _| {
            let doc = &item.document().expect("document ready").doc;
            let node = doc.scene.get(rect).expect("rect exists");
            let NodeData::Vector(vector) = &node.data else {
                panic!("rect is a vector");
            };
            assert!(matches!(
                vector.fills.first(),
                Some(Fill::Gradient {
                    blend: BlendMode::Multiply,
                    ..
                })
            ));
        });

        let new_color = DesignColor::rgb(0x22, 0xbb, 0x77);
        emit(
            &panel,
            cx,
            DesignPaintProperty::GradientStopColor {
                stop_id: "".into(),
                index: 1,
            },
            DesignPaintValue::Color(new_color),
        );
        let solid = DesignPaint::solid(new_color);
        emit(
            &panel,
            cx,
            DesignPaintProperty::Payload,
            DesignPaintValue::Payload(solid.payload),
        );
        item.update(cx, |item, cx| {
            item.with_document(cx, |document| {
                let doc = &mut document.doc;
                let node = doc.scene.get(rect).expect("rect exists");
                let NodeData::Vector(vector) = &node.data else {
                    panic!("rect is a vector");
                };
                assert!(matches!(
                    vector.fills.first(),
                    Some(Fill::Solid {
                        color,
                        blend: BlendMode::Multiply
                    }) if *color == fanta_color(new_color)
                ));
                assert!(doc.undo().expect("undo solid conversion"));
                let node = doc.scene.get(rect).expect("rect exists");
                let NodeData::Vector(vector) = &node.data else {
                    panic!("rect is a vector");
                };
                let Some(Fill::Gradient { gradient, .. }) = vector.fills.first() else {
                    panic!("undo restores the gradient");
                };
                assert_eq!(
                    crate::color_picker::gradient_stops(gradient)[1].color,
                    fanta_color(new_color)
                );
                assert!(doc.undo().expect("undo color edit"));
                let node = doc.scene.get(rect).expect("rect exists");
                let NodeData::Vector(vector) = &node.data else {
                    panic!("rect is a vector");
                };
                let Some(Fill::Gradient { gradient, .. }) = vector.fills.first() else {
                    panic!("undo restores the gradient");
                };
                assert_eq!(
                    crate::color_picker::gradient_stops(gradient)[1].color,
                    FantaColor::rgb(0x30, 0x30, 0xe0)
                );
                assert!(doc.undo().expect("undo gradient creation"));
                let node = doc.scene.get(rect).expect("rect exists");
                let NodeData::Vector(vector) = &node.data else {
                    panic!("rect is a vector");
                };
                assert!(matches!(
                    vector.fills.first(),
                    Some(Fill::Solid {
                        blend: BlendMode::Multiply,
                        ..
                    })
                ));
                ((), DocChange::Content)
            });
        });
    }

    #[gpui::test]
    async fn group_fill_reorder_updates_the_primary_background(cx: &mut TestAppContext) {
        let (mut doc, page, _) = doc_with_rect();
        if let Some(node) = doc.scene.get_mut(page)
            && let NodeData::Group(group) = &mut node.data
        {
            group.background = Some(Fill::solid(FantaColor::rgb(255, 0, 0)));
            group
                .background_fills
                .push(Fill::solid(FantaColor::rgb(0, 0, 255)));
        }
        doc.selection.replace_with([page]);
        let (view, panel, mut cx) = setup_view(doc, cx).await;
        let cx = &mut cx;
        panel.update_in(cx, |_, _, cx| {
            cx.emit(DesignPanelAction::PaintReorderRequested {
                node_id: SharedString::from(page.to_string()),
                collection: DesignPanelCollection::Fill,
                target: DesignPaintTarget::WholeLayer,
                paint_id: SharedString::from(format!("{page}-fill-1")),
                from_index: 1,
                to_index: 0,
            });
        });
        cx.run_until_parked();
        let item = view.read_with(cx, |view, _| view.item().clone());
        item.read_with(cx, |item, _| {
            let doc = &item.document().expect("document ready").doc;
            let NodeData::Group(group) = &doc.scene.get(page).expect("page exists").data else {
                panic!("page is a group");
            };
            assert_eq!(
                group.background,
                Some(Fill::solid(FantaColor::rgb(0, 0, 255)))
            );
            assert_eq!(
                group.background_fills[0],
                Fill::solid(FantaColor::rgb(255, 0, 0))
            );
            assert!(doc.history.can_undo());
        });
    }

    #[test]
    fn gradient_transform_projection_preserves_geometry_and_angular_flip() {
        let linear = crate::color_picker::seed_gradient_from_color(FantaColor::rgb(20, 40, 60));
        let projected = design_gradient_transform(&linear);
        let rebuilt = gradient_from_design(
            DesignPaintKind::LinearGradient,
            &gradient_stops(&linear),
            projected,
        )
        .expect("linear projection is valid");
        assert_eq!(rebuilt, linear);

        let angular = Gradient::Angular {
            center: [0.5, 0.5],
            start_angle: 0.0,
            stops: crate::color_picker::gradient_stops(&linear).to_vec(),
        };
        let flipped = gradient_from_design(
            DesignPaintKind::AngularGradient,
            &gradient_stops(&angular),
            design_gradient_transform(&angular).flipped_horizontal(),
        )
        .expect("angular flip is valid");
        let restored = gradient_from_design(
            DesignPaintKind::AngularGradient,
            &gradient_stops(&flipped),
            design_gradient_transform(&flipped).flipped_horizontal(),
        )
        .expect("second angular flip is valid");
        let Gradient::Angular {
            start_angle, stops, ..
        } = restored
        else {
            panic!("angular kind is preserved");
        };
        assert!(start_angle.abs() < 1e-5);
        assert_eq!(stops, crate::color_picker::gradient_stops(&angular));
    }

    #[test]
    fn gradient_opacity_scales_stop_alpha_in_one_undo_step() {
        let (mut doc, _page, rect) = doc_with_rect();
        let original_gradient = Gradient::Linear {
            start: [0.5, 0.0],
            end: [0.5, 1.0],
            stops: vec![
                fanta_doc::GradientStop {
                    position: 0.0,
                    color: FantaColor::rgba(20, 40, 60, 255),
                },
                fanta_doc::GradientStop {
                    position: 1.0,
                    color: FantaColor::rgba(80, 100, 120, 128),
                },
            ],
        };
        if let Some(node) = doc.scene.get_mut(rect)
            && let NodeData::Vector(vector) = &mut node.data
        {
            vector.fills[0] = Fill::Gradient {
                gradient: original_gradient.clone(),
                blend: BlendMode::Normal,
            };
        }
        let ops = paint_edit_operations(&doc, rect, false, 0, &PaintEditValue::Opacity(50.0));
        assert_eq!(ops.len(), 1);
        for operation in ops {
            doc.apply(operation).expect("apply gradient opacity");
        }
        let node = doc.scene.get(rect).expect("rect exists");
        let NodeData::Vector(vector) = &node.data else {
            panic!("rect is a vector");
        };
        let Some(Fill::Gradient { gradient, .. }) = vector.fills.first() else {
            panic!("fill is a gradient");
        };
        let stops = crate::color_picker::gradient_stops(gradient);
        assert_eq!(stops[0].color.a, 128);
        assert_eq!(stops[1].color.a, 64);
        assert!(doc.undo().expect("undo opacity edit"));
        let node = doc.scene.get(rect).expect("rect exists");
        let NodeData::Vector(vector) = &node.data else {
            panic!("rect is a vector");
        };
        assert_eq!(
            vector.fills.first(),
            Some(&Fill::Gradient {
                gradient: original_gradient,
                blend: BlendMode::Normal,
            })
        );
    }

    #[test]
    fn text_fill_opacity_edits_glyph_alpha_and_undoes() {
        let (mut doc, page, _) = doc_with_rect();
        let mut text = CanvasNode::new(NodeData::Text(fanta_doc::TextNode::new(
            "Hello", 100.0, 30.0,
        )));
        text.parent = Some(page);
        let id = text.id;
        doc.scene.insert(text).expect("insert text");
        let ops = paint_edit_operations(&doc, id, false, 0, &PaintEditValue::Opacity(40.0));
        assert_eq!(ops.len(), 1);
        for operation in ops {
            doc.apply(operation).expect("apply text fill opacity");
        }
        let node = doc.scene.get(id).expect("text exists");
        let NodeData::Text(text) = &node.data else {
            panic!("text node");
        };
        assert_eq!(text.style.color.a, 102);
        assert!(doc.undo().expect("undo text fill opacity"));
        let node = doc.scene.get(id).expect("text exists");
        let NodeData::Text(text) = &node.data else {
            panic!("text node");
        };
        assert_eq!(text.style.color.a, 255);

        let ops = paint_edit_operations(
            &doc,
            id,
            false,
            0,
            &PaintEditValue::BlendMode(BlendMode::Multiply),
        );
        assert_eq!(ops.len(), 1);
        for operation in ops {
            doc.apply(operation).expect("apply text fill blend");
        }
        assert_eq!(
            doc.scene.get(id).expect("text exists").blend_mode,
            BlendMode::Multiply
        );
        assert!(doc.undo().expect("undo text fill blend"));
        assert_eq!(
            doc.scene.get(id).expect("text exists").blend_mode,
            BlendMode::Normal
        );
    }

    #[test]
    fn font_catalog_deduplicates_names_and_exposes_italic() {
        let catalog = design_font_catalog([
            SharedString::from("Inter"),
            SharedString::from("inter"),
            SharedString::from(".Hidden Font"),
        ]);
        let inter = catalog
            .families
            .iter()
            .filter(|family| family.name.eq_ignore_ascii_case("Inter"))
            .collect::<Vec<_>>();
        assert_eq!(inter.len(), 1);
        assert!(
            !catalog
                .families
                .iter()
                .any(|family| family.name.starts_with('.'))
        );
        assert!(
            catalog
                .font(&DesignFontSelection::local("Inter", "italic"))
                .is_some_and(|(_, style)| style.italic)
        );
    }

    #[test]
    fn applying_a_font_and_text_decoration_is_undoable() {
        let (mut doc, page, _) = doc_with_rect();
        let mut text = CanvasNode::new(NodeData::Text(fanta_doc::TextNode::new(
            "Hello", 100.0, 30.0,
        )));
        text.parent = Some(page);
        let id = text.id;
        doc.scene.insert(text).expect("insert text");

        let catalog = design_font_catalog(Vec::<SharedString>::new());
        let (family, style) = catalog
            .font(&DesignFontSelection::local("Source Serif 4", "italic"))
            .expect("bundled family and style");
        for operation in font_apply_operations(&doc, id, family.name.as_ref(), style) {
            doc.apply(operation).expect("apply font");
        }
        for operation in property_operations(
            &doc,
            id,
            DesignPanelProperty::TextDecoration,
            &DesignPanelValue::TextDecoration(DesignTextDecoration::Underline),
        )
        .expect("underline is supported")
        {
            doc.apply(operation).expect("apply underline");
        }
        let NodeData::Text(text) = &doc.scene.get(id).expect("text node").data else {
            panic!("node is text");
        };
        assert_eq!(text.style.font_family, "Source Serif 4");
        assert!(text.style.italic);
        assert!(text.style.underline);
        assert!(doc.undo().expect("undo underline"));
        assert!(!matches!(
            &doc.scene.get(id).expect("text node").data,
            NodeData::Text(text) if text.style.underline
        ));
        assert!(doc.undo().expect("undo font"));
        assert!(matches!(
            &doc.scene.get(id).expect("text node").data,
            NodeData::Text(text) if text.style.font_family == "Inter" && !text.style.italic
        ));
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
    async fn page_background_rejects_a_stale_page_id(cx: &mut TestAppContext) {
        let (doc, page, rect) = doc_with_rect();
        let (view, panel, mut cx) = setup_view(doc, cx).await;
        let cx = &mut cx;
        let color = DesignColor::rgb(0x12, 0x34, 0x56);
        panel.update_in(cx, |_, _, cx| {
            cx.emit(DesignPanelAction::PageBackgroundChangeRequested {
                page_id: SharedString::from(rect.to_string()),
                color,
            });
        });
        cx.run_until_parked();
        let item = view.read_with(cx, |view, _| view.item().clone());
        item.read_with(cx, |item, _| {
            assert!(!item.is_dirty());
            let doc = &item.document().expect("document ready").doc;
            assert!(doc.scene.get(rect).is_some());
            assert!(!doc.history.can_undo());
        });

        panel.update_in(cx, |_, _, cx| {
            cx.emit(DesignPanelAction::PageBackgroundChangeRequested {
                page_id: SharedString::from(page.to_string()),
                color,
            });
        });
        cx.run_until_parked();
        item.read_with(cx, |item, _| {
            let doc = &item.document().expect("document ready").doc;
            let NodeData::Group(group) = &doc.scene.get(page).expect("page exists").data else {
                panic!("page is a group");
            };
            assert_eq!(group.background, Some(Fill::solid(fanta_color(color))));
        });
    }

    #[gpui::test]
    async fn page_background_picker_accepts_a_click_in_the_mounted_inspector(
        cx: &mut TestAppContext,
    ) {
        let (doc, page, _) = doc_with_rect();
        let (view, panel, mut cx) = setup_view(doc, cx).await;
        let cx = &mut cx;
        let actions = Rc::new(RefCell::new(Vec::<DesignPanelAction>::new()));
        let _subscription = cx.update(|_, app| {
            let actions = actions.clone();
            app.subscribe(&panel, move |_, action: &DesignPanelAction, _| {
                actions.borrow_mut().push(action.clone());
            })
        });
        cx.simulate_resize(size(px(1200.), px(900.)));
        view.update_in(cx, |view, _, cx| view.refresh_gpui_design(cx));
        cx.run_until_parked();

        panel.read_with(cx, |panel, _| {
            assert!(panel.inspection_context().permissions().can_edit());
            assert!(
                !panel
                    .page_view_data()
                    .expect("page projection")
                    .background
                    .read_only
            );
        });
        let row = cx
            .debug_bounds("design-page-background-row")
            .expect("the mounted inspector shows a page background row");
        cx.simulate_click(row.center(), Modifiers::none());
        cx.run_until_parked();
        let spectrum = cx
            .debug_bounds("color-picker-spectrum")
            .expect("the page background picker is visible");
        let color_point = point(
            spectrum.left() + spectrum.size.width * 0.75,
            spectrum.top() + spectrum.size.height * 0.25,
        );
        cx.simulate_click(color_point, Modifiers::none());
        cx.run_until_parked();

        assert!(actions.borrow().iter().any(|action| {
            matches!(
                action,
                DesignPanelAction::PageBackgroundEditRequested {
                    page_id,
                    phase: DesignPanelEditPhase::Commit,
                    ..
                } if page_id.as_ref() == page.to_string()
            )
        }));
        let item = view.read_with(cx, |view, _| view.item().clone());
        item.read_with(cx, |item, _| {
            let doc = &item.document().expect("document ready").doc;
            let NodeData::Group(group) = &doc.scene.get(page).expect("page exists").data else {
                panic!("page is a group");
            };
            assert!(
                group.background.is_some(),
                "clicking the picker spectrum changes the page background"
            );
            assert!(doc.history.can_undo());
        });

        let close = cx
            .debug_bounds("paint-picker-close")
            .expect("the picker close button is visible");
        cx.simulate_click(close.center(), Modifiers::none());
        cx.run_until_parked();
        assert!(
            cx.debug_bounds("color-picker-spectrum").is_none(),
            "the picker close button accepts clicks"
        );
    }

    #[gpui::test]
    async fn page_background_preview_cancels_and_commits_cleanly(cx: &mut TestAppContext) {
        let (doc, page, _) = doc_with_rect();
        let (view, panel, mut cx) = setup_view(doc, cx).await;
        let cx = &mut cx;
        let color = DesignColor::rgb(0x4c, 0x91, 0xdc);
        let item = view.read_with(cx, |view, _| view.item().clone());
        let emit = |panel: &Entity<DesignPanel>, cx: &mut VisualTestContext, phase| {
            panel.update_in(cx, |_, _, cx| {
                cx.emit(DesignPanelAction::PageBackgroundEditRequested {
                    page_id: SharedString::from(page.to_string()),
                    color,
                    phase,
                });
            });
            cx.run_until_parked();
        };
        for phase in [DesignPanelEditPhase::Begin, DesignPanelEditPhase::Preview] {
            emit(&panel, cx, phase);
        }
        item.read_with(cx, |item, _| {
            let doc = &item.document().expect("document ready").doc;
            let NodeData::Group(group) = &doc.scene.get(page).expect("page exists").data else {
                panic!("page is a group");
            };
            assert_eq!(group.background, Some(Fill::solid(fanta_color(color))));
            assert!(!doc.history.can_undo());
        });
        emit(&panel, cx, DesignPanelEditPhase::Cancel);
        item.read_with(cx, |item, _| {
            let doc = &item.document().expect("document ready").doc;
            let NodeData::Group(group) = &doc.scene.get(page).expect("page exists").data else {
                panic!("page is a group");
            };
            assert_eq!(group.background, None);
            assert!(!doc.history.can_undo());
        });

        for phase in [
            DesignPanelEditPhase::Begin,
            DesignPanelEditPhase::Preview,
            DesignPanelEditPhase::Commit,
        ] {
            emit(&panel, cx, phase);
        }
        item.update(cx, |item, cx| {
            item.with_document(cx, |document| {
                let doc = &mut document.doc;
                let NodeData::Group(group) = &doc.scene.get(page).expect("page exists").data else {
                    panic!("page is a group");
                };
                assert_eq!(group.background, Some(Fill::solid(fanta_color(color))));
                assert!(doc.undo().expect("undo background color"));
                let NodeData::Group(group) = &doc.scene.get(page).expect("page exists").data else {
                    panic!("page is a group");
                };
                assert_eq!(group.background, None);
                ((), DocChange::Content)
            });
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
    async fn aligning_a_three_node_selection_is_one_undo_step(cx: &mut TestAppContext) {
        let (mut doc, _page, ids) = doc_with_three_squares();
        doc.selection.replace_with(ids);
        let (view, panel, mut cx) = setup_view(doc, cx).await;
        let cx = &mut cx;
        view.update_in(cx, |view, _, cx| view.refresh_gpui_design(cx));
        cx.run_until_parked();

        panel.update_in(cx, |_, _, cx| {
            cx.emit(DesignPanelAction::ArrangeRequested {
                target: DesignPanelTarget::Nodes {
                    node_ids: ids
                        .iter()
                        .map(|id| SharedString::from(id.to_string()))
                        .collect(),
                },
                operation: DesignArrangeOperation::AlignLeft,
            });
        });
        cx.run_until_parked();

        let item = view.read_with(cx, |view, _| view.item().clone());
        item.read_with(cx, |item, _| {
            let doc = &item.document().expect("document ready").doc;
            for id in ids {
                assert!(
                    (min_x(doc, id) - 10.0).abs() < 1e-9,
                    "every selected square aligned to the selection's left edge"
                );
            }
        });

        // One undo restores all three: the arrange is a single history entry.
        item.update(cx, |item, cx| {
            item.undo(cx).expect("undo applies");
        });
        cx.run_until_parked();
        item.read_with(cx, |item, _| {
            let doc = &item.document().expect("document ready").doc;
            for (id, x) in ids.into_iter().zip([10.0_f64, 50.0, 120.0]) {
                assert!(
                    (min_x(doc, id) - x).abs() < 1e-9,
                    "one undo restores the whole arrange"
                );
            }
            assert!(
                !doc.history.can_undo(),
                "the arrange left exactly one history entry behind"
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
    /// A page holding a group whose stored box collapsed to 0×0 (what an
    /// auto-layout Hug leaves behind when it measures no content) around a
    /// 200×100 rectangle at (10, 20) in the group's space. A plain group never
    /// clips, so the rectangle keeps rendering at full size.
    fn doc_with_collapsed_group() -> (Doc, NodeId) {
        let mut doc = Doc::new();
        let mut page = CanvasNode::new(NodeData::Group(GroupNode::default()));
        page.name = "Page 1".to_owned();
        let page_id = page.id;
        doc.scene.insert(page).expect("insert page");
        doc.add_page(page_id);
        doc.set_active_page(Some(page_id));
        let mut group = CanvasNode::new(NodeData::Group(GroupNode {
            local_size: Some([0.0, 0.0]),
            ..GroupNode::default()
        }));
        group.name = "Collapsed".to_owned();
        group.parent = Some(page_id);
        let group_id = group.id;
        doc.scene.insert(group).expect("insert group");
        let mut rect = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
            0.0,
            0.0,
            200.0,
            100.0,
            FantaColor::rgb(0xe0, 0x30, 0x30),
        )));
        rect.transform = Transform2D::translation(10.0, 20.0);
        rect.parent = Some(group_id);
        doc.scene.insert(rect).expect("insert rect");
        (doc, group_id)
    }

    #[test]
    fn a_layer_with_no_box_reports_no_value_rather_than_a_zero() {
        let mut doc = Doc::new();
        let page = CanvasNode::new(NodeData::Group(GroupNode::default()));
        let page_id = page.id;
        doc.scene.insert(page).expect("insert page");
        doc.add_page(page_id);

        let section = node_section(&doc, page_id, &HashMap::new()).expect("node section");
        let states = boxless_geometry_states(&section);
        assert_eq!(
            states
                .iter()
                .map(|(property, _)| *property)
                .collect::<Vec<_>>(),
            vec![
                DesignPanelProperty::X,
                DesignPanelProperty::Y,
                DesignPanelProperty::Width,
                DesignPanelProperty::Height,
            ]
        );
        for (property, state) in &states {
            assert!(state.is_unset(), "{property:?} renders as no value");
            assert!(state.is_read_only(), "{property:?} refuses an edit");
        }
    }

    #[test]
    fn a_layer_with_a_box_keeps_its_geometry_fields_live() {
        let (doc, group_id) = doc_with_collapsed_group();
        let section = node_section(&doc, group_id, &HashMap::new()).expect("node section");
        assert!(
            boxless_geometry_states(&section).is_empty(),
            "a layer whose box is derivable keeps editable X/Y/W/H"
        );
    }

    #[gpui::test]
    async fn a_collapsed_group_echoes_the_box_its_content_still_draws(cx: &mut TestAppContext) {
        let (mut doc, group_id) = doc_with_collapsed_group();
        doc.selection.replace_with([group_id]);
        let (view, panel, mut cx) = setup_view(doc, cx).await;
        let cx = &mut cx;
        view.update_in(cx, |view, _, cx| view.refresh_gpui_design(cx));
        cx.run_until_parked();

        panel.read_with(cx, |panel, _| {
            let node = panel.node();
            assert_eq!(node.kind, DesignPanelNodeKind::Group);
            // The stored box says 0×0; the group is plainly at (10, 20) with a
            // 200×100 child that nothing clips.
            assert_eq!((node.x, node.y), (10.0, 20.0));
            assert_eq!((node.width, node.height), (200.0, 100.0));
        });
    }

    /// The snapshot must carry exactly what the three granular setters carry —
    /// inspection context, property value states, Page projection — and must
    /// not drop the Page projection a selected node cannot derive. The granular
    /// echo simply skips `set_page_view_data` there; a snapshot omitting
    /// `projections.page` would wipe it instead.
    ///
    /// This exercises `DesignPanel::set_view_data` directly to pin the
    /// snapshot's completeness. `refresh_gpui_design` deliberately does *not*
    /// use that entry point yet — see its docs, and
    /// `the_echo_keeps_the_page_projection_and_panel_owned_state` for the path
    /// the host actually takes.
    #[gpui::test]
    async fn snapshot_round_trips_through_view_data(cx: &mut TestAppContext) {
        let (mut doc, page_id, rect) = doc_with_rect();
        doc.selection.replace_with([rect]);
        let (view, panel, mut cx) = setup_view(doc, cx).await;
        let cx = &mut cx;
        view.update_in(cx, |view, _, cx| view.refresh_gpui_design(cx));
        cx.run_until_parked();

        let item = view.read_with(cx, |view, _| view.item().clone());
        let editable = item.read_with(cx, |item, _| item.is_editable());
        assert!(editable, "the fixture item is editable");

        // A Page projection the panel already holds, exactly as an earlier
        // empty-selection echo would have left it behind.
        let retained_page = DesignPageViewData::canonical(
            page_id.to_string(),
            DesignPageBackground::new(DesignColor::rgb(0x12, 0x34, 0x56)),
        );

        let (view_data, expected_context, expected_states) = item.read_with(cx, |item, _| {
            let document = item.document().expect("document ready");
            let view_data = build_design_view_data(
                document,
                &[rect],
                None,
                editable,
                Some(retained_page.clone()),
            );
            // What the granular path fed the three setters for this document.
            let masters = master_roots(&document.doc.components);
            let (node, mut states) =
                design_node(document, rect, &masters).expect("the rect projects");
            states.extend(bound_states(&document.doc, rect));
            let context = DesignPanelInspectionContext::single(
                node,
                parent_layout_for(&document.doc, rect),
                DesignPanelPermissions::editor(),
            );
            (view_data, context, states)
        });
        let expected_states: HashMap<_, _> = expected_states.into_iter().collect();

        assert_eq!(
            view_data.inspection_context, expected_context,
            "the snapshot carries the context `set_inspection_context` carried"
        );
        assert_eq!(
            view_data.property_states, expected_states,
            "the snapshot carries every property state, bindings included"
        );
        assert_eq!(
            view_data.projections.page.as_ref(),
            Some(&retained_page),
            "a selected node leaves the panel's Page projection standing"
        );

        // Round trip: the panel accepts the snapshot and reports it back.
        panel.update_in(cx, |panel, _, cx| {
            panel.set_view_data(view_data.clone(), cx);
        });
        cx.run_until_parked();
        panel.read_with(cx, |panel, _| {
            assert_eq!(
                panel.node().id.as_ref(),
                rect.to_string(),
                "the inspected node survives the snapshot"
            );
            assert_eq!(
                panel.view_data().property_states,
                expected_states,
                "the property states survive the snapshot"
            );
            assert_eq!(
                panel.page_view_data(),
                Some(&retained_page),
                "set_view_data kept the Page projection the snapshot carried"
            );
        });

        // An empty selection derives the document's own Page projection, and
        // that derived value wins over whatever the panel was holding.
        let page_snapshot = item.read_with(cx, |item, _| {
            build_design_view_data(
                item.document().expect("document ready"),
                &[],
                None,
                editable,
                Some(retained_page.clone()),
            )
        });
        let derived = page_snapshot
            .projections
            .page
            .expect("an empty selection derives a Page projection");
        assert_eq!(
            derived.page_id.as_ref(),
            page_id.to_string(),
            "the derived projection names the selected page's root"
        );
        assert_ne!(
            derived.background, retained_page.background,
            "the document's own background replaces the retained one"
        );
        assert!(
            page_snapshot.property_states.is_empty(),
            "a Page context inspects no node and so carries no property states"
        );
    }

    /// The echo applies the snapshot through the granular setters, so it
    /// writes only the three halves this adapter owns.
    ///
    /// Two things must survive a selection change: the Page projection an
    /// earlier empty-selection echo left behind (the document cannot re-derive
    /// it while a node is selected), and panel-owned navigation the adapter
    /// never wrote. Both would be cleared by a `set_view_data` handoff that
    /// forgot to carry the panel's own half forward — and the complete-snapshot
    /// handoff additionally drags in the unguarded
    /// `apply_clear_export_view_data`, whose per-echo teardown of the option
    /// dropdown entities is only observable from inside fanta-gpui.
    #[gpui::test]
    async fn the_echo_keeps_the_page_projection_and_panel_owned_state(cx: &mut TestAppContext) {
        // No selection, so the first echo derives the document's own Page
        // projection.
        let (doc, page_id, rect) = doc_with_rect();
        let (view, panel, mut cx) = setup_view(doc, cx).await;
        let cx = &mut cx;
        view.update_in(cx, |view, _, cx| view.refresh_gpui_design(cx));
        cx.run_until_parked();
        panel.read_with(cx, |panel, _| {
            assert_eq!(
                panel.page_view_data().map(|page| page.page_id.to_string()),
                Some(page_id.to_string()),
                "an empty selection echoes the document's Page projection"
            );
        });

        // Panel-owned state: the adapter writes the active surface only from
        // its own `SurfaceChangeRequested` handler, never from an echo.
        panel.update_in(cx, |panel, _, cx| {
            assert!(
                panel.set_active_surface(DesignPanelSurface::Prototype, cx),
                "Prototype is an editor surface"
            );
        });
        cx.run_until_parked();

        // Select the rectangle and echo again.
        let item = view.read_with(cx, |view, _| view.item().clone());
        item.update(cx, |item, cx| {
            item.with_document(cx, |document| {
                document.doc.selection.replace_with([rect]);
                ((), DocChange::Selection)
            });
        });
        view.update_in(cx, |view, _, cx| view.refresh_gpui_design(cx));
        cx.run_until_parked();

        let editable = item.read_with(cx, |item, _| item.is_editable());
        let expected = item.read_with(cx, |item, _| {
            // `page_index` only feeds the Page projection, which an occupied
            // selection does not derive; the two halves compared below are
            // independent of it.
            build_design_view_data(
                item.document().expect("document ready"),
                &[rect],
                None,
                editable,
                None,
            )
        });

        panel.read_with(cx, |panel, _| {
            assert_eq!(
                panel.inspection_context(),
                &expected.inspection_context,
                "the echo applied the inspection context"
            );
            assert_eq!(
                panel.view_data().property_states,
                expected.property_states,
                "the echo applied the property value states"
            );
            assert_eq!(
                panel.page_view_data().map(|page| page.page_id.to_string()),
                Some(page_id.to_string()),
                "a selected node leaves the panel's Page projection standing"
            );
            assert_eq!(
                panel.active_surface(),
                DesignPanelSurface::Prototype,
                "the echo never writes panel-owned navigation"
            );
        });
    }
}
