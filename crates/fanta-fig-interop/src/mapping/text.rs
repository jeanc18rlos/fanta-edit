//! TEXT node construction, font-weight parsing, and `textCase` transforms.

use super::{
    Fill, KiwiValue, NodeData, TextAlign, TextAutoResize, TextNode, TextStyle, TextStyleRun,
    VAlign, first_paint_fill, make_rect, read_line_height, read_number_px,
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

    if let Some(color) = fills.and_then(|f| match f {
        Fill::Solid { color } => Some(color),
        _ => None,
    }) {
        style.color = color;
    }

    apply_text_decoration(change, &mut style);

    if let Some(ls) = read_number_px(change.get("letterSpacing"), style.size_px) {
        style.letter_spacing = ls;
    }
    let line_height = read_line_height(change.get("lineHeight"), style.size_px);

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
    if let Some(lh) = line_height {
        style.line_height = normalize_node_line_height(lh, style.size_px, size.1);
    }
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

    NodeData::Text(TextNode {
        content,
        local_size: [size.0, size.1],
        align,
        vertical_align,
        style,
        style_runs,
        auto_resize,
    })
}

fn build_content_and_style_runs(
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

fn normalize_node_line_height(raw: f64, font_px: f64, box_h: f64) -> f64 {
    // Figma's "auto" / normal line height appears in exported `.fig` files as
    // RAW 1.0 for many Spectrum labels, while Dev Mode reports it as CSS
    // `normal` and the text box height already carries the real line box. A
    // literal 1.0 makes centered text sit too low/high, so infer the authored
    // one-line ratio when possible and otherwise use Source Sans' normal-ish
    // fallback.
    if (raw - 1.0).abs() > 1e-6 || font_px <= 0.0 {
        return raw;
    }
    let authored = box_h / font_px;
    if authored.is_finite() && (1.05..=1.5).contains(&authored) {
        authored
    } else {
        1.25
    }
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
    if let Some(color) = first_paint_fill(change.get("fillPaints")).and_then(|fill| match fill {
        Fill::Solid { color } => Some(color),
        _ => None,
    }) {
        style.color = color;
    }
    if let Some(ls) = read_number_px(change.get("letterSpacing"), style.size_px) {
        style.letter_spacing = ls;
    }
    if let Some(lh) = read_line_height(change.get("lineHeight"), style.size_px) {
        style.line_height = normalize_node_line_height(lh, style.size_px, 0.0);
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
