//! Variable collections + variables (pass 4) and stable id hashing.

use super::{
    Doc, HashMap, KiwiValue, MapReport, Mode, ModeId, NodeData, NodeId, PendingCollection,
    PendingVariable, VarValue, Variable, VariableCollection, VariableCollectionId, VariableId,
    VariableType, guid_key, node_name, read_color,
};

/// Read a VARIABLE_SET's modes: (mode guid, mode name) in declaration order.
pub(crate) fn read_set_modes(change: &KiwiValue) -> Vec<(String, String)> {
    let Some(arr) = change.get("variableSetModes").and_then(KiwiValue::as_array) else {
        return Vec::new();
    };
    arr.iter()
        .filter_map(|m| {
            let guid = m.get("id").and_then(guid_key)?;
            let name = m
                .get("name")
                .and_then(KiwiValue::as_str)
                .unwrap_or("Mode")
                .to_owned();
            Some((guid, name))
        })
        .collect()
}

/// Read a VARIABLE node change into a [`PendingVariable`].
pub(crate) fn read_pending_variable(guid: &str, change: &KiwiValue) -> Option<PendingVariable> {
    let name = node_name(change);
    let set_guid = change
        .get("variableSetID")
        .and_then(|s| s.get("guid"))
        .and_then(guid_key);
    let ty = change
        .get("variableResolvedType")
        .and_then(KiwiValue::as_str)
        .map(resolved_type_to_variable_type)
        .unwrap_or(VariableType::String);

    let mut values: Vec<(String, VarValue)> = Vec::new();
    if let Some(entries) = change
        .get("variableDataValues")
        .and_then(|v| v.get("entries"))
        .and_then(KiwiValue::as_array)
    {
        for entry in entries {
            let Some(mode_guid) = entry.get("modeID").and_then(guid_key) else {
                continue;
            };
            if let Some(vv) = entry.get("variableData").and_then(read_var_value) {
                values.push((mode_guid, vv));
            }
        }
    }

    Some(PendingVariable {
        guid: guid.to_owned(),
        name,
        set_guid,
        ty,
        values,
    })
}

/// Map a Figma `VariableResolvedDataType` member to our [`VariableType`].
/// Types we don't model (MAP, GRADIENT, …) fall back to `String` so the
/// variable still imports rather than being dropped.
pub(crate) fn resolved_type_to_variable_type(member: &str) -> VariableType {
    match member {
        "BOOLEAN" => VariableType::Boolean,
        "FLOAT" => VariableType::Float,
        "COLOR" => VariableType::Color,
        _ => VariableType::String,
    }
}

/// Read a Figma `VariableData { value: VariableAnyValue, dataType, resolvedDataType }`
/// into a [`VarValue`]. Returns `None` for value kinds we don't model.
pub(crate) fn read_var_value(data: &KiwiValue) -> Option<VarValue> {
    let any = data.get("value")?;
    // Alias takes precedence — it's a reference, resolved later.
    if let Some(alias) = any
        .get("alias")
        .and_then(|a| a.get("guid"))
        .and_then(guid_key)
    {
        return Some(VarValue::Alias {
            variable: guid_to_variable_id(&alias),
        });
    }
    if let Some(c) = any.get("colorValue").and_then(|c| read_color(c, any)) {
        return Some(VarValue::Color { value: c });
    }
    if let Some(f) = any.get("floatValue").and_then(KiwiValue::as_f64) {
        return Some(VarValue::Float { value: f });
    }
    if let Some(KiwiValue::Bool(v)) = any.get("boolValue") {
        return Some(VarValue::Boolean { value: *v });
    }
    if let Some(s) = any.get("textValue").and_then(KiwiValue::as_str) {
        return Some(VarValue::String {
            value: s.to_owned(),
        });
    }
    None
}

/// The Figma-guid → Fantaisa-id maps minted while building the variable
/// collections. Returned by [`build_variables`] so a later pass can resolve a
/// node's `explicitVariableModes` (which names a collection + mode by guid)
/// onto a group's [`GroupNode::explicit_modes`].
///
/// [`GroupNode::explicit_modes`]: fanta_doc::node::GroupNode::explicit_modes
#[derive(Default)]
pub(crate) struct VariableGuidMaps {
    /// VARIABLE_SET node guid (`"sessionID:localID"`) → its collection id.
    pub(crate) coll_guid_to_id: HashMap<String, VariableCollectionId>,
    /// Mode guid (`"sessionID:localID"`) → its mode id, across all collections.
    pub(crate) mode_guid_to_id: HashMap<String, ModeId>,
}

/// Build [`Doc::variables`] from pending collections + variables. Returns the
/// guid→id maps minted along the way (see [`VariableGuidMaps`]).
pub(crate) fn build_variables(
    doc: &mut Doc,
    report: &mut MapReport,
    pending_collections: &[PendingCollection],
    pending_variables: &[PendingVariable],
) -> VariableGuidMaps {
    // guid string -> ids, so variables can reference their collection + mode.
    let mut coll_guid_to_id: HashMap<String, VariableCollectionId> = HashMap::new();
    let mut mode_guid_to_id: HashMap<String, ModeId> = HashMap::new();

    for pc in pending_collections {
        let cid = VariableCollectionId::new();
        coll_guid_to_id.insert(pc.guid.clone(), cid);
        let mut modes: Vec<Mode> = Vec::new();
        for (mguid, mname) in &pc.modes {
            let mid = ModeId::new();
            mode_guid_to_id.insert(mguid.clone(), mid);
            modes.push(Mode {
                id: mid,
                name: mname.clone(),
            });
        }
        // A collection needs at least one mode; synthesize a default if empty.
        if modes.is_empty() {
            modes.push(Mode {
                id: ModeId::new(),
                name: "Mode 1".to_owned(),
            });
        }
        let default_mode = modes[0].id;
        doc.variables.collections.insert(
            cid,
            VariableCollection {
                id: cid,
                name: pc.name.clone(),
                modes,
                default_mode,
                variable_order: Vec::new(),
            },
        );
        report.variable_collections += 1;
    }

    for pv in pending_variables {
        let var_id = guid_to_variable_id(&pv.guid);
        // Resolve the owning collection; if the variable names no set or an
        // unknown one, synthesize a fallback collection so the variable still
        // imports with a valid (non-dangling) collection ref.
        let coll_id = pv
            .set_guid
            .as_ref()
            .and_then(|g| coll_guid_to_id.get(g).copied())
            .unwrap_or_else(|| {
                // Reuse a single synthesized collection for all orphan variables.
                let fallback = VariableCollectionId::from_u128(ORPHAN_COLLECTION_U128);
                doc.variables
                    .collections
                    .entry(fallback)
                    .or_insert_with(|| {
                        report.variable_collections += 1;
                        let mode = Mode {
                            id: ModeId::from_u128(ORPHAN_MODE_U128),
                            name: "Mode 1".to_owned(),
                        };
                        let default_mode = mode.id;
                        VariableCollection {
                            id: fallback,
                            name: "Imported".to_owned(),
                            modes: vec![mode],
                            default_mode,
                            variable_order: Vec::new(),
                        }
                    });
                fallback
            });

        let default_mode = doc
            .variables
            .collections
            .get(&coll_id)
            .map(|c| c.default_mode)
            .unwrap_or(ModeId::from_u128(ORPHAN_MODE_U128));

        let mut values_by_mode = std::collections::BTreeMap::new();
        for (mode_guid, vv) in &pv.values {
            let mode_id = mode_guid_to_id
                .get(mode_guid)
                .copied()
                .unwrap_or(default_mode);
            values_by_mode.insert(mode_id, vv.clone());
        }

        doc.variables.variables.insert(
            var_id,
            Variable {
                id: var_id,
                collection: coll_id,
                name: pv.name.clone(),
                ty: pv.ty,
                values_by_mode,
                scopes: Vec::new(),
            },
        );
        if let Some(c) = doc.variables.collections.get_mut(&coll_id) {
            c.variable_order.push(var_id);
        }
        report.variables += 1;
    }

    VariableGuidMaps {
        coll_guid_to_id,
        mode_guid_to_id,
    }
}

/// Read a node's `explicitVariableModes` into `(collection-set guid, mode guid)`
/// pairs — Figma's per-frame mode pinning. Each entry of the array is a
/// `VariableModeBySet { variableSetID: GUID, modeID: GUID }`; the `variableSetID`
/// names the VARIABLE_SET node guid (the same key [`build_variables`] maps to a
/// collection id) and `modeID` names one of that set's modes.
///
/// The GUID extraction is LENIENT: a GUID may appear either bare (the
/// `sessionID`/`localID` struct directly, how `modeID` reads on a
/// `variableDataValues` entry) or wrapped in a `{ guid: GUID }` holder (how a
/// VARIABLE's own `variableSetID` reads, see [`read_pending_variable`]). We try
/// the bare form first, then the wrapped form, so the same reader handles both
/// shapes without depending on which the file's schema used. An entry missing
/// either guid is skipped. Returns `Vec::new()` when the field is absent/empty.
pub(crate) fn read_explicit_modes(change: &KiwiValue) -> Vec<(String, String)> {
    let Some(entries) = change
        .get("explicitVariableModes")
        .and_then(KiwiValue::as_array)
    else {
        return Vec::new();
    };
    // A GUID that is either the bare `sessionID`/`localID` struct or a
    // `{ guid: GUID }` wrapper around one.
    let guid_either = |v: &KiwiValue| -> Option<String> {
        guid_key(v).or_else(|| v.get("guid").and_then(guid_key))
    };
    entries
        .iter()
        .filter_map(|e| {
            let set = e.get("variableSetID").and_then(guid_either)?;
            let mode = e.get("modeID").and_then(guid_either)?;
            Some((set, mode))
        })
        .collect()
}

/// Apply captured `explicitVariableModes` pins onto the scene. For each pin
/// `(node, [(set guid, mode guid)])`, resolve the set + mode guids through the
/// [`VariableGuidMaps`] minted by [`build_variables`] and insert the resulting
/// `(collection id -> mode id)` onto the node's [`GroupNode::explicit_modes`].
/// Only group-like nodes carry that map (the resolver reads it there); a pin on
/// a non-group node, or one naming an unknown collection/mode, is silently
/// skipped (partial-import resilience). Counts each applied pin in the report.
///
/// [`GroupNode::explicit_modes`]: fanta_doc::node::GroupNode::explicit_modes
pub(crate) fn apply_explicit_modes(
    doc: &mut Doc,
    report: &mut MapReport,
    pending: &[(NodeId, Vec<(String, String)>)],
    maps: &VariableGuidMaps,
) {
    for (node_id, pins) in pending {
        let Some(node) = doc.scene.get_mut(*node_id) else {
            continue;
        };
        let NodeData::Group(g) = &mut node.data else {
            continue; // only groups (frames/sections/components) pin modes
        };
        for (set_guid, mode_guid) in pins {
            let (Some(&cid), Some(&mid)) = (
                maps.coll_guid_to_id.get(set_guid),
                maps.mode_guid_to_id.get(mode_guid),
            ) else {
                continue; // names a collection/mode we didn't import
            };
            g.explicit_modes.insert(cid, mid);
            report.explicit_modes_imported += 1;
        }
    }
}

/// Stable fallback collection/mode ids for variables whose set isn't known.
pub(crate) const ORPHAN_COLLECTION_U128: u128 = 0xFA00_0000_0000_0000_0000_0000_0000_0001;
pub(crate) const ORPHAN_MODE_U128: u128 = 0xFA00_0000_0000_0000_0000_0000_0000_0002;

/// Derive a deterministic [`VariableId`] from a Figma variable guid string, so
/// an alias referencing the same guid resolves to the same id without needing a
/// second lookup table. Uses a stable hash of the guid into the 128-bit space.
pub(crate) fn guid_to_variable_id(guid: &str) -> VariableId {
    VariableId::from_u128(stable_hash_u128(guid))
}

/// FNV-1a-ish stable 128-bit hash of a string. Deterministic across runs so
/// alias targets resolve to the same id as their definition.
pub(crate) fn stable_hash_u128(s: &str) -> u128 {
    // Two independent 64-bit FNV-1a streams combined into a u128.
    let mut hi: u64 = 0xcbf2_9ce4_8422_2325;
    let mut lo: u64 = 0x84222325cbf29ce4u64.rotate_left(17);
    for (i, b) in s.bytes().enumerate() {
        hi ^= b as u64;
        hi = hi.wrapping_mul(0x100_0000_01b3);
        lo ^= (b as u64).rotate_left((i % 31) as u32);
        lo = lo.wrapping_mul(0x100_0000_01b3);
    }
    // Avoid colliding with the reserved orphan ids.
    let v = ((hi as u128) << 64) | (lo as u128);
    if v == ORPHAN_COLLECTION_U128 || v == ORPHAN_MODE_U128 || v == 0 {
        v ^ 0xABCD
    } else {
        v
    }
}
