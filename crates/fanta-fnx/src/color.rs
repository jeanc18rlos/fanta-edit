//! Color sugar: render a Color object (`{r,g,b,a}`) as a `#RRGGBB[AA]` literal in
//! `.fnx` source, and expand those literals back to JSON on parse. Lossless —
//! the only readable shorthand the codec applies, kept unambiguous because a
//! bare `#` never occurs in JSON outside a (quoted) string.

use serde_json::{Map, Value};

/// `#RRGGBB` (opaque) or `#RRGGBBAA` for an object that is exactly a Color
/// (`{r,g,b,a}`, every channel a `u8`); `None` for any other object.
pub(crate) fn color_obj_to_hex(obj: &Map<String, Value>) -> Option<String> {
    if obj.len() != 4 {
        return None;
    }
    let chan = |k: &str| {
        obj.get(k)
            .and_then(Value::as_u64)
            .filter(|n| *n <= 255)
            .map(|n| n as u8)
    };
    let (r, g, b, a) = (chan("r")?, chan("g")?, chan("b")?, chan("a")?);
    Some(if a == 255 {
        format!("#{r:02X}{g:02X}{b:02X}")
    } else {
        format!("#{r:02X}{g:02X}{b:02X}{a:02X}")
    })
}

/// The compact JSON of the Color object a `#RRGGBB`/`#RRGGBBAA` literal denotes
/// (case-insensitive), or `None` if `hex` isn't a 6/8-digit hex color.
fn hex_to_color_json(hex: &str) -> Option<String> {
    let h = hex.strip_prefix('#')?;
    let byte = |s: &str| u8::from_str_radix(s, 16).ok();
    let (r, g, b, a) = match h.len() {
        6 => (byte(&h[0..2])?, byte(&h[2..4])?, byte(&h[4..6])?, 255u8),
        8 => (
            byte(&h[0..2])?,
            byte(&h[2..4])?,
            byte(&h[4..6])?,
            byte(&h[6..8])?,
        ),
        _ => return None,
    };
    Some(format!("{{\"r\":{r},\"g\":{g},\"b\":{b},\"a\":{a}}}"))
}

/// Replace bare `#RRGGBB[AA]` color literals in an attribute-value expression
/// with their JSON Color objects, so the result is valid JSON. Hex inside a
/// quoted string is left untouched (a real string that happens to start `#`).
pub(crate) fn expand_color_literals(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    let mut in_str = false;
    while i < chars.len() {
        let c = chars[i];
        if in_str {
            out.push(c);
            if c == '\\' && i + 1 < chars.len() {
                out.push(chars[i + 1]);
                i += 2;
                continue;
            }
            if c == '"' {
                in_str = false;
            }
            i += 1;
            continue;
        }
        if c == '"' {
            in_str = true;
            out.push(c);
            i += 1;
            continue;
        }
        if c == '#' {
            let start = i + 1;
            let mut j = start;
            while j < chars.len() && chars[j].is_ascii_hexdigit() {
                j += 1;
            }
            let len = j - start;
            if len == 6 || len == 8 {
                let hex: String = std::iter::once('#')
                    .chain(chars[start..j].iter().copied())
                    .collect();
                if let Some(json) = hex_to_color_json(&hex) {
                    out.push_str(&json);
                    i = j;
                    continue;
                }
            }
        }
        out.push(c);
        i += 1;
    }
    out
}
