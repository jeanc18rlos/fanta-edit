//! Node-level effect tests: drop/inner shadows, layer + background blur, blend modes, radial gradient, opacity/visibility, PNG encode, and display-scale.
use super::*;
use fanta_doc::{Doc, IndexKey, Operation, VectorNode};

/// A drop shadow of `color` at `offset` with `blur` (no spread, knocked out
/// behind the node — Figma's default).
fn drop_shadow(color: Color, offset: [f64; 2], blur: f64) -> Shadow {
    Shadow {
        kind: ShadowKind::Drop,
        color,
        blur,
        spread: 0.0,
        offset,
        show_behind_node: false,
    }
}

/// An inner shadow of `color` at `offset` with `blur` (no spread).
fn inner_shadow(color: Color, offset: [f64; 2], blur: f64) -> Shadow {
    Shadow {
        kind: ShadowKind::Inner,
        ..drop_shadow(color, offset, blur)
    }
}

// -----------------------------------------------------------------------
// Node-level effects: drop shadows + blend modes
// -----------------------------------------------------------------------

#[test]
fn drop_shadow_bleeds_pixels_outside_the_node_rect() {
    // A 20x20 opaque red rect centred on the origin, with a black drop shadow
    // offset down-right and blurred. The shadow must paint pixels in the
    // region just OUTSIDE the rect (down-right of it) that is empty when the
    // same rect has no shadow. This proves the effect layer paints the
    // node's silhouette shadow beyond its own bounds (Figma card/button look).
    //
    // Layout (64x64 surface, origin-centred, zoom 1): the rect spans world
    // (-10,-10)..(10,10) → screen pixels (22,22)..(42,42). A sample at screen
    // (48, 48) is below-right of the rect (outside it). With an 8px-down-right
    // shadow + blur it lands in the soft shadow; with no shadow it is empty.
    fn shadowed(with_shadow: bool) -> Vec<u8> {
        let mut doc = Doc::new();
        let mut n = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
            -10.0,
            -10.0,
            20.0,
            20.0,
            Color::rgb(255, 0, 0),
        )));
        if with_shadow {
            n.effects.push(Shadow {
                kind: ShadowKind::Drop,
                color: Color::rgba(0, 0, 0, 255),
                blur: 8.0,
                spread: 0.0,
                offset: [8.0, 8.0],
                show_behind_node: false,
            });
        }
        doc.apply(Operation::create_node(n)).unwrap();
        let mut r = RasterRenderer::new(64, 64).unwrap();
        r.render(&doc.scene, &doc.viewport);
        r.copy_rgba()
    }

    let no_shadow = shadowed(false);
    let with_shadow = shadowed(true);

    // Sample just outside the rect, down-right where the offset shadow lands.
    let plain = rgba_at(&no_shadow, 64, 48, 48);
    let shad = rgba_at(&with_shadow, 64, 48, 48);

    assert_eq!(
        plain[3], 0,
        "without a shadow the pixel outside the rect must be empty, got {plain:?}"
    );
    assert!(
        shad[3] > 0,
        "the drop shadow must bleed opaque pixels outside the rect, got {shad:?}"
    );
    // And it is shadow-dark (low RGB), not the rect's red leaking out.
    assert!(
        shad[0] < 120 && shad[1] < 120 && shad[2] < 120,
        "the bled pixel should be the dark shadow, not red, got {shad:?}"
    );
    // The rect's own centre is still the opaque red it always was — the
    // shadow draws BEHIND the node, not over it.
    let centre = rgba_at(&with_shadow, 64, 32, 32);
    assert!(
        centre[0] > 200 && centre[1] < 40 && centre[2] < 40 && centre[3] > 200,
        "the node itself still paints over its shadow, got {centre:?}"
    );
}

// -----------------------------------------------------------------------
// Node-level effects: blur (layer + background)
// -----------------------------------------------------------------------

#[test]
fn layer_blur_softens_the_node_edge() {
    // A 20x20 opaque red rect centred on the origin. With a LAYER blur its
    // hard edge must spread: a pixel just OUTSIDE the original rect boundary
    // (empty without the blur) gains partial red coverage, and the sharp edge
    // pixel loses full opacity. This proves the layer's content is
    // Gaussian-blurred (Figma LAYER_BLUR), not merely passed through.
    //
    // Layout (64x64, origin-centred, zoom 1): rect spans world (-10,-10)..
    // (10,10) → screen (22,22)..(42,42). Screen (45,32) is 3px right of the
    // rect's right edge (x=42) — empty with no blur, fogged with one.
    fn blurred(radius: f64) -> Vec<u8> {
        let mut doc = Doc::new();
        let mut n = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
            -10.0,
            -10.0,
            20.0,
            20.0,
            Color::rgb(255, 0, 0),
        )));
        if radius > 0.0 {
            n.blurs.push(Blur::layer(radius));
        }
        doc.apply(Operation::create_node(n)).unwrap();
        let mut r = RasterRenderer::new(64, 64).unwrap();
        r.render(&doc.scene, &doc.viewport);
        r.copy_rgba()
    }

    let sharp = blurred(0.0);
    let soft = blurred(12.0);

    // Just outside the right edge: empty when sharp, fogged red when blurred.
    let out_sharp = rgba_at(&sharp, 64, 45, 32);
    let out_soft = rgba_at(&soft, 64, 45, 32);
    assert_eq!(
        out_sharp[3], 0,
        "without a layer blur the pixel outside the rect is empty, got {out_sharp:?}"
    );
    assert!(
        out_soft[3] > 0 && out_soft[3] < 255,
        "a layer blur must spread partial coverage past the edge, got {out_soft:?}"
    );
    // The blurred edge is still reddish (the rect's own color smeared), not a
    // dark shadow — distinguishes layer blur from a drop shadow.
    assert!(
        out_soft[0] >= out_soft[2],
        "the spread should be the rect's red, got {out_soft:?}"
    );

    // The sharp rect is fully opaque at its edge; the blurred one is softened
    // there (alpha pulled below full).
    let edge_sharp = rgba_at(&sharp, 64, 41, 32);
    let edge_soft = rgba_at(&soft, 64, 41, 32);
    assert!(
        edge_sharp[3] > 240,
        "sharp edge stays opaque, got {edge_sharp:?}"
    );
    assert!(
        edge_soft[3] < edge_sharp[3],
        "blurred edge must lose opacity vs the sharp one ({} vs {})",
        edge_soft[3],
        edge_sharp[3]
    );
}

#[test]
fn layer_blur_zero_radius_is_a_noop() {
    // A zero-radius layer blur must render byte-identically to no blur at all
    // (the sub-pixel-sigma fast path keeps the hot path free).
    fn render(with_blur: bool) -> Vec<u8> {
        let mut doc = Doc::new();
        let mut n = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
            -10.0,
            -10.0,
            20.0,
            20.0,
            Color::rgb(0, 200, 0),
        )));
        if with_blur {
            n.blurs.push(Blur::layer(0.0));
        }
        doc.apply(Operation::create_node(n)).unwrap();
        let mut r = RasterRenderer::new(48, 48).unwrap();
        r.render(&doc.scene, &doc.viewport);
        r.copy_rgba()
    }
    assert_eq!(render(false), render(true), "0-radius blur must be a no-op");
}

#[test]
fn background_blur_frosts_the_backdrop_within_the_node() {
    // A high-frequency checkerboard backdrop (alternating black/white rects),
    // with a semi-transparent panel carrying a BACKGROUND blur on top. Inside
    // the panel the backdrop must be frosted: the local pixel variance there
    // drops sharply versus the same backdrop with no blurring panel, because
    // the Gaussian averages the black/white checker toward grey. Proves the
    // backdrop (not the panel) is blurred (Figma BACKGROUND_BLUR).
    fn render(with_bg_blur: bool) -> Vec<u8> {
        let mut doc = Doc::new();
        // Checkerboard of 4x4 cells across the centre region.
        for gy in 0..8 {
            for gx in 0..8 {
                if (gx + gy) % 2 == 0 {
                    continue;
                }
                let x = -16.0 + gx as f64 * 4.0;
                let y = -16.0 + gy as f64 * 4.0;
                let cell = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
                    x,
                    y,
                    4.0,
                    4.0,
                    Color::rgb(0, 0, 0),
                )));
                doc.apply(Operation::create_node(cell)).unwrap();
            }
        }
        // White base so the gaps read white (full checker contrast).
        // A translucent panel covering the centre, with a background blur.
        let mut panel = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
            -12.0,
            -12.0,
            24.0,
            24.0,
            Color::rgba(255, 255, 255, 30),
        )));
        if with_bg_blur {
            panel.blurs.push(Blur::background(10.0));
        }
        // The panel is "on top" (it frosts the checker behind it), so give it a
        // z-index strictly above the cells. Without this every node shares
        // `IndexKey::FIRST` and z-order falls back to the random ULID tie-break,
        // dropping the panel to a random depth — flaky frosting.
        panel.index = IndexKey::after(IndexKey::FIRST);
        doc.apply(Operation::create_node(panel)).unwrap();
        let mut r = RasterRenderer::new(64, 64).unwrap();
        r.background = Color::WHITE;
        r.render(&doc.scene, &doc.viewport);
        r.copy_rgba()
    }

    // Local luma variance over a small window inside the panel.
    fn variance(buf: &[u8]) -> f64 {
        let mut vals = Vec::new();
        for y in 28..36 {
            for x in 28..36 {
                let p = rgba_at(buf, 64, x, y);
                vals.push((p[0] as f64 + p[1] as f64 + p[2] as f64) / 3.0);
            }
        }
        let mean = vals.iter().sum::<f64>() / vals.len() as f64;
        vals.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / vals.len() as f64
    }

    let plain = variance(&render(false));
    let frosted = variance(&render(true));
    assert!(
        frosted < plain * 0.6,
        "a background blur must reduce backdrop variance (frost it): plain {plain:.1} vs frosted {frosted:.1}"
    );
}

#[test]
fn inner_shadow_does_not_bleed_outside_the_node_rect() {
    // Inner shadows are a documented TODO (Skia has no direct filter); we must
    // NOT paint a drop-shadow-like bleed for them. A rect with ONLY an inner
    // shadow must render exactly like a rect with no effects: nothing outside
    // its bounds. (Guards against accidentally treating Inner as Drop.)
    let mut doc = Doc::new();
    let mut n = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        -10.0,
        -10.0,
        20.0,
        20.0,
        Color::rgb(255, 0, 0),
    )));
    n.effects.push(Shadow {
        kind: ShadowKind::Inner,
        color: Color::rgba(0, 0, 0, 255),
        blur: 8.0,
        spread: 0.0,
        offset: [8.0, 8.0],
        show_behind_node: false,
    });
    doc.apply(Operation::create_node(n)).unwrap();
    let mut r = RasterRenderer::new(64, 64).unwrap();
    r.render(&doc.scene, &doc.viewport);
    let buf = r.copy_rgba();

    // Down-right of the rect must stay empty: no drop-shadow bleed.
    let outside = rgba_at(&buf, 64, 48, 48);
    assert_eq!(
        outside[3], 0,
        "an inner shadow must not bleed outside the node, got {outside:?}"
    );
}

#[test]
fn inner_shadow_darkens_the_interior_edge_away_from_the_offset() {
    // An inner shadow is now rendered: it darkens the INSIDE of the shape
    // near the edges, biased opposite the offset direction. We render a big
    // white rect with a black inner shadow offset down-right; the soft band
    // should hug the TOP-LEFT inner edge (the side the offset/blurred copy
    // pulls away from), while the centre stays the white fill. The whole
    // effect stays inside the rect (the no-bleed invariant is its own test).
    fn render(with_inner: bool) -> Vec<u8> {
        let mut doc = Doc::new();
        // 40x40 white rect centred on origin → world (-20,-20)..(20,20).
        let mut n = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
            -20.0,
            -20.0,
            40.0,
            40.0,
            Color::rgb(255, 255, 255),
        )));
        if with_inner {
            n.effects.push(Shadow {
                kind: ShadowKind::Inner,
                color: Color::rgba(0, 0, 0, 255),
                blur: 6.0,
                spread: 0.0,
                offset: [6.0, 6.0],
                show_behind_node: false,
            });
        }
        doc.apply(Operation::create_node(n)).unwrap();
        let mut r = RasterRenderer::new(64, 64).unwrap();
        r.render(&doc.scene, &doc.viewport);
        r.copy_rgba()
    }

    let plain = render(false);
    let inner = render(true);

    // Screen layout (64x64, origin-centred, zoom 1): world (-20,-20) maps to
    // screen (12,12); the rect spans screen (12,12)..(52,52). Sample just
    // inside the TOP-LEFT corner — where the inner shadow concentrates.
    let plain_tl = rgba_at(&plain, 64, 15, 15);
    let inner_tl = rgba_at(&inner, 64, 15, 15);
    // Without the effect the corner is the white fill.
    assert!(
        plain_tl[0] > 230 && plain_tl[1] > 230 && plain_tl[2] > 230 && plain_tl[3] > 200,
        "plain rect corner should be opaque white, got {plain_tl:?}"
    );
    // With it, the top-left inner edge is visibly darkened (shadow blends
    // black over the white fill) yet still opaque (clipped inside the shape).
    assert!(
        inner_tl[0] < plain_tl[0].saturating_sub(40),
        "inner shadow must darken the top-left inner edge: plain {plain_tl:?} vs inner {inner_tl:?}"
    );
    assert!(
        inner_tl[3] > 200,
        "the inner-shadowed pixel is still inside the opaque shape, got {inner_tl:?}"
    );
    // The centre of the rect is far from every edge, so the shadow band does
    // not reach it — it stays (near) white, proving the effect is an EDGE
    // ring, not a flat tint over the whole fill.
    let centre = rgba_at(&inner, 64, 32, 32);
    assert!(
        centre[0] > 200 && centre[1] > 200 && centre[2] > 200,
        "the rect centre should stay near-white (shadow is an inner ring), got {centre:?}"
    );
}

#[test]
fn radial_gradient_is_a_node_aspect_ellipse_not_a_circle() {
    // A radial gradient on a WIDE (non-square) node must stretch into an
    // ellipse matching the node's aspect — the fixed `radius·(w+h)·0.5`
    // approximation forced a single circular radius (the bug this replaces).
    // We fill a 60x20 rect (3:1) with a white-centre → black-edge radial
    // gradient and compare, at the SAME pixel distance from centre, a sample
    // along the SHORT (vertical) axis vs the LONG (horizontal) axis. For an
    // ellipse the short axis reaches the dark edge sooner, so the vertical
    // sample is darker than the horizontal one. A circle would make them
    // equal (a wrong, axis-symmetric falloff).
    use fanta_doc::{Fill, Gradient, GradientStop};
    let mut doc = Doc::new();
    let mut v = VectorNode::rect_solid(-30.0, -10.0, 60.0, 20.0, Color::rgb(0, 0, 0));
    // Replace the solid fill (rect_solid seeds exactly one) with the radial.
    v.fills[0] = Fill::Gradient {
        blend: fanta_doc::BlendMode::Normal,
        gradient: Gradient::Radial {
            center: [0.5, 0.5],
            radius: 0.5,
            handles: None,
            stops: vec![
                GradientStop {
                    position: 0.0,
                    color: Color::rgb(255, 255, 255),
                },
                GradientStop {
                    position: 1.0,
                    color: Color::rgb(0, 0, 0),
                },
            ],
        },
    };
    doc.apply(Operation::create_node(CanvasNode::new(NodeData::Vector(v))))
        .unwrap();
    let mut r = RasterRenderer::new(80, 80).unwrap();
    r.render(&doc.scene, &doc.viewport);
    let buf = r.copy_rgba();

    // Screen centre is (40, 40) (origin-centred, zoom 1). The rect spans
    // screen x 10..70, y 30..50. Sample 8 px from centre on each axis — well
    // inside the rect on both — so the only difference is the elliptical
    // falloff. Vertical 8 px is 80% of the 10 px half-height (near the dark
    // edge); horizontal 8 px is ~27% of the 30 px half-width (still bright).
    let along_long = rgba_at(&buf, 80, 48, 40); // +8 px on the wide axis
    let along_short = rgba_at(&buf, 80, 40, 48); // +8 px on the narrow axis
    assert!(
        (along_short[0] as i32) < (along_long[0] as i32) - 40,
        "radial gradient must fall off faster on the short axis (ellipse): \
             short {along_short:?} should be darker than long {along_long:?}"
    );
}

#[test]
fn multiply_blend_composites_darker_than_normal() {
    // Two overlapping rects: a yellow base and a cyan top. Under Normal blend
    // the top simply covers the base (cyan shows). Under Multiply the overlap
    // is the component-wise product — yellow (255,255,0) * cyan (0,255,255) /
    // 255 = (0,255,0), pure green — which is visibly different from the cyan
    // that Normal produces. We assert the overlap pixel differs between the
    // two modes, and specifically that Multiply darkens the red channel to ~0
    // while Normal keeps it at the cyan's red (0) — so we check the GREEN/BLUE
    // split that distinguishes green (Multiply) from cyan (Normal).
    fn overlap_pixel(blend: BlendMode) -> [u8; 4] {
        let mut doc = Doc::new();
        // Base: yellow 30x30 centred on origin (drawn first, bottom).
        let base = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
            -15.0,
            -15.0,
            30.0,
            30.0,
            Color::rgb(255, 255, 0),
        )));
        doc.apply(Operation::create_node(base)).unwrap();
        // Top: cyan 30x30 at the same spot, with the blend mode under test.
        // A strictly-higher z-index forces it to draw ON TOP of the base, so
        // the overlap pixel is base⊕top under `blend` (both rects default to
        // IndexKey::FIRST otherwise, an undefined tie).
        let mut top = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
            -15.0,
            -15.0,
            30.0,
            30.0,
            Color::rgb(0, 255, 255),
        )));
        top.index = fanta_doc::IndexKey::after(fanta_doc::IndexKey::FIRST);
        top.blend_mode = blend;
        doc.apply(Operation::create_node(top)).unwrap();

        let mut r = RasterRenderer::new(64, 64).unwrap();
        r.render(&doc.scene, &doc.viewport);
        let buf = r.copy_rgba();
        rgba_at(&buf, 64, 32, 32)
    }

    let normal = overlap_pixel(BlendMode::Normal);
    let multiply = overlap_pixel(BlendMode::Multiply);

    // The two modes must produce different pixels at the overlap.
    assert_ne!(
        normal, multiply,
        "Multiply must composite differently than Normal, both were {normal:?}"
    );
    // Normal: the cyan top covers the base → blue channel high.
    assert!(
        normal[2] > 200,
        "Normal blend should show the cyan top (blue high), got {normal:?}"
    );
    // Multiply: yellow * cyan = green → green high, blue driven down to ~0.
    assert!(
        multiply[1] > 200 && multiply[2] < 60,
        "Multiply of yellow*cyan should be green (blue ~0), got {multiply:?}"
    );
}

#[test]
fn normal_blend_and_no_effects_render_unchanged() {
    // The fast path: a plain rect with Normal blend and no effects must be
    // pixel-identical to before the effects layer existed — the centre is the
    // opaque rect colour and nothing bleeds outside it. (Regression guard that
    // begin_effects_layer is a true no-op for the common case.)
    let doc = red_rect_doc(20.0, 20.0);
    let mut r = RasterRenderer::new(64, 64).unwrap();
    r.render(&doc.scene, &doc.viewport);
    let buf = r.copy_rgba();
    let centre = rgba_at(&buf, 64, 32, 32);
    assert!(
        centre[0] > 200 && centre[1] < 30 && centre[2] < 30 && centre[3] > 200,
        "plain rect centre still opaque red, got {centre:?}"
    );
    // The rect spans screen (22,22)..(42,42); (50,50) is outside and empty.
    let outside = rgba_at(&buf, 64, 50, 50);
    assert_eq!(
        outside[3], 0,
        "no effect ⇒ nothing outside the rect, got {outside:?}"
    );
}

// -----------------------------------------------------------------------
// Opacity fold: single-draw leaves skip the opacity save-layer
// -----------------------------------------------------------------------

/// A blue backdrop rect with a translucent red rect on top. `layered` wraps
/// the red rect in a full-opacity-child / half-opacity GROUP (which must take
/// the save-layer path); otherwise the red VECTOR itself carries the opacity
/// (the fold path). Both must composite the same pixels.
fn half_red_over_blue(layered: bool, strokes: usize) -> (Vec<u8>, RenderMetrics) {
    use fanta_doc::{GroupNode, Stroke, UnitInterval};
    let mut doc = Doc::new();
    let base = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        -20.0,
        -20.0,
        40.0,
        40.0,
        Color::rgb(0, 0, 255),
    )));
    doc.apply(Operation::create_node(base)).unwrap();

    let mut red = VectorNode::rect_solid(-10.0, -10.0, 20.0, 20.0, Color::rgb(255, 0, 0));
    for _ in 0..strokes {
        red.strokes.push(Stroke::solid(Color::rgb(0, 255, 0), 4.0));
    }
    let mut red = CanvasNode::new(NodeData::Vector(red));
    red.index = IndexKey::after(IndexKey::FIRST);
    if layered {
        let mut group = CanvasNode::new(NodeData::Group(GroupNode::default()));
        group.index = IndexKey::after(IndexKey::FIRST);
        group.opacity = UnitInterval::new(0.5);
        let group_id = group.id;
        doc.apply(Operation::create_node(group)).unwrap();
        red.parent = Some(group_id);
    } else {
        red.opacity = UnitInterval::new(0.5);
    }
    doc.apply(Operation::create_node(red)).unwrap();

    let mut r = RasterRenderer::new(64, 64).unwrap();
    let metrics = r.render(&doc.scene, &doc.viewport);
    (r.copy_rgba(), metrics)
}

#[test]
fn single_fill_leaf_opacity_folds_into_paint_and_matches_the_layer() {
    // The fold path (vector opacity 0.5, one fill) pushes NO effects layer and
    // composites within 1 LSB of the layered reference (group opacity 0.5
    // around the same opaque fill) — the pixel-identity contract of
    // `opacity_folds_into_paint`.
    let (folded, fm) = half_red_over_blue(false, 0);
    let (layered, lm) = half_red_over_blue(true, 0);
    assert_eq!(fm.effect_layers, 0, "single-fill leaf must not layer");
    assert_eq!(fm.opacity_folds, 1);
    assert_eq!(lm.effect_layers, 1, "the group reference must layer");
    assert_eq!(lm.opacity_folds, 0);
    let f = rgba_at(&folded, 64, 32, 32);
    let l = rgba_at(&layered, 64, 32, 32);
    // 50% red over opaque blue: (~128, 0, ~127, 255).
    assert!(
        f[0] > 110 && f[0] < 145 && f[2] > 110 && f[2] < 145 && f[3] == 255,
        "folded centre should be half red over blue, got {f:?}"
    );
    for (a, b) in folded.iter().zip(&layered) {
        assert!(
            a.abs_diff(*b) <= 1,
            "fold and layer must be pixel-identical (±1 LSB); differ: {f:?} vs {l:?}"
        );
    }
}

#[test]
fn single_stroke_leaf_opacity_folds_but_fill_plus_stroke_keeps_the_layer() {
    use fanta_doc::{Stroke, UnitInterval};
    // Stroke-only leaf: one draw → folds.
    let mut ring = VectorNode::rect_solid(-10.0, -10.0, 20.0, 20.0, Color::rgb(255, 0, 0));
    ring.fills.clear();
    ring.strokes.push(Stroke::solid(Color::rgb(255, 0, 0), 4.0));
    let mut ring = CanvasNode::new(NodeData::Vector(ring));
    ring.opacity = UnitInterval::new(0.5);
    let mut doc = Doc::new();
    doc.apply(Operation::create_node(ring)).unwrap();
    let mut r = RasterRenderer::new(64, 64).unwrap();
    let m = r.render(&doc.scene, &doc.viewport);
    assert_eq!((m.effect_layers, m.opacity_folds), (0, 1));
    let edge = rgba_at(&r.copy_rgba(), 64, 22, 32);
    assert!(
        edge[0] > 200 && edge[3] > 100 && edge[3] < 160,
        "stroke-only leaf at 50% should paint half-alpha red on its edge, got {edge:?}"
    );

    // Fill + stroke: two overlapping draws → the layer must stay (the stroke
    // covers the fill BEFORE the alpha applies, which per-draw alpha cannot
    // reproduce), and the result equals the layered group reference.
    let (folded, fm) = half_red_over_blue(false, 1);
    let (layered, lm) = half_red_over_blue(true, 1);
    assert_eq!((fm.effect_layers, fm.opacity_folds), (1, 0));
    assert_eq!((lm.effect_layers, lm.opacity_folds), (1, 0));
    assert_eq!(folded, layered, "layered leaf equals the layered group");
}

#[test]
fn opacity_fold_declines_multi_draw_blend_and_layer_effects() {
    use fanta_doc::{Fill, GroupNode, Stroke, UnitInterval, VectorNode};
    let leaf = |v: VectorNode| CanvasNode::new(NodeData::Vector(v));
    let plain = || VectorNode::rect_solid(0.0, 0.0, 10.0, 10.0, Color::rgb(1, 2, 3));
    // The fold decision reads the node's ZOOM-EFFECTIVE effects; at scale 1
    // every authored effect of these fixtures paints, so this is the
    // authored-list behaviour.
    let folds = |node: &CanvasNode, has_children: bool| {
        opacity_folds_into_paint(node, has_children, visible_effects(node, 1.0))
    };

    assert!(folds(&leaf(plain()), false));
    // A zero-width stroke paints nothing, so fill + zero-stroke is one draw.
    let mut zero_stroke = plain();
    zero_stroke.strokes.push(Stroke::solid(Color::BLACK, 0.0));
    assert!(folds(&leaf(zero_stroke), false));

    // Two fills, a per-paint blend, an image paint, a per-side border, or a
    // second drawing stroke: keep the layer.
    let mut two_fills = plain();
    two_fills.fills.push(Fill::solid(Color::WHITE));
    assert!(!folds(&leaf(two_fills), false));
    let mut blended = plain();
    blended.fills[0] = Fill::Solid {
        color: Color::WHITE,
        blend: BlendMode::Multiply,
    };
    assert!(!folds(&leaf(blended), false));
    let mut per_side = plain();
    per_side.fills.clear();
    let mut stroke = Stroke::solid(Color::BLACK, 1.0);
    stroke.per_side = Some([1.0, 2.0, 1.0, 2.0]);
    per_side.strokes.push(stroke);
    assert!(!folds(&leaf(per_side), false));
    let mut fill_and_stroke = plain();
    fill_and_stroke
        .strokes
        .push(Stroke::solid(Color::BLACK, 1.0));
    assert!(!folds(&leaf(fill_and_stroke), false));

    // Node-level layer reasons and non-leaves never fold.
    assert!(!folds(&leaf(plain()), true));
    let mut blend = leaf(plain());
    blend.blend_mode = BlendMode::Screen;
    assert!(!folds(&blend, false));
    let mut isolated = leaf(plain());
    isolated.flags |= NodeFlags::ISOLATED_BLEND;
    assert!(!folds(&isolated, false));
    let mut group = CanvasNode::new(NodeData::Group(GroupNode::default()));
    group.opacity = UnitInterval::new(0.5);
    assert!(!folds(&group, false));

    // A visible layer blur / drop shadow / inner shadow blocks the fold; a
    // background blur never does (it frosts the backdrop before the draw).
    let mut blurred = leaf(plain());
    blurred.blurs.push(Blur::layer(8.0));
    assert!(!folds(&blurred, false));
    let mut shadowed = leaf(plain());
    shadowed
        .effects
        .push(drop_shadow(Color::BLACK, [4.0, 4.0], 8.0));
    assert!(!folds(&shadowed, false));
    let mut inner = leaf(plain());
    inner
        .effects
        .push(inner_shadow(Color::BLACK, [2.0, 2.0], 4.0));
    assert!(!folds(&inner, false));
    let mut frosted = leaf(plain());
    frosted.blurs.push(Blur::background(8.0));
    assert!(folds(&frosted, false));
}

/// The zoom-effective effect summary tracks what the filter builders paint:
/// an authored blur / shadow that is sub-pixel at the frame's scale is
/// reported invisible (and so no longer blocks the fold or forces a layer),
/// and becomes visible again once the scale grows.
#[test]
fn visible_effects_follow_the_on_screen_sigma_and_reach() {
    let plain = || {
        CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
            0.0,
            0.0,
            10.0,
            10.0,
            Color::rgb(1, 2, 3),
        )))
    };
    let mut blurred = plain();
    blurred.blurs.push(Blur::layer(2.0)); // world sigma 1
    // On-screen sigma 0.2 < SIGMA_SCREEN_MIN → the filter is skipped.
    assert_eq!(visible_effects(&blurred, 0.2), VisibleEffects::default());
    assert!(visible_effects(&blurred, 1.0).layer_blur);
    // A blur floors at SIGMA_SCREEN_MIN exactly like `capped_render_sigma`.
    assert_eq!(
        visible_effects(&blurred, SIGMA_SCREEN_MIN).layer_blur,
        capped_render_sigma(1.0, SIGMA_SCREEN_MIN) > 0.0
    );

    let mut shadowed = plain();
    shadowed
        .effects
        .push(drop_shadow(Color::BLACK, [1.0, 1.0], 2.0)); // reach 1 + 3 = 4
    // 4 world px × 0.1 = 0.4 device px < SHADOW_SUBPIXEL_MAX_PX → skipped.
    assert_eq!(visible_effects(&shadowed, 0.1), VisibleEffects::default());
    assert!(visible_effects(&shadowed, 1.0).drop_shadow);
    // Zero alpha never paints at any scale.
    let mut clear = plain();
    clear
        .effects
        .push(drop_shadow(Color::TRANSPARENT, [4.0, 4.0], 8.0));
    assert_eq!(visible_effects(&clear, 10.0), VisibleEffects::default());

    let mut inner = plain();
    inner
        .effects
        .push(inner_shadow(Color::BLACK, [1.0, 1.0], 2.0));
    assert!(!visible_effects(&inner, 0.1).inner_shadow);
    assert!(visible_effects(&inner, 1.0).inner_shadow);

    // Background blur is not part of the summary at any scale.
    let mut frosted = plain();
    frosted.blurs.push(Blur::background(64.0));
    assert_eq!(visible_effects(&frosted, 4.0), VisibleEffects::default());
}

/// Render `half_red_over_blue`'s translucent (0.5) red leaf carrying `blurs` +
/// `effects`, at `zoom`, both as a leaf (fold-eligible) and as the reference
/// layered form (a 0.5 group around an opaque leaf with the same effects).
fn translucent_leaf_with_effects(
    layered: bool,
    zoom: f64,
    blurs: &[Blur],
    effects: &[Shadow],
) -> (Vec<u8>, RenderMetrics) {
    use fanta_doc::{GroupNode, IndexKey, UnitInterval};
    let mut doc = Doc::new();
    doc.viewport.zoom = zoom;
    let base = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        -30.0 / zoom,
        -30.0 / zoom,
        60.0 / zoom,
        60.0 / zoom,
        Color::rgb(0, 0, 255),
    )));
    doc.apply(Operation::create_node(base)).unwrap();
    let mut red = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        -10.0 / zoom,
        -10.0 / zoom,
        20.0 / zoom,
        20.0 / zoom,
        Color::rgb(255, 0, 0),
    )));
    red.index = IndexKey::after(IndexKey::FIRST);
    red.blurs.extend(blurs.iter().cloned());
    red.effects.extend(effects.iter().cloned());
    if layered {
        let mut group = CanvasNode::new(NodeData::Group(GroupNode::default()));
        group.index = IndexKey::after(IndexKey::FIRST);
        group.opacity = UnitInterval::new(0.5);
        let group_id = group.id;
        doc.apply(Operation::create_node(group)).unwrap();
        red.parent = Some(group_id);
    } else {
        red.opacity = UnitInterval::new(0.5);
    }
    doc.apply(Operation::create_node(red)).unwrap();
    let mut r = RasterRenderer::new(64, 64).unwrap();
    let metrics = r.render(&doc.scene, &doc.viewport);
    (r.copy_rgba(), metrics)
}

/// FOLD UNLOCK: a translucent leaf whose authored layer blur is sub-pixel at
/// the frame's zoom (world sigma 1 × zoom 0.2 = 0.2 device px <
/// `SIGMA_SCREEN_MIN`) folds its opacity into the paint — no effects layer —
/// and composites within 1 LSB of the layered reference. The same node zoomed
/// to where the blur is visible keeps its layer.
#[test]
fn sub_pixel_layer_blur_no_longer_blocks_the_opacity_fold() {
    let blur = [Blur::layer(2.0)];
    let (folded, fm) = translucent_leaf_with_effects(false, 0.2, &blur, &[]);
    let (layered, lm) = translucent_leaf_with_effects(true, 0.2, &blur, &[]);
    assert_eq!(
        (fm.effect_layers, fm.opacity_folds),
        (0, 1),
        "a sub-pixel blur must not force a layer"
    );
    assert_eq!((lm.effect_layers, lm.opacity_folds), (1, 0));
    let centre = rgba_at(&folded, 64, 32, 32);
    assert!(
        centre[0] > 110 && centre[0] < 145 && centre[2] > 110 && centre[2] < 145,
        "folded centre should be half red over blue, got {centre:?}"
    );
    for (i, (a, b)) in folded.iter().zip(&layered).enumerate() {
        assert!(
            a.abs_diff(*b) <= 1,
            "fold and layer must be pixel-identical (±1 LSB) at byte {i}: {a} vs {b}"
        );
    }
    // Zoomed in (device sigma 2): the blur paints, so the layer stays.
    let (_, visible) = translucent_leaf_with_effects(false, 2.0, &blur, &[]);
    assert_eq!((visible.effect_layers, visible.opacity_folds), (1, 0));
}

/// FOLD UNLOCK for shadows: a translucent leaf whose drop shadow is
/// sub-pixel at this zoom (reach 1 + 3·1 = 4 world px × 0.1 = 0.4 device px)
/// folds and matches the layered reference within 1 LSB; a background blur
/// (drawn before the node, outside any layer) never blocks the fold.
#[test]
fn sub_pixel_shadow_and_background_blur_do_not_block_the_opacity_fold() {
    let shadow = [drop_shadow(Color::BLACK, [1.0, 1.0], 2.0)];
    let (folded, fm) = translucent_leaf_with_effects(false, 0.1, &[], &shadow);
    let (layered, lm) = translucent_leaf_with_effects(true, 0.1, &[], &shadow);
    assert_eq!((fm.effect_layers, fm.opacity_folds), (0, 1));
    assert_eq!((lm.effect_layers, lm.opacity_folds), (1, 0));
    for (a, b) in folded.iter().zip(&layered) {
        assert!(a.abs_diff(*b) <= 1, "fold vs layer differ: {a} vs {b}");
    }
    let (_, visible) = translucent_leaf_with_effects(false, 2.0, &[], &shadow);
    assert_eq!(visible.opacity_folds, 0, "a visible shadow keeps the layer");

    let frost = [Blur::background(8.0)];
    let (folded, fm) = translucent_leaf_with_effects(false, 1.0, &frost, &[]);
    let (layered, lm) = translucent_leaf_with_effects(true, 1.0, &frost, &[]);
    assert_eq!((fm.effect_layers, fm.opacity_folds), (0, 1));
    assert_eq!((lm.effect_layers, lm.opacity_folds), (1, 0));
    for (a, b) in folded.iter().zip(&layered) {
        assert!(a.abs_diff(*b) <= 1, "fold vs layer differ: {a} vs {b}");
    }
}

/// THE BLUR LOD THRESHOLD IS INVISIBLE: a Gaussian layer blur at any on-screen
/// sigma below `SIGMA_SCREEN_MIN` (which the renderer skips entirely) moves no
/// pixel of a hard-edged opaque shape by more than 1 LSB versus actually
/// running the blur — so the skip is a true no-op at 8 bits, at every zoom.
/// Analytically the edge pixel of a unit step blurred by σ picks up
/// `≈ 255·e^(−1/2σ²)` LSB (0.09 at σ = 0.25; the constant is set where that
/// stays under 1). Also pins that the threshold is not loose: at σ = 0.5 the
/// same blur is plainly visible (tens of LSB), which is why it is not raised.
#[test]
fn sub_pixel_layer_blur_gate_is_invisible_at_one_lsb() {
    use skia_safe::{Paint, Rect, image_filters, surfaces};
    let render = |sigma: f32| -> Vec<u8> {
        let mut surface = surfaces::raster_n32_premul((32, 32)).unwrap();
        let canvas = surface.canvas();
        canvas.clear(skia_safe::Color::WHITE);
        let mut paint = Paint::default();
        paint.set_anti_alias(true);
        paint.set_color(skia_safe::Color::BLACK);
        if sigma > 0.0 {
            // A layer with a blur image filter — the same construction
            // `begin_effects_layer` uses for a visible layer blur.
            let mut layer_paint = Paint::default();
            layer_paint.set_image_filter(image_filters::blur((sigma, sigma), None, None, None));
            let rec = skia_safe::canvas::SaveLayerRec::default().paint(&layer_paint);
            canvas.save_layer(&rec);
            canvas.draw_rect(Rect::from_ltrb(8.0, 8.0, 24.0, 24.0), &paint);
            canvas.restore();
        } else {
            canvas.draw_rect(Rect::from_ltrb(8.0, 8.0, 24.0, 24.0), &paint);
        }
        let info = skia_safe::ImageInfo::new(
            (32, 32),
            skia_safe::ColorType::RGBA8888,
            skia_safe::AlphaType::Unpremul,
            None,
        );
        let mut buf = vec![0u8; 32 * 32 * 4];
        assert!(surface.read_pixels(&info, &mut buf, 32 * 4, (0, 0)));
        buf
    };
    let crisp = render(0.0);
    let max_diff = |buf: &[u8]| {
        buf.iter()
            .zip(&crisp)
            .map(|(a, b)| a.abs_diff(*b))
            .max()
            .unwrap()
    };
    // Every sigma the gate skips, up to the threshold itself, is ≤ 1 LSB.
    for sigma in [0.05f32, 0.1, 0.15, 0.2, 0.24, SIGMA_SCREEN_MIN - 1e-3] {
        let d = max_diff(&render(sigma));
        assert!(
            d <= 1,
            "a σ={sigma} blur must be invisible (≤1 LSB) but differs by {d}"
        );
    }
    // Control: doubling the threshold would NOT be invisible.
    let d = max_diff(&render(SIGMA_SCREEN_MIN * 2.0));
    assert!(
        d > 1,
        "σ={} should visibly blur a hard edge (got {d} LSB); if Skia now treats it as a \
         no-op the threshold may be raised",
        SIGMA_SCREEN_MIN * 2.0
    );
}

#[test]
fn png_encode_produces_a_signed_png_header() {
    let mut r = RasterRenderer::new(32, 32).unwrap();
    let doc = red_rect_doc(10.0, 10.0);
    r.render(&doc.scene, &doc.viewport);
    let png = r.encode_png().unwrap();
    // PNG signature: 89 50 4E 47 0D 0A 1A 0A
    assert_eq!(&png[..8], &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]);
}

#[test]
fn display_scale_doubles_world_unit_size_on_surface() {
    // World 20x20 rect rendered into a 64x64 surface at display_scale=1
    // covers a 20x20 pixel block centered at (32, 32). At display_scale=2
    // it covers a 40x40 block centered at the same point — so a sample at
    // pixel (45, 32) is OUTSIDE the rect at scale 1 (rect ends at x=42)
    // but INSIDE at scale 2 (rect ends at x=52).
    let doc = red_rect_doc(20.0, 20.0);

    let mut r1 = RasterRenderer::new(64, 64).unwrap();
    r1.display_scale = 1.0;
    r1.render(&doc.scene, &doc.viewport);
    let buf1 = r1.copy_rgba();
    let sample = |buf: &[u8], x: usize, y: usize| -> [u8; 4] {
        let i = (y * 64 + x) * 4;
        [buf[i], buf[i + 1], buf[i + 2], buf[i + 3]]
    };
    let at_45 = sample(&buf1, 45, 32);
    assert_eq!(
        at_45[3], 0,
        "at scale 1, pixel (45, 32) should be outside the rect"
    );

    let mut r2 = RasterRenderer::new(64, 64).unwrap();
    r2.display_scale = 2.0;
    r2.render(&doc.scene, &doc.viewport);
    let buf2 = r2.copy_rgba();
    let at_45_scaled = sample(&buf2, 45, 32);
    assert!(
        at_45_scaled[0] > 200,
        "at scale 2, pixel (45, 32) should be inside the rect (got {at_45_scaled:?})"
    );
}

#[test]
fn hidden_nodes_do_not_draw() {
    let mut doc = Doc::new();
    let mut n = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        -50.0,
        -50.0,
        100.0,
        100.0,
        Color::rgb(255, 0, 0),
    )));
    n.flags |= NodeFlags::HIDDEN;
    doc.apply(Operation::create_node(n)).unwrap();
    let mut r = RasterRenderer::new(32, 32).unwrap();
    r.render(&doc.scene, &doc.viewport);
    let buf = r.copy_rgba();
    assert!(
        buf.iter().all(|&b| b == 0),
        "hidden node should not contribute pixels"
    );
}

/// The frosted-checker scene of `background_blur_frosts_the_backdrop_within_the_node`
/// at `zoom`, with the panel optionally nested inside `wrap` (a translucent
/// group or a masked group with an image-like checker inside), rendered under
/// the given background-blur strategy.
#[derive(Clone, Copy, PartialEq)]
enum FrostWrap {
    None,
    TranslucentGroup,
    MaskedGroup,
}

fn frosted_checker(backdrop: bool, zoom: f64, wrap: FrostWrap) -> Vec<u8> {
    use fanta_doc::{GroupNode, UnitInterval};
    crate::raster::effects::with_background_blur_backdrop(backdrop, || {
        let mut doc = Doc::new();
        doc.viewport.zoom = zoom;
        // For the masked wrap the checker lives INSIDE the group (it is the
        // "photo" the mask clips); otherwise it is page content below.
        let mut group_id = None;
        if wrap != FrostWrap::None {
            let mut group = CanvasNode::new(NodeData::Group(GroupNode::default()));
            // The group sits ABOVE the page-level checker (translucent wrap).
            group.index = IndexKey::after(IndexKey::after(IndexKey::FIRST));
            if wrap == FrostWrap::TranslucentGroup {
                group.opacity = UnitInterval::new(0.5);
            }
            group_id = Some(group.id);
            doc.apply(Operation::create_node(group)).unwrap();
        }
        if wrap == FrostWrap::MaskedGroup {
            // The mask: a rounded box; masks its following siblings.
            let mut mask = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
                -20.0 / zoom,
                -20.0 / zoom,
                40.0 / zoom,
                40.0 / zoom,
                Color::BLACK,
            )));
            mask.is_mask = true;
            mask.parent = group_id;
            doc.apply(Operation::create_node(mask)).unwrap();
        }
        let checker_parent = if wrap == FrostWrap::MaskedGroup {
            group_id
        } else {
            None
        };
        for gy in 0..8 {
            for gx in 0..8 {
                if (gx + gy) % 2 == 0 {
                    continue;
                }
                let x = (-16.0 + gx as f64 * 4.0) / zoom;
                let y = (-16.0 + gy as f64 * 4.0) / zoom;
                let mut cell = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
                    x,
                    y,
                    4.0 / zoom,
                    4.0 / zoom,
                    Color::rgb(0, 0, 0),
                )));
                cell.parent = checker_parent;
                // Page-level: below the group. In-group: after the mask.
                cell.index = IndexKey::after(IndexKey::FIRST);
                doc.apply(Operation::create_node(cell)).unwrap();
            }
        }
        let mut panel = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
            -12.0 / zoom,
            -12.0 / zoom,
            24.0 / zoom,
            24.0 / zoom,
            Color::rgba(255, 255, 255, 30),
        )));
        panel.blurs.push(Blur::background(10.0 / zoom));
        panel.index = IndexKey::after(IndexKey::after(IndexKey::FIRST));
        panel.parent = group_id;
        doc.apply(Operation::create_node(panel)).unwrap();
        let mut r = RasterRenderer::new(64, 64).unwrap();
        r.background = Color::WHITE;
        r.render(&doc.scene, &doc.viewport);
        r.copy_rgba()
    })
}

/// Local luma variance over the 8×8 window at the panel's centre.
fn centre_variance(buf: &[u8]) -> f64 {
    let mut vals = Vec::new();
    for y in 28..36 {
        for x in 28..36 {
            let p = rgba_at(buf, 64, x, y);
            vals.push((p[0] as f64 + p[1] as f64 + p[2] as f64) / 3.0);
        }
    }
    let mean = vals.iter().sum::<f64>() / vals.len() as f64;
    vals.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / vals.len() as f64
}

/// The default background-blur strategy (Skia's backdrop save-layer) is
/// byte-identical to the read-pixels → blur → blit path for a frosted node
/// whose ancestors push no layer — at every zoom, i.e. the world-space sigma
/// handed to the layer's filter reproduces the device-space blur of the
/// readback exactly. This is the equivalence the wave-3 promotion rests on
/// (on a GPU canvas it removes one flush + CPU sync per frosted node).
#[test]
fn backdrop_layer_frost_is_byte_identical_to_the_readback_at_top_level() {
    for zoom in [0.37, 0.5, 1.0, 2.0] {
        let readback = frosted_checker(false, zoom, FrostWrap::None);
        let backdrop = frosted_checker(true, zoom, FrostWrap::None);
        let worst = readback
            .iter()
            .zip(&backdrop)
            .map(|(a, b)| a.abs_diff(*b))
            .max()
            .unwrap();
        assert!(
            worst <= 1,
            "zoom {zoom}: backdrop layer vs readback must agree within 1 LSB, worst {worst}"
        );
        // And both actually frost (guards against two identically-broken no-ops).
        let plain = centre_variance(&frosted_checker(true, zoom, FrostWrap::None)); // same
        assert!(
            plain < 200.0,
            "zoom {zoom}: the panel centre must be frosted flat, var {plain}"
        );
    }
}

/// Inside a MASKED group — a frosted caption bar over a mask-clipped photo,
/// the common nesting — the backdrop layer keeps the photo under the panel and
/// frosts it (the mask's content layer holds the photo; the blurred copy
/// composites over it, so the light cells fall from 255 toward the frosted
/// gray while the dark cells stay), whereas the root-device readback cannot
/// see the photo at all: it paints the frosted PAGE (plain white here) over
/// the layer and erases the photo under the panel. Pins the fidelity gain
/// that motivates the default. (The reverse trade-off — a frosted panel
/// inside a TRANSLUCENT group frosts the page below only under the readback —
/// is the documented cost; masks and shadowed cards are the frequent case.)
#[test]
fn backdrop_layer_keeps_and_frosts_content_inside_a_masked_group() {
    let readback = frosted_checker(false, 1.0, FrostWrap::MaskedGroup);
    let backdrop = frosted_checker(true, 1.0, FrostWrap::MaskedGroup);
    let centre = |buf: &[u8]| -> Vec<u8> {
        let mut v = Vec::new();
        for y in 26..38 {
            for x in 26..38 {
                v.push(rgba_at(buf, 64, x, y)[0]);
            }
        }
        v
    };
    let (rb, bd) = (centre(&readback), centre(&backdrop));
    assert!(
        rb.iter().all(|&v| v >= 250),
        "readback erases the masked photo under the panel (expected all white): {rb:?}"
    );
    assert!(
        bd.iter().any(|&v| v <= 60),
        "backdrop layer keeps the photo's dark cells visible under the panel: {bd:?}"
    );
    assert!(
        bd.iter().all(|&v| v <= 200),
        "backdrop layer frosts the photo's light cells (none stays near white): {bd:?}"
    );
}

/// A recording canvas (PDF) cannot read pixels back; the backdrop layer must
/// simply not frost there rather than fail — same outcome the readback had.
#[test]
fn background_blur_on_a_pdf_canvas_does_not_panic() {
    let mut doc = Doc::new();
    let mut panel = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        -12.0,
        -12.0,
        24.0,
        24.0,
        Color::rgba(255, 255, 255, 30),
    )));
    panel.blurs.push(Blur::background(10.0));
    doc.apply(Operation::create_node(panel)).unwrap();
    let mut r = RasterRenderer::new(64, 64).unwrap();
    let mut pdf = Vec::new();
    {
        let document = skia_safe::pdf::new_document(&mut pdf, None);
        let mut page = document.begin_page((64.0, 64.0), None);
        r.render_to_canvas(
            page.canvas(),
            64,
            64,
            &doc.scene,
            &doc.viewport,
            None,
            &RenderInputs::empty(),
        );
        page.end_page().close();
    }
    assert!(!pdf.is_empty());
}
