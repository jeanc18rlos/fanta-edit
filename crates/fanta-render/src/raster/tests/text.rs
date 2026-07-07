//! Text glyph-rendering tests (real glyphs, color, auto-width, vertical align, repeated frames).
use super::*;
use fanta_doc::{Doc, Operation};

// -----------------------------------------------------------------------
// Text glyph rendering
// -----------------------------------------------------------------------

/// A text node whose local top-left sits at world `(x, y)` (so its glyphs
/// land on-screen near the surface centre), styled opaque black at a size
/// big enough to light up several pixels.
fn text_doc(content: &str, x: f64, y: f64) -> Doc {
    use fanta_doc::TextNode;
    let mut node = CanvasNode::new(NodeData::Text(TextNode::new(content, 200.0, 80.0)));
    node.transform = Transform2D::translation(x, y);
    let mut doc = Doc::new();
    doc.apply(Operation::create_node(node)).unwrap();
    doc
}

#[test]
fn text_node_draws_real_glyphs() {
    // Place the box top-left up-and-left of the origin so the (top-left
    // anchored) glyphs paint across the surface centre rather than off the
    // bottom-right edge.
    let doc = text_doc("Hello", -40.0, -10.0);
    let mut r = RasterRenderer::new(128, 64).unwrap();
    let metrics = r.render(&doc.scene, &doc.viewport);

    assert!(metrics.nodes_drawn >= 1, "the text node counts as drawn");
    let buf = r.copy_rgba();
    assert!(
        opaque_pixel_count(&buf) > 0,
        "glyphs must light up pixels distinct from the cleared background"
    );
}

#[test]
fn empty_text_node_draws_nothing_but_still_counts_visited() {
    // An empty-content text node paints no glyphs (nothing to shape) yet is
    // still a visited, drawn node — the walk does not skip it.
    let doc = text_doc("", -40.0, -10.0);
    let mut r = RasterRenderer::new(64, 64).unwrap();
    let metrics = r.render(&doc.scene, &doc.viewport);
    assert_eq!(metrics.nodes_visited, 1);
    assert!(metrics.nodes_drawn >= 1);
    let buf = r.copy_rgba();
    assert!(
        buf.iter().all(|&b| b == 0),
        "empty text contributes no pixels"
    );
}

#[test]
fn text_node_glyph_color_follows_style() {
    // A red-styled text node must paint red glyphs (not the old grey-blue
    // placeholder box). We assert at least one strongly-red opaque pixel.
    use fanta_doc::TextNode;
    let mut tn = TextNode::new("ABC", 200.0, 80.0);
    tn.style.color = Color::rgb(255, 0, 0);
    tn.style.size_px = 40.0;
    let mut node = CanvasNode::new(NodeData::Text(tn));
    node.transform = Transform2D::translation(-40.0, -20.0);
    let mut doc = Doc::new();
    doc.apply(Operation::create_node(node)).unwrap();

    let mut r = RasterRenderer::new(128, 96).unwrap();
    r.render(&doc.scene, &doc.viewport);
    let buf = r.copy_rgba();
    // RGBA8888: a glyph pixel is red-dominant with low green/blue.
    let red_pixel = buf
        .chunks_exact(4)
        .any(|px| px[3] > 150 && px[0] > 150 && px[1] < 80 && px[2] < 80);
    assert!(red_pixel, "expected red glyph pixels from the styled text");
}

#[test]
fn text_node_style_runs_override_base_glyph_color() {
    use fanta_doc::{TextNode, TextStyleRun};

    let mut tn = TextNode::new("AB", 200.0, 80.0);
    tn.style.color = Color::rgb(0, 0, 0);
    tn.style.size_px = 48.0;
    let mut red = tn.style.clone();
    red.color = Color::rgb(255, 0, 0);
    let start = tn.content.find('B').unwrap();
    tn.style_runs.push(TextStyleRun {
        start,
        end: tn.content.len(),
        style: red,
    });

    let mut node = CanvasNode::new(NodeData::Text(tn));
    node.transform = Transform2D::translation(-40.0, -25.0);
    let mut doc = Doc::new();
    doc.apply(Operation::create_node(node)).unwrap();

    let mut r = RasterRenderer::new(128, 96).unwrap();
    r.render(&doc.scene, &doc.viewport);
    let buf = r.copy_rgba();
    let red_pixel = buf
        .chunks_exact(4)
        .any(|px| px[3] > 150 && px[0] > 150 && px[1] < 80 && px[2] < 80);
    let black_pixel = buf
        .chunks_exact(4)
        .any(|px| px[3] > 150 && px[0] < 80 && px[1] < 80 && px[2] < 80);

    assert!(red_pixel, "expected red glyph pixels from the style run");
    assert!(
        black_pixel,
        "expected black glyph pixels from the base style"
    );
}

#[test]
fn auto_width_text_renders_single_line_beyond_its_box_width() {
    // Regression for the Action-Bar import defect: an auto-width
    // (WIDTH_AND_HEIGHT) label whose box width is the hugged glyph width must
    // render on ONE line at its natural width — never re-wrapped to the box.
    // We give the node a deliberately *too-narrow* box (10px) but a word that
    // shapes far wider, and assert the painted glyphs extend well past 10px
    // and stay within a single line's height. A wrapping renderer would break
    // "Delete" into stacked fragments confined to ~10px and run taller.
    use fanta_doc::{TextAutoResize, TextNode};

    let render_buf = |auto: TextAutoResize, box_w: f64| -> (usize, usize) {
        let mut tn = TextNode::new("Delete", box_w, 80.0);
        tn.auto_resize = auto;
        tn.style.size_px = 24.0;
        tn.style.color = Color::rgb(0, 0, 0);
        let mut node = CanvasNode::new(NodeData::Text(tn));
        // Anchor near the left so the wide single line stays on-surface.
        node.transform = Transform2D::translation(-90.0, -10.0);
        let mut doc = Doc::new();
        doc.apply(Operation::create_node(node)).unwrap();
        let (w, h) = (220i32, 96i32);
        let mut r = RasterRenderer::new(w as u32, h as u32).unwrap();
        r.render(&doc.scene, &doc.viewport);
        let buf = r.copy_rgba();
        let row = (w as usize) * 4;
        // Width of the painted ink (max lit x − min lit x) and its lit-row span.
        let (mut min_x, mut max_x) = (w as usize, 0usize);
        let (mut min_y, mut max_y) = (h as usize, 0usize);
        for y in 0..h as usize {
            for x in 0..w as usize {
                if buf[y * row + x * 4 + 3] != 0 {
                    min_x = min_x.min(x);
                    max_x = max_x.max(x);
                    min_y = min_y.min(y);
                    max_y = max_y.max(y);
                }
            }
        }
        let ink_w = max_x.saturating_sub(min_x);
        let ink_h = max_y.saturating_sub(min_y);
        (ink_w, ink_h)
    };

    let (auto_w, auto_h) = render_buf(TextAutoResize::WidthAndHeight, 10.0);
    // Auto-width "Delete" at 24px is far wider than the 10px box and one line.
    assert!(
        auto_w > 40,
        "auto-width text should paint a wide single line, got ink width {auto_w}px"
    );
    assert!(
        auto_h < 40,
        "auto-width text should occupy one line, got ink height {auto_h}px"
    );

    // Sanity contrast: a fixed (None) box at the same narrow 10px width wraps,
    // producing taller / narrower ink. (Not asserted as the headline, but it
    // documents the behavioural split the fix introduces.)
    let (none_w, none_h) = render_buf(TextAutoResize::None, 10.0);
    assert!(
        auto_w > none_w && auto_h < none_h,
        "auto-width must be wider+shorter than the wrapped fixed box: \
             auto=({auto_w},{auto_h}) none=({none_w},{none_h})"
    );
}

#[test]
fn auto_width_center_aligned_text_draws_at_hugged_origin() {
    use fanta_doc::{TextAlign, TextAutoResize, TextNode, VAlign};

    let mut text = TextNode::new("Action", 39.0, 18.0);
    text.auto_resize = TextAutoResize::WidthAndHeight;
    text.align = TextAlign::Center;
    text.vertical_align = VAlign::Center;
    text.style.size_px = 14.0;
    text.style.weight = 600;
    text.style.color = Color::rgb(255, 255, 255);

    let mut node = CanvasNode::new(NodeData::Text(text));
    node.transform = Transform2D::translation(-20.0, -9.0);
    let mut doc = Doc::new();
    doc.apply(Operation::create_node(node)).unwrap();

    let mut renderer = RasterRenderer::new(96, 48).unwrap();
    let metrics = renderer.render(&doc.scene, &doc.viewport);
    let pixels = renderer.copy_rgba();

    assert!(metrics.nodes_drawn >= 1);
    assert!(
        opaque_pixel_count(&pixels) > 0,
        "center-aligned auto-width text must paint at its hugged box origin"
    );
}

#[test]
fn vertical_center_offsets_text_below_top_aligned() {
    // In a tall box, CENTER-aligned text must paint lower than TOP-aligned
    // text (offset down by half the box's unused height). We compare the mean
    // y of opaque glyph pixels for the two alignments in the same geometry.
    use fanta_doc::{TextNode, VAlign};
    let render_mean_y = |valign: VAlign| -> f64 {
        let mut tn = TextNode::new("Hg", 200.0, 80.0); // 80px-tall box
        tn.style.size_px = 18.0;
        tn.vertical_align = valign;
        let mut node = CanvasNode::new(NodeData::Text(tn));
        // Box top-left at world (-60, -40): box spans world y ∈ [-40, 40] →
        // screen y ∈ [8, 88] on a 96-px surface centred on the origin.
        node.transform = Transform2D::translation(-60.0, -40.0);
        let mut doc = Doc::new();
        doc.apply(Operation::create_node(node)).unwrap();
        let mut r = RasterRenderer::new(160, 96).unwrap();
        r.render(&doc.scene, &doc.viewport);
        let buf = r.copy_rgba();
        let w = r.width() as usize;
        let (mut sum, mut count) = (0.0, 0.0);
        for (idx, px) in buf.chunks_exact(4).enumerate() {
            if px[3] != 0 {
                sum += (idx / w) as f64;
                count += 1.0;
            }
        }
        assert!(count > 0.0, "glyphs must paint for {valign:?}");
        sum / count
    };
    let top_y = render_mean_y(VAlign::Top);
    let center_y = render_mean_y(VAlign::Center);
    // The unused vertical space is large (80px box vs ~20px line); centering
    // shifts the block down by roughly half that. Require a clear separation.
    assert!(
        center_y > top_y + 15.0,
        "center-aligned text must sit well below top-aligned: top={top_y:.1} center={center_y:.1}"
    );
}

#[test]
fn vertical_center_can_offset_overflowing_line_box_upward() {
    use fanta_doc::{TextNode, VAlign};

    let mut text = TextNode::new("51", 24.0, 11.0);
    text.style.size_px = 13.0;
    text.style.line_height = 1.25;
    text.vertical_align = VAlign::Center;

    assert!(
        crate::raster::text::vertical_paint_offset(&text, 16.25) < 0.0,
        "centered fixed text must center an oversized line box instead of pinning it to the top"
    );
}

#[test]
fn measure_and_draw_share_one_cache_entry_for_identical_inputs() {
    // The page-switch perf fix: the auto-layout solver's `measure_text_node`
    // and the renderer's `draw_text_node` must shape through the SAME shared
    // cache, so measuring a run pre-warms exactly the entry the render paints.
    // Identical inputs → the cache grows by 1, not 2.
    use fanta_doc::{TextAutoResize, TextNode};

    // Start from a known-empty cache (tests share a thread-local; clearing makes
    // the size deltas below exact regardless of test ordering on this thread).
    clear_layout_cache();
    assert_eq!(layout_cache_len(), 0, "cache starts empty");

    // An auto-width label — the kind the solver actually measures. Auto-width
    // shapes at an unbounded width, so the measure path and the draw path use
    // identical (wrap_width, align), giving them the same key.
    let mut tn = TextNode::new("Shared Label", 200.0, 80.0);
    tn.auto_resize = TextAutoResize::WidthAndHeight;
    tn.style.size_px = 18.0;

    // (1) MEASURE — populates the shared cache once.
    let (mw, mh) = measure_text_node(&tn);
    assert!(mw > 0.0 && mh > 0.0, "measured a non-empty box");
    assert_eq!(layout_cache_len(), 1, "measure shapes exactly one entry");

    // (2) DRAW the SAME node — must HIT the entry the measure pass created, so
    // the cache does NOT grow (it would be 2 if the two paths keyed differently).
    let mut r = RasterRenderer::new(64, 64).unwrap();
    draw_text_node(r.canvas(), &tn);
    assert_eq!(
        layout_cache_len(),
        1,
        "draw must reuse the measured entry, not shape a second one"
    );

    // Sanity: a genuinely different run keys to a fresh entry (the cache is
    // content-addressed, not collapsing everything onto one key).
    let mut tn2 = tn;
    tn2.content = "Different".to_string();
    draw_text_node(r.canvas(), &tn2);
    assert_eq!(
        layout_cache_len(),
        2,
        "a different run shapes a distinct entry"
    );
    measure_text_node(&tn2);
    assert_eq!(
        layout_cache_len(),
        2,
        "measuring the already-drawn second run also hits, no growth"
    );
}

#[test]
fn fixed_box_clips_overflowing_text_but_auto_resize_does_not() {
    // A fixed (`TextAutoResize::None`) box is a hard frame: glyphs taller than
    // `local_size[1]` are clipped at the box bottom edge, exactly like Figma's
    // fixed text box. The SAME content in auto-height (`Height`) — which wraps to
    // the identical `local_size[0]`, so it shapes the very same multi-line
    // paragraph — grows its box to fit and is therefore NOT clipped: it paints
    // below the nominal box height. This pins the resize-gated clip the fix
    // introduces (same wrap, same glyphs, clip vs no-clip).
    use fanta_doc::{TextAutoResize, TextNode};

    // Wide, multi-line content that, wrapped to ~100px, shapes FAR taller than
    // the deliberately short 20px box height.
    const CONTENT: &str = "Line one Line two Line three Line four Line five";
    const BOX_W: f64 = 100.0;
    const BOX_H: f64 = 20.0; // far shorter than the wrapped paragraph

    // Render onto a surface tall enough to hold the unclipped overflow, with the
    // box top-left at a known on-surface position so the box bottom maps to a
    // fixed screen row.
    let (surf_w, surf_h) = (160u32, 200u32);
    // Box top-left in world space; the default viewport centres world (0,0) at
    // the surface centre, so place the box so its top sits near the top.
    let box_x = -(surf_w as f64) / 2.0;
    let box_y = -(surf_h as f64) / 2.0;

    let render = |auto: TextAutoResize| -> Vec<u8> {
        let mut tn = TextNode::new(CONTENT, BOX_W, BOX_H);
        tn.auto_resize = auto;
        tn.style.size_px = 16.0;
        tn.style.color = Color::rgb(0, 0, 0);
        let mut node = CanvasNode::new(NodeData::Text(tn));
        node.transform = Transform2D::translation(box_x, box_y);
        let mut doc = Doc::new();
        doc.apply(Operation::create_node(node)).unwrap();
        let mut r = RasterRenderer::new(surf_w, surf_h).unwrap();
        r.render(&doc.scene, &doc.viewport);
        r.copy_rgba()
    };

    // The box bottom edge in screen rows: world y of the box bottom is
    // `box_y + BOX_H`, mapped by the centring viewport to screen row
    // `surf_h/2 + (box_y + BOX_H)`.
    let box_bottom_row = ((surf_h as f64) / 2.0 + box_y + BOX_H).round() as usize;
    // A comfortably-below-the-box band to inspect for overflow ink (a few rows
    // below the bottom edge, avoiding sub-pixel anti-aliasing right at the seam).
    let below_band_start = box_bottom_row + 4;

    let row = (surf_w as usize) * 4;
    let opaque_in_rows = |buf: &[u8], y0: usize, y1: usize| -> usize {
        let mut n = 0usize;
        for y in y0..y1.min(surf_h as usize) {
            for x in 0..surf_w as usize {
                if buf[y * row + x * 4 + 3] != 0 {
                    n += 1;
                }
            }
        }
        n
    };

    // Fixed box (None): there MUST be glyph ink inside the box, and NO ink in
    // the band below the box bottom — the clip shaved the overflow.
    let fixed = render(TextAutoResize::None);
    let fixed_inside = opaque_in_rows(&fixed, 0, box_bottom_row);
    let fixed_below = opaque_in_rows(&fixed, below_band_start, surf_h as usize);
    assert!(
        fixed_inside > 0,
        "fixed box must paint glyphs inside its [0,h] band"
    );
    assert_eq!(
        fixed_below, 0,
        "fixed box must clip overflow: expected no ink below the box bottom \
         (row {below_band_start}), found {fixed_below} lit pixels"
    );

    // Auto-height (`Height`) is the tight contrast: it WRAPS to the same
    // `local_size[0]` as the fixed box (so the paragraph shapes identically and
    // is multi-line tall), but grows its box vertically to fit — so it is exempt
    // from the clip and the SAME glyphs now paint well below the nominal 20px
    // box. This isolates the resize gate: same wrap, same glyphs, clip vs no-clip.
    let auto = render(TextAutoResize::Height);
    let auto_below = opaque_in_rows(&auto, below_band_start, surf_h as usize);
    assert!(
        auto_below > 0,
        "auto-resize text must NOT be clipped to the nominal box: expected ink \
         below row {below_band_start}, found none (the gate failed to exempt it)"
    );
}

#[test]
fn text_rendering_does_not_panic_on_repeated_frames() {
    // The thread-local LayoutEngine must service many draws across many
    // frames without rebuilding-induced issues; a tight loop is a cheap
    // smoke test that the cache path is reentrant and stable.
    let doc = text_doc("Frame", -40.0, -10.0);
    let mut r = RasterRenderer::new(96, 48).unwrap();
    for _ in 0..8 {
        let m = r.render(&doc.scene, &doc.viewport);
        assert!(m.nodes_drawn >= 1);
    }
    let buf = r.copy_rgba();
    assert!(
        opaque_pixel_count(&buf) > 0,
        "glyphs still draw after reuse"
    );
}
