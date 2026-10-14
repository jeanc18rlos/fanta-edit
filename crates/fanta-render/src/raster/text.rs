//! Text rendering: the per-thread [`LayoutEngine`] cache and the
//! [`draw_text_node`] glyph shaping/painting + doc→engine style/align mapping.
use super::{
    Align, Canvas, CanvasNode, LayoutEngine, Rect, RefCell, TextAlign, TextAutoResize, TextBuffer,
    TextNode, TextStyle, Transform2D, VAlign,
};
use fanta_text::{LayoutOptions, TextLayout};
use lru::LruCache;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::num::NonZeroUsize;

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

/// Keep recently measured or painted runs warm when new text exceeds the cap.
const LAYOUT_CACHE_CAP: NonZeroUsize = NonZeroUsize::MIN.saturating_add(16_383);
const TEXT_GEOMETRY_CACHE_CAP: NonZeroUsize = NonZeroUsize::MIN.saturating_add(4_095);
const TEXT_GEOMETRY_CACHE_BYTES: usize = 32 * 1024 * 1024;

struct CachedTextLayout {
    node: TextNode,
    layout: TextLayout,
}

struct TextGeometry {
    outline: Option<Option<fanta_doc::PathData>>,
    paint_bounds: Result<Option<fanta_doc::Bounds>, TextPaintBoundsUnavailable>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TextPaintBoundsUnavailable;

struct TextGeometryEntry {
    node: TextNode,
    font_generation: u64,
    geometry: TextGeometry,
    bytes: usize,
}

struct TextGeometryCache {
    entries: LruCache<u64, TextGeometryEntry>,
    bytes: usize,
    maximum_bytes: usize,
}

impl Default for TextGeometryCache {
    fn default() -> Self {
        Self::new(TEXT_GEOMETRY_CACHE_CAP, TEXT_GEOMETRY_CACHE_BYTES)
    }
}

impl TextGeometryCache {
    fn new(capacity: NonZeroUsize, maximum_bytes: usize) -> Self {
        Self {
            entries: LruCache::new(capacity),
            bytes: 0,
            maximum_bytes,
        }
    }

    fn get(&mut self, key: u64, node: &TextNode, font_generation: u64) -> Option<&TextGeometry> {
        self.entries.get(&key).and_then(|entry| {
            // A hash only locates a candidate; clips, styles and font changes
            // must be proven equal before reusing an authored silhouette.
            (entry.font_generation == font_generation && entry.node == *node)
                .then_some(&entry.geometry)
        })
    }

    fn insert(&mut self, key: u64, entry: TextGeometryEntry) {
        if entry.bytes > self.maximum_bytes {
            return;
        }
        if let Some(previous) = self.entries.pop(&key) {
            self.bytes = self.bytes.saturating_sub(previous.bytes);
        }
        while self.bytes.saturating_add(entry.bytes) > self.maximum_bytes {
            let Some((_, previous)) = self.entries.pop_lru() else {
                break;
            };
            self.bytes = self.bytes.saturating_sub(previous.bytes);
        }
        self.bytes = self.bytes.saturating_add(entry.bytes);
        if let Some((_, previous)) = self.entries.push(key, entry) {
            self.bytes = self.bytes.saturating_sub(previous.bytes);
        }
    }
}

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
    static LAYOUT_CACHE: RefCell<LruCache<u64, CachedTextLayout>> = RefCell::new(LruCache::new(LAYOUT_CACHE_CAP));
    static LAYOUT_FONT_GENERATION: std::cell::Cell<Option<u64>> = const { std::cell::Cell::new(None) };
    static TEXT_GEOMETRY_CACHE: RefCell<TextGeometryCache> = RefCell::new(TextGeometryCache::default());
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
    // Truncation + paragraph indent change the SHAPED result (line clamp,
    // ellipsis glyphs, first-line inset), so they key the cache too.
    // `paragraph_spacing` deliberately does NOT: it only moves paint bands
    // (like v-align), never the shaped lines.
    node.max_lines.hash(&mut h);
    node.truncate.hash(&mut h);
    node.paragraph_indent.to_bits().hash(&mut h);
    // The box height participates only when it drives the truncation clamp
    // (see `shape_layout`): truncating a fixed box with no explicit max_lines.
    if truncates_at_box_height(node) {
        node.local_size[1].to_bits().hash(&mut h);
    }
    h.finish()
}

pub(super) fn hash_doc_text_style(s: &fanta_doc::TextStyle, h: &mut DefaultHasher) {
    s.font_family.hash(h);
    s.size_px.to_bits().hash(h);
    s.weight.hash(h);
    s.italic.hash(h);
    s.underline.hash(h);
    s.strikethrough.hash(h);
    s.color.hash(h);
    s.letter_spacing.to_bits().hash(h);
    s.line_height.to_bits().hash(h);
    s.line_height_auto_percent.map(f64::to_bits).hash(h);
    // Variable-font axes change the shaped glyphs, so two runs differing only in
    // their variation coordinates must not share a cached layout.
    for variation in &s.font_variations {
        variation.axis.hash(h);
        variation.value.to_bits().hash(h);
    }
}

/// Whether `node` truncates at its BOX HEIGHT: Figma's `textTruncation:
/// ENDING` with no explicit `maxLines` clamps a fixed box to however many
/// lines fit `local_size[1]` (an auto-resize box grows instead, so only the
/// fixed box has a height to clamp to).
fn truncates_at_box_height(node: &TextNode) -> bool {
    node.truncate
        && node.max_lines.is_none()
        && node.auto_resize == TextAutoResize::None
        && node.local_size[1] > 0.0
}

/// The paragraph-level [`LayoutOptions`] a [`TextNode`] shapes with (line
/// clamp + ellipsis + first-line indent). The box-height-derived clamp is NOT
/// resolved here — it needs a probe layout, done in [`shape_layout`].
fn layout_options(node: &TextNode) -> LayoutOptions {
    LayoutOptions {
        max_lines: node.max_lines.map(|limit| (limit.max(1)) as usize),
        ellipsize: node.truncate,
        first_line_indent: node.paragraph_indent.max(0.0),
    }
}

/// Shape `node`'s content — the single shaping path behind the cache and the
/// outline query, so both produce the identical paragraph. Resolves the
/// box-height truncation clamp ([`truncates_at_box_height`]) by probing an
/// unclamped layout for how many lines fit, then re-shaping with that limit
/// (skipping the second pass when everything already fits).
fn shape_layout(node: &TextNode) -> TextLayout {
    let (wrap_width, align) = shape_params(node);
    let style = to_text_style(&node.style);
    let mut buffer = TextBuffer::from_str(node.content.as_str(), style);
    for run in &node.style_runs {
        let style = to_text_style(&run.style);
        let _ = buffer.set_style(run.start..run.end, style);
    }
    let mut options = layout_options(node);
    with_layout_engine(|engine| {
        if truncates_at_box_height(node) {
            let unclamped = engine.layout_with(
                &buffer,
                wrap_width,
                align,
                &LayoutOptions {
                    max_lines: None,
                    ellipsize: false,
                    ..options.clone()
                },
            );
            let fitting = unclamped
                .lines()
                .iter()
                // Half a pixel of slack so a line that fits up to rounding is
                // not truncated away.
                .filter(|line| line.top + line.height <= node.local_size[1] + 0.5)
                .count();
            if fitting >= unclamped.line_count() {
                return unclamped;
            }
            options.max_lines = Some(fitting.max(1));
        }
        engine.layout_with(&buffer, wrap_width, align, &options)
    })
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
/// Past [`LAYOUT_CACHE_CAP`] distinct runs, evict only the least recently used
/// layout so measuring or painting a hot run keeps it resident.
pub(crate) fn with_shaped_layout<R>(node: &TextNode, f: impl FnOnce(&TextLayout) -> R) -> R {
    let (wrap_width, align) = shape_params(node);
    let key = shape_key(node, wrap_width, align);
    let font_generation = with_layout_engine(LayoutEngine::font_generation);
    if font_generation.is_none() {
        return f(&shape_layout(node));
    }
    LAYOUT_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        LAYOUT_FONT_GENERATION.with(|previous| {
            // A registered face may change fallback metrics even for a run
            // that does not name that family, so invalidate every paragraph.
            if previous.get() != font_generation {
                cache.clear();
                previous.set(font_generation);
            }
        });
        if let Some(entry) = cache.get(&key)
            && shaping_inputs_match(&entry.node, node)
        {
            return f(&entry.layout);
        }
        let layout = shape_layout(node);
        let result = f(&layout);
        if with_layout_engine(LayoutEngine::font_generation) == font_generation {
            drop(cache.put(
                key,
                CachedTextLayout {
                    node: node.clone(),
                    layout,
                },
            ));
        }
        result
    })
}

fn shaping_inputs_match(previous: &TextNode, node: &TextNode) -> bool {
    let (previous_width, previous_align) = shape_params(previous);
    let (width, align) = shape_params(node);
    previous_width.to_bits() == width.to_bits()
        && previous_align == align
        && previous.content == node.content
        && previous.style == node.style
        && previous.style_runs == node.style_runs
        && previous.max_lines == node.max_lines
        && previous.truncate == node.truncate
        && previous.paragraph_indent.to_bits() == node.paragraph_indent.to_bits()
        && truncates_at_box_height(previous).then_some(previous.local_size[1].to_bits())
            == truncates_at_box_height(node).then_some(node.local_size[1].to_bits())
}

/// The height `node`'s glyphs actually occupy when painted: the shaped layout
/// height plus the paragraph-spacing bands [`draw_text_node`] inserts. This —
/// not the bare `layout.height()` — is what vertical alignment and the
/// auto-height measure must use so pixels and geometry agree.
pub(crate) fn painted_text_height(node: &TextNode, layout: &TextLayout) -> f64 {
    layout.spaced_height(node.paragraph_spacing.max(0.0))
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

/// First visual line baseline in node-local coordinates, including the same
/// vertical-alignment offset used by painting. Empty text falls back to the
/// font-size baseline inside its configured line box so a newly created text
/// layer still exposes a useful alignment guide.
pub fn text_first_baseline(node: &TextNode) -> f64 {
    if node.content.is_empty() {
        let line_height = text_line_height(node);
        return vertical_paint_offset(node, line_height) + node.style.size_px.min(line_height);
    }
    with_shaped_layout(node, |layout| {
        let paint_height = painted_text_height(node, layout);
        let offset = vertical_paint_offset(node, paint_height);
        layout
            .lines()
            .first()
            .map(|line| offset + line.baseline)
            .unwrap_or(offset + node.style.size_px)
    })
}

/// The glyph outlines of `node`'s text as a single vector path in node-local
/// pixels — the "convert text to path / outline text" primitive. Reuses the
/// exact glyph geometry until its text inputs or resolved fonts change.
/// `None` for empty content / no geometry.
pub fn text_node_outline(node: &TextNode) -> Option<fanta_doc::PathData> {
    with_text_geometry(node, true, |geometry| geometry.outline.clone().flatten())
}

#[cfg(test)]
pub(crate) fn text_node_ink_bounds(node: &TextNode) -> Option<fanta_doc::Bounds> {
    text_node_paint_bounds(node).expect("native paint bounds proved")
}

pub(crate) fn text_node_paint_bounds(
    node: &TextNode,
) -> Result<Option<fanta_doc::Bounds>, TextPaintBoundsUnavailable> {
    with_text_geometry(node, false, |geometry| geometry.paint_bounds)
}

fn text_geometry_key(node: &TextNode) -> u64 {
    let (width, align) = shape_params(node);
    let mut hasher = DefaultHasher::new();
    shape_key(node, width, align).hash(&mut hasher);
    node.local_size.map(f64::to_bits).hash(&mut hasher);
    std::mem::discriminant(&node.align).hash(&mut hasher);
    std::mem::discriminant(&node.vertical_align).hash(&mut hasher);
    std::mem::discriminant(&node.auto_resize).hash(&mut hasher);
    node.paragraph_spacing.to_bits().hash(&mut hasher);
    hasher.finish()
}

fn with_text_geometry<R>(
    node: &TextNode,
    include_outline: bool,
    query: impl FnOnce(&TextGeometry) -> R,
) -> R {
    if node.content.is_empty() {
        return query(&TextGeometry {
            outline: Some(None),
            paint_bounds: Ok(None),
        });
    }
    let font_generation = with_layout_engine(LayoutEngine::font_generation);
    let key = text_geometry_key(node);
    TEXT_GEOMETRY_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if let Some(font_generation) = font_generation
            && let Some(geometry) = cache.get(key, node, font_generation)
            && (!include_outline || geometry.outline.is_some())
        {
            return query(geometry);
        }
        let geometry = build_text_geometry(node, include_outline);
        let result = query(&geometry);
        // Font resolution can register a downloaded face while shaping. An
        // overlapping registration cannot certify the result's font epoch.
        if let Some(font_generation) = font_generation
            && with_layout_engine(LayoutEngine::font_generation) == Some(font_generation)
        {
            let source_bytes = text_source_heap_bytes(node);
            let geometry_bytes = text_geometry_heap_bytes(&geometry);
            if source_bytes.saturating_add(geometry_bytes) <= cache.maximum_bytes {
                let node = node.clone();
                let bytes = std::mem::size_of::<TextGeometryEntry>()
                    .saturating_add(64)
                    .saturating_add(text_source_heap_bytes(&node))
                    .saturating_add(geometry_bytes);
                cache.insert(
                    key,
                    TextGeometryEntry {
                        node,
                        font_generation,
                        geometry,
                        bytes,
                    },
                );
            }
        }
        result
    })
}

fn text_source_heap_bytes(node: &TextNode) -> usize {
    node.style_runs.iter().fold(
        node.content
            .capacity()
            .saturating_add(text_style_heap_bytes(&node.style))
            .saturating_add(
                node.style_runs
                    .capacity()
                    .saturating_mul(std::mem::size_of::<fanta_doc::TextStyleRun>()),
            ),
        |bytes, run| bytes.saturating_add(text_style_heap_bytes(&run.style)),
    )
}

pub(super) fn text_style_heap_bytes(style: &fanta_doc::TextStyle) -> usize {
    style.font_variations.iter().fold(
        style.font_family.capacity().saturating_add(
            style
                .font_variations
                .capacity()
                .saturating_mul(std::mem::size_of::<fanta_doc::FontVariation>()),
        ),
        |bytes, variation| bytes.saturating_add(variation.axis.capacity()),
    )
}

fn text_geometry_heap_bytes(geometry: &TextGeometry) -> usize {
    geometry
        .outline
        .as_ref()
        .and_then(Option::as_ref)
        .map_or(0, |outline| {
            outline
                .segments
                .capacity()
                .saturating_mul(std::mem::size_of::<fanta_doc::PathSegment>())
                .saturating_add(
                    outline
                        .subpath_rules
                        .capacity()
                        .saturating_mul(std::mem::size_of::<fanta_doc::FillRule>()),
                )
        })
}

fn build_text_geometry(node: &TextNode, include_outline: bool) -> TextGeometry {
    if !include_outline {
        let paint_bounds = with_shaped_layout(node, |layout| {
            paragraph_paint_bounds(
                node,
                layout,
                vertical_paint_offset(node, painted_text_height(node, layout)),
            )
        });
        return TextGeometry {
            outline: None,
            paint_bounds,
        };
    }
    // The paint cache owns immutable Paragraphs; glyph extraction needs a
    // mutable Paragraph and must not mutate a layout borrowed by caret/paint.
    let mut layout = shape_layout(node);
    // Match `draw_text_node`'s vertical-align origin, paragraph shifts, and its
    // fixed-box clip so the outline is the alpha silhouette that was painted.
    let dy = vertical_paint_offset(node, painted_text_height(node, &layout));
    let [width, height] = node.local_size;
    let clip = (node.auto_resize == TextAutoResize::None && width > 0.0 && height > 0.0)
        .then_some([0.0, 0.0, width, height]);
    let paint_bounds = paragraph_paint_bounds(node, &layout, dy);
    let outline = include_outline.then(|| {
        layout.outline_with_spacing_and_clip([0.0, dy], node.paragraph_spacing.max(0.0), clip)
    });
    TextGeometry {
        outline,
        paint_bounds,
    }
}

fn paragraph_paint_bounds(
    node: &TextNode,
    layout: &TextLayout,
    vertical_offset: f64,
) -> Result<Option<fanta_doc::Bounds>, TextPaintBoundsUnavailable> {
    let [width, height] = node.local_size;
    let spacing = node.paragraph_spacing.max(0.0);
    if !vertical_offset.is_finite()
        || !(vertical_offset as f32).is_finite()
        || !spacing.is_finite()
        || !(spacing as f32).is_finite()
    {
        return Err(TextPaintBoundsUnavailable);
    }
    let limit = f32::MAX / 4.0;
    let (wrap_width, _) = shape_params(node);
    if vertical_offset.abs() >= f64::from(limit)
        || (wrap_width.is_finite() && wrap_width.abs() >= f64::from(limit))
        || node.paragraph_indent.abs() >= f64::from(limit)
    {
        return Err(TextPaintBoundsUnavailable);
    }
    let recording_bounds = Rect::new(-limit, -limit, limit, limit);
    let mut recorder = skia_safe::PictureRecorder::new();
    let canvas = recorder.begin_recording(recording_bounds, true);
    if node.auto_resize == TextAutoResize::None && width > 0.0 && height > 0.0 {
        if !(width as f32).is_finite() || !(height as f32).is_finite() {
            return Err(TextPaintBoundsUnavailable);
        }
        canvas.clip_rect(
            Rect::from_xywh(0.0, 0.0, width as f32, height as f32),
            None,
            true,
        );
    }
    if spacing > 0.0 {
        layout.paint_spaced(canvas, [0.0, vertical_offset], spacing);
    } else {
        layout.paint(canvas, [0.0, vertical_offset]);
    }
    // RTree-enabled finish joins the native draw bounds and tightens the
    // recording's cull rect. Glyph blobs, bitmap glyphs and decorations all
    // contribute their actual paint bounds without becoming editable paths.
    let picture = recorder
        .finish_recording_as_picture(None)
        .ok_or(TextPaintBoundsUnavailable)?;
    let bounds = picture.cull_rect();
    if !bounds.is_finite()
        || !bounds.width().is_finite()
        || !bounds.height().is_finite()
        || bounds.left <= recording_bounds.left
        || bounds.top <= recording_bounds.top
        || bounds.right >= recording_bounds.right
        || bounds.bottom >= recording_bounds.bottom
    {
        return Err(TextPaintBoundsUnavailable);
    }
    if bounds.is_empty() {
        return Ok(None);
    }
    Ok(Some(fanta_doc::Bounds {
        min_x: f64::from(bounds.left),
        min_y: f64::from(bounds.top),
        max_x: f64::from(bounds.right),
        max_y: f64::from(bounds.bottom),
    }))
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
            let dy = vertical_paint_offset(node, painted_text_height(node, layout));
            let spacing = node.paragraph_spacing.max(0.0);
            if spacing > 0.0 {
                layout.paint_spaced(canvas, [0.0, dy], spacing);
            } else {
                layout.paint(canvas, [0.0, dy]);
            }
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
        line_height_auto_percent: s.line_height_auto_percent,
        font_variations: s.font_variations.clone(),
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

#[cfg(test)]
mod cache_tests {
    use super::*;
    use crate::raster::{RasterRenderer, RenderError};

    struct GeometryCacheScope {
        previous: TextGeometryCache,
    }

    impl GeometryCacheScope {
        fn new() -> Self {
            Self {
                previous: TEXT_GEOMETRY_CACHE
                    .with(|cache| std::mem::take(&mut *cache.borrow_mut())),
            }
        }
    }

    impl Drop for GeometryCacheScope {
        fn drop(&mut self) {
            TEXT_GEOMETRY_CACHE.with(|cache| {
                std::mem::swap(&mut *cache.borrow_mut(), &mut self.previous);
            });
        }
    }

    fn geometry_entry(node: &TextNode, font_generation: u64) -> TextGeometryEntry {
        let node = node.clone();
        let geometry = build_text_geometry(&node, true);
        let bytes = std::mem::size_of::<TextGeometryEntry>()
            + 64
            + text_source_heap_bytes(&node)
            + text_geometry_heap_bytes(&geometry);
        TextGeometryEntry {
            node,
            font_generation,
            geometry,
            bytes,
        }
    }

    #[test]
    fn retained_text_ink_preserves_rich_text_clipping_spacing_and_layout_edits() {
        let _scope = GeometryCacheScope::new();
        let mut node = TextNode::new("Italic\nUnderline", 185.0, 100.0);
        node.style.size_px = 24.0;
        node.style.italic = true;
        node.vertical_align = VAlign::Bottom;
        node.paragraph_spacing = 9.0;
        let mut style = node.style.clone();
        style.underline = true;
        style.weight = 700;
        node.style_runs.push(fanta_doc::TextStyleRun {
            start: 7,
            end: node.content.len(),
            style,
        });
        let mut variants = vec![node.clone()];
        let mut clipped = node.clone();
        clipped.local_size = [120.0, 65.0];
        variants.push(clipped);
        let mut centered = node.clone();
        centered.vertical_align = VAlign::Center;
        variants.push(centered);
        let mut spaced = node.clone();
        spaced.paragraph_spacing = 20.0;
        variants.push(spaced);
        let mut truncated = node.clone();
        truncated.truncate = true;
        truncated.max_lines = Some(1);
        variants.push(truncated);
        let mut auto_width = node.clone();
        auto_width.auto_resize = TextAutoResize::WidthAndHeight;
        variants.push(auto_width);
        let mut styled = node.clone();
        styled.style_runs.clear();
        styled.style.italic = false;
        variants.push(styled);
        for node in variants {
            let expected = build_text_geometry(&node, true);
            assert!(
                expected.outline.as_ref().is_some_and(Option::is_some),
                "bundled glyphs have outlines"
            );
            assert_eq!(text_node_outline(&node), expected.outline.clone().flatten());
            assert_eq!(text_node_paint_bounds(&node), expected.paint_bounds);
            assert_eq!(text_node_outline(&node), expected.outline.clone().flatten());
            let count = TEXT_GEOMETRY_CACHE.with(|cache| cache.borrow().entries.len());
            for _ in 0..8 {
                assert_eq!(text_node_paint_bounds(&node), expected.paint_bounds);
            }
            assert_eq!(
                TEXT_GEOMETRY_CACHE.with(|cache| cache.borrow().entries.len()),
                count
            );
        }
        let mut indented = node.clone();
        indented.paragraph_indent = 7.0;
        for node in [indented, TextNode::new(" ", 120.0, 65.0)] {
            let expected = build_text_geometry(&node, true);
            assert_eq!(text_node_outline(&node), expected.outline.clone().flatten());
            assert_eq!(text_node_paint_bounds(&node), expected.paint_bounds);
            assert_eq!(text_node_outline(&node), expected.outline.clone().flatten());
        }
        let empty = TextNode::new(" ", 120.0, 65.0);
        assert!(text_node_outline(&empty).is_none());
        assert!(text_node_ink_bounds(&empty).is_none());
    }

    #[test]
    fn text_geometry_requires_exact_source_and_font_epoch_even_on_hash_collision() {
        let mut cache = TextGeometryCache::default();
        let first = TextNode::new("First glyphs", 160.0, 80.0);
        let second = TextNode::new("Other glyphs", 160.0, 80.0);
        let first_entry = geometry_entry(&first, 4);
        let expected = first_entry.geometry.outline.clone();
        cache.insert(17, first_entry);
        assert_eq!(
            cache.get(17, &first, 4).map(|value| &value.outline),
            Some(&expected)
        );
        assert!(cache.get(17, &second, 4).is_none());
        assert!(cache.get(17, &first, 5).is_none());
        let mut moved_clip = first.clone();
        moved_clip.local_size[1] = 90.0;
        assert!(cache.get(17, &moved_clip, 4).is_none());
    }

    #[test]
    fn text_geometry_byte_budget_evicts_and_does_not_retain_oversize_outlines() {
        let first = TextNode::new("First glyphs", 160.0, 80.0);
        let second = TextNode::new("Other glyphs", 160.0, 80.0);
        let first_entry = geometry_entry(&first, 0);
        let second_entry = geometry_entry(&second, 0);
        let maximum_bytes = first_entry.bytes.max(second_entry.bytes);
        let mut cache = TextGeometryCache::new(TEXT_GEOMETRY_CACHE_CAP, maximum_bytes);
        cache.insert(1, first_entry);
        cache.insert(2, second_entry);
        assert!(cache.get(1, &first, 0).is_none());
        assert!(cache.get(2, &second, 0).is_some());
        assert!(cache.bytes <= maximum_bytes);
        let oversized = TextNode::new("A large outlined paragraph ".repeat(40), 320.0, 800.0);
        let oversized = geometry_entry(&oversized, 0);
        assert!(oversized.bytes > maximum_bytes);
        cache.insert(3, oversized);
        assert!(!cache.entries.contains(&3));
        assert!(cache.get(2, &second, 0).is_some());
        assert!(cache.bytes <= maximum_bytes);
    }

    struct CacheScope {
        previous: LruCache<u64, CachedTextLayout>,
        previous_font_generation: Option<u64>,
    }

    impl CacheScope {
        fn new(capacity: NonZeroUsize) -> Self {
            Self {
                previous: LAYOUT_CACHE.with(|cache| {
                    std::mem::replace(&mut *cache.borrow_mut(), LruCache::new(capacity))
                }),
                previous_font_generation: LAYOUT_FONT_GENERATION
                    .with(|generation| generation.get()),
            }
        }
    }

    impl Drop for CacheScope {
        fn drop(&mut self) {
            LAYOUT_CACHE.with(|cache| {
                std::mem::swap(&mut *cache.borrow_mut(), &mut self.previous);
            });
            LAYOUT_FONT_GENERATION.with(|generation| generation.set(self.previous_font_generation));
        }
    }

    #[test]
    fn changed_font_epoch_invalidates_painted_paragraphs_and_keeps_measurement_parity()
    -> Result<(), RenderError> {
        let _scope = CacheScope::new(LAYOUT_CACHE_CAP);
        let first = TextNode::new("Font metrics", 180.0, 80.0);
        let unrelated = TextNode::new("Fallback metrics", 180.0, 80.0);
        let measurement = crate::raster::instance::measure_text_node(&first);
        crate::raster::instance::measure_text_node(&unrelated);
        assert_eq!(layout_cache_len(), 2);
        let mut renderer = RasterRenderer::new(180, 80)?;
        draw_text_node(renderer.canvas(), &first);
        let pixels = renderer.copy_rgba();
        let generation =
            with_layout_engine(LayoutEngine::font_generation).expect("unsaturated fonts");
        LAYOUT_FONT_GENERATION.with(|previous| previous.set(Some(generation.wrapping_add(1))));
        assert_eq!(
            crate::raster::instance::measure_text_node(&first),
            measurement
        );
        assert_eq!(
            layout_cache_len(),
            1,
            "old fallback layouts cannot survive a font epoch change"
        );
        let mut renderer = RasterRenderer::new(180, 80)?;
        draw_text_node(renderer.canvas(), &first);
        assert_eq!(pixels, renderer.copy_rgba());
        Ok(())
    }

    fn key(node: &TextNode) -> u64 {
        let (width, align) = shape_params(node);
        shape_key(node, width, align)
    }

    #[test]
    fn paint_bounds_share_verified_shaping_without_extracting_editable_paths() {
        let _scope = CacheScope::new(LAYOUT_CACHE_CAP);
        let _geometry_scope = GeometryCacheScope::new();
        let target = TextNode::new("Labels with ordinary spaces", 220.0, 80.0);
        let other = TextNode::new("I", 220.0, 80.0);
        let expected = build_text_geometry(&target, true).paint_bounds;
        let generation = with_layout_engine(LayoutEngine::font_generation);
        LAYOUT_FONT_GENERATION.with(|previous| previous.set(generation));
        LAYOUT_CACHE.with(|cache| {
            drop(cache.borrow_mut().put(
                key(&target),
                CachedTextLayout {
                    node: other.clone(),
                    layout: shape_layout(&other),
                },
            ));
        });
        assert_eq!(text_node_paint_bounds(&target), expected);
        LAYOUT_CACHE.with(|cache| {
            let cache = cache.borrow();
            let cached = cache.peek(&key(&target)).expect("cached paragraph");
            assert!(shaping_inputs_match(&cached.node, &target));
        });
        TEXT_GEOMETRY_CACHE.with(|cache| {
            let cache = cache.borrow();
            let cached = cache
                .entries
                .peek(&text_geometry_key(&target))
                .expect("cached paint bounds");
            assert!(
                cached.geometry.outline.is_none(),
                "ink metadata must not extract vector paths"
            );
        });
        crate::raster::instance::measure_text_node(&target);
        assert_eq!(
            layout_cache_len(),
            1,
            "paint metadata shares measure/draw shaping"
        );
        assert_eq!(text_node_paint_bounds(&target), expected);
    }

    #[test]
    fn paragraph_paint_bounds_contain_real_glyph_decorations_spacing_indent_and_emoji_pixels()
    -> Result<(), RenderError> {
        let _geometry_scope = GeometryCacheScope::new();
        let mut base = TextNode::new("Italic labels\nSecond paragraph", 180.0, 150.0);
        base.auto_resize = TextAutoResize::Height;
        base.style.size_px = 28.0;
        base.style.italic = true;
        base.paragraph_spacing = 14.0;
        base.paragraph_indent = 11.0;
        base.vertical_align = VAlign::Center;
        let mut underlined = base.clone();
        underlined.style.underline = true;
        let mut struck = base.clone();
        struck.style.strikethrough = true;
        let mut rich = base.clone();
        let mut style = rich.style.clone();
        style.size_px = 36.0;
        style.underline = true;
        style.strikethrough = true;
        rich.style_runs.push(fanta_doc::TextStyleRun {
            start: 14,
            end: rich.content.len(),
            style,
        });
        let mut clipped = underlined.clone();
        clipped.auto_resize = TextAutoResize::None;
        clipped.local_size = [110.0, 45.0];
        clipped.vertical_align = VAlign::Bottom;
        let mut emoji = TextNode::new("A 😀 B", 180.0, 80.0);
        emoji.auto_resize = TextAutoResize::Height;
        emoji.style.size_px = 40.0;
        for (index, node) in [base, underlined, struck, rich, clipped, emoji]
            .into_iter()
            .enumerate()
        {
            let bounds = text_node_paint_bounds(&node)
                .expect("finite paragraph paint proof")
                .expect("fixture paints");
            assert!(bounds.is_finite());
            assert!(
                bounds.width() < 512.0 && bounds.height() < 512.0,
                "RTree must tighten its infinite recording cull"
            );
            for scale in [0.5, 1.0, 2.0] {
                let mut renderer = RasterRenderer::new(512, 512)?;
                renderer.canvas().translate((96.0, 96.0));
                renderer.canvas().scale((scale, scale));
                draw_text_node(renderer.canvas(), &node);
                let pixels = renderer.copy_rgba();
                let mut count = 0;
                for (pixel_index, pixel) in pixels.chunks_exact(4).enumerate() {
                    if pixel.last().copied() == Some(0) {
                        continue;
                    }
                    count += 1;
                    let x = (pixel_index % 512) as f64;
                    let y = (pixel_index / 512) as f64;
                    let scale = f64::from(scale);
                    assert!(
                        x + 1.0 > 96.0 + bounds.min_x * scale
                            && x < 96.0 + bounds.max_x * scale
                            && y + 1.0 > 96.0 + bounds.min_y * scale
                            && y < 96.0 + bounds.max_y * scale,
                        "painted pixel ({x},{y}) escapes paragraph {index} ink bounds {bounds:?} at scale {scale}"
                    );
                }
                assert!(count > 0, "paragraph {index} paints at scale {scale}");
            }
            assert_eq!(text_node_paint_bounds(&node), Ok(Some(bounds)));
            TEXT_GEOMETRY_CACHE.with(|cache| {
                let cache = cache.borrow();
                let cached = cache
                    .entries
                    .peek(&text_geometry_key(&node))
                    .expect("paint metadata retained");
                assert!(cached.geometry.outline.is_none());
            });
        }
        assert_eq!(
            text_node_paint_bounds(&TextNode::new("", 80.0, 80.0)),
            Ok(None)
        );
        assert_eq!(
            text_node_paint_bounds(&TextNode::new("   ", 80.0, 80.0)),
            Ok(None)
        );
        let mut invalid = TextNode::new("Invalid", 80.0, f64::INFINITY);
        invalid.vertical_align = VAlign::Bottom;
        assert_eq!(
            text_node_paint_bounds(&invalid),
            Err(TextPaintBoundsUnavailable)
        );
        Ok(())
    }

    #[test]
    fn shaped_layout_overflow_keeps_measured_and_painted_hot_entries() -> Result<(), RenderError> {
        let _scope = CacheScope::new(NonZeroUsize::MIN.saturating_add(2));
        let first = TextNode::new("First", 200.0, 80.0);
        let second = TextNode::new("Second", 200.0, 80.0);
        let third = TextNode::new("Third", 200.0, 80.0);
        let fourth = TextNode::new("Fourth", 200.0, 80.0);
        let measured = crate::raster::instance::measure_text_node(&first);
        for node in [&first, &second, &third] {
            crate::raster::instance::measure_text_node(node);
        }
        let render = || -> Result<Vec<u8>, RenderError> {
            let mut renderer = RasterRenderer::new(200, 80)?;
            draw_text_node(renderer.canvas(), &first);
            Ok(renderer.copy_rgba())
        };
        let before = render()?;
        assert!(
            before
                .chunks_exact(4)
                .any(|pixel| pixel.last().copied() != Some(0))
        );
        crate::raster::instance::measure_text_node(&fourth);
        LAYOUT_CACHE.with(|cache| {
            let cache = cache.borrow();
            assert_eq!(cache.len(), 3);
            assert!(cache.contains(&key(&first)));
            assert!(!cache.contains(&key(&second)));
            assert!(cache.contains(&key(&third)));
            assert!(cache.contains(&key(&fourth)));
        });
        assert_eq!(before, render()?);
        assert_eq!(measured, crate::raster::instance::measure_text_node(&first));
        crate::raster::instance::measure_text_node(&third);
        crate::raster::instance::measure_text_node(&second);
        LAYOUT_CACHE.with(|cache| {
            let cache = cache.borrow();
            assert_eq!(cache.len(), 3);
            assert!(cache.contains(&key(&first)));
            assert!(cache.contains(&key(&third)));
            assert!(!cache.contains(&key(&fourth)));
            assert!(cache.contains(&key(&second)));
        });
        Ok(())
    }

    #[test]
    fn evicted_layout_rebuild_keeps_exact_pixels_and_measurement() -> Result<(), RenderError> {
        let _scope = CacheScope::new(NonZeroUsize::MIN.saturating_add(1));
        let mut target = TextNode::new("Cache parity", 120.0, 64.0);
        target.style.size_px = 22.0;
        let measured = crate::raster::instance::measure_text_node(&target);
        let render = || -> Result<Vec<u8>, RenderError> {
            let mut renderer = RasterRenderer::new(120, 64)?;
            draw_text_node(renderer.canvas(), &target);
            Ok(renderer.copy_rgba())
        };
        let before = render()?;
        assert!(
            before
                .chunks_exact(4)
                .any(|pixel| pixel.last().copied() != Some(0))
        );
        for content in ["Another run", "One more run"] {
            crate::raster::instance::measure_text_node(&TextNode::new(content, 120.0, 64.0));
        }
        LAYOUT_CACHE.with(|cache| assert!(!cache.borrow().contains(&key(&target))));
        assert_eq!(
            measured,
            crate::raster::instance::measure_text_node(&target)
        );
        assert_eq!(before, render()?);
        assert_eq!(layout_cache_len(), 2);
        Ok(())
    }
}
