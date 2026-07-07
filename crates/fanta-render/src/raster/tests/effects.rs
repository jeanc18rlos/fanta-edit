//! Node-level effect tests: drop/inner shadows, layer + background blur, blend modes, radial gradient, opacity/visibility, PNG encode, and display-scale.
use super::*;
use fanta_doc::{Doc, IndexKey, Operation, VectorNode};

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
        gradient: Gradient::Radial {
            center: [0.5, 0.5],
            radius: 0.5,
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
