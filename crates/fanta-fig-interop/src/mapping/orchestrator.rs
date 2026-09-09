//! The top-level `fig_to_doc` orchestrator: the multi-pass build that walks
//! the decoded `KiwiValue` tree into a Fantaisa scene + side-tables, plus the
//! pending side-table records and parent-resolution helper it drives.

use super::{
    AssetId, BoundProp, CanvasNode, Doc, FigDocument, FigError, FigResult, Fill, GroupNode,
    HashMap, IndexKey, KiwiValue, MapReport, NodeBuild, NodeData, NodeId, PendingMotion,
    PropDefInfo, PropRefKind, Stroke, VarValue, VariableId, VariableType, apply_bindings,
    apply_explicit_modes, apply_instance_overrides, apply_motion, apply_paint_color_bindings,
    apply_reactions, asset_id_for_image, build_components_and_sets, build_node, build_stroke,
    build_variables, clips_content, collect_motion_change, collect_motion_consumers,
    collect_prop_def_infos, guid_key, hide_master_variant_placeholders, image_hash_hex,
    is_state_group, node_name, populate_instance_prop_values, read_component_prop_refs,
    read_explicit_modes, read_fills, read_paint, read_paint_color_bindings, read_pending_variable,
    read_prop_defs_raw, read_set_modes, resolve_style_references, tally_fidelity,
};
use std::collections::HashSet;

/// Map a parsed `.fig` document into a Fantaisa [`Doc`].
///
/// Returns the doc, a [`MapReport`], and the embedded-image asset map — every
/// `AssetId` an [`Fill::Image`] in the doc references, paired with the raw
/// PNG/JPEG bytes from the `.fig`'s `images/<hash>` entry. The app installs an
/// `AssetResolver` over this map so image fills render real pixels; a referenced
/// hash with no matching ZIP entry is simply absent (the renderer falls back to
/// its placeholder). The doc is always returned even if some nodes were
/// skipped — partial import beats no import.
pub fn fig_to_doc(fig: &FigDocument) -> FigResult<(Doc, MapReport, HashMap<AssetId, Vec<u8>>)> {
    let raw_node_changes = fig
        .root
        .get("nodeChanges")
        .and_then(KiwiValue::as_array)
        .ok_or_else(|| FigError::Mapping("document has no `nodeChanges` array".to_owned()))?;

    // PRE-PASS — resolve shared-style references. Figma stores shared styles as
    // separate NodeChanges (each carrying a `styleType` of FILL/TEXT/EFFECT and a
    // paint/text/effect payload); a *consuming* node carries only a
    // `styleIdFor{Fill,StrokeFill,Text,Effect}` ref and an EMPTY paint array. We
    // inline each style's payload into a copy of each consumer (and into every
    // `symbolData.symbolOverrides[]` entry), so the downstream readers see real
    // paints/text instead of an empty array; changes with no style ref stay
    // borrowed from `fig`. Mirrors op2's `resolveStyleReferences`
    // (figma-node-mapper.ts:25-90).
    let (node_changes, style_report) = resolve_style_references(raw_node_changes);

    let mut doc = Doc::new();
    let mut report = MapReport {
        style_def_count: style_report.style_def_count,
        style_ref_empty_fill: style_report.ref_empty_fill,
        style_ref_resolved_fill: style_report.resolved_fill,
        ..MapReport::default()
    };

    // guid string -> the Fantaisa node we created for it (None if recognized as
    // a structural-only parent like DOCUMENT, or if skipped).
    let mut guid_to_node: HashMap<String, Option<NodeId>> = HashMap::new();
    // guid string -> parent guid string, kept for the re-parent pass so we can
    // climb over skipped intermediates.
    let mut guid_to_parent: HashMap<String, Option<String>> = HashMap::new();
    // guid string -> its `parentIndex.position` — Figma's authoritative
    // fractional-index sibling order (a string compared LEXICOGRAPHICALLY, not
    // numerically). Pass 2 sorts each parent's children by this so the resulting
    // `IndexKey` z-order reflects Figma stacking instead of the (unrelated)
    // NodeChange STREAM order. A missing position is `None` and tie-breaks to
    // stream order. See `attach_order`.
    let mut guid_to_position: HashMap<String, Option<String>> = HashMap::new();
    // The order nodes were created, so attachment is deterministic.
    let mut order: Vec<String> = Vec::new();
    // Every node pass 1 built, parentless until pass 2 has resolved each
    // parent and z-index and inserts the whole document into the scene in one
    // batch. Boxed as `build_node` hands them over, so the map holds pointers
    // rather than shuffling the nodes themselves.
    let mut built: HashMap<NodeId, Box<CanvasNode>> = HashMap::new();
    // Node ids of CANVAS-derived nodes, in document order, registered as pages
    // once the scene is fully assembled.
    let mut page_ids: Vec<NodeId> = Vec::new();

    // ---- side-tables (resolved + applied after the scene is assembled) ----
    let mut pending = Pending::default();
    // The document-level prototype start node guid, if any.
    let mut flow_start_guid: Option<String> = None;
    // Named flow entry points (Figma `flowStartingPoints`, in author order)
    // and the presentation device, first-seen wins.
    let mut flow_guids: Vec<(String, String)> = Vec::new();
    let mut presentation: Option<fanta_doc::PresentationConfig> = None;

    // ---- pass 1: create nodes ----
    for change in node_changes.iter().map(|change| -> &KiwiValue { change }) {
        let guid = match change.get("guid").and_then(guid_key) {
            Some(g) => g,
            None => {
                report.malformed += 1;
                continue;
            }
        };
        let parent = change
            .get("parentIndex")
            .and_then(|p| p.get("guid"))
            .and_then(guid_key);
        // Figma's `parentIndex.position` fractional-index string (sibling order).
        let position = change
            .get("parentIndex")
            .and_then(|p| p.get("position"))
            .and_then(KiwiValue::as_str)
            .map(str::to_owned);
        guid_to_parent.insert(guid.clone(), parent);
        guid_to_position.insert(guid.clone(), position);
        order.push(guid.clone());

        let type_name = change
            .get("type")
            .and_then(KiwiValue::as_str)
            .unwrap_or("NONE");

        // The document node carries the prototype start id for the whole flow.
        if type_name == "DOCUMENT" {
            if let Some(g) = change.get("prototypeStartNodeID").and_then(guid_key) {
                flow_start_guid = Some(g);
            }
        }
        // Named multi-flow entry points + the presentation device live on the
        // CANVAS changes (per page). Every named flow is collected — the old
        // behavior kept only a single last-wins start. ASSUMPTION: kiwi field
        // spellings (`flowStartingPoints[].{nodeID,name}`, `prototypeDevice`)
        // mirror the REST shapes; pin against a real fixture when available.
        if matches!(type_name, "CANVAS" | "DOCUMENT") {
            if let Some(points) = change
                .get("flowStartingPoints")
                .and_then(KiwiValue::as_array)
            {
                for point in points {
                    let Some(node_guid) = point
                        .get("nodeID")
                        .or_else(|| point.get("nodeId"))
                        .and_then(guid_key)
                    else {
                        continue;
                    };
                    let name = point
                        .get("name")
                        .and_then(KiwiValue::as_str)
                        .unwrap_or("Flow")
                        .to_owned();
                    if !flow_guids.iter().any(|(_, g)| *g == node_guid) {
                        flow_guids.push((name, node_guid));
                    }
                }
            }
            if presentation.is_none()
                && let Some(device) = change.get("prototypeDevice")
            {
                let size = device.get("size").map(|s| {
                    [
                        s.get("x").and_then(KiwiValue::as_f64).unwrap_or(0.0),
                        s.get("y").and_then(KiwiValue::as_f64).unwrap_or(0.0),
                    ]
                });
                let config = fanta_doc::PresentationConfig {
                    device_size: size.filter(|[w, h]| *w > 0.0 && *h > 0.0),
                    preset: device
                        .get("presetIdentifier")
                        .and_then(KiwiValue::as_str)
                        .filter(|p| !p.is_empty())
                        .map(str::to_owned),
                    landscape: matches!(
                        device.get("rotation").and_then(KiwiValue::as_str),
                        Some("CCW_90" | "CW_90" | "LANDSCAPE")
                    ),
                    frame_color: device
                        .get("frameColor")
                        .and_then(|c| super::read_color_with_opacity(c, 1.0)),
                };
                // A device of type NONE carries nothing renderable — skip it.
                if config.device_size.is_some()
                    || config.preset.is_some()
                    || config.frame_color.is_some()
                {
                    presentation = Some(config);
                }
            }
        }

        // Keyframe-motion changes (ANIMATION_PRESET_INSTANCE / KEYFRAME_TRACK /
        // KEYFRAME) are animation DATA, not scene content: collect them into
        // the motion side-table (resolved onto `doc.motion` in pass 4) and
        // record the guid as structural. They are no longer counted as skipped.
        if collect_motion_change(&mut pending.motion, type_name, &guid, change) {
            guid_to_node.insert(guid, None);
            continue;
        }

        match build_node(type_name, change, &fig.blobs) {
            NodeBuild::Node {
                node,
                is_page,
                geometry_decoded,
            } => {
                let id = node.id;
                if node.constraints.is_some() {
                    report.constraints_imported += 1;
                }
                guid_to_node.insert(guid.clone(), Some(id));
                if is_page {
                    page_ids.push(id);
                }
                report.mapped += 1;
                tally_recovered_geometry(&mut report, &node, type_name, geometry_decoded);
                tally_fidelity(&mut report, change, Some(&node));
                // Count frame-like containers whose "clip content" toggle is OFF
                // (`frameMaskDisabled == true`), so they import with their box
                // intact and `meta.clip_content=false`. Counted off the source
                // `change` + `type_name`; SECTION is excluded because it is
                // always an unclipped organizational container.
                if matches!(
                    type_name,
                    "FRAME" | "SYMBOL" | "COMPONENT" | "COMPONENT_SET" | "INSTANCE"
                ) && !clips_content(change)
                {
                    report.frames_clip_disabled += 1;
                }

                collect_typed_side_tables(&mut pending, &mut report, type_name, &guid, id, change);
                collect_per_node_side_tables(&mut pending, &guid, id, change);
                built.insert(id, node);
            }
            NodeBuild::Structural => {
                guid_to_node.insert(guid, None);
            }
            NodeBuild::Unsupported => {
                guid_to_node.insert(guid, None);
                *report
                    .skipped_by_type
                    .entry(type_name.to_owned())
                    .or_default() += 1;
            }
        }
    }
    let Pending {
        components: pending_components,
        sets: pending_sets,
        instances: pending_instances,
        collections: pending_collections,
        variables: pending_variables,
        reactions: pending_reactions,
        bindings: pending_bindings,
        paint_bindings: pending_paint_bindings,
        explicit_modes: pending_explicit_modes,
        instance_overrides: pending_instance_overrides,
        node_prop_refs,
        prop_def_infos,
        variant_masters,
        motion: pending_motion,
    } = pending;

    // Which NodeIds are instances — so pass 2 can prune their virtual subtrees.
    // An INSTANCE only becomes `NodeData::Instance` when it named a component;
    // one without a symbol ref fell back to a Group and keeps its real children.
    let all_instance_ids: HashSet<NodeId> = built
        .iter()
        .filter(|(_, node)| matches!(node.data, NodeData::Instance(_)))
        .map(|(id, _)| *id)
        .collect();

    // ---- pass 2: attach to parents ----
    //
    // The order we *attach* siblings in is what mints their `IndexKey` z-order
    // (each sibling planned under a parent lands ABOVE the last one planned
    // there — higher key = painted later = on top).
    // Figma's `.fig` stores nodes in an arbitrary STREAM order that is unrelated
    // to stacking; the authoritative sibling order lives in
    // `parentIndex.position` (a fractional-index STRING, compared
    // lexicographically). So we visit nodes in `attach_order`: each Figma
    // parent's children sorted ascending by `position` (ties + missing positions
    // fall back to stable stream order), while the parent GROUPS keep stream
    // order. Ascending position → ascending `IndexKey` → the highest-position
    // sibling ends up topmost, matching Figma + the OpenPencil oracle (its
    // `sortChildren` sorts children ascending by `position`, then paints them
    // first-to-last so the last/highest-position child is on top).
    let attach_order = attach_order(&order, &guid_to_parent, &guid_to_position);
    attach_to_parents(
        &mut doc,
        &mut report,
        built,
        &attach_order,
        &guid_to_parent,
        &mut guid_to_node,
        &all_instance_ids,
    )?;

    // ---- pass 3: register pages + relocate component masters ----
    for &id in &page_ids {
        doc.add_page(id);
    }

    // The hidden "Components" page hosts every SYMBOL master subtree, so the app
    // can hide it from the normal page switcher. Created lazily — only if there
    // is at least one component/set to route under it.
    let needs_components_page = !pending_components.is_empty() || !pending_sets.is_empty();
    let components_page: Option<NodeId> = if needs_components_page {
        let mut page = CanvasNode::new(NodeData::Group(GroupNode::default()));
        page.name = COMPONENTS_PAGE_NAME.to_owned();
        page.meta = serde_json::json!({ "figma_type": "CANVAS", "hidden_page": true });
        let id = page.id;
        page.index = doc.scene.next_root_index();
        doc.scene
            .insert(*Box::new(page))
            .map_err(|e| FigError::Mapping(e.to_string()))?;
        doc.add_page(id);
        Some(id)
    } else {
        None
    };

    // Relocate detached masters (and component-set roots) under the Components
    // page so orphaned library definitions stay out of regular root rendering. A
    // component-set root is itself a master container holding its variant
    // members, so moving the set root also moves its members; skip a member
    // whose set was already relocated (its parent is no longer a scene root) to
    // avoid a redundant move.
    //
    // Masters that are on a Figma CANVAS render IN PLACE. The Figma canvas shows
    // page-level component masters beside frames (e.g. design-system building
    // blocks), and the read-only `.fig` viewer must match that page. Either way
    // the `ComponentDef` still points at the master's NodeId, so
    // `expand_instance` finds it wherever it lives.
    if let Some(page) = components_page {
        relocate_masters(
            &mut doc,
            &mut report,
            page,
            &page_ids,
            &pending_components,
            &pending_sets,
        )?;
    }

    // Default to the most substantial *design* page (never the hidden Components
    // page) rather than whatever CANVAS happened to be first.
    if let Some(&biggest) = page_ids
        .iter()
        .max_by_key(|id| doc.scene.descendants_of(**id).count())
    {
        doc.set_active_page(Some(biggest));
    }

    // ---- pass 4: build side-tables ----
    let component_maps = build_components_and_sets(
        &mut doc,
        &mut report,
        &pending_components,
        &pending_sets,
        &pending_instances,
    );
    // Populate each instance's typed `prop_values` from its
    // `componentPropAssignments`, keyed by the prop schema we just built. This is
    // the schema-aware view of an instance's chosen props (text/bool/swap/variant)
    // that drives variant selection and prop defaults.
    populate_instance_prop_values(
        &mut doc,
        &mut report,
        &pending_instance_overrides,
        &component_maps.prop_guid_to_id,
    );
    // Instance overrides (text/fill/visibility/swap) — resolves each instance's
    // symbolOverrides + componentPropAssignments to def-local override paths
    // (full guidPath, routing nested-instance crossings) and pushes them onto
    // the InstanceNode so `expand_instance` applies them.
    apply_instance_overrides(
        &mut doc,
        &mut report,
        &pending_instance_overrides,
        &node_prop_refs,
        &guid_to_node,
        &component_maps,
        &fig.blobs,
    );
    // Hide property-bound placeholder affordances on in-place component-set
    // variant masters (the spurious-▪/▾ fix). A `VISIBLE`-bound child whose prop
    // is a truthy variant axis of THIS member stays visible; a non-axis bool
    // placeholder (e.g. `Hold Icon ?`, `Asterisk ?`) has no per-variant supplier
    // in a static master render, so it is hidden — matching the reference.
    hide_master_variant_placeholders(
        &mut doc,
        &mut report,
        &variant_masters,
        &node_prop_refs,
        &guid_to_node,
        &prop_def_infos,
    );
    let var_maps = build_variables(
        &mut doc,
        &mut report,
        &pending_collections,
        &pending_variables,
    );
    // Per-frame mode pins, now that the collection/mode guid→id maps exist.
    apply_explicit_modes(&mut doc, &mut report, &pending_explicit_modes, &var_maps);
    apply_reactions(
        &mut doc,
        &mut report,
        &pending_reactions,
        &guid_to_node,
        &component_maps,
    );
    apply_bindings(&mut doc, &mut report, &pending_bindings, &pending_variables);
    apply_paint_color_bindings(&mut doc, &mut report, &pending_paint_bindings);
    // Keyframe motion: resolve the collected preset/track/keyframe material +
    // consumer bindings onto `doc.motion` now that node existence is settled.
    apply_motion(&mut doc, &mut report, &pending_motion);

    // Document-level prototype start frame.
    if let Some(g) = flow_start_guid {
        if let Some(Some(id)) = guid_to_node.get(&g) {
            doc.flow_start = Some(*id);
        }
    }
    // Named flows: resolve each collected starting point; the first flow also
    // backs `flow_start` when the document didn't carry a single start of its
    // own (multi-flow files often have no document-level start at all).
    for (name, guid) in flow_guids {
        if let Some(Some(id)) = guid_to_node.get(&guid) {
            doc.flows.push(fanta_doc::Flow { name, start: *id });
        }
    }
    if doc.flow_start.is_none()
        && let Some(first) = doc.flows.first()
    {
        doc.flow_start = Some(first.start);
    }
    doc.presentation = presentation;

    // Embedded-image assets: pair every referenced `AssetId` with its bytes from
    // the `.fig` ZIP, so the app can install a resolver. Done after the scene is
    // assembled by re-reading the source paints (the `AssetId` is a pure function
    // of the image hash, so this reproduces exactly the ids `read_paint` minted).
    let assets = collect_image_assets(
        node_changes.iter().map(|change| -> &KiwiValue { change }),
        &fig.images,
    );
    report.image_assets_extracted = assets.len();

    Ok((doc, report, assets))
}

/// Pass 2 — resolve each recognized node's parent and z-index in
/// `attach_order`, then insert the whole document into the scene as one
/// batch. The order siblings are planned in mints their `IndexKey` z-order,
/// so this visits nodes in Figma's authoritative sibling order. A node inside
/// an instance's virtual subtree (or under a non-container) is dropped —
/// partial import beats a hard failure — and takes any descendant planned
/// before it along, exactly as removing its subtree from a live scene would.
fn attach_to_parents(
    doc: &mut Doc,
    report: &mut MapReport,
    mut built: HashMap<NodeId, Box<CanvasNode>>,
    attach_order: &[String],
    guid_to_parent: &HashMap<String, Option<String>>,
    guid_to_node: &mut HashMap<String, Option<NodeId>>,
    all_instance_ids: &HashSet<NodeId>,
) -> FigResult<()> {
    let mut plan = AttachPlan::default();
    for guid in attach_order {
        let Some(Some(node_id)) = guid_to_node.get(guid).copied() else {
            continue; // structural/unsupported/skipped — nothing to attach
        };
        match resolve_parent(guid, guid_to_parent, guid_to_node, all_instance_ids) {
            ParentResolution::Node(parent_id) => {
                // The resolved parent should be a container, but a malformed /
                // unusual chain can land on a non-container (e.g. an instance
                // the pre-scan missed) or on a node already dropped with its
                // subtree; rather than abort the whole import, that node is
                // treated as virtual instance content and dropped.
                let parent_accepts_children = built
                    .get(&parent_id)
                    .is_some_and(|parent| parent.can_have_children());
                if parent_accepts_children {
                    plan.attach(&mut built, node_id, Some(parent_id));
                } else {
                    drop_virtual_node(report, guid, node_id, &mut built, &mut plan, guid_to_node);
                }
            }
            ParentResolution::Root => plan.attach(&mut built, node_id, None),
            ParentResolution::InsideInstance => {
                // Inside an instance's virtual subtree — drop it. The expansion
                // (`expand_instance`) reproduces this content from the master.
                drop_virtual_node(report, guid, node_id, &mut built, &mut plan, guid_to_node);
            }
        }
    }
    // A node the loop above never reached: pass 1 built it, but a LATER change
    // sharing its guid (a motion change, a structural node, an unsupported one)
    // overwrote `guid_to_node[guid]` with `None`, so the loop skipped it and it
    // is in neither `plan.order` nor the dropped set. Before pass 2 planned the
    // whole batch, pass 1 inserted every built node at the root and only
    // re-parented what it visited, so these landed as root layers; keep that
    // rather than discarding content with no counter. Sorted so the order is
    // the pass-1 build order (ULIDs are minted in sequence) and not the
    // `built` map's iteration order.
    let mut unplanned: Vec<NodeId> = built
        .keys()
        .copied()
        .filter(|id| !plan.planned.contains(id))
        .collect();
    unplanned.sort_unstable();
    for id in unplanned {
        plan.attach(&mut built, id, None);
    }
    // A dropped subtree's descendants are still listed in `plan.order`; they
    // left `built` when their ancestor was dropped, so the filter skips them.
    let batch = plan
        .order
        .into_iter()
        .filter_map(|id| built.remove(&id))
        .map(|node| *node);
    doc.scene
        .insert_many(batch)
        .map_err(|e| FigError::Mapping(e.to_string()))?;
    Ok(())
}

/// Pass 2's stand-in for the live scene while parents and z-indexes are
/// resolved: which nodes attach under which parent, in what order, and the
/// key the next sibling under each parent takes — what
/// `Scene::next_child_index` would answer had the nodes already been
/// inserted one by one.
#[derive(Default)]
struct AttachPlan {
    /// Planned nodes in attach order — the batch handed to `insert_many`.
    order: Vec<NodeId>,
    /// Planned children per parent, so dropping a parent drops the subtree
    /// planned under it.
    children: HashMap<NodeId, Vec<NodeId>>,
    /// The highest `IndexKey` minted under each parent (`None` = root) so far.
    last_index: HashMap<Option<NodeId>, IndexKey>,
    /// Every node the plan has decided on — attached or dropped. What is built
    /// but in neither set was never visited, and would otherwise be discarded
    /// silently when the batch is filtered out of `order`.
    planned: HashSet<NodeId>,
}

impl AttachPlan {
    fn attach(
        &mut self,
        built: &mut HashMap<NodeId, Box<CanvasNode>>,
        id: NodeId,
        parent: Option<NodeId>,
    ) {
        let Some(node) = built.get_mut(&id) else {
            return;
        };
        self.planned.insert(id);
        let index = self
            .last_index
            .get(&parent)
            .copied()
            .map(IndexKey::after)
            .unwrap_or(IndexKey::FIRST);
        node.parent = parent;
        node.index = index;
        self.last_index.insert(parent, index);
        self.order.push(id);
        if let Some(parent) = parent {
            self.children.entry(parent).or_default().push(id);
        }
    }

    /// Forget `root` and every node planned under it, transitively.
    fn drop_subtree(&mut self, built: &mut HashMap<NodeId, Box<CanvasNode>>, root: NodeId) {
        let mut stack = vec![root];
        while let Some(id) = stack.pop() {
            self.planned.insert(id);
            built.remove(&id);
            if let Some(children) = self.children.remove(&id) {
                stack.extend(children);
            }
        }
    }
}

/// Drop a node that lives in an instance's virtual subtree (or under a
/// non-container) together with whatever was planned under it, update the
/// mapped/dropped counters, and forget its guid→id mapping so later passes
/// don't reference a node that never reached the scene.
fn drop_virtual_node(
    report: &mut MapReport,
    guid: &str,
    node_id: NodeId,
    built: &mut HashMap<NodeId, Box<CanvasNode>>,
    plan: &mut AttachPlan,
    guid_to_node: &mut HashMap<String, Option<NodeId>>,
) {
    plan.drop_subtree(built, node_id);
    report.mapped = report.mapped.saturating_sub(1);
    report.instance_children_dropped += 1;
    guid_to_node.insert(guid.to_owned(), None);
}

/// Relocate component masters (and component-set roots) under the hidden
/// Components `page` so they don't render on the design pages — except masters
/// embedded in design content, which render IN PLACE (Figma fidelity) and are
/// only counted. Sets are relocated first (they carry their variant members),
/// then standalone components; a node already living under the Components page is
/// skipped.
fn relocate_masters(
    doc: &mut Doc,
    report: &mut MapReport,
    page: NodeId,
    page_ids: &[NodeId],
    pending_components: &[PendingComponent<'_>],
    pending_sets: &[PendingSet],
) -> FigResult<()> {
    let page_set: HashSet<NodeId> = page_ids.iter().copied().collect();
    let master_roots: HashSet<NodeId> = pending_sets
        .iter()
        .map(|ps| ps.root)
        .chain(pending_components.iter().map(|pc| pc.root))
        .collect();
    for ps in pending_sets {
        relocate_master(doc, report, ps.root, page, &page_set, &master_roots)?;
    }
    for pc in pending_components {
        relocate_master(doc, report, pc.root, page, &page_set, &master_roots)?;
    }
    Ok(())
}

/// Relocate one detached master `root` under the Components `page`, unless it is
/// already there or visible on a Figma canvas (left in place and counted as
/// `masters_kept_in_place`).
fn relocate_master(
    doc: &mut Doc,
    report: &mut MapReport,
    root: NodeId,
    page: NodeId,
    page_set: &HashSet<NodeId>,
    master_roots: &HashSet<NodeId>,
) -> FigResult<()> {
    let scene = &mut doc.scene;
    // Don't relocate a node already living under the Components page (e.g. a
    // variant member whose set root we just moved).
    let is_under_components = scene.ancestors_of(root).any(|a| a.id == page);
    if scene.get(root).is_none() || is_under_components {
        return Ok(());
    }
    if !is_library_master(root, page, page_set, master_roots, scene) {
        // Embedded in design content — leave it in place so it renders where the
        // designer put it (Figma fidelity).
        report.masters_kept_in_place += 1;
        return Ok(());
    }
    let idx = scene.next_child_index(Some(page));
    scene
        .set_parent(root, Some(page), idx)
        .map_err(|e| FigError::Mapping(e.to_string()))?;
    Ok(())
}

/// Whether a master `root` is detached library data (relocate to the Components
/// page) rather than visible Figma-canvas content (render in place).
/// `master_roots` is every component master and set root.
fn is_library_master(
    root: NodeId,
    page: NodeId,
    page_set: &HashSet<NodeId>,
    master_roots: &HashSet<NodeId>,
    scene: &fanta_doc::scene::Scene,
) -> bool {
    // The master's immediate scene parent. `None` means it is a detached root,
    // so keep it off regular root rendering. A visible CANVAS page parent means
    // Figma renders it on that page, so keep it in place. Another master
    // container parent is relocated with its container.
    let Some(parent) = scene.get(root).and_then(|n| n.parent) else {
        return true;
    };
    if parent == page {
        return true;
    }
    if page_set.contains(&parent) {
        return false;
    }
    master_roots.contains(&parent)
}

/// The side-table material collected during pass 1, resolved and applied after
/// the scene is assembled. Grouping these keeps `fig_to_doc`'s pass-1 loop and
/// the collection helpers from threading a dozen separate `&mut` locals. Raw
/// Kiwi material is borrowed from the (style-resolved) node changes for the
/// import's lifetime `'a` rather than cloned: an instance's `derivedSymbolData`
/// alone is a large share of a real file's bytes.
#[derive(Default)]
struct Pending<'a> {
    /// Component master definitions: (symbol guid, master root NodeId, name,
    /// prop defs).
    components: Vec<PendingComponent<'a>>,
    /// Component sets: state-group SYMBOL / COMPONENT_SET roots.
    sets: Vec<PendingSet>,
    /// Instances: (instance NodeId, symbol guid string) to wire the component ref
    /// after we know guid -> ComponentId.
    instances: Vec<(NodeId, String)>,
    /// Variable collections (from VARIABLE_SET).
    collections: Vec<PendingCollection>,
    /// Variables (from VARIABLE).
    variables: Vec<PendingVariable>,
    /// Per-node prototype reactions.
    reactions: Vec<(NodeId, &'a [KiwiValue])>,
    /// Per-node variable bindings (the consumption map).
    bindings: Vec<(NodeId, &'a KiwiValue)>,
    /// Per-node paint-level color bindings.
    paint_bindings: Vec<(NodeId, Vec<(BoundProp, VariableId)>)>,
    /// Per-frame variable-mode pins (`explicitVariableModes`): (node id,
    /// [(collection-set guid, mode guid)]). Resolved in pass 4 once the variable
    /// collections — and their guid→id maps — exist.
    explicit_modes: Vec<(NodeId, Vec<(String, String)>)>,
    /// Instance overrides: the raw `symbolData.symbolOverrides` array and the
    /// top-level `componentPropAssignments` array per instance, applied in pass 4
    /// once the master subtree + guid→NodeId map are settled.
    instance_overrides: Vec<PendingInstanceOverrides<'a>>,
    /// Every node's `componentPropRefs` — which prop-def guid drives which
    /// property of that node — keyed by the node's own guid string. Lets pass 4
    /// resolve an instance's `componentPropAssignments[defID → value]` to the
    /// master descendant it sets.
    node_prop_refs: HashMap<String, Vec<(String, PropRefKind)>>,
    /// Every `componentPropDef`'s name + parent guid, across ALL nodes, so the
    /// master-placeholder pass can resolve a `VISIBLE` ref's bound prop NAME
    /// through the `parentPropDefId` chain (a variant member's own def is unnamed;
    /// the name lives on the set-level parent it inherits from).
    prop_def_infos: HashMap<String, PropDefInfo>,
    /// SYMBOL/COMPONENT masters whose NAME carries `Axis=Value` variant pairs (a
    /// component-set member): (master root NodeId, name). Drives the in-place
    /// placeholder-hiding pass.
    variant_masters: Vec<(NodeId, String)>,
    /// Keyframe-motion material: ANIMATION_PRESET_INSTANCE / KEYFRAME_TRACK /
    /// KEYFRAME changes plus the consuming nodes' KEYFRAME-expression bindings
    /// and timeline durations. Resolved onto [`Doc::motion`] in pass 4.
    motion: PendingMotion,
}

/// Tally a freshly-built node's geometry-recovery counters: decoded-vs-fallback
/// geometry (splitting out the `vector_network` blob fallback by `meta`) and the
/// VECTOR-family recovery count.
fn tally_recovered_geometry(
    report: &mut MapReport,
    node: &CanvasNode,
    type_name: &str,
    geometry_decoded: bool,
) {
    if geometry_decoded {
        report.geometry_decoded += 1;
        // STEP 2b vectorNetworkBlob fallbacks are tagged in `meta` so the fixture
        // report can split them out of `geometry_decoded`.
        if node.meta.get("geometry").and_then(|g| g.as_str()) == Some("vector_network") {
            report.vector_network_decoded += 1;
        }
    }
    if matches!(
        type_name,
        "VECTOR" | "STAR" | "LINE" | "BOOLEAN_OPERATION" | "REGULAR_POLYGON"
    ) {
        report.vectors_recovered += 1;
    }
}

/// Collect the type-specific side-table material for a recognized node
/// (component master/set, instance overrides, variable collection/variable).
fn collect_typed_side_tables<'a>(
    pending: &mut Pending<'a>,
    report: &mut MapReport,
    type_name: &str,
    guid: &str,
    id: NodeId,
    change: &'a KiwiValue,
) {
    match type_name {
        "SYMBOL" => {
            if is_state_group(change) {
                pending.sets.push(PendingSet {
                    guid: guid.to_owned(),
                    root: id,
                    name: node_name(change),
                });
            } else {
                let name = node_name(change);
                // A non-state-group SYMBOL whose name carries `Axis=Value` pairs is
                // a component-set variant member; remember it for the in-place
                // placeholder-hiding pass.
                if name.contains('=') {
                    pending.variant_masters.push((id, name.clone()));
                }
                pending.components.push(PendingComponent {
                    guid: guid.to_owned(),
                    root: id,
                    name,
                    prop_defs: read_prop_defs_raw(change),
                });
            }
            report.components += if is_state_group(change) { 0 } else { 1 };
        }
        "COMPONENT_SET" => {
            pending.sets.push(PendingSet {
                guid: guid.to_owned(),
                root: id,
                name: node_name(change),
            });
        }
        "COMPONENT" => {
            let name = node_name(change);
            if name.contains('=') {
                pending.variant_masters.push((id, name.clone()));
            }
            pending.components.push(PendingComponent {
                guid: guid.to_owned(),
                root: id,
                name,
                prop_defs: read_prop_defs_raw(change),
            });
            report.components += 1;
        }
        "INSTANCE" => collect_instance_side_tables(pending, report, guid, id, change),
        "VARIABLE_SET" => {
            pending.collections.push(PendingCollection {
                guid: guid.to_owned(),
                name: node_name(change),
                modes: read_set_modes(change),
            });
        }
        "VARIABLE" => {
            if let Some(pv) = read_pending_variable(guid, change) {
                pending.variables.push(pv);
            }
        }
        _ => {}
    }
}

/// Collect an INSTANCE's pending side-table material: the symbol ref + its raw
/// override material (symbolOverrides, componentPropAssignments, derivedSymbolData,
/// own surface fills) for pass 4.
fn collect_instance_side_tables<'a>(
    pending: &mut Pending<'a>,
    report: &mut MapReport,
    _guid: &str,
    id: NodeId,
    change: &'a KiwiValue,
) {
    report.instances += 1;
    let Some(sym) = change
        .get("symbolData")
        .and_then(|sd| sd.get("symbolID"))
        .and_then(guid_key)
    else {
        return;
    };
    pending.instances.push((id, sym));
    // Stash this instance's override material for pass 4.
    let symbol_overrides = change
        .get("symbolData")
        .and_then(|sd| sd.get("symbolOverrides"))
        .and_then(KiwiValue::as_array)
        .unwrap_or(&[]);
    let prop_assignments = change
        .get("componentPropAssignments")
        .and_then(KiwiValue::as_array)
        .unwrap_or(&[]);
    // Figma's baked per-descendant render data: the `derivedSymbolData`
    // NodeChange[] (field 125). Each entry carries the RESOLVED transform/size/
    // geometry/text for one descendant of this instance's expanded subtree —
    // applied in pass 4 so dark-theme instances render exactly as Figma resolved
    // them.
    let derived_symbol_data = change
        .get("derivedSymbolData")
        .and_then(KiwiValue::as_array)
        .unwrap_or(&[]);
    // The instance's OWN (style-resolved) surface fills/strokes. The style pre-pass
    // already inlined a `styleIdForFill` ref into `fillPaints` (so a dark
    // `_Header` carries its resolved `#1D1D1D` here, not the light master's white).
    let own_fills = read_fills(change);
    let own_strokes = has_stroke_fields(change).then(|| build_stroke(change));
    // Push a pending record for EVERY symbol-bound instance, even one with no
    // explicit override material: pass 4 also applies a master's component-property
    // DEFAULTS to an instance that leaves a prop unset (a prop-less master is a
    // cheap no-op — its empty default map skips the walk), and the merge-surface
    // tally already iterates all of them.
    pending.instance_overrides.push(PendingInstanceOverrides {
        instance: id,
        symbol_overrides,
        prop_assignments,
        derived_symbol_data,
        own_fills,
        own_strokes,
    });
}

pub(crate) fn has_stroke_fields(change: &KiwiValue) -> bool {
    change.get("strokePaints").is_some()
        || change.get("strokeWeight").is_some()
        || change.get("strokeGeometry").is_some()
}

/// Collect the per-node side-table material that applies regardless of type:
/// prototype interactions, variable bindings, paint-color bindings, explicit
/// variable-mode pins, and component-prop refs/defs.
fn collect_per_node_side_tables<'a>(
    pending: &mut Pending<'a>,
    guid: &str,
    id: NodeId,
    change: &'a KiwiValue,
) {
    // Per-node prototype interactions.
    if let Some(arr) = change
        .get("prototypeInteractions")
        .and_then(KiwiValue::as_array)
    {
        if !arr.is_empty() {
            pending.reactions.push((id, arr));
        }
    }
    // Per-node variable bindings (the consumption map).
    if let Some(map) = change.get("variableConsumptionMap") {
        pending.bindings.push((id, map));
    }
    let paint_bindings = read_paint_color_bindings(change);
    if !paint_bindings.is_empty() {
        pending.paint_bindings.push((id, paint_bindings));
    }
    // Per-frame variable-mode pins (`explicitVariableModes`). Captured raw (guid
    // pairs) here; resolved to ids in pass 4.
    let explicit_modes = read_explicit_modes(change);
    if !explicit_modes.is_empty() {
        pending.explicit_modes.push((id, explicit_modes));
    }
    // Per-node component-prop refs: which prop-def guid drives which property of
    // this node. Recorded so an instance's prop assignments resolve to the master
    // descendant they set (pass 4).
    if let Some(refs) = read_component_prop_refs(change) {
        pending.node_prop_refs.insert(guid.to_owned(), refs);
    }
    // Per-node component-prop DEFS: a master / state-group declares its exposed
    // props here. Collected globally (name + parentPropDefId) so the in-place
    // placeholder pass resolves a child's `VISIBLE` prop NAME through the variant
    // member → set-level parent chain.
    collect_prop_def_infos(change, &mut pending.prop_def_infos);
    // Keyframe-motion consumers: KEYFRAME expressions in the node's consumption
    // maps (which track drives which motion channel of this node) plus any
    // timeline definitions (clip durations) the node carries.
    collect_motion_consumers(&mut pending.motion, id, change);
}

/// Collect the embedded-image asset map: every `image.hash` referenced by a
/// visible IMAGE paint (in node paints, background paints, strokes, or instance
/// override/derived data) mapped to its deterministic [`AssetId`] and the raw
/// bytes from the `.fig`'s `images/<hash>` table.
///
/// Only *referenced* images are included — a `.fig` can carry images no paint
/// uses, and there's no point handing the resolver bytes nothing will draw. A
/// referenced hash absent from `images` (a thumbnail-only ref, or a stripped
/// export) is skipped: the paint still resolved to a `Fill::Image`, and the
/// renderer falls back to its placeholder for the unbacked asset.
pub(crate) fn collect_image_assets<'a>(
    node_changes: impl IntoIterator<Item = &'a KiwiValue>,
    images: &HashMap<String, Vec<u8>>,
) -> HashMap<AssetId, Vec<u8>> {
    let mut assets = HashMap::new();
    for change in node_changes {
        collect_image_assets_from_change(change, images, &mut assets);
    }
    assets
}

fn collect_image_assets_from_change(
    change: &KiwiValue,
    images: &HashMap<String, Vec<u8>>,
    assets: &mut HashMap<AssetId, Vec<u8>>,
) {
    let fill_paints = match change.get("fillPaints").and_then(KiwiValue::as_array) {
        Some(paints) if !paints.is_empty() => Some(paints),
        _ => change.get("backgroundPaints").and_then(KiwiValue::as_array),
    };
    collect_image_assets_from_paints(fill_paints, images, assets);
    collect_image_assets_from_paints(
        change.get("strokePaints").and_then(KiwiValue::as_array),
        images,
        assets,
    );

    if let Some(overrides) = change
        .get("symbolData")
        .and_then(|sd| sd.get("symbolOverrides"))
        .and_then(KiwiValue::as_array)
    {
        for override_change in overrides {
            collect_image_assets_from_change(override_change, images, assets);
        }
    }
    if let Some(derived) = change
        .get("derivedSymbolData")
        .and_then(KiwiValue::as_array)
    {
        for derived_change in derived {
            collect_image_assets_from_change(derived_change, images, assets);
        }
    }
}

fn collect_image_assets_from_paints(
    paints: Option<&[KiwiValue]>,
    images: &HashMap<String, Vec<u8>>,
    assets: &mut HashMap<AssetId, Vec<u8>>,
) {
    let Some(arr) = paints else {
        return;
    };
    for p in arr {
        // Mirror `read_paint`'s visibility + type gate so we only key images a
        // fill actually carries.
        if read_paint(p).is_none() {
            continue;
        }
        if p.get("type").and_then(KiwiValue::as_str) != Some("IMAGE") {
            continue;
        }
        let Some(hash) = image_hash_hex(p) else {
            continue;
        };
        let asset = asset_id_for_image(&hash);
        if assets.contains_key(&asset) {
            continue; // one decode per distinct bitmap
        }
        if let Some(bytes) = images.get(&hash) {
            assets.insert(asset, bytes.clone());
        }
    }
}

/// Name of the hidden page that hosts component master subtrees.
pub(crate) const COMPONENTS_PAGE_NAME: &str = "Components";

// =============================================================================
// Pending side-table records
// =============================================================================

pub(crate) struct PendingComponent<'a> {
    pub(crate) guid: String,
    pub(crate) root: NodeId,
    pub(crate) name: String,
    /// The component's `componentPropDefs` array (Figma `ComponentPropDef[]`):
    /// each entry exposes one instance-settable property — `id` (a GUID), `name`,
    /// `type` (TEXT / BOOL / INSTANCE_SWAP / VARIANT), and a `varValue` default.
    /// Parsed into [`ComponentDef::props`] so instances can apply DEFAULTS for
    /// props they don't explicitly assign, and so variant selection has a schema.
    /// Empty for a hand-built component or a master with no exposed props.
    pub(crate) prop_defs: &'a [KiwiValue],
}

pub(crate) struct PendingSet {
    /// The state-group / COMPONENT_SET SYMBOL's own guid, so an INSTANCE whose
    /// `symbolData.symbolID` names the *set* (rather than a member variant)
    /// resolves to the set's [`ComponentId`] — [`expand_instance`] then picks the
    /// default (or variant-matched) member.
    ///
    /// [`expand_instance`]: fanta_doc::resolve::expand_instance
    pub(crate) guid: String,
    pub(crate) root: NodeId,
    pub(crate) name: String,
}

pub(crate) struct PendingCollection {
    pub(crate) guid: String,
    pub(crate) name: String,
    /// (mode guid string, mode name) in declaration order.
    pub(crate) modes: Vec<(String, String)>,
}

pub(crate) struct PendingVariable {
    pub(crate) guid: String,
    pub(crate) name: String,
    /// The owning collection's guid (`variableSetID`), if present.
    pub(crate) set_guid: Option<String>,
    pub(crate) ty: VariableType,
    /// (mode guid string, value) per-mode values.
    pub(crate) values: Vec<(String, VarValue)>,
}

/// The raw override material for one instance, captured in pass 1 and resolved
/// to typed [`Override`]s in pass 4 (once the master subtree and guid→NodeId map
/// exist). Both arrays are the instance's two override mechanisms:
///
/// - `symbol_overrides`: a `NodeChange[]` (Figma's `symbolData.symbolOverrides`).
///   Each carries a `guidPath` (a path of master-descendant guids) plus the
///   overridden node fields inline (`textData`, `fillPaints`, `visible`).
/// - `prop_assignments`: a `ComponentPropAssignment[]` (the instance's exposed
///   component-property values). Each carries a `defID` (prop-def guid) and a
///   `value.textValue` — resolved against the master descendants that bind that
///   prop-def via their `componentPropRefs`.
pub(crate) struct PendingInstanceOverrides<'a> {
    pub(crate) instance: NodeId,
    pub(crate) symbol_overrides: &'a [KiwiValue],
    pub(crate) prop_assignments: &'a [KiwiValue],
    /// The instance's `derivedSymbolData` (Kiwi field 125): a `NodeChange[]` of
    /// Figma's baked, fully-resolved per-descendant render data — each entry's
    /// `guidPath` addresses one master descendant and carries its resolved
    /// `size`/`transform`/`fillGeometry`/`strokeGeometry`/`strokeWeight`/
    /// `derivedTextData` for *this* placement. Resolved to typed
    /// [`fanta_doc::node::DerivedOverride`]s in pass 4.
    pub(crate) derived_symbol_data: &'a [KiwiValue],
    /// The instance node's OWN surface fills, read from its (style-resolved)
    /// `fillPaints`. Figma resolves a per-placement surface color onto the
    /// instance itself — for a dark-theme card `_Header` this is `#1D1D1D`,
    /// pinned via the instance's `styleIdForFill` even though its light master
    /// root is white. op2 surfaces this through `mergeSymbolProps`, where the
    /// instance's own `fillPaints` take priority over the symbol's; we route it
    /// as a root-targeted [`OverrideValue::Fills`] so [`expand_instance`] paints
    /// the expanded master root with the instance's resolved surface instead of
    /// the master's. Empty ⇒ inherit the master surface (unchanged behavior).
    pub(crate) own_fills: Vec<Fill>,
    /// The instance node's OWN stroke stack, when the instance explicitly carries
    /// stroke fields. `Some(empty)` means Figma explicitly resolved "no border",
    /// which must clear a stroked master root.
    pub(crate) own_strokes: Option<Vec<Stroke>>,
}

// =============================================================================
// Sibling-order resolution (parentIndex.position)
// =============================================================================

/// Reorder the stream-order `order` so that, within each Figma parent, children
/// are sorted ascending by their `parentIndex.position` fractional-index string
/// — Figma's authoritative z-order — while the parent groups themselves keep
/// stream order. This is the order pass 2 attaches siblings in, so the minted
/// `IndexKey`s end up reflecting Figma stacking, not the unrelated NodeChange
/// stream order.
///
/// ## Ordering rules
/// - Each node's GROUP key is the stream index at which its parent guid first
///   appears in `order` (root-parented nodes share the synthetic root group).
///   Groups stay in first-appearance (stream) order — so two distinct Figma
///   parents that resolve to the same ancestor still attach back-to-back in
///   document order (a skipped wrapper's children inline in place).
/// - WITHIN a group, nodes sort ascending by `position` (a STRING compared
///   lexicographically — fractional indices are NOT numeric).
/// - Ties and missing positions fall back to the node's own stream index, so the
///   sort is stable and order-preserving where Figma gives us nothing better.
///
/// Position strings are only comparable among true siblings (same Figma parent),
/// which is exactly the within-group scope here; we never compare positions
/// across different parents.
pub(crate) fn attach_order(
    order: &[String],
    guid_to_parent: &HashMap<String, Option<String>>,
    guid_to_position: &HashMap<String, Option<String>>,
) -> Vec<String> {
    // Stream index of each guid + the first-appearance stream index of each
    // parent guid (the group anchor). A node with no recorded parent shares one
    // synthetic root group whose anchor is its own stream index's minimum —
    // captured by treating `None` parents as the same group key (usize::MAX),
    // which keeps every root node's group stable relative to others (roots tie
    // on the group key and then sort by their own stream index).
    let mut stream_index: HashMap<&str, usize> = HashMap::with_capacity(order.len());
    for (i, g) in order.iter().enumerate() {
        stream_index.entry(g.as_str()).or_insert(i);
    }
    let parent_anchor = |guid: &str| -> usize {
        match guid_to_parent.get(guid).and_then(Option::as_deref) {
            Some(parent) => *stream_index.get(parent).unwrap_or(&usize::MAX),
            None => usize::MAX,
        }
    };

    let mut indexed: Vec<(usize, &str, usize, &str)> = order
        .iter()
        .enumerate()
        .map(|(i, g)| {
            let group = parent_anchor(g);
            let pos = guid_to_position
                .get(g)
                .and_then(Option::as_deref)
                .unwrap_or("");
            (group, pos, i, g.as_str())
        })
        .collect();
    // Stable sort: primary = parent group (stream first-appearance), secondary =
    // position string (lexicographic), tertiary = own stream index (tiebreak).
    indexed.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then_with(|| a.1.cmp(b.1))
            .then_with(|| a.2.cmp(&b.2))
    });
    indexed
        .into_iter()
        .map(|(_, _, _, g)| g.to_owned())
        .collect()
}

// =============================================================================
// Parent resolution
// =============================================================================

/// The outcome of resolving a node's effective parent.
pub(crate) enum ParentResolution {
    /// Attach under this recognized container node.
    Node(NodeId),
    /// Attach at the scene root.
    Root,
    /// The nearest recognized ancestor is an instance — this node is virtual
    /// content and should be dropped.
    InsideInstance,
}

/// Climb the Figma parent chain from `guid` to the nearest ancestor that has a
/// real Fantaisa `NodeId`. If that ancestor is an instance, the node lives in a
/// virtual subtree ([`ParentResolution::InsideInstance`]); if no recognized
/// ancestor exists, it attaches at root.
pub(crate) fn resolve_parent(
    guid: &str,
    guid_to_parent: &HashMap<String, Option<String>>,
    guid_to_node: &HashMap<String, Option<NodeId>>,
    instance_ids: &std::collections::HashSet<NodeId>,
) -> ParentResolution {
    let mut cursor = guid_to_parent.get(guid).cloned().flatten();
    let mut budget = guid_to_parent.len() + 1;
    while let Some(parent_guid) = cursor {
        if budget == 0 {
            break;
        }
        budget -= 1;
        match guid_to_node.get(&parent_guid) {
            Some(Some(id)) => {
                if instance_ids.contains(id) {
                    return ParentResolution::InsideInstance;
                }
                return ParentResolution::Node(*id);
            }
            Some(None) => {
                cursor = guid_to_parent.get(&parent_guid).cloned().flatten();
            }
            None => return ParentResolution::Root,
        }
    }
    ParentResolution::Root
}
