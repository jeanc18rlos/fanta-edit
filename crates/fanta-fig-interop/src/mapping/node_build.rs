//! Per-node construction: turn a recognized Figma `NodeChange` into a
//! Fantaisa [`CanvasNode`], and tally element-fidelity counters off the result.

use super::{
    AxisSizing, BlendMode, CanvasNode, ComponentId, GroupNode, IndexKey, InstanceNode, KiwiValue,
    LayoutMode, MapReport, MaskType, NodeData, NodeFlags, PathData, StrokeCap, StrokeJoin,
    TextAutoResize, VectorNode, arc_ellipse_path, blend_mode, build_stroke, build_text,
    build_vector, corner_radii, first_fill, group_with_optional_clip, guid_key,
    has_independent_corners, make_shape, read_auto_layout, read_blurs, read_effects, read_fills,
    read_layout_child, read_mask, read_paint, read_size, read_transform, text_case_of,
};

/// The outcome of trying to turn a Figma node change into a Fantaisa node.
pub(crate) enum NodeBuild {
    /// A real node was built. `is_page` is set for a Figma `CANVAS`;
    /// `geometry_decoded` is set for a VECTOR-family node whose real path
    /// geometry was decoded from the file's command blobs (vs the bbox fallback).
    Node {
        node: Box<CanvasNode>,
        is_page: bool,
        geometry_decoded: bool,
    },
    /// A structural container (DOCUMENT) that owns children but is not a node.
    Structural,
    /// A node type this importer does not support.
    Unsupported,
}

/// Build a Fantaisa node from a Figma `NodeChange` of the given type. `blobs` is
/// the document's command-blob table, indexed by a vector path's `commandsBlob`.
pub(crate) fn build_node(type_name: &str, change: &KiwiValue, blobs: &[Vec<u8>]) -> NodeBuild {
    let size = read_size(change); // (w, h), defaulting to a small box
    let transform = read_transform(change);
    // All visible fill paints (solid + gradient + image), bottom-first. The
    // first paint feeds the single-fill contexts (frame background, text color).
    let fills = read_fills(change);
    let first = first_fill(&fills);

    let is_page = type_name == "CANVAS";

    let data = match type_name {
        "GROUP" => NodeData::Group(GroupNode::default()),
        "CANVAS" => NodeData::Group(GroupNode {
            clip_size: None,
            background: first,
            background_fills: fills.iter().skip(1).cloned().collect(),
            explicit_modes: Default::default(),
            // Set after construction in `build_node` (a page is never auto-layout).
            auto_layout: None,
            // A page carries no border or rounding.
            strokes: Default::default(),
            corner_radius: None,
            corner_radii: None,
            corner_smoothing: 0.0,
        }),
        // A FRAME has a real box, paints a background fill, and carries its own
        // border (stroke) + corner rounding just like a shape — so a frame's
        // visible border and rounded card/panel corners survive import. Figma's
        // "Clip content" toggle (`frameMaskDisabled`, see `clips_content`) decides
        // whether descendants are cropped to that box, not whether the box exists.
        "FRAME" => {
            let (uniform, per_corner) = corner_radii(change);
            NodeData::Group(GroupNode {
                clip_size: Some([size.0, size.1]),
                background: first,
                background_fills: fills.iter().skip(1).cloned().collect(),
                explicit_modes: Default::default(),
                // Filled in by `build_node` from the change's `stack*` fields.
                auto_layout: None,
                strokes: build_stroke(change).into_iter().collect(),
                corner_radius: uniform,
                corner_radii: per_corner,
                corner_smoothing: 0.0,
            })
        }
        // A SECTION is Figma's organizational container: it paints a background
        // and carries a box, but unlike a FRAME it never crops its content
        // (sections always show overflow). We keep its `clip_size` set so the box
        // bounds + background still resolve from the declared size — clipping to a
        // box the content already lives inside is a visual no-op — and so we don't
        // honor `frameMaskDisabled` here (a section is unclipped regardless, and a
        // dropped box would lose its declared bounds + background surface).
        "SECTION" => {
            let (uniform, per_corner) = corner_radii(change);
            NodeData::Group(GroupNode {
                clip_size: Some([size.0, size.1]),
                background: first,
                background_fills: fills.iter().skip(1).cloned().collect(),
                explicit_modes: Default::default(),
                auto_layout: None,
                strokes: build_stroke(change).into_iter().collect(),
                corner_radius: uniform,
                corner_radii: per_corner,
                corner_smoothing: 0.0,
            })
        }
        // A SYMBOL (main component) or COMPONENT/COMPONENT_SET master is an
        // ordinary group subtree; the [`ComponentDef`]/[`ComponentSet`] metadata
        // is recorded separately and points back at this root.
        "SYMBOL" | "COMPONENT" | "COMPONENT_SET" => {
            NodeData::Group(group_with_optional_clip(change, size, first))
        }
        // An INSTANCE maps to a real instance node when it names a component; if
        // it doesn't, fall back to a clipping group so its (now non-virtual)
        // subtree is at least preserved. The component ref is wired up after the
        // guid -> ComponentId map exists; here we stash a placeholder id that the
        // side-table pass replaces.
        "INSTANCE" => {
            if change
                .get("symbolData")
                .and_then(|sd| sd.get("symbolID"))
                .is_some()
            {
                NodeData::Instance(InstanceNode {
                    component: ComponentId::from_u128(0), // placeholder, fixed in pass 4
                    overrides: Vec::new(),
                    prop_values: Default::default(),
                    derived: Vec::new(), // baked render data resolved in pass 4
                    local_size: [size.0, size.1],
                })
            } else {
                NodeData::Group(group_with_optional_clip(change, size, first))
            }
        }
        // VARIABLE_SET / VARIABLE carry no geometry — they exist only to host
        // the design-system side-tables. We still create a tiny structural group
        // so the guid resolves to a NodeId (parenting stays intact) and so the
        // node count reflects them; the app can hide these by figma_type.
        "VARIABLE_SET" | "VARIABLE" => NodeData::Group(GroupNode::default()),
        // A RECTANGLE can itself carry rounding (uniform `cornerRadius` or mixed
        // per-corner fields), so read radii for both rect variants. Mirrors op2,
        // which calls `mapCornerRadius` on every rectangle.
        "RECTANGLE" | "ROUNDED_RECTANGLE" => {
            let (uniform, per_corner) = corner_radii(change);
            NodeData::Vector(make_shape(size, &fills, change, uniform, per_corner))
        }
        "ELLIPSE" => {
            // An ellipse may carry `arcData` (a partial sweep and/or an inner
            // radius) making it a pie / donut / gauge ring. When present and not a
            // full 360° solid ellipse, tessellate the real arc geometry into the
            // path; otherwise keep the plain full ellipse. Render-only — no new
            // model primitive.
            let path = arc_ellipse_path(change, size).unwrap_or_else(|| {
                PathData::ellipse(size.0 / 2.0, size.1 / 2.0, size.0 / 2.0, size.1 / 2.0)
            });
            NodeData::Vector(VectorNode {
                path,
                fills: fills.iter().cloned().collect(),
                strokes: build_stroke(change).into_iter().collect(),
                corner_radius: None,
                corner_radii: None,
            })
        }
        // VECTOR-family geometry. STEP 2: decode the real path from the node's
        // `fillGeometry` command blobs; on failure keep the STEP-1 bbox fallback.
        "VECTOR" | "STAR" | "LINE" | "BOOLEAN_OPERATION" | "REGULAR_POLYGON" => {
            return build_vector(type_name, change, size, &fills, transform, blobs);
        }
        "TEXT" => build_text(change, size, first),
        "DOCUMENT" => return NodeBuild::Structural,
        _ => return NodeBuild::Unsupported,
    };

    let mut node = CanvasNode::new(data);
    node.transform = transform;
    if let Some(name) = change.get("name").and_then(KiwiValue::as_str) {
        if !name.is_empty() {
            node.name = name.to_owned();
        }
    }
    apply_node_visual_props(&mut node, change);
    // Auto-layout: attach OpenPencil-compatible stack data to frame-like groups.
    // Besides explicit `stackMode`, OpenPencil preserves stack padding/spacing
    // and lets its renderer infer a horizontal flow from those props; importing
    // the same signal keeps `.fig` layouts aligned with that reference. Per-child
    // grow/positioning/alignSelf is read onto the wrapper regardless of variant,
    // since any node can be an auto-layout child. INSTANCE master subtrees go
    // through this same builder, so expanded-subtree nodes capture their stack
    // data too (Stage 2 lays them out).
    if let NodeData::Group(g) = &mut node.data {
        g.auto_layout = read_auto_layout(change);
    }
    node.layout_child = read_layout_child(change);
    // Masks: a node flagged `mask`/`isMask` masks its following siblings within
    // the same parent. Read the flag + type (ALPHA default / LUMINANCE; VECTOR &
    // OUTLINE collapse to ALPHA of the shape). Reading is cheap on every node; a
    // non-mask change leaves `is_mask == false`. Applied by `fanta-render`'s
    // children-painting path (alpha/luminance DstIn compositing).
    read_mask(change, &mut node);
    // Stamp the node's stable correlation keys onto `meta`: the Figma node type
    // and — when present — the Figma node id in public `sessionID:localID` form.
    // The id is the same `change.guid` the orchestrator keys nodes by (extracted
    // with `guid_key`), so downstream consumers (the fidelity farm) can join a
    // rendered frame back to its source Figma node. Absent guid => no `figma_id`.
    let mut meta = serde_json::json!({ "figma_type": type_name });
    if let Some(figma_id) = change.get("guid").and_then(guid_key) {
        meta["figma_id"] = serde_json::Value::String(figma_id);
    }
    if type_name == "CANVAS" && node.name == "Internal Only Canvas" {
        meta["hidden"] = serde_json::Value::Bool(true);
        meta["hidden_page"] = serde_json::Value::Bool(true);
    }
    if type_name == "SECTION"
        || (matches!(
            type_name,
            "FRAME" | "SYMBOL" | "COMPONENT" | "COMPONENT_SET" | "INSTANCE"
        ) && !clips_content(change))
    {
        // Figma SECTION nodes and `frameMaskDisabled=true` frame-like nodes have
        // a real box/background but do not crop their children. Keep `clip_size`
        // as the box/bounds carrier and mark only the render-time clip disabled.
        meta["clip_content"] = serde_json::Value::Bool(false);
    }
    node.meta = meta;
    node.index = IndexKey::FIRST;
    NodeBuild::Node {
        node: Box::new(node),
        is_page,
        geometry_decoded: false,
    }
}

/// Whether a FRAME/SECTION-style container clips its content to its box (Figma's
/// "Clip content" toggle). Figma stores the toggle as the boolean
/// `frameMaskDisabled` on the NodeChange: `true` means clip is OFF, while an
/// absent or `false` value means clip is ON — the default. The importer must
/// honor this, otherwise a frame that intentionally lets badges / bleed art /
/// overflowing children spill past its edge (the common case — 68% of frames in
/// the Spectrum fixture) is wrongly cropped. Mirrors the kiwi-value bool
/// accessor style used elsewhere in mapping (`auto_layout`'s `mask`/`isMask`,
/// `stroke`'s `borderStrokeWeightsIndependent`).
pub(crate) fn clips_content(change: &KiwiValue) -> bool {
    !matches!(change.get("frameMaskDisabled"), Some(KiwiValue::Bool(true)))
}

/// Carry the node-level visual properties Figma stores outside the fill/stroke
/// paints onto the [`CanvasNode`]: opacity, blend mode, effects (drop/inner
/// shadow), and blurs (layer/background). These apply to the composited node,
/// not to individual paints, which is exactly the doc model's split. A
/// `PASS_THROUGH` blend (groups) maps to `Normal`. Absent fields leave the node
/// defaults untouched.
pub(crate) fn apply_node_visual_props(node: &mut CanvasNode, change: &KiwiValue) {
    let opacity = change.get("opacity").and_then(KiwiValue::as_f64);
    if let Some(o) = opacity {
        let o = o.clamp(0.0, 1.0) as f32;
        if o < 0.999 {
            node.opacity = o;
        }
    }
    if let Some(bm) = change
        .get("blendMode")
        .and_then(KiwiValue::as_str)
        .and_then(blend_mode)
    {
        node.blend_mode = bm;
    }
    node.effects = read_effects(change).into_iter().collect();
    node.blurs = read_blurs(change).into_iter().collect();

    // Visibility: a `visible:false` node (or an opacity<=0 one) is hidden. The
    // renderer skips `HIDDEN` nodes, so this kills hidden light-master layers
    // that otherwise leak white behind a dark-theme layer stacked on top.
    let explicitly_hidden = matches!(change.get("visible"), Some(KiwiValue::Bool(false)));
    let fully_transparent = opacity.map(|o| o <= 0.0).unwrap_or(false);
    if explicitly_hidden || fully_transparent {
        node.flags |= NodeFlags::HIDDEN;
    }
}

/// Update the [`MapReport`] element-fidelity counters from a freshly-built node.
/// Reads the source `change` for the gradient/image/per-corner signals and the
/// built `node` (if present) for what actually landed (strokes, opacity, blend,
/// effects). Counting from the built node keeps the report honest — it reflects
/// applied properties, not just present source fields.
pub(crate) fn tally_fidelity(
    report: &mut MapReport,
    change: &KiwiValue,
    node: Option<&CanvasNode>,
) {
    tally_source_fills(report, change);
    let Some(node) = node else { return };
    tally_node_visuals(report, node);
    tally_vector_strokes(report, node);
    tally_arc_ellipse(report, change);
    tally_text_fidelity(report, change, node);
    tally_auto_layout(report, node);
    tally_masks(report, node);
}

/// Gradients + images + per-corner radii: counted from the source `change`'s
/// recognized fill paints. (An IMAGE paint maps to a real `Fill::Image` keyed by
/// a deterministic `AssetId`; we still count it by source type so the report
/// reflects every image paint recognized, including any whose bytes the resolver
/// can't back.)
fn tally_source_fills(report: &mut MapReport, change: &KiwiValue) {
    if let Some(paints) = change.get("fillPaints").and_then(KiwiValue::as_array) {
        for p in paints {
            if read_paint(p).is_none() {
                continue; // invisible / unrecognized
            }
            match p.get("type").and_then(KiwiValue::as_str) {
                Some("GRADIENT_ANGULAR") => {
                    report.gradients_imported += 1;
                    report.conic_gradients_imported += 1;
                }
                Some("GRADIENT_DIAMOND") => {
                    report.gradients_imported += 1;
                    report.diamond_gradients_imported += 1;
                }
                Some(t) if t.starts_with("GRADIENT") => report.gradients_imported += 1,
                Some("IMAGE") => report.images_imported += 1,
                _ => {}
            }
        }
    }
    if has_independent_corners(change) {
        report.per_corner_radius += 1;
    }
}

/// Node-level visuals (opacity, blend, effects, blurs) and stroke/frame counters,
/// read off the BUILT node so the report reflects what actually landed.
fn tally_node_visuals(report: &mut MapReport, node: &CanvasNode) {
    if node.opacity < 0.999 {
        report.node_opacity_imported += 1;
    }
    if node.blend_mode != BlendMode::Normal {
        report.blend_modes_imported += 1;
    }
    report.effects_imported += node.effects.len();
    report.blurs_imported += node.blurs.len();
    // A stroke can live on a shape (VECTOR) or on a frame (GROUP) — a frame's
    // border. Count both so `strokes_imported` reflects every imported stroke.
    let stroke_count = match &node.data {
        NodeData::Vector(v) => v.strokes.len(),
        NodeData::Group(g) => g.strokes.len(),
        _ => 0,
    };
    if stroke_count > 0 {
        report.strokes_imported += 1;
    }

    // Frame-specific counters: how many FRAME/SECTION-style groups gained a
    // border and/or any corner rounding from this fixture (a frame is a GROUP
    // with a `clip_size`; a page/plain group has none). Surfaced for the report.
    if let NodeData::Group(g) = &node.data {
        if g.clip_size.is_some() {
            if !g.strokes.is_empty() {
                report.frames_with_stroke += 1;
            }
            if g.corner_radius.is_some() || g.corner_radii.is_some() {
                report.frames_rounded += 1;
            }
        }
    }
}

/// Vector-stroke fidelity: Gap A (stroke-only outline), Gap C (cap/join/dash),
/// and F1 (per-side border weights). Counted off the BUILT node.
fn tally_vector_strokes(report: &mut MapReport, node: &CanvasNode) {
    // Gap A — stroke-only icon vectors (tagged by `build_vector` in `meta`).
    if node
        .meta
        .get("stroke_only_outline")
        .and_then(serde_json::Value::as_bool)
        == Some(true)
    {
        report.stroke_only_vectors += 1;
    }

    // Gap C — strokes carrying a non-default cap/join or a dash pattern (data the
    // old `build_stroke` dropped). Counted from the built strokes so the report
    // reflects what actually landed.
    if let NodeData::Vector(v) = &node.data {
        let has_decoration = v.strokes.iter().any(|s| {
            s.cap != StrokeCap::default() || s.join != StrokeJoin::default() || !s.dash.is_empty()
        });
        if has_decoration {
            report.strokes_with_cap_join_dash += 1;
        }
    }

    // F1 — nodes whose stroke carries per-side border weights (Figma
    // `borderStrokeWeightsIndependent`). Counted across both shape and frame
    // strokes so a per-side frame border counts too.
    let has_per_side = match &node.data {
        NodeData::Vector(v) => v.strokes.iter().any(|s| s.per_side.is_some()),
        NodeData::Group(g) => g.strokes.iter().any(|s| s.per_side.is_some()),
        _ => false,
    };
    if has_per_side {
        report.per_side_borders += 1;
    }
}

/// F3 — ELLIPSE nodes imported as a real arc/pie/donut/ring (non-default
/// `arcData`). Counted off the change so it reflects exactly what
/// `arc_ellipse_path` tessellated (it returns `None` for a plain full disc, and
/// only ELLIPSE nodes carry `arcData`).
fn tally_arc_ellipse(report: &mut MapReport, change: &KiwiValue) {
    if arc_ellipse_path(change, read_size(change)).is_some() {
        report.arc_ellipses += 1;
    }
}

/// TEXT fidelity: Gap B (non-default weight), Gap D (effective `textCase`
/// transform), and the auto-resize bucket.
fn tally_text_fidelity(report: &mut MapReport, change: &KiwiValue, node: &CanvasNode) {
    let NodeData::Text(t) = &node.data else {
        return;
    };
    // Gap B — TEXT nodes whose parsed weight is neither 400 nor 700 (a weight the
    // old mapper could not produce).
    if t.style.weight != 400 && t.style.weight != 700 {
        report.non_default_weights += 1;
    }

    // Gap D — TEXT nodes whose rendered string was actually changed by a
    // non-ORIGINAL `textCase`. We re-derive the source string + transform here and
    // compare, so we count only labels where the case transform had a visible
    // effect (an already-uppercase label under UPPER doesn't inflate the count).
    let raw = change
        .get("textData")
        .and_then(|td| td.get("characters"))
        .and_then(KiwiValue::as_str);
    if let Some(raw) = raw {
        let case = text_case_of(change);
        if matches!(case, Some("UPPER" | "LOWER" | "TITLE")) && t.content != raw {
            report.text_case_transformed += 1;
        }
    }

    match t.auto_resize {
        TextAutoResize::WidthAndHeight => report.text_auto_width += 1,
        TextAutoResize::Height => report.text_auto_height += 1,
        TextAutoResize::None => report.text_fixed_box += 1,
    }
}

/// Stage 1 — auto-layout import counters (the data Stage 2's layout pass
/// consumes). Counted off the BUILT node so the report reflects what landed.
fn tally_auto_layout(report: &mut MapReport, node: &CanvasNode) {
    if let NodeData::Group(g) = &node.data {
        if let Some(al) = &g.auto_layout {
            match al.mode {
                LayoutMode::Horizontal => report.auto_layout_horizontal += 1,
                LayoutMode::Vertical => report.auto_layout_vertical += 1,
            }
            if al.primary_sizing == AxisSizing::Hug {
                report.auto_layout_primary_hug += 1;
            }
            if al.counter_sizing == AxisSizing::Hug {
                report.auto_layout_counter_hug += 1;
            }
            // F2 — auto-layout frames that WRAP (`stackWrap`): children flow onto
            // multiple rows/columns, which the Stage-2 solver now handles.
            if al.wrap {
                report.auto_layout_wrap += 1;
            }
        }
    }
    if let Some(lc) = &node.layout_child {
        if lc.grow > 0.0 {
            report.layout_children_grow += 1;
        }
        if lc.absolute {
            report.layout_children_absolute += 1;
        }
    }
}

/// Masks — counted off the BUILT node so the report reflects what actually
/// landed (the flag + type set by `read_mask`). A LUMINANCE mask is also counted
/// in `masks_imported` (it is a mask), then once more in `masks_luminance`.
fn tally_masks(report: &mut MapReport, node: &CanvasNode) {
    if node.is_mask {
        report.masks_imported += 1;
        if node.mask_type == MaskType::Luminance {
            report.masks_luminance += 1;
        }
    }
}
