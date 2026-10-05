use super::{
    Doc, FigError, FigResult, Fill, HashMap, KiwiValue, MapReport, NodeBuild, NodeData, NodeId,
    Transform2D, build_stroke, build_vector,
};
use fanta_doc::{BooleanBakedGeometry, BooleanNode, BooleanOp, boolean_geometry_signature};
use std::collections::HashSet;

pub(crate) fn build_boolean(
    change: &KiwiValue,
    size: (f64, f64),
    fills: &[Fill],
    transform: Transform2D,
    blobs: &[Vec<u8>],
) -> NodeBuild {
    let mut built = build_vector("BOOLEAN_OPERATION", change, size, fills, transform, blobs);
    let NodeBuild::Node { node, .. } = &mut built else {
        return built;
    };
    let operation = change.get("booleanOperation").and_then(KiwiValue::as_str);
    node.meta["figma_boolean_operation"] = serde_json::json!(operation);
    let operation = match operation {
        Some("UNION") => BooleanOp::Union,
        Some("SUBTRACT") => BooleanOp::Subtract,
        Some("INTERSECT") => BooleanOp::Intersect,
        Some("XOR" | "EXCLUDE") => BooleanOp::Exclude,
        Some(value) => {
            node.meta["boolean_fallback"] = serde_json::json!(format!("unknown operation {value}"));
            return built;
        }
        None => {
            node.meta["boolean_fallback"] =
                serde_json::json!(if change.get("booleanOperation").is_some() {
                    "invalid operation field"
                } else {
                    "missing operation"
                });
            return built;
        }
    };
    let NodeData::Vector(vector) = &node.data else {
        return built;
    };
    node.data = NodeData::Boolean(BooleanNode {
        op: operation,
        fills: fills.iter().cloned().collect(),
        strokes: build_stroke(change).into_iter().collect(),
        baked: Some(BooleanBakedGeometry {
            vector: vector.clone(),
            source: String::new(),
            stroke_outline: node.meta["stroke_only_outline"].as_bool() == Some(true),
        }),
    });
    built
}

pub(crate) fn prepare_imported_booleans(
    doc: &mut Doc,
    report: &mut MapReport,
    guid_to_node: &mut HashMap<String, Option<NodeId>>,
    guid_to_parent: &HashMap<String, Option<String>>,
) -> FigResult<()> {
    let mut missing_operands = HashSet::new();
    for (guid, _) in guid_to_node.iter().filter(|(_, node)| node.is_none()) {
        let mut parent = guid_to_parent.get(guid).and_then(Option::as_ref);
        let mut seen = HashSet::new();
        while let Some(guid) = parent {
            if !seen.insert(guid) {
                break;
            }
            if let Some(Some(id)) = guid_to_node.get(guid)
                && doc
                    .scene
                    .get(*id)
                    .is_some_and(|node| matches!(node.data, NodeData::Boolean(_)))
            {
                missing_operands.insert(*id);
            }
            parent = guid_to_parent.get(guid).and_then(Option::as_ref);
        }
    }
    let candidates = doc
        .scene
        .roots()
        .iter()
        .flat_map(|root| doc.scene.descendants_of(*root))
        .filter(|id| {
            doc.scene
                .get(*id)
                .is_some_and(|node| matches!(node.data, NodeData::Boolean(_)))
        })
        .collect::<Vec<_>>();
    let mut removed = HashSet::new();
    // Check children before parents: a nested fallback must also invalidate an
    // outer editable operation rather than silently become a baked operand.
    for id in candidates.into_iter().rev() {
        let unsupported = doc.scene.descendants_of(id).skip(1).find_map(|child| {
            let node = doc.scene.get(child)?;
            if node.meta.get("boolean_fallback").is_some() {
                Some("unsupported nested Boolean operation".to_owned())
            } else if matches!(node.data, NodeData::Vector(_))
                && node.meta["geometry"] == "bbox_fallback"
            {
                Some("operand geometry is unavailable".to_owned())
            } else {
                None
            }
        });
        let reason = unsupported
            .or_else(|| {
                missing_operands
                    .contains(&id)
                    .then(|| "unmapped operand nodes".to_owned())
            })
            .or_else(|| {
                boolean_geometry_signature(&doc.scene, id)
                    .err()
                    .map(|error| error.to_string())
            });
        let node = doc
            .scene
            .get(id)
            .ok_or_else(|| FigError::Mapping("missing imported Boolean".into()))?;
        let has_geometry = node.meta["geometry"] != "bbox_fallback";
        let has_operands = !doc.scene.children_of(Some(id)).is_empty();
        let reason = reason
            .or_else(|| (!has_operands).then(|| "editable operands are unavailable".to_owned()));
        if let Some(reason) = reason {
            let children = doc.scene.children_of(Some(id)).to_vec();
            for child in children {
                removed.extend(doc.scene.descendants_of(child));
                doc.scene
                    .remove(child)
                    .map_err(|error| FigError::Mapping(error.to_string()))?;
            }
            let node = doc
                .scene
                .get_mut(id)
                .ok_or_else(|| FigError::Mapping("missing imported Boolean".into()))?;
            let NodeData::Boolean(boolean) = &mut node.data else {
                continue;
            };
            let baked = boolean
                .baked
                .take()
                .ok_or_else(|| FigError::Mapping("missing Boolean fallback".into()))?;
            node.data = NodeData::Vector(baked.vector);
            node.meta["boolean_fallback"] = serde_json::json!(reason);
        } else if !has_geometry {
            // A bounding-box placeholder is not authored Boolean artwork. When
            // real operands exist, their live fold is the available geometry.
            if let Some(node) = doc.scene.get_mut(id)
                && let NodeData::Boolean(boolean) = &mut node.data
            {
                boolean.baked = None;
            }
        }
    }
    report.boolean_operands_dropped += removed.len();
    report.mapped -= removed.len();
    for node in guid_to_node.values_mut() {
        if node.is_some_and(|id| removed.contains(&id)) {
            *node = None;
        }
    }
    for id in doc
        .scene
        .roots()
        .iter()
        .flat_map(|root| doc.scene.descendants_of(*root))
    {
        let Some(node) = doc.scene.get(id) else {
            continue;
        };
        if matches!(node.data, NodeData::Vector(_))
            && node.meta["figma_type"] == "BOOLEAN_OPERATION"
        {
            report.boolean_operations_flattened += 1;
            let reason = node.meta["boolean_fallback"]
                .as_str()
                .unwrap_or("unsupported geometry");
            *report
                .boolean_fallbacks_by_reason
                .entry(reason.to_owned())
                .or_default() += 1;
        }
    }
    Ok(())
}

pub(crate) fn seal_imported_booleans(doc: &mut Doc) -> FigResult<()> {
    let candidates = doc
        .scene
        .roots()
        .iter()
        .flat_map(|root| doc.scene.descendants_of(*root))
        .filter(|id| {
            doc.scene.get(*id).is_some_and(
                |node| matches!(&node.data, NodeData::Boolean(boolean) if boolean.baked.is_some()),
            )
        })
        .collect::<Vec<_>>();
    for id in candidates {
        let source = boolean_geometry_signature(&doc.scene, id)
            .map_err(|error| FigError::Mapping(error.to_string()))?;
        if let Some(node) = doc.scene.get_mut(id)
            && let NodeData::Boolean(boolean) = &mut node.data
            && let Some(baked) = &mut boolean.baked
        {
            baked.source = source;
        }
    }
    Ok(())
}
