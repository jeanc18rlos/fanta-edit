//! Parse React/TSX-style `.fnx` source back into an [`FnxElement`] tree.
//!
//! A deliberately small grammar: the function wrapper is skipped (its name is
//! cosmetic), then one root element is parsed. An element is
//! `<Tag attr=… …/>` or `<Tag …>children</Tag>`; an attribute value is a JSON
//! string (`"…"`) or a braced JSON expression (`{…}`). All scalar/array/object
//! values go through `serde_json`, so this never re-implements JSON.

use crate::convert::FnxError;
use crate::model::FnxElement;
use crate::refs::RefTable;
use serde_json::Value;

/// Parse a `.fnx` source string into its single root element. The function
/// wrapper and its comments are ignored: parsing starts at the first `<`
/// outside a comment, and a second element after the root is an error.
///
/// Equivalent to [`parse_doc_with`] with an empty [`RefTable`]: name-based
/// references pass through untouched, exactly as the codec always behaved.
pub fn parse_doc(src: &str) -> Result<FnxElement, FnxError> {
    parse_doc_with(src, &RefTable::default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn byte_scanning_preserves_unicode_and_vertical_tab_whitespace() {
        let source = "<Frame\u{b}name=\"雪🙂\"\u{2003}>\u{85}{/* keep 雪🙂 */}\n<Text\u{b}content=\"héllo 雪🙂\" />\u{2003}</Frame>";
        let root = parse_doc(source).expect("existing whitespace grammar");
        assert_eq!(root.attrs.get("name"), Some(&Value::from("雪🙂")));
        assert_eq!(
            root.children
                .first()
                .and_then(|child| child.attrs.get("content")),
            Some(&Value::from("héllo 雪🙂"))
        );
    }

    fn nested_frames(count: usize) -> String {
        format!("{}{}", "<Frame>".repeat(count), "</Frame>".repeat(count))
    }

    #[test]
    fn the_deepest_allowed_page_parses_on_a_worker_thread_stack() {
        let parsed = std::thread::Builder::new()
            .stack_size(512 * 1024)
            .spawn(|| parse_doc(&nested_frames(MAX_DEPTH)).is_ok())
            .expect("spawn")
            .join()
            .expect("no stack overflow");
        assert!(parsed);
    }

    #[test]
    fn elements_nested_past_the_limit_are_refused() {
        let error = parse_doc(&nested_frames(MAX_DEPTH + 1)).expect_err("too deep");
        assert!(error.to_string().contains("nest deeper"), "{error}");
    }

    #[test]
    fn a_second_root_element_is_refused_instead_of_dropped() {
        let source = "export default function Home() {\n  return (\n    <Frame name=\"Hero\" />\n    <Frame name=\"Footer\" />\n  );\n}\n";
        let error = parse_doc(source).expect_err("two roots");
        assert!(error.to_string().contains("one root element"), "{error}");
        // The wrapper's closing text and JSX comments after the root are fine.
        let source = "export default function Home() {\n  return (\n    <Frame name=\"Hero\" />\n    {/* end */}\n  );\n}\n";
        assert!(parse_doc(source).is_ok());
    }

    #[test]
    fn a_tag_inside_a_wrapper_comment_is_not_the_root() {
        let source = "// e.g. <Frame name=\"Example\" />\n/* or <Text /> */\nexport default function Home() {\n  return (<Frame name=\"Real\" />);\n}\n";
        let root = parse_doc(source).expect("parse");
        assert_eq!(root.attrs.get("name"), Some(&Value::from("Real")));
    }

    #[test]
    fn borrowed_attribute_spans_preserve_large_json_payloads() {
        let values: Vec<_> = (0..4096)
            .map(|index| serde_json::json!({"position": [index, -0.25], "label": "雪"}))
            .collect();
        let payload = serde_json::to_string(&values).expect("payload JSON");
        let source = format!("<Vector name=\"Large geometry\" future_hint={{{payload}}} />");
        let root = parse_doc(&source).expect("borrowed payload parse");
        assert_eq!(root.attrs.get("future_hint"), Some(&Value::Array(values)));
    }

    #[test]
    fn json_diagnostics_map_byte_offsets_to_unicode_columns() {
        let source = r#"<Frame name="雪🙂" future_hint={{"label": "é", bad: 1}} />"#;
        let bad_key = source.find("bad:").expect("bad key");
        let column = source[..bad_key].chars().count() + 1;
        let error = parse_doc(source).expect_err("malformed JSON").to_string();
        assert!(
            error.contains(&format!("line 1, column {column}:")),
            "{error}"
        );
        assert!(error.contains("key must be a string"), "{error}");

        let source =
            r##"<Frame name="雪🙂" background={{"color": fnxColor("#FFFFFF"), bad: 1}} />"##;
        let brace = source.find("={{").expect("attribute brace") + 1;
        let column = source[..brace].chars().count() + 1;
        let error = parse_doc(source)
            .expect_err("error after color expansion")
            .to_string();
        assert!(
            error.contains(&format!("line 1, column {column}:")),
            "{error}"
        );
        assert!(error.contains("key must be a string"), "{error}");
    }
}

/// [`parse_doc`] with a name-resolution context: `component="Button"` and
/// `$Collection/Name` binding paths desugar into their canonical ULIDs (see
/// [`crate::refs`]).
pub fn parse_doc_with(src: &str, refs: &RefTable) -> Result<FnxElement, FnxError> {
    let mut s = Scanner::new(src);
    s.skip_to_root()?;
    let mut root = s.element(0)?;
    s.skip_child_trivia()?;
    if s.peek() == Some(b'<') {
        return Err(s.err(SECOND_ROOT_ERROR));
    }
    // Desugar `<Rect>`/`<Ellipse>` shape sugar into canonical `<Vector>`
    // geometry FIRST: it consumes `width`/`height` before desugar_size could
    // misread them (a sugar tag is not in its Frame/Vector table), and it
    // rewrites tags before anything downstream fingerprints them.
    crate::sugar::desugar_shapes(&mut root)?;
    // Fold any `x`/`y` position sugar back into a `transform` array (the inverse
    // of the print-time sugar — see [`crate::sugar`]). Errors when both an
    // explicit `transform` and `x`/`y` are present (silently dropping either
    // would lose rotation or position).
    crate::sugar::desugar_transform(&mut root)?;
    // Fold `width`/`height` sugar into `clip_size`/`local_size` — accepted on
    // parse only, for hand- and agent-authored sources.
    crate::sugar::desugar_size(&mut root);
    // Resolve name-based references LAST, so its errors address the final tag
    // shapes and no earlier pass ever observes a name.
    crate::refs::desugar_refs(&mut root, refs)?;
    Ok(root)
}

/// How deep elements may nest. Parsing and every later pass over the tree
/// recurse once per level, and projects load on 512 KiB worker-thread stacks,
/// where 256 levels overflow a debug build. Real designs nest about 20 levels.
pub(crate) const MAX_DEPTH: usize = 128;

pub(crate) const SECOND_ROOT_ERROR: &str =
    "a source holds one root element; wrap sibling layers in it";

pub(crate) fn too_deep() -> String {
    format!("elements nest deeper than {MAX_DEPTH} levels")
}

/// The position after any whitespace at `pos`: ASCII whitespace, vertical tab
/// and Unicode spaces such as U+00A0. The parser and the source scanner share
/// it, so a page that parses also opens.
pub(crate) fn skip_whitespace(source: &str, mut pos: usize) -> usize {
    while let Some(&byte) = source.as_bytes().get(pos) {
        if byte.is_ascii_whitespace() || byte == b'\x0b' {
            pos += 1;
        } else if !byte.is_ascii()
            && let Some(character) = source.get(pos..).and_then(|rest| rest.chars().next())
            && character.is_whitespace()
        {
            pos += character.len_utf8();
        } else {
            break;
        }
    }
    pos
}

/// The root element's `<`: the first one outside `//` and `/* */` comments
/// in the function wrapper.
pub(crate) fn root_start(source: &str) -> Option<usize> {
    let bytes = source.as_bytes();
    let mut pos = 0;
    while let Some(&byte) = bytes.get(pos) {
        match (byte, bytes.get(pos + 1)) {
            (b'<', _) => return Some(pos),
            (b'/', Some(b'/')) => {
                pos = source[pos..]
                    .find('\n')
                    .map_or(source.len(), |end| pos + end);
            }
            (b'/', Some(b'*')) => {
                pos = source[pos + 2..]
                    .find("*/")
                    .map_or(source.len(), |end| pos + 2 + end + 2);
            }
            _ => pos += 1,
        }
    }
    None
}

struct Scanner<'a> {
    source: &'a str,
    pos: usize,
}

impl<'a> Scanner<'a> {
    fn new(source: &'a str) -> Self {
        Self { source, pos: 0 }
    }

    fn err(&self, msg: impl Into<String>) -> FnxError {
        self.err_at(self.pos, msg)
    }

    /// A parse error anchored at `pos`, rendered author-first:
    ///
    /// ```text
    /// line 7, column 12: expected attribute value
    ///   <Text content="Hi" style=oops />
    ///                            ^
    /// ```
    ///
    /// The excerpt is windowed around the caret (printed `.fnx` puts a whole
    /// node on one line, so a raw line echo could be thousands of characters);
    /// caret padding mirrors tabs so alignment survives them.
    fn err_at(&self, pos: usize, msg: impl Into<String>) -> FnxError {
        const WINDOW: usize = 60;
        let mut pos = pos.min(self.source.len());
        while !self.source.is_char_boundary(pos) {
            pos -= 1;
        }
        let before = &self.source[..pos];
        let line = before.bytes().filter(|byte| *byte == b'\n').count() + 1;
        let line_start = before.rfind('\n').map_or(0, |position| position + 1);
        let line_end = self.source[pos..]
            .find('\n')
            .map_or(self.source.len(), |offset| pos + offset);
        let column = self.source[line_start..pos].chars().count() + 1;
        let window_start = self.source[line_start..pos]
            .char_indices()
            .rev()
            .nth(WINDOW - 1)
            .map_or(line_start, |(offset, _)| line_start + offset);
        let window_end = self.source[pos..line_end]
            .char_indices()
            .nth(WINDOW)
            .map_or(line_end, |(offset, _)| pos + offset);
        let prefix = if window_start > line_start {
            "… "
        } else {
            ""
        };
        let suffix = if window_end < line_end { " …" } else { "" };
        let excerpt = &self.source[window_start..window_end];
        let pad: String = std::iter::repeat_n(' ', prefix.chars().count())
            .chain(
                self.source[window_start..pos]
                    .chars()
                    .map(|character| if character == '\t' { '\t' } else { ' ' }),
            )
            .collect();
        FnxError::Parse(format!(
            "line {line}, column {column}: {}\n  {prefix}{excerpt}{suffix}\n  {pad}^",
            msg.into()
        ))
    }

    /// Anchor a `serde_json` failure inside a braced attribute value at its
    /// absolute source position. serde reports line/column relative to the
    /// (trimmed, color-expanded) value text; that maps exactly back to source
    /// as long as the error sits before the first `fnxColor(...)` expansion —
    /// past it, offsets have shifted, so the caret falls back to the `{`.
    fn json_value_err(
        &self,
        brace_pos: usize,
        inner: &str,
        expanded: &str,
        error: &serde_json::Error,
    ) -> FnxError {
        let message = {
            // serde's Display appends " at line L column C" in value-relative
            // coordinates; strip it — we anchor in source coordinates.
            let text = error.to_string();
            match text.rfind(" at line ") {
                Some(cut) => text[..cut].to_owned(),
                None => text,
            }
        };
        let trimmed = expanded.trim();
        let lead_bytes = expanded.len() - expanded.trim_start().len();
        // serde_json reports byte columns; convert to display columns only
        // when constructing the diagnostic, after mapping back into source.
        let value_offset = trimmed
            .split('\n')
            .take(error.line().saturating_sub(1))
            .map(|line| line.len() + 1)
            .sum::<usize>()
            + error.column().saturating_sub(1);
        let expanded_offset = lead_bytes + value_offset;
        let first_diff = inner
            .bytes()
            .zip(expanded.bytes())
            .position(|(a, b)| a != b)
            .unwrap_or_else(|| inner.len().min(expanded.len()));
        if expanded_offset <= first_diff {
            // +1 steps over the opening `{` the captured inner text excludes.
            self.err_at(
                brace_pos + 1 + expanded_offset,
                format!("bad value: {message}"),
            )
        } else {
            self.err_at(brace_pos, format!("bad value: {message}"))
        }
    }

    fn peek(&self) -> Option<u8> {
        self.source.as_bytes().get(self.pos).copied()
    }

    fn peek2(&self) -> Option<u8> {
        self.source.as_bytes().get(self.pos + 1).copied()
    }

    fn bump(&mut self) -> Option<u8> {
        let c = self.peek();
        if c.is_some() {
            self.pos += 1;
        }
        c
    }

    fn skip_ws(&mut self) {
        self.pos = skip_whitespace(self.source, self.pos);
    }

    /// Advance to the root element's open tag.
    fn skip_to_root(&mut self) -> Result<(), FnxError> {
        self.pos = root_start(self.source)
            .ok_or_else(|| self.err_at(self.source.len(), "no element found"))?;
        Ok(())
    }

    fn expect(&mut self, c: u8) -> Result<(), FnxError> {
        if self.peek() == Some(c) {
            self.pos += 1;
            Ok(())
        } else {
            Err(self.err(format!("expected '{}'", char::from(c))))
        }
    }

    /// `[A-Za-z0-9_]+` — covers PascalCase tags (incl. `Model3D`) and
    /// snake_case attribute keys.
    fn ident(&mut self) -> Result<String, FnxError> {
        let start = self.pos;
        while let Some(c) = self.peek() {
            if c.is_ascii_alphanumeric() || c == b'_' {
                self.pos += 1;
            } else {
                break;
            }
        }
        if self.pos == start {
            return Err(self.err("expected identifier"));
        }
        Ok(self.source[start..self.pos].to_owned())
    }

    /// Parse one element inside `depth` enclosing ones.
    fn element(&mut self, depth: usize) -> Result<FnxElement, FnxError> {
        if depth >= MAX_DEPTH {
            return Err(self.err(too_deep()));
        }
        self.skip_ws();
        self.expect(b'<')?;
        let tag = self.ident()?;
        let mut el = FnxElement::new(tag.clone());
        loop {
            self.skip_ws();
            match self.peek() {
                Some(b'/') => {
                    self.expect(b'/')?;
                    self.expect(b'>')?;
                    return Ok(el);
                }
                Some(b'>') => {
                    self.expect(b'>')?;
                    break;
                }
                Some(_) => {
                    let key_start = self.pos;
                    let key = self.ident()?;
                    if el.attrs.contains_key(&key) {
                        return Err(self.err_at(key_start, format!("duplicate attribute {key}")));
                    }
                    self.skip_ws();
                    self.expect(b'=')?;
                    self.skip_ws();
                    let value = self.attr_value()?;
                    el.attrs.insert(key, value);
                }
                None => return Err(self.err("unterminated open tag")),
            }
        }
        // Children until the matching close tag.
        loop {
            self.skip_child_trivia()?;
            if self.peek() == Some(b'<') && self.peek2() == Some(b'/') {
                self.expect(b'<')?;
                self.expect(b'/')?;
                let close = self.ident()?;
                if close != tag {
                    return Err(self.err(format!("</{close}> closing <{tag}>")));
                }
                self.skip_ws();
                self.expect(b'>')?;
                return Ok(el);
            }
            if self.peek().is_none() {
                return Err(self.err(format!("unterminated <{tag}>")));
            }
            el.children.push(self.element(depth + 1)?);
        }
    }

    fn skip_child_trivia(&mut self) -> Result<(), FnxError> {
        loop {
            self.skip_ws();
            if !self.source[self.pos..].starts_with("{/*") {
                return Ok(());
            }
            self.pos += 3;
            let Some(end) = self.source[self.pos..].find("*/}") else {
                return Err(self.err_at(self.source.len(), "unterminated JSX comment"));
            };
            self.pos += end + 3;
        }
    }

    fn attr_value(&mut self) -> Result<Value, FnxError> {
        match self.peek() {
            Some(b'"') => {
                let token_start = self.pos;
                let raw = self.json_string_token()?;
                serde_json::from_str(raw)
                    .map_err(|e| self.err_at(token_start, format!("bad string: {e}")))
            }
            Some(b'{') => {
                let brace_pos = self.pos;
                // Expand `#RRGGBB[AA]` color literals back to JSON, then parse.
                let inner = self.braced()?;
                let expanded = if inner.contains("fnxColor") || inner.contains('#') {
                    std::borrow::Cow::Owned(crate::color::expand_color_literals(inner))
                } else {
                    std::borrow::Cow::Borrowed(inner)
                };
                serde_json::from_str(expanded.trim())
                    .map_err(|e| self.json_value_err(brace_pos, inner, &expanded, &e))
            }
            _ => Err(self.err("expected attribute value")),
        }
    }

    /// Capture a `"…"` token verbatim (including quotes), honoring `\"` escapes,
    /// so it can be handed to `serde_json` as a JSON string literal.
    fn json_string_token(&mut self) -> Result<&'a str, FnxError> {
        let start = self.pos;
        self.expect(b'"')?;
        loop {
            match self.bump() {
                Some(b'\\') => {
                    self.bump();
                }
                Some(b'"') => break,
                Some(_) => {}
                None => return Err(self.err("unterminated string")),
            }
        }
        Ok(&self.source[start..self.pos])
    }

    /// Capture the text between a balanced `{ … }` (excluding the outer braces),
    /// tracking string literals so braces inside strings don't miscount.
    fn braced(&mut self) -> Result<&'a str, FnxError> {
        self.expect(b'{')?;
        let start = self.pos;
        let mut depth = 1usize;
        let mut in_str = false;
        while let Some(c) = self.bump() {
            if in_str {
                match c {
                    b'\\' => {
                        self.bump();
                    }
                    b'"' => in_str = false,
                    _ => {}
                }
                continue;
            }
            match c {
                b'"' => in_str = true,
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        return Ok(&self.source[start..self.pos - 1]);
                    }
                }
                _ => {}
            }
        }
        Err(self.err("unterminated '{'"))
    }
}
