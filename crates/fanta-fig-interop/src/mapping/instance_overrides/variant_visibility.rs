//! Property-bound visibility on in-place component-set variant masters.

use super::{
    Doc, HashMap, MapReport, NodeFlags, NodeId, PropDefInfo, PropRefKind, parse_variant_axes,
    resolve_prop_def_default_bool, resolve_prop_def_name,
};

/// Resolve property-bound visibility on **in-place** component-set variant
/// masters.
///
/// A component-set variant member is a SYMBOL whose name carries `Axis=Value`
/// pairs (e.g. `… Label ?=True, Icon ?=False`). When such a master appears
/// directly on a design page (kept in place, not via an `INSTANCE`), Figma still
/// resolves each descendant whose `VISIBLE` is driven by a component property.
/// We reproduce that resolution at import:
///
/// - The bound prop's NAME is `"X ?"`. If `"X ?"` is one of this member's
///   variant AXES (present in its name), the child is shown iff that axis is
///   `True` — `Label ?=True` shows the Label, `Icon ?=False` hides the Icon.
/// - If the bound prop is NOT a variant axis, use the component prop's BOOL
///   default. Spectrum uses this for real content too: Coach Mark card images
///   are controlled by an `Image ?` visible prop whose default is `true`.
///
/// Instances are untouched: their visibility flows through
/// [`fanta_doc::resolve::expand_instance`] (a `Visible{false}` from a bool
/// assignment, or a `Visible{true}` that un-hides), which already works.
pub(crate) fn hide_master_variant_placeholders(
    doc: &mut Doc,
    report: &mut MapReport,
    variant_masters: &[(NodeId, String)],
    node_prop_refs: &HashMap<String, Vec<(String, PropRefKind)>>,
    guid_to_node: &HashMap<String, Option<NodeId>>,
    prop_def_infos: &HashMap<String, PropDefInfo>,
) {
    if variant_masters.is_empty() {
        return;
    }
    // NodeId → guid for every mapped node, so a master descendant resolves to the
    // `node_prop_refs` entry recorded under its guid in pass 1.
    let node_to_guid: HashMap<NodeId, &str> = guid_to_node
        .iter()
        .filter_map(|(g, opt)| opt.map(|id| (id, g.as_str())))
        .collect();

    for (root, name) in variant_masters {
        // The member's variant axes, keyed by axis name (e.g. `"Label ?"`). A
        // master with no `Axis=Value` name isn't a variant member — skip it.
        let Some(axes) = parse_variant_axes(name) else {
            continue;
        };
        // Walk the master subtree (descendants, root excluded). For each node
        // whose VISIBLE is driven by a prop ref, resolve the bound prop name and
        // decide show/hide.
        let descendants: Vec<NodeId> = doc.scene.descendants_of(*root).collect();
        for id in descendants {
            if id == *root {
                continue;
            }
            let Some(guid) = node_to_guid.get(&id) else {
                continue;
            };
            let Some(refs) = node_prop_refs.get(*guid) else {
                continue;
            };
            // A node can bind several refs; only its VISIBLE binding matters here.
            let Some((def_guid, _)) = refs.iter().find(|(_, k)| *k == PropRefKind::Visible) else {
                continue;
            };
            let prop_name = resolve_prop_def_name(def_guid, prop_def_infos);
            // Decide visibility from the variant axis when present, otherwise
            // from the prop's BOOL default. If neither is known, preserve the
            // imported node visibility instead of inventing a value.
            let visible = match prop_name.as_deref().and_then(|n| axes.get(n)) {
                Some(value) => Some(variant_value_truthy(value)),
                None => resolve_prop_def_default_bool(def_guid, prop_def_infos),
            };
            let Some(visible) = visible else {
                continue;
            };
            if let Some(node) = doc.scene.get_mut(id) {
                if visible {
                    node.flags.remove(NodeFlags::HIDDEN);
                } else if !node.flags.contains(NodeFlags::HIDDEN) {
                    node.flags |= NodeFlags::HIDDEN;
                    report.master_placeholders_hidden += 1;
                }
            }
        }
    }
}

/// Whether a Figma variant axis value string denotes the "on" state of a boolean
/// axis. Figma writes these as `"True"`/`"False"` (the `Label ?=True` form); a
/// few exporters use `Yes`/`On`/`1`. Anything else (a non-bool axis value like
/// `Standard`) is treated as not-on, but such props are excluded upstream by the
/// name-is-an-axis check, so this only ever sees a bool axis.
pub(crate) fn variant_value_truthy(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "true" | "yes" | "on" | "1"
    )
}
