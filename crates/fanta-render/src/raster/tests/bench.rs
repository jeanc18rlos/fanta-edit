//! Headless perf benchmark (`#[ignore]`d): drop-shadow blur cost vs zoom.
use super::*;
use fanta_doc::{Doc, Operation, VectorNode};

// -----------------------------------------------------------------------
// Drop-shadow blur cost vs zoom (perf bench, run with --ignored --nocapture)
// -----------------------------------------------------------------------

/// Headless benchmark: build a grid of `n×n` drop-shadowed rects filling the
/// world origin region, render the scene through a 1024×768 surface at a
/// sweep of zoom levels, and print the per-frame time at each zoom.
///
/// Run with: `cargo test -p fanta-render shadow_zoom_bench -- --ignored --nocapture`.
///
/// Before the sigma cap, the per-frame time grew ~quadratically with zoom
/// (the on-screen Gaussian kernel area is ∝ (sigma·scale)²). After the cap +
/// shadow-aware cull it should stay roughly flat — most rects scroll off the
/// fixed surface as zoom climbs, and the few still on-screen have a bounded
/// kernel. This test never asserts; it is a measurement harness.
#[test]
#[ignore = "perf benchmark; run explicitly with --ignored --nocapture"]
fn shadow_zoom_bench() {
    // Scenario A — a grid of small shadowed rects centred on the world
    // origin. As zoom climbs most scroll off the fixed surface (the existing
    // world-bounds cull removes them), so this reflects a typical "many
    // cards, zoom into one corner" pan.
    let grid = 16; // 256 rects
    let cell = 40.0;
    let span = grid as f64 * cell;
    let mut grid_doc = Doc::new();
    for gy in 0..grid {
        for gx in 0..grid {
            let x = -span * 0.5 + gx as f64 * cell;
            let y = -span * 0.5 + gy as f64 * cell;
            let mut n = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
                x,
                y,
                cell * 0.7,
                cell * 0.7,
                Color::rgb(220, 80, 80),
            )));
            n.effects.push(Shadow {
                kind: ShadowKind::Drop,
                color: Color::rgba(0, 0, 0, 160),
                blur: 16.0,
                spread: 2.0,
                offset: [4.0, 6.0],
            });
            grid_doc.apply(Operation::create_node(n)).unwrap();
        }
    }

    // Scenario B — a few big soft-shadowed "cards" centred on the origin that
    // STAY on-screen at every zoom. This is the realistic "zoom into a card
    // with a soft drop shadow" case and isolates the per-shadow Gaussian-
    // kernel cost: the blurred device region is the card silhouette grown by
    // the on-screen sigma, so before the cap the cost scales ∝ (sigma·zoom)²
    // — the actual "slows as you zoom in" pathology. The cap clamps the
    // on-screen sigma, flattening the curve.
    let mut onscreen_doc = Doc::new();
    for i in 0..6 {
        let off = (i as f64) * 4.0;
        let mut n = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
            -100.0 + off,
            -70.0 + off,
            200.0,
            140.0,
            Color::rgb(220, 80, 80),
        )));
        n.effects.push(Shadow {
            kind: ShadowKind::Drop,
            color: Color::rgba(0, 0, 0, 120),
            blur: 48.0, // world sigma 24 → on-screen sigma hits the 128 cap at ~5.3x
            spread: 0.0,
            offset: [0.0, 12.0],
        });
        onscreen_doc.apply(Operation::create_node(n)).unwrap();
    }

    let (w, h) = (1024u32, 768u32);
    let mut r = RasterRenderer::new(w, h).unwrap();
    let zooms = [1.0_f64, 2.0, 4.0, 8.0, 16.0];

    let bench = |r: &mut RasterRenderer, doc: &Doc, label: &str| {
        // Warm any one-time costs (thread-local layout engine, allocations).
        r.render(&doc.scene, &doc.viewport);
        eprintln!("{label} ({w}x{h} surface):");
        for &zoom in &zooms {
            let vp = Viewport {
                center: [0.0, 0.0],
                zoom,
            };
            let reps = 5; // median damps scheduler noise
            let mut times = Vec::with_capacity(reps);
            let mut last = RenderMetrics::default();
            for _ in 0..reps {
                last = r.render(&doc.scene, &vp);
                times.push(last.frame_micros);
            }
            times.sort_unstable();
            let median = times[times.len() / 2];
            eprintln!(
                "  zoom {zoom:>4}x: {median:>8} us/frame  (drawn {}, culled {})",
                last.nodes_drawn, last.nodes_culled
            );
        }
    };

    bench(&mut r, &grid_doc, "A: 256 shadowed rects (grid)");
    bench(
        &mut r,
        &onscreen_doc,
        "B: 6 soft-shadowed cards (stay on-screen)",
    );
}
