//! The pass-4 driver: resolve every instance's symbolOverrides, prop
//! assignments, derivedSymbolData, nested-instance routing, and swaps.

use super::{
    BoundProp, ComponentId, ComponentMaps, Doc, HashMap, KiwiValue, MapReport, NodeData, NodeId,
    Override, OverridePath, OverrideValue, PendingInstanceOverrides, PropRefKind, VarValue,
    build_stroke, build_swap_redirects, guid_key, has_stroke_fields, master_guid_paths,
    master_root_for, prop_assignment_text, read_derived_override, read_fills,
    resolve_full_guid_path, text_override,
};

/// Resolve each instance's override material to typed [`Override`]s addressed by
/// def-local path, and push them onto the [`InstanceNode`] so
/// [`fanta_doc::resolve::expand_instance`] applies them at render/export time.
///
/// Two Figma mechanisms feed an instance's content (both confirmed against the
/// Adobe Spectrum file):
///
/// 1. **`symbolData.symbolOverrides`** — a `NodeChange[]`, each addressing a
///    master descendant by a `guidPath` (a path of master-descendant guids) and
///    carrying the overridden fields inline (`textData.characters`,
///    `fillPaints`, `visible`).
/// 2. **`componentPropAssignments`** — the instance's exposed component-property
///    values (`defID → value.textValue`). A master descendant *binds* a prop-def
///    via its `componentPropRefs` (e.g. a TEXT node whose TEXT_DATA is driven by
///    prop-def `defID`); the assignment's value flows to every such descendant.
///    This is how the Spectrum "Action Bar" header sets its title/description.
/// 3. **`derivedSymbolData`** (field 125) — Figma's baked, fully-resolved
///    per-descendant render data: a `NodeChange[]` where each entry's `guidPath`
///    addresses one master descendant and carries the RESOLVED
///    `size`/`transform`/`fillGeometry`/`strokeGeometry`/`strokeWeight`/
///    `derivedTextData` for *this* placement. Decoded into typed
///    [`fanta_doc::node::DerivedOverride`]s ([`read_derived_override`]) and pushed
///    onto the instance so `expand_instance` renders it exactly as Figma resolved
///    it (the light master no longer leaks through on dark-theme instances).
///
/// The target master descendant is resolved to its def-local [`OverridePath`]
/// (the original master `NodeId`s from the def root's child down to the target,
/// root excluded) — the exact path `expand_instance` matches against.
///
/// **Nested instances.** A `guidPath` of length 1 targets a direct master
/// descendant and resolves at this level as above (the common case: 26.7k of the
/// 28.4k override entries in the Spectrum fixture). A `guidPath` of length > 1
/// crosses into a *nested* instance's own master subtree — its leading guids name
/// the nested instances along the chain and its terminal guid names a descendant
/// of the deepest nested master. We resolve the FULL path across masters into a
/// single flat [`OverridePath`] (each segment is the def-local path within one
/// master, concatenated); [`fanta_doc::resolve::expand_instance`] then peels the
/// matched-nested-instance prefix and re-attaches the remainder onto the cloned
/// nested instance so it applies on recursion. Mirrors op1's per-sub-instance
/// nested override/derived maps (and op2 `frame-converter.ts` nested routing),
/// adapted to our flattened-master + def-local-path model.
///
/// **Instance swaps.** An override carrying `overriddenSymbolID` SWAPS which
/// component a (nested) instance points to. We resolve the swapped symbol guid
/// to its [`ComponentId`] and emit a
/// [`fanta_doc::node::OverrideValue::SwapInstance`] addressed at the path to the
/// (nested) instance, so `expand_instance` re-points it and the right nested
/// master expands (op1 `symbol/overrides.ts`).
///
/// **Main-vs-published duality.** Figma roots a `guidPath` at the SYMBOL the
/// instance references, which surfaces two cases [`resolve_full_guid_path`] must
/// handle (both diagnosed against the Spectrum fixture; together ~16.3k entries
/// that previously resolved to nothing):
///
/// 1. A length-1 path whose single guid IS the master root addresses the
///    EXPANSION ROOT (def-local path `[]`) — the instance's own resolved surface
///    fill / baked root size — not a descendant. `build_master_guid_paths`
///    excludes the root, so it used to drop these (~14.5k).
/// 2. A path that descends through a nested instance which a sibling
///    `overriddenSymbolID` override SWAPPED: the trailing segments address the
///    SWAPPED master's descendants, not the declared `symbolID`'s. We thread a
///    per-instance swap-redirect map (keyed by the source-guid prefix → swapped
///    master root) so the descent enters the swapped master (~1.85k).
///
/// A target / swap that doesn't resolve (guid not in scene, not under any master,
/// no binding) is skipped, never fatal.
pub(crate) fn apply_instance_overrides(
    doc: &mut Doc,
    report: &mut MapReport,
    pending: &[PendingInstanceOverrides],
    node_prop_refs: &HashMap<String, Vec<(String, PropRefKind)>>,
    guid_to_node: &HashMap<String, Option<NodeId>>,
    maps: &ComponentMaps,
    blobs: &[Vec<u8>],
) {
    let symbol_guid_to_component = &maps.guid_to_component;
    let prop_guid_to_id = &maps.prop_guid_to_id;
    // Per-master `guid → def-local-path` cache, keyed by master root NodeId, so
    // the cross-master path walk (and repeated instances of the same master)
    // don't rebuild the same small map. A component master is dozens of nodes.
    let mut path_cache: HashMap<NodeId, HashMap<String, OverridePath>> = HashMap::new();

    for po in pending {
        // The instance must still exist (not dropped as virtual content).
        let Some(component) = instance_component(doc, po.instance) else {
            continue;
        };
        // Resolve the master root this instance expands against. For a member
        // def: its root. For a set: the default member's root. Mirrors
        // `expand_instance`'s def resolution but we only need the root NodeId.
        let Some(master_root) = master_root_for(doc, component) else {
            continue;
        };

        let mut overrides: Vec<Override> = Vec::new();

        // SWAP MAP for the main-vs-published nested duality. A nested instance can
        // be SWAPPED to a different variant by a sibling `overriddenSymbolID`
        // override; its *declared* `symbolID` still names the original (published)
        // master, but the OTHER overrides whose `guidPath` descends THROUGH that
        // instance address descendants of the SWAPPED master. So when
        // `resolve_full_guid_path` descends into a nested instance, it must enter
        // the swapped master — not the declared one — or the segment guid won't be
        // found and the whole override is dropped (the ~1,854 fixture entries the
        // duality diagnosis pinned). We key the swap by the joined SOURCE guidPath
        // prefix (the guids naming the nested instance), mapping it to the swapped
        // component's master ROOT NodeId, and consult it during descent. The swap
        // override ITSELF resolves at the level above (its terminal guid is the
        // instance), so it's emitted as a `SwapInstance` exactly as before; this
        // map only redirects the descent for the *content* overrides that cross it.
        let swap_redirects: HashMap<String, NodeId> =
            build_swap_redirects(&po.symbol_overrides, symbol_guid_to_component, |cid| {
                master_root_for(doc, cid)
            });

        apply_symbol_overrides(
            doc,
            report,
            po,
            master_root,
            guid_to_node,
            symbol_guid_to_component,
            &swap_redirects,
            &mut path_cache,
            &mut overrides,
        );
        apply_own_surface_fill(doc, po, master_root, &mut overrides);
        apply_own_surface_strokes(doc, po, master_root, &mut overrides);
        apply_prop_assignments(
            doc,
            report,
            po,
            master_root,
            guid_to_node,
            node_prop_refs,
            symbol_guid_to_component,
            &mut path_cache,
            &mut overrides,
        );
        apply_prop_defaults(
            doc,
            report,
            po,
            component,
            master_root,
            guid_to_node,
            node_prop_refs,
            prop_guid_to_id,
            &mut path_cache,
            &mut overrides,
        );
        let derived = apply_derived_overrides(
            doc,
            report,
            po,
            master_root,
            guid_to_node,
            &swap_redirects,
            &mut path_cache,
            blobs,
        );

        commit_instance_overrides(doc, report, po.instance, overrides, derived);
    }

    tally_merged_surfaces(doc, report, pending);
}

/// The [`ComponentId`] of the still-present instance at `id`, or `None` if it was
/// dropped as virtual content / is no longer an instance.
fn instance_component(doc: &Doc, id: NodeId) -> Option<ComponentId> {
    match doc.scene.get(id).map(|n| &n.data) {
        Some(NodeData::Instance(i)) => Some(i.component),
        _ => None,
    }
}

/// Mechanism 1 — `symbolData.symbolOverrides`: each entry addresses a master
/// descendant by a `guidPath` (resolved across nested masters into one flat
/// def-local path) and carries an inline swap, text, fill, and/or visibility
/// override.
#[allow(clippy::too_many_arguments)]
fn apply_symbol_overrides(
    doc: &Doc,
    report: &mut MapReport,
    po: &PendingInstanceOverrides,
    master_root: NodeId,
    guid_to_node: &HashMap<String, Option<NodeId>>,
    symbol_guid_to_component: &HashMap<String, ComponentId>,
    swap_redirects: &HashMap<String, NodeId>,
    path_cache: &mut HashMap<NodeId, HashMap<String, OverridePath>>,
    overrides: &mut Vec<Override>,
) {
    for ov in &po.symbol_overrides {
        let guids = ov
            .get("guidPath")
            .and_then(|p| p.get("guids"))
            .and_then(KiwiValue::as_array);
        let path_len = guids.map(<[KiwiValue]>::len).unwrap_or(0);
        tally_path_len(report, path_len);

        // Resolve the FULL guidPath across nested-instance masters into one flat
        // def-local path. For length 1 this is the direct descendant; for length >
        // 1 it spans into nested masters and `expand_instance` routes the
        // remainder. An `overriddenSymbolID` swap targets the LAST instance in the
        // path, so resolve a path-to-the-instance for it too.
        let Some(guids) = guids else { continue };
        let path = resolve_full_guid_path(
            doc,
            master_root,
            guid_to_node,
            guids,
            swap_redirects,
            path_cache,
        );

        // Instance swap (overriddenSymbolID) — re-points a (nested) instance.
        if let Some(swap_guid) = ov.get("overriddenSymbolID").and_then(guid_key) {
            report.overridden_symbol_swaps += 1;
            if let (Some(path), Some(&swap_cid)) =
                (path.as_ref(), symbol_guid_to_component.get(&swap_guid))
            {
                overrides.push(Override {
                    target_path: path.clone(),
                    target_prop: BoundProp::Visible, // unused for a swap value
                    value: OverrideValue::SwapInstance {
                        component: swap_cid,
                    },
                });
                report.overridden_symbol_resolved += 1;
            }
        }

        let Some(path) = path else {
            continue;
        };
        if path_len > 1 {
            report.override_nested_resolved += 1;
        }
        // Text content override.
        if let Some(text) = ov
            .get("textData")
            .and_then(|t| t.get("characters"))
            .and_then(KiwiValue::as_str)
        {
            overrides.push(text_override(path.clone(), text));
        }
        // Fill override — the FULL stacked paint set, emitted UNCONDITIONALLY (not
        // only when textData is absent). A frame/instance override often carries
        // both text and a (theme) fill; keeping both means a dark header keeps its
        // dark background *and* its relabeled text. The override entry's paints are
        // already style-resolved by the pre-pass.
        let ov_fills = if should_emit_symbol_fill_override(ov) {
            read_fills(ov)
        } else {
            Vec::new()
        };
        if !ov_fills.is_empty() {
            overrides.push(Override {
                target_path: path.clone(),
                target_prop: BoundProp::FillColor { index: 0 },
                value: OverrideValue::Fills {
                    fills: ov_fills.into_iter().collect(),
                },
            });
        }
        if has_stroke_fields(ov) {
            overrides.push(Override {
                target_path: path.clone(),
                target_prop: BoundProp::StrokeColor { index: 0 },
                value: OverrideValue::Strokes {
                    strokes: build_stroke(ov).into_iter().collect(),
                },
            });
        }
        // Visibility override.
        if let Some(KiwiValue::Bool(visible)) = ov.get("visible") {
            overrides.push(Override {
                target_path: path.clone(),
                target_prop: BoundProp::Visible,
                value: OverrideValue::Visible { value: *visible },
            });
        }
    }
}

fn should_emit_symbol_fill_override(override_change: &KiwiValue) -> bool {
    !is_text_layout_snapshot_paint(override_change)
}

fn is_text_layout_snapshot_paint(override_change: &KiwiValue) -> bool {
    // Figma's text-layout NodeChange entries can include the node's current
    // fillPaints as snapshot state. Without a paint-style ref or text-content
    // change, those paints are not an authored recolor.
    let carries_text_layout = override_change.get("textAlignHorizontal").is_some()
        || override_change.get("textAlignVertical").is_some()
        || override_change.get("textAutoResize").is_some();
    if !carries_text_layout {
        return false;
    }

    override_change.get("textData").is_none()
        && override_change.get("styleIdForFill").is_none()
        && override_change.get("styleIdForText").is_none()
}

/// Count a guidPath by length: len ≤ 1 is a direct descendant, len > 1 crosses a
/// nested-instance boundary.
fn tally_path_len(report: &mut MapReport, path_len: usize) {
    if path_len <= 1 {
        report.override_path_len1 += 1;
    } else {
        report.override_path_nested += 1;
    }
}

/// Mechanism 1b — the instance's OWN surface fill (`mergeSymbolProps`). op2 merges
/// the instance's own `fillPaints` onto the inlined frame, where the instance
/// value takes priority over the master's. We emit it as a root-targeted (`[]`
/// def-local path) fill override so `expand_instance` repaints the expanded master
/// root's surface with the instance's resolved color. This is the headline fix: a
/// dark `_Header` instance carries its own (style-resolved) `#1D1D1D` here while
/// its light master root is white.
///
/// Guard: only when the master root is itself a surface-bearing frame (a `Group`
/// with a `background`). That keeps the blast radius to instances that already
/// paint a surface — we recolor it, we never invent one — and matches
/// `mergeSymbolProps`'s own clip guard. The Light `_Header` resolves to the same
/// white its master carries, so the override is a visual no-op there.
fn apply_own_surface_fill(
    doc: &Doc,
    po: &PendingInstanceOverrides,
    master_root: NodeId,
    overrides: &mut Vec<Override>,
) {
    if !po.own_fills.is_empty()
        && matches!(
            doc.scene.get(master_root).map(|n| &n.data),
            Some(NodeData::Group(g)) if g.background.is_some()
        )
    {
        overrides.push(Override {
            target_path: OverridePath::new(), // the expanded master root
            target_prop: BoundProp::FillColor { index: 0 },
            value: OverrideValue::Fills {
                fills: po.own_fills.iter().cloned().collect(),
            },
        });
    }
}

fn apply_own_surface_strokes(
    doc: &Doc,
    po: &PendingInstanceOverrides,
    master_root: NodeId,
    overrides: &mut Vec<Override>,
) {
    let Some(strokes) = &po.own_strokes else {
        return;
    };
    if matches!(
        doc.scene.get(master_root).map(|n| &n.data),
        Some(NodeData::Group(g)) if !g.strokes.is_empty()
    ) {
        overrides.push(Override {
            target_path: OverridePath::new(),
            target_prop: BoundProp::StrokeColor { index: 0 },
            value: OverrideValue::Strokes {
                strokes: strokes.iter().cloned().collect(),
            },
        });
    }
}

/// Mechanism 2 — `componentPropAssignments` (defID → value). Figma component
/// properties drive descendants through componentPropRefs: a TEXT prop changes
/// bound text content, a BOOL prop toggles visibility, and an INSTANCE_SWAP prop
/// replaces a nested instance's component.
#[allow(clippy::too_many_arguments)]
fn apply_prop_assignments(
    doc: &Doc,
    report: &mut MapReport,
    po: &PendingInstanceOverrides,
    master_root: NodeId,
    guid_to_node: &HashMap<String, Option<NodeId>>,
    node_prop_refs: &HashMap<String, Vec<(String, PropRefKind)>>,
    symbol_guid_to_component: &HashMap<String, ComponentId>,
    path_cache: &mut HashMap<NodeId, HashMap<String, OverridePath>>,
    overrides: &mut Vec<Override>,
) {
    let direct_paths = master_guid_paths(doc, master_root, guid_to_node, path_cache);
    for cpa in &po.prop_assignments {
        let Some(def_guid) = cpa.get("defID").and_then(guid_key) else {
            continue;
        };
        let value = cpa.get("value");
        let text = prop_assignment_text(cpa);
        let bool_val = value
            .and_then(|v| v.get("boolValue"))
            .and_then(|b| match b {
                KiwiValue::Bool(x) => Some(*x),
                _ => None,
            });
        let swap_cid = value
            .and_then(|v| v.get("guidValue"))
            .and_then(guid_key)
            .and_then(|g| symbol_guid_to_component.get(&g).copied());

        for (target_guid, path) in direct_paths {
            let Some(refs) = node_prop_refs.get(target_guid) else {
                continue;
            };
            for (d, kind) in refs {
                if d != &def_guid {
                    continue;
                }
                push_assignment_override(
                    report,
                    overrides,
                    kind,
                    path,
                    text.as_deref(),
                    bool_val,
                    swap_cid,
                );
            }
        }
    }
}

/// Emit the override for one matched `componentPropAssignment` (text / visibility
/// / instance-swap), if the assignment carries the value that `kind` expects.
fn push_assignment_override(
    report: &mut MapReport,
    overrides: &mut Vec<Override>,
    kind: &PropRefKind,
    path: &OverridePath,
    text: Option<&str>,
    bool_val: Option<bool>,
    swap_cid: Option<ComponentId>,
) {
    match kind {
        PropRefKind::Text => {
            if let Some(t) = text {
                overrides.push(text_override(path.clone(), t));
            }
        }
        PropRefKind::Visible => {
            if let Some(v) = bool_val {
                overrides.push(Override {
                    target_path: path.clone(),
                    target_prop: BoundProp::Visible,
                    value: OverrideValue::Visible { value: v },
                });
                report.prop_visible_resolved += 1;
            }
        }
        PropRefKind::InstanceSwap => {
            if let Some(cid) = swap_cid {
                overrides.push(Override {
                    target_path: path.clone(),
                    target_prop: BoundProp::Visible,
                    value: OverrideValue::SwapInstance { component: cid },
                });
                report.prop_instance_swap_resolved += 1;
            }
        }
    }
}

/// Mechanism 2b — component-property DEFAULTS for props the instance left unset.
/// When an instance leaves an exposed prop unset, Figma resolves the descendant
/// binding to the prop default from `componentPropDefs`.
#[allow(clippy::too_many_arguments)]
fn apply_prop_defaults(
    doc: &Doc,
    report: &mut MapReport,
    po: &PendingInstanceOverrides,
    component: ComponentId,
    master_root: NodeId,
    guid_to_node: &HashMap<String, Option<NodeId>>,
    node_prop_refs: &HashMap<String, Vec<(String, PropRefKind)>>,
    prop_guid_to_id: &HashMap<String, (ComponentId, fanta_doc::ComponentPropId)>,
    path_cache: &mut HashMap<NodeId, HashMap<String, OverridePath>>,
    overrides: &mut Vec<Override>,
) {
    let assigned_prop_guids: std::collections::HashSet<String> = po
        .prop_assignments
        .iter()
        .filter_map(|cpa| cpa.get("defID").and_then(guid_key))
        .collect();

    let mut prop_def_defaults: HashMap<String, (fanta_doc::ComponentPropKind, VarValue)> =
        HashMap::new();
    if let Some(def) = doc.components.def(component) {
        for (guid, (cid, pid)) in prop_guid_to_id {
            if *cid != component || assigned_prop_guids.contains(guid) {
                continue;
            }
            if let Some(p) = def.props.iter().find(|p| p.id == *pid) {
                prop_def_defaults.insert(guid.clone(), (p.kind.clone(), p.default.clone()));
            }
        }
    }
    if prop_def_defaults.is_empty() {
        return;
    }

    let direct_paths = master_guid_paths(doc, master_root, guid_to_node, path_cache);
    for (target_guid, path) in direct_paths {
        let Some(refs) = node_prop_refs.get(target_guid) else {
            continue;
        };
        for (d, kind) in refs {
            let Some((_prop_kind, default)) = prop_def_defaults.get(d) else {
                continue;
            };
            push_default_override(report, overrides, kind, path, default);
        }
    }
}

/// Emit the override for one matched component-property default (text /
/// visibility), if the default carries a usable value.
fn push_default_override(
    report: &mut MapReport,
    overrides: &mut Vec<Override>,
    kind: &PropRefKind,
    path: &OverridePath,
    default: &VarValue,
) {
    match kind {
        PropRefKind::Text => {
            if let VarValue::String { value } = default
                && !value.is_empty()
            {
                overrides.push(text_override(path.clone(), value));
                report.prop_defaults_applied += 1;
            }
        }
        PropRefKind::Visible => {
            if let VarValue::Boolean { value } = default {
                overrides.push(Override {
                    target_path: path.clone(),
                    target_prop: BoundProp::Visible,
                    value: OverrideValue::Visible { value: *value },
                });
                report.prop_defaults_applied += 1;
            }
        }
        PropRefKind::InstanceSwap => {}
    }
}

/// Mechanism 3 — `derivedSymbolData` (Figma's baked, fully-resolved
/// per-descendant render data). Each entry is a `NodeChange` addressing a master
/// descendant by a `guidPath` (resolved across nested masters exactly like
/// overrides) and carrying its RESOLVED size/transform/geometry/text. A length >
/// 1 entry routes onto the nested instance on recursion.
#[allow(clippy::too_many_arguments)]
fn apply_derived_overrides(
    doc: &Doc,
    report: &mut MapReport,
    po: &PendingInstanceOverrides,
    master_root: NodeId,
    guid_to_node: &HashMap<String, Option<NodeId>>,
    swap_redirects: &HashMap<String, NodeId>,
    path_cache: &mut HashMap<NodeId, HashMap<String, OverridePath>>,
    blobs: &[Vec<u8>],
) -> Vec<fanta_doc::node::DerivedOverride> {
    let mut derived: Vec<fanta_doc::node::DerivedOverride> = Vec::new();
    for d in &po.derived_symbol_data {
        let guids = d
            .get("guidPath")
            .and_then(|p| p.get("guids"))
            .and_then(KiwiValue::as_array);
        let path_len = guids.map(<[KiwiValue]>::len).unwrap_or(0);
        tally_path_len(report, path_len);
        let Some(guids) = guids else { continue };
        let Some(path) = resolve_full_guid_path(
            doc,
            master_root,
            guid_to_node,
            guids,
            swap_redirects,
            path_cache,
        ) else {
            continue;
        };
        if path_len > 1 {
            report.override_nested_resolved += 1;
        }
        if let Some(over) = read_derived_override(path, d, blobs, report) {
            derived.push(over);
        }
    }
    derived
}

/// Push the resolved `overrides` + `derived` onto the instance node and update
/// the report's instance/override/derived counters.
fn commit_instance_overrides(
    doc: &mut Doc,
    report: &mut MapReport,
    instance: NodeId,
    overrides: Vec<Override>,
    derived: Vec<fanta_doc::node::DerivedOverride>,
) {
    if !overrides.is_empty() {
        report.instances_with_overrides += 1;
        report.overrides_applied += overrides.len();
    }
    if !derived.is_empty() {
        report.instances_with_derived += 1;
        report.derived_overrides_applied += derived.len();
    }
    if overrides.is_empty() && derived.is_empty() {
        return;
    }
    if let Some(node) = doc.scene.get_mut(instance) {
        if let NodeData::Instance(inst) = &mut node.data {
            if !overrides.is_empty() {
                inst.overrides = overrides;
            }
            if !derived.is_empty() {
                inst.derived = derived;
            }
        }
    }
}

/// mergeSymbolProps surface tally (op2 `mergeSymbolProps` / op1 sync): count
/// instances whose resolved master root carries a background fill — i.e. the
/// instances whose inherited surface `expand_instance` now paints at their box.
fn tally_merged_surfaces(doc: &Doc, report: &mut MapReport, pending: &[PendingInstanceOverrides]) {
    for po in pending {
        let Some(component) = instance_component(doc, po.instance) else {
            continue;
        };
        if let Some(root) = master_root_for(doc, component) {
            if matches!(
                doc.scene.get(root).map(|n| &n.data),
                Some(NodeData::Group(g)) if g.background.is_some()
            ) {
                report.instances_with_merged_surface += 1;
            }
        }
    }
}
