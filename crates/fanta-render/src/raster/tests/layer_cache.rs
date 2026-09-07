//! Fidelity tests for the cross-frame effect-layer cache (`raster::layer_cache`):
//! a cached layer must composite exactly what the direct save-layer would —
//! byte for byte, on every hit — drop on any edit, and stay out of the way
//! where it cannot be exact.
use super::*;
use fanta_doc::{Doc, GroupNode, IndexKey, Operation, UnitInterval, VectorNode};

const W: u32 = 256;
const H: u32 = 192;

fn vp(cx: f64, cy: f64) -> Viewport {
    Viewport {
        center: [cx, cy],
        zoom: 1.0,
    }
}

fn rect(x: f64, y: f64, w: f64, h: f64, color: Color) -> CanvasNode {
    CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(x, y, w, h, color)))
}

fn shadow(offset: [f64; 2], blur: f64) -> Shadow {
    Shadow {
        kind: ShadowKind::Drop,
        color: Color::rgba(0, 0, 0, 200),
        blur,
        spread: 0.0,
        offset,
        show_behind_node: false,
    }
}

/// Every layer kind the walk composites through an effects save-layer, in
/// one scene: a layer-blurred leaf, a drop-shadowed leaf, a Multiply-blend
/// leaf, a translucent GROUP of two overlapping children (a non-foldable
/// opacity layer), and a blurred group whose child carries its own shadow
/// layer (nested layers),
/// all over an opaque backdrop so blend and alpha have something to meet.
/// Returns the doc and the shadowed leaf's id (edited by one test).
fn sampler_doc() -> (Doc, NodeId) {
    let mut doc = Doc::new();
    let mut key = IndexKey::FIRST;
    let mut next = || {
        key = IndexKey::after(key);
        key
    };
    let mut backdrop = rect(-70.0, -50.0, 140.0, 100.0, Color::rgb(60, 90, 200));
    backdrop.index = next();
    doc.apply(Operation::create_node(backdrop)).unwrap();

    let mut blurred = rect(-55.0, -35.0, 30.0, 20.0, Color::rgb(255, 40, 40));
    blurred.blurs.push(Blur::layer(6.0)); // σ = 3 device px at zoom 1
    blurred.index = next();
    doc.apply(Operation::create_node(blurred)).unwrap();

    let mut shadowed = rect(-15.0, -35.0, 30.0, 20.0, Color::rgb(40, 200, 80));
    shadowed.effects.push(shadow([4.0, 4.0], 6.0));
    shadowed.index = next();
    let shadowed_id = shadowed.id;
    doc.apply(Operation::create_node(shadowed)).unwrap();

    let mut multiply = rect(25.0, -35.0, 30.0, 20.0, Color::rgb(250, 230, 60));
    multiply.blend_mode = BlendMode::Multiply;
    multiply.index = next();
    doc.apply(Operation::create_node(multiply)).unwrap();

    let mut group = CanvasNode::new(NodeData::Group(GroupNode::default()));
    group.opacity = UnitInterval::new(0.5);
    group.index = next();
    let group_id = group.id;
    doc.apply(Operation::create_node(group)).unwrap();
    for (i, (x, y, c)) in [
        (-55.0, 5.0, Color::rgb(220, 40, 220)),
        (-45.0, 12.0, Color::rgb(40, 220, 220)),
    ]
    .into_iter()
    .enumerate()
    {
        let mut child = rect(x, y, 30.0, 20.0, c);
        child.parent = Some(group_id);
        child.index = if i == 0 {
            IndexKey::FIRST
        } else {
            IndexKey::after(IndexKey::FIRST)
        };
        doc.apply(Operation::create_node(child)).unwrap();
    }

    let mut blurred_group = CanvasNode::new(NodeData::Group(GroupNode::default()));
    blurred_group.blurs.push(Blur::layer(4.0)); // σ = 2 device px
    blurred_group.index = next();
    let bg_id = blurred_group.id;
    doc.apply(Operation::create_node(blurred_group)).unwrap();
    let mut inner = rect(10.0, 5.0, 30.0, 20.0, Color::rgb(255, 150, 40));
    inner.effects.push(shadow([3.0, 3.0], 4.0));
    inner.parent = Some(bg_id);
    inner.index = IndexKey::FIRST;
    doc.apply(Operation::create_node(inner)).unwrap();
    let mut inner2 = rect(20.0, 15.0, 30.0, 20.0, Color::rgb(40, 40, 40));
    inner2.opacity = UnitInterval::new(0.6);
    inner2.parent = Some(bg_id);
    inner2.index = IndexKey::after(IndexKey::FIRST);
    doc.apply(Operation::create_node(inner2)).unwrap();

    (doc, shadowed_id)
}

/// A one-shot render with a fresh renderer: the direct save-layer path (a
/// fresh renderer's first frame never populates or hits the cache).
fn fresh(doc: &Doc, viewport: &Viewport) -> (Vec<u8>, RenderMetrics) {
    let mut r = RasterRenderer::new(W, H).unwrap();
    let m = r.render(&doc.scene, viewport);
    assert_eq!((m.layer_cache_hits, m.layer_cache_misses), (0, 0));
    (r.copy_rgba(), m)
}

/// Largest per-channel difference between two straight-RGBA buffers, compared
/// PREMULTIPLIED (what the surface actually holds): a straight-alpha channel
/// under a near-zero alpha is noise the un-premultiply amplifies, not pixels.
fn max_diff(a: &[u8], b: &[u8]) -> u8 {
    assert_eq!(a.len(), b.len());
    let premul = |px: &[u8]| -> [u8; 4] {
        let a = px[3] as u32;
        [
            ((px[0] as u32 * a + 127) / 255) as u8,
            ((px[1] as u32 * a + 127) / 255) as u8,
            ((px[2] as u32 * a + 127) / 255) as u8,
            px[3],
        ]
    };
    a.chunks_exact(4)
        .zip(b.chunks_exact(4))
        .map(|(x, y)| {
            let (x, y) = (premul(x), premul(y));
            (0..4).map(|c| x[c].abs_diff(y[c])).max().unwrap()
        })
        .max()
        .unwrap_or(0)
}

/// POPULATE IS EXACT: the frame that renders every layer through the cache's
/// offscreen path (the second frame at a zoom) composites byte for byte what
/// the direct save-layer path composites — the two halves of Skia's restore
/// (filter, then alpha + blend) reproduce the whole.
#[test]
fn populating_frame_matches_the_direct_render_byte_for_byte() {
    let (doc, _) = sampler_doc();
    let (direct, dm) = fresh(&doc, &vp(0.0, 0.0));
    assert!(dm.effect_layers >= 5, "sampler must exercise ≥5 layers");

    let mut r = RasterRenderer::new(W, H).unwrap();
    let first = r.render(&doc.scene, &vp(0.0, 0.0));
    assert_eq!(
        first.layer_cache_misses, 0,
        "first frame at a zoom renders directly"
    );
    let second = r.render(&doc.scene, &vp(0.0, 0.0));
    assert!(
        second.layer_cache_misses >= 5,
        "second frame populates every eligible layer, got {second:?}"
    );
    assert_eq!(second.layer_cache_hits, 0);
    assert_eq!(
        r.copy_rgba(),
        direct,
        "offscreen-populated frame must equal the direct render"
    );
    let (n, bytes) = r.layer_cache_stats();
    assert!(n >= 5 && bytes > 0, "entries stored: {n} / {bytes} B");
}

/// INTEGER PAN IS EXACT: after populating, a pan by whole device pixels is
/// served from the cache (no save-layers pushed, subtrees not walked) and is
/// pixel-identical to a fresh direct render of the panned viewport.
#[test]
fn integer_pan_hits_are_pixel_identical_to_a_fresh_render() {
    let (doc, _) = sampler_doc();
    let mut r = RasterRenderer::new(W, H).unwrap();
    r.render(&doc.scene, &vp(0.0, 0.0));
    // The populating frame walks every node (it renders every layer's body).
    let visited_full = r.render(&doc.scene, &vp(0.0, 0.0)).nodes_visited;

    for (dx, dy) in [(7.0, -3.0), (-11.0, 5.0), (40.0, 30.0)] {
        let panned = vp(dx, dy);
        let m = r.render(&doc.scene, &panned);
        let (reference, _) = fresh(&doc, &panned);
        assert!(
            m.layer_cache_hits >= 5,
            "pan ({dx},{dy}) should hit, got {m:?}"
        );
        assert_eq!(
            m.layer_cache_misses, 0,
            "nothing to re-render on an integer pan"
        );
        // Hits skip the cached subtree: the nested blurred group's children
        // are never visited.
        assert!(
            m.nodes_visited < visited_full,
            "hits must not walk cached subtrees"
        );
        assert_eq!(
            max_diff(&r.copy_rgba(), &reference),
            0,
            "integer pan ({dx},{dy}) must be byte-identical to a fresh render"
        );
    }
}

/// FRACTIONAL PAN: a pan that is not a whole number of device pixels cannot
/// be served from an integer-placed image, so nothing hits — and, because
/// such a frame could not be hit later either, nothing is populated: every
/// layer renders directly, the frame is exact, and the cache costs nothing.
/// (`set_pixel_snap_pan` is how a host turns these into hits.) A whole-pixel
/// pan from there populates again, and the next one hits.
#[test]
fn fractional_pan_neither_hits_nor_populates_and_stays_exact() {
    let (doc, _) = sampler_doc();
    let mut r = RasterRenderer::new(W, H).unwrap();
    r.render(&doc.scene, &vp(0.0, 0.0));
    r.render(&doc.scene, &vp(0.0, 0.0));
    for (dx, dy) in [(0.5, 0.25), (3.3, -1.7), (0.0, 0.5)] {
        let panned = vp(dx, dy);
        let m = r.render(&doc.scene, &panned);
        let (reference, _) = fresh(&doc, &panned);
        assert_eq!(
            (m.layer_cache_hits, m.layer_cache_misses),
            (0, 0),
            "fractional pan must neither hit nor populate: {m:?}"
        );
        assert!(m.effect_layers >= 5, "every layer renders directly: {m:?}");
        assert_eq!(max_diff(&r.copy_rgba(), &reference), 0);
    }
    // Whole-pixel pans from the last phase: populate, then hit.
    let m = r.render(&doc.scene, &vp(1.0, 0.5));
    assert!(m.layer_cache_misses >= 5, "{m:?}");
    let m = r.render(&doc.scene, &vp(4.0, -2.5));
    assert!(m.layer_cache_hits >= 5, "{m:?}");
    assert_eq!(max_diff(&r.copy_rgba(), &fresh(&doc, &vp(4.0, -2.5)).0), 0);
}

/// ANY EDIT DROPS THE CACHE: mutating a node inside a cached layer (a fill
/// color, and separately a transform-only move — which does NOT stamp the
/// node) makes the next frame render the change; nothing stale survives.
#[test]
fn any_scene_edit_invalidates_every_cached_layer() {
    let (mut doc, shadowed_id) = sampler_doc();
    let mut r = RasterRenderer::new(W, H).unwrap();
    r.render(&doc.scene, &vp(0.0, 0.0));
    r.render(&doc.scene, &vp(0.0, 0.0));
    assert!(r.render(&doc.scene, &vp(0.0, 0.0)).layer_cache_hits >= 5);

    // Fill edit.
    if let NodeData::Vector(v) = &mut doc.scene.get_mut(shadowed_id).unwrap().data {
        v.fills[0] = Fill::solid(Color::rgb(10, 10, 250));
    }
    let m = r.render(&doc.scene, &vp(0.0, 0.0));
    assert_eq!(m.layer_cache_hits, 0, "edit must drop the cache: {m:?}");
    assert_eq!(r.copy_rgba(), fresh(&doc, &vp(0.0, 0.0)).0);

    // Transform-only edit (no geometry stamp) — still invalidates.
    r.render(&doc.scene, &vp(0.0, 0.0));
    assert!(r.render(&doc.scene, &vp(0.0, 0.0)).layer_cache_hits >= 5);
    doc.scene
        .set_transform(shadowed_id, Transform2D::translation(9.0, -4.0))
        .unwrap();
    let m = r.render(&doc.scene, &vp(0.0, 0.0));
    assert_eq!(
        m.layer_cache_hits, 0,
        "transform edit must drop the cache: {m:?}"
    );
    assert_eq!(r.copy_rgba(), fresh(&doc, &vp(0.0, 0.0)).0);
}

/// CLIPPED SUBTREES: a blurred child that pokes past its clipping frame's
/// box (its content is cropped by the frame, its blur reaches back in) and a
/// sibling well inside are both cached, and both frames stay identical to the
/// direct render — before and after a pan moves the frame's clip. Skia's
/// direct save-layer already renders every pixel that can influence the
/// clipped output and clips only the result, which is exactly what a
/// complete cached rendering composited under the clip does.
#[test]
fn layers_crossing_the_clip_are_cached_and_stay_exact() {
    let mut doc = Doc::new();
    let mut frame = CanvasNode::new(NodeData::Group(GroupNode {
        clip_size: Some([160.0, 120.0]),
        corner_radius: Some(24.0),
        background: Some(Fill::solid(Color::rgb(230, 230, 230))),
        ..GroupNode::default()
    }));
    frame.transform = Transform2D::translation(-80.0, -60.0);
    let frame_id = frame.id;
    doc.apply(Operation::create_node(frame)).unwrap();
    // Crosses the frame's right edge (frame spans 0..160 in its local space)
    // AND its rounded bottom-right corner.
    let mut crossing = rect(140.0, 90.0, 40.0, 40.0, Color::rgb(200, 30, 30));
    crossing.blurs.push(Blur::layer(4.0));
    crossing.parent = Some(frame_id);
    crossing.index = IndexKey::FIRST;
    doc.apply(Operation::create_node(crossing)).unwrap();
    // A shadow whose reach crosses the frame's top edge.
    let mut shadowed = rect(20.0, 2.0, 30.0, 20.0, Color::rgb(30, 200, 30));
    shadowed.effects.push(shadow([-4.0, -6.0], 8.0));
    shadowed.parent = Some(frame_id);
    shadowed.index = IndexKey::after(IndexKey::FIRST);
    doc.apply(Operation::create_node(shadowed)).unwrap();
    let mut inside = rect(76.0, 56.0, 8.0, 8.0, Color::rgb(30, 30, 200));
    inside.blurs.push(Blur::layer(2.0));
    inside.parent = Some(frame_id);
    inside.index = IndexKey::after(IndexKey::after(IndexKey::FIRST));
    doc.apply(Operation::create_node(inside)).unwrap();

    let mut r = RasterRenderer::new(W, H).unwrap();
    r.render(&doc.scene, &vp(0.0, 0.0));
    let populate = r.render(&doc.scene, &vp(0.0, 0.0));
    assert_eq!(
        populate.layer_cache_misses, 3,
        "all three layers cache: {populate:?}"
    );
    assert_eq!(r.copy_rgba(), fresh(&doc, &vp(0.0, 0.0)).0);
    for (dx, dy) in [(3.0, 2.0), (-25.0, 14.0), (60.0, -30.0)] {
        let hit = r.render(&doc.scene, &vp(dx, dy));
        assert_eq!(
            (hit.layer_cache_hits, hit.layer_cache_misses),
            (3, 0),
            "{hit:?}"
        );
        assert_eq!(hit.effect_layers, 0);
        assert_eq!(
            max_diff(&r.copy_rgba(), &fresh(&doc, &vp(dx, dy)).0),
            0,
            "pan ({dx},{dy})"
        );
    }
}

/// BUDGET + SWITCH: an over-budget layer is simply not stored (the frame is
/// still exact); disabling the cache restores the direct path (no hits, no
/// misses) and drops the entries.
#[test]
fn budget_and_switch_only_ever_fall_back_to_the_direct_path() {
    let (doc, _) = sampler_doc();
    let mut r = RasterRenderer::new(W, H).unwrap();
    r.set_layer_cache_budget_bytes(64); // smaller than any layer
    r.render(&doc.scene, &vp(0.0, 0.0));
    let m = r.render(&doc.scene, &vp(0.0, 0.0));
    assert_eq!(m.layer_cache_misses, 0, "nothing fits the budget: {m:?}");
    let m = r.render(&doc.scene, &vp(4.0, 0.0));
    assert_eq!(m.layer_cache_hits, 0);
    assert_eq!(r.copy_rgba(), fresh(&doc, &vp(4.0, 0.0)).0);
    assert_eq!(r.layer_cache_stats(), (0, 0));

    let mut r = RasterRenderer::new(W, H).unwrap();
    r.render(&doc.scene, &vp(0.0, 0.0));
    r.render(&doc.scene, &vp(0.0, 0.0));
    assert!(r.layer_cache_stats().0 >= 5);
    r.set_layer_cache_enabled(false);
    assert_eq!(r.layer_cache_stats(), (0, 0), "disabling drops the entries");
    let m = r.render(&doc.scene, &vp(2.0, 0.0));
    assert_eq!((m.layer_cache_hits, m.layer_cache_misses), (0, 0));
    assert!(m.effect_layers >= 5);
    assert_eq!(r.copy_rgba(), fresh(&doc, &vp(2.0, 0.0)).0);
}

/// PIXEL-SNAP PAN: with the option on, viewports whose device translation
/// rounds to the same whole pixel render identically, and a fractional
/// trackpad-style pan therefore lands on integer device offsets — every
/// cached layer hits, exactly.
#[test]
fn pixel_snap_pan_makes_fractional_pans_exact_cache_hits() {
    let (doc, _) = sampler_doc();
    let mut a = RasterRenderer::new(W, H).unwrap();
    a.set_pixel_snap_pan(true);
    let mut b = RasterRenderer::new(W, H).unwrap();
    b.set_pixel_snap_pan(true);
    a.render(&doc.scene, &vp(0.0, 0.0));
    b.render(&doc.scene, &vp(0.3, -0.2));
    assert_eq!(
        a.copy_rgba(),
        b.copy_rgba(),
        "both snap to the same device phase"
    );
    // Off, the same two viewports differ (the sub-pixel phase is real).
    let (off_a, _) = fresh(&doc, &vp(0.0, 0.0));
    let (off_b, _) = fresh(&doc, &vp(0.3, -0.2));
    assert_ne!(off_a, off_b);

    let mut r = RasterRenderer::new(W, H).unwrap();
    r.set_pixel_snap_pan(true);
    r.render(&doc.scene, &vp(0.0, 0.0));
    r.render(&doc.scene, &vp(0.0, 0.0));
    for (dx, dy) in [(0.6, 0.0), (1.4, -2.3), (10.5, 7.5)] {
        let m = r.render(&doc.scene, &vp(dx, dy));
        assert!(
            m.layer_cache_hits >= 5,
            "snapped pan ({dx},{dy}) hits: {m:?}"
        );
        assert_eq!(m.layer_cache_misses, 0);
        let mut reference = RasterRenderer::new(W, H).unwrap();
        reference.set_pixel_snap_pan(true);
        reference.render(&doc.scene, &vp(dx, dy));
        assert_eq!(r.copy_rgba(), reference.copy_rgba());
    }
}

/// A NEW ZOOM renders directly first (no populate churn during a continuous
/// zoom), populates on the next frame at that zoom, then hits; and the
/// entries from the previous zoom are simply stale, never served.
#[test]
fn zoom_change_misses_then_repopulates_at_the_new_scale() {
    let (doc, _) = sampler_doc();
    let mut r = RasterRenderer::new(W, H).unwrap();
    r.render(&doc.scene, &vp(0.0, 0.0));
    r.render(&doc.scene, &vp(0.0, 0.0));
    let zoomed = Viewport {
        center: [0.0, 0.0],
        zoom: 1.5,
    };
    let m = r.render(&doc.scene, &zoomed);
    assert_eq!((m.layer_cache_hits, m.layer_cache_misses), (0, 0), "{m:?}");
    let m = r.render(&doc.scene, &zoomed);
    assert!(m.layer_cache_misses >= 5, "{m:?}");
    let panned = Viewport {
        center: [2.0, 0.0], // 3 device px at zoom 1.5
        zoom: 1.5,
    };
    let m = r.render(&doc.scene, &panned);
    assert!(m.layer_cache_hits >= 5, "{m:?}");
    let mut reference = RasterRenderer::new(W, H).unwrap();
    reference.render(&doc.scene, &panned);
    assert_eq!(r.copy_rgba(), reference.copy_rgba());
}

/// A layer holding VOLATILE content — an image whose asset cannot be
/// resolved (placeholder) — is never stored: were the asset to finish
/// decoding, no epoch input would move, and a cached layer would keep
/// showing the placeholder.
#[test]
fn volatile_layers_are_rendered_but_never_stored() {
    let mut doc = Doc::new();
    let mut group = CanvasNode::new(NodeData::Group(GroupNode::default()));
    group.opacity = UnitInterval::new(0.5);
    let group_id = group.id;
    doc.apply(Operation::create_node(group)).unwrap();
    let mut bitmap = CanvasNode::new(NodeData::Bitmap(fanta_doc::BitmapNode {
        asset: fanta_doc::AssetId::new(),
        natural_size: [30, 20],
        local_size: [30.0, 20.0],
        crop: None,
        fit: fanta_doc::ImageFitMode::Fill,
        tint: None,
    }));
    bitmap.parent = Some(group_id);
    bitmap.index = IndexKey::FIRST;
    doc.apply(Operation::create_node(bitmap)).unwrap();
    let mut sibling = rect(-20.0, 5.0, 30.0, 20.0, Color::rgb(0, 200, 0));
    sibling.parent = Some(group_id);
    sibling.index = IndexKey::after(IndexKey::FIRST);
    doc.apply(Operation::create_node(sibling)).unwrap();

    let mut r = RasterRenderer::new(W, H).unwrap();
    r.render(&doc.scene, &vp(0.0, 0.0));
    let m = r.render(&doc.scene, &vp(0.0, 0.0));
    assert_eq!(
        m.layer_cache_misses, 1,
        "rendered through the offscreen path: {m:?}"
    );
    assert_eq!(r.layer_cache_stats(), (0, 0), "but not stored");
    let m = r.render(&doc.scene, &vp(3.0, 0.0));
    assert_eq!(m.layer_cache_hits, 0);
    assert_eq!(r.copy_rgba(), fresh(&doc, &vp(3.0, 0.0)).0);
}
