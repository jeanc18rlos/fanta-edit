//! Print an [`FnxElement`] tree as React/TSX-style `.fnx` source.
//!
//! Deterministic: attributes come from a `BTreeMap` (sorted keys), values are
//! rendered through `serde_json` (which itself sorts object keys), and indent
//! is fixed. Identical input ⇒ identical bytes, so git diffs stay clean.

use crate::model::FnxElement;
use crate::refs::RefTable;
use serde_json::Value;

const GENERATED_MARKER: &str = "// @generated fanta source — the design is the source of truth; ids live in the .ids sidecar\n";

/// Render a single-root element tree as one `.fnx` file. `name` becomes the
/// (cosmetic) function name; the authoritative name is the root's `name`
/// attribute, so the function name is free to be sanitized.
///
/// Equivalent to [`print_doc_with`] with an empty [`RefTable`]: references
/// print as raw ULIDs, exactly as the codec always behaved.
pub fn print_doc(name: &str, root: &FnxElement) -> String {
    print_doc_with(name, root, &RefTable::default())
}

/// [`print_doc`] with a name-emission context: when the table opted into
/// `emit_names`, `component` ids and binding variable ids re-sugar into their
/// unambiguous names / `$Collection/Name` paths (see [`crate::refs`]).
pub fn print_doc_with(name: &str, root: &FnxElement, refs: &RefTable) -> String {
    // Sugar a clone so the caller's tree is untouched: a generator-exact
    // rect/ellipse `Vector` prints as `<Rect>`/`<Ellipse>` and a
    // pure-translation `transform` prints as readable `x`/`y` (see
    // [`crate::sugar`]). Both re-sugar only when strictly lossless — as does
    // the reference sugar below (unambiguous names only, all-or-nothing per
    // bindings attribute).
    let mut root = root.clone();
    crate::sugar::sugar_shapes(&mut root);
    crate::sugar::sugar_transform(&mut root);
    crate::refs::sugar_refs(&mut root, refs);
    let mut out = String::with_capacity(256);
    out.push_str(crate::canonicalize::JSX_RUNTIME_PRAGMA);
    out.push('\n');
    out.push_str(crate::canonicalize::JSX_FACTORY_PRAGMA);
    out.push('\n');
    out.push_str(crate::canonicalize::FNX_TAG_IMPORT);
    out.push('\n');
    out.push_str(GENERATED_MARKER);
    out.push_str("export default function ");
    out.push_str(&fn_ident(name));
    out.push_str("() {\n  return (\n");
    print_element(&root, 2, &mut out);
    out.push_str("  );\n}\n");
    out
}

fn print_element(el: &FnxElement, depth: usize, out: &mut String) {
    let pad = "  ".repeat(depth);
    out.push_str(&pad);
    out.push_str(&render_open_tag(el));
    if el.children.is_empty() {
        out.push('\n');
        return;
    }
    out.push('\n');
    for child in &el.children {
        print_element(child, depth + 1, out);
    }
    out.push_str(&pad);
    out.push_str(&render_close_tag(el));
    out.push('\n');
}

pub(crate) fn render_open_tag(el: &FnxElement) -> String {
    let mut out = String::new();
    out.push('<');
    out.push_str(&el.tag);
    for (key, value) in &el.attrs {
        out.push(' ');
        out.push_str(key);
        out.push('=');
        out.push_str(&render_attr(value));
    }
    if el.children.is_empty() {
        out.push_str(" />");
    } else {
        out.push('>');
    }
    out
}

pub(crate) fn render_close_tag(el: &FnxElement) -> String {
    format!("</{}>", el.tag)
}

/// String attributes render as `"…"`; everything else as `{value}`, where the
/// value uses a JSON-ish grammar with one shorthand: a Color object renders as
/// `fnxColor("#RRGGBB[AA]")`. The helper-call spelling keeps generated source
/// valid TSX, allowing standard formatters and language tooling to parse it.
pub(crate) fn render_attr(value: &Value) -> String {
    if value.is_string() {
        return serde_json::to_string(value).unwrap_or_else(|_| "\"\"".to_owned());
    }
    format!("{{{}}}", render_value(value))
}

/// Recursively render an attribute value: Color objects become `fnxColor(...)`,
/// other objects/arrays recurse, scalars/strings go through `serde_json`.
fn render_value(value: &Value) -> String {
    match value {
        Value::Object(obj) => {
            if let Some(hex) = crate::color::color_obj_to_hex(obj) {
                return format!("fnxColor({hex:?})");
            }
            let parts: Vec<String> = obj
                .iter()
                .map(|(k, v)| {
                    format!(
                        "{}: {}",
                        serde_json::to_string(k).unwrap_or_else(|_| "\"\"".to_owned()),
                        render_value(v)
                    )
                })
                .collect();
            format!("{{{}}}", parts.join(", "))
        }
        Value::Array(arr) => {
            let parts: Vec<String> = arr.iter().map(render_value).collect();
            format!("[{}]", parts.join(", "))
        }
        scalar => serde_json::to_string(scalar).unwrap_or_else(|_| "null".to_owned()),
    }
}

/// A cosmetic, identifier-ish function name. Lossy on purpose — the real name
/// is the root's `name` attribute, so this never has to round-trip.
fn fn_ident(name: &str) -> String {
    let mut id: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    if id.is_empty() || id.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        id.insert(0, '_');
    }
    id
}
