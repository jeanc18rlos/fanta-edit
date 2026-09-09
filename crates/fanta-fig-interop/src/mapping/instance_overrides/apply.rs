//! The pass-4 driver: resolve every instance's symbolOverrides, prop
//! assignments, derivedSymbolData, nested-instance routing, and swaps.

use super::{
    BoundProp, ComponentId, ComponentMaps, Doc, HashMap, KiwiValue, MapReport, MasterPathCache,
    NodeData, NodeId, Override, OverridePath, OverrideValue, PendingInstanceOverrides, PropRefKind,
    VarValue, apply_text_style_fields, blend_mode, build_content_and_style_runs, build_stroke,
    build_swap_redirects, corner_radii, corner_smoothing, guid_key, has_stroke_fields,
    master_root_for, prop_assignment_text, read_blurs, read_derived_override, read_effects,
    read_fills, read_size, read_transform, resolve_full_guid_path, text_case_of, text_override,
};
use fanta_doc::id::ComponentPropId;
use fanta_doc::{Fill, Stroke};
use std::collections::HashSet;

/// The master node's own fills and strokes at `node_id`, for detecting a
/// redundant override (see the drop in [`apply_symbol_overrides`]). A group's
/// single background counts as its fill.
fn master_paints(doc: &Doc, node_id: NodeId) -> (&[Fill], &[Stroke]) {
    match doc.scene.get(node_id).map(|node| &node.data) {
        Some(NodeData::Vector(v)) => (v.fills.as_slice(), v.strokes.as_slice()),
        Some(NodeData::Group(g)) => (g.background.as_slice(), g.strokes.as_slice()),
        _ => (&[], &[]),
    }
}

/// The serialized master nodes the snapshot-vs-authored rule compares override
/// fields against, keyed by master target. One override pass reads the same
/// few hundred master nodes tens of thousands of times (once per override
/// entry that carries a generic field), and nothing in the pass writes the
/// compared keys: `commit_instance_overrides` only sets an instance's
/// `overrides` / `derived`, so a serialization stays valid for the whole pass.
type MasterJsonCache = HashMap<NodeId, Option<serde_json::Value>>;

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
    pending: &[PendingInstanceOverrides<'_>],
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
    let mut path_cache = MasterPathCache::new(guid_to_node);
    let mut master_json_cache: MasterJsonCache = HashMap::new();
    // Each component's exposed props (prop-def guid + prop id), so the defaults
    // pass reads one component's props rather than every prop in the file for
    // every instance.
    let mut props_by_component: HashMap<ComponentId, Vec<(&str, ComponentPropId)>> = HashMap::new();
    for (guid, (component, prop)) in prop_guid_to_id {
        props_by_component
            .entry(*component)
            .or_default()
            .push((guid.as_str(), *prop));
    }

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

        // The source-guidPath keys `derivedSymbolData` covers (`>`-joined — the
        // same key form the swap-redirect map uses). The generic Field pass
        // skips `size`/`transform` for these paths: the derived pass already
        // carries Figma's baked resolved geometry for them (R2 — prefer baked
        // truth), so re-emitting the override entry's copy would only duplicate
        // the write.
        let derived_paths = derived_guid_path_keys(&po.derived_symbol_data);

        apply_symbol_overrides(
            doc,
            report,
            po,
            master_root,
            guid_to_node,
            symbol_guid_to_component,
            &swap_redirects,
            &derived_paths,
            &mut path_cache,
            &mut master_json_cache,
            &mut overrides,
        );
        apply_own_surface_fill(doc, po, master_root, &mut overrides);
        apply_own_surface_strokes(doc, po, master_root, &mut overrides);
        apply_prop_assignments(
            doc,
            report,
            po,
            master_root,
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
            node_prop_refs,
            props_by_component
                .get(&component)
                .map_or(&[][..], Vec::as_slice),
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
/// override — plus, since the F1 completeness fix, every REMAINING renderable
/// field the entry carries, collected into one generic
/// [`OverrideValue::Field`] per path (see [`push_field_override`]).
#[allow(clippy::too_many_arguments)]
fn apply_symbol_overrides(
    doc: &Doc,
    report: &mut MapReport,
    po: &PendingInstanceOverrides<'_>,
    master_root: NodeId,
    guid_to_node: &HashMap<String, Option<NodeId>>,
    symbol_guid_to_component: &HashMap<String, ComponentId>,
    swap_redirects: &HashMap<String, NodeId>,
    derived_paths: &HashSet<String>,
    path_cache: &mut MasterPathCache<'_>,
    master_json_cache: &mut MasterJsonCache,
    overrides: &mut Vec<Override>,
) {
    for ov in po.symbol_overrides {
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
        // The master node this override targets (empty path = the master root).
        // Figma bakes each instance's fully-resolved node state into its override
        // list, so a fill/stroke override that merely RESTATES the master's own
        // value is a snapshot, not an authored recolor. Keeping it would pin the
        // instance to the pre-edit look and stop a master edit from ever reaching
        // it. We drop those redundant snapshots below and keep only real deltas.
        let master_target = path.last().copied().unwrap_or(master_root);
        let (master_fills, master_strokes) = master_paints(doc, master_target);
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
        // Keep the fill override only when it actually differs from the master.
        if !ov_fills.is_empty() && ov_fills.as_slice() != master_fills {
            overrides.push(Override {
                target_path: path.clone(),
                target_prop: BoundProp::FillColor { index: 0 },
                value: OverrideValue::Fills {
                    fills: ov_fills.into_iter().collect(),
                },
            });
        }
        if has_stroke_fields(ov) {
            let ov_strokes = build_stroke(ov);
            if ov_strokes.as_slice() != master_strokes {
                overrides.push(Override {
                    target_path: path.clone(),
                    target_prop: BoundProp::StrokeColor { index: 0 },
                    value: OverrideValue::Strokes {
                        strokes: ov_strokes.into_iter().collect(),
                    },
                });
            }
        }
        // Visibility override.
        if let Some(KiwiValue::Bool(visible)) = ov.get("visible") {
            overrides.push(Override {
                target_path: path.clone(),
                target_prop: BoundProp::Visible,
                value: OverrideValue::Visible { value: *visible },
            });
        }
        // OV-1..OV-8 — everything else the entry carries that the typed arms
        // above don't consume, folded into ONE generic Field override per path.
        let derived_covers_path = joined_guid_key(guids)
            .map(|key| derived_paths.contains(&key))
            .unwrap_or(false);
        push_field_override(
            doc,
            report,
            ov,
            master_target,
            &path,
            derived_covers_path,
            master_json_cache,
            overrides,
        );
    }
}

/// The `>`-joined source-guid key of a `guidPath`'s guids (the same key form
/// [`build_swap_redirects`] uses), or `None` when any guid is malformed.
fn joined_guid_key(guids: &[KiwiValue]) -> Option<String> {
    let keys: Vec<String> = guids.iter().filter_map(guid_key).collect();
    (keys.len() == guids.len() && !keys.is_empty()).then(|| keys.join(">"))
}

/// The set of `>`-joined source-guidPath keys the instance's `derivedSymbolData`
/// entries address. Keyed on the RAW source path (not the resolved def-local
/// one) so overrides and derived entries that name the same target compare
/// without resolving the path twice.
fn derived_guid_path_keys(derived: &[KiwiValue]) -> HashSet<String> {
    derived
        .iter()
        .filter_map(|d| {
            let guids = d
                .get("guidPath")
                .and_then(|p| p.get("guids"))
                .and_then(KiwiValue::as_array)?;
            joined_guid_key(guids)
        })
        .collect()
}

/// OV-1..OV-8 (§3.1 / §5 F1 of docs/research/figma-parity-divergence.md) —
/// collect the REMAINING renderable fields of one override entry (everything
/// the typed swap/text/fill/stroke/visible arms don't consume) into ONE
/// generic [`OverrideValue::Field`] at `path`, reusing the exact field readers
/// node building uses. The Field value is a partial CanvasNode JSON object
/// that `expand_instance` serde-merges onto the clone, so this import pass and
/// the model's apply pass close over the same key set — the inverse of the old
/// double allow-list that dropped everything but five override kinds.
///
/// SNAPSHOT-vs-AUTHORED rule (the same one the fills/strokes arms apply
/// above): Figma bakes each instance's fully-resolved state into its override
/// entries, so a candidate key whose serialized value equals the master target
/// node's own is a baked snapshot, not an authored delta — kept, it would pin
/// the instance against master edits. Each candidate is compared against the
/// master's serialized form and dropped (counted) when equal; the Field
/// override is emitted only when at least one key survives.
#[allow(clippy::too_many_arguments)]
fn push_field_override(
    doc: &Doc,
    report: &mut MapReport,
    ov: &KiwiValue,
    master_target: NodeId,
    path: &OverridePath,
    derived_covers_path: bool,
    master_json_cache: &mut MasterJsonCache,
    overrides: &mut Vec<Override>,
) {
    let mut fields = serde_json::Map::new();
    collect_wrapper_fields(ov, &mut fields);
    collect_corner_fields(ov, &mut fields);
    if !derived_covers_path {
        collect_geometry_fields(doc, ov, master_target, &mut fields);
    }
    collect_text_fields(doc, ov, master_target, &mut fields);
    if fields.is_empty() {
        return;
    }

    let master_json = master_json_cache.entry(master_target).or_insert_with(|| {
        doc.scene
            .get(master_target)
            .and_then(|node| serde_json::to_value(node).ok())
    });
    let mut kept = serde_json::Map::new();
    for (key, value) in fields {
        if master_restates(master_json.as_ref(), &key, &value) {
            report.override_fields_dropped_snapshot += 1;
        } else {
            report.override_fields_applied += 1;
            kept.insert(key, value);
        }
    }
    if kept.is_empty() {
        return;
    }
    overrides.push(Override {
        target_path: path.clone(),
        target_prop: BoundProp::Visible, // unused for a Field value
        value: OverrideValue::Field {
            value: serde_json::Value::Object(kept),
        },
    });
}

/// OV-1 `opacity`, OV-2 `effects` → shadows + blurs, OV-4 `blendMode`.
/// Candidates are serialized through the SAME doc types the node builder
/// writes (`UnitInterval`, `Shadow`/`Blur` lists, `BlendMode`) so the snapshot
/// comparison sees identical JSON representations on both sides.
fn collect_wrapper_fields(ov: &KiwiValue, fields: &mut serde_json::Map<String, serde_json::Value>) {
    if let Some(opacity) = ov.get("opacity").and_then(KiwiValue::as_f64) {
        if let Ok(v) = serde_json::to_value(fanta_doc::style::UnitInterval::new(opacity as f32)) {
            fields.insert("opacity".to_owned(), v);
        }
    }
    // One Kiwi `effects` array carries shadows AND blurs; the doc model splits
    // them into two lists, so an effects override replaces BOTH (an emptied
    // list is an authored "remove the shadow", not an absence — the null/empty
    // handling in the snapshot rule keeps a no-op from pinning the instance).
    if ov.get("effects").is_some() {
        if let Ok(v) = serde_json::to_value(read_effects(ov)) {
            fields.insert("effects".to_owned(), v);
        }
        if let Ok(v) = serde_json::to_value(read_blurs(ov)) {
            fields.insert("blurs".to_owned(), v);
        }
    }
    if let Some(bm) = ov
        .get("blendMode")
        .and_then(KiwiValue::as_str)
        .and_then(blend_mode)
    {
        if let Ok(v) = serde_json::to_value(bm) {
            fields.insert("blend_mode".to_owned(), v);
        }
    }
}

/// OV-3 — corner rounding: `cornerRadius` / the four `rectangle*CornerRadius`
/// fields / `cornerSmoothing`, through the SAME readers node building uses
/// ([`corner_radii`] / [`corner_smoothing`]). BOTH radius keys are written
/// whenever the entry carries any radius field: `corner_radii` takes render
/// precedence over `corner_radius`, so an override that collapses a
/// mixed-corner master to a uniform radius — or squares it off entirely (the
/// reader yields `None` for 0) — must explicitly null the other key or the
/// master's value would win through the shallow merge.
fn collect_corner_fields(ov: &KiwiValue, fields: &mut serde_json::Map<String, serde_json::Value>) {
    let carries_radius = ov.get("cornerRadius").is_some()
        || ov.get("rectangleCornerRadiiIndependent").is_some()
        || [
            "rectangleTopLeftCornerRadius",
            "rectangleTopRightCornerRadius",
            "rectangleBottomRightCornerRadius",
            "rectangleBottomLeftCornerRadius",
        ]
        .iter()
        .any(|key| ov.get(key).is_some());
    if carries_radius {
        let (uniform, per_corner) = corner_radii(ov);
        if let Ok(v) = serde_json::to_value(uniform) {
            fields.insert("corner_radius".to_owned(), v);
        }
        if let Ok(v) = serde_json::to_value(per_corner) {
            fields.insert("corner_radii".to_owned(), v);
        }
    }
    if ov.get("cornerSmoothing").is_some() {
        if let Ok(v) = serde_json::to_value(corner_smoothing(ov)) {
            fields.insert("corner_smoothing".to_owned(), v);
        }
    }
}

/// OV-5 — `size` / `transform`, ONLY when no `derivedSymbolData` entry covers
/// this same path (checked by the caller): for covered paths the derived pass
/// already applies Figma's baked resolved geometry (R2 — prefer baked truth),
/// so this is the fallback for files where that cache is absent. `size` lands
/// on the serde key the master target's variant actually carries (`clip_size`
/// for a clipping frame, `local_size` for plain groups and box-shaped
/// variants); vectors/booleans are skipped — their box IS path geometry, which
/// a shallow field write can't resize (writing `local_size` would only clip).
fn collect_geometry_fields(
    doc: &Doc,
    ov: &KiwiValue,
    master_target: NodeId,
    fields: &mut serde_json::Map<String, serde_json::Value>,
) {
    if ov.get("transform").is_some() {
        if let Ok(v) = serde_json::to_value(read_transform(ov)) {
            fields.insert("transform".to_owned(), v);
        }
    }
    if ov.get("size").is_some() {
        let (w, h) = read_size(ov);
        let key = match doc.scene.get(master_target).map(|node| &node.data) {
            Some(NodeData::Group(g)) if g.clip_size.is_some() => Some("clip_size"),
            Some(NodeData::Group(_)) => Some("local_size"),
            Some(NodeData::Vector(_) | NodeData::Boolean(_)) | None => None,
            Some(_) => Some("local_size"),
        };
        if let Some(key) = key {
            if let Ok(v) = serde_json::to_value([w, h]) {
                fields.insert(key.to_owned(), v);
            }
        }
    }
}

/// OV-6 / OV-7 — text style scalars and per-run styles on the override entry,
/// for a TEXT master target only.
///
/// The `style` key carries a FULL [`TextStyle`] — the master's style with the
/// entry's scalars applied on top through [`apply_text_style_fields`] (the
/// same readers `build_text` uses; absent fields inherit the master's value).
/// Replacement-not-deep-merge matches the model's shallow-merge apply. Per-run
/// tables go through the same [`build_content_and_style_runs`] decoder master
/// text uses; whenever runs are emitted, the (case-transformed) `content` they
/// were computed against is emitted too — run byte offsets are only meaningful
/// against that exact string. A bare `textCase` transform likewise emits
/// `content` (the doc model applies textCase destructively to the string), and
/// because the Field override is pushed AFTER the typed Text arm, the
/// transformed copy wins at apply time.
fn collect_text_fields(
    doc: &Doc,
    ov: &KiwiValue,
    master_target: NodeId,
    fields: &mut serde_json::Map<String, serde_json::Value>,
) {
    let Some(NodeData::Text(master_text)) = doc.scene.get(master_target).map(|node| &node.data)
    else {
        return;
    };
    let carries_scalars = [
        "fontSize",
        "fontName",
        "letterSpacing",
        "lineHeight",
        "textDecoration",
    ]
    .iter()
    .any(|key| ov.get(key).is_some());
    let text_case = text_case_of(ov);
    let carries_runs = ov
        .get("textData")
        .map(|td| td.get("characterStyleIDs").is_some() && td.get("styleOverrideTable").is_some())
        .unwrap_or(false);
    if !carries_scalars && text_case.is_none() && !carries_runs {
        return;
    }

    // The overridden base style: master style + the entry's scalars.
    let mut style = master_text.style.clone();
    apply_text_style_fields(ov, &mut style);
    if carries_scalars {
        if let Ok(v) = serde_json::to_value(&style) {
            fields.insert("style".to_owned(), v);
        }
    }

    // The content the runs index into: the entry's own characters when it
    // carries them (the typed Text arm emitted that raw string; the Field copy
    // is applied after it), else the master's.
    let base_content = ov
        .get("textData")
        .and_then(|td| td.get("characters"))
        .and_then(KiwiValue::as_str)
        .unwrap_or(&master_text.content);
    let (content, style_runs) =
        build_content_and_style_runs(base_content, text_case, ov.get("textData"), &style);
    if carries_runs && !style_runs.is_empty() {
        if let Ok(v) = serde_json::to_value(&style_runs) {
            fields.insert("style_runs".to_owned(), v);
            if let Ok(c) = serde_json::to_value(&content) {
                fields.insert("content".to_owned(), c);
            }
        }
    } else if content != base_content {
        if let Ok(c) = serde_json::to_value(&content) {
            fields.insert("content".to_owned(), c);
        }
    }
}

/// Whether `candidate` at `key` merely RESTATES the master node's own
/// serialized value (so it is a baked snapshot to drop, not an authored
/// delta). An absent master key means the field sits at its serde default
/// (`skip_serializing_if`), so a candidate that IS that default — JSON null
/// (an `Option::None`), an empty list, or `corner_smoothing`'s skipped `0` —
/// is a restatement too. A missing master node can't be restated: keep the
/// key (nothing to pin against).
fn master_restates(
    master: Option<&serde_json::Value>,
    key: &str,
    candidate: &serde_json::Value,
) -> bool {
    let Some(master) = master else {
        return false;
    };
    match master.get(key) {
        Some(value) => value == candidate,
        None => {
            candidate.is_null()
                || candidate.as_array().is_some_and(Vec::is_empty)
                || (key == "corner_smoothing" && candidate.as_f64() == Some(0.0))
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
    po: &PendingInstanceOverrides<'_>,
    master_root: NodeId,
    overrides: &mut Vec<Override>,
) {
    if po.own_fills.is_empty() {
        return;
    }
    let Some(NodeData::Group(g)) = doc.scene.get(master_root).map(|node| &node.data) else {
        return;
    };
    // Only recolor a surface-bearing frame; never invent one.
    let Some(background) = &g.background else {
        return;
    };
    // A surface fill equal to the master's own background is a snapshot no-op:
    // emitting it would pin the instance and block a master fill edit from ever
    // reaching it. Keep it only when the instance genuinely differs (e.g. a dark
    // instance over a light master).
    let own: Vec<Fill> = po.own_fills.iter().cloned().collect();
    if own.as_slice() == std::slice::from_ref(background) {
        return;
    }
    overrides.push(Override {
        target_path: OverridePath::new(), // the expanded master root
        target_prop: BoundProp::FillColor { index: 0 },
        value: OverrideValue::Fills {
            fills: own.into_iter().collect(),
        },
    });
}

fn apply_own_surface_strokes(
    doc: &Doc,
    po: &PendingInstanceOverrides<'_>,
    master_root: NodeId,
    overrides: &mut Vec<Override>,
) {
    let Some(strokes) = &po.own_strokes else {
        return;
    };
    let Some(NodeData::Group(g)) = doc.scene.get(master_root).map(|node| &node.data) else {
        return;
    };
    if g.strokes.is_empty() {
        return;
    }
    // Same snapshot-no-op drop as the surface fill above.
    let own: Vec<Stroke> = strokes.iter().cloned().collect();
    if own.as_slice() == g.strokes.as_slice() {
        return;
    }
    overrides.push(Override {
        target_path: OverridePath::new(),
        target_prop: BoundProp::StrokeColor { index: 0 },
        value: OverrideValue::Strokes {
            strokes: own.into_iter().collect(),
        },
    });
}

/// Mechanism 2 — `componentPropAssignments` (defID → value). Figma component
/// properties drive descendants through componentPropRefs: a TEXT prop changes
/// bound text content, a BOOL prop toggles visibility, and an INSTANCE_SWAP prop
/// replaces a nested instance's component.
#[allow(clippy::too_many_arguments)]
fn apply_prop_assignments(
    doc: &Doc,
    report: &mut MapReport,
    po: &PendingInstanceOverrides<'_>,
    master_root: NodeId,
    node_prop_refs: &HashMap<String, Vec<(String, PropRefKind)>>,
    symbol_guid_to_component: &HashMap<String, ComponentId>,
    path_cache: &mut MasterPathCache<'_>,
    overrides: &mut Vec<Override>,
) {
    let direct_paths = path_cache.master_guid_paths(doc, master_root);
    for cpa in po.prop_assignments {
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
/// binding to the prop default from `componentPropDefs`. `component_props` is
/// this component's `(prop-def guid, prop id)` list.
#[allow(clippy::too_many_arguments)]
fn apply_prop_defaults(
    doc: &Doc,
    report: &mut MapReport,
    po: &PendingInstanceOverrides<'_>,
    component: ComponentId,
    master_root: NodeId,
    node_prop_refs: &HashMap<String, Vec<(String, PropRefKind)>>,
    component_props: &[(&str, ComponentPropId)],
    path_cache: &mut MasterPathCache<'_>,
    overrides: &mut Vec<Override>,
) {
    if component_props.is_empty() {
        return;
    }
    let assigned_prop_guids: HashSet<String> = po
        .prop_assignments
        .iter()
        .filter_map(|cpa| cpa.get("defID").and_then(guid_key))
        .collect();

    let mut prop_def_defaults: HashMap<String, (fanta_doc::ComponentPropKind, VarValue)> =
        HashMap::new();
    if let Some(def) = doc.components.def(component) {
        for (guid, pid) in component_props {
            if assigned_prop_guids.contains(*guid) {
                continue;
            }
            if let Some(p) = def.props.iter().find(|p| p.id == *pid) {
                prop_def_defaults.insert((*guid).to_owned(), (p.kind.clone(), p.default.clone()));
            }
        }
    }
    if prop_def_defaults.is_empty() {
        return;
    }

    let direct_paths = path_cache.master_guid_paths(doc, master_root);
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
    po: &PendingInstanceOverrides<'_>,
    master_root: NodeId,
    guid_to_node: &HashMap<String, Option<NodeId>>,
    swap_redirects: &HashMap<String, NodeId>,
    path_cache: &mut MasterPathCache<'_>,
    blobs: &[Vec<u8>],
) -> Vec<fanta_doc::node::DerivedOverride> {
    let mut derived: Vec<fanta_doc::node::DerivedOverride> = Vec::new();
    for d in po.derived_symbol_data {
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
fn tally_merged_surfaces(
    doc: &Doc,
    report: &mut MapReport,
    pending: &[PendingInstanceOverrides<'_>],
) {
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
