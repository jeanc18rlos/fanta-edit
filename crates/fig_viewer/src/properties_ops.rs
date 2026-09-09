//! Mutation half of the Fanta properties panel: free functions that turn a
//! committed field edit into document [`Operation`]s, transient-preview
//! plumbing for scrubs and picker sessions, direct `NodeData` mutators shared
//! by those builders, and the panel's number/color formatting helpers. The
//! read-only snapshot model lives in `properties_snapshot`.

use std::collections::HashMap;

use fanta_canvas::{
    HAlign, ResizeHandle, VAlign, align_to_bounds_h, align_to_bounds_v, resize_box_keep_rotation,
    resize_transform_keep_rotation, rotate_about, transform_angle,
};
use fanta_doc::{
    BlendMode, Blur, BlurKind, Bounds as FantaBounds, CanvasNode, Color as FantaColor,
    ComponentDef, ComponentId, ComponentPropId, ComponentSet, ComponentSetMembership, Doc, Fill,
    Gradient, GroupNode, LayoutMode, NodeData, NodeId, Operation, Shadow, ShadowKind, Stroke,
    TextAutoResize, Transform2D, UnitInterval, VarValue, VariantAxis, expand_instance,
};
use glam::DVec2;
use gpui::Rgba;
use smallvec::SmallVec;

use crate::color_picker::{representative_gradient_color, seed_gradient_from_color};
use crate::properties_snapshot::{
    CornerRadiusValue, FONT_WEIGHTS, InspectorField, NodeSnapshot, PaintKind, auto_layout_snapshot,
    corner_radius_value, corner_smoothing_value, master_roots, resolved_instance_def,
};

pub(crate) const DEFAULT_FILL_COLOR: FantaColor = FantaColor::rgb(217, 217, 217);

/// Figma's default page background, seeded when a page without one gets its
/// first color from the inspector.
pub(crate) const DEFAULT_PAGE_BACKGROUND: FantaColor = FantaColor::rgb(245, 245, 245);

fn inspector_local_bounds(doc: &Doc, id: NodeId) -> Option<FantaBounds> {
    let node = doc.scene.get(id)?;
    match &node.data {
        NodeData::Group(group) => group
            .clip_size
            .or(group.local_size)
            .map(|[width, height]| FantaBounds::from_xywh(0.0, 0.0, width, height))
            .or_else(|| doc.scene.local_bounds(id)),
        _ => doc.scene.local_bounds(id),
    }
}

pub(crate) fn inspector_world_size(doc: &Doc, id: NodeId) -> Option<(f64, f64)> {
    let local = inspector_local_bounds(doc, id)?;
    let transform = doc.scene.world_transform(id)?;
    let top_left = transform.transform_point(DVec2::new(local.min_x, local.min_y));
    let top_right = transform.transform_point(DVec2::new(local.max_x, local.min_y));
    let bottom_left = transform.transform_point(DVec2::new(local.min_x, local.max_y));
    Some((
        (top_right - top_left).length(),
        (bottom_left - top_left).length(),
    ))
}

// =============================================================================
// Operations
// =============================================================================

/// Select one exact value on a component instance's variant axis. Other axis
/// values stay unchanged; sparse variant sets reject combinations that do not
/// name an existing member instead of silently cycling to a different value.
pub(crate) fn variant_select_operations(
    doc: &Doc,
    id: NodeId,
    axis: &str,
    selected_value: &str,
) -> Vec<Operation> {
    let Some(node) = doc.scene.get(id) else {
        return Vec::new();
    };
    let NodeData::Instance(instance) = &node.data else {
        return Vec::new();
    };
    let components = &doc.components;
    let Some(definition) = resolved_instance_def(components, instance) else {
        return Vec::new();
    };
    let Some(membership) = &definition.variant_of else {
        return Vec::new();
    };
    let Some(set) = components.sets.get(&membership.set) else {
        return Vec::new();
    };
    let Some(axis_definition) = set.axes.iter().find(|candidate| candidate.name == axis) else {
        return Vec::new();
    };
    if !axis_definition
        .values
        .iter()
        .any(|value| value == selected_value)
        || membership
            .axis_values
            .get(axis)
            .is_some_and(|value| value == selected_value)
    {
        return Vec::new();
    }

    let mut target_values = membership.axis_values.clone();
    target_values.insert(axis.to_string(), selected_value.to_string());
    let Some(target) = set.members.iter().copied().find(|member| {
        components
            .def(*member)
            .and_then(|member_definition| member_definition.variant_of.as_ref())
            .is_some_and(|member_membership| member_membership.axis_values == target_values)
    }) else {
        return Vec::new();
    };
    if target == instance.component {
        return Vec::new();
    }
    vec![Operation::SwapInstance {
        id,
        old: instance.component,
        new: target,
    }]
}

pub(crate) fn field_operations(doc: &Doc, field: &InspectorField, text: &str) -> Vec<Operation> {
    let scene = &doc.scene;
    match field {
        InspectorField::Name(id) => {
            let Some(node) = scene.get(*id) else {
                return Vec::new();
            };
            if text.is_empty() || node.name == text {
                return Vec::new();
            }
            vec![Operation::SetName {
                id: *id,
                old: node.name.clone(),
                new: text.to_string(),
            }]
        }
        InspectorField::X(id) => parse_number(text)
            .map(|x| {
                align_to_bounds_h(
                    scene,
                    &[*id],
                    FantaBounds::from_xywh(x, 0.0, 0.0, 0.0),
                    HAlign::Left,
                )
            })
            .unwrap_or_default(),
        InspectorField::Y(id) => parse_number(text)
            .map(|y| {
                align_to_bounds_v(
                    scene,
                    &[*id],
                    FantaBounds::from_xywh(0.0, y, 0.0, 0.0),
                    VAlign::Top,
                )
            })
            .unwrap_or_default(),
        InspectorField::Width(id) => parse_number(text)
            .map(|width| resize_operations(doc, *id, width, true))
            .unwrap_or_default(),
        InspectorField::Height(id) => parse_number(text)
            .map(|height| resize_operations(doc, *id, height, false))
            .unwrap_or_default(),
        InspectorField::Rotation(id) => parse_number(text.trim_end_matches('°'))
            .map(|degrees| rotation_operations(doc, *id, degrees))
            .unwrap_or_default(),
        InspectorField::CornerRadius(id) => parse_number(text)
            .map(|radius| {
                replace_data_operation(doc, *id, |data| set_corner_radius(data, radius.max(0.0)))
            })
            .unwrap_or_default(),
        InspectorField::CornerSmoothing(id) => parse_number(text.trim_end_matches('%'))
            .map(|percent| {
                let smoothing = (percent / 100.0).clamp(0.0, 1.0) as f32;
                replace_data_operation(doc, *id, |data| set_corner_smoothing(data, smoothing))
            })
            .unwrap_or_default(),
        InspectorField::Opacity(id) => {
            let Some(percent) = parse_number(text.trim_end_matches('%')) else {
                return Vec::new();
            };
            let Some(node) = scene.get(*id) else {
                return Vec::new();
            };
            let new = (percent / 100.0).clamp(0.0, 1.0) as f32;
            if (new - node.opacity.get()).abs() < f32::EPSILON {
                return Vec::new();
            }
            vec![Operation::SetOpacity {
                id: *id,
                old: node.opacity,
                new: UnitInterval::new(new),
            }]
        }
        InspectorField::FillColor { id, index } => {
            let index = *index;
            parse_color(text)
                .map(|color| {
                    replace_data_operation(doc, *id, |data| set_fill_color(data, index, color))
                })
                .unwrap_or_default()
        }
        InspectorField::StrokeColor { id, index } => {
            let index = *index;
            parse_color(text)
                .map(|color| {
                    replace_data_operation(doc, *id, |data| {
                        if let Some(strokes) = stroke_list_mut(data)
                            && let Some(stroke) = strokes.get_mut(index)
                        {
                            let blend = paint_blend(&stroke.paint);
                            stroke.paint = solid_fill_with_blend(color, blend);
                        }
                    })
                })
                .unwrap_or_default()
        }
        InspectorField::PaintOpacity {
            id,
            index,
            is_stroke,
        } => {
            let (index, is_stroke) = (*index, *is_stroke);
            parse_number(text.trim_end_matches('%'))
                .map(|percent| {
                    let fraction = (percent / 100.0).clamp(0.0, 1.0);
                    replace_data_operation(doc, *id, |data| {
                        if let Some(paint) = paint_slot_mut(data, index, is_stroke) {
                            match paint {
                                Fill::Solid { color, .. } => {
                                    color.a = (fraction * 255.0).round() as u8;
                                }
                                Fill::Image { opacity, .. } => *opacity = fraction as f32,
                                Fill::Gradient { .. } => {}
                            }
                        }
                    })
                })
                .unwrap_or_default()
        }
        // Gradients commit through the dedicated gradient-editor path, not the
        // text field, so there is no text operation to author here.
        InspectorField::Gradient { .. } => Vec::new(),
        InspectorField::StrokeWidth { id, index } => {
            let index = *index;
            parse_number(text)
                .map(|width| {
                    replace_data_operation(doc, *id, |data| {
                        if let Some(strokes) = stroke_list_mut(data)
                            && let Some(stroke) = strokes.get_mut(index)
                        {
                            stroke.width = width.max(0.0);
                        }
                    })
                })
                .unwrap_or_default()
        }
        InspectorField::FontFamily(id) => {
            if text.is_empty() {
                return Vec::new();
            }
            replace_data_operation(doc, *id, |data| {
                if let NodeData::Text(node_text) = data {
                    node_text.style.font_family = text.to_string();
                }
            })
        }
        InspectorField::FontSize(id) => parse_number(text)
            .filter(|size| *size > 0.0)
            .map(|size| {
                replace_data_operation(doc, *id, |data| {
                    if let NodeData::Text(node_text) = data {
                        node_text.style.size_px = size;
                    }
                })
            })
            .unwrap_or_default(),
        InspectorField::LineHeight(id) => parse_number(text)
            .filter(|line_height| *line_height > 0.0)
            .map(|line_height| {
                replace_data_operation(doc, *id, |data| {
                    if let NodeData::Text(node_text) = data {
                        node_text.style.line_height = line_height;
                        node_text.style.line_height_auto_percent = None;
                    }
                })
            })
            .unwrap_or_default(),
        InspectorField::LetterSpacing(id) => parse_number(text)
            .map(|letter_spacing| {
                replace_data_operation(doc, *id, |data| {
                    if let NodeData::Text(node_text) = data {
                        node_text.style.letter_spacing = letter_spacing;
                    }
                })
            })
            .unwrap_or_default(),
        InspectorField::CornerRadiusCorner { id, corner } => {
            let corner = *corner;
            parse_number(text)
                .map(|radius| {
                    replace_data_operation(doc, *id, |data| {
                        set_corner_radius_corner(data, corner, radius.max(0.0))
                    })
                })
                .unwrap_or_default()
        }
        InspectorField::LayoutGapH(id) => layout_gap_operations(doc, *id, text, true),
        InspectorField::LayoutGapV(id) => layout_gap_operations(doc, *id, text, false),
        InspectorField::LayoutPadH(id) => layout_padding_operations(doc, *id, text, true),
        InspectorField::LayoutPadV(id) => layout_padding_operations(doc, *id, text, false),
        InspectorField::LayoutMinWidth(id) => layout_limit_operations(doc, *id, text, true, true),
        InspectorField::LayoutMaxWidth(id) => layout_limit_operations(doc, *id, text, true, false),
        InspectorField::LayoutMinHeight(id) => layout_limit_operations(doc, *id, text, false, true),
        InspectorField::LayoutMaxHeight(id) => {
            layout_limit_operations(doc, *id, text, false, false)
        }
        InspectorField::EffectOffsetX { id, index } => {
            let index = *index;
            parse_number(text)
                .map(|x| {
                    shadow_field_operations(doc, *id, index, move |shadow| shadow.offset[0] = x)
                })
                .unwrap_or_default()
        }
        InspectorField::EffectOffsetY { id, index } => {
            let index = *index;
            parse_number(text)
                .map(|y| {
                    shadow_field_operations(doc, *id, index, move |shadow| shadow.offset[1] = y)
                })
                .unwrap_or_default()
        }
        InspectorField::EffectBlur { id, index } => {
            let index = *index;
            parse_number(text)
                .map(|blur| {
                    shadow_field_operations(doc, *id, index, move |shadow| {
                        shadow.blur = blur.max(0.0)
                    })
                })
                .unwrap_or_default()
        }
        InspectorField::EffectSpread { id, index } => {
            let index = *index;
            parse_number(text)
                .map(|spread| {
                    shadow_field_operations(doc, *id, index, move |shadow| shadow.spread = spread)
                })
                .unwrap_or_default()
        }
        InspectorField::EffectColor { id, index } => {
            let index = *index;
            parse_color(text)
                .map(|color| {
                    shadow_field_operations(doc, *id, index, move |shadow| shadow.color = color)
                })
                .unwrap_or_default()
        }
        InspectorField::BlurRadius { id, index } => {
            let index = *index;
            parse_number(text)
                .map(|radius| {
                    blurs_operations(doc, *id, |blurs| {
                        if let Some(blur) = blurs.get_mut(index) {
                            blur.radius = radius.max(0.0);
                        }
                    })
                })
                .unwrap_or_default()
        }
        InspectorField::TextColor(id) => parse_color(text)
            .map(|color| {
                replace_data_operation(doc, *id, |data| {
                    if let NodeData::Text(node_text) = data {
                        node_text.style.color = color;
                    }
                })
            })
            .unwrap_or_default(),
        InspectorField::InstanceTextProp { id, prop } => instance_prop_operations(
            doc,
            *id,
            *prop,
            Some(VarValue::String {
                value: text.to_string(),
            }),
        ),
        InspectorField::InstanceNumberProp { id, prop } => parse_number(text)
            .map(|value| instance_prop_operations(doc, *id, *prop, Some(VarValue::Float { value })))
            .unwrap_or_default(),
        InspectorField::InstanceColorProp { id, prop } => parse_color(text)
            .map(|value| instance_prop_operations(doc, *id, *prop, Some(VarValue::Color { value })))
            .unwrap_or_default(),
        InspectorField::SelectionColor { from } => parse_color(text)
            .filter(|to| to != from)
            .map(|to| selection_color_operations(doc, *from, to))
            .unwrap_or_default(),
        InspectorField::InstanceText { id, path } => {
            crate::instance_text::commit_ops(doc, *id, path, Some(text), None)
        }
        InspectorField::CommentText { page, id } => {
            crate::comments::update_comment_text_op(doc, *page, id, text)
                .into_iter()
                .collect()
        }
        InspectorField::PageBackground(id) => {
            if text.is_empty() {
                replace_data_operation(doc, *id, |data| {
                    if let NodeData::Group(group) = data {
                        group.background = None;
                    }
                })
            } else {
                parse_color(text)
                    .map(|color| {
                        replace_data_operation(doc, *id, |data| {
                            if let NodeData::Group(group) = data {
                                group.background = Some(Fill::solid(color));
                            }
                        })
                    })
                    .unwrap_or_default()
            }
        }
    }
}

/// The committed display text a field currently shows, for seeding the shared
/// editor when Tab hops between paired fields. Only the fields reachable via
/// [`paired_field`] need coverage.
pub(crate) fn read_field_text(doc: &Doc, field: &InspectorField) -> Option<String> {
    let scene = &doc.scene;
    match field {
        InspectorField::InstanceText { id, path } => crate::instance_text::text_clones(doc, *id)
            .into_iter()
            .find(|(clone_path, ..)| clone_path == path)
            .map(|(_, _, content)| content),
        InspectorField::CommentText { page, id } => crate::comments::read_comments(doc, *page)
            .into_iter()
            .find(|comment| comment.id == *id)
            .map(|comment| comment.text),
        InspectorField::X(id) => scene.world_bounds(*id).map(|b| format_number(b.min_x)),
        InspectorField::Y(id) => scene.world_bounds(*id).map(|b| format_number(b.min_y)),
        InspectorField::Width(id) => inspector_world_size(doc, *id)
            .map(|(w, _)| w)
            .or_else(|| scene.world_bounds(*id).map(|b| b.width()))
            .map(format_number),
        InspectorField::Height(id) => inspector_world_size(doc, *id)
            .map(|(_, h)| h)
            .or_else(|| scene.world_bounds(*id).map(|b| b.height()))
            .map(format_number),
        InspectorField::Rotation(id) => scene
            .world_transform(*id)
            .map(|transform| format_number(transform_angle(&transform).to_degrees())),
        InspectorField::CornerRadius(id) => {
            let node = scene.get(*id)?;
            match corner_radius_value(node) {
                CornerRadiusValue::Uniform(radius) => Some(format_number(radius)),
                CornerRadiusValue::PerCorner(_) => Some(String::new()),
                CornerRadiusValue::NotApplicable => None,
            }
        }
        InspectorField::CornerRadiusCorner { id, corner } => {
            let node = scene.get(*id)?;
            match corner_radius_value(node) {
                CornerRadiusValue::Uniform(radius) => Some(format_number(radius)),
                CornerRadiusValue::PerCorner(radii) => {
                    radii.get(*corner).copied().map(format_number)
                }
                CornerRadiusValue::NotApplicable => None,
            }
        }
        InspectorField::CornerSmoothing(id) => {
            corner_smoothing_value(scene.get(*id)?).map(format_number)
        }
        InspectorField::LineHeight(id) => text_style_value(scene, *id, |style| style.line_height),
        InspectorField::LetterSpacing(id) => {
            text_style_value(scene, *id, |style| style.letter_spacing)
        }
        InspectorField::FontSize(id) => text_style_value(scene, *id, |style| style.size_px),
        InspectorField::LayoutGapH(id) | InspectorField::LayoutGapV(id) => {
            let node = scene.get(*id)?;
            let layout = auto_layout_snapshot(node)?;
            Some(format_number(
                if matches!(field, InspectorField::LayoutGapH(_)) {
                    layout.gap_h
                } else {
                    layout.gap_v
                },
            ))
        }
        InspectorField::LayoutPadH(id) | InspectorField::LayoutPadV(id) => {
            let node = scene.get(*id)?;
            let layout = auto_layout_snapshot(node)?;
            let pad = if matches!(field, InspectorField::LayoutPadH(_)) {
                layout.pad_h
            } else {
                layout.pad_v
            };
            Some(pad.map(format_number).unwrap_or_default())
        }
        InspectorField::LayoutMinWidth(id)
        | InspectorField::LayoutMaxWidth(id)
        | InspectorField::LayoutMinHeight(id)
        | InspectorField::LayoutMaxHeight(id) => {
            let node = scene.get(*id)?;
            let layout = auto_layout_snapshot(node)?;
            let value = match field {
                InspectorField::LayoutMinWidth(_) => layout.min_width,
                InspectorField::LayoutMaxWidth(_) => layout.max_width,
                InspectorField::LayoutMinHeight(_) => layout.min_height,
                InspectorField::LayoutMaxHeight(_) => layout.max_height,
                _ => return None,
            };
            Some(value.map(format_number).unwrap_or_default())
        }
        InspectorField::EffectOffsetX { id, index } => {
            effect_value(scene, *id, *index, |shadow| shadow.offset[0])
        }
        InspectorField::EffectOffsetY { id, index } => {
            effect_value(scene, *id, *index, |shadow| shadow.offset[1])
        }
        InspectorField::EffectBlur { id, index } => {
            effect_value(scene, *id, *index, |shadow| shadow.blur)
        }
        InspectorField::EffectSpread { id, index } => {
            effect_value(scene, *id, *index, |shadow| shadow.spread)
        }
        _ => None,
    }
}

pub(crate) fn text_style_value(
    scene: &fanta_doc::Scene,
    id: NodeId,
    read: impl FnOnce(&fanta_doc::TextStyle) -> f64,
) -> Option<String> {
    let node = scene.get(id)?;
    let NodeData::Text(text) = &node.data else {
        return None;
    };
    Some(format_number(read(&text.style)))
}

pub(crate) fn effect_value(
    scene: &fanta_doc::Scene,
    id: NodeId,
    index: usize,
    read: impl FnOnce(&Shadow) -> f64,
) -> Option<String> {
    let node = scene.get(id)?;
    node.effects
        .get(index)
        .map(|shadow| format_number(read(shadow)))
}

pub(crate) fn replace_data_operation(
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

pub(crate) fn set_clip_content_meta_operation(
    doc: &Doc,
    id: NodeId,
    enabled: bool,
) -> Vec<Operation> {
    let Some(node) = doc.scene.get(id) else {
        return Vec::new();
    };
    let old = node.meta.clone();
    let mut new = match &old {
        serde_json::Value::Object(map) => serde_json::Value::Object(map.clone()),
        _ => serde_json::json!({}),
    };
    let Some(meta) = new.as_object_mut() else {
        return Vec::new();
    };
    if enabled {
        // Presence historically meant clipping enabled. Keep authored files
        // compatible by using that canonical default rather than storing true.
        meta.remove("clip_content");
    } else {
        meta.insert("clip_content".to_string(), serde_json::Value::Bool(false));
    }
    (new != old)
        .then_some(Operation::SetMeta { id, old, new })
        .into_iter()
        .collect()
}

pub(crate) fn effects_operations(
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

pub(crate) fn shadow_field_operations(
    doc: &Doc,
    id: NodeId,
    index: usize,
    mutate: impl FnOnce(&mut Shadow),
) -> Vec<Operation> {
    effects_operations(doc, id, |effects| {
        if let Some(shadow) = effects.get_mut(index) {
            mutate(shadow);
        }
    })
}

/// The blur twin of [`effects_operations`]: snapshot the node's blur stack,
/// mutate a copy, and emit one `SetBlurs` when it actually changed.
pub(crate) fn blurs_operations(
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

/// Set (or clear) one exposed prop on an instance. `None` clears the override
/// back to the def's default.
pub(crate) fn instance_prop_operations(
    doc: &Doc,
    id: NodeId,
    prop: ComponentPropId,
    new: Option<VarValue>,
) -> Vec<Operation> {
    let Some(node) = doc.scene.get(id) else {
        return Vec::new();
    };
    let NodeData::Instance(instance) = &node.data else {
        return Vec::new();
    };
    let old = instance.prop_values.get(&prop).cloned();
    if old == new {
        return Vec::new();
    }
    vec![Operation::SetInstanceProp { id, prop, old, new }]
}

/// Replace one solid color across every selected node — the multi-select
/// "Selection colors" edit. One `ReplaceData` per node that actually uses the
/// color; the caller batches them into a single undo step.
pub(crate) fn selection_color_operations(
    doc: &Doc,
    from: FantaColor,
    to: FantaColor,
) -> Vec<Operation> {
    doc.selection
        .iter()
        .copied()
        .filter(|id| doc.scene.contains(*id))
        .flat_map(|id| {
            replace_data_operation(doc, id, |data| {
                let replace = |fill: &mut Fill| {
                    if let Fill::Solid { color, .. } = fill
                        && *color == from
                    {
                        *color = to;
                    }
                };
                match data {
                    NodeData::Vector(vector) => {
                        vector.fills.iter_mut().for_each(replace);
                        vector
                            .strokes
                            .iter_mut()
                            .for_each(|stroke| replace(&mut stroke.paint));
                    }
                    NodeData::Group(group) => {
                        group
                            .background
                            .iter_mut()
                            .chain(group.background_fills.iter_mut())
                            .for_each(replace);
                        group
                            .strokes
                            .iter_mut()
                            .for_each(|stroke| replace(&mut stroke.paint));
                    }
                    NodeData::Text(text) => {
                        // If live text edit with selection, the color change will have been
                        // applied via with_text_selection_style already in the caller path.
                        if text.style.color == from {
                            text.style.color = to;
                        }
                    }
                    _ => {}
                }
            })
        })
        .collect()
}

/// Detach an instance into a concrete subtree: the instance node becomes the
/// resolved master-root frame and the expansion's descendants are materialized
/// under it. The master root's own wrapper-level surface props (opacity,
/// effects, blurs, blend) live outside `NodeData`, so the data swap alone would
/// drop them — they are composed onto the detached node by the follow-up ops.
pub(crate) fn detach_instance_operations(doc: &Doc, id: NodeId) -> Vec<Operation> {
    let Some(node) = doc.scene.get(id) else {
        return Vec::new();
    };
    let NodeData::Instance(instance) = &node.data else {
        return Vec::new();
    };
    let expanded = expand_instance(&doc.scene, &doc.components, instance);
    // `expand_instance` yields the root first (empty def path), then its subtree
    // parents-before-children, so this insertion order stays valid.
    let Some(root) = expanded.iter().find(|entry| entry.def_path.is_empty()) else {
        // A dangling master: nothing to materialize.
        return Vec::new();
    };
    let root_id = root.node.id;
    let children: Vec<CanvasNode> = expanded
        .iter()
        .filter(|entry| !entry.def_path.is_empty())
        .map(|entry| {
            let mut child = entry.node.clone();
            if child.parent == Some(root_id) {
                child.parent = Some(id);
            }
            child
        })
        .collect();

    let mut operations = vec![Operation::DetachInstance {
        id,
        old: Box::new(node.data.clone()),
        new: Box::new(root.node.data.clone()),
        expanded: children,
    }];
    let composed_opacity = UnitInterval::new(node.opacity.get() * root.node.opacity.get());
    if (composed_opacity.get() - node.opacity.get()).abs() > f32::EPSILON {
        operations.push(Operation::SetOpacity {
            id,
            old: node.opacity,
            new: composed_opacity,
        });
    }
    if node.effects.is_empty() && !root.node.effects.is_empty() {
        operations.push(Operation::SetEffects {
            id,
            old: node.effects.clone(),
            new: root.node.effects.clone(),
        });
    }
    if node.blurs.is_empty() && !root.node.blurs.is_empty() {
        operations.push(Operation::SetBlurs {
            id,
            old: node.blurs.clone(),
            new: root.node.blurs.clone(),
        });
    }
    if node.blend_mode.is_normal() && !root.node.blend_mode.is_normal() {
        operations.push(Operation::SetBlendMode {
            id,
            old: node.blend_mode,
            new: root.node.blend_mode,
        });
    }
    operations
}

/// Promote the frame or group rooted at `id` into a new component master —
/// one undoable [`Operation::DefineComponent`], named after the node. The
/// master stays where it is on the canvas (Figma behavior). Non-groups,
/// instances, and nodes that already are masters are refused.
pub(crate) fn create_component_operations(doc: &Doc, id: NodeId) -> Vec<Operation> {
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

/// Merge the selected component masters into one variant set along a single
/// "Variant" axis whose values are the masters' names. The first selected master
/// becomes the set's default variant.
pub(crate) fn combine_as_variants_operations(doc: &Doc) -> Vec<Operation> {
    let masters = master_roots(&doc.components);
    let members: Vec<ComponentId> = doc
        .selection
        .iter()
        .filter_map(|id| masters.get(id).copied())
        // A master already in a set would need its old set repaired first.
        .filter(|component| {
            doc.components
                .def(*component)
                .is_some_and(|def| def.variant_of.is_none())
        })
        .collect();
    let (Some(default_variant), true) = (members.first().copied(), members.len() >= 2) else {
        return Vec::new();
    };
    const AXIS: &str = "Variant";
    let values: Vec<String> = members
        .iter()
        .filter_map(|component| doc.components.def(*component))
        .map(|def| def.name.clone())
        .collect();
    let set = ComponentSet {
        id: ComponentId::new(),
        name: doc
            .components
            .def(default_variant)
            .map(|def| def.name.clone())
            .unwrap_or_else(|| "Components".to_string()),
        axes: vec![VariantAxis {
            name: AXIS.to_string(),
            values,
        }],
        members: members.clone(),
        default_variant,
    };
    let set_id = set.id;
    let mut operations = vec![Operation::DefineComponentSet { set: Box::new(set) }];
    for component in members {
        let Some(def) = doc.components.def(component) else {
            continue;
        };
        let mut axis_values = std::collections::BTreeMap::new();
        axis_values.insert(AXIS.to_string(), def.name.clone());
        operations.push(Operation::SetVariantMembership {
            id: component,
            old: def.variant_of.clone(),
            new: Some(ComponentSetMembership {
                set: set_id,
                axis_values,
            }),
        });
    }
    operations
}

/// Write one screen-axis gap of an auto-layout frame. The horizontal gap is
/// the primary spacing of a horizontal stack but the counter (wrap) spacing of
/// a vertical one; see [`auto_layout_snapshot`].
pub(crate) fn layout_gap_operations(
    doc: &Doc,
    id: NodeId,
    text: &str,
    horizontal: bool,
) -> Vec<Operation> {
    let Some(gap) = parse_number(text).map(|gap| gap.max(0.0)) else {
        return Vec::new();
    };
    replace_data_operation(doc, id, |data| {
        if let NodeData::Group(group) = data
            && let Some(layout) = group.auto_layout.as_mut()
        {
            let primary = horizontal == (layout.mode == LayoutMode::Horizontal);
            if primary {
                layout.spacing = gap;
            } else {
                layout.counter_spacing = gap;
            }
        }
    })
}

pub(crate) fn layout_padding_operations(
    doc: &Doc,
    id: NodeId,
    text: &str,
    horizontal: bool,
) -> Vec<Operation> {
    let Some(padding) = parse_number(text).map(|padding| padding.max(0.0)) else {
        return Vec::new();
    };
    replace_data_operation(doc, id, |data| {
        if let NodeData::Group(group) = data
            && let Some(layout) = group.auto_layout.as_mut()
        {
            // padding is [top, right, bottom, left].
            if horizontal {
                layout.padding[1] = padding;
                layout.padding[3] = padding;
            } else {
                layout.padding[0] = padding;
                layout.padding[2] = padding;
            }
        }
    })
}

pub(crate) fn layout_limit_operations(
    doc: &Doc,
    id: NodeId,
    text: &str,
    horizontal: bool,
    minimum: bool,
) -> Vec<Operation> {
    let value = if text.trim().is_empty() {
        None
    } else {
        let Some(value) = parse_number(text).map(|value| value.max(0.0)) else {
            return Vec::new();
        };
        Some(value)
    };
    replace_data_operation(doc, id, |data| {
        if let NodeData::Group(group) = data
            && let Some(layout) = group.auto_layout.as_mut()
        {
            let [mut minimum_width, mut minimum_height] = layout.min_size;
            let [mut maximum_width, mut maximum_height] = layout.max_size;
            let (minimum_limit, maximum_limit) = if horizontal {
                (&mut minimum_width, &mut maximum_width)
            } else {
                (&mut minimum_height, &mut maximum_height)
            };
            if minimum {
                *minimum_limit = value;
                if let (Some(minimum), Some(maximum)) = (*minimum_limit, *maximum_limit)
                    && maximum < minimum
                {
                    *maximum_limit = Some(minimum);
                }
            } else {
                *maximum_limit = value;
                if let (Some(minimum), Some(maximum)) = (*minimum_limit, *maximum_limit)
                    && minimum > maximum
                {
                    *minimum_limit = Some(maximum);
                }
            }
            layout.min_size = [minimum_width, minimum_height];
            layout.max_size = [maximum_width, maximum_height];
        }
    })
}

/// Change one oriented dimension of a node by pinning the opposite edge, the
/// same math the canvas resize handles use so rotation is preserved. Text
/// changes its content box and reflows; groups change their own box without
/// scaling descendants; leaf geometry resizes through its transform.
pub(crate) fn resize_operations(
    doc: &Doc,
    id: NodeId,
    new_size: f64,
    horizontal: bool,
) -> Vec<Operation> {
    if new_size <= 0.0 {
        return Vec::new();
    }
    let scene = &doc.scene;
    let Some(node) = scene.get(id) else {
        return Vec::new();
    };
    let Some(scene_local) = scene.local_bounds(id) else {
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

/// Drop transform operations whose result is non-finite. Imported documents
/// can carry non-finite parent transforms, which the world→local rebase in
/// the field editors would propagate into NaN matrices; writing one poisons
/// layout and rendering for the whole subtree.
pub(crate) fn finite_transform_operations(operations: Vec<Operation>) -> Vec<Operation> {
    operations
        .into_iter()
        .filter(|operation| match operation {
            Operation::SetTransform { new, .. } => {
                let finite = new
                    .to_components()
                    .iter()
                    .all(|component| component.is_finite());
                if !finite {
                    log::warn!("discarding inspector edit that produced a non-finite transform");
                }
                finite
            }
            _ => true,
        })
        .collect()
}

pub(crate) fn rotation_operations(doc: &Doc, id: NodeId, degrees: f64) -> Vec<Operation> {
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
    let pivot = inspector_local_bounds(doc, id)
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

// =============================================================================
// Transient previews
// =============================================================================

/// Restore the snapshotted node state directly into the scene — the
/// gesture-start baseline every preview frame recomputes from, mirroring the
/// canvas move tool's transient staging.
pub(crate) fn restore_snapshot(doc: &mut Doc, snapshot: &NodeSnapshot) {
    if let Some(node) = doc.scene.get_mut(snapshot.id) {
        node.transform = snapshot.transform;
        node.opacity = snapshot.opacity;
        node.data = (*snapshot.data).clone();
        node.effects = snapshot.effects.clone();
        node.blurs = snapshot.blurs.clone();
        doc.components.bump_rev_for_node(&doc.scene, snapshot.id);
    }
}

/// Write an operation's `new` value straight into the scene WITHOUT history —
/// the preview twin of `Doc::apply`, so a scrub frame shows exactly what the
/// commit will produce. Like `apply`, it bumps the revision of any component
/// master containing the node, so instances re-expand for the preview.
pub(crate) fn apply_preview_operation(doc: &mut Doc, operation: &Operation) {
    if let Some(target) = operation.primary_target() {
        doc.components.bump_rev_for_node(&doc.scene, target);
    }
    match operation {
        Operation::SetTransform { id, new, .. } => {
            if let Some(node) = doc.scene.get_mut(*id) {
                node.transform = *new;
            }
        }
        Operation::SetOpacity { id, new, .. } => {
            if let Some(node) = doc.scene.get_mut(*id) {
                node.opacity = *new;
            }
        }
        Operation::ReplaceData { id, new, .. } => {
            if let Some(node) = doc.scene.get_mut(*id) {
                node.data = (**new).clone();
            }
        }
        Operation::SetEffects { id, new, .. } => {
            if let Some(node) = doc.scene.get_mut(*id) {
                node.effects = new.clone();
            }
        }
        Operation::SetBlurs { id, new, .. } => {
            if let Some(node) = doc.scene.get_mut(*id) {
                node.blurs = new.clone();
            }
        }
        Operation::SetInstanceProp { id, prop, new, .. } => {
            if let Some(node) = doc.scene.get_mut(*id)
                && let NodeData::Instance(instance) = &mut node.data
            {
                match new {
                    Some(value) => instance.prop_values.insert(*prop, value.clone()),
                    None => instance.prop_values.remove(prop),
                };
            }
        }
        _ => {}
    }
}

// =============================================================================
// Node data mutators
// =============================================================================

pub(crate) fn set_corner_radius(data: &mut NodeData, radius: f64) {
    let (corner_radius, corner_radii) = match data {
        NodeData::Vector(vector) => (&mut vector.corner_radius, &mut vector.corner_radii),
        NodeData::Group(group) => (&mut group.corner_radius, &mut group.corner_radii),
        _ => return,
    };
    *corner_radius = (radius > 0.0).then_some(radius);
    *corner_radii = None;
}

/// Set one corner of the per-corner radii (indexed [TL, TR, BR, BL]), seeding
/// the other three from the uniform radius. Collapses back to the uniform
/// field when all four end up equal so the compact representation survives.
pub(crate) fn set_corner_radius_corner(data: &mut NodeData, corner: usize, radius: f64) {
    let (corner_radius, corner_radii) = match data {
        NodeData::Vector(vector) => (&mut vector.corner_radius, &mut vector.corner_radii),
        NodeData::Group(group) => (&mut group.corner_radius, &mut group.corner_radii),
        _ => return,
    };
    let mut radii = corner_radii.unwrap_or([corner_radius.unwrap_or(0.0); 4]);
    let Some(slot) = radii.get_mut(corner) else {
        return;
    };
    *slot = radius;
    if radii.iter().all(|value| *value == radii[0]) {
        *corner_radius = (radii[0] > 0.0).then_some(radii[0]);
        *corner_radii = None;
    } else {
        *corner_radii = Some(radii);
    }
}

/// Set the squircle corner smoothing (0.0 = circular corners, 1.0 = full
/// squircle) on a corner-capable node.
pub(crate) fn set_corner_smoothing(data: &mut NodeData, smoothing: f32) {
    let slot = match data {
        NodeData::Vector(vector) => &mut vector.corner_smoothing,
        NodeData::Group(group) => &mut group.corner_smoothing,
        _ => return,
    };
    *slot = smoothing.clamp(0.0, 1.0);
}

pub(crate) fn stroke_list_mut(data: &mut NodeData) -> Option<&mut SmallVec<[Stroke; 1]>> {
    match data {
        NodeData::Vector(vector) => Some(&mut vector.strokes),
        NodeData::Group(group) => Some(&mut group.strokes),
        _ => None,
    }
}

pub(crate) fn group_fill_slot(group: &mut GroupNode, index: usize) -> Option<&mut Fill> {
    if group.background.is_some() {
        if index == 0 {
            group.background.as_mut()
        } else {
            group.background_fills.get_mut(index - 1)
        }
    } else {
        group.background_fills.get_mut(index)
    }
}

pub(crate) fn fill_slot_mut(data: &mut NodeData, index: usize) -> Option<&mut Fill> {
    match data {
        NodeData::Vector(vector) => vector.fills.get_mut(index),
        NodeData::Group(group) => group_fill_slot(group, index),
        _ => None,
    }
}

pub(crate) fn paint_blend(fill: &Fill) -> BlendMode {
    match fill {
        Fill::Solid { blend, .. } | Fill::Gradient { blend, .. } | Fill::Image { blend, .. } => {
            *blend
        }
    }
}

pub(crate) fn solid_fill_with_blend(color: FantaColor, blend: BlendMode) -> Fill {
    Fill::Solid { color, blend }
}

pub(crate) fn set_fill_color(data: &mut NodeData, index: usize, color: FantaColor) {
    if let Some(fill) = fill_slot_mut(data, index) {
        let blend = paint_blend(fill);
        *fill = solid_fill_with_blend(color, blend);
    }
}

/// A mutable handle to the paint at `index` in either the fill or stroke list.
pub(crate) fn paint_slot_mut(
    data: &mut NodeData,
    index: usize,
    is_stroke: bool,
) -> Option<&mut Fill> {
    if is_stroke {
        stroke_list_mut(data)
            .and_then(|strokes| strokes.get_mut(index))
            .map(|stroke| &mut stroke.paint)
    } else {
        fill_slot_mut(data, index)
    }
}

/// Replace the paint at `index` with a gradient fill, preserving the paint's
/// per-paint blend mode when it already carried one.
pub(crate) fn set_paint_gradient(
    data: &mut NodeData,
    index: usize,
    is_stroke: bool,
    gradient: Gradient,
) {
    if let Some(paint) = paint_slot_mut(data, index, is_stroke) {
        let blend = paint_blend(paint);
        *paint = Fill::Gradient { gradient, blend };
    }
}

/// Convert the paint at `index` to `kind`. Solid ⇄ gradient seeding matches
/// Figma: a solid becomes a two-stop gradient seeded from its color, and a
/// gradient flattens back to its representative (first-stop) color.
pub(crate) fn convert_paint_kind(
    data: &mut NodeData,
    index: usize,
    is_stroke: bool,
    kind: PaintKind,
) {
    let Some(paint) = paint_slot_mut(data, index, is_stroke) else {
        return;
    };
    match kind {
        PaintKind::Solid => {
            let blend = paint_blend(paint);
            let color = match paint {
                Fill::Solid { color, .. } => *color,
                Fill::Gradient { gradient, .. } => representative_gradient_color(gradient),
                Fill::Image { .. } => return,
            };
            *paint = solid_fill_with_blend(color, blend);
        }
        PaintKind::Gradient(gradient_kind) => {
            let blend = paint_blend(paint);
            let gradient = match paint {
                Fill::Gradient { gradient, .. } => {
                    crate::color_picker::convert_gradient_kind(gradient, gradient_kind)
                }
                Fill::Solid { color, .. } => crate::color_picker::convert_gradient_kind(
                    &seed_gradient_from_color(*color),
                    gradient_kind,
                ),
                Fill::Image { .. } => return,
            };
            *paint = Fill::Gradient { gradient, blend };
        }
    }
}

pub(crate) fn remove_fill(data: &mut NodeData, index: usize) {
    match data {
        NodeData::Vector(vector) => {
            if index < vector.fills.len() {
                vector.fills.remove(index);
            }
        }
        NodeData::Group(group) => {
            if group.background.is_some() {
                if index == 0 {
                    group.background = None;
                } else if index - 1 < group.background_fills.len() {
                    group.background_fills.remove(index - 1);
                }
            } else if index < group.background_fills.len() {
                group.background_fills.remove(index);
            }
        }
        _ => {}
    }
}

pub(crate) fn add_fill(data: &mut NodeData) {
    match data {
        NodeData::Vector(vector) => vector.fills.push(Fill::solid(DEFAULT_FILL_COLOR)),
        NodeData::Group(group) => {
            if group.background.is_none() {
                group.background = Some(Fill::solid(DEFAULT_FILL_COLOR));
            } else {
                group.background_fills.push(Fill::solid(DEFAULT_FILL_COLOR));
            }
        }
        _ => {}
    }
}

pub(crate) fn default_shadow() -> Shadow {
    Shadow {
        kind: ShadowKind::Drop,
        color: FantaColor::rgba(0, 0, 0, 64),
        blur: 4.0,
        spread: 0.0,
        offset: [0.0, 2.0],
        // Figma's default: the shadow is knocked out under the node's body.
        show_behind_node: false,
    }
}

/// Figma's default blur radius for a freshly added layer / background blur.
pub(crate) fn default_blur(kind: BlurKind) -> Blur {
    Blur { kind, radius: 4.0 }
}

// =============================================================================
// Formatting and parsing
// =============================================================================

pub(crate) fn format_number(value: f64) -> String {
    if !value.is_finite() {
        return "0".to_string();
    }
    let rounded = (value * 100.0).round() / 100.0;
    if rounded.fract() == 0.0 {
        format!("{rounded:.0}")
    } else {
        format!("{rounded}")
    }
}

pub(crate) fn parse_number(text: &str) -> Option<f64> {
    let cleaned = text.trim().trim_end_matches("px").trim();
    cleaned
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite())
}

pub(crate) fn parse_color(text: &str) -> Option<FantaColor> {
    let trimmed = text.trim();
    if let Some(color) = FantaColor::from_hex(trimmed) {
        return Some(color);
    }
    FantaColor::from_hex(&format!("#{trimmed}"))
}

/// The dropdown label for a numeric OpenType weight. Values between the standard
/// 100-step stops retain their exact number and are never snapped onto a stop.
pub(crate) fn font_weight_label(weight: u16) -> gpui::SharedString {
    FONT_WEIGHTS
        .iter()
        .find(|(value, _)| *value == weight)
        .map(|(_, label)| gpui::SharedString::from(*label))
        .unwrap_or_else(|| format!("Custom · {weight}").into())
}

pub(crate) fn fanta_color_rgba(color: FantaColor) -> Rgba {
    Rgba {
        r: f32::from(color.r) / 255.0,
        g: f32::from(color.g) / 255.0,
        b: f32::from(color.b) / 255.0,
        a: f32::from(color.a) / 255.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use fanta_doc::{ComponentDef, ComponentPropKind, ImageFitMode, InstanceNode};

    use crate::color_picker::GradientKind;
    use crate::properties_snapshot::tests::{frame_group, text_node, vector_with_fill};
    use crate::properties_snapshot::{
        HiddenPaintAlpha, opaque_paint_alpha, paint_alpha, paint_is_visible, set_paint_alpha,
        zeroed_paint_alpha,
    };

    #[test]
    fn font_weight_dropdown_labels_the_ladder_and_keeps_custom_weights() {
        assert_eq!(font_weight_label(100), "Thin");
        assert_eq!(font_weight_label(200), "Extra Light");
        assert_eq!(font_weight_label(300), "Light");
        assert_eq!(font_weight_label(400), "Regular");
        assert_eq!(font_weight_label(500), "Medium");
        assert_eq!(font_weight_label(600), "Semi Bold");
        assert_eq!(font_weight_label(700), "Bold");
        assert_eq!(font_weight_label(800), "Extra Bold");
        assert_eq!(font_weight_label(900), "Black");
        // Off-ladder weights retain their exact numeric value in the trigger.
        assert_eq!(font_weight_label(350), "Custom · 350");
        // Every option the dropdown offers must round-trip through the label.
        for (weight, label) in FONT_WEIGHTS {
            assert_eq!(font_weight_label(weight), label);
        }
    }

    #[test]
    fn explicit_line_height_edit_clears_imported_metric_relative_override() {
        let mut text = text_node();
        text.style.line_height = 1.1;
        text.style.line_height_auto_percent = Some(100.0);
        let node = CanvasNode::new(NodeData::Text(text));
        let id = node.id;
        let mut doc = Doc::new();
        doc.scene.insert(node).expect("insert text node");

        for operation in field_operations(&doc, &InspectorField::LineHeight(id), "1.75") {
            doc.apply(operation).expect("apply line-height edit");
        }

        let NodeData::Text(text) = &doc.scene.get(id).expect("text node remains").data else {
            panic!("expected text node");
        };
        assert_eq!(text.style.line_height, 1.75);
        assert_eq!(text.style.line_height_auto_percent, None);
    }

    fn fill_at(data: &NodeData, index: usize) -> Option<&Fill> {
        match data {
            NodeData::Vector(vector) => vector.fills.get(index),
            _ => None,
        }
    }

    #[test]
    fn convert_solid_to_gradient_seeds_two_stops_from_color() {
        let color = FantaColor::rgb(0x11, 0x22, 0x33);
        let mut data = vector_with_fill(Fill::solid(color));
        convert_paint_kind(
            &mut data,
            0,
            false,
            PaintKind::Gradient(GradientKind::Linear),
        );
        let Some(Fill::Gradient { gradient, .. }) = fill_at(&data, 0) else {
            panic!("fill should be a gradient after conversion");
        };
        let stops = crate::color_picker::gradient_stops(gradient);
        assert_eq!(stops.len(), 2);
        assert_eq!(stops[0].color, color);
        assert_eq!(stops[1].color, FantaColor::rgba(0x11, 0x22, 0x33, 0));
        assert_eq!(GradientKind::of(gradient), GradientKind::Linear);
    }

    #[test]
    fn convert_gradient_to_solid_flattens_to_first_stop() {
        let mut data = vector_with_fill(Fill::solid(FantaColor::rgb(0x40, 0x80, 0xC0)));
        // Solid → linear gradient, then back to solid keeps the base color.
        convert_paint_kind(
            &mut data,
            0,
            false,
            PaintKind::Gradient(GradientKind::Linear),
        );
        convert_paint_kind(&mut data, 0, false, PaintKind::Solid);
        assert_eq!(
            fill_at(&data, 0),
            Some(&Fill::solid(FantaColor::rgb(0x40, 0x80, 0xC0)))
        );
    }

    #[test]
    fn convert_between_gradient_kinds_preserves_stops() {
        let mut data = vector_with_fill(Fill::solid(FantaColor::rgb(1, 2, 3)));
        convert_paint_kind(
            &mut data,
            0,
            false,
            PaintKind::Gradient(GradientKind::Linear),
        );
        let stops_before = match fill_at(&data, 0) {
            Some(Fill::Gradient { gradient, .. }) => {
                crate::color_picker::gradient_stops(gradient).to_vec()
            }
            _ => panic!("expected gradient"),
        };
        convert_paint_kind(
            &mut data,
            0,
            false,
            PaintKind::Gradient(GradientKind::Radial),
        );
        let Some(Fill::Gradient { gradient, .. }) = fill_at(&data, 0) else {
            panic!("expected gradient");
        };
        assert_eq!(GradientKind::of(gradient), GradientKind::Radial);
        assert_eq!(
            crate::color_picker::gradient_stops(gradient),
            stops_before.as_slice()
        );
    }

    #[test]
    fn set_paint_gradient_preserves_per_paint_blend() {
        let base = Gradient::Linear {
            start: [0.0, 0.0],
            end: [1.0, 0.0],
            stops: vec![
                fanta_doc::GradientStop {
                    position: 0.0,
                    color: FantaColor::BLACK,
                },
                fanta_doc::GradientStop {
                    position: 1.0,
                    color: FantaColor::WHITE,
                },
            ],
        };
        let mut data = vector_with_fill(Fill::Gradient {
            gradient: base.clone(),
            blend: BlendMode::Multiply,
        });
        let radial = crate::color_picker::convert_gradient_kind(&base, GradientKind::Radial);
        set_paint_gradient(&mut data, 0, false, radial.clone());
        assert_eq!(
            fill_at(&data, 0),
            Some(&Fill::Gradient {
                gradient: radial,
                blend: BlendMode::Multiply,
            })
        );
    }

    // === Corner smoothing ================================================

    #[test]
    fn corner_smoothing_writes_the_fraction_and_clamps() {
        let mut data = vector_with_fill(Fill::solid(FantaColor::BLACK));
        set_corner_smoothing(&mut data, 0.6);
        let NodeData::Vector(vector) = &data else {
            panic!("expected a vector");
        };
        assert!((vector.corner_smoothing - 0.6).abs() < 1e-6);

        set_corner_smoothing(&mut data, 4.0);
        let NodeData::Vector(vector) = &data else {
            panic!("expected a vector");
        };
        assert_eq!(vector.corner_smoothing, 1.0);

        let mut group = NodeData::Group(GroupNode::default());
        set_corner_smoothing(&mut group, -1.0);
        let NodeData::Group(group) = &group else {
            panic!("expected a group");
        };
        assert_eq!(group.corner_smoothing, 0.0);

        // Kinds without corners are untouched.
        let mut text = NodeData::Text(text_node());
        let before = text.clone();
        set_corner_smoothing(&mut text, 1.0);
        assert_eq!(text, before);
    }

    // === Blur op builder =================================================

    /// A one-node doc, so the op builders can be exercised against a real `Doc`.
    fn doc_with_node(data: NodeData) -> (Doc, NodeId) {
        let mut doc = Doc::new();
        let node = CanvasNode::new(data);
        let id = node.id;
        doc.scene.insert(node).expect("inserting the test node");
        (doc, id)
    }

    fn blurs_of(doc: &Doc, id: NodeId) -> Vec<Blur> {
        doc.scene
            .get(id)
            .map(|node| node.blurs.to_vec())
            .unwrap_or_default()
    }

    fn text_data(doc: &Doc, id: NodeId) -> &fanta_doc::TextNode {
        let node = doc.scene.get(id).expect("the test text node");
        let NodeData::Text(text) = &node.data else {
            panic!("expected text data");
        };
        text
    }

    #[test]
    fn width_field_resizes_and_reflows_the_text_box_without_scaling_glyphs() {
        use fanta_text::{LayoutEngine, TextBuffer, TextStyle};

        let content = "one two three four five six seven eight";
        let mut text = fanta_doc::TextNode::new(content, 160.0, 90.0);
        text.auto_resize = TextAutoResize::WidthAndHeight;
        let (mut doc, id) = doc_with_node(NodeData::Text(text));
        let angle: f64 = 0.4;
        doc.scene.get_mut(id).expect("the text node").transform = Transform2D::from_components([
            angle.cos(),
            angle.sin(),
            -angle.sin(),
            angle.cos(),
            24.0,
            12.0,
        ]);
        let original_transform = doc.scene.get(id).expect("the text node").transform;
        let buffer = TextBuffer::from_str(content, TextStyle::default());
        let layout_engine = LayoutEngine::new();
        let original_lines = layout_engine.layout(&buffer, 160.0).line_count();

        let operations = field_operations(&doc, &InspectorField::Width(id), "70");
        assert!(
            operations
                .iter()
                .any(|operation| matches!(operation, Operation::ReplaceData { .. })),
            "a text width edit must replace its box data"
        );
        for operation in operations {
            doc.apply(operation).expect("applying the text width edit");
        }

        let node = doc.scene.get(id).expect("the resized text node");
        assert_eq!(
            &node.transform.to_components()[..4],
            &original_transform.to_components()[..4],
            "the glyph transform's linear part must stay unchanged"
        );
        let text = text_data(&doc, id);
        assert!((text.local_size[0] - 70.0).abs() < 1e-6);
        assert!((text.local_size[1] - 90.0).abs() < 1e-6);
        assert_eq!(text.auto_resize, TextAutoResize::Height);
        assert_eq!(text.style.size_px, 16.0);
        assert!(
            layout_engine
                .layout(&buffer, text.local_size[0])
                .line_count()
                > original_lines,
            "the narrower fixed-width box must produce more wrapped lines"
        );
    }

    #[test]
    fn height_field_fixes_the_text_box_without_scaling_glyphs() {
        let mut text = fanta_doc::TextNode::new("Hello", 100.0, 24.0);
        text.auto_resize = TextAutoResize::Height;
        let (mut doc, id) = doc_with_node(NodeData::Text(text));
        let original_transform = doc.scene.get(id).expect("the text node").transform;

        for operation in field_operations(&doc, &InspectorField::Height(id), "60") {
            doc.apply(operation).expect("applying the text height edit");
        }

        let node = doc.scene.get(id).expect("the resized text node");
        assert_eq!(
            &node.transform.to_components()[..4],
            &original_transform.to_components()[..4],
            "the glyph transform's linear part must stay unchanged"
        );
        let text = text_data(&doc, id);
        assert_eq!(text.local_size, [100.0, 60.0]);
        assert_eq!(text.auto_resize, TextAutoResize::None);
    }

    #[test]
    fn frame_width_field_resizes_box_without_scaling_child_text() {
        let mut doc = Doc::new();
        let mut frame = CanvasNode::new(NodeData::Group(GroupNode {
            clip_size: Some([120.0, 80.0]),
            ..Default::default()
        }));
        frame.transform = Transform2D::translation(20.0, 30.0);
        let frame_id = frame.id;
        doc.scene.insert(frame).unwrap();
        let mut text = CanvasNode::new(NodeData::Text(fanta_doc::TextNode::new(
            "Hello", 80.0, 24.0,
        )));
        text.parent = Some(frame_id);
        text.transform = Transform2D::translation(12.0, 14.0);
        let text_id = text.id;
        doc.scene.insert(text).unwrap();
        let child_world_before = doc.scene.world_transform(text_id).unwrap();

        let operations = field_operations(&doc, &InspectorField::Width(frame_id), "240");
        assert!(operations.iter().any(|operation| {
            matches!(operation, Operation::ReplaceData { id, .. } if *id == frame_id)
        }));
        for operation in operations {
            doc.apply(operation).unwrap();
        }

        let frame = doc.scene.get(frame_id).unwrap();
        let NodeData::Group(group) = &frame.data else {
            panic!("expected frame");
        };
        assert_eq!(group.clip_size, Some([240.0, 80.0]));
        assert_eq!(&frame.transform.to_components()[..4], &[1.0, 0.0, 0.0, 1.0]);
        assert_eq!(
            doc.scene.world_transform(text_id).unwrap(),
            child_world_before
        );
    }

    #[test]
    fn plain_group_width_uses_its_box_even_when_a_child_overflows() {
        let mut doc = Doc::new();
        let group = CanvasNode::new(NodeData::Group(GroupNode {
            local_size: Some([100.0, 80.0]),
            ..Default::default()
        }));
        let group_id = group.id;
        doc.scene.insert(group).unwrap();
        let mut child = CanvasNode::new(NodeData::Vector(fanta_doc::VectorNode::rect_solid(
            0.0,
            0.0,
            20.0,
            20.0,
            FantaColor::BLACK,
        )));
        child.parent = Some(group_id);
        child.transform = Transform2D::translation(150.0, 10.0);
        let child_transform = child.transform;
        doc.scene.insert(child).unwrap();

        for operation in resize_operations(&doc, group_id, 120.0, true) {
            doc.apply(operation).unwrap();
        }
        let NodeData::Group(group) = &doc.scene.get(group_id).unwrap().data else {
            panic!("expected group");
        };
        assert_eq!(group.local_size, Some([120.0, 80.0]));
        assert_eq!(doc.scene.children_of(Some(group_id)).len(), 1);
        assert_eq!(
            doc.scene
                .get(doc.scene.children_of(Some(group_id))[0])
                .unwrap()
                .transform,
            child_transform
        );
    }

    #[test]
    fn legacy_plain_group_width_materializes_box_without_moving_children() {
        let mut doc = Doc::new();
        let group = CanvasNode::new(NodeData::Group(GroupNode::default()));
        let group_id = group.id;
        doc.scene.insert(group).unwrap();
        let mut child = CanvasNode::new(NodeData::Text(fanta_doc::TextNode::new(
            "Legacy", 40.0, 20.0,
        )));
        child.parent = Some(group_id);
        child.transform = Transform2D::translation(20.0, 30.0);
        let child_id = child.id;
        doc.scene.insert(child).unwrap();
        let child_world_before = doc.scene.world_transform(child_id).unwrap();

        for operation in resize_operations(&doc, group_id, 80.0, true) {
            doc.apply(operation).unwrap();
        }
        let NodeData::Group(group) = &doc.scene.get(group_id).unwrap().data else {
            panic!("expected group");
        };
        assert_eq!(group.local_size, Some([80.0, 20.0]));
        assert_eq!(group.clip_size, None);
        assert_eq!(
            doc.scene.world_transform(child_id).unwrap(),
            child_world_before
        );
    }

    #[test]
    fn rotation_field_composes_rigidly_in_world_space_under_scaled_parent() {
        let mut doc = Doc::new();
        let mut parent = CanvasNode::new(NodeData::Group(GroupNode {
            clip_size: Some([300.0, 200.0]),
            ..Default::default()
        }));
        parent.transform = Transform2D::from_components([2.0, 0.0, 0.0, 0.75, 10.0, 20.0]);
        let parent_id = parent.id;
        doc.scene.insert(parent).unwrap();
        let mut child = CanvasNode::new(NodeData::Vector(fanta_doc::VectorNode::rect_solid(
            0.0,
            0.0,
            60.0,
            40.0,
            FantaColor::BLACK,
        )));
        child.parent = Some(parent_id);
        child.transform = Transform2D::translation(30.0, 40.0);
        let child_id = child.id;
        doc.scene.insert(child).unwrap();
        let before = doc.scene.world_transform(child_id).unwrap();
        let pivot = before.transform_point(DVec2::new(30.0, 20.0));

        for operation in rotation_operations(&doc, child_id, 90.0) {
            doc.apply(operation).unwrap();
        }
        let after = doc.scene.world_transform(child_id).unwrap();
        let expected = before.then(&rotate_about(pivot, std::f64::consts::FRAC_PI_2));
        for (actual, expected) in after.to_components().iter().zip(expected.to_components()) {
            assert!((actual - expected).abs() < 1e-9);
        }
    }

    #[test]
    fn rotation_field_displays_and_sets_world_angle_under_nonuniform_scale() {
        let mut doc = Doc::new();
        let mut parent = CanvasNode::new(NodeData::Group(GroupNode {
            clip_size: Some([300.0, 200.0]),
            ..Default::default()
        }));
        parent.transform = Transform2D::from_components([2.0, 0.0, 0.0, 0.5, 10.0, 20.0]);
        let parent_id = parent.id;
        doc.scene.insert(parent).unwrap();
        let mut child = CanvasNode::new(NodeData::Vector(fanta_doc::VectorNode::rect_solid(
            0.0,
            0.0,
            60.0,
            40.0,
            FantaColor::BLACK,
        )));
        child.parent = Some(parent_id);
        child.transform = Transform2D::rotation(std::f64::consts::FRAC_PI_4)
            .then(&Transform2D::translation(30.0, 40.0));
        let child_id = child.id;
        doc.scene.insert(child).unwrap();

        let initial_world = doc.scene.world_transform(child_id).unwrap();
        let initial_world_degrees = transform_angle(&initial_world).to_degrees();
        let displayed = read_field_text(&doc, &InspectorField::Rotation(child_id))
            .expect("rotation text")
            .parse::<f64>()
            .expect("numeric rotation");
        assert!((displayed - initial_world_degrees).abs() < 0.01);
        assert!(
            (displayed - 45.0).abs() > 1.0,
            "the parent scale skews the local angle"
        );

        for operation in rotation_operations(&doc, child_id, 45.0) {
            doc.apply(operation).unwrap();
        }
        let world = doc.scene.world_transform(child_id).unwrap();
        assert!((transform_angle(&world).to_degrees() - 45.0).abs() < 1e-9);
    }

    #[test]
    fn outlined_text_keeps_vector_transform_resize_semantics() {
        let vector = fanta_doc::VectorNode::rect_solid(0.0, 0.0, 100.0, 40.0, FantaColor::BLACK);
        let (doc, id) = doc_with_node(NodeData::Vector(vector));

        let operations = field_operations(&doc, &InspectorField::Width(id), "200");
        assert!(
            operations
                .iter()
                .any(|operation| matches!(operation, Operation::SetTransform { .. }))
        );
        assert!(
            operations
                .iter()
                .all(|operation| !matches!(operation, Operation::ReplaceData { .. })),
            "vector outlines should remain normally distortable"
        );
    }

    #[test]
    fn preview_writes_inside_a_master_bump_its_revision() {
        let mut doc = Doc::new();
        let master = CanvasNode::new(NodeData::Group(frame_group()));
        let master_id = master.id;
        doc.scene.insert(master).expect("inserting the master");
        let mut child = CanvasNode::new(NodeData::Text(text_node()));
        child.parent = Some(master_id);
        let child_id = child.id;
        doc.scene.insert(child).expect("inserting the master's text");
        let component = fanta_doc::ComponentId::new();
        doc.components
            .defs
            .insert(component, ComponentDef::new(component, master_id, "Card"));
        let rev = |doc: &Doc| doc.components.defs[&component].rev;

        let node = doc.scene.get(child_id).expect("the text node");
        let snapshot = NodeSnapshot {
            id: child_id,
            transform: node.transform,
            opacity: node.opacity,
            data: Box::new(node.data.clone()),
            effects: node.effects.clone(),
            blurs: node.blurs.clone(),
        };

        let before = rev(&doc);
        for operation in field_operations(&doc, &InspectorField::Width(child_id), "72") {
            apply_preview_operation(&mut doc, &operation);
        }
        let previewed = rev(&doc);
        assert!(
            previewed > before,
            "a scrub frame inside a master must re-key its instances"
        );
        restore_snapshot(&mut doc, &snapshot);
        assert!(rev(&doc) > previewed, "so must restoring the baseline");
    }

    #[test]
    fn text_width_scrub_previews_live_and_commits_as_one_undo_step() {
        let mut text = fanta_doc::TextNode::new("one two three four", 120.0, 48.0);
        text.auto_resize = TextAutoResize::WidthAndHeight;
        let (mut doc, id) = doc_with_node(NodeData::Text(text));
        let node = doc.scene.get(id).expect("the text node");
        let snapshot = NodeSnapshot {
            id,
            transform: node.transform,
            opacity: node.opacity,
            data: Box::new(node.data.clone()),
            effects: node.effects.clone(),
            blurs: node.blurs.clone(),
        };

        for operation in field_operations(&doc, &InspectorField::Width(id), "72") {
            apply_preview_operation(&mut doc, &operation);
        }
        assert_eq!(text_data(&doc, id).local_size, [72.0, 48.0]);
        assert_eq!(
            doc.history.undo_depth(),
            0,
            "a scrub preview must stay out of history"
        );

        restore_snapshot(&mut doc, &snapshot);
        assert_eq!(text_data(&doc, id).local_size, [120.0, 48.0]);
        let operations = field_operations(&doc, &InspectorField::Width(id), "84");
        doc.history.begin("Resize", &mut doc.scene);
        for operation in operations {
            doc.apply(operation)
                .expect("committing the text width scrub");
        }
        doc.history.commit(&mut doc.scene);
        assert_eq!(doc.history.undo_depth(), 1);
        assert_eq!(text_data(&doc, id).local_size, [84.0, 48.0]);

        assert!(doc.undo().expect("undoing the text width scrub"));
        assert_eq!(text_data(&doc, id).local_size, [120.0, 48.0]);
        assert_eq!(
            text_data(&doc, id).auto_resize,
            TextAutoResize::WidthAndHeight
        );
        assert!(!doc.undo().expect("checking for a second undo step"));
    }

    #[test]
    fn blurs_operations_emits_one_set_blurs_carrying_the_prior_stack() {
        let (doc, id) = doc_with_node(vector_with_fill(Fill::solid(FantaColor::BLACK)));
        let operations =
            blurs_operations(&doc, id, |blurs| blurs.push(default_blur(BlurKind::Layer)));
        assert_eq!(operations.len(), 1);
        let Some(Operation::SetBlurs { old, new, .. }) = operations.first() else {
            panic!("expected a SetBlurs operation");
        };
        assert!(old.is_empty(), "undo must restore the empty stack");
        assert_eq!(new.len(), 1);
        assert_eq!(new[0].kind, BlurKind::Layer);
        assert_eq!(new[0].radius, 4.0);
    }

    #[test]
    fn blurs_operations_is_a_no_op_when_nothing_changed() {
        let (doc, id) = doc_with_node(vector_with_fill(Fill::solid(FantaColor::BLACK)));
        assert!(blurs_operations(&doc, id, |_| {}).is_empty());
        // Removing past the end changes nothing either.
        assert!(
            blurs_operations(&doc, id, |blurs| {
                if 3 < blurs.len() {
                    blurs.remove(3);
                }
            })
            .is_empty()
        );
        assert!(
            blurs_operations(&doc, NodeId::new(), |blurs| blurs
                .push(default_blur(BlurKind::Layer)))
            .is_empty()
        );
    }

    #[test]
    fn blur_radius_commits_and_previews_through_set_blurs() {
        let (mut doc, id) = doc_with_node(vector_with_fill(Fill::solid(FantaColor::BLACK)));
        for operation in blurs_operations(&doc, id, |blurs| {
            blurs.push(default_blur(BlurKind::Background))
        }) {
            doc.apply(operation).expect("adding a blur");
        }
        assert_eq!(blurs_of(&doc, id).len(), 1);

        let operations = field_operations(&doc, &InspectorField::BlurRadius { id, index: 0 }, "12");
        assert_eq!(operations.len(), 1);
        // The scrub preview path must understand the op it will produce.
        for operation in &operations {
            apply_preview_operation(&mut doc, operation);
        }
        assert_eq!(blurs_of(&doc, id)[0].radius, 12.0);
        assert_eq!(blurs_of(&doc, id)[0].kind, BlurKind::Background);

        // Negative radii clamp rather than poisoning the renderer.
        let operations = field_operations(&doc, &InspectorField::BlurRadius { id, index: 0 }, "-5");
        for operation in &operations {
            apply_preview_operation(&mut doc, operation);
        }
        assert_eq!(blurs_of(&doc, id)[0].radius, 0.0);
    }

    #[test]
    fn a_blur_snapshot_restore_rolls_back_the_stack() {
        let (mut doc, id) = doc_with_node(vector_with_fill(Fill::solid(FantaColor::BLACK)));
        let snapshot = {
            let node = doc.scene.get(id).expect("the test node");
            NodeSnapshot {
                id,
                transform: node.transform,
                opacity: node.opacity,
                data: Box::new(node.data.clone()),
                effects: node.effects.clone(),
                blurs: node.blurs.clone(),
            }
        };
        for operation in
            blurs_operations(&doc, id, |blurs| blurs.push(default_blur(BlurKind::Layer)))
        {
            doc.apply(operation).expect("adding a blur");
        }
        assert_eq!(blurs_of(&doc, id).len(), 1);
        restore_snapshot(&mut doc, &snapshot);
        assert!(blurs_of(&doc, id).is_empty());
    }

    // === Selection colors ================================================

    #[test]
    fn selection_color_edit_replaces_the_color_on_every_selected_node() {
        let from = FantaColor::rgb(10, 20, 30);
        let to = FantaColor::rgb(40, 50, 60);
        let mut doc = Doc::new();
        let mut ids = Vec::new();
        for _ in 0..2 {
            let node = CanvasNode::new(vector_with_fill(Fill::solid(from)));
            ids.push(node.id);
            doc.scene.insert(node).expect("inserting a test node");
        }
        // A third node that does not use the color must not be touched.
        let untouched = CanvasNode::new(vector_with_fill(Fill::solid(FantaColor::WHITE)));
        let untouched_id = untouched.id;
        doc.scene.insert(untouched).expect("inserting a test node");
        doc.selection
            .replace_with(ids.iter().copied().chain([untouched_id]));

        let operations = selection_color_operations(&doc, from, to);
        assert_eq!(
            operations.len(),
            2,
            "only the nodes carrying the color author an op"
        );
        for operation in operations {
            doc.apply(operation).expect("replacing a selection color");
        }
        for id in ids {
            let node = doc.scene.get(id).expect("the test node");
            assert_eq!(fill_at(&node.data, 0), Some(&Fill::solid(to)));
        }
        let node = doc.scene.get(untouched_id).expect("the untouched node");
        assert_eq!(
            fill_at(&node.data, 0),
            Some(&Fill::solid(FantaColor::WHITE))
        );
    }

    // === Per-paint visibility (non-destructive) ==========================

    #[test]
    fn hiding_and_showing_a_paint_round_trips_partial_alpha() {
        let translucent = FantaColor::rgba(10, 20, 30, 128);
        let mut data = vector_with_fill(Fill::solid(translucent));
        let fill = fill_at(&data, 0).expect("a fill").clone();
        assert!(paint_is_visible(&fill));

        let remembered = paint_alpha(&fill);
        assert_eq!(remembered, HiddenPaintAlpha::Solid(128));

        // Hide.
        if let Some(paint) = paint_slot_mut(&mut data, 0, false) {
            let zeroed = zeroed_paint_alpha(paint);
            set_paint_alpha(paint, &zeroed);
        }
        assert!(!paint_is_visible(fill_at(&data, 0).expect("a fill")));

        // Show, restoring the remembered alpha rather than forcing 255.
        if let Some(paint) = paint_slot_mut(&mut data, 0, false) {
            set_paint_alpha(paint, &remembered);
        }
        assert_eq!(fill_at(&data, 0), Some(&Fill::solid(translucent)));
    }

    #[test]
    fn gradient_and_image_paints_can_be_hidden_too() {
        let gradient = Gradient::Linear {
            start: [0.0, 0.0],
            end: [1.0, 0.0],
            stops: vec![
                fanta_doc::GradientStop {
                    position: 0.0,
                    color: FantaColor::rgba(0, 0, 0, 200),
                },
                fanta_doc::GradientStop {
                    position: 1.0,
                    color: FantaColor::rgba(255, 255, 255, 90),
                },
            ],
        };
        let mut data = vector_with_fill(Fill::Gradient {
            gradient,
            blend: BlendMode::Normal,
        });
        let remembered = paint_alpha(fill_at(&data, 0).expect("a fill"));
        assert_eq!(remembered, HiddenPaintAlpha::Gradient(vec![200, 90]));

        if let Some(paint) = paint_slot_mut(&mut data, 0, false) {
            let zeroed = zeroed_paint_alpha(paint);
            set_paint_alpha(paint, &zeroed);
        }
        assert!(!paint_is_visible(fill_at(&data, 0).expect("a fill")));
        if let Some(paint) = paint_slot_mut(&mut data, 0, false) {
            set_paint_alpha(paint, &remembered);
        }
        assert_eq!(
            paint_alpha(fill_at(&data, 0).expect("a fill")),
            HiddenPaintAlpha::Gradient(vec![200, 90])
        );

        let mut image = vector_with_fill(Fill::Image {
            asset: fanta_doc::AssetId::new(),
            mode: ImageFitMode::Fill,
            opacity: 0.4,
            crop: None,
            scale: None,
            rotation: None,
            blend: BlendMode::Normal,
            adjust: fanta_doc::ImageAdjust::default(),
        });
        let remembered = paint_alpha(fill_at(&image, 0).expect("a fill"));
        assert_eq!(remembered, HiddenPaintAlpha::Image(0.4));
        if let Some(paint) = paint_slot_mut(&mut image, 0, false) {
            let zeroed = zeroed_paint_alpha(paint);
            set_paint_alpha(paint, &zeroed);
        }
        assert!(!paint_is_visible(fill_at(&image, 0).expect("a fill")));
        if let Some(paint) = paint_slot_mut(&mut image, 0, false) {
            set_paint_alpha(paint, &remembered);
        }
        assert_eq!(
            paint_alpha(fill_at(&image, 0).expect("a fill")),
            HiddenPaintAlpha::Image(0.4)
        );
    }

    #[test]
    fn showing_a_paint_with_no_remembered_alpha_falls_back_to_opaque() {
        assert_eq!(
            opaque_paint_alpha(&HiddenPaintAlpha::Solid(0)),
            HiddenPaintAlpha::Solid(255)
        );
        assert_eq!(
            opaque_paint_alpha(&HiddenPaintAlpha::Gradient(vec![0, 0, 0])),
            HiddenPaintAlpha::Gradient(vec![255, 255, 255])
        );
        assert_eq!(
            opaque_paint_alpha(&HiddenPaintAlpha::Image(0.0)),
            HiddenPaintAlpha::Image(1.0)
        );
    }

    // === Instances: props, detach, combine ===============================

    /// A doc holding a component master (a frame with one vector child) and one
    /// live instance of it. Returns the doc, the master's component id, the
    /// master root's node id and the instance's node id.
    fn doc_with_instance() -> (Doc, ComponentId, NodeId, NodeId) {
        let mut doc = Doc::new();
        let master_root = CanvasNode::new(NodeData::Group(frame_group()));
        let master_root_id = master_root.id;
        doc.scene.insert(master_root).expect("inserting the master");

        let mut child = CanvasNode::new(vector_with_fill(Fill::solid(FantaColor::BLACK)));
        child.parent = Some(master_root_id);
        let child_id = child.id;
        doc.scene.insert(child).expect("inserting the master child");

        let component = ComponentId::new();
        let mut def = ComponentDef::new(component, master_root_id, "Button");
        def.props.push(fanta_doc::ComponentPropDef {
            id: ComponentPropId::new(),
            name: "Radius".into(),
            kind: ComponentPropKind::Number,
            formatter: Default::default(),
            default: VarValue::Float { value: 4.0 },
            bindings: Vec::new(),
        });
        doc.components.defs.insert(component, def);

        let instance = CanvasNode::new(NodeData::Instance(InstanceNode {
            component,
            overrides: Vec::new(),
            prop_values: std::collections::BTreeMap::new(),
            derived: Vec::new(),
            local_size: [120.0, 60.0],
        }));
        let instance_id = instance.id;
        doc.scene.insert(instance).expect("inserting the instance");
        let _ = child_id;
        (doc, component, master_root_id, instance_id)
    }

    #[test]
    fn variant_dropdown_selects_the_exact_requested_member() {
        let mut doc = Doc::new();
        let set_id = ComponentId::new();
        let mut members = Vec::new();
        for value in ["Small", "Large"] {
            let root = CanvasNode::new(NodeData::Group(frame_group()));
            let root_id = root.id;
            doc.scene.insert(root).expect("inserting a variant root");
            let component_id = ComponentId::new();
            let mut definition = ComponentDef::new(component_id, root_id, value);
            definition.variant_of = Some(ComponentSetMembership {
                set: set_id,
                axis_values: std::collections::BTreeMap::from([(
                    "Size".to_string(),
                    value.to_string(),
                )]),
            });
            doc.components.defs.insert(component_id, definition);
            members.push(component_id);
        }
        let small = members.first().copied().expect("the small member");
        let large = members.get(1).copied().expect("the large member");
        doc.components.sets.insert(
            set_id,
            ComponentSet {
                id: set_id,
                name: "Button".to_string(),
                axes: vec![VariantAxis {
                    name: "Size".to_string(),
                    values: vec!["Small".to_string(), "Large".to_string()],
                }],
                members: members.clone(),
                default_variant: small,
            },
        );
        let instance = CanvasNode::new(NodeData::Instance(InstanceNode {
            component: small,
            overrides: Vec::new(),
            prop_values: std::collections::BTreeMap::new(),
            derived: Vec::new(),
            local_size: [120.0, 60.0],
        }));
        let instance_id = instance.id;
        doc.scene.insert(instance).expect("inserting an instance");

        let operations = variant_select_operations(&doc, instance_id, "Size", "Large");
        assert_eq!(operations.len(), 1);
        assert!(matches!(
            operations.first(),
            Some(Operation::SwapInstance { old, new, .. })
                if *old == small && *new == large
        ));
        assert!(variant_select_operations(&doc, instance_id, "Size", "Small").is_empty());
        assert!(variant_select_operations(&doc, instance_id, "Size", "Unknown").is_empty());
    }

    /// The panel's per-instance "Content" row: editing it writes a text
    /// override on the instance (never the master), and the row re-reads the
    /// resolved content back.
    #[test]
    fn instance_text_field_round_trips_through_an_override() {
        let (mut doc, _, master_root_id, instance_id) = doc_with_instance();
        let mut label = CanvasNode::new(NodeData::Text(text_node()));
        label.parent = Some(master_root_id);
        let label_id = label.id;
        doc.scene.insert(label).expect("inserting the master label");

        let clones = crate::instance_text::text_clones(&doc, instance_id);
        assert_eq!(clones.len(), 1);
        let path = clones[0].0.clone();

        let field = InspectorField::InstanceText {
            id: instance_id,
            path,
        };
        let operations = field_operations(&doc, &field, "FROM PANEL");
        assert!(!operations.is_empty(), "content edit builds an override op");
        for operation in operations {
            doc.apply(operation).expect("applying the override");
        }
        assert_eq!(
            read_field_text(&doc, &field).as_deref(),
            Some("FROM PANEL"),
            "the row re-reads the resolved override"
        );
        // The master text is untouched.
        let NodeData::Text(master_text) = &doc.scene.get(label_id).unwrap().data else {
            panic!("expected the master label");
        };
        assert_ne!(master_text.content, "FROM PANEL");
    }

    #[test]
    fn instance_number_and_color_props_commit_through_set_instance_prop() {
        let (mut doc, component, _, instance_id) = doc_with_instance();
        let prop = doc.components.def(component).unwrap().props[0].id;

        let field = InspectorField::InstanceNumberProp {
            id: instance_id,
            prop,
        };
        let operations = field_operations(&doc, &field, "12.5");
        assert_eq!(operations.len(), 1);
        let Some(Operation::SetInstanceProp { old, new, .. }) = operations.first() else {
            panic!("expected a SetInstanceProp operation");
        };
        assert_eq!(*old, None, "undo must clear back to the def default");
        assert_eq!(*new, Some(VarValue::Float { value: 12.5 }));

        // The scrub preview path must understand the op it will produce.
        for operation in &operations {
            apply_preview_operation(&mut doc, operation);
        }
        let NodeData::Instance(instance) = &doc.scene.get(instance_id).unwrap().data else {
            panic!("expected an instance");
        };
        assert_eq!(
            instance.prop_values.get(&prop),
            Some(&VarValue::Float { value: 12.5 })
        );

        // Re-committing the same value authors nothing.
        assert!(field_operations(&doc, &field, "12.5").is_empty());

        let field = InspectorField::InstanceColorProp {
            id: instance_id,
            prop,
        };
        let operations = field_operations(&doc, &field, "#204080");
        let Some(Operation::SetInstanceProp { new, .. }) = operations.first() else {
            panic!("expected a SetInstanceProp operation");
        };
        assert_eq!(
            *new,
            Some(VarValue::Color {
                value: FantaColor::rgb(0x20, 0x40, 0x80)
            })
        );
    }

    #[test]
    fn detach_instance_materializes_the_master_subtree_in_one_step() {
        let (mut doc, _, master_root_id, instance_id) = doc_with_instance();
        let operations = detach_instance_operations(&doc, instance_id);
        let Some(Operation::DetachInstance {
            id, new, expanded, ..
        }) = operations.first()
        else {
            panic!("expected a DetachInstance operation");
        };
        assert_eq!(*id, instance_id);
        // The instance node becomes the resolved master-root frame.
        assert!(matches!(**new, NodeData::Group(_)));
        // The master's one child is materialized, re-parented onto the node.
        assert_eq!(expanded.len(), 1);
        assert_eq!(expanded[0].parent, Some(instance_id));
        assert_ne!(
            expanded[0].id, master_root_id,
            "the expansion must use fresh ids, not the master's"
        );

        for operation in operations {
            doc.apply(operation).expect("detaching the instance");
        }
        let node = doc.scene.get(instance_id).expect("the detached node");
        assert!(matches!(node.data, NodeData::Group(_)));
        assert_eq!(doc.scene.children_of(Some(instance_id)).len(), 1);

        // Detaching something that is not an instance is a no-op.
        assert!(detach_instance_operations(&doc, master_root_id).is_empty());
        assert!(detach_instance_operations(&doc, NodeId::new()).is_empty());
    }

    #[test]
    fn create_component_promotes_a_frame_in_place_and_is_idempotent() {
        let (mut doc, id) = doc_with_node(NodeData::Group(frame_group()));
        doc.scene.get_mut(id).expect("the frame").name = "Card".to_owned();

        let operations = create_component_operations(&doc, id);
        assert_eq!(operations.len(), 1);
        let Some(Operation::DefineComponent { def }) = operations.first() else {
            panic!("expected a DefineComponent operation");
        };
        assert_eq!(def.root, id);
        assert_eq!(def.name, "Card");

        for operation in operations {
            doc.apply(operation).expect("promoting the frame");
        }
        assert!(
            doc.components.defs.values().any(|def| def.root == id),
            "the frame is now a master root"
        );

        // Promoting a node that already is a master is a no-op.
        assert!(create_component_operations(&doc, id).is_empty());
    }

    #[test]
    fn create_component_refuses_non_groups_and_names_unnamed_frames() {
        let (doc, id) = doc_with_node(NodeData::Text(text_node()));
        assert!(create_component_operations(&doc, id).is_empty());
        assert!(create_component_operations(&doc, NodeId::new()).is_empty());

        let (mut doc, id) = doc_with_node(NodeData::Group(frame_group()));
        doc.scene.get_mut(id).expect("the frame").name = "  ".to_owned();
        let operations = create_component_operations(&doc, id);
        let Some(Operation::DefineComponent { def }) = operations.first() else {
            panic!("expected a DefineComponent operation");
        };
        assert_eq!(def.name, "Component");
    }

    #[test]
    fn combine_as_variants_defines_a_set_and_joins_every_selected_master() {
        let mut doc = Doc::new();
        let mut components = Vec::new();
        for name in ["Default", "Hover"] {
            let root = CanvasNode::new(NodeData::Group(frame_group()));
            let root_id = root.id;
            doc.scene.insert(root).expect("inserting a master");
            let component = ComponentId::new();
            doc.components
                .defs
                .insert(component, ComponentDef::new(component, root_id, name));
            components.push((component, root_id));
        }
        doc.selection
            .replace_with(components.iter().map(|(_, root)| *root));

        let operations = combine_as_variants_operations(&doc);
        assert_eq!(operations.len(), 3, "one DefineComponentSet + two members");
        let Some(Operation::DefineComponentSet { set }) = operations.first() else {
            panic!("expected a DefineComponentSet operation");
        };
        assert_eq!(set.members.len(), 2);
        assert_eq!(set.default_variant, components[0].0);
        assert_eq!(set.axes.len(), 1);
        assert_eq!(set.axes[0].values, vec!["Default", "Hover"]);

        for operation in operations {
            doc.apply(operation).expect("combining as variants");
        }
        for (component, _) in &components {
            let membership = doc
                .components
                .def(*component)
                .expect("the master")
                .variant_of
                .as_ref()
                .expect("the master joined the set");
            assert_eq!(membership.axis_values.len(), 1);
        }

        // Masters already inside a set are skipped, so the button no-ops now.
        assert!(combine_as_variants_operations(&doc).is_empty());
    }

    #[test]
    fn combine_as_variants_needs_at_least_two_masters() {
        let mut doc = Doc::new();
        let root = CanvasNode::new(NodeData::Group(frame_group()));
        let root_id = root.id;
        doc.scene.insert(root).expect("inserting a master");
        let component = ComponentId::new();
        doc.components
            .defs
            .insert(component, ComponentDef::new(component, root_id, "Only"));
        doc.selection.select_only(root_id);
        assert!(combine_as_variants_operations(&doc).is_empty());
    }

    #[test]
    fn paint_opacity_commits_to_alpha_for_solids_and_opacity_for_images() {
        let (doc, id) = doc_with_node(vector_with_fill(Fill::solid(FantaColor::rgb(1, 2, 3))));
        let field = InspectorField::PaintOpacity {
            id,
            index: 0,
            is_stroke: false,
        };
        let operations = field_operations(&doc, &field, "50");
        let Some(Operation::ReplaceData { new, .. }) = operations.first() else {
            panic!("expected a ReplaceData operation");
        };
        let Some(Fill::Solid { color, .. }) = fill_at(new, 0) else {
            panic!("expected a solid fill");
        };
        assert_eq!(color.a, 128);

        let (doc, id) = doc_with_node(vector_with_fill(Fill::Image {
            asset: fanta_doc::AssetId::new(),
            mode: ImageFitMode::Fill,
            opacity: 1.0,
            crop: None,
            scale: None,
            rotation: None,
            blend: BlendMode::Normal,
            adjust: fanta_doc::ImageAdjust::default(),
        }));
        let field = InspectorField::PaintOpacity {
            id,
            index: 0,
            is_stroke: false,
        };
        let operations = field_operations(&doc, &field, "25%");
        let Some(Operation::ReplaceData { new, .. }) = operations.first() else {
            panic!("expected a ReplaceData operation");
        };
        let Some(Fill::Image { opacity, .. }) = fill_at(new, 0) else {
            panic!("expected an image fill");
        };
        assert!((opacity - 0.25).abs() < 1e-6);
    }
}
