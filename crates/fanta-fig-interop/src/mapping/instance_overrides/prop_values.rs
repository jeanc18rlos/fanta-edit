//! Typed instance `prop_values` and the componentProp assignment/ref readers
//! (`prop_assignment_*`, `PropRefKind`, `read_component_prop_refs`).

use crate::kiwi::KiwiValue;
use crate::mapping::{MapReport, PendingInstanceOverrides, guid_key, read_var_value};
use fanta_doc::component::ComponentPropKind;
use fanta_doc::doc::Doc;
use fanta_doc::id::{ComponentId, ComponentPropId};
use fanta_doc::node::NodeData;
use fanta_doc::value::VarValue;
use std::collections::{BTreeMap, HashMap};

/// Populate each instance's typed [`InstanceNode::prop_values`] from its
/// `componentPropAssignments`, keyed by the component prop schema
/// ([`build_components_and_sets`] gives `prop-def guid → (ComponentId,
/// ComponentPropId)`).
///
/// This is the schema-aware view of an instance's chosen props — the same data
/// the override pass reads for text/visible/swap, but stored typed on the node so
/// (a) variant selection ([`fanta_doc::resolve::expand_instance`]) can pick the
/// right member of a component set by its `Variant { axis }` props, and (b) the
/// editor/inspector can surface the instance's prop values. An assignment whose
/// `defID` doesn't resolve to a prop (cross-file ref, stale) is skipped.
pub(crate) fn populate_instance_prop_values(
    doc: &mut Doc,
    report: &mut MapReport,
    pending: &[PendingInstanceOverrides],
    prop_guid_to_id: &HashMap<String, (ComponentId, ComponentPropId)>,
) {
    for po in pending {
        let mut values: BTreeMap<ComponentPropId, VarValue> = BTreeMap::new();
        for cpa in &po.prop_assignments {
            let Some(def_guid) = cpa.get("defID").and_then(guid_key) else {
                continue;
            };
            let Some((_cid, pid)) = prop_guid_to_id.get(&def_guid) else {
                continue;
            };
            if let Some(v) = prop_assignment_value(cpa) {
                values.insert(*pid, v);
            }
        }
        if values.is_empty() {
            continue;
        }
        // Does this instance belong to a component SET and carry a variant axis
        // selection? (Counted for the report so variant selection is observable
        // even when — as in the Spectrum file — instances point at member defs
        // directly and no instance-of-a-set exists.)
        let is_variant_set_instance = match doc.scene.get(po.instance).map(|n| &n.data) {
            Some(NodeData::Instance(i)) => doc
                .components
                .sets
                .get(&i.component)
                .map(|set| {
                    set.members.iter().any(|&m| {
                        doc.components.def(m).is_some_and(|d| {
                            d.props.iter().any(|p| {
                                matches!(p.kind, ComponentPropKind::Variant { .. })
                                    && values.contains_key(&p.id)
                            })
                        })
                    })
                })
                .unwrap_or(false),
            _ => false,
        };
        if let Some(NodeData::Instance(inst)) = doc.scene.get_mut(po.instance).map(|n| &mut n.data)
        {
            inst.prop_values = values;
            report.instances_with_prop_values += 1;
        }
        if is_variant_set_instance {
            report.variant_selections += 1;
        }
    }
}

/// Read a `ComponentPropAssignment`'s chosen value into a [`VarValue`], trying the
/// typed channels in order: TEXT (`value.textValue.characters` → String), BOOL
/// (`value.boolValue` → Boolean), then the generic `varValue` (a `VariableData`,
/// covering VARIANT string selections and float/color props). A SWAP's
/// `guidValue` is captured as a String guid so it round-trips, though variant
/// selection reads only the String channel.
pub(crate) fn prop_assignment_value(cpa: &KiwiValue) -> Option<VarValue> {
    let value = cpa.get("value");
    if let Some(text) = prop_assignment_text(cpa) {
        return Some(VarValue::String { value: text });
    }
    if let Some(KiwiValue::Bool(b)) = value.and_then(|v| v.get("boolValue")) {
        return Some(VarValue::Boolean { value: *b });
    }
    if let Some(g) = value.and_then(|v| v.get("guidValue")).and_then(guid_key) {
        return Some(VarValue::String { value: g });
    }
    // Generic VariableData (variant string selections live here as a textValue).
    cpa.get("varValue").and_then(read_var_value)
}

/// The text a `ComponentPropAssignment` sets, reading `value.textValue.characters`
/// first and falling back to `varValue.value.textDataValue.characters` (both seen
/// in the real file; the former is the canonical authored value).
pub(crate) fn prop_assignment_text(cpa: &KiwiValue) -> Option<String> {
    cpa.get("value")
        .and_then(|v| v.get("textValue"))
        .and_then(|t| t.get("characters"))
        .and_then(KiwiValue::as_str)
        .or_else(|| {
            cpa.get("varValue")
                .and_then(|v| v.get("value"))
                .and_then(|v| v.get("textDataValue"))
                .and_then(|t| t.get("characters"))
                .and_then(KiwiValue::as_str)
        })
        .map(str::to_owned)
}

/// What a master descendant's `componentPropRefs` entry binds to a component
/// prop-def. A descendant declares "my X is driven by prop-def G"; the outer
/// instance's `componentPropAssignments[G → value]` then sets it. We model the
/// three node-fields that change rendered output:
///
/// - [`Text`](PropRefKind::Text): the node's text content (`TEXT_DATA`) — the
///   button label (e.g. "Skip Tour"/"Next").
/// - [`Visible`](PropRefKind::Visible): the node's visibility (`VISIBLE`) — a
///   bool prop that shows/hides a descendant (e.g. a button's "Hold Icon"
///   placeholder vector — the spurious ▪ when left visible).
/// - [`InstanceSwap`](PropRefKind::InstanceSwap): the node (a nested instance)
///   is icon-swapped via the prop's chosen component (`OVERRIDDEN_SYMBOL_ID`) —
///   the Edit/Copy/Delete icon distinction on a shared Button master.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PropRefKind {
    Text,
    Visible,
    InstanceSwap,
}

/// Read a node's `componentPropRefs` into `(prop-def guid, PropRefKind)` pairs.
/// We keep the refs whose effect we can apply: `TEXT_DATA` (text content),
/// `VISIBLE` (show/hide), and `OVERRIDDEN_SYMBOL_ID` (nested-instance icon swap).
/// Returns `None` when the node has no usable prop refs.
pub(crate) fn read_component_prop_refs(change: &KiwiValue) -> Option<Vec<(String, PropRefKind)>> {
    let refs = change
        .get("componentPropRefs")
        .and_then(KiwiValue::as_array)?;
    let mut out = Vec::new();
    for r in refs {
        if matches!(r.get("isDeleted"), Some(KiwiValue::Bool(true))) {
            continue;
        }
        let Some(def_guid) = r.get("defID").and_then(guid_key) else {
            continue;
        };
        // The bound field is named either `componentPropNodeField` (current) or
        // `nodeField` (legacy).
        let field = r
            .get("componentPropNodeField")
            .and_then(KiwiValue::as_str)
            .or_else(|| r.get("nodeField").and_then(KiwiValue::as_str));
        let kind = match field {
            Some("TEXT_DATA") => PropRefKind::Text,
            Some("VISIBLE") => PropRefKind::Visible,
            Some("OVERRIDDEN_SYMBOL_ID") => PropRefKind::InstanceSwap,
            _ => continue,
        };
        out.push((def_guid, kind));
    }
    if out.is_empty() { None } else { Some(out) }
}
