use crate::{FnxError, color::expand_color_literals};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FssDeclaration {
    pub value: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub binding: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FssRule {
    pub selector: String,
    pub declarations: BTreeMap<String, FssDeclaration>,
}

pub fn parse_fss(source: &str) -> Result<Vec<FssRule>, FnxError> {
    let mut cursor = Cursor::new(source);
    let mut rules = Vec::new();
    let mut selectors = BTreeSet::new();
    while cursor.skip_trivia()? {
        cursor.expect(b'.')?;
        let selector = cursor.identifier()?.to_owned();
        let body = selector
            .strip_prefix("s_")
            .ok_or_else(|| cursor.error("selector must be .s_<ULID>"))?;
        if body.len() != 26
            || body
                .as_bytes()
                .first()
                .is_none_or(|byte| !matches!(byte, b'0'..=b'7'))
            || !body
                .bytes()
                .all(|byte| b"0123456789ABCDEFGHJKMNPQRSTVWXYZ".contains(&byte))
        {
            return Err(cursor.error("selector must contain a canonical 26-character ULID"));
        }
        if !selectors.insert(selector.clone()) {
            return Err(cursor.error("duplicate style selector"));
        }
        cursor.expect(b'{')?;
        let mut declarations = BTreeMap::new();
        loop {
            cursor.skip_trivia()?;
            if cursor.take(b'}') {
                break;
            }
            let property = cursor.identifier()?.to_owned();
            cursor.expect(b':')?;
            let expression = cursor.expression(b';')?;
            cursor.expect(b';')?;
            let declaration = parse_declaration(expression)?;
            if declarations.insert(property, declaration).is_some() {
                return Err(cursor.error("duplicate style property"));
            }
        }
        rules.push(FssRule {
            selector,
            declarations,
        });
    }
    Ok(rules)
}

fn parse_declaration(expression: &str) -> Result<FssDeclaration, FnxError> {
    let expression = expression.trim();
    if let Some(arguments) = expression
        .strip_prefix("bind(")
        .and_then(|value| value.strip_suffix(')'))
    {
        let mut cursor = Cursor::new(arguments);
        let reference = cursor.expression(b',')?;
        cursor.expect(b',')?;
        let binding: String = serde_json::from_str(reference.trim()).map_err(|error| {
            FnxError::Parse(format!(
                "binding must be a quoted variable reference: {error}"
            ))
        })?;
        let value = parse_literal(cursor.remaining())?;
        return Ok(FssDeclaration {
            value,
            binding: Some(binding),
        });
    }
    Ok(FssDeclaration {
        value: parse_literal(expression)?,
        binding: None,
    })
}

fn parse_literal(expression: &str) -> Result<Value, FnxError> {
    let expression = expression.trim();
    if expression.is_empty() {
        return Err(FnxError::Parse("style value is empty".into()));
    }
    if expression
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
        && expression
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphabetic)
        && !matches!(expression, "true" | "false" | "null")
    {
        return Ok(Value::String(expression.to_owned()));
    }
    serde_json::from_str(&expand_color_literals(expression))
        .map_err(|error| FnxError::Parse(format!("invalid FSS literal: {error}")))
}

/// Read static side-effect imports without evaluating the surrounding JavaScript.
pub fn stylesheet_imports(source: &str) -> Result<Vec<String>, FnxError> {
    let mut cursor = Cursor::new(source);
    let mut imports = Vec::new();
    while cursor.skip_trivia()? {
        let Some(byte) = cursor.peek() else {
            break;
        };
        if byte == b'<' {
            break;
        }
        if matches!(byte, b'\'' | b'"' | b'`') {
            cursor.string()?;
            continue;
        }
        if byte.is_ascii_alphabetic() || byte == b'_' {
            let identifier = cursor.identifier()?;
            if identifier == "import" {
                cursor.skip_trivia()?;
                if cursor.peek() == Some(b'(') {
                    return Err(cursor.error("dynamic imports are unsupported"));
                }
                if matches!(cursor.peek(), Some(b'\'' | b'"')) {
                    let import = cursor.string()?;
                    if import.ends_with(".fss") {
                        imports.push(import);
                    }
                } else {
                    let start = cursor.position;
                    while let Some(byte) = cursor.peek() {
                        if byte == b';' || byte == b'\n' {
                            break;
                        }
                        if matches!(byte, b'\'' | b'"') {
                            let path = cursor.string()?;
                            if path.ends_with(".fss") {
                                return Err(
                                    cursor.error("FSS imports must be static side-effect imports")
                                );
                            }
                        } else {
                            cursor.advance_char();
                        }
                    }
                    if cursor.position == start {
                        return Err(cursor.error("invalid import"));
                    }
                }
            }
        } else {
            cursor.advance_char();
        }
    }
    Ok(imports)
}

struct Cursor<'a> {
    source: &'a str,
    position: usize,
}

impl<'a> Cursor<'a> {
    fn new(source: &'a str) -> Self {
        Self {
            source,
            position: 0,
        }
    }
    fn remaining(&self) -> &'a str {
        self.source.get(self.position..).unwrap_or_default()
    }
    fn peek(&self) -> Option<u8> {
        self.source.as_bytes().get(self.position).copied()
    }
    fn advance_char(&mut self) {
        if let Some(character) = self.remaining().chars().next() {
            self.position += character.len_utf8();
        }
    }
    fn error(&self, message: &str) -> FnxError {
        FnxError::Parse(format!("{message} at byte {}", self.position))
    }
    fn skip_trivia(&mut self) -> Result<bool, FnxError> {
        loop {
            while self.peek().is_some_and(|byte| byte.is_ascii_whitespace()) {
                self.position += 1;
            }
            if self.remaining().starts_with("//") {
                while self.peek().is_some_and(|byte| byte != b'\n') {
                    self.advance_char();
                }
            } else if self.remaining().starts_with("/*") {
                let end = self
                    .remaining()
                    .find("*/")
                    .ok_or_else(|| self.error("unterminated comment"))?;
                self.position += end + 2;
            } else {
                return Ok(self.peek().is_some());
            }
        }
    }
    fn take(&mut self, byte: u8) -> bool {
        if self.peek() == Some(byte) {
            self.position += 1;
            true
        } else {
            false
        }
    }
    fn expect(&mut self, byte: u8) -> Result<(), FnxError> {
        self.skip_trivia()?;
        if self.take(byte) {
            Ok(())
        } else {
            Err(self.error(&format!("expected {}", char::from(byte))))
        }
    }
    fn identifier(&mut self) -> Result<&'a str, FnxError> {
        self.skip_trivia()?;
        let start = self.position;
        while self
            .peek()
            .is_some_and(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
        {
            self.position += 1;
        }
        if start == self.position {
            return Err(self.error("expected identifier"));
        }
        self.source
            .get(start..self.position)
            .ok_or_else(|| self.error("invalid identifier"))
    }
    fn string(&mut self) -> Result<String, FnxError> {
        let start = self.position;
        let quote = self.peek().ok_or_else(|| self.error("expected string"))?;
        self.position += 1;
        let mut result = String::new();
        loop {
            let character = self
                .remaining()
                .chars()
                .next()
                .ok_or_else(|| self.error("unterminated string"))?;
            self.position += character.len_utf8();
            if character as u32 == u32::from(quote) {
                if quote == b'"' {
                    return serde_json::from_str(
                        self.source
                            .get(start..self.position)
                            .ok_or_else(|| self.error("invalid string"))?,
                    )
                    .map_err(|error| self.error(&format!("invalid string: {error}")));
                }
                return Ok(result);
            }
            if character == '\\' {
                let escaped = self
                    .remaining()
                    .chars()
                    .next()
                    .ok_or_else(|| self.error("unterminated escape"))?;
                self.position += escaped.len_utf8();
                match escaped {
                    '\\' | '\'' | '"' | '`' => result.push(escaped),
                    _ if quote == b'"' => {}
                    _ => return Err(self.error("unsupported import string escape")),
                }
            } else if quote == b'`' && character == '$' && self.peek() == Some(b'{') {
                return Err(self.error("template expressions are unsupported"));
            } else {
                result.push(character);
            }
        }
    }
    fn expression(&mut self, delimiter: u8) -> Result<&'a str, FnxError> {
        self.skip_trivia()?;
        let start = self.position;
        let mut stack = Vec::new();
        while let Some(byte) = self.peek() {
            if byte == delimiter && stack.is_empty() {
                return self
                    .source
                    .get(start..self.position)
                    .ok_or_else(|| self.error("invalid value"));
            }
            match byte {
                b'"' => {
                    self.string()?;
                }
                b'{' => {
                    stack.push(b'}');
                    self.position += 1;
                }
                b'[' => {
                    stack.push(b']');
                    self.position += 1;
                }
                b'(' => {
                    stack.push(b')');
                    self.position += 1;
                }
                b'}' | b']' | b')' => {
                    if stack.pop() != Some(byte) {
                        return Err(self.error("unbalanced value"));
                    }
                    self.position += 1;
                }
                _ => self.advance_char(),
            }
        }
        Err(self.error("unterminated style value"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_structured_values_and_explicit_bindings() {
        let rules = parse_fss("/* hi */ .s_00000000000000000000000001 { fill: bind(\"$Theme/bg\", #AABBCC); layout: vertical; effects: []; }").expect("FSS");
        assert_eq!(
            rules[0].declarations["fill"].binding.as_deref(),
            Some("$Theme/bg")
        );
        assert_eq!(rules[0].declarations["fill"].value["a"], 255);
        assert_eq!(rules[0].declarations["layout"].value, "vertical");
    }
    #[test]
    fn imports_ignore_comments_and_strings() {
        assert_eq!(stylesheet_imports("// import \"wrong.fss\";\nconst x = \"import 'wrong.fss'\"; import \"../styles/main.fss\"; <Frame/>").expect("imports"), vec!["../styles/main.fss"]);
        assert!(stylesheet_imports("import styles from \"main.fss\"; <Frame/>").is_err());
    }
    #[test]
    fn rejects_duplicates_and_invalid_identity() {
        assert!(parse_fss(".button {fill: #FFFFFF;}").is_err());
        assert!(parse_fss(".s_00000000000000000000000001 {opacity: 1; opacity: 0;}").is_err());
        assert!(parse_fss(".s_ZZZZZZZZZZZZZZZZZZZZZZZZZZ {opacity: 1;}").is_err());
    }
}
