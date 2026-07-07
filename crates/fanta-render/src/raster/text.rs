//! Text rendering: the per-thread [`LayoutEngine`] cache and the
//! [`draw_text_node`] glyph shaping/painting + doc→engine style/align mapping.
use super::{
    Align, Canvas, CanvasNode, LayoutEngine, Rect, RefCell, TextAlign, TextAutoResize, TextBuffer,
    TextNode, TextStyle, Transform2D, VAlign,
};
use fanta_text::TextLayout;
use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

// ---------------------------------------------------------------------------
// Text rendering
// ---------------------------------------------------------------------------

thread_local! {
    /// One [`LayoutEngine`] per render thread, built lazily on first text draw.
    ///
    /// The engine owns a Skia `FontCollection` + system `FontMgr`; building
    /// those scans the installed fonts and is the dominant per-text cost — a
    /// document with thousands of `TextNode`s would thrash if it rebuilt them on
    /// every glyph draw. Mirrors the `FONT_CTX` thread-local in
    /// `fanta-app`'s `overlays.rs`: construct once, reuse for every node. The
    /// engine is `Clone` (its collection is reference-counted), but we never
    /// even need to clone it — `layout_aligned` borrows `&self`, so a single
    /// shared instance services the whole frame. `RefCell` because the engine is
    /// stored behind a thread-local and accessed by shared reference.
    static LAYOUT_ENGINE: RefCell<Option<LayoutEngine>> = const { RefCell::new(None) };
}

/// Run `f` with a reference to this thread's cached [`LayoutEngine`], building
/// it on first use.
pub(crate) fn with_layout_engine<R>(f: impl FnOnce(&LayoutEngine) -> R) -> R {
    LAYOUT_ENGINE.with(|cell| {
        {
            let mut slot = cell.borrow_mut();
            if slot.is_none() {
                *slot = Some(LayoutEngine::new());
            }
        }
        let slot = cell.borrow();
        f(slot.as_ref().expect("just populated"))
    })
}

/// Soft cap on the shaped-layout cache. A fit-zoom design-system page has a few
/// thousand distinct (content+style+width) text runs; past this we clear
/// wholesale (cheap, rare) so a long editing session can't grow it unbounded.
const LAYOUT_CACHE_CAP: usize = 16384;

thread_local! {
    /// Per-thread cache of shaped [`TextLayout`]s, keyed by a hash of the
    /// inputs that determine SHAPING (content + resolved style + wrap width +
    /// horizontal align). Building a Skia paragraph is the dominant per-frame
    /// cost — a fit-zoom view draws thousands of text nodes, and re-shaping each
    /// one EVERY frame took ~seconds (0.2 fps). The scene is static between
    /// edits, so identical runs re-shape identically frame after frame; caching
    /// the shaped result turns steady-state frames into pure paint. It is
    /// content-addressed, so an edit changing text/style/width keys to a fresh
    /// entry — never a stale paint. Vertical align + box height affect only the
    /// paint offset (computed per call), so they are NOT part of the key.
    ///
    /// This cache is **shared between measure and draw** ([`with_shaped_layout`]):
    /// the auto-layout solver measures a page's text through `measure_text_node`,
    /// which PRE-WARMS the very entries the subsequent render paints — so a
    /// first-visit page switch shapes each run ONCE (in the measure pass) instead
    /// of twice (measure + cold draw). The key is computed identically on both
    /// paths so they genuinely share entries.
    static LAYOUT_CACHE: RefCell<HashMap<u64, TextLayout>> = RefCell::new(HashMap::new());
}

/// Compute the `(wrap_width, align)` a [`TextNode`] shapes at. Shared by measure
/// and draw so both produce — and key — the identical shaped layout.
///
/// **Wrapping follows the node's [`TextAutoResize`]**, matching Figma's text
/// resize behaviour and the Stage-2 solver's sizing:
/// - [`WidthAndHeight`](TextAutoResize::WidthAndHeight) ("Auto width") — the box
///   hugs its glyphs and lines *never* wrap. We lay out at an effectively
///   unbounded width so a label like "Edit"/"Copy"/"224 selected" stays on ONE
///   line regardless of the (hugged) box width. Re-using the box width here would
///   risk a sub-pixel spill onto a second line, which is exactly the import
///   defect this guards against.
/// - [`Height`](TextAutoResize::Height) ("Auto height") and [`None`](TextAutoResize::None)
///   (fixed box) — the shaper wraps to `node.local_size[0]` (the box width).
fn shape_params(node: &TextNode) -> (f64, Align) {
    let (wrap_width, align) = match node.auto_resize {
        TextAutoResize::WidthAndHeight => (f64::INFINITY, Align::Left),
        TextAutoResize::Height | TextAutoResize::None => (node.local_size[0], to_align(node.align)),
    };
    (wrap_width, align)
}

/// Hash of everything that determines a [`TextNode`]'s SHAPING: content +
/// resolved style + wrap width + horizontal align. Vertical align + box height
/// only move the paint origin (computed per call), so they are deliberately
/// excluded — one shaped layout serves every v-align. Computed identically for
/// measure and draw so the two share cache entries exactly.
fn shape_key(node: &TextNode, wrap_width: f64, align: Align) -> u64 {
    let mut h = DefaultHasher::new();
    node.content.hash(&mut h);
    hash_doc_text_style(&node.style, &mut h);
    node.style_runs.len().hash(&mut h);
    for run in &node.style_runs {
        run.start.hash(&mut h);
        run.end.hash(&mut h);
        hash_doc_text_style(&run.style, &mut h);
    }
    wrap_width.to_bits().hash(&mut h);
    std::mem::discriminant(&align).hash(&mut h);
    h.finish()
}

fn hash_doc_text_style(s: &fanta_doc::TextStyle, h: &mut DefaultHasher) {
    s.font_family.hash(h);
    s.size_px.to_bits().hash(h);
    s.weight.hash(h);
    s.italic.hash(h);
    s.underline.hash(h);
    s.strikethrough.hash(h);
    s.color.hash(h);
    s.letter_spacing.to_bits().hash(h);
    s.line_height.to_bits().hash(h);
}

/// Run `f` with this thread's cached shaped [`TextLayout`] for `node`, shaping
/// on a miss and reusing it on every subsequent call — the single get-or-shape
/// path both [`measure_text_node`](crate::raster::instance::measure_text_node)
/// and [`draw_text_node`] go through. Because the key + shaping parameters are
/// computed by [`shape_key`]/[`shape_params`] for both callers, the solver's
/// measure pass populates exactly the entries the render then paints: a
/// first-visit page switch shapes each run ONCE, not twice.
///
/// `node.content` is assumed non-empty (callers special-case the empty string).
/// Keeps the existing soft cap: past [`LAYOUT_CACHE_CAP`] distinct runs we clear
/// the whole cache (cheap, rare) so a long session can't grow it unbounded.
pub(crate) fn with_shaped_layout<R>(node: &TextNode, f: impl FnOnce(&TextLayout) -> R) -> R {
    let (wrap_width, align) = shape_params(node);
    let key = shape_key(node, wrap_width, align);
    LAYOUT_CACHE.with(|c| {
        let mut cache = c.borrow_mut();
        if cache.len() >= LAYOUT_CACHE_CAP && !cache.contains_key(&key) {
            cache.clear();
        }
        // Shape on a miss (the expensive path); reuse the shaped paragraph on
        // every subsequent measure/draw of the same run.
        let layout = cache.entry(key).or_insert_with(|| {
            let style = to_text_style(&node.style);
            let mut buffer = TextBuffer::from_str(node.content.as_str(), style);
            for run in &node.style_runs {
                let style = to_text_style(&run.style);
                let _ = buffer.set_style(run.start..run.end, style);
            }
            with_layout_engine(|engine| engine.layout_aligned(&buffer, wrap_width, align))
        });
        f(layout)
    })
}

pub(crate) fn vertical_paint_offset(node: &TextNode, layout_height: f64) -> f64 {
    let factor = match node.vertical_align {
        VAlign::Top => 0.0,
        VAlign::Center => 0.5,
        VAlign::Bottom => 1.0,
    };
    (node.local_size[1] - layout_height) * factor
}

/// Test-only access to the shared shaped-text cache size, used to prove that
/// measure and draw collapse onto ONE entry for identical inputs.
#[cfg(test)]
pub(crate) fn layout_cache_len() -> usize {
    LAYOUT_CACHE.with(|c| c.borrow().len())
}

// ---------------------------------------------------------------------------
// Public text-geometry queries for the on-canvas text editor (fanta-app). They
// reuse the same shared shaped-layout cache as measure/draw, so a caret query
// during editing rides the layout already shaped for the frame's paint.
// All coordinates are paragraph-local (node-local) pixels; the caller maps to
// world/screen via the node transform + viewport.
// ---------------------------------------------------------------------------

/// Caret rect `[x, y, w, h]` for byte offset `byte` in `node`'s text. Empty
/// content yields a zero rect (the editor draws a default-height caret).
pub fn text_caret_rect(node: &TextNode, byte: usize) -> [f64; 4] {
    if node.content.is_empty() {
        return [0.0, 0.0, 0.0, 0.0];
    }
    with_shaped_layout(node, |l| l.caret_rect(byte))
}

/// Byte offset nearest `point` (node-local) — turns a click into a caret.
pub fn text_hit_test(node: &TextNode, point: [f64; 2]) -> usize {
    if node.content.is_empty() {
        return 0;
    }
    with_shaped_layout(node, |l| l.hit_test(point))
}

/// Selection-highlight rects `[x, y, w, h]` (node-local) for `start..end`.
pub fn text_selection_rects(node: &TextNode, start: usize, end: usize) -> Vec<[f64; 4]> {
    if node.content.is_empty() {
        return Vec::new();
    }
    with_shaped_layout(node, |l| l.selection_rects(start, end))
}

/// The first line's height — a sensible caret height, including for empty text
/// (where layout reports nothing), derived from the style size as a fallback.
pub fn text_line_height(node: &TextNode) -> f64 {
    let fallback = node.style.size_px * 1.2;
    if node.content.is_empty() {
        return fallback;
    }
    with_shaped_layout(node, |l| {
        l.lines().first().map(|m| m.height).unwrap_or(fallback)
    })
}

/// The glyph outlines of `node`'s text as a single vector path in node-local
/// pixels — the "convert text to path / outline text" primitive. Shapes a fresh
/// layout (the shared render cache is immutable; Skia builds glyph paths via a
/// `&mut` paragraph) with the node's exact wrap width + alignment, then unions
/// every line's glyph contours, offset by the same vertical-alignment paint
/// origin [`draw_text_node`] uses — so the resulting path renders pixel-identical
/// to the text it replaces. `None` for empty content / no geometry.
pub fn text_node_outline(node: &TextNode) -> Option<fanta_doc::PathData> {
    if node.content.is_empty() {
        return None;
    }
    let (wrap_width, align) = shape_params(node);
    let mut buffer = TextBuffer::from_str(node.content.as_str(), to_text_style(&node.style));
    for run in &node.style_runs {
        let _ = buffer.set_style(run.start..run.end, to_text_style(&run.style));
    }
    let mut layout = with_layout_engine(|engine| engine.layout_aligned(&buffer, wrap_width, align));
    // Match `draw_text_node`'s vertical-align paint origin so the outline sits
    // exactly where the glyphs were drawn.
    let dy = vertical_paint_offset(node, layout.height());
    layout.outline([0.0, dy])
}

/// Test-only reset of the shared shaped-text cache, so a test can assert exact
/// post-shape sizes from a known-empty start independent of other tests on the
/// same thread.
#[cfg(test)]
pub(crate) fn clear_layout_cache() {
    LAYOUT_CACHE.with(|c| c.borrow_mut().clear());
}

/// Shape and paint a [`TextNode`] at the canvas's current local origin (0, 0).
///
/// Shaping goes through the shared [`with_shaped_layout`] cache (see its docs
/// for the measure/draw unification and the wrap-width/auto-resize rules), so
/// the paragraph is reused frame-to-frame and, on a first-visit page switch, was
/// already shaped by the solver's measure pass — this call is then pure paint.
///
/// The box height (`local_size[1]`) governs bounds/positioning, not shaping.
///
/// **Clipping follows the node's [`TextAutoResize`]**, matching Figma and the
/// doc-model contract (`local_size[1]` is documented as a clipping bound for the
/// fixed box — see [`TextNode::local_size`](fanta_doc::TextNode::local_size)):
/// - [`None`](TextAutoResize::None) (the fixed box) — the box is a hard frame,
///   so we clip the glyphs to `[0, 0, w, h]`. Centered/bottom v-aligned text
///   can have a line box taller than Figma's authored text frame; its paragraph
///   origin is still centered/bottom-aligned in that frame, so fixed controls do
///   not pin labels to their top edge.
/// - [`Height`](TextAutoResize::Height) (auto-height) and
///   [`WidthAndHeight`](TextAutoResize::WidthAndHeight) (auto-width) — these
///   modes grow the box to fit the glyphs, so there is by definition no overflow;
///   clipping would be a no-op at best and a sub-pixel glyph-edge shaver at worst,
///   so they stay UNCLIPPED. This keeps the common label path byte-identical.
///
/// Per-run rich styling is shaped and painted through the shared paragraph
/// builder, so mixed colors, weights, sizes, and decorations in one Figma text
/// node survive import. Empty content paints nothing.
pub(crate) fn draw_text_node(canvas: &Canvas, node: &TextNode) {
    if node.content.is_empty() {
        return;
    }
    // The shape-and-paint step, factored once so the clipped (fixed-box) and
    // unclipped (auto-resize / zero-size) paths share identical paint logic and
    // cannot drift. The canvas transform already places the box; we offset
    // within it for vertical alignment.
    let paint = || {
        with_shaped_layout(node, |layout| {
            // Vertical alignment: offset the paint origin so the measured
            // paragraph block sits Top (0), Center (½), or Bottom (1) within the
            // box height. Negative slack is intentional: Figma centers/bottoms
            // the line box even when it is taller than the authored text frame.
            let dy = vertical_paint_offset(node, layout.height());
            layout.paint(canvas, [0.0, dy]);
        });
    };

    let [w, h] = node.local_size;
    // Clip ONLY a fixed (non-resizing) box with a real area. The auto-resize
    // modes hug their glyphs, so clipping them is pointless; the zero-size case
    // would clip everything away, so it also takes the fast path. Anti-aliased
    // (`true`) to match the soft frame-clip edges in `content.rs`.
    if node.auto_resize == TextAutoResize::None && w > 0.0 && h > 0.0 {
        canvas.save();
        canvas.clip_rect(Rect::from_xywh(0.0, 0.0, w as f32, h as f32), None, true);
        paint();
        canvas.restore();
    } else {
        // Fast path: no save/clip, zero added cost, behavior unchanged.
        paint();
    }
}

/// Convert the doc model's [`fanta_doc::TextStyle`] into the text engine's
/// [`fanta_text::TextStyle`]. The two are field-identical on purpose (see the
/// doc-model type's docs), so this is a plain struct literal — `color` is the
/// same `fanta_doc::Color` in both, carried straight through.
pub(crate) fn to_text_style(s: &fanta_doc::TextStyle) -> TextStyle {
    TextStyle {
        font_family: s.font_family.clone(),
        size_px: s.size_px,
        weight: s.weight,
        italic: s.italic,
        underline: s.underline,
        strikethrough: s.strikethrough,
        color: s.color,
        letter_spacing: s.letter_spacing,
        line_height: s.line_height,
    }
}

/// Map the doc model's [`TextAlign`] to the text engine's [`Align`].
pub(crate) fn to_align(align: TextAlign) -> Align {
    match align {
        TextAlign::Left => Align::Left,
        TextAlign::Center => Align::Center,
        TextAlign::Right => Align::Right,
        TextAlign::Justify => Align::Justify,
    }
}

// Reference-only `use` so the symbol exists in scope for doc links.
#[allow(dead_code)]
fn _doc_refs(_: Transform2D, _: &CanvasNode) {}
