//! Per-node construction: turn a recognized Figma `NodeChange` into a
//! Fantaisa [`CanvasNode`], and tally element-fidelity counters off the result.

use super::{
    AxisSizing, BlendMode, CanvasNode, ComponentId, Fill, GroupNode, IndexKey, InstanceNode,
    KiwiValue, LayoutMode, MapReport, MaskType, NodeData, NodeFlags, PathData, ScrollDirection,
    StrokeCap, StrokeJoin, TextAutoResize, VectorNode, arc_ellipse_path, blend_mode, build_stroke,
    build_text, build_vector, corner_radii, corner_smoothing, first_fill, group_with_optional_clip,
    guid_key, has_independent_corners, make_shape, read_arc_shape, read_auto_layout, read_blurs,
    read_color_with_opacity, read_effects, read_fills, read_layout_child, read_mask, read_paint,
    read_scroll_behavior, read_scroll_direction, read_scroll_offset, read_size, read_transform,
    text_case_of, viewport,
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
        "GROUP" => NodeData::Group(GroupNode {
            local_size: (change.get("size").is_some() && size.0 > 0.0 && size.1 > 0.0)
                .then_some([size.0, size.1]),
            ..GroupNode::default()
        }),
        // A page's canvas color lives in CANVAS.backgroundColor (+ opacity +
        // enabled), NOT in the paint arrays — read it first and fall back to
        // any background paints for older exports.
        "CANVAS" => NodeData::Group(GroupNode {
            local_size: None,
            scrollable: false,
            scroll_direction: None,
            scroll_offset: None,
            clip_size: None,
            background: canvas_background(change).or(first),
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
                local_size: None,
                clip_size: Some([size.0, size.1]),
                background: first,
                background_fills: fills.iter().skip(1).cloned().collect(),
                explicit_modes: Default::default(),
                // Filled in by `build_node` from the change's `stack*` fields.
                auto_layout: None,
                strokes: build_stroke(change).into_iter().collect(),
                corner_radius: uniform,
                corner_radii: per_corner,
                corner_smoothing: corner_smoothing(change),
                // Legacy readers key generic ScrollTo containment off this
                // bool; the authored axes below are what the present runtime
                // honors (`effective_scroll_direction` — a frame with no
                // authored overflow no longer free-scrolls, matching Figma).
                scrollable: true,
                scroll_direction: Some(
                    read_scroll_direction(change).unwrap_or(ScrollDirection::None),
                ),
                scroll_offset: read_scroll_offset(change),
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
                local_size: None,
                clip_size: Some([size.0, size.1]),
                background: first,
                background_fills: fills.iter().skip(1).cloned().collect(),
                explicit_modes: Default::default(),
                auto_layout: None,
                strokes: build_stroke(change).into_iter().collect(),
                corner_radius: uniform,
                corner_radii: per_corner,
                corner_smoothing: corner_smoothing(change),
                scrollable: false,
                // Sections never clip, so they never scroll.
                scroll_direction: None,
                scroll_offset: None,
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
            // A non-default `arcData` is kept as a scrubable ParametricShape::Arc
            // (so the pie/donut sweep can be edited) alongside the tessellated
            // path the renderer draws. A plain full disc stays parametric-free.
            let parametric = read_arc_shape(change);
            let path = arc_ellipse_path(change, size).unwrap_or_else(|| {
                PathData::ellipse(size.0 / 2.0, size.1 / 2.0, size.0 / 2.0, size.1 / 2.0)
            });
            NodeData::Vector(VectorNode {
                path,
                fills: fills.iter().cloned().collect(),
                strokes: build_stroke(change).into_iter().collect(),
                corner_radius: None,
                corner_radii: None,
                corner_smoothing: 0.0,
                local_size: viewport(size),
                parametric,
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

    // Locked flag for selection/editing protection.
    if let Some(KiwiValue::Bool(true)) = change.get("locked") {
        node.flags |= NodeFlags::LOCKED;
    }

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

    // Constraints (responsive resize for non-auto-layout).
    // Figma: "constraints" { "horizontal": "MIN"|"MAX"|"CENTER"|"STRETCH"|"SCALE", "vertical": ... }
    if let Some(cons) = change.get("constraints") {
        let h = match cons.get("horizontal").and_then(KiwiValue::as_str) {
            Some("MIN") | Some("LEFT") => fanta_doc::ConstraintH::Left,
            Some("MAX") | Some("RIGHT") => fanta_doc::ConstraintH::Right,
            Some("CENTER") => fanta_doc::ConstraintH::Center,
            Some("STRETCH") | Some("LEFTRIGHT") => fanta_doc::ConstraintH::LeftRight,
            Some("SCALE") => fanta_doc::ConstraintH::Scale,
            _ => fanta_doc::ConstraintH::Left,
        };
        let v = match cons.get("vertical").and_then(KiwiValue::as_str) {
            Some("MIN") | Some("TOP") => fanta_doc::ConstraintV::Top,
            Some("MAX") | Some("BOTTOM") => fanta_doc::ConstraintV::Bottom,
            Some("CENTER") => fanta_doc::ConstraintV::Center,
            Some("STRETCH") | Some("TOPBOTTOM") => fanta_doc::ConstraintV::TopBottom,
            Some("SCALE") => fanta_doc::ConstraintV::Scale,
            _ => fanta_doc::ConstraintV::Top,
        };
        node.constraints = Some(fanta_doc::Constraints {
            horizontal: h,
            vertical: v,
        });
        // count in caller? or here, but report is not in scope; increment in build_node caller or add to NodeBuild.
        // For now, the setting is the import.
    }

    // Masks: a node flagged `mask`/`isMask` masks its following siblings within
    // the same parent. Read the flag + type (ALPHA default / LUMINANCE; VECTOR &
    // OUTLINE collapse to ALPHA of the shape). Reading is cheap on every node; a
    // non-mask change leaves `is_mask == false`. Applied by `fanta-render`'s
    // children-painting path (alpha/luminance DstIn compositing).
    read_mask(change, &mut node);
    // Per-child scroll behavior (fixed headers / sticky rows). Cheap on every
    // node; the overwhelming default (`Scrolls`) leaves the field skipped.
    node.scroll_behavior = read_scroll_behavior(change);
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

/// A CANVAS page's editor canvas color: `backgroundColor` × `backgroundOpacity`
/// (Figma writes these on every modern page), honoring `backgroundEnabled`
/// (absent ⇒ enabled). `None` when the fields are missing — the caller then
/// falls back to the paint arrays (older exports) or no background.
pub(crate) fn canvas_background(change: &KiwiValue) -> Option<Fill> {
    if matches!(
        change.get("backgroundEnabled"),
        Some(KiwiValue::Bool(false))
    ) {
        return None;
    }
    let color = change.get("backgroundColor")?;
    let opacity = change
        .get("backgroundOpacity")
        .and_then(KiwiValue::as_f64)
        .unwrap_or(1.0);
    read_color_with_opacity(color, opacity).map(Fill::solid)
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
            node.opacity = o.into();
        }
    }
    if let Some(bm) = change
        .get("blendMode")
        .and_then(KiwiValue::as_str)
        .and_then(blend_mode)
    {
        node.blend_mode = bm;
    }
    // A container whose blend is EXPLICITLY `NORMAL` (vs the absent default,
    // `PASS_THROUGH`) isolates its subtree: descendants' blend modes composite
    // against the flattened group only, never leaking to the backdrop behind
    // it. Both members map to `BlendMode::Normal` for painting; the isolation
    // itself is the `ISOLATED_BLEND` flag. Leaf nodes paint atomically, so the
    // distinction only matters for nodes that own (real or virtual) children.
    if matches!(node.data, NodeData::Group(_) | NodeData::Instance(_))
        && change.get("blendMode").and_then(KiwiValue::as_str) == Some("NORMAL")
    {
        node.flags |= NodeFlags::ISOLATED_BLEND;
    }
    node.effects = read_effects(change).into_iter().collect();
    node.blurs = read_blurs(change).into_iter().collect();
    apply_glass_tint(change, node);

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
    tally_scroll(report, node);
    tally_effect_fidelity(report, change);
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
    if node.opacity.get() < 0.999 {
        report.node_opacity_imported += 1;
    }
    if node.blend_mode != BlendMode::Normal {
        report.blend_modes_imported += 1;
    }
    report.effects_imported += node.effects.len();
    report.blurs_imported += node.blurs.len();
    // A stroke can live on a shape (VECTOR) or on a frame (GROUP) — a frame's
    // border. Count both so `strokes_imported` reflects every imported stroke.
    if node.data.strokes().is_some_and(|s| !s.is_empty()) {
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
    let has_per_side = node
        .data
        .strokes()
        .is_some_and(|strokes| strokes.iter().any(|s| s.per_side.is_some()));
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

/// The second half of the GLASS approximation (see `read_blurs`): Figma
/// composites the effect's `color` as a translucent film over the blurred
/// backdrop, UNDER the node's own fills. Without it a dark glass panel reads
/// far too bright (verified against Figma's render of a 25%-black glass —
/// the tint is most of the remaining delta after the blur). The tint becomes
/// the node's bottommost paint: the empty-background common case takes the
/// `background` slot; an authored background shifts up one layer.
fn apply_glass_tint(change: &KiwiValue, node: &mut CanvasNode) {
    let Some(effects) = change.get("effects").and_then(KiwiValue::as_array) else {
        return;
    };
    let Some(tint) = effects.iter().find_map(|effect| {
        if matches!(effect.get("visible"), Some(KiwiValue::Bool(false))) {
            return None;
        }
        if effect.get("type").and_then(KiwiValue::as_str) != Some("GLASS") {
            return None;
        }
        effect
            .get("color")
            .and_then(|color| read_color_with_opacity(color, 1.0))
            .filter(|color| color.a > 0)
    }) else {
        return;
    };
    let tint = Fill::solid(tint);
    match &mut node.data {
        NodeData::Group(group) => match group.background.take() {
            Some(background) => {
                group.background = Some(tint);
                group.background_fills.insert(0, background);
            }
            None => group.background = Some(tint),
        },
        NodeData::Vector(vector) => vector.fills.insert(0, tint),
        _ => {}
    }
}

/// Effect fidelity — counted off the SOURCE change (the built node no longer
/// knows which member each blur came from): GLASS approximations and effect
/// kinds we drop outright, so per-file loss is a number, never silence.
fn tally_effect_fidelity(report: &mut MapReport, change: &KiwiValue) {
    let Some(effects) = change.get("effects").and_then(KiwiValue::as_array) else {
        return;
    };
    for effect in effects {
        if matches!(effect.get("visible"), Some(KiwiValue::Bool(false))) {
            continue;
        }
        match effect.get("type").and_then(KiwiValue::as_str) {
            Some("GLASS") => report.effects_glass_approximated += 1,
            Some(
                "DROP_SHADOW" | "INNER_SHADOW" | "FOREGROUND_BLUR" | "LAYER_BLUR"
                | "BACKGROUND_BLUR",
            )
            | None => {}
            Some(_) => report.effects_dropped_unknown += 1,
        }
    }
}

/// Scroll semantics — counted off the BUILT node: frames whose authored
/// overflow enables an axis, and children pinned against ancestor scrolling.
fn tally_scroll(report: &mut MapReport, node: &CanvasNode) {
    if let NodeData::Group(group) = &node.data
        && group
            .scroll_direction
            .is_some_and(|direction| direction != ScrollDirection::None)
    {
        report.scroll_frames += 1;
    }
    if !node.scroll_behavior.is_default() {
        report.scroll_pinned_children += 1;
    }
}
