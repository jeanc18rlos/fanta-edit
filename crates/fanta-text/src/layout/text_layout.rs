//! The laid-out paragraph artifact: query geometry, hit-test, caret, paint.

use skia_safe::{
    Canvas,
    textlayout::{RectHeightStyle, RectWidthStyle},
};

/// Per-line geometry extracted from a laid-out paragraph.
///
/// Mirrors the subset of Skia's `LineMetrics` the rest of Fantaisa needs for
/// drawing and selection: the vertical band the line occupies and its width.
/// Stored in our own `f64` type rather than re-exposing Skia's struct so the
/// public API has no Skia in its signature.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LineMetrics {
    /// Zero-based line index.
    pub line: usize,
    /// Top edge of the line in paragraph-local pixels (y grows downward).
    pub top: f64,
    /// Baseline y, measured from the paragraph top.
    pub baseline: f64,
    /// Total line height (`ascent + descent`).
    pub height: f64,
    /// Rendered width of the line's text.
    pub width: f64,
    /// First byte offset on the line.
    pub start_byte: usize,
    /// One-past-last byte offset on the line (including any trailing newline).
    pub end_byte: usize,
}

/// A laid-out paragraph: the shaped, line-broken result of running a
/// [`TextBuffer`](crate::buffer::TextBuffer) through Skia at a fixed wrap width.
///
/// Holds the Skia `Paragraph` plus a copy of the source text. The source text
/// is retained because hit-testing and caret math need to clamp byte offsets to
/// `char` boundaries, and re-deriving that from the paragraph would be
/// circuitous. A layout is a transient render artifact — rebuild it whenever the
/// buffer content or wrap width changes; it is not meant to be mutated in place.
pub struct TextLayout {
    pub(super) paragraph: skia_safe::textlayout::Paragraph,
    pub(super) text: String,
    pub(super) max_width: f64,
}

impl std::fmt::Debug for TextLayout {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TextLayout")
            .field("text_len", &self.text.len())
            .field("max_width", &self.max_width)
            .field("line_count", &self.line_count())
            .field("height", &self.height())
            .field("width", &self.width())
            .finish()
    }
}

impl TextLayout {
    /// Number of laid-out lines. Zero for empty text; at least one for any
    /// non-empty text (even a single unbroken word).
    pub fn line_count(&self) -> usize {
        if self.text.is_empty() {
            0
        } else {
            self.paragraph.line_number()
        }
    }

    /// Total laid-out height in pixels. Zero for empty text.
    ///
    /// Empty text is special-cased to zero rather than reporting the height of
    /// one blank line: a zero-glyph text node should occupy no space until the
    /// user types, which is what the canvas and the layout tests expect.
    pub fn height(&self) -> f64 {
        if self.text.is_empty() {
            0.0
        } else {
            self.paragraph.height() as f64
        }
    }

    /// Width of the widest line in pixels (the paragraph's longest line). Zero
    /// for empty text. This is the *content* width, which can be less than the
    /// `max_width` the paragraph was laid out against.
    pub fn width(&self) -> f64 {
        if self.text.is_empty() {
            0.0
        } else {
            self.paragraph.longest_line() as f64
        }
    }

    /// The wrap width this layout was produced at.
    pub fn max_width(&self) -> f64 {
        self.max_width
    }

    /// Per-line metrics, in line order.
    pub fn lines(&self) -> Vec<LineMetrics> {
        if self.text.is_empty() {
            return Vec::new();
        }
        self.paragraph
            .get_line_metrics()
            .iter()
            .map(|lm| LineMetrics {
                line: lm.line_number,
                top: lm.baseline - lm.ascent,
                baseline: lm.baseline,
                height: lm.height,
                width: lm.width,
                start_byte: lm.start_index,
                end_byte: lm.end_index,
            })
            .collect()
    }

    /// The byte offset nearest a point in paragraph-local coordinates.
    ///
    /// This is what turns a mouse click into a caret position. Skia reports the
    /// glyph position at the coordinate as a UTF-8 index; we clamp it into range
    /// and onto a `char` boundary so the result is always a valid caret for the
    /// buffer. A click inside the first glyph therefore maps to byte 0, a click
    /// past the end maps to `len`.
    pub fn hit_test(&self, point: [f64; 2]) -> usize {
        if self.text.is_empty() {
            return 0;
        }
        let pos = self
            .paragraph
            .get_glyph_position_at_coordinate((point[0] as f32, point[1] as f32));
        // `position` is a UTF-8 code-unit (byte) index; it can be negative only
        // in degenerate cases, so clamp through 0.
        let raw = pos.position.max(0) as usize;
        self.clamp_to_boundary(raw)
    }

    /// The caret rectangle `[x, y, width, height]` for the cursor sitting *at*
    /// byte offset `byte`, in paragraph-local coordinates.
    ///
    /// A caret is a zero-width vertical bar, so `width` is 0 and `height` is the
    /// line height at that offset. We obtain the geometry from the glyph rect at
    /// the offset: the caret sits on that glyph's leading (left) edge. At
    /// end-of-text we use the trailing edge of the last glyph instead, since
    /// there is no glyph starting at `len`. This is what the app draws as the
    /// blinking cursor and uses to scroll the caret into view.
    pub fn caret_rect(&self, byte: usize) -> [f64; 4] {
        if self.text.is_empty() {
            // No content: caret at the origin with a nominal line height taken
            // from a zero-length layout would be 0, so report a zero rect. The
            // editor overlays its own default-height caret for empty fields.
            return [0.0, 0.0, 0.0, 0.0];
        }
        let byte = self.clamp_to_boundary(byte);
        let len = self.text.len();

        // For a caret before a glyph, query the rect of the single grapheme
        // starting at `byte`; the caret is its left edge.
        if byte < len {
            let end = self.next_char_boundary(byte);
            let boxes = self.paragraph.get_rects_for_range(
                byte..end,
                RectHeightStyle::Max,
                RectWidthStyle::Tight,
            );
            if let Some(tb) = boxes.first() {
                let r = tb.rect;
                return [r.left as f64, r.top as f64, 0.0, (r.bottom - r.top) as f64];
            }
        }

        // End-of-text (or no box found): use the trailing edge of the last
        // grapheme so the caret lands after the final character.
        let start = self.prev_char_boundary(len);
        let boxes = self.paragraph.get_rects_for_range(
            start..len,
            RectHeightStyle::Max,
            RectWidthStyle::Tight,
        );
        if let Some(tb) = boxes.last() {
            let r = tb.rect;
            return [r.right as f64, r.top as f64, 0.0, (r.bottom - r.top) as f64];
        }
        [0.0, 0.0, 0.0, 0.0]
    }

    /// Filled rectangles `[x, y, w, h]` covering the glyphs in `start..end`,
    /// in paragraph-local pixels — for drawing a text selection highlight. Skia
    /// returns one box per visual run, so a multi-line selection yields several
    /// boxes. Empty / collapsed ranges yield no boxes.
    pub fn selection_rects(&self, start: usize, end: usize) -> Vec<[f64; 4]> {
        let (lo, hi) = (start.min(end), start.max(end));
        if self.text.is_empty() || lo >= hi {
            return Vec::new();
        }
        let a = self.clamp_to_boundary(lo);
        let b = self.clamp_to_boundary(hi);
        self.paragraph
            .get_rects_for_range(a..b, RectHeightStyle::Max, RectWidthStyle::Tight)
            .iter()
            .map(|tb| {
                let r = tb.rect;
                [
                    r.left as f64,
                    r.top as f64,
                    (r.right - r.left) as f64,
                    (r.bottom - r.top) as f64,
                ]
            })
            .collect()
    }

    /// Paint the laid-out glyphs into `canvas`, with the paragraph's top-left at
    /// `origin` (paragraph-local pixels; y grows downward).
    ///
    /// This is the bridge from a layout to actual pixels — the rest of the
    /// crate confines every Skia call to this module, so a renderer never
    /// touches `skia_safe::textlayout` directly: it builds a [`TextLayout`] via
    /// [`LayoutEngine`](crate::layout::LayoutEngine) and calls this. Glyph color,
    /// size, weight, and alignment are all baked into the layout at build time
    /// (the per-run [`TextStyle`](crate::style::TextStyle) sets color;
    /// [`LayoutEngine::layout_aligned`](crate::layout::LayoutEngine::layout_aligned)
    /// sets alignment), so there is no paint argument — the caller positions the
    /// box and Skia draws the styled glyphs.
    ///
    /// Empty text paints nothing (the paragraph has no glyphs), which is a
    /// no-op rather than an error so callers don't special-case it.
    pub fn paint(&self, canvas: &Canvas, origin: [f64; 2]) {
        if self.text.is_empty() {
            return;
        }
        self.paragraph
            .paint(canvas, (origin[0] as f32, origin[1] as f32));
    }

    /// Byte offsets at which each hard-break paragraph begins: 0, then the
    /// byte after every `'\n'`. A trailing newline therefore starts a final
    /// (possibly empty) paragraph, matching how Skia lays out a trailing blank
    /// line.
    fn paragraph_starts(&self) -> Vec<usize> {
        let mut starts = vec![0];
        starts.extend(self.text.match_indices('\n').map(|(i, _)| i + 1));
        starts
    }

    /// Total painted height when `spacing` extra pixels are inserted after
    /// every hard newline (Figma's `paragraphSpacing`); equals
    /// [`height`](Self::height) for zero spacing or single-paragraph text.
    /// This is the height [`paint_spaced`](Self::paint_spaced) occupies, so
    /// vertical alignment and auto-height measurement agree with the pixels.
    pub fn spaced_height(&self, spacing: f64) -> f64 {
        if self.text.is_empty() || spacing == 0.0 {
            return self.height();
        }
        let breaks = self.text.matches('\n').count();
        self.height() + spacing * breaks as f64
    }

    /// Paint like [`paint`](Self::paint), inserting `spacing` extra vertical
    /// pixels after each hard-break paragraph (Figma's `paragraphSpacing`).
    ///
    /// Skia's `Paragraph` has no between-paragraph spacing, so the paragraph
    /// is painted once per hard-break block, each pass clipped to that block's
    /// line band and shifted down by the spacing accumulated before it. The
    /// bands are the exact line boxes (line tops tile), so each glyph is
    /// painted by exactly one pass. Zero spacing (or one paragraph) takes the
    /// single-paint fast path, byte-identical to [`paint`](Self::paint).
    pub fn paint_spaced(&self, canvas: &Canvas, origin: [f64; 2], spacing: f64) {
        if self.text.is_empty() {
            return;
        }
        let starts = self.paragraph_starts();
        if spacing == 0.0 || starts.len() <= 1 {
            self.paint(canvas, origin);
            return;
        }
        let lines = self.lines();
        // The y where each paragraph's band begins: the top of its first line.
        // Lines are in order and a paragraph's first line starts exactly at the
        // paragraph's start byte (the preceding '\n' ends the previous line). A
        // paragraph past the last laid-out line (e.g. truncated away) has no
        // band and paints nothing.
        let mut band_tops: Vec<Option<f64>> = Vec::with_capacity(starts.len());
        let mut line_index = 0;
        for &start in &starts {
            while line_index < lines.len() && lines[line_index].start_byte < start {
                line_index += 1;
            }
            band_tops.push(lines.get(line_index).map(|line| line.top));
        }
        for (paragraph, top) in band_tops.iter().enumerate() {
            let Some(top) = *top else {
                continue;
            };
            let bottom = band_tops[paragraph + 1..]
                .iter()
                .find_map(|t| *t)
                .unwrap_or(f64::INFINITY);
            let shift = spacing * paragraph as f64;
            // Effectively-unbounded extent that still survives the canvas CTM
            // without overflowing to infinity (glyphs may overhang the layout
            // box horizontally; the last band is open-ended downward).
            const BAND_REACH: f32 = 1.0e7;
            canvas.save();
            // Clip in the shifted frame so the band keeps exactly this
            // paragraph's lines.
            canvas.clip_rect(
                skia_safe::Rect::new(
                    -BAND_REACH,
                    (origin[1] + top + shift) as f32,
                    BAND_REACH,
                    if bottom.is_finite() {
                        (origin[1] + bottom + shift) as f32
                    } else {
                        BAND_REACH
                    },
                ),
                None,
                false,
            );
            self.paint(canvas, [origin[0], origin[1] + shift]);
            canvas.restore();
        }
    }

    /// The glyph outlines of the laid-out text as a single vector
    /// [`fanta_doc::PathData`], in paragraph-local pixels translated by `offset`
    /// (the renderer's paint origin — its vertical-alignment shift). This is the
    /// "convert text to path / outline text" primitive: it unions every line's
    /// filled glyph contours (Skia [`Paragraph::get_path_at`]) into one path so a
    /// text node can be replaced by an editable vector that renders pixel-identical
    /// to the text. Returns `None` for empty text or when no glyph produced
    /// geometry. `&mut self` because Skia builds the per-line path lazily.
    pub fn outline(&mut self, offset: [f64; 2]) -> Option<fanta_doc::PathData> {
        self.outline_with_spacing_and_clip(offset, 0.0, None)
    }

    /// The glyph outline with Figma paragraph spacing applied and an optional
    /// `[x, y, width, height]` clip in the returned path's coordinate space.
    /// This is the geometry counterpart of [`paint_spaced`](Self::paint_spaced):
    /// later hard-break paragraphs receive the same accumulated vertical shift.
    pub fn outline_with_spacing_and_clip(
        &mut self,
        offset: [f64; 2],
        spacing: f64,
        clip: Option<[f64; 4]>,
    ) -> Option<fanta_doc::PathData> {
        if self.text.is_empty() {
            return None;
        }

        let lines = self.lines();
        let paragraph_starts = self.paragraph_starts();
        let spacing = spacing.max(0.0);
        let mut outline = skia_safe::Path::new();
        let line_count = self.paragraph.line_number();
        for line in 0..line_count {
            let (_, path) = self.paragraph.get_path_at(line);
            if line == 0 {
                outline.set_fill_type(path.fill_type());
            }
            let paragraph = lines
                .get(line)
                .map(|metrics| {
                    paragraph_starts
                        .partition_point(|start| *start <= metrics.start_byte)
                        .saturating_sub(1)
                })
                .unwrap_or(0);
            outline.add_path(
                &path,
                (
                    offset[0] as f32,
                    (offset[1] + spacing * paragraph as f64) as f32,
                ),
                None,
            );
        }

        if let Some([x, y, width, height]) = clip {
            if width <= 0.0 || height <= 0.0 {
                return None;
            }
            let mut clip_path = skia_safe::Path::new();
            clip_path.add_rect(
                skia_safe::Rect::from_xywh(x as f32, y as f32, width as f32, height as f32),
                None,
            );
            outline = outline.op(&clip_path, skia_safe::PathOp::Intersect)?;
        }

        path_data_from_sk_path(&outline)
    }

    // -- boundary helpers --------------------------------------------------

    /// Clamp `byte` into `0..=len` and back to the nearest `char` boundary at or
    /// below it, so a value Skia hands back (or a caller passes) is always a
    /// legal caret position for the source text.
    fn clamp_to_boundary(&self, byte: usize) -> usize {
        let mut b = byte.min(self.text.len());
        while b > 0 && !self.text.is_char_boundary(b) {
            b -= 1;
        }
        b
    }

    /// Next `char` boundary strictly after `byte` (or `len`).
    fn next_char_boundary(&self, byte: usize) -> usize {
        let len = self.text.len();
        let mut b = (byte + 1).min(len);
        while b < len && !self.text.is_char_boundary(b) {
            b += 1;
        }
        b
    }

    /// Previous `char` boundary strictly before `byte` (or 0).
    fn prev_char_boundary(&self, byte: usize) -> usize {
        if byte == 0 {
            return 0;
        }
        let mut b = byte - 1;
        while b > 0 && !self.text.is_char_boundary(b) {
            b -= 1;
        }
        b
    }
}

fn path_data_from_sk_path(path: &skia_safe::Path) -> Option<fanta_doc::PathData> {
    use skia_safe::path::Verb;

    let point = |point: skia_safe::Point| (point.x as f64, point.y as f64);
    let mut data = fanta_doc::PathData::new();
    data.fill_rule = match path.fill_type() {
        skia_safe::PathFillType::EvenOdd | skia_safe::PathFillType::InverseEvenOdd => {
            fanta_doc::FillRule::EvenOdd
        }
        _ => fanta_doc::FillRule::NonZero,
    };
    let mut iterator = skia_safe::path::Iter::new(path, false);
    while let Some((verb, points)) = iterator.next() {
        match verb {
            Verb::Move => {
                let (x, y) = point(points[0]);
                data.move_to(x, y);
            }
            Verb::Line => {
                if iterator.is_close_line() {
                    continue;
                }
                let (x, y) = point(points[1]);
                data.line_to(x, y);
            }
            Verb::Quad => {
                let (control_x, control_y) = point(points[1]);
                let (x, y) = point(points[2]);
                data.quad_to(control_x, control_y, x, y);
            }
            Verb::Conic => {
                const POW2: usize = 2;
                let weight = iterator.conic_weight().unwrap_or(1.0);
                let mut quads = [skia_safe::Point::default(); 1 + 2 * (1 << POW2)];
                let count = skia_safe::Path::convert_conic_to_quads(
                    points[0], points[1], points[2], weight, &mut quads, POW2,
                )
                .unwrap_or(0);
                for index in 0..count {
                    let (control_x, control_y) = point(quads[2 * index + 1]);
                    let (x, y) = point(quads[2 * index + 2]);
                    data.quad_to(control_x, control_y, x, y);
                }
            }
            Verb::Cubic => {
                let (control_1_x, control_1_y) = point(points[1]);
                let (control_2_x, control_2_y) = point(points[2]);
                let (x, y) = point(points[3]);
                data.cubic_to(control_1_x, control_1_y, control_2_x, control_2_y, x, y);
            }
            Verb::Close => {
                data.close();
            }
            Verb::Done => break,
        }
    }
    (!data.segments.is_empty()).then_some(data)
}

#[cfg(test)]
mod tests {
    use crate::buffer::TextBuffer;
    use crate::layout::{Align, LayoutEngine};
    use crate::style::TextStyle;

    fn body() -> TextStyle {
        TextStyle::default()
    }

    #[test]
    fn empty_buffer_lays_out_to_zero_metrics() {
        let engine = LayoutEngine::new();
        let buf = TextBuffer::new();
        let layout = engine.layout(&buf, 200.0);
        assert_eq!(layout.line_count(), 0);
        assert_eq!(layout.height(), 0.0);
        assert_eq!(layout.width(), 0.0);
        assert!(layout.lines().is_empty());
    }

    #[test]
    fn variable_font_weight_axis_widens_the_text() {
        use fanta_doc::FontVariation;
        // The bundled "Source Sans 3" is a variable font. Setting the `wght` axis
        // to a heavy value widens the glyphs versus a light value — proving the
        // run's variation coordinates reach Skia. Both must lay out without panic.
        let engine = LayoutEngine::new();
        let width_at = |weight: f32| -> f64 {
            let mut style = TextStyle::new("Source Sans 3", 48.0);
            style.font_variations = vec![FontVariation::new("wght", weight)];
            engine
                .layout(&TextBuffer::from_str("Weight", style), 1.0e7)
                .width()
        };
        let light = width_at(200.0);
        let heavy = width_at(900.0);
        assert!(
            light > 0.0 && heavy > 0.0,
            "both weights lay out: {light} / {heavy}"
        );
        assert!(
            heavy > light,
            "wght=900 should be wider than wght=200: {light} vs {heavy}"
        );
    }

    #[test]
    fn non_empty_text_has_positive_height_and_width() {
        let engine = LayoutEngine::new();
        let buf = TextBuffer::from_str("Hello, world", body());
        let layout = engine.layout(&buf, 1000.0);
        assert!(layout.height() > 0.0, "height should be positive");
        assert!(layout.width() > 0.0, "width should be positive");
        assert_eq!(layout.line_count(), 1, "short text fits on one line");
    }

    #[test]
    fn long_text_wraps_to_multiple_lines() {
        let engine = LayoutEngine::new();
        let text = "The quick brown fox jumps over the lazy dog repeatedly and at length";
        let buf = TextBuffer::from_str(text, body());
        // Narrow width forces wrapping.
        let narrow = engine.layout(&buf, 80.0);
        let wide = engine.layout(&buf, 5000.0);
        assert!(
            narrow.line_count() > wide.line_count(),
            "narrow ({}) should wrap into more lines than wide ({})",
            narrow.line_count(),
            wide.line_count()
        );
        assert_eq!(wide.line_count(), 1);
    }

    #[test]
    fn auto_width_label_stays_one_line_at_no_wrap_width() {
        // Regression for the Figma-import card overlap: an auto-width label whose
        // hugged box width barely fits the text wraps to 2 lines when laid out at
        // that width, but stays 1 line at a no-wrap-wide width. The importer feeds
        // the latter for WIDTH_AND_HEIGHT text so a title never spills onto its
        // subtitle below.
        let engine = LayoutEngine::new();
        let buf = TextBuffer::from_str("View guidelines", TextStyle::new("Helvetica", 15.0));
        // A width just under the natural text width forces a wrap.
        let natural = engine.layout(&buf, 1.0e7).width();
        let tight = engine.layout(&buf, natural * 0.7);
        let no_wrap = engine.layout(&buf, 1.0e7);
        assert!(tight.line_count() >= 2, "a too-narrow box wraps the label");
        assert_eq!(
            no_wrap.line_count(),
            1,
            "no-wrap width keeps it on one line"
        );
        // And the no-wrap layout is exactly one line tall — it can't overlap a
        // sibling positioned one line below it.
        assert!(no_wrap.height() < tight.height());
    }

    #[test]
    fn taller_text_for_more_lines() {
        let engine = LayoutEngine::new();
        let buf = TextBuffer::from_str("line one\nline two\nline three", body());
        let layout = engine.layout(&buf, 5000.0);
        assert_eq!(layout.line_count(), 3, "hard newlines make three lines");
        let single = engine.layout(&TextBuffer::from_str("one", body()), 5000.0);
        assert!(layout.height() > single.height());
    }

    #[test]
    fn hit_test_inside_first_glyph_is_zero() {
        let engine = LayoutEngine::new();
        let buf = TextBuffer::from_str("Hello", body());
        let layout = engine.layout(&buf, 1000.0);
        // A point at the very top-left, inside the first glyph, is byte 0.
        assert_eq!(layout.hit_test([0.5, 2.0]), 0);
    }

    #[test]
    fn hit_test_far_right_is_end_of_text() {
        let engine = LayoutEngine::new();
        let buf = TextBuffer::from_str("Hello", body());
        let layout = engine.layout(&buf, 1000.0);
        // Way past the end of the single line maps to the last byte.
        let hit = layout.hit_test([10_000.0, 2.0]);
        assert_eq!(hit, buf.len(), "click past end should land at len");
    }

    #[test]
    fn hit_test_empty_text_is_zero() {
        let engine = LayoutEngine::new();
        let layout = engine.layout(&TextBuffer::new(), 200.0);
        assert_eq!(layout.hit_test([50.0, 50.0]), 0);
    }

    #[test]
    fn hit_test_progresses_left_to_right() {
        let engine = LayoutEngine::new();
        let buf = TextBuffer::from_str("Hello world", body());
        let layout = engine.layout(&buf, 2000.0);
        let near_start = layout.hit_test([1.0, 2.0]);
        let near_mid = layout.hit_test([layout.width() / 2.0, 2.0]);
        let near_end = layout.hit_test([layout.width() - 1.0, 2.0]);
        assert!(
            near_start <= near_mid && near_mid <= near_end,
            "hit offsets should be monotonic left→right: {near_start} {near_mid} {near_end}"
        );
        assert_eq!(near_start, 0);
    }

    #[test]
    fn caret_rect_has_line_height_and_advances() {
        let engine = LayoutEngine::new();
        let buf = TextBuffer::from_str("Hello", body());
        let layout = engine.layout(&buf, 1000.0);
        let r0 = layout.caret_rect(0);
        let r3 = layout.caret_rect(3);
        // Width is always zero (a caret is a bar); height is the line height.
        assert_eq!(r0[2], 0.0);
        assert!(r0[3] > 0.0, "caret height should be positive");
        // The caret at byte 3 is to the right of the caret at byte 0.
        assert!(
            r3[0] > r0[0],
            "caret x should advance: {} vs {}",
            r3[0],
            r0[0]
        );
    }

    #[test]
    fn caret_rect_at_end_is_past_last_glyph() {
        let engine = LayoutEngine::new();
        let buf = TextBuffer::from_str("Hi", body());
        let layout = engine.layout(&buf, 1000.0);
        let r_start = layout.caret_rect(0);
        let r_end = layout.caret_rect(buf.len());
        assert!(
            r_end[0] > r_start[0],
            "end caret should be right of start caret"
        );
    }

    #[test]
    fn caret_rect_for_empty_is_zero() {
        let engine = LayoutEngine::new();
        let layout = engine.layout(&TextBuffer::new(), 200.0);
        assert_eq!(layout.caret_rect(0), [0.0, 0.0, 0.0, 0.0]);
    }

    #[test]
    fn spaced_height_adds_spacing_per_hard_break() {
        let engine = LayoutEngine::new();
        let layout = engine.layout(&TextBuffer::from_str("a\nb\nc", body()), 1000.0);
        assert_eq!(layout.spaced_height(0.0), layout.height());
        assert!((layout.spaced_height(10.0) - layout.height() - 20.0).abs() < 1e-9);
        // Single paragraph: no breaks, no growth.
        let single = engine.layout(&TextBuffer::from_str("abc", body()), 1000.0);
        assert_eq!(single.spaced_height(10.0), single.height());
        // Empty text stays zero.
        let empty = engine.layout(&TextBuffer::new(), 1000.0);
        assert_eq!(empty.spaced_height(10.0), 0.0);
    }

    #[test]
    fn paint_spaced_shifts_later_paragraphs_down() {
        use skia_safe::{AlphaType, ColorType, ImageInfo, surfaces};
        let engine = LayoutEngine::new();
        let style =
            crate::style::TextStyle::new("Inter", 20.0).with_color(fanta_doc::Color::rgb(0, 0, 0));
        let layout = engine.layout(&TextBuffer::from_str("Top\nBottom", style), 1000.0);

        let lowest_ink = |spacing: f64| -> usize {
            let (w, h) = (160i32, 160i32);
            let mut surface = surfaces::raster_n32_premul((w, h)).unwrap();
            surface.canvas().clear(skia_safe::Color::TRANSPARENT);
            layout.paint_spaced(surface.canvas(), [4.0, 4.0], spacing);
            let info = ImageInfo::new((w, h), ColorType::RGBA8888, AlphaType::Unpremul, None);
            let row = info.min_row_bytes();
            let mut px = vec![0u8; row * h as usize];
            assert!(surface.read_pixels(&info, &mut px, row, (0, 0)));
            let mut lowest = 0;
            for y in 0..h as usize {
                for x in 0..w as usize {
                    if px[y * row + x * 4 + 3] != 0 {
                        lowest = y;
                    }
                }
            }
            lowest
        };
        let plain = lowest_ink(0.0);
        let spaced = lowest_ink(32.0);
        let shift = spaced as i64 - plain as i64;
        assert!(
            (shift - 32).abs() <= 2,
            "the second paragraph must paint ~32px lower, moved {shift}px"
        );
    }

    #[test]
    fn paint_draws_glyphs_onto_a_surface() {
        use skia_safe::{AlphaType, ColorType, ImageInfo, surfaces};

        let engine = LayoutEngine::new();
        // Opaque red text so painted glyphs are unambiguous against a cleared
        // (transparent) surface.
        let style = TextStyle::new("Helvetica", 32.0).with_color(fanta_doc::Color::rgb(255, 0, 0));
        let buf = TextBuffer::from_str("Hi", style);
        let layout = engine.layout(&buf, 500.0);

        let mut surface = surfaces::raster_n32_premul((128, 64)).unwrap();
        surface.canvas().clear(skia_safe::Color::TRANSPARENT);
        layout.paint(surface.canvas(), [4.0, 4.0]);

        let info = ImageInfo::new((128, 64), ColorType::RGBA8888, AlphaType::Unpremul, None);
        let row = info.min_row_bytes();
        let mut buf_px = vec![0u8; row * 64];
        assert!(surface.read_pixels(&info, &mut buf_px, row, (0, 0)));
        // Some pixel must be non-transparent: the glyphs actually drew.
        assert!(
            buf_px.iter().any(|&b| b != 0),
            "painting non-empty text must light up pixels"
        );
    }

    #[test]
    fn outline_produces_glyph_geometry_for_non_empty_text() {
        let engine = LayoutEngine::new();
        let buf = TextBuffer::from_str("Hello", TextStyle::new("Helvetica", 32.0));
        let mut layout = engine.layout(&buf, 1000.0);
        let path = layout.outline([0.0, 0.0]).expect("non-empty text outlines");
        // Real glyph contours: several segments, and a non-degenerate bounding box.
        assert!(
            path.segments.len() > 4,
            "expected glyph contours, got {:?}",
            path.segments.len()
        );
        let b = path.rough_bounds().expect("outline has bounds");
        assert!(b.width() > 0.0 && b.height() > 0.0, "outline bounds {b:?}");
    }

    #[test]
    fn outline_offset_translates_the_path() {
        let engine = LayoutEngine::new();
        let buf = TextBuffer::from_str("Ag", TextStyle::new("Helvetica", 24.0));
        let a = engine.layout(&buf, 1000.0).outline([0.0, 0.0]).unwrap();
        let b = engine.layout(&buf, 1000.0).outline([0.0, 50.0]).unwrap();
        let (ba, bb) = (a.rough_bounds().unwrap(), b.rough_bounds().unwrap());
        // The offset shifts every glyph down by 50px (within rough-bounds slack).
        assert!(
            (bb.min_y - ba.min_y - 50.0).abs() < 2.0,
            "expected +50 y shift: {ba:?} vs {bb:?}"
        );
    }

    #[test]
    fn outline_empty_text_is_none() {
        let engine = LayoutEngine::new();
        let mut layout = engine.layout(&TextBuffer::new(), 200.0);
        assert!(layout.outline([0.0, 0.0]).is_none());
    }

    #[test]
    fn paint_empty_text_is_a_noop() {
        use skia_safe::{AlphaType, ColorType, ImageInfo, surfaces};

        let engine = LayoutEngine::new();
        let layout = engine.layout(&TextBuffer::new(), 200.0);
        let mut surface = surfaces::raster_n32_premul((32, 32)).unwrap();
        surface.canvas().clear(skia_safe::Color::TRANSPARENT);
        layout.paint(surface.canvas(), [0.0, 0.0]);

        let info = ImageInfo::new((32, 32), ColorType::RGBA8888, AlphaType::Unpremul, None);
        let row = info.min_row_bytes();
        let mut buf_px = vec![0u8; row * 32];
        assert!(surface.read_pixels(&info, &mut buf_px, row, (0, 0)));
        assert!(buf_px.iter().all(|&b| b == 0), "empty text paints nothing");
    }

    #[test]
    fn alignment_shifts_glyph_position_within_the_box() {
        use skia_safe::{AlphaType, ColorType, ImageInfo, surfaces};

        // A short word in a wide box. Left vs right alignment must place the
        // glyphs at different horizontal positions, so the rightmost lit column
        // differs between the two. This is the load-bearing behaviour the
        // renderer relies on for `TextAlign`.
        let engine = LayoutEngine::new();
        let style = TextStyle::new("Helvetica", 24.0).with_color(fanta_doc::Color::rgb(0, 0, 0));
        let buf = TextBuffer::from_str("Hi", style);
        let box_w = 200.0;

        let rightmost_lit = |align: Align| -> Option<usize> {
            let layout = engine.layout_aligned(&buf, box_w, align);
            let (w, h) = (220i32, 40i32);
            let mut surface = surfaces::raster_n32_premul((w, h)).unwrap();
            surface.canvas().clear(skia_safe::Color::TRANSPARENT);
            layout.paint(surface.canvas(), [0.0, 0.0]);
            let info = ImageInfo::new((w, h), ColorType::RGBA8888, AlphaType::Unpremul, None);
            let row = info.min_row_bytes();
            let mut px = vec![0u8; row * h as usize];
            surface.read_pixels(&info, &mut px, row, (0, 0));
            // Find the largest x with any opaque pixel in any row.
            let mut max_x = None;
            for y in 0..h as usize {
                for x in 0..w as usize {
                    let i = y * row + x * 4;
                    if px[i + 3] != 0 {
                        max_x = Some(max_x.map_or(x, |m: usize| m.max(x)));
                    }
                }
            }
            max_x
        };

        let left = rightmost_lit(Align::Left).expect("left-aligned text drew");
        let right = rightmost_lit(Align::Right).expect("right-aligned text drew");
        assert!(
            right > left,
            "right-aligned glyphs ({right}) must sit further right than left-aligned ({left})"
        );
    }

    #[test]
    fn mixed_styles_lay_out_without_panicking() {
        use crate::style::FontWeight;
        // Bold prefix + larger colored suffix exercises multiple pushed runs.
        let engine = LayoutEngine::new();
        let mut buf = TextBuffer::from_str("Bold then big", body());
        buf.set_style(0..4, body().with_weight(FontWeight::Bold))
            .unwrap();
        buf.set_style(
            10..13,
            TextStyle::new("Helvetica", 32.0).with_color(fanta_doc::Color::rgb(200, 0, 0)),
        )
        .unwrap();
        let layout = engine.layout(&buf, 2000.0);
        assert!(layout.height() > 0.0);
        assert!(layout.width() > 0.0);
    }

    #[test]
    fn emoji_text_lays_out_via_fallback() {
        // The primary family won't have the crab glyph; the collection's
        // fallback manager must still shape it (non-zero width, no panic).
        let engine = LayoutEngine::new();
        let buf = TextBuffer::from_str("hi 🦀", body());
        let layout = engine.layout(&buf, 2000.0);
        assert!(layout.width() > 0.0);
        assert_eq!(layout.line_count(), 1);
    }
}
