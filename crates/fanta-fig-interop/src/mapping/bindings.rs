//! Variable bindings (pass 4): a node's `variableConsumptionMap` plus
//! paint-level color bindings.

use super::{
    BoundProp, CanvasNode, Doc, KiwiValue, MapReport, NodeData, NodeId, PendingVariable,
    VariableId, guid_key, guid_to_variable_id, read_paint,
};

/// Attach variable bindings (a node's `variableConsumptionMap`) to nodes as
/// [`CanvasNode::bindings`], mapping each Figma `VariableField` to a [`BoundProp`].
pub(crate) fn apply_bindings(
    doc: &mut Doc,
    report: &mut MapReport,
    pending: &[(NodeId, KiwiValue)],
    _pending_variables: &[PendingVariable],
) {
    for (node_id, map) in pending {
        if doc.scene.get(*node_id).is_none() {
            continue;
        }
        let Some(entries) = map.get("entries").and_then(KiwiValue::as_array) else {
            continue;
        };
        let mut binds: Vec<(BoundProp, VariableId)> = Vec::new();
        for entry in entries {
            // The variable referenced: variableData.value.alias (a VariableID).
            let var_guid = entry
                .get("variableData")
                .and_then(|d| d.get("value"))
                .and_then(|v| v.get("alias"))
                .and_then(|a| a.get("guid"))
                .and_then(guid_key);
            let Some(var_guid) = var_guid else { continue };
            let field = entry.get("variableField").and_then(KiwiValue::as_str);
            let Some(prop) = field.and_then(variable_field_to_bound_prop) else {
                continue;
            };
            binds.push((prop, guid_to_variable_id(&var_guid)));
        }
        if !binds.is_empty() {
            report.bindings += binds.len();
            if let Some(node) = doc.scene.get_mut(*node_id) {
                for (prop, var) in binds {
                    node.bindings.insert(prop, var);
                }
            }
        }
    }
}

/// Read Figma paint-level color variable bindings from `Paint.boundVariables`.
///
/// Node-level `variableConsumptionMap` covers scalar properties such as opacity
/// and corner radius. Fill/stroke colors are attached to the paint entries
/// themselves, so we walk the same visible/importable paint arrays used by
/// `read_fills` and `build_stroke` and emit indexed [`BoundProp`] addresses that
/// line up with the imported paint stacks.
pub(crate) fn read_paint_color_bindings(change: &KiwiValue) -> Vec<(BoundProp, VariableId)> {
    let mut binds = Vec::new();

    let fill_paints = match change.get("fillPaints").and_then(KiwiValue::as_array) {
        Some(paints) if !paints.is_empty() => Some(paints),
        _ => change.get("backgroundPaints").and_then(KiwiValue::as_array),
    };
    if let Some(paints) = fill_paints {
        collect_paint_color_bindings(paints, |index| BoundProp::FillColor { index }, &mut binds);
    }

    if stroke_paints_can_land(change) {
        if let Some(paints) = change.get("strokePaints").and_then(KiwiValue::as_array) {
            collect_paint_color_bindings(
                paints,
                |index| BoundProp::StrokeColor { index },
                &mut binds,
            );
        }
    }

    binds
}

/// Attach paint-level color bindings after the scene has been assembled. We
/// filter against the built node so the report counts bindings that can actually
/// affect render (e.g. a frame background uses only fill index 0; a toggled-off
/// stroke with `strokeWeight: 0` lands no stroke and gets no binding).
pub(crate) fn apply_paint_color_bindings(
    doc: &mut Doc,
    report: &mut MapReport,
    pending: &[(NodeId, Vec<(BoundProp, VariableId)>)],
) {
    for (node_id, binds) in pending {
        let Some(node) = doc.scene.get_mut(*node_id) else {
            continue;
        };
        let mut inserted = 0usize;
        for (prop, var) in binds {
            if paint_binding_applies(node, prop) {
                node.bindings.insert(*prop, *var);
                inserted += 1;
            }
        }
        report.bindings += inserted;
    }
}

/// Map a Figma `VariableField` enum member to a [`BoundProp`]. Returns `None`
/// for fields the doc model doesn't address yet (skipped + not counted).
pub(crate) fn variable_field_to_bound_prop(field: &str) -> Option<BoundProp> {
    Some(match field {
        "CORNER_RADIUS" => BoundProp::CornerRadius,
        "STROKE_WEIGHT" => BoundProp::StrokeWidth { index: 0 },
        "OPACITY" => BoundProp::Opacity,
        "VISIBLE" => BoundProp::Visible,
        "TEXT_DATA" => BoundProp::TextContent,
        "WIDTH" => BoundProp::ClipWidth,
        "HEIGHT" => BoundProp::ClipHeight,
        _ => return None,
    })
}

fn collect_paint_color_bindings(
    paints: &[KiwiValue],
    prop: impl Fn(u16) -> BoundProp,
    out: &mut Vec<(BoundProp, VariableId)>,
) {
    let mut landed_index = 0u16;
    for paint in paints {
        if read_paint(paint).is_none() {
            continue;
        }
        if let Some(var_guid) = paint_color_variable_guid(paint) {
            out.push((prop(landed_index), guid_to_variable_id(&var_guid)));
        }
        landed_index = landed_index.saturating_add(1);
    }
}

fn stroke_paints_can_land(change: &KiwiValue) -> bool {
    let has_stroke_geom = change
        .get("strokeGeometry")
        .and_then(KiwiValue::as_array)
        .map(|a| !a.is_empty())
        .unwrap_or(false);
    let positive_weight = change
        .get("strokeWeight")
        .and_then(KiwiValue::as_f64)
        .is_some_and(|w| w > 0.0);
    has_stroke_geom || positive_weight
}

fn paint_color_variable_guid(paint: &KiwiValue) -> Option<String> {
    let color_binding = paint.get("boundVariables").and_then(|b| b.get("color"))?;
    variable_ref_guid(color_binding)
}

fn variable_ref_guid(value: &KiwiValue) -> Option<String> {
    guid_key(value)
        .or_else(|| value.get("guid").and_then(guid_key))
        .or_else(|| value.get("id").and_then(guid_key))
        .or_else(|| {
            value
                .get("id")
                .and_then(|id| id.get("guid"))
                .and_then(guid_key)
        })
        .or_else(|| {
            value
                .get("alias")
                .and_then(|a| a.get("guid"))
                .and_then(guid_key)
        })
        .or_else(|| {
            value
                .get("value")
                .and_then(|v| v.get("alias"))
                .and_then(|a| a.get("guid"))
                .and_then(guid_key)
        })
        .or_else(|| {
            value
                .get("variableData")
                .and_then(|d| d.get("value"))
                .and_then(|v| v.get("alias"))
                .and_then(|a| a.get("guid"))
                .and_then(guid_key)
        })
}

fn paint_binding_applies(node: &CanvasNode, prop: &BoundProp) -> bool {
    match (prop, &node.data) {
        (BoundProp::FillColor { index }, NodeData::Vector(v)) => {
            v.fills.get(*index as usize).is_some()
        }
        (BoundProp::FillColor { index }, NodeData::Group(g)) => {
            *index == 0 && g.background.is_some()
        }
        (BoundProp::FillColor { index }, NodeData::Text(_)) => *index == 0,
        (
            BoundProp::StrokeColor { index } | BoundProp::StrokeWidth { index },
            NodeData::Vector(v),
        ) => v.strokes.get(*index as usize).is_some(),
        (
            BoundProp::StrokeColor { index } | BoundProp::StrokeWidth { index },
            NodeData::Group(g),
        ) => g.strokes.get(*index as usize).is_some(),
        _ => false,
    }
}
