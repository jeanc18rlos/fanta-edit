//! Component defs / sets / variants and instance→component wiring (pass 4).

use crate::kiwi::KiwiValue;
use crate::mapping::{MapReport, PendingComponent, PendingSet, guid_key, read_var_value};
use fanta_doc::NodeId;
use fanta_doc::component::{
    ComponentDef, ComponentPropDef, ComponentPropKind, ComponentSet, ComponentSetMembership,
    VariantAxis,
};
use fanta_doc::doc::Doc;
use fanta_doc::id::{ComponentId, ComponentPropId};
use fanta_doc::node::NodeData;
use fanta_doc::value::VarValue;
use std::collections::{BTreeMap, HashMap};

/// Whether a SYMBOL change is a *state group* — the master of a variant set.
pub(crate) fn is_state_group(change: &KiwiValue) -> bool {
    matches!(change.get("isStateGroup"), Some(KiwiValue::Bool(true)))
}

/// The node's `name`, or empty string if absent.
pub(crate) fn node_name(change: &KiwiValue) -> String {
    change
        .get("name")
        .and_then(KiwiValue::as_str)
        .unwrap_or("")
        .to_owned()
}

/// The side-tables [`build_components_and_sets`] hands the override pass.
pub(crate) struct ComponentMaps {
    /// `symbol guid → ComponentId` for member defs *and* sets — lets the override
    /// pass resolve an `overriddenSymbolID` swap to the swapped component.
    pub(crate) guid_to_component: HashMap<String, ComponentId>,
    /// `prop-def guid → (ComponentId, ComponentPropId)`: every exposed component
    /// property, keyed by its Figma prop-def GUID, so an instance's
    /// `componentPropAssignments[defID → value]` populates the right
    /// [`InstanceNode::prop_values`] entry (and so prop DEFAULTS can be applied
    /// for unset props).
    pub(crate) prop_guid_to_id: HashMap<String, (ComponentId, ComponentPropId)>,
}

struct RegisteredComponents {
    guid_to_component: HashMap<String, ComponentId>,
    prop_guid_to_id: HashMap<String, (ComponentId, ComponentPropId)>,
    root_to_guid: HashMap<NodeId, String>,
}

/// Resolve component defs, sets, and instance refs into [`Doc::components`].
///
/// Returns a [`ComponentMaps`]: the `symbol guid → ComponentId` map (member defs
/// *and* sets) so the override pass can resolve an `overriddenSymbolID` swap to
/// the swapped component (op1 `symbol/overrides.ts`), plus the
/// `prop-def guid → (ComponentId, ComponentPropId)` map so component-property
/// assignments populate the right instance prop values and prop defaults apply.
pub(crate) fn build_components_and_sets(
    doc: &mut Doc,
    report: &mut MapReport,
    pending_components: &[PendingComponent<'_>],
    pending_sets: &[PendingSet],
    pending_instances: &[(NodeId, String)],
) -> ComponentMaps {
    let components = register_component_defs(doc, report, pending_components);
    let guid_to_set = register_component_sets(doc, report, pending_sets, &components);
    wire_pending_instances(
        doc,
        pending_instances,
        &components.guid_to_component,
        &guid_to_set,
    );

    let guid_to_any = combined_component_map(guid_to_set, components.guid_to_component);
    ComponentMaps {
        guid_to_component: guid_to_any,
        prop_guid_to_id: components.prop_guid_to_id,
    }
}

fn register_component_defs(
    doc: &mut Doc,
    report: &mut MapReport,
    pending_components: &[PendingComponent<'_>],
) -> RegisteredComponents {
    let mut guid_to_component = HashMap::new();
    let mut prop_guid_to_id = HashMap::new();
    let mut root_to_guid = HashMap::new();

    for pending in pending_components {
        let component_id = ComponentId::new();
        guid_to_component.insert(pending.guid.clone(), component_id);
        root_to_guid.insert(pending.root, pending.guid.clone());

        let name = display_name(doc, pending.root, &pending.name);
        let mut definition = ComponentDef::new(component_id, pending.root, name);
        definition.props =
            parse_component_prop_defs(pending.prop_defs, component_id, &mut prop_guid_to_id);
        report.component_props += definition.props.len();

        doc.components.defs.insert(component_id, definition);
    }

    RegisteredComponents {
        guid_to_component,
        prop_guid_to_id,
        root_to_guid,
    }
}

fn register_component_sets(
    doc: &mut Doc,
    report: &mut MapReport,
    pending_sets: &[PendingSet],
    components: &RegisteredComponents,
) -> HashMap<String, ComponentId> {
    let mut guid_to_set = HashMap::new();

    for pending in pending_sets {
        let set_id = ComponentId::new();
        guid_to_set.insert(pending.guid.clone(), set_id);

        let members = collect_set_members(doc, pending.root, set_id, components);
        let default_variant = members.ids.first().copied().unwrap_or(set_id);
        let set = ComponentSet {
            id: set_id,
            name: display_name(doc, pending.root, &pending.name),
            axes: variant_axes_from_map(members.axes),
            members: members.ids,
            default_variant,
        };

        doc.components.sets.insert(set_id, set);
        report.component_sets += 1;
    }

    guid_to_set
}

struct SetMembers {
    ids: Vec<ComponentId>,
    axes: BTreeMap<String, Vec<String>>,
}

fn collect_set_members(
    doc: &mut Doc,
    set_root: NodeId,
    set_id: ComponentId,
    components: &RegisteredComponents,
) -> SetMembers {
    let mut ids = Vec::new();
    let mut axes = BTreeMap::new();

    let child_ids: Vec<NodeId> = doc.scene.descendants_of(set_root).collect();
    for child in child_ids {
        if child == set_root {
            continue;
        }
        if let Some(component_id) = component_for_root(child, components) {
            ids.push(component_id);
            assign_variant_membership(doc, component_id, set_id, &mut axes);
        }
    }

    SetMembers { ids, axes }
}

fn component_for_root(root: NodeId, components: &RegisteredComponents) -> Option<ComponentId> {
    let guid = components.root_to_guid.get(&root)?;
    components.guid_to_component.get(guid).copied()
}

fn assign_variant_membership(
    doc: &mut Doc,
    component_id: ComponentId,
    set_id: ComponentId,
    axes: &mut BTreeMap<String, Vec<String>>,
) {
    let Some(definition) = doc.components.defs.get_mut(&component_id) else {
        return;
    };

    let axis_values = parse_variant_axes(&definition.name).unwrap_or_default();
    for (axis, value) in &axis_values {
        let values = axes.entry(axis.clone()).or_default();
        if !values.contains(value) {
            values.push(value.clone());
        }
    }
    definition.variant_of = Some(ComponentSetMembership {
        set: set_id,
        axis_values,
    });
}

fn variant_axes_from_map(axes: BTreeMap<String, Vec<String>>) -> Vec<VariantAxis> {
    axes.into_iter()
        .map(|(name, values)| VariantAxis { name, values })
        .collect()
}

fn display_name(doc: &Doc, root: NodeId, explicit_name: &str) -> String {
    if !explicit_name.is_empty() {
        return explicit_name.to_owned();
    }

    doc.scene
        .get(root)
        .map(|node| node.name.clone())
        .unwrap_or_default()
}

fn wire_pending_instances(
    doc: &mut Doc,
    pending_instances: &[(NodeId, String)],
    guid_to_component: &HashMap<String, ComponentId>,
    guid_to_set: &HashMap<String, ComponentId>,
) {
    for (instance_id, symbol_guid) in pending_instances {
        let Some(node) = doc.scene.get_mut(*instance_id) else {
            continue;
        };
        let NodeData::Instance(instance) = &mut node.data else {
            continue;
        };

        if let Some(&component_id) = guid_to_component.get(symbol_guid) {
            instance.component = component_id;
        } else if let Some(&set_id) = guid_to_set.get(symbol_guid) {
            instance.component = set_id;
        }
    }
}

fn combined_component_map(
    guid_to_set: HashMap<String, ComponentId>,
    guid_to_component: HashMap<String, ComponentId>,
) -> HashMap<String, ComponentId> {
    let mut guid_to_any = guid_to_set;
    guid_to_any.extend(guid_to_component);
    guid_to_any
}

/// Borrow a component master's raw `componentPropDefs` array (a
/// `ComponentPropDef[]`) for later parsing. Empty when the master exposes no
/// instance-settable properties. The legacy field name `componentPropertyDefinitions`
/// is also honored for forward/back-compat with other `.fig` exporters.
pub(crate) fn read_prop_defs_raw(change: &KiwiValue) -> &[KiwiValue] {
    change
        .get("componentPropDefs")
        .or_else(|| change.get("componentPropertyDefinitions"))
        .and_then(KiwiValue::as_array)
        .unwrap_or(&[])
}

/// One `componentPropDef`'s identity for variant-placeholder resolution: its
/// display `name` (e.g. `"Hold Icon ?"`) and the guid of its `parentPropDefId`
/// (the set-level prop a variant member's thin def inherits from). A variant
/// member's own prop def carries only `id` + `parentPropDefId` (no name/type);
/// the name + default live on the parent (the state-group's def). Walking the
/// parent chain recovers the prop NAME a `VISIBLE` ref ultimately binds.
#[derive(Debug, Clone)]
pub(crate) struct PropDefInfo {
    pub(crate) name: Option<String>,
    pub(crate) parent: Option<String>,
    pub(crate) default_bool: Option<bool>,
}

/// Collect every `componentPropDef`'s [`PropDefInfo`] across ALL nodes (a
/// prop-def guid → its name + parent guid), so the placeholder-hiding pass can
/// resolve a `VISIBLE` ref's bound prop NAME through the `parentPropDefId`
/// chain. Honors both the current `componentPropDefs` and the legacy
/// `componentPropertyDefinitions` field name. A later occurrence of the same
/// guid overwrites an earlier one (they describe the same prop).
pub(crate) fn collect_prop_def_infos(change: &KiwiValue, out: &mut HashMap<String, PropDefInfo>) {
    let defs = change
        .get("componentPropDefs")
        .or_else(|| change.get("componentPropertyDefinitions"))
        .and_then(KiwiValue::as_array);
    let Some(defs) = defs else { return };
    for d in defs {
        let Some(id) = d.get("id").and_then(guid_key) else {
            continue;
        };
        let name = d
            .get("name")
            .and_then(KiwiValue::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_owned);
        let parent = d.get("parentPropDefId").and_then(guid_key);
        let default_bool = d.get("varValue").and_then(read_var_value).and_then(|v| {
            if let VarValue::Boolean { value } = v {
                Some(value)
            } else {
                None
            }
        });
        out.insert(
            id,
            PropDefInfo {
                name,
                parent,
                default_bool,
            },
        );
    }
}

/// Resolve a prop-def guid to its display NAME, walking the `parentPropDefId`
/// chain until a named def is found (a variant member's thin def is unnamed; the
/// name lives on the set-level parent). Bounded to avoid a pathological cycle.
pub(crate) fn resolve_prop_def_name(
    start: &str,
    infos: &HashMap<String, PropDefInfo>,
) -> Option<String> {
    let mut cur = start.to_owned();
    for _ in 0..8 {
        let info = infos.get(&cur)?;
        if let Some(name) = &info.name {
            return Some(name.clone());
        }
        cur = info.parent.clone()?;
    }
    None
}

/// Resolve a prop-def guid to a BOOL default, walking `parentPropDefId` exactly
/// like [`resolve_prop_def_name`]. Variant-member prop defs are often thin
/// aliases, while the set-level parent carries the actual default value.
pub(crate) fn resolve_prop_def_default_bool(
    start: &str,
    infos: &HashMap<String, PropDefInfo>,
) -> Option<bool> {
    let mut cur = start.to_owned();
    for _ in 0..8 {
        let info = infos.get(&cur)?;
        if let Some(value) = info.default_bool {
            return Some(value);
        }
        cur = info.parent.clone()?;
    }
    None
}

/// Parse a master's `componentPropDefs` into typed [`ComponentPropDef`]s,
/// minting a [`ComponentPropId`](fanta_doc::id::ComponentPropId) per prop and
/// recording `prop-def guid → (ComponentId, ComponentPropId)` in `prop_guid_to_id`
/// so instance assignments + defaults resolve.
///
/// Each entry carries:
/// - `id` (a `GUID`): the prop-def guid an instance assignment / a descendant's
///   `componentPropRefs` references.
/// - `type` (`ComponentPropType` enum): TEXT / BOOL / INSTANCE_SWAP / VARIANT.
/// - `name`: a `"Label"` (TEXT/BOOL/SWAP) or `"Size"` (VARIANT axis) string —
///   for a VARIANT prop the name *is* the axis name.
/// - `varValue` (a `VariableData`): the prop's default value.
///
/// A prop with no parseable id or an unrecognized type is skipped (tolerated).
pub(crate) fn parse_component_prop_defs(
    defs: &[KiwiValue],
    cid: ComponentId,
    prop_guid_to_id: &mut HashMap<String, (ComponentId, ComponentPropId)>,
) -> Vec<ComponentPropDef> {
    let mut out = Vec::new();
    for d in defs {
        let Some(def_guid) = d.get("id").and_then(guid_key) else {
            continue;
        };
        let ty = d.get("type").and_then(KiwiValue::as_str).unwrap_or("");
        let name = d
            .get("name")
            .and_then(KiwiValue::as_str)
            .unwrap_or("")
            .to_owned();
        let kind = match ty {
            "TEXT" => ComponentPropKind::Text,
            "BOOL" => ComponentPropKind::Bool,
            "INSTANCE_SWAP" => ComponentPropKind::InstanceSwap,
            // A VARIANT prop selects along the axis named by the prop's `name`.
            "VARIANT" => ComponentPropKind::Variant { axis: name.clone() },
            _ => continue,
        };
        // Default value from the prop's `varValue` (a `VariableData`); a SWAP/
        // VARIANT default may be a guid/string we still capture as a String so
        // `select_set_variant` can read it.
        let default = d
            .get("varValue")
            .and_then(read_var_value)
            .or_else(|| {
                // VARIANT defaults sometimes sit as a plain string elsewhere.
                d.get("varValue")
                    .and_then(|v| v.get("value"))
                    .and_then(|v| v.get("textValue"))
                    .and_then(KiwiValue::as_str)
                    .map(|s| VarValue::String {
                        value: s.to_owned(),
                    })
            })
            .unwrap_or(VarValue::String {
                value: String::new(),
            });
        let pid = ComponentPropId::new();
        prop_guid_to_id.insert(def_guid, (cid, pid));
        out.push(ComponentPropDef {
            id: pid,
            name,
            kind,
            formatter: Default::default(),
            default,
            bindings: Vec::new(),
        });
    }
    out
}

/// Parse a Figma variant name like `"State=Hover, Size=Large"` into an
/// axis-name → value map. Returns `None` when the name carries no `=` pair (a
/// plain component, not a variant).
pub(crate) fn parse_variant_axes(name: &str) -> Option<std::collections::BTreeMap<String, String>> {
    if !name.contains('=') {
        return None;
    }
    let mut map = BTreeMap::new();
    for part in name.split(',') {
        let part = part.trim();
        if let Some((axis, value)) = part.split_once('=') {
            let axis = axis.trim();
            let value = value.trim();
            if !axis.is_empty() {
                map.insert(axis.to_owned(), value.to_owned());
            }
        }
    }
    if map.is_empty() { None } else { Some(map) }
}
