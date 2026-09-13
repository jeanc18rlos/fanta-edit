//! Conservative source upgrades for readable `.fnx` files.

use crate::model::is_known_tag;

pub(crate) const JSX_RUNTIME_PRAGMA: &str = "/** @jsxRuntime classic */";
pub(crate) const JSX_FACTORY_PRAGMA: &str = "/** @jsx fnxElement */";
pub(crate) const FNX_TAG_IMPORT: &str = "import { AiArtifact, Audio, Boolean, Ellipse, Embed, Frame, Image, Instance, Model3D, NodeGraph, Rect, Text, TextPath, Vector, Video } from \"../../fnx\";";

/// Import lines emitted by PREVIOUS printer versions, byte-exact. A file whose
/// import line still equals one of these upgrades to the current
/// [`FNX_TAG_IMPORT`] during canonicalization, so a hand-added sugar tag
/// (`<Rect>`) in an old file resolves in TypeScript tooling instead of erroring
/// as an unimported identifier. Byte equality is the deliberate gate: an import
/// the user reshaped (reordered, split, aliased) is theirs and stays untouched.
const LEGACY_TAG_IMPORTS: &[&str] = &[
    "import { AiArtifact, Audio, Boolean, Ellipse, Embed, Frame, Image, Instance, Model3D, NodeGraph, Rect, Text, Vector, Video } from \"../../fnx\";",
    "import { AiArtifact, Audio, Boolean, Embed, Frame, Image, Instance, Model3D, NodeGraph, Text, Vector, Video } from \"../../fnx\";",
];

const FNX_MODULE_PATH: &str = "../../fnx";
const GENERATED_MARKER: &str = "@generated fanta source";

#[derive(Default)]
struct Inspection {
    looks_like_fnx: bool,
    has_jsx_runtime_pragma: bool,
    has_jsx_factory_pragma: bool,
    has_fnx_tag_import: bool,
    color_edits: Vec<ColorEdit>,
}

struct ColorEdit {
    start: usize,
    end: usize,
    replacement: String,
}

/// Upgrade legacy generated `.fnx` source without parsing or reprinting it.
///
/// The pass adds missing JSX runtime declarations and rewrites legacy bare
/// `#RRGGBB` / `#RRGGBBAA` values only inside braced attributes of known FNX
/// tags. Strings, template literals, comments, JSX text, and ordinary
/// JavaScript remain byte-for-byte unchanged. Existing `fnxColor` calls and
/// runtime declarations are also preserved, making the operation idempotent.
///
/// Returns `None` when `source` is already canonical or does not look like an
/// FNX document. A returned string contains at least one migration.
pub fn canonicalize_legacy_source(source: &str) -> Option<String> {
    let inspection = inspect(source);
    if !inspection.looks_like_fnx {
        return None;
    }

    let missing_runtime = !inspection.has_jsx_runtime_pragma;
    let missing_factory = !inspection.has_jsx_factory_pragma;
    let missing_import = !inspection.has_fnx_tag_import;
    // A byte-exact previous-generation import line upgrades in place. Plain
    // substring search is sufficient: the needle is a complete import
    // statement, so a false positive would require that exact statement inside
    // a string or comment — and rewriting it there is still harmless noise,
    // not a semantic change.
    let legacy_import = LEGACY_TAG_IMPORTS
        .iter()
        .find(|line| source.contains(*line))
        .copied();
    if inspection.color_edits.is_empty()
        && !missing_runtime
        && !missing_factory
        && !missing_import
        && legacy_import.is_none()
    {
        return None;
    }

    let mut output = apply_color_edits(source, &inspection.color_edits);
    if let Some(legacy) = legacy_import {
        // Color edits never touch the import line (it contains no `#RRGGBB`),
        // so the byte-exact needle survives into `output`.
        output = output.replacen(legacy, FNX_TAG_IMPORT, 1);
    }
    if missing_runtime || missing_factory {
        let mut pragmas = String::new();
        if missing_runtime {
            pragmas.push_str(JSX_RUNTIME_PRAGMA);
            pragmas.push('\n');
        }
        if missing_factory {
            pragmas.push_str(JSX_FACTORY_PRAGMA);
            pragmas.push('\n');
        }
        output.insert_str(source_prefix_offset(&output), &pragmas);
    }
    if missing_import {
        let insertion = import_insertion_offset(&output);
        output.insert_str(insertion, &format!("{FNX_TAG_IMPORT}\n"));
    }
    Some(output)
}

fn inspect(source: &str) -> Inspection {
    let bytes = source.as_bytes();
    let mut inspection = Inspection::default();
    let mut cursor = 0;
    let mut previous_identifier = None;
    let mut in_import = false;
    let mut awaiting_import_modifier = false;
    let mut import_is_type_only = false;

    while cursor < bytes.len() {
        if let Some(end) = comment_end(bytes, cursor) {
            inspect_comment(&source[cursor..end], &mut inspection);
            cursor = end;
            continue;
        }

        match bytes[cursor] {
            b'\'' | b'"' => {
                let end = quoted_end(bytes, cursor, bytes[cursor]);
                if in_import
                    && !import_is_type_only
                    && matches!(previous_identifier, Some("import" | "from"))
                    && quoted_value(source, cursor, end) == Some(FNX_MODULE_PATH)
                {
                    inspection.has_fnx_tag_import = true;
                }
                in_import = false;
                awaiting_import_modifier = false;
                import_is_type_only = false;
                previous_identifier = None;
                cursor = end;
            }
            b'`' => {
                in_import = false;
                awaiting_import_modifier = false;
                import_is_type_only = false;
                previous_identifier = None;
                cursor = quoted_end(bytes, cursor, b'`');
            }
            b'<' => {
                if let Some(tag_end) = known_opening_tag_end(source, cursor) {
                    inspection.looks_like_fnx = true;
                    previous_identifier = None;
                    cursor = inspect_opening_tag(source, tag_end, &mut inspection.color_edits);
                } else {
                    previous_identifier = None;
                    cursor += 1;
                }
            }
            byte if is_identifier_start(byte) => {
                let start = cursor;
                cursor += 1;
                while cursor < bytes.len() && is_identifier_continue(bytes[cursor]) {
                    cursor += 1;
                }
                let identifier = &source[start..cursor];
                if identifier == "import" {
                    in_import = true;
                    awaiting_import_modifier = true;
                    import_is_type_only = false;
                } else if in_import && awaiting_import_modifier {
                    import_is_type_only = identifier == "type";
                    awaiting_import_modifier = false;
                }
                previous_identifier = Some(identifier);
            }
            byte if byte.is_ascii_whitespace() => cursor += 1,
            _ => {
                if awaiting_import_modifier {
                    awaiting_import_modifier = false;
                }
                if matches!(bytes[cursor], b'(' | b';') {
                    in_import = false;
                    import_is_type_only = false;
                }
                previous_identifier = None;
                cursor += 1;
            }
        }
    }
    inspection
}

fn inspect_comment(comment: &str, inspection: &mut Inspection) {
    if comment.contains("@jsxRuntime classic") {
        inspection.has_jsx_runtime_pragma = true;
    }
    if comment.contains("@jsx fnxElement") {
        inspection.has_jsx_factory_pragma = true;
    }
    if comment.contains(GENERATED_MARKER) {
        inspection.looks_like_fnx = true;
    }
}

fn inspect_opening_tag(source: &str, mut cursor: usize, edits: &mut Vec<ColorEdit>) -> usize {
    let bytes = source.as_bytes();
    while cursor < bytes.len() {
        if let Some(end) = comment_end(bytes, cursor) {
            cursor = end;
            continue;
        }
        match bytes[cursor] {
            b'\'' | b'"' | b'`' => cursor = quoted_end(bytes, cursor, bytes[cursor]),
            b'=' => {
                let mut value_start = cursor + 1;
                while value_start < bytes.len() && bytes[value_start].is_ascii_whitespace() {
                    value_start += 1;
                }
                if bytes.get(value_start) == Some(&b'{') {
                    let Some(close) = matching_brace(source, value_start) else {
                        return bytes.len();
                    };
                    collect_color_edits(source, value_start + 1, close, edits);
                    cursor = close + 1;
                } else {
                    cursor += 1;
                }
            }
            b'>' => return cursor + 1,
            _ => cursor += 1,
        }
    }
    cursor
}

fn known_opening_tag_end(source: &str, start: usize) -> Option<usize> {
    let bytes = source.as_bytes();
    let mut cursor = start.checked_add(1)?;
    if !bytes.get(cursor).copied().is_some_and(is_identifier_start) {
        return None;
    }
    cursor += 1;
    while bytes
        .get(cursor)
        .copied()
        .is_some_and(is_identifier_continue)
    {
        cursor += 1;
    }
    // Sugar tags (`Rect`/`Ellipse`) count as FNX opening tags too, so their
    // braced attributes get the same color canonicalization as canonical tags.
    is_known_tag(&source[start + 1..cursor]).then_some(cursor)
}

fn matching_brace(source: &str, opening: usize) -> Option<usize> {
    let bytes = source.as_bytes();
    let mut cursor = opening + 1;
    let mut depth = 1usize;
    while cursor < bytes.len() {
        if let Some(end) = comment_end(bytes, cursor) {
            cursor = end;
            continue;
        }
        match bytes[cursor] {
            b'\'' | b'"' | b'`' => cursor = quoted_end(bytes, cursor, bytes[cursor]),
            b'{' => {
                depth += 1;
                cursor += 1;
            }
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(cursor);
                }
                cursor += 1;
            }
            _ => cursor += 1,
        }
    }
    None
}

fn collect_color_edits(source: &str, start: usize, end: usize, edits: &mut Vec<ColorEdit>) {
    let bytes = source.as_bytes();
    let mut cursor = start;
    while cursor < end {
        if let Some(comment_end) = comment_end(bytes, cursor) {
            cursor = comment_end.min(end);
            continue;
        }
        match bytes[cursor] {
            b'\'' | b'"' | b'`' => cursor = quoted_end(bytes, cursor, bytes[cursor]).min(end),
            b'#' => {
                if let Some(color_end) = legacy_color_end(source, start, end, cursor) {
                    let hex = source[cursor + 1..color_end].to_ascii_uppercase();
                    edits.push(ColorEdit {
                        start: cursor,
                        end: color_end,
                        replacement: format!("fnxColor(\"#{hex}\")"),
                    });
                    cursor = color_end;
                } else {
                    cursor += 1;
                }
            }
            _ => cursor += 1,
        }
    }
}

fn legacy_color_end(
    source: &str,
    expression_start: usize,
    expression_end: usize,
    hash: usize,
) -> Option<usize> {
    let bytes = source.as_bytes();
    let mut end = hash + 1;
    while end < expression_end && bytes[end].is_ascii_hexdigit() {
        end += 1;
    }
    if !matches!(end - hash - 1, 6 | 8) {
        return None;
    }

    let before = source[expression_start..hash].chars().next_back();
    if before.is_some_and(|character| {
        !character.is_whitespace() && !matches!(character, '{' | '[' | '(' | ':' | ',' | '=' | '?')
    }) {
        return None;
    }
    let after = source[end..expression_end].chars().next();
    if after.is_some_and(|character| {
        !character.is_whitespace() && !matches!(character, '}' | ']' | ')' | ',' | ';' | ':' | '?')
    }) {
        return None;
    }
    Some(end)
}

fn apply_color_edits(source: &str, edits: &[ColorEdit]) -> String {
    let extra_capacity: usize = edits
        .iter()
        .map(|edit| edit.replacement.len().saturating_sub(edit.end - edit.start))
        .sum();
    let mut output = String::with_capacity(source.len() + extra_capacity);
    let mut cursor = 0;
    for edit in edits {
        output.push_str(&source[cursor..edit.start]);
        output.push_str(&edit.replacement);
        cursor = edit.end;
    }
    output.push_str(&source[cursor..]);
    output
}

fn source_prefix_offset(source: &str) -> usize {
    let bytes = source.as_bytes();
    let mut cursor = usize::from(bytes.starts_with(&[0xEF, 0xBB, 0xBF])) * 3;
    if bytes.get(cursor..cursor + 2) == Some(b"#!") {
        cursor = bytes[cursor..]
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(bytes.len(), |newline| cursor + newline + 1);
    }
    cursor
}

fn import_insertion_offset(source: &str) -> usize {
    let bytes = source.as_bytes();
    let mut cursor = source_prefix_offset(source);
    loop {
        while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        let Some(end) = comment_end(bytes, cursor) else {
            return cursor;
        };
        if source[cursor..end].contains(GENERATED_MARKER) {
            return cursor;
        }
        cursor = end;
    }
}

fn quoted_value(source: &str, start: usize, end: usize) -> Option<&str> {
    if end <= start + 1 || source.as_bytes().get(end - 1) != source.as_bytes().get(start) {
        return None;
    }
    Some(&source[start + 1..end - 1])
}

fn quoted_end(bytes: &[u8], start: usize, quote: u8) -> usize {
    let mut cursor = start + 1;
    while cursor < bytes.len() {
        if bytes[cursor] == b'\\' {
            cursor = (cursor + 2).min(bytes.len());
        } else if bytes[cursor] == quote {
            return cursor + 1;
        } else {
            cursor += 1;
        }
    }
    bytes.len()
}

fn comment_end(bytes: &[u8], start: usize) -> Option<usize> {
    match bytes.get(start..start + 2) {
        Some(b"//") => {
            let mut cursor = start + 2;
            while cursor < bytes.len() && !matches!(bytes[cursor], b'\n' | b'\r') {
                cursor += 1;
            }
            Some(cursor)
        }
        Some(b"/*") => {
            let mut cursor = start + 2;
            while cursor + 1 < bytes.len() {
                if bytes.get(cursor..cursor + 2) == Some(b"*/") {
                    return Some(cursor + 2);
                }
                cursor += 1;
            }
            Some(bytes.len())
        }
        _ => None,
    }
}

fn is_identifier_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || matches!(byte, b'_' | b'$')
}

fn is_identifier_continue(byte: u8) -> bool {
    is_identifier_start(byte) || byte.is_ascii_digit()
}
