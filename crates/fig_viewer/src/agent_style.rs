//! The `style` projection agents read nodes through (`batch_get` /
//! `design_state` with `detail: "style"`, and by id by default).
//!
//! It speaks the vocabulary of the ops agents write with (`design_surface::DesignOp`):
//! a value read here can be sent back unchanged — `auto_layout` takes the
//! fields of `set_auto_layout`, `fills` the paints of `set_fill`, `effects`
//! the layers of `set_effects`, and so on. The raw node record (`detail:
//! "raw"`) uses the document model's own names and matrices instead, which
//! made agents translate between two schemas and edit what they could not
//! see.

use fanta_doc::{
    AxisSizing, Blur, BlurKind, CanvasNode, ConstraintH, ConstraintV, CounterAlign, Doc, Fill,
    Gradient, ImageFitMode, LayoutMode, NodeData, NodeFlags, NodeId, PrimaryAlign, Shadow,
    ShadowKind, Stroke, StrokeAlign, StrokeCap, StrokeJoin, TextAlign,
};
use serde_json::{Map, Value, json};

/// The style of `id`, or `null` when the node does not exist.
pub(crate) fn node_style(doc: &Doc, id: NodeId) -> Value {
    let Some(node) = doc.scene.get(id) else {
        return Value::Null;
    };
    let mut style = Map::new();
    style.insert("id".into(), json!(id.to_string()));
    style.insert("kind".into(), json!(node.data.kind_tag()));
    style.insert("name".into(), json!(node.name));
    style.insert(
        "parent".into(),
        json!(node.parent.map(|parent| parent.to_string())),
    );
    // World bounds: the same space `set_props` x/y/width/height write in.
    style.insert(
        "bounds".into(),
        crate::agent_surface::world_bounds_json(doc, id),
    );
    let [a, b, ..] = node.transform.to_components();
    let rotation = b.atan2(a).to_degrees();
    if rotation.abs() > 1e-6 {
        style.insert("rotation".into(), json!(round(rotation)));
    }
    if node.opacity.get() < 1.0 {
        style.insert("opacity".into(), json!(node.opacity.get()));
    }
    if node.flags.contains(NodeFlags::HIDDEN) {
        style.insert("hidden".into(), json!(true));
    }
    if node.flags.contains(NodeFlags::LOCKED) {
        style.insert("locked".into(), json!(true));
    }
    let blend = serde_json::to_value(node.blend_mode).unwrap_or(Value::Null);
    if blend != json!("normal") && !blend.is_null() {
        style.insert("blend_mode".into(), blend);
    }
    data_style(doc, node, &mut style);
    if let Some(child) = node.layout_child {
        style.insert(
            "layout_child".into(),
            json!({
                "grow": child.grow,
                "align_self": child.align_self.map(cross_axis_name),
                "absolute": child.absolute,
            }),
        );
    }
    if let Some(constraints) = node.constraints {
        style.insert(
            "constraints".into(),
            json!({
                "horizontal": horizontal_name(constraints.horizontal),
                "vertical": vertical_name(constraints.vertical),
            }),
        );
    }
    let effects = effects(&node.effects, &node.blurs);
    if !effects.is_empty() {
        style.insert("effects".into(), Value::Array(effects));
    }
    if !node.bindings.is_empty() {
        style.insert(
            "bindings".into(),
            Value::Array(
                node.bindings
                    .iter()
                    .map(|(property, variable)| {
                        json!({ "property": property, "variable": variable.to_string() })
                    })
                    .collect(),
            ),
        );
    }
    Value::Object(style)
}

/// Merge every listed node's style into a page listing (the tree
/// `node_summary` builds), so each entry is one flat node: the listing's own
/// keys plus its style. Style wins where both name a key (`auto_layout` grows
/// from the mode string to the full layout); the listing's `fill` hex stays.
pub(crate) fn attach_styles(doc: &Doc, listing: &mut Value) {
    let Some(object) = listing.as_object_mut() else {
        return;
    };
    if let Some(id) = object
        .get("id")
        .and_then(Value::as_str)
        .and_then(|id| id.parse::<NodeId>().ok())
        && let Value::Object(style) = node_style(doc, id)
    {
        for (key, value) in style {
            if !matches!(key.as_str(), "id" | "kind" | "name" | "parent") {
                object.insert(key, value);
            }
        }
    }
    if let Some(children) = object.get_mut("children").and_then(Value::as_array_mut) {
        for child in children {
            attach_styles(doc, child);
        }
    }
}

fn data_style(doc: &Doc, node: &CanvasNode, style: &mut Map<String, Value>) {
    match &node.data {
        NodeData::Group(group) => {
            let fills: Vec<Value> = group
                .background
                .iter()
                .chain(group.background_fills.iter())
                .map(paint)
                .collect();
            if !fills.is_empty() {
                style.insert("fills".into(), Value::Array(fills));
            }
            corner_radius(group.corner_radius, group.corner_radii, style);
            if group.clip_size.is_some() {
                style.insert("clips_content".into(), json!(true));
            }
            if let Some(layout) = &group.auto_layout {
                style.insert(
                    "auto_layout".into(),
                    json!({
                        "direction": match layout.mode {
                            LayoutMode::Horizontal => "horizontal",
                            LayoutMode::Vertical => "vertical",
                        },
                        "gap": layout.spacing,
                        "counter_gap": layout.counter_spacing,
                        "padding": layout.padding,
                        "align_items": cross_axis_name(layout.counter_align),
                        "justify": main_axis_name(layout.primary_align),
                        "primary_sizing": sizing_name(layout.primary_sizing),
                        "counter_sizing": sizing_name(layout.counter_sizing),
                        "wrap": layout.wrap,
                        "min_size": layout.min_size,
                        "max_size": layout.max_size,
                    }),
                );
            }
            if !group.explicit_modes.is_empty() {
                let modes: Map<String, Value> = group
                    .explicit_modes
                    .iter()
                    .map(|(collection, mode)| {
                        let collection = doc.variables.collections.get(collection);
                        let mode_name = collection
                            .and_then(|collection| {
                                collection
                                    .modes
                                    .iter()
                                    .find(|candidate| candidate.id == *mode)
                            })
                            .map(|mode| mode.name.clone())
                            .unwrap_or_else(|| mode.to_string());
                        let collection_name = collection
                            .map(|collection| collection.name.clone())
                            .unwrap_or_default();
                        (collection_name, json!(mode_name))
                    })
                    .collect();
                style.insert("variable_modes".into(), Value::Object(modes));
            }
            strokes(&group.strokes, style);
        }
        NodeData::Vector(vector) => {
            fills(&vector.fills, style);
            strokes(&vector.strokes, style);
            corner_radius(vector.corner_radius, vector.corner_radii, style);
        }
        NodeData::Boolean(boolean) => {
            fills(&boolean.fills, style);
            strokes(&boolean.strokes, style);
        }
        NodeData::Text(text) => {
            let style_ref = &text.style;
            style.insert(
                "text".into(),
                json!({
                    "content": text.content,
                    "font_family": style_ref.font_family,
                    "font_weight": style_ref.weight,
                    "font_size": style_ref.size_px,
                    "line_height": style_ref.line_height,
                    "letter_spacing": style_ref.letter_spacing,
                    "align": match text.align {
                        TextAlign::Left => "left",
                        TextAlign::Center => "center",
                        TextAlign::Right => "right",
                        TextAlign::Justify => "justify",
                    },
                    "color": style_ref.color.to_hex(),
                }),
            );
        }
        NodeData::Instance(instance) => {
            style.insert(
                "component".into(),
                json!(
                    doc.components
                        .def(instance.component)
                        .map(|definition| definition.name.as_str())
                ),
            );
        }
        _ => {}
    }
}

fn fills(fills: &[Fill], style: &mut Map<String, Value>) {
    if !fills.is_empty() {
        style.insert("fills".into(), fills.iter().map(paint).collect());
    }
}

/// A fill in `set_fill`'s `DesignPaint` vocabulary. Gradient kinds the op
/// cannot write (angular, diamond) still report their stops.
fn paint(fill: &Fill) -> Value {
    let stops = |stops: &[fanta_doc::GradientStop]| -> Vec<Value> {
        stops
            .iter()
            .map(|stop| json!({ "position": stop.position, "color": stop.color.to_hex() }))
            .collect()
    };
    match fill {
        Fill::Solid { color, .. } => json!({ "kind": "solid", "color": color.to_hex() }),
        Fill::Gradient { gradient, .. } => match gradient {
            Gradient::Linear {
                start,
                end,
                stops: list,
            } => {
                json!({ "kind": "linear", "from": start, "to": end, "stops": stops(list) })
            }
            Gradient::Radial {
                center,
                radius,
                stops: list,
                ..
            } => {
                json!({ "kind": "radial", "center": center, "radius": radius, "stops": stops(list) })
            }
            other => {
                let raw = serde_json::to_value(other).unwrap_or(Value::Null);
                let kind = raw.get("kind").cloned().unwrap_or(json!("gradient"));
                let list = raw.get("stops").cloned().unwrap_or(json!([]));
                json!({ "kind": kind, "stops": list })
            }
        },
        // Paint kinds `set_fill` cannot write: reported so an agent knows a
        // `set_fill` on this node would replace them.
        Fill::Video { .. } => json!({ "kind": "video", "read_only": true }),
        Fill::Pattern { .. } => json!({ "kind": "pattern", "read_only": true }),
        Fill::Shader { .. } => json!({ "kind": "shader", "read_only": true }),
        Fill::Image { asset, mode, .. } => json!({
            "kind": "image",
            "asset": asset.to_string(),
            "fit": match mode {
                ImageFitMode::Fill => "fill",
                ImageFitMode::Fit => "fit",
                ImageFitMode::Stretch => "stretch",
                ImageFitMode::Tile => "tile",
            },
        }),
    }
}

fn strokes(strokes: &[Stroke], style: &mut Map<String, Value>) {
    if strokes.is_empty() {
        return;
    }
    let list: Vec<Value> = strokes
        .iter()
        .map(|stroke| {
            let mut entry = Map::new();
            match &stroke.paint {
                Fill::Solid { color, .. } => {
                    entry.insert("color".into(), json!(color.to_hex()));
                }
                other => {
                    entry.insert("paint".into(), paint(other));
                }
            }
            entry.insert("width".into(), json!(stroke.width));
            entry.insert(
                "align".into(),
                json!(match stroke.align {
                    StrokeAlign::Inside => "inside",
                    StrokeAlign::Center => "center",
                    StrokeAlign::Outside => "outside",
                }),
            );
            if let Some(sides) = stroke.per_side {
                entry.insert("sides".into(), json!(sides));
            }
            if !stroke.dash.is_empty() {
                entry.insert("dash".into(), json!(stroke.dash));
            }
            if stroke.cap != StrokeCap::Butt {
                entry.insert(
                    "cap".into(),
                    json!(match stroke.cap {
                        StrokeCap::Butt => "butt",
                        StrokeCap::Round => "round",
                        StrokeCap::Square => "square",
                    }),
                );
            }
            if stroke.join != StrokeJoin::Miter {
                entry.insert(
                    "join".into(),
                    json!(match stroke.join {
                        StrokeJoin::Miter => "miter",
                        StrokeJoin::Round => "round",
                        StrokeJoin::Bevel => "bevel",
                    }),
                );
            }
            Value::Object(entry)
        })
        .collect();
    style.insert("strokes".into(), Value::Array(list));
}

/// Shadows then blurs, in `set_effects`' `DesignEffect` vocabulary.
fn effects(shadows: &[Shadow], blurs: &[Blur]) -> Vec<Value> {
    shadows
        .iter()
        .map(|shadow| {
            json!({
                "kind": match shadow.kind {
                    ShadowKind::Drop => "drop_shadow",
                    ShadowKind::Inner => "inner_shadow",
                },
                "color": shadow.color.to_hex(),
                "x": shadow.offset[0],
                "y": shadow.offset[1],
                "blur": shadow.blur,
                "spread": shadow.spread,
            })
        })
        .chain(blurs.iter().map(|blur| {
            json!({
                "kind": match blur.kind {
                    BlurKind::Layer => "layer_blur",
                    BlurKind::Background => "background_blur",
                },
                "radius": blur.radius,
            })
        }))
        .collect()
}

fn corner_radius(uniform: Option<f64>, corners: Option<[f64; 4]>, style: &mut Map<String, Value>) {
    if let Some(corners) = corners {
        style.insert("corner_radii".into(), json!(corners));
    } else if let Some(radius) = uniform.filter(|radius| *radius > 0.0) {
        style.insert("corner_radius".into(), json!(radius));
    }
}

fn cross_axis_name(alignment: CounterAlign) -> &'static str {
    match alignment {
        CounterAlign::Start => "start",
        CounterAlign::Center => "center",
        CounterAlign::End => "end",
        CounterAlign::Stretch => "stretch",
        CounterAlign::Baseline => "baseline",
    }
}

fn main_axis_name(alignment: PrimaryAlign) -> &'static str {
    match alignment {
        PrimaryAlign::Start => "start",
        PrimaryAlign::Center => "center",
        PrimaryAlign::End => "end",
        PrimaryAlign::SpaceBetween => "space_between",
        PrimaryAlign::SpaceEvenly => "space_evenly",
    }
}

fn sizing_name(sizing: AxisSizing) -> &'static str {
    match sizing {
        AxisSizing::Fixed => "fixed",
        AxisSizing::Hug => "hug",
    }
}

fn horizontal_name(constraint: ConstraintH) -> &'static str {
    match constraint {
        ConstraintH::Left => "left",
        ConstraintH::Right => "right",
        ConstraintH::LeftRight => "left_right",
        ConstraintH::Center => "center",
        ConstraintH::Scale => "scale",
    }
}

fn vertical_name(constraint: ConstraintV) -> &'static str {
    match constraint {
        ConstraintV::Top => "top",
        ConstraintV::Bottom => "bottom",
        ConstraintV::TopBottom => "top_bottom",
        ConstraintV::Center => "center",
        ConstraintV::Scale => "scale",
    }
}

fn round(value: f64) -> f64 {
    (value * 1000.0).round() / 1000.0
}
