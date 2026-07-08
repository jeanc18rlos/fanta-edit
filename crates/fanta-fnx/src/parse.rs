//! Parse React/TSX-style `.fnx` source back into an [`FnxElement`] tree.
//!
//! A deliberately small grammar: the function wrapper is skipped (its name is
//! cosmetic), then one root element is parsed. An element is
//! `<Tag attr=… …/>` or `<Tag …>children</Tag>`; an attribute value is a JSON
//! string (`"…"`) or a braced JSON expression (`{…}`). All scalar/array/object
//! values go through `serde_json`, so this never re-implements JSON.

use crate::convert::FnxError;
use crate::model::FnxElement;
use serde_json::Value;

/// Parse a `.fnx` source string into its single root element. The function
/// wrapper and any leading comment are ignored — parsing starts at the first
/// `<` (the only `<` before the root element is none).
pub fn parse_doc(src: &str) -> Result<FnxElement, FnxError> {
    let mut s = Scanner::new(src);
    s.skip_to_root()?;
    let mut root = s.element()?;
    // Fold any `x`/`y` position sugar back into a `transform` array (the inverse
    // of the print-time sugar — see [`crate::sugar`]).
    crate::sugar::desugar_transform(&mut root);
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
        FnxError::Parse(format!("at char {}: {}", self.pos, msg.into()))
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
                    let key = self.ident()?;
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
                let raw = self.json_string_token()?;
                serde_json::from_str(&raw).map_err(|e| self.err(format!("bad string: {e}")))
            }
            Some('{') => {
                // Expand `#RRGGBB[AA]` color literals back to JSON, then parse.
                let inner = crate::color::expand_color_literals(&self.braced()?);
                serde_json::from_str(inner.trim()).map_err(|e| self.err(format!("bad value: {e}")))
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
