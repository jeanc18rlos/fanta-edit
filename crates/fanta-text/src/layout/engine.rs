//! The layout engine: owns the Skia font collection and turns a styled
//! [`TextBuffer`] into a [`TextLayout`].

use crate::buffer::TextBuffer;
use crate::font_resolver::{FontResolver, font_style};
use crate::layout::align::Align;
use crate::layout::text_layout::TextLayout;
use crate::style::TextStyle;
use skia_safe::{
    FontArguments, FontMgr, FontStyle, FourByteTag,
    font_arguments::{VariationPosition, variation_position::Coordinate},
    textlayout::{
        FontCollection, ParagraphBuilder, ParagraphStyle, PlaceholderStyle, TextDecoration,
        TextStyle as SkTextStyle,
    },
};

/// Paragraph-level layout controls beyond wrap width + alignment — the Figma
/// text-node properties that must be baked into the Skia `ParagraphStyle` /
/// builder before shaping. [`Default`] is "no limits, no truncation, no
/// indent", which reproduces [`LayoutEngine::layout_aligned`] exactly.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LayoutOptions {
    /// Lay out at most this many lines (Figma `maxLines`); text beyond them is
    /// dropped. `None` = unlimited.
    pub max_lines: Option<usize>,
    /// Truncate the last visible line with an ellipsis (`…`) when the text
    /// exceeds [`max_lines`](Self::max_lines) (Figma `textTruncation: ENDING`).
    /// Only meaningful together with a line limit — Skia applies the ellipsis
    /// when lines overflow `max_lines`.
    pub ellipsize: bool,
    /// First-line indent of each hard-break paragraph in logical pixels
    /// (Figma `paragraphIndent`). Realized as a zero-height inline placeholder
    /// at every paragraph start, so the indent participates in line breaking.
    /// NOTE: placeholders insert an object-replacement character into the Skia
    /// paragraph's internal text, so byte-offset queries (hit-test / caret) on
    /// an indented layout drift by 3 bytes per paragraph — acceptable for the
    /// rare imported indent, and irrelevant to painting.
    pub first_line_indent: f64,
}

/// Lays out [`TextBuffer`]s into [`TextLayout`]s, owning the Skia font
/// collection so font managers are set up once and reused.
///
/// Why a struct rather than a free function: building a `FontCollection` and
/// registering the system `FontMgr` on every layout call is wasteful (and the
/// font manager scan is not free). A long-lived engine amortizes that and is the
/// natural place to later add custom/asset fonts. The engine is cheap to clone
/// (the collection is reference-counted) but is intended to be held by the text
/// tool / render layer for the document's lifetime.
#[derive(Clone)]
pub struct LayoutEngine {
    fonts: FontCollection,
    resolver: FontResolver,
}

impl LayoutEngine {
    /// Construct an engine backed by the system font manager *plus* the bundled
    /// and resolver-managed faces.
    ///
    /// Two managers are installed on the collection:
    ///
    /// - **Asset manager** — the [`FontResolver`]'s `TypefaceFontProvider`,
    ///   carrying the bundled Inter + Source families and any font downloaded
    ///   this run. Skia probes this *before* the default manager, but it only
    ///   answers for the families it actually carries, so it never shadows a
    ///   genuinely-installed family of a different name.
    /// - **Default + fallback manager** — `FontMgr::default()`, the system font
    ///   set. It resolves every other family in a run's list (so a real installed
    ///   "Adobe Clean" still wins), and as the fallback manager it shapes glyphs
    ///   no listed family covers (emoji, CJK).
    ///
    /// Net behavior: the resolution chain in [`FontResolver::resolve_families`]
    /// — installed font if present, else a bundled/downloaded face for the
    /// requested family, else the Source substitute for a proprietary family,
    /// else the right generic-class system face — with system glyph fallback.
    pub fn new() -> Self {
        let resolver = FontResolver::new();
        let font_mgr = FontMgr::default();
        let mut fonts = FontCollection::new();
        // Resolver's asset provider: probed before the system manager, but only
        // answers for the families it carries, so it never shadows an installed
        // family of a different name.
        fonts.set_asset_font_manager(Some(resolver.provider().clone().into()));
        // The default manager resolves all other families and is also the
        // fallback manager for glyphs no listed family covers.
        fonts.set_default_font_manager(font_mgr, None);
        Self { fonts, resolver }
    }

    /// Lay out `buffer` wrapping at `max_width` pixels.
    ///
    /// Each [`crate::buffer::StyleRun`] becomes a pushed Skia text style over
    /// its text slice, so mixed styling (bold spans, colored words, size
    /// changes) lays out correctly in one paragraph. `max_width` of `f64::MAX`
    /// (or any very large value) effectively disables wrapping; finite widths
    /// soft-wrap on word boundaries the way Skia decides.
    ///
    /// An empty buffer still produces a valid (empty) [`TextLayout`] whose
    /// metrics are all zero — callers don't have to special-case it.
    ///
    /// Lines are left-aligned; use [`layout_aligned`](Self::layout_aligned) to
    /// pick a different horizontal alignment.
    pub fn layout(&self, buffer: &TextBuffer, max_width: f64) -> TextLayout {
        self.layout_aligned(buffer, max_width, Align::Left)
    }

    /// Lay out `buffer` wrapping at `max_width` pixels with the given horizontal
    /// [`Align`]ment.
    ///
    /// Identical to [`layout`](Self::layout) except the paragraph's lines are
    /// aligned within the wrap width per `align` — left, center, right, or
    /// justified. Alignment must be set on the `ParagraphStyle` *before* the
    /// paragraph is built (Skia bakes it into line layout), which is why it is a
    /// layout-time parameter rather than something the caller can flip after the
    /// fact. Centering and right-alignment are relative to `max_width`, so a
    /// node whose box is wider than its text shows the text shifted accordingly.
    pub fn layout_aligned(&self, buffer: &TextBuffer, max_width: f64, align: Align) -> TextLayout {
        self.layout_with(buffer, max_width, align, &LayoutOptions::default())
    }

    /// Lay out `buffer` like [`layout_aligned`](Self::layout_aligned), also
    /// honoring the paragraph-level [`LayoutOptions`] — line clamp, ellipsis
    /// truncation, and first-line paragraph indent. `LayoutOptions::default()`
    /// reproduces `layout_aligned` exactly.
    pub fn layout_with(
        &self,
        buffer: &TextBuffer,
        max_width: f64,
        align: Align,
        options: &LayoutOptions,
    ) -> TextLayout {
        let mut para_style = ParagraphStyle::new();
        para_style.set_text_align(align.to_sk());
        if let Some(limit) = options.max_lines {
            para_style.set_max_lines(Some(limit.max(1)));
            if options.ellipsize {
                para_style.set_ellipsis("…");
            }
        }
        let mut builder = ParagraphBuilder::new(&para_style, self.fonts.clone());
        // The paragraph indent, as a zero-height inline box at each paragraph
        // start (there is no first-line-indent on Skia's ParagraphStyle). Width
        // participates in line breaking exactly like a leading glyph.
        let indent = (options.first_line_indent > 0.0).then(|| {
            PlaceholderStyle::new(
                options.first_line_indent as f32,
                0.0,
                skia_safe::textlayout::PlaceholderAlignment::Baseline,
                skia_safe::textlayout::TextBaseline::Alphabetic,
                0.0,
            )
        });

        if buffer.is_empty() {
            // Push the default style so the builder is well-formed, add no text.
            let sk = self.to_sk_style(buffer.default_style());
            builder.push_style(&sk);
        } else {
            // Whether the next added character starts a paragraph (and so gets
            // the indent placeholder): at the very beginning and after each
            // hard newline.
            let mut at_paragraph_start = true;
            for run in buffer.runs() {
                let slice = &buffer.text()[run.start..run.end];
                let sk = self.to_sk_style(&run.style);
                builder.push_style(&sk);
                match &indent {
                    None => {
                        builder.add_text(slice);
                    }
                    Some(placeholder) => {
                        // Split the run at hard newlines so the placeholder can
                        // be inserted at every paragraph start within it. The
                        // newline itself is added with its piece, keeping the
                        // paragraph's own text byte-for-byte intact.
                        for piece in slice.split_inclusive('\n') {
                            if at_paragraph_start && !piece.is_empty() {
                                // Skia's addPlaceholder pushes/pops its own
                                // internal style, so the run's pushed style
                                // still applies to the following text.
                                builder.add_placeholder(placeholder);
                            }
                            builder.add_text(piece);
                            at_paragraph_start = piece.ends_with('\n');
                        }
                    }
                }
                builder.pop();
            }
        }

        let mut paragraph = builder.build();
        // Skia wants a finite f32; clamp absurd widths to f32::MAX so "no wrap"
        // is expressible without overflowing the cast.
        let w = if max_width.is_finite() {
            (max_width as f32).max(0.0)
        } else {
            f32::MAX
        };
        paragraph.layout(w);

        TextLayout {
            paragraph,
            text: buffer.text().to_string(),
            max_width,
        }
    }

    /// Translate one of our [`TextStyle`]s into a Skia `TextStyle`, honoring
    /// weight + italic and resolving the family through the [`FontResolver`].
    ///
    /// Weight and slant come straight from [`font_style_for`], so a 700 run is
    /// genuinely heavier than a 400 run and italic genuinely slants — even when
    /// the underlying face is a substitute. The family is set as the ordered
    /// *list* the resolver returns (requested name first, then bundled/
    /// downloaded/substitute/generic faces), so Skia's paragraph fallback paints
    /// with the requested font if installed and otherwise the first resolvable
    /// face down the chain. Glyph-level fallback (emoji/CJK) still flows through
    /// the registered `FontMgr`. The same list is used for measurement and
    /// paint, so geometry and pixels agree.
    ///
    /// ## Metric-compatible substitution
    ///
    /// When the requested family is one we substitute for a proprietary/renamed
    /// original (Adobe Clean / Source Sans Pro → bundled Source Sans 3, etc.),
    /// the open face we actually paint has slightly different advance widths than
    /// the original the `.fig` was laid out in — so text would over/under-flow
    /// the baked boxes. We correct for this exactly the way Figma does (a
    /// `size-adjust`-style metric match): the resolver hands back a per-family
    /// width ratio ([`FontResolver::metric_ratio_for`]), which we realize as
    /// **proportional letter-spacing** on the run. Letter-spacing is a width-only
    /// adjust (it changes total advance, not glyph cap-height), which the task
    /// requires — and the Skia paragraph `TextStyle` exposes no per-run `scale_x`.
    /// Because the adjusted `SkTextStyle` is the single style used for *both*
    /// line-breaking/measurement and painting, auto-layout widths and the painted
    /// glyphs stay in agreement. Families that render at their true metrics
    /// (installed exactly, bundled by their own name, or Google-Fonts-obtainable)
    /// get ratio `1.0` and so no extra spacing.
    fn to_sk_style(&self, style: &TextStyle) -> SkTextStyle {
        let mut sk = SkTextStyle::new();

        let font_style = font_style_for(style);
        sk.set_font_style(font_style);

        let families = self.resolver.resolve_families(&style.font_family);
        let refs: Vec<&str> = families.iter().map(String::as_str).collect();
        sk.set_font_families(&refs);

        sk.set_font_size(style.size_px as f32);

        // Metric-compatible substitution: widen/tighten the substitute to the
        // original's advance widths via proportional letter-spacing, on top of
        // the run's own tracking. See the method docs.
        let metric_spacing = metric_letter_spacing(
            self.resolver.metric_ratio_for(&style.font_family),
            style.size_px,
        );
        sk.set_letter_spacing((style.letter_spacing + metric_spacing) as f32);

        // `line_height` is a multiple of the font size; Skia's `set_height`
        // takes exactly that multiplier when height-override is enabled.
        // A metric-relative line height (`line_height_auto_percent`, Figma's
        // "auto" = 100%) is a percentage of the RESOLVED FACE'S intrinsic
        // ascent+descent+gap rather than of the em size, so it is converted to
        // the equivalent em multiplier through the face metrics; if the face
        // resolves no usable metrics we fall back to the scalar approximation
        // the doc layer carries alongside it.
        let height_multiplier = style
            .line_height_auto_percent
            .and_then(|percent| {
                self.metric_relative_height(&families, font_style, style.size_px, percent)
            })
            .unwrap_or(style.line_height);
        sk.set_height(height_multiplier as f32);
        sk.set_height_override(true);
        // Figma's (post-2019) line-height model is CSS half-leading: the extra
        // space beyond the font's intrinsic ascent+descent is split evenly
        // above and below the glyph box. Skia's default height override instead
        // scales ascent and descent proportionally, which pushes baselines
        // progressively lower as the line height grows — glyphs sat visibly low
        // inside fixed boxes compared to Figma. Line geometry (total height,
        // wrapping) is unchanged by this; only the baseline position within
        // each line moves.
        sk.set_half_leading(true);

        let c = style.color;
        let sk_color = skia_safe::Color::from_argb(c.a, c.r, c.g, c.b);
        sk.set_color(sk_color);

        let mut decoration = TextDecoration::NO_DECORATION;
        if style.underline {
            decoration |= TextDecoration::UNDERLINE;
        }
        if style.strikethrough {
            decoration |= TextDecoration::LINE_THROUGH;
        }
        if decoration != TextDecoration::NO_DECORATION {
            sk.set_decoration_type(decoration);
            sk.set_decoration_color(sk_color);
        }

        // Variable-font axes: apply the run's variation coordinates so a variable
        // face renders at the authored `wght`/`wdth`/`opsz`/… instead of only the
        // matched static weight. No axes ⇒ no font arguments (the default face).
        if !style.font_variations.is_empty() {
            let coordinates: Vec<Coordinate> = style
                .font_variations
                .iter()
                .map(|variation| Coordinate {
                    axis: FourByteTag::from(variation.axis_tag()),
                    value: variation.value,
                })
                .collect();
            let arguments = FontArguments::new().set_variation_design_position(VariationPosition {
                coordinates: &coordinates,
            });
            sk.set_font_arguments(&arguments);
        }

        sk
    }

    /// The `set_height` em-multiplier realizing a METRIC-RELATIVE line height:
    /// `percent`% of the resolved face's intrinsic line height (ascent +
    /// descent + line gap, from the face metrics at `size_px`), expressed as a
    /// multiple of `size_px` — i.e. Figma's `lineHeight` PERCENT units, whose
    /// `100` is the "auto" line height. `None` when the inputs are degenerate
    /// or no typeface resolves (the caller then falls back to the scalar
    /// multiplier), so a missing face degrades rather than laying out at a
    /// nonsense height.
    ///
    /// The typeface is looked up through the SAME font collection the paragraph
    /// shapes with (`find_typefaces` walks asset manager → system manager in
    /// the same order), so the metrics measured here belong to the face that
    /// actually paints. The collection handle is cloned for the lookup because
    /// `find_typefaces` needs `&mut` — a refcount bump, not a copy.
    fn metric_relative_height(
        &self,
        families: &[String],
        font_style: FontStyle,
        size_px: f64,
        percent: f64,
    ) -> Option<f64> {
        if size_px <= 0.0 || !size_px.is_finite() || !percent.is_finite() || percent < 0.0 {
            return None;
        }
        let typeface = self
            .fonts
            .clone()
            .find_typefaces(families, font_style)
            .into_iter()
            .next()?;
        let font = skia_safe::Font::new(typeface, Some(size_px as f32));
        let (_, metrics) = font.metrics();
        // `ascent` is negative (y-up from the baseline); `leading` is the
        // font's line gap.
        let intrinsic =
            f64::from(-metrics.ascent) + f64::from(metrics.descent) + f64::from(metrics.leading);
        if !intrinsic.is_finite() || intrinsic <= 0.0 {
            return None;
        }
        Some((percent / 100.0) * intrinsic / size_px)
    }
}

/// Per-glyph letter-spacing (logical px) that scales a run's total advance width
/// by `ratio`, used to realize a metric-compatible substitution as a width-only
/// adjust (see [`LayoutEngine::to_sk_style`]).
///
/// A line of *n* glyphs has total advance `Σ advance + (n-1)·spacing`. To scale
/// the advance sum by `ratio` we need `(n-1)·spacing ≈ (ratio-1)·Σ advance`.
/// Approximating each glyph's advance as a fixed fraction of the em
/// ([`MEAN_ADVANCE_PER_EM`]) makes the per-glyph spacing independent of *n*:
/// `spacing = (ratio - 1)·size_px·MEAN_ADVANCE_PER_EM`. This is the same coarse
/// average-advance model the doc-layer auto-width solver uses, so the two agree
/// on the adjusted width; the small one-glyph residual is absorbed by the
/// empirically-tuned ratio. `ratio == 1.0` (the common, unsubstituted case)
/// yields exactly `0.0`, so non-substituted runs are byte-for-byte unchanged.
fn metric_letter_spacing(ratio: f32, size_px: f64) -> f64 {
    if (ratio - 1.0).abs() < f32::EPSILON {
        return 0.0;
    }
    (f64::from(ratio) - 1.0) * size_px * MEAN_ADVANCE_PER_EM
}

/// Mean Latin glyph advance as a fraction of the em, used to convert a width
/// ratio into letter-spacing in [`metric_letter_spacing`]. Matches the
/// `AVG_ADVANCE` the doc-layer auto-layout solver uses for its Skia-free text
/// measurement, so the metric adjust and the solver size auto-width labels
/// consistently.
const MEAN_ADVANCE_PER_EM: f64 = 0.52;

/// Translate a [`TextStyle`]'s weight + italic into a Skia [`FontStyle`].
///
/// A thin wrapper over [`crate::font_resolver::font_style`] so the layout layer
/// and resolver agree on what a run's style maps to. Pure — it touches no font
/// manager — which lets tests assert the resolved weight/slant without
/// constructing an engine or depending on which faces happen to be installed.
pub(crate) fn font_style_for(style: &TextStyle) -> FontStyle {
    font_style(style.weight, style.italic)
}

impl Default for LayoutEngine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::style::{FontWeight, TextStyle};

    // -- font selection: weight, italic, resolved family --------------------

    use skia_safe::font_style::{Slant, Weight};

    #[test]
    fn weight_700_resolves_heavier_than_400() {
        // The whole point of honoring weight: a 700 run must produce a heavier
        // Skia font style than a 400 run. Asserting on the resolved FontStyle is
        // installed-font-independent (synthetic faces still carry the weight).
        let regular = font_style_for(&TextStyle::new("Inter", 16.0));
        let bold = font_style_for(&TextStyle::new("Inter", 16.0).with_weight(FontWeight::Bold));
        assert_eq!(regular.weight(), Weight::from(400));
        assert_eq!(bold.weight(), Weight::from(700));
        assert!(
            *bold.weight() > *regular.weight(),
            "bold ({:?}) must be heavier than regular ({:?})",
            bold.weight(),
            regular.weight()
        );
    }

    #[test]
    fn intermediate_numeric_weight_survives() {
        // Non-named weights (e.g. a 500 "medium" from a .fig) must pass through
        // numerically rather than snapping to 400/700.
        let mut s = TextStyle::new("Inter", 16.0);
        s.weight = 500;
        assert_eq!(font_style_for(&s).weight(), Weight::from(500));
    }

    #[test]
    fn italic_sets_the_slant() {
        let upright = font_style_for(&TextStyle::new("Inter", 16.0));
        let italic = font_style_for(&TextStyle::new("Inter", 16.0).with_italic(true));
        assert_eq!(upright.slant(), Slant::Upright);
        assert_eq!(italic.slant(), Slant::Italic);
    }

    #[test]
    fn explicit_line_height_distributes_as_half_leading() {
        // Figma's line-height model is CSS half-leading: the extra space beyond
        // the font's intrinsic ascent+descent splits evenly above and below the
        // glyph box. Consequence (face-independent): growing a 20px run's line
        // height from 2.0x (40px) to 4.0x (80px) moves the first baseline down
        // by exactly half the added 40px — 20px. Skia's default proportional
        // ascent/descent scaling (the old behavior) moves it by
        // `40·ascent/(ascent+descent)` ≈ 32px for Inter, sinking glyphs low in
        // fixed boxes. Bundled Inter keeps the numbers deterministic.
        let engine = LayoutEngine::new();
        let baseline_at = |line_height: f64| {
            let mut style = TextStyle::new("Inter", 20.0);
            style.line_height = line_height;
            let layout = engine.layout(&TextBuffer::from_str("Hg", style), 1.0e7);
            let lines = layout.lines();
            assert_eq!(lines.len(), 1);
            lines[0].baseline
        };
        let b2 = baseline_at(2.0);
        let b4 = baseline_at(4.0);
        assert!(
            ((b4 - b2) - 20.0).abs() < 1.5,
            "baseline must shift by half the added line height (half-leading); \
             got Δ={:.2} (proportional scaling would give ~32)",
            b4 - b2
        );
    }

    #[test]
    fn metric_relative_line_height_overrides_the_scalar() {
        // Figma's PERCENT line height ("auto" = 100%) is metric-relative: the
        // scalar multiplier must be IGNORED whenever the percent is present.
        let engine = LayoutEngine::new();
        let height_of = |scalar: f64, percent: Option<f64>| {
            let mut style = TextStyle::new("Inter", 20.0);
            style.line_height = scalar;
            style.line_height_auto_percent = percent;
            engine
                .layout(&TextBuffer::from_str("Hg", style), 1.0e7)
                .height()
        };
        let auto = height_of(1.0, Some(100.0));
        let auto_with_wild_scalar = height_of(3.0, Some(100.0));
        assert!(
            (auto - auto_with_wild_scalar).abs() < 1e-6,
            "the scalar must not leak into a metric-relative layout: {auto} vs {auto_with_wild_scalar}"
        );
        // 200% is (near-)exactly twice 100% — both percentages of the same
        // intrinsic metric height. Tolerance covers Skia per-line rounding.
        let double = height_of(1.0, Some(200.0));
        assert!(
            (double - 2.0 * auto).abs() < 1.5,
            "200% must be twice 100%: {double} vs 2×{auto}"
        );
        // Sanity: Inter's intrinsic ascent+descent+gap is a bit over one em —
        // the auto height is near (but not equal to) the em size, far from the
        // 3.0-em scalar it must be overriding.
        assert!(
            auto >= 20.0 && auto < 20.0 * 1.5,
            "auto line height should be ~1.2 em for Inter, got {auto}"
        );
    }

    #[test]
    fn layout_with_clamps_lines_and_truncates() {
        let engine = LayoutEngine::new();
        let buf = TextBuffer::from_str(
            "alpha beta gamma delta epsilon zeta eta theta iota kappa",
            TextStyle::new("Inter", 16.0),
        );
        let unclamped = engine.layout(&buf, 70.0);
        assert!(
            unclamped.line_count() > 2,
            "fixture must wrap past two lines, got {}",
            unclamped.line_count()
        );
        let clamped = engine.layout_with(
            &buf,
            70.0,
            Align::Left,
            &LayoutOptions {
                max_lines: Some(2),
                ellipsize: true,
                ..LayoutOptions::default()
            },
        );
        assert_eq!(clamped.line_count(), 2, "max_lines must clamp the layout");
        assert!(
            clamped.height() < unclamped.height(),
            "the clamped paragraph is shorter"
        );
        // Default options reproduce layout_aligned exactly.
        let default_options =
            engine.layout_with(&buf, 70.0, Align::Left, &LayoutOptions::default());
        assert_eq!(default_options.line_count(), unclamped.line_count());
        assert_eq!(default_options.height(), unclamped.height());
    }

    #[test]
    fn first_line_indent_insets_each_paragraph_start() {
        // The indent placeholder widens the FIRST line of every hard-break
        // paragraph by exactly the indent (single-line paragraphs here, so
        // every line is a first line).
        let engine = LayoutEngine::new();
        let buf = TextBuffer::from_str("Hello\nWorld", TextStyle::new("Inter", 16.0));
        let plain = engine.layout(&buf, 1.0e7);
        let indented = engine.layout_with(
            &buf,
            1.0e7,
            Align::Left,
            &LayoutOptions {
                first_line_indent: 24.0,
                ..LayoutOptions::default()
            },
        );
        let plain_widths: Vec<f64> = plain.lines().iter().map(|line| line.width).collect();
        let indented_widths: Vec<f64> = indented.lines().iter().map(|line| line.width).collect();
        assert_eq!(plain_widths.len(), 2);
        assert_eq!(indented_widths.len(), 2);
        for (plain_width, indented_width) in plain_widths.iter().zip(&indented_widths) {
            assert!(
                (indented_width - plain_width - 24.0).abs() < 1.5,
                "each paragraph's first line must widen by the indent: {plain_width} → {indented_width}"
            );
        }
    }

    #[test]
    fn sk_style_preserves_text_decorations() {
        let engine = LayoutEngine::new();
        let style = TextStyle::new("Helvetica", 16.0)
            .with_underline(true)
            .with_strikethrough(true);
        let sk = engine.to_sk_style(&style);
        let decoration = sk.decoration_type();
        assert!(decoration.contains(TextDecoration::UNDERLINE));
        assert!(decoration.contains(TextDecoration::LINE_THROUGH));
    }

    #[test]
    fn bold_run_measures_wider_than_regular_run() {
        // End-to-end fidelity check through actual shaping: the same text at 700
        // weight should occupy at least as much width as at 400 (bold glyphs are
        // never narrower). Uses a system family so a real bold face exists; the
        // assertion is `>=` because some faces synthesize bold without widening.
        let engine = LayoutEngine::new();
        let text = "Heading Sample";
        let regular = TextStyle::new("Helvetica", 40.0);
        let bold = TextStyle::new("Helvetica", 40.0).with_weight(FontWeight::Bold);
        let w_regular = engine
            .layout(&TextBuffer::from_str(text, regular), 1.0e7)
            .width();
        let w_bold = engine
            .layout(&TextBuffer::from_str(text, bold), 1.0e7)
            .width();
        assert!(w_regular > 0.0 && w_bold > 0.0);
        assert!(
            w_bold >= w_regular,
            "bold ({w_bold}) should be at least as wide as regular ({w_regular})"
        );
    }

    // -- resolved-family fidelity (the Adobe Clean → Source fix) -------------

    #[test]
    fn adobe_clean_shapes_as_metric_adjusted_source_sans_not_inter_not_helvetica() {
        // The load-bearing fidelity proof for the bug this work fixes: an
        // uninstalled "Adobe Clean" must shape with bundled *Source Sans 3*
        // (Adobe's open sans) — NOT Inter (the old blanket override) and NOT a
        // generic system sans like Helvetica. AND, because Adobe Clean is a
        // proprietary→open *substitution*, it must carry the metric-compatible
        // width adjust so the substitute approximates the original's advances:
        //   - "Adobe Clean" measures NARROWER than raw "Source Sans 3" (the
        //     canonical bundled name, which renders unadjusted), by ~the
        //     calibrated ratio, and
        //   - "Adobe Clean" differs from both "Inter" and installed "Helvetica".
        let engine = LayoutEngine::new();
        let text = "224 selected Edit Copy Delete View guidelines";
        let width = |fam: &str| {
            engine
                .layout(
                    &TextBuffer::from_str(text, TextStyle::new(fam, 18.0)),
                    1.0e7,
                )
                .width()
        };
        let adobe = width("Adobe Clean");
        let source = width(crate::font_resolver::SOURCE_SANS_FAMILY);
        let inter = width("Inter");
        let helvetica = width("Helvetica");
        assert!(adobe > 0.0 && source > 0.0 && inter > 0.0 && helvetica > 0.0);
        // The metric adjust tightens the substitute: Adobe Clean is narrower than
        // the unadjusted canonical Source Sans 3 — by a few percent, in the
        // direction the calibrated ratio (< 1.0) dictates, not by accident.
        assert!(
            adobe < source,
            "metric-adjusted Adobe Clean must be narrower than raw Source Sans 3: \
             Adobe Clean={adobe:.2} vs Source={source:.2}"
        );
        let observed = adobe / source;
        assert!(
            (0.90..0.995).contains(&observed),
            "Adobe Clean/Source width ratio {observed:.4} should reflect the \
             calibrated metric adjust (~{}), not be identical or wildly off",
            crate::font_resolver::ADOBE_CLEAN_SANS_METRIC_RATIO,
        );
        assert!(
            (adobe - inter).abs() > 0.5,
            "Adobe Clean must NOT shape as Inter (the old blanket override): \
             Adobe Clean={adobe:.2} vs Inter={inter:.2}"
        );
        assert!(
            (adobe - helvetica).abs() > 0.5,
            "Adobe Clean must differ from system Helvetica: \
             Adobe Clean={adobe:.2} vs Helvetica={helvetica:.2}"
        );
    }

    #[test]
    fn installed_family_still_wins_for_its_own_name() {
        // Exact-match invariant: a genuinely-installed family resolves to itself.
        // Helvetica (installed on macOS CI) must measure differently from the
        // Source substitute, proving it isn't being overridden.
        let engine = LayoutEngine::new();
        let text = "Sample Heading Text";
        let width = |fam: &str| {
            engine
                .layout(
                    &TextBuffer::from_str(text, TextStyle::new(fam, 24.0)),
                    1.0e7,
                )
                .width()
        };
        let helvetica = width("Helvetica");
        let source = width(crate::font_resolver::SOURCE_SANS_FAMILY);
        assert!(helvetica > 0.0 && source > 0.0);
        assert!(
            (helvetica - source).abs() > 0.5,
            "installed Helvetica must not be overridden by the Source substitute: \
             Helvetica={helvetica:.2} vs Source={source:.2}"
        );
    }

    #[test]
    fn measure_and_paint_share_the_resolved_face() {
        // Measurement and painting must use the *same* resolved family list, so
        // the painted glyphs occupy exactly the measured width. We measure
        // "Adobe Clean", then paint it into a surface sized to the measured
        // width: the rightmost lit column must land at (not past) that width.
        use skia_safe::{AlphaType, ColorType, ImageInfo, surfaces};
        let engine = LayoutEngine::new();
        let style = TextStyle::new("Adobe Clean", 24.0).with_color(fanta_doc::Color::rgb(0, 0, 0));
        let buf = TextBuffer::from_str("Action", style);
        let layout = engine.layout(&buf, 1.0e7);
        let measured = layout.width();
        assert!(measured > 0.0);

        let w = (measured.ceil() as i32) + 8;
        let h = 48;
        let mut surface = surfaces::raster_n32_premul((w, h)).unwrap();
        surface.canvas().clear(skia_safe::Color::TRANSPARENT);
        layout.paint(surface.canvas(), [0.0, 0.0]);
        let info = ImageInfo::new((w, h), ColorType::RGBA8888, AlphaType::Unpremul, None);
        let row = info.min_row_bytes();
        let mut px = vec![0u8; row * h as usize];
        assert!(surface.read_pixels(&info, &mut px, row, (0, 0)));
        let mut max_x = 0usize;
        for y in 0..h as usize {
            for x in 0..w as usize {
                if px[y * row + x * 4 + 3] != 0 {
                    max_x = max_x.max(x);
                }
            }
        }
        // Painted glyphs must fit within the measured width (allowing 1px of
        // antialiasing spill); if paint used a different face the ink would
        // overshoot or fall well short of the measured width.
        assert!(
            max_x as f64 <= measured + 1.0,
            "ink ({max_x}) overshot measured width ({measured:.2})"
        );
        assert!(
            (max_x as f64) > measured * 0.5,
            "ink ({max_x}) far short of measured width ({measured:.2}) — face mismatch?"
        );
    }

    #[test]
    fn serif_and_sans_runs_both_lay_out_nonempty() {
        // A serif-named family and a sans family must both shape to non-empty
        // geometry — i.e. the serif fallback chain resolves to a real face
        // rather than dropping to blank glyphs. (We can't assert "has serifs"
        // pixel-perfectly without shipping the font, so we assert the layout is
        // valid; the screenshot is the visual check per the build instructions.)
        let engine = LayoutEngine::new();
        let serif = TextStyle::new("Source Serif Pro", 32.0);
        let sans = TextStyle::new("Inter", 32.0);
        let ws = engine.layout(&TextBuffer::from_str("Title", serif), 1.0e7);
        let wn = engine.layout(&TextBuffer::from_str("Title", sans), 1.0e7);
        assert!(ws.width() > 0.0 && ws.height() > 0.0, "serif laid out");
        assert!(wn.width() > 0.0 && wn.height() > 0.0, "sans laid out");
    }

    #[test]
    fn source_serif_pro_headings_fit_figma_header_boxes() {
        // Spectrum's component frames bake these headers at 341–342×56 in Figma.
        // When Source Serif Pro is substituted by the bundled Source Serif 4,
        // the metric adjust must keep the headings on one line; otherwise the
        // title wraps and collides with the body copy below it.
        let engine = LayoutEngine::new();
        for (text, width) in [("Workflow Icons", 342.0), ("Scroll-Zoom Bar", 341.0)] {
            let style = TextStyle::new("Source Serif Pro", 45.0).with_weight(FontWeight::Bold);
            let unwrapped = engine.layout(&TextBuffer::from_str(text, style.clone()), 1.0e7);
            let layout = engine.layout(&TextBuffer::from_str(text, style), width);
            assert_eq!(
                layout.line_count(),
                1,
                "{text} should fit Figma's {width}px header box; wrapped_width={:.2}, unwrapped_width={:.2}, height={:.2}",
                layout.width(),
                unwrapped.width(),
                layout.height()
            );
        }
    }

    // -- metric-compatible substitution (the FONT-FIT width fix) --------------

    #[test]
    fn metric_letter_spacing_is_zero_for_unadjusted_and_signed_for_adjust() {
        // The unsubstituted path (ratio 1.0) must add exactly zero spacing, so a
        // true/obtainable family is byte-for-byte unchanged.
        assert_eq!(metric_letter_spacing(1.0, 14.0), 0.0);
        // A tightening ratio (< 1.0) yields negative spacing; a widening ratio
        // (> 1.0) yields positive; both scale with the font size.
        assert!(metric_letter_spacing(0.95, 14.0) < 0.0);
        assert!(metric_letter_spacing(1.05, 14.0) > 0.0);
        assert!(
            metric_letter_spacing(0.95, 28.0) < metric_letter_spacing(0.95, 14.0),
            "spacing scales with size (more negative at the larger size)"
        );
    }

    #[test]
    fn substituted_family_is_metric_adjusted_within_target_band() {
        // The core acceptance check the task asks for: a substituted-Adobe-Clean
        // string measures within ~X% of a target width because the metric adjust
        // is applied — while a non-substituted, true/obtainable family is NOT
        // adjusted (it shapes at its own raw width).
        //
        // Target band: the substituted "Adobe Clean" must land within 6% of the
        // calibrated fraction of the unadjusted bundled Source Sans 3 width. (We
        // calibrate against our own bundled face's width rather than a magic pixel
        // constant so the test is robust to Skia/face revisions; the *ratio* is
        // the thing under test.)
        let engine = LayoutEngine::new();
        let text = "224 selected";
        let width = |fam: &str, sz: f64| {
            engine
                .layout(&TextBuffer::from_str(text, TextStyle::new(fam, sz)), 1.0e7)
                .width()
        };

        // Unadjusted reference: the bundled family by its own canonical name gets
        // ratio 1.0 (it is not a substitution), so it shapes at true metrics.
        let source_raw = width(crate::font_resolver::SOURCE_SANS_FAMILY, 14.0);
        // Substituted + adjusted: both proprietary "Adobe Clean" and the renamed
        // "Source Sans Pro" route to the bundled Source successor *and* carry the
        // metric adjust.
        let adobe = width("Adobe Clean", 14.0);
        let source_pro = width("Source Sans Pro", 14.0);

        let ratio = f64::from(crate::font_resolver::ADOBE_CLEAN_SANS_METRIC_RATIO);
        let target = source_raw * ratio;
        let tol = target * 0.06; // ~within 6% of the metric-adjusted target width
        assert!(
            (adobe - target).abs() < tol,
            "substituted Adobe Clean ({adobe:.2}) must be within 6% of the \
             metric-adjusted target ({target:.2} = raw {source_raw:.2} * {ratio})"
        );
        // The two substituted spellings agree exactly (same substitute + ratio).
        assert!(
            (adobe - source_pro).abs() < 0.01,
            "Adobe Clean ({adobe:.2}) and Source Sans Pro ({source_pro:.2}) must \
             measure identically — same substitute, same adjust"
        );
        // And the adjust genuinely moved the width off the unadjusted reference.
        assert!(
            adobe < source_raw,
            "the adjust must tighten the substitute below the unadjusted face: \
             adobe={adobe:.2} raw={source_raw:.2}"
        );
    }

    #[test]
    fn substituted_serif_family_uses_serif_metric_adjust() {
        // "Adobe Clean Serif" contains the generic "Adobe Clean" substring, so
        // it must be checked before the sans branch in the substitution table.
        // Prove the end-to-end layout path uses the serif ratio, not the sans
        // one, by comparing against raw Source Serif 4.
        let engine = LayoutEngine::new();
        let text = "Darkest Theme";
        let width = |fam: &str| {
            engine
                .layout(
                    &TextBuffer::from_str(
                        text,
                        TextStyle::new(fam, 45.0).with_weight(FontWeight::Bold),
                    ),
                    1.0e7,
                )
                .width()
        };

        let source_raw = width(crate::font_resolver::SOURCE_SERIF_FAMILY);
        let adobe_serif = width("Adobe Clean Serif");
        let source_pro = width("Source Serif Pro");
        let serif_target =
            source_raw * f64::from(crate::font_resolver::ADOBE_CLEAN_SERIF_METRIC_RATIO);
        let sans_target =
            source_raw * f64::from(crate::font_resolver::ADOBE_CLEAN_SANS_METRIC_RATIO);

        assert!(
            (adobe_serif - serif_target).abs() < serif_target * 0.06,
            "Adobe Clean Serif ({adobe_serif:.2}) must follow the serif metric \
             target ({serif_target:.2}), not the sans one ({sans_target:.2})"
        );
        assert!(
            (adobe_serif - source_pro).abs() < 0.01,
            "Adobe Clean Serif ({adobe_serif:.2}) and Source Serif Pro \
             ({source_pro:.2}) must measure identically"
        );
        assert!(
            (adobe_serif - sans_target).abs() > source_raw * 0.01,
            "serif substitution should not accidentally reuse the sans ratio"
        );
    }

    #[test]
    fn obtainable_and_bundled_families_are_not_metric_adjusted() {
        // Requirement #3: only the PROPRIETARY→open substitutions are adjusted.
        // A bundled family referenced by its own canonical name (Inter, Source
        // Sans 3) and an installed system family (Helvetica) must shape at their
        // true metrics — i.e. identically with and without going through the
        // substitution map. We prove "no adjust" by checking the resolver hands
        // back ratio 1.0 for them, and that the adjusted-style width equals the
        // raw width the ratio would produce.
        let resolver = crate::font_resolver::FontResolver::new();
        for fam in [
            "Inter",
            crate::font_resolver::SOURCE_SANS_FAMILY,
            "Helvetica",
            "Roboto",
        ] {
            assert_eq!(
                resolver.metric_ratio_for(fam),
                1.0,
                "{fam} is obtainable/installed and must NOT be metric-adjusted"
            );
        }
        // Conversely, the proprietary/renamed names DO get an adjust (< 1.0).
        for fam in [
            "Adobe Clean",
            "Source Sans Pro",
            "Adobe Clean Serif",
            "Source Serif Pro",
        ] {
            assert!(
                resolver.metric_ratio_for(fam) < 1.0,
                "{fam} is a proprietary→open substitution and must be metric-adjusted"
            );
        }
    }
}
