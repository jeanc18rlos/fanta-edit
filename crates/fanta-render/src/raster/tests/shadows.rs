//! Shadow-fidelity tests: `spread` is a GEOMETRIC inflate/erode of the
//! silhouette (not a blur term), drop shadows paint behind / inner shadows
//! clip inside, and stacked drop shadows all contribute. These guard the
//! Figma/OpenPencil semantics the spread/stacking fix introduced.
use super::*;
// `shadow_world_sigma` / `shadow_world_spread` are `pub(crate)` helpers in the
// `effects` submodule but not re-exported through `raster/mod.rs`; reach them by
// their module path so this test stays additive (no out-of-scope re-export edit).
use crate::raster::effects::{shadow_world_sigma, shadow_world_spread};
use fanta_doc::{Doc, Operation, VectorNode};

/// Build a single-rect doc with one drop shadow of the given `spread`, render
/// it into a 96x96 surface, and return the straight-RGBA buffer. The rect is a
/// 20x20 opaque red square centred on the origin → screen (38,38)..(58,58); the
/// shadow has no offset, a small fixed blur, and only `spread` varies, so the
/// only thing that can change between calls is how far the shadow extends.
fn drop_spread_buf(spread: f64) -> Vec<u8> {
    let mut doc = Doc::new();
    let mut n = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        -10.0,
        -10.0,
        20.0,
        20.0,
        Color::rgb(255, 0, 0),
    )));
    n.effects.push(Shadow {
        kind: ShadowKind::Drop,
        color: Color::rgba(0, 0, 0, 255),
        blur: 4.0,
        spread,
        offset: [0.0, 0.0],
        show_behind_node: false,
    });
    doc.apply(Operation::create_node(n)).unwrap();
    let mut r = RasterRenderer::new(96, 96).unwrap();
    r.render(&doc.scene, &doc.viewport);
    r.copy_rgba()
}

#[test]
fn positive_spread_expands_the_drop_shadow_extent() {
    // The fix: a positive `spread` DILATES the shadow's silhouette before the
    // blur (Figma/OpenPencil inflate the shadow rect by +spread on each side),
    // so the shadow reaches farther from the rect than a spread-0 shadow with
    // the same blur. The old code folded spread into the blur sigma, which made
    // the shadow fuzzier but NOT meaningfully larger — and at a fixed sample
    // well outside the rect that fuzz is far weaker than a real dilation.
    let none = drop_spread_buf(0.0);
    let wide = drop_spread_buf(12.0);

    // Sample 6 px outside the rect's right edge. Rect right edge is screen
    // x=58; sample at (64, 48) is 6 px out — inside a +12 spread shadow's
    // dilated silhouette, but past a spread-0 shadow's faint blur tail.
    let s_none = rgba_at(&none, 96, 64, 48);
    let s_wide = rgba_at(&wide, 96, 64, 48);
    assert!(
        s_wide[3] > s_none[3] + 40,
        "positive spread must extend the shadow farther out: \
         spread0 alpha {} vs spread12 alpha {}",
        s_none[3],
        s_wide[3]
    );
    // And the dilated shadow covers strictly more opaque pixels overall.
    assert!(
        opaque_pixel_count(&wide) > opaque_pixel_count(&none),
        "a +spread shadow must cover more pixels: {} vs {}",
        opaque_pixel_count(&wide),
        opaque_pixel_count(&none)
    );
}

#[test]
fn negative_spread_shrinks_the_drop_shadow() {
    // Negative spread ERODES the silhouette: the shadow pulls IN toward the
    // rect, so it covers fewer pixels than a spread-0 shadow. The old path
    // `.max(0.0)`-clamped spread and dropped the negative case entirely — this
    // is the regression guard for it.
    let none = drop_spread_buf(0.0);
    let tight = drop_spread_buf(-6.0);
    assert!(
        opaque_pixel_count(&tight) < opaque_pixel_count(&none),
        "a negative spread must shrink the shadow: {} vs {}",
        opaque_pixel_count(&tight),
        opaque_pixel_count(&none)
    );
    // The rect body itself is untouched — negative spread only affects the
    // shadow, the node still paints opaque red over it.
    let centre = rgba_at(&tight, 96, 48, 48);
    assert!(
        centre[0] > 200 && centre[1] < 40 && centre[2] < 40 && centre[3] > 200,
        "negative spread must not eat the node body, got {centre:?}"
    );
}

#[test]
fn stacked_drop_shadows_each_contribute_on_their_own_side() {
    // Figma stacks multiple drop shadows. Two opaque shadows offset to OPPOSITE
    // sides (one left, one right) of a small rect must BOTH paint: the merge of
    // shadow-only layers keeps every shadow, rather than one masking the other.
    // Without independent stacking, only one side would show.
    let mut doc = Doc::new();
    let mut n = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        -8.0,
        -8.0,
        16.0,
        16.0,
        Color::rgb(255, 0, 0),
    )));
    // Shadow A: hard (crisp) black, pushed left.
    n.effects.push(Shadow {
        kind: ShadowKind::Drop,
        color: Color::rgba(0, 0, 0, 255),
        blur: 2.0,
        spread: 0.0,
        offset: [-18.0, 0.0],
        show_behind_node: false,
    });
    // Shadow B: hard black, pushed right.
    n.effects.push(Shadow {
        kind: ShadowKind::Drop,
        color: Color::rgba(0, 0, 0, 255),
        blur: 2.0,
        spread: 0.0,
        offset: [18.0, 0.0],
        show_behind_node: false,
    });
    doc.apply(Operation::create_node(n)).unwrap();
    let mut r = RasterRenderer::new(96, 96).unwrap();
    r.render(&doc.scene, &doc.viewport);
    let buf = r.copy_rgba();

    // Rect spans screen (40,40)..(56,56), centred (48,48). Shadow A lands ~18px
    // left → around screen x=30; shadow B ~18px right → around screen x=66.
    let left = rgba_at(&buf, 96, 30, 48);
    let right = rgba_at(&buf, 96, 66, 48);
    assert!(
        left[3] > 0 && left[0] < 120,
        "the left-offset shadow must paint a dark pixel left of the rect, got {left:?}"
    );
    assert!(
        right[3] > 0 && right[0] < 120,
        "the right-offset shadow must paint a dark pixel right of the rect, got {right:?}"
    );
}

#[test]
fn drop_shadow_paints_behind_the_node_with_spread() {
    // Even with a large spread, the node draws OVER its own shadow — the
    // shadow is a background effect, never a foreground tint. The rect centre
    // stays opaque red.
    let buf = drop_spread_buf(10.0);
    let centre = rgba_at(&buf, 96, 48, 48);
    assert!(
        centre[0] > 200 && centre[1] < 40 && centre[2] < 40 && centre[3] > 200,
        "node body must paint over its spread shadow, got {centre:?}"
    );
}

/// Render a big white rect with one inner shadow of the given `spread` (no
/// offset, fixed blur), into a 96x96 surface; return the RGBA buffer. The rect
/// is 60x60 centred on origin → screen (18,18)..(78,78). With no offset the
/// inner shadow rings the WHOLE inner edge; `spread` controls how far the ring
/// reaches inward.
fn inner_spread_buf(spread: f64) -> Vec<u8> {
    let mut doc = Doc::new();
    let mut n = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        -30.0,
        -30.0,
        60.0,
        60.0,
        Color::rgb(255, 255, 255),
    )));
    n.effects.push(Shadow {
        kind: ShadowKind::Inner,
        color: Color::rgba(0, 0, 0, 255),
        blur: 4.0,
        spread,
        offset: [0.0, 0.0],
        show_behind_node: false,
    });
    doc.apply(Operation::create_node(n)).unwrap();
    let mut r = RasterRenderer::new(96, 96).unwrap();
    r.render(&doc.scene, &doc.viewport);
    r.copy_rgba()
}

#[test]
fn inner_shadow_positive_spread_thickens_the_ring_inward() {
    // For an INNER shadow, positive spread chokes the shape inward → a THICKER
    // inner ring that reaches farther toward the centre. We sample a point well
    // inside the edge (12 px in from the left edge): a spread-0 inner ring's
    // soft band has faded out there, but a +spread ring still darkens it.
    let none = inner_spread_buf(0.0);
    let wide = inner_spread_buf(10.0);

    // Rect left edge is screen x=18; sample at (30, 48) is 12 px inside.
    let s_none = rgba_at(&none, 96, 30, 48);
    let s_wide = rgba_at(&wide, 96, 30, 48);
    // The wider-spread inner shadow is darker (lower luma) at this interior
    // point — the ring reaches it; the no-spread ring has faded.
    assert!(
        (s_wide[0] as i32) < (s_none[0] as i32) - 30,
        "positive inner spread must darken farther inward: \
         spread0 {s_none:?} vs spread10 {s_wide:?}"
    );
    // Both stay opaque (the inner shadow is clipped inside the filled shape).
    assert!(
        s_wide[3] > 200 && s_none[3] > 200,
        "inner-shadowed pixels stay inside the opaque shape: {s_none:?} / {s_wide:?}"
    );
}

#[test]
fn inner_shadow_with_spread_does_not_bleed_outside() {
    // The no-bleed invariant must hold regardless of spread: a rect with ONLY
    // an inner shadow (even a big positive spread) paints nothing outside its
    // own bounds — the shape clip confines it.
    let buf = inner_spread_buf(10.0);
    // Rect spans screen (18,18)..(78,78); (10,10) is up-left and outside.
    let outside = rgba_at(&buf, 96, 10, 10);
    assert_eq!(
        outside[3], 0,
        "an inner shadow with spread must not bleed outside the node, got {outside:?}"
    );
}

// -----------------------------------------------------------------------
// Pure helper units: spread is split out of the blur sigma.
// -----------------------------------------------------------------------

#[test]
fn shadow_world_sigma_no_longer_folds_spread() {
    // The faithful blur sigma is exactly `blur / 2`, INDEPENDENT of spread —
    // spread is geometric now, not a blur term. (The old code returned
    // `(blur + spread*0.5) / 2`, which this guards against regressing to.)
    let base = Shadow {
        kind: ShadowKind::Drop,
        color: Color::rgba(0, 0, 0, 255),
        blur: 8.0,
        spread: 0.0,
        offset: [0.0, 0.0],
        show_behind_node: false,
    };
    let spread_pos = Shadow {
        spread: 20.0,
        ..base
    };
    let spread_neg = Shadow {
        spread: -20.0,
        ..base
    };
    assert!((shadow_world_sigma(&base) - 4.0).abs() < 1e-9);
    // Spread (either sign) does not move the sigma.
    assert_eq!(shadow_world_sigma(&base), shadow_world_sigma(&spread_pos));
    assert_eq!(shadow_world_sigma(&base), shadow_world_sigma(&spread_neg));
}

#[test]
fn shadow_world_spread_passes_signed_spread_through() {
    // The spread accessor carries the signed value (positive = grow, negative =
    // shrink) the morphology filter consumes.
    let mk = |spread: f64| Shadow {
        kind: ShadowKind::Drop,
        color: Color::rgba(0, 0, 0, 255),
        blur: 4.0,
        spread,
        offset: [0.0, 0.0],
        show_behind_node: false,
    };
    assert_eq!(shadow_world_spread(&mk(12.0)), 12.0);
    assert_eq!(shadow_world_spread(&mk(-7.5)), -7.5);
    assert_eq!(shadow_world_spread(&mk(0.0)), 0.0);
}

#[test]
fn shadow_expanded_bounds_grows_by_positive_spread() {
    // The cull box must account for a positive spread's outward dilation: with
    // blur 0 (sigma 0, reach 0) and spread 10, a 20x20 rect's drop shadow box
    // grows by exactly +10 on each side → [-20,-20,20,20]. (The old code grew
    // the box via the sigma-folded spread; now it adds spread directly.)
    let mut doc = Doc::new();
    let mut n = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        -10.0,
        -10.0,
        20.0,
        20.0,
        Color::rgb(255, 0, 0),
    )));
    n.effects.push(Shadow {
        kind: ShadowKind::Drop,
        color: Color::rgba(0, 0, 0, 255),
        blur: 0.0,
        spread: 10.0,
        offset: [0.0, 0.0],
        show_behind_node: false,
    });
    let id = n.id;
    doc.apply(Operation::create_node(n)).unwrap();
    let world = doc.scene.world_bounds(id).unwrap();
    let exp = shadow_expanded_world_bounds(
        &doc.scene,
        id,
        &doc.scene.get(id).unwrap().effects,
        &[],
        world,
        1.0,
    );
    assert!((exp.min_x - (-20.0)).abs() < 1e-6, "min_x {}", exp.min_x);
    assert!((exp.min_y - (-20.0)).abs() < 1e-6, "min_y {}", exp.min_y);
    assert!((exp.max_x - 20.0).abs() < 1e-6, "max_x {}", exp.max_x);
    assert!((exp.max_y - 20.0).abs() < 1e-6, "max_y {}", exp.max_y);
}

#[test]
fn frame_inner_shadow_composites_over_children() {
    // Figma applies node-level effects to the COMPOSITED node: a frame's inner
    // shadow darkens edge-touching children instead of hiding beneath them. The
    // old walk painted inner shadows before the child recursion, so a full-bleed
    // opaque child completely covered the shadow ring.
    use fanta_doc::GroupNode;

    let mut doc = Doc::new();
    let mut frame = CanvasNode::new(NodeData::Group(GroupNode {
        clip_size: Some([40.0, 40.0]),
        background: Some(Fill::solid(Color::rgb(255, 255, 255))),
        explicit_modes: Default::default(),
        ..Default::default()
    }));
    frame.transform = Transform2D::translation(-20.0, -20.0);
    frame.effects.push(Shadow {
        kind: ShadowKind::Inner,
        color: Color::rgba(0, 0, 0, 255),
        blur: 12.0,
        spread: 0.0,
        offset: [0.0, 0.0],
        show_behind_node: false,
    });
    let frame_id = frame.id;
    doc.apply(Operation::create_node(frame)).unwrap();

    // Full-bleed opaque green child covering the whole frame box.
    let mut child = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        0.0,
        0.0,
        40.0,
        40.0,
        Color::rgb(0, 200, 0),
    )));
    child.parent = Some(frame_id);
    doc.apply(Operation::create_node(child)).unwrap();

    let mut r = RasterRenderer::new(64, 64).unwrap();
    r.render(&doc.scene, &doc.viewport);
    let buf = r.copy_rgba();

    // Frame box is screen (12,12)..(52,52). 2px inside the left edge the inner
    // shadow must darken the green child; the centre stays (nearly) pure green.
    let edge = rgba_at(&buf, 64, 14, 32);
    let centre = rgba_at(&buf, 64, 32, 32);
    assert!(
        centre[1] > 160,
        "centre must stay the child's green, got {centre:?}"
    );
    assert!(
        edge[1] + 40 < centre[1],
        "inner shadow must darken the child at the frame edge: edge {edge:?} vs centre {centre:?}"
    );
}

#[test]
fn instance_inner_shadow_uses_its_box_silhouette() {
    // `node_silhouette_path` used to return `None` for Instance nodes, silently
    // dropping their inner shadows (UI3 carries 38 of them). An instance now
    // rings its `local_size` box, matching Figma.
    use fanta_doc::{ComponentDef, ComponentId, ComponentLibrary, InstanceNode, VariableRegistry};

    let mut doc = Doc::new();
    let mut master = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        0.0,
        0.0,
        20.0,
        20.0,
        Color::rgb(255, 0, 0),
    )));
    master.transform = Transform2D::translation(10_000.0, 0.0);
    let root = master.id;
    doc.apply(Operation::create_node(master)).unwrap();

    let comp = ComponentId::new();
    let mut lib = ComponentLibrary::new();
    lib.defs.insert(comp, ComponentDef::new(comp, root, "Rect"));

    let mut inst = CanvasNode::new(NodeData::Instance(InstanceNode {
        component: comp,
        overrides: Vec::new(),
        prop_values: Default::default(),
        derived: Vec::new(),
        local_size: [20.0, 20.0],
    }));
    inst.transform = Transform2D::translation(-10.0, -10.0);
    inst.effects.push(Shadow {
        kind: ShadowKind::Inner,
        color: Color::rgba(0, 0, 0, 255),
        blur: 8.0,
        spread: 0.0,
        offset: [0.0, 0.0],
        show_behind_node: false,
    });
    doc.apply(Operation::create_node(inst)).unwrap();

    let registry = VariableRegistry::new();
    let inputs = RenderInputs {
        components: &lib,
        variables: &registry,
        active_modes: &std::collections::BTreeMap::new(),
        mode_generation: 0,
        playback: None,
        dark_ui: false,
    };
    let mut r = RasterRenderer::new(64, 64).unwrap();
    r.render_with(&doc.scene, &doc.viewport, &inputs);
    let buf = r.copy_rgba();

    // Instance box is screen (22,22)..(42,42). 1px inside the left edge the
    // shadow must darken the master's red; the centre stays (nearly) pure red.
    let edge = rgba_at(&buf, 64, 23, 32);
    let centre = rgba_at(&buf, 64, 32, 32);
    assert!(
        centre[0] > 160,
        "centre must stay the master's red, got {centre:?}"
    );
    assert!(
        edge[0] + 40 < centre[0],
        "instance inner shadow must darken its box edge: edge {edge:?} vs centre {centre:?}"
    );
}
