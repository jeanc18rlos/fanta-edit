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
/// wrapper and any leading comment are ignored — parsing starts at the first
/// `<` (the only `<` before the root element is none).
///
/// Equivalent to [`parse_doc_with`] with an empty [`RefTable`]: name-based
/// references pass through untouched, exactly as the codec always behaved.
pub fn parse_doc(src: &str) -> Result<FnxElement, FnxError> {
    parse_doc_with(src, &RefTable::default())
}

/// [`parse_doc`] with a name-resolution context: `component="Button"` and
/// `$Collection/Name` binding paths desugar into their canonical ULIDs (see
/// [`crate::refs`]).
pub fn parse_doc_with(src: &str, refs: &RefTable) -> Result<FnxElement, FnxError> {
    let mut s = Scanner::new(src);
    s.skip_to_root()?;
    let mut root = s.element()?;
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

struct Scanner {
    chars: Vec<char>,
    pos: usize,
}

impl Scanner {
    fn new(src: &str) -> Self {
        Self {
            chars: src.chars().collect(),
            pos: 0,
        }
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
        let pos = pos.min(self.chars.len());
        let mut line = 1usize;
        let mut line_start = 0usize;
        for (i, &c) in self.chars[..pos].iter().enumerate() {
            if c == '\n' {
                line += 1;
                line_start = i + 1;
            }
        }
        let column = pos - line_start + 1;
        let line_end = self.chars[line_start..]
            .iter()
            .position(|&c| c == '\n')
            .map(|off| line_start + off)
            .unwrap_or(self.chars.len());

        let window_start = line_start.max(pos.saturating_sub(WINDOW));
        let window_end = line_end.min(pos + WINDOW);
        let prefix = if window_start > line_start {
            "… "
        } else {
            ""
        };
        let suffix = if window_end < line_end { " …" } else { "" };
        let excerpt: String = self.chars[window_start..window_end].iter().collect();
        let pad: String = std::iter::repeat_n(' ', prefix.chars().count())
            .chain(
                self.chars[window_start..pos]
                    .iter()
                    .map(|&c| if c == '\t' { '\t' } else { ' ' }),
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
        let lead_chars = {
            let lead_bytes = expanded.len() - expanded.trim_start().len();
            expanded[..lead_bytes].chars().count()
        };
        // (line, column) → char offset into the trimmed expanded text.
        let value_offset = trimmed
            .split('\n')
            .take(error.line().saturating_sub(1))
            .map(|l| l.chars().count() + 1)
            .sum::<usize>()
            + error.column().saturating_sub(1);
        let expanded_offset = lead_chars + value_offset;
        let first_diff = inner
            .chars()
            .zip(expanded.chars())
            .position(|(a, b)| a != b)
            .unwrap_or_else(|| inner.chars().count().min(expanded.chars().count()));
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

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn peek2(&self) -> Option<char> {
        self.chars.get(self.pos + 1).copied()
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek();
        if c.is_some() {
            self.pos += 1;
        }
        c
    }

    fn skip_ws(&mut self) {
        while let Some(c) = self.peek() {
            if c.is_whitespace() {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    /// Advance to the first `<` — the root element's open tag.
    fn skip_to_root(&mut self) -> Result<(), FnxError> {
        while let Some(c) = self.peek() {
            if c == '<' {
                return Ok(());
            }
            self.pos += 1;
        }
        Err(self.err("no element found"))
    }

    fn expect(&mut self, c: char) -> Result<(), FnxError> {
        if self.peek() == Some(c) {
            self.pos += 1;
            Ok(())
        } else {
            Err(self.err(format!("expected '{c}'")))
        }
    }

    /// `[A-Za-z0-9_]+` — covers PascalCase tags (incl. `Model3D`) and
    /// snake_case attribute keys.
    fn ident(&mut self) -> Result<String, FnxError> {
        let start = self.pos;
        while let Some(c) = self.peek() {
            if c.is_ascii_alphanumeric() || c == '_' {
                self.pos += 1;
            } else {
                break;
            }
        }
        if self.pos == start {
            return Err(self.err("expected identifier"));
        }
        Ok(self.chars[start..self.pos].iter().collect())
    }

    fn element(&mut self) -> Result<FnxElement, FnxError> {
        self.skip_ws();
        self.expect('<')?;
        let tag = self.ident()?;
        let mut el = FnxElement::new(tag.clone());
        loop {
            self.skip_ws();
            match self.peek() {
                Some('/') => {
                    self.expect('/')?;
                    self.expect('>')?;
                    return Ok(el);
                }
                Some('>') => {
                    self.expect('>')?;
                    break;
                }
                Some(_) => {
                    let key_start = self.pos;
                    let key = self.ident()?;
                    if el.attrs.contains_key(&key) {
                        return Err(self.err_at(key_start, format!("duplicate attribute {key}")));
                    }
                    self.skip_ws();
                    self.expect('=')?;
                    self.skip_ws();
                    let value = self.attr_value()?;
                    el.attrs.insert(key, value);
                }
                None => return Err(self.err("unterminated open tag")),
            }
        }
        // Children until the matching close tag.
        loop {
            self.skip_ws();
            if self.peek() == Some('<') && self.peek2() == Some('/') {
                self.expect('<')?;
                self.expect('/')?;
                let close = self.ident()?;
                if close != tag {
                    return Err(self.err(format!("</{close}> closing <{tag}>")));
                }
                self.skip_ws();
                self.expect('>')?;
                return Ok(el);
            }
            if self.peek().is_none() {
                return Err(self.err(format!("unterminated <{tag}>")));
            }
            el.children.push(self.element()?);
        }
    }

    fn attr_value(&mut self) -> Result<Value, FnxError> {
        match self.peek() {
            Some('"') => {
                let token_start = self.pos;
                let raw = self.json_string_token()?;
                serde_json::from_str(&raw)
                    .map_err(|e| self.err_at(token_start, format!("bad string: {e}")))
            }
            Some('{') => {
                let brace_pos = self.pos;
                // Expand `#RRGGBB[AA]` color literals back to JSON, then parse.
                let inner = self.braced()?;
                let expanded = crate::color::expand_color_literals(&inner);
                serde_json::from_str(expanded.trim())
                    .map_err(|e| self.json_value_err(brace_pos, &inner, &expanded, &e))
            }
            _ => Err(self.err("expected attribute value")),
        }
    }

    /// Capture a `"…"` token verbatim (including quotes), honoring `\"` escapes,
    /// so it can be handed to `serde_json` as a JSON string literal.
    fn json_string_token(&mut self) -> Result<String, FnxError> {
        let start = self.pos;
        self.expect('"')?;
        loop {
            match self.bump() {
                Some('\\') => {
                    self.bump();
                }
                Some('"') => break,
                Some(_) => {}
                None => return Err(self.err("unterminated string")),
            }
        }
        Ok(self.chars[start..self.pos].iter().collect())
    }

    /// Capture the text between a balanced `{ … }` (excluding the outer braces),
    /// tracking string literals so braces inside strings don't miscount.
    fn braced(&mut self) -> Result<String, FnxError> {
        self.expect('{')?;
        let start = self.pos;
        let mut depth = 1usize;
        let mut in_str = false;
        while let Some(c) = self.bump() {
            if in_str {
                match c {
                    '\\' => {
                        self.bump();
                    }
                    '"' => in_str = false,
                    _ => {}
                }
                continue;
            }
            match c {
                '"' => in_str = true,
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        let inner: String = self.chars[start..self.pos - 1].iter().collect();
                        return Ok(inner);
                    }
                }
                _ => {}
            }
        }
        Err(self.err("unterminated '{'"))
    }
}
