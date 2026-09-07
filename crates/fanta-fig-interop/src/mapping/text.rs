//! TEXT node construction, font-weight parsing, and `textCase` transforms.

use super::{
    Fill, FontVariation, KiwiValue, LineHeight, NodeData, TextAlign, TextAutoResize, TextNode,
    TextStyle, TextStyleRun, VAlign, first_paint_fill, make_rect, read_line_height, read_number_px,
};

/// Build a [`NodeData::Text`] from a Figma TEXT `NodeChange`.
pub(crate) fn build_text(change: &KiwiValue, size: (f64, f64), fills: Option<Fill>) -> NodeData {
    let characters = change
        .get("textData")
        .and_then(|td| td.get("characters"))
        .and_then(KiwiValue::as_str);

    let Some(content) = characters else {
        return NodeData::Vector(make_rect(size, fills, None));
    };

    let mut style = TextStyle::default();
    apply_text_style_fields(change, &mut style);
    // An absent (or unreadable) `lineHeight` means Figma's default — "auto",
    // 100% of the font's intrinsic metric line height — NOT our 1.2 constant.
    // This default is deliberately NOT inside `apply_text_style_fields`: a
    // symbol-override entry that omits `lineHeight` must inherit the master's
    // value, not reset it to auto.
    if read_line_height(change.get("lineHeight"), style.size_px).is_none() {
        apply_line_height(&mut style, LineHeight::IntrinsicPercent(100.0));
    }

    if let Some(color) = fills.and_then(|f| match f {
        Fill::Solid { color, .. } => Some(color),
        _ => None,
    }) {
        style.color = color;
    }

    let align = match change
        .get("textAlignHorizontal")
        .and_then(KiwiValue::as_str)
    {
        Some("CENTER") => TextAlign::Center,
        Some("RIGHT") => TextAlign::Right,
        Some("JUSTIFIED") => TextAlign::Justify,
        _ => TextAlign::Left,
    };

    // `textAlignVertical` positions the paragraph block within the box height.
    // Centered button/cell/badge labels store CENTER here; without it the text
    // pins to the top of the box. Mirrors op2 `mapTextAlignVertical`.
    let vertical_align = match change.get("textAlignVertical").and_then(KiwiValue::as_str) {
        Some("CENTER") => VAlign::Center,
        Some("BOTTOM") => VAlign::Bottom,
        _ => VAlign::Top,
    };

    // `textAutoResize` decides whether the stored box width is a wrap boundary.
    // A Figma *auto-width* text node (`WIDTH_AND_HEIGHT`) hugs its content and
    // must NOT wrap; the renderer enforces that via `TextAutoResize`, so the doc
    // can keep the real authored box instead of inflating it to a giant no-wrap
    // width that would poison bounds/culling/export.
    let auto_resize = match change.get("textAutoResize").and_then(KiwiValue::as_str) {
        Some("WIDTH_AND_HEIGHT") => TextAutoResize::WidthAndHeight,
        Some("HEIGHT") => TextAutoResize::Height,
        _ => TextAutoResize::None,
    };
    // Gap D — `textCase` is a display transform Figma applies on top of the
    // stored characters (so ALL-CAPS nav/button labels store mixed-case but
    // render uppercase). Apply it to the string so the rendered glyphs match.
    // It's a TOP-LEVEL NodeChange field (verified on the Spectrum fixture: 130
    // UPPER labels at `change.textCase`, none nested under `textData`); we still
    // fall back to `textData.textCase` to be robust against schema variants.
    // Mirrors op2 `figma-text-mapper.ts` `applyTextCase`.
    let text_case = text_case_of(change);
    let (content, style_runs) =
        build_content_and_style_runs(content, text_case, change.get("textData"), &style);

    // Figma's "Truncate text" (`textTruncation: ENDING`) with an optional line
    // clamp (`maxLines`): the label ellipsizes instead of showing every wrapped
    // line. Carried on the doc TextNode; the text engine consumes it.
    let truncate = change.get("textTruncation").and_then(KiwiValue::as_str) == Some("ENDING");
    let max_lines = change
        .get("maxLines")
        .and_then(KiwiValue::as_f64)
        .filter(|n| n.is_finite() && *n >= 1.0)
        .map(|n| n as u32);
    let paragraph_spacing = change
        .get("paragraphSpacing")
        .and_then(KiwiValue::as_f64)
        .filter(|s| s.is_finite() && *s > 0.0)
        .unwrap_or(0.0);
    let paragraph_indent = change
        .get("paragraphIndent")
        .and_then(KiwiValue::as_f64)
        .filter(|s| s.is_finite() && *s > 0.0)
        .unwrap_or(0.0);

    NodeData::Text(TextNode {
        content,
        local_size: [size.0, size.1],
        align,
        vertical_align,
        style,
        style_runs,
        auto_resize,
        max_lines,
        truncate,
        paragraph_spacing,
        paragraph_indent,
    })
}

/// Apply the text-style SCALAR fields a `NodeChange` carries onto `style`,
/// leaving ABSENT fields untouched: `fontSize`, `fontName` (family / weight /
/// italic), `fontVariations`, `textDecoration`, `letterSpacing`, `lineHeight`.
///
/// Shared by [`build_text`] (base node styling) and the symbol-override Field
/// collection (OV-6 — an override entry carries these same scalars) so both
/// paths consume the same readers and can't drift. The absent-field semantics
/// are the point of the split: an override entry must INHERIT the master's
/// value for any field it doesn't carry, while `build_text` layers its own
/// "absent lineHeight = auto" default on top at the call site.
pub(crate) fn apply_text_style_fields(change: &KiwiValue, style: &mut TextStyle) {
    if let Some(fs) = change.get("fontSize").and_then(KiwiValue::as_f64) {
        if fs > 0.0 {
            style.size_px = fs;
        }
    }
    if let Some(font) = change.get("fontName") {
        if let Some(family) = font.get("family").and_then(KiwiValue::as_str) {
            if !family.is_empty() {
                style.font_family = family.to_owned();
            }
        }
        if let Some(fstyle) = font.get("style").and_then(KiwiValue::as_str) {
            let lower = fstyle.to_ascii_lowercase();
            // Gap B — full OpenType weight table from `fontName.style`. The old
            // `contains("bold") -> 700 else 400` collapsed Light/Medium/SemiBold/
            // Black etc. The layout engine already honors arbitrary numeric
            // weights. Mirrors op2 `figma-text-mapper.ts` `parseFontWeight`.
            style.weight = parse_font_weight(&lower).unwrap_or(400);
            style.italic = lower.contains("italic") || lower.contains("oblique");
        }
    }
    apply_font_variations(change, style);
    apply_text_decoration(change, style);
    // `letterSpacing` / `lineHeight` resolve PERCENT units against the font
    // size, so they read after `fontSize` has landed.
    if let Some(ls) = read_number_px(change.get("letterSpacing"), style.size_px) {
        style.letter_spacing = ls;
    }
    if let Some(lh) = read_line_height(change.get("lineHeight"), style.size_px) {
        apply_line_height(style, lh);
    }
}

/// Consume `fontVariations` (variable-font axis values). A `Weight` axis (tag
/// `'wght'`) overrides the weight parsed from the style NAME — Figma's own UI
/// kit sets body text to 450/550 via axes while the name stays "Medium"/etc.
fn apply_font_variations(change: &KiwiValue, style: &mut TextStyle) {
    let Some(variations) = change.get("fontVariations").and_then(KiwiValue::as_array) else {
        return;
    };
    for variation in variations {
        let Some(value) = variation.get("value").and_then(KiwiValue::as_f64) else {
            continue;
        };
        let Some(tag) = variation_axis_tag(variation) else {
            continue;
        };
        // Record EVERY axis so non-weight axes (wdth / opsz / slnt / custom)
        // actually render — the text engine applies them as variation
        // coordinates (previously only Weight was read, collapsing the rest).
        style
            .font_variations
            .push(FontVariation::new(tag.clone(), value as f32));
        // The Weight axis also drives the numeric `weight`, which the layout
        // engine uses to match a static instance when the face is not variable.
        if tag == "wght" && (1.0..=1000.0).contains(&value) {
            style.weight = value.round() as u16;
        }
    }
}

/// The 4-char OpenType axis tag for a Figma `fontVariations` entry: prefer the
/// numeric `axisTag` (a big-endian packed `u32`) unpacked to ASCII, else map a
/// known `axisName`. `None` when neither is recognized (the axis is skipped).
fn variation_axis_tag(variation: &KiwiValue) -> Option<String> {
    if let Some(packed) = variation.get("axisTag").and_then(KiwiValue::as_f64) {
        if packed.is_finite() && (0.0..=f64::from(u32::MAX)).contains(&packed) {
            let bytes = (packed as u32).to_be_bytes();
            if bytes.iter().all(|b| b.is_ascii_graphic() || *b == b' ') {
                return Some(String::from_utf8_lossy(&bytes).trim_end().to_string());
            }
        }
    }
    let tag = match variation.get("axisName").and_then(KiwiValue::as_str)? {
        "Weight" => "wght",
        "Width" => "wdth",
        "Optical Size" => "opsz",
        "Slant" => "slnt",
        "Italic" => "ital",
        _ => return None,
    };
    Some(tag.to_string())
}

/// Apply a decoded [`LineHeight`] onto the style: a multiple lands directly in
/// `line_height`; an intrinsic percentage (Figma "auto" = 100) is recorded in
/// `line_height_auto_percent` for metric-aware consumers, with `line_height`
/// holding a metric-free approximation (most UI faces' intrinsic line height is
/// ≈1.2× the em size).
fn apply_line_height(style: &mut TextStyle, line_height: LineHeight) {
    match line_height {
        LineHeight::Multiple(multiple) => {
            style.line_height = multiple;
            style.line_height_auto_percent = None;
        }
        LineHeight::IntrinsicPercent(percent) => {
            style.line_height = 1.2 * percent / 100.0;
            style.line_height_auto_percent = Some(percent);
        }
    }
}

/// Split `raw_content` into (case-transformed content, [`TextStyleRun`]s) from
/// `textData`'s `characterStyleIDs` + `styleOverrideTable`. Returns the plain
/// transformed string with no runs when the table is absent/empty or every run
/// resolves to the base style. Also reused by the symbol-override Field
/// collection (OV-7 — an override entry's `textData` carries the same per-run
/// tables), so instance text and master text share one run decoder.
pub(crate) fn build_content_and_style_runs(
    raw_content: &str,
    text_case: Option<&str>,
    text_data: Option<&KiwiValue>,
    base_style: &TextStyle,
) -> (String, Vec<TextStyleRun>) {
    let plain = || (apply_text_case(raw_content, text_case), Vec::new());

    let Some(text_data) = text_data else {
        return plain();
    };
    let Some(style_ids) = text_data
        .get("characterStyleIDs")
        .and_then(KiwiValue::as_array)
    else {
        return plain();
    };
    let Some(table) = text_data
        .get("styleOverrideTable")
        .and_then(KiwiValue::as_array)
    else {
        return plain();
    };
    if raw_content.is_empty() || style_ids.is_empty() || table.is_empty() {
        return plain();
    }

    let char_starts: Vec<usize> = raw_content
        .char_indices()
        .map(|(idx, _)| idx)
        .chain(std::iter::once(raw_content.len()))
        .collect();
    let char_count = char_starts.len().saturating_sub(1);
    if char_count == 0 {
        return plain();
    }

    let mut segments: Vec<(&str, TextStyle)> = Vec::new();
    let mut current_style_id = style_id_at(style_ids, 0);
    let mut seg_start_char = 0usize;
    for char_idx in 1..=char_count {
        let next_style_id = if char_idx < style_ids.len() {
            style_id_at(style_ids, char_idx)
        } else {
            -1
        };
        if next_style_id != current_style_id || char_idx == char_count {
            let start = char_starts[seg_start_char];
            let end = char_starts[char_idx];
            if start < end {
                let style = style_for_override_id(current_style_id, table, base_style);
                segments.push((&raw_content[start..end], style));
            }
            current_style_id = next_style_id;
            seg_start_char = char_idx;
        }
    }

    if segments.iter().all(|(_, style)| style == base_style) {
        return plain();
    }

    let mut content = String::new();
    let mut style_runs = Vec::new();
    for (segment, style) in segments {
        let transformed = apply_text_case(segment, text_case);
        let start = content.len();
        content.push_str(&transformed);
        let end = content.len();
        if start < end && style != *base_style {
            push_style_run(&mut style_runs, start, end, style);
        }
    }
    if style_runs.is_empty() {
        return plain();
    }
    (content, style_runs)
}

fn style_id_at(style_ids: &[KiwiValue], index: usize) -> i32 {
    style_ids
        .get(index)
        .and_then(KiwiValue::as_f64)
        .map(|id| id as i32)
        .unwrap_or(0)
}

fn style_for_override_id(style_id: i32, table: &[KiwiValue], base: &TextStyle) -> TextStyle {
    if style_id <= 0 {
        return base.clone();
    }
    let change = table.iter().find(|entry| {
        entry
            .get("styleID")
            .and_then(KiwiValue::as_f64)
            .is_some_and(|id| id as i32 == style_id)
    });
    let idx = style_id as usize;
    let Some(change) = change
        .or_else(|| table.get(idx))
        .or_else(|| table.get(idx.saturating_sub(1)))
    else {
        return base.clone();
    };

    let mut style = base.clone();
    if let Some(fs) = change.get("fontSize").and_then(KiwiValue::as_f64) {
        if fs > 0.0 {
            style.size_px = fs;
        }
    }
    if let Some(font) = change.get("fontName") {
        if let Some(family) = font.get("family").and_then(KiwiValue::as_str) {
            if !family.is_empty() {
                style.font_family = family.to_owned();
            }
        }
        if let Some(fstyle) = font.get("style").and_then(KiwiValue::as_str) {
            let lower = fstyle.to_ascii_lowercase();
            if let Some(weight) = parse_font_weight(&lower) {
                style.weight = weight;
            }
            style.italic = lower.contains("italic") || lower.contains("oblique");
        }
    }
    apply_font_variations(change, &mut style);
    if let Some(color) = first_paint_fill(change.get("fillPaints")).and_then(|fill| match fill {
        Fill::Solid { color, .. } => Some(color),
        _ => None,
    }) {
        style.color = color;
    }
    if let Some(ls) = read_number_px(change.get("letterSpacing"), style.size_px) {
        style.letter_spacing = ls;
    }
    // Absent lineHeight on an override entry inherits the base style's.
    if let Some(lh) = read_line_height(change.get("lineHeight"), style.size_px) {
        apply_line_height(&mut style, lh);
    }
    apply_text_decoration(change, &mut style);
    style
}

fn push_style_run(runs: &mut Vec<TextStyleRun>, start: usize, end: usize, style: TextStyle) {
    if let Some(last) = runs.last_mut() {
        if last.end == start && last.style == style {
            last.end = end;
            return;
        }
    }
    runs.push(TextStyleRun { start, end, style });
}

fn apply_text_decoration(change: &KiwiValue, style: &mut TextStyle) {
    match text_decoration_of(change) {
        Some("UNDERLINE") => {
            style.underline = true;
            style.strikethrough = false;
        }
        Some("STRIKETHROUGH") => {
            style.underline = false;
            style.strikethrough = true;
        }
        Some("NONE") => {
            style.underline = false;
            style.strikethrough = false;
        }
        _ => {}
    }
}

fn text_decoration_of(change: &KiwiValue) -> Option<&str> {
    change
        .get("textDecoration")
        .and_then(KiwiValue::as_str)
        .or_else(|| {
            change
                .get("textData")
                .and_then(|td| td.get("textDecoration"))
                .and_then(KiwiValue::as_str)
        })
}

/// Parse a lowercased `fontName.style` (e.g. "semibold", "light italic") into an
/// OpenType numeric weight (100–900), or `None` for a style with no recognizable
/// weight token (the caller defaults to 400/Regular). Order matters: longer
/// tokens that *contain* a shorter one are tested first ("extralight" before
/// "light", "extrabold"/"semibold" before "bold"). Mirrors op2's
/// `parseFontWeight`.
pub(crate) fn parse_font_weight(lower: &str) -> Option<u16> {
    if lower.contains("thin") || lower.contains("hairline") {
        Some(100)
    } else if lower.contains("extralight")
        || lower.contains("ultralight")
        || lower.contains("extra light")
        || lower.contains("ultra light")
    {
        Some(200)
    } else if lower.contains("light") {
        Some(300)
    } else if lower.contains("medium") {
        Some(500)
    } else if lower.contains("semibold")
        || lower.contains("demibold")
        || lower.contains("semi bold")
        || lower.contains("demi bold")
    {
        Some(600)
    } else if lower.contains("extrabold")
        || lower.contains("ultrabold")
        || lower.contains("extra bold")
        || lower.contains("ultra bold")
    {
        Some(800)
    } else if lower.contains("black") || lower.contains("heavy") {
        Some(900)
    } else if lower.contains("bold") {
        Some(700)
    } else if lower.contains("regular") || lower.contains("normal") {
        Some(400)
    } else {
        None
    }
}

/// The node's `textCase` enum: top-level `change.textCase` (where the real `.fig`
/// schema stores it), falling back to a nested `textData.textCase` for schema
/// variants. `None` when neither is present.
pub(crate) fn text_case_of(change: &KiwiValue) -> Option<&str> {
    change
        .get("textCase")
        .and_then(KiwiValue::as_str)
        .or_else(|| {
            change
                .get("textData")
                .and_then(|td| td.get("textCase"))
                .and_then(KiwiValue::as_str)
        })
}

/// Apply Figma's `textCase` display transform to a string. `UPPER`/`LOWER`
/// uppercase/lowercase; `TITLE` capitalizes the first letter of each word
/// (whitespace-delimited); `ORIGINAL`/absent/unknown leave the text unchanged.
/// Mirrors op2 `applyTextCase`.
pub(crate) fn apply_text_case(content: &str, text_case: Option<&str>) -> String {
    match text_case {
        Some("UPPER") => content.to_uppercase(),
        Some("LOWER") => content.to_lowercase(),
        Some("TITLE") => title_case(content),
        // ORIGINAL, SMALL_CAPS / SMALL_CAPS_FORCED (no glyph-substitution path
        // yet — leave the casing as authored), absent, or unknown.
        _ => content.to_owned(),
    }
}

/// Title-case each whitespace-delimited word: uppercase the first character of
/// every run that follows a word boundary, leave the rest untouched. Preserves
/// the original whitespace runs exactly. Matches op2's `\b\w` first-letter rule
/// closely enough for label text.
pub(crate) fn title_case(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut at_word_start = true;
    for ch in s.chars() {
        if ch.is_whitespace() {
            at_word_start = true;
            out.push(ch);
        } else if at_word_start {
            // First letter of a word → uppercase (may expand to multiple chars).
            for up in ch.to_uppercase() {
                out.push(up);
            }
            at_word_start = false;
        } else {
            out.push(ch);
        }
    }
    out
}
