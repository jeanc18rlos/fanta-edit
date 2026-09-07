//! SVG `d=` parsing: `PathData::from_svg_d`, its error type, and tokenizer.

use super::arc::append_svg_arc;
use super::{PathData, PathSegment};

impl PathData {
    /// Parse an SVG `d=` attribute string into a [`PathData`].
    ///
    /// Supports `M`/`m`, `L`/`l`, `H`/`h`, `V`/`v`, `Q`/`q`, `C`/`c`, `Z`/`z`,
    /// including repeated implicit commands (`M 0 0 1 1 2 2` ⇒ a Move then two
    /// Lines, per the SVG spec) and relative coordinates. The resulting
    /// [`PathData`] keeps the default [`FillRule`]; callers set `fill_rule`
    /// from the source's winding rule. Returns [`SvgPathError`] on malformed
    /// input rather than panicking.
    pub fn from_svg_d(d: &str) -> Result<Self, SvgPathError> {
        let mut path = PathData::new();
        let mut cur = [0.0_f64; 2]; // current point
        let mut start = [0.0_f64; 2]; // subpath start (for Z)
        let mut tokens = SvgTokenizer::new(d);
        let mut cmd: Option<char> = None;
        // For the smooth-shorthand commands: the previous segment's last control
        // point, so S/s reflects the prior cubic's ctrl2 and T/t the prior quad's
        // ctrl about the current point. `None` ⇒ no reflectable predecessor (the
        // reflected control coincides with the current point, per the SVG spec).
        let mut last_cubic_c2: Option<[f64; 2]> = None;
        let mut last_quad_c: Option<[f64; 2]> = None;
        // The previous command (uppercased) — S/T only reflect when it was a
        // matching curve (C/S resp. Q/T), else the control is the current point.
        let mut prev_upper = ' ';

        loop {
            // A command letter, or an implicit repeat of the previous command.
            let next = tokens.peek_command();
            match next {
                Some(c) => {
                    tokens.next_command();
                    cmd = Some(c);
                }
                None => {
                    if tokens.at_end() {
                        break;
                    }
                    // No letter and not at end ⇒ implicit repeat. A leading
                    // implicit `M` repeat is a `L` per the SVG spec.
                    match cmd {
                        Some('M') => cmd = Some('L'),
                        Some('m') => cmd = Some('l'),
                        Some(_) => {}
                        None => return Err(SvgPathError::ExpectedCommand),
                    }
                }
            }
            let Some(c) = cmd else {
                return Err(SvgPathError::ExpectedCommand);
            };
            let relative = c.is_ascii_lowercase();
            match c.to_ascii_uppercase() {
                'M' => {
                    let p = tokens.coord_pair(cur, relative)?;
                    cur = p;
                    start = p;
                    path.segments.push(PathSegment::Move { to: p });
                }
                'L' => {
                    let p = tokens.coord_pair(cur, relative)?;
                    cur = p;
                    path.segments.push(PathSegment::Line { to: p });
                }
                'H' => {
                    let x = tokens.number()?;
                    let to = [if relative { cur[0] + x } else { x }, cur[1]];
                    cur = to;
                    path.segments.push(PathSegment::Line { to });
                }
                'V' => {
                    let y = tokens.number()?;
                    let to = [cur[0], if relative { cur[1] + y } else { y }];
                    cur = to;
                    path.segments.push(PathSegment::Line { to });
                }
                'Q' => {
                    let ctrl = tokens.coord_pair(cur, relative)?;
                    let to = tokens.coord_pair(cur, relative)?;
                    cur = to;
                    path.segments.push(PathSegment::Quad { ctrl, to });
                    last_quad_c = Some(ctrl);
                    last_cubic_c2 = None;
                }
                'T' => {
                    // Smooth quadratic: ctrl is the reflection of the previous
                    // quad's ctrl about the current point (or the point itself).
                    let ctrl = match last_quad_c {
                        Some(c) if prev_upper == 'Q' || prev_upper == 'T' => {
                            [2.0 * cur[0] - c[0], 2.0 * cur[1] - c[1]]
                        }
                        _ => cur,
                    };
                    let to = tokens.coord_pair(cur, relative)?;
                    cur = to;
                    path.segments.push(PathSegment::Quad { ctrl, to });
                    last_quad_c = Some(ctrl);
                    last_cubic_c2 = None;
                }
                'C' => {
                    let ctrl1 = tokens.coord_pair(cur, relative)?;
                    let ctrl2 = tokens.coord_pair(cur, relative)?;
                    let to = tokens.coord_pair(cur, relative)?;
                    cur = to;
                    path.segments.push(PathSegment::Cubic { ctrl1, ctrl2, to });
                    last_cubic_c2 = Some(ctrl2);
                    last_quad_c = None;
                }
                'S' => {
                    // Smooth cubic: ctrl1 is the reflection of the previous cubic's
                    // ctrl2 about the current point (or the point itself).
                    let ctrl1 = match last_cubic_c2 {
                        Some(c) if prev_upper == 'C' || prev_upper == 'S' => {
                            [2.0 * cur[0] - c[0], 2.0 * cur[1] - c[1]]
                        }
                        _ => cur,
                    };
                    let ctrl2 = tokens.coord_pair(cur, relative)?;
                    let to = tokens.coord_pair(cur, relative)?;
                    cur = to;
                    path.segments.push(PathSegment::Cubic { ctrl1, ctrl2, to });
                    last_cubic_c2 = Some(ctrl2);
                    last_quad_c = None;
                }
                'A' => {
                    // rx ry x-axis-rotation large-arc-flag sweep-flag x y.
                    // Only the endpoint is relative for `a`; radii/flags are not.
                    let rx = tokens.number()?;
                    let ry = tokens.number()?;
                    let rot_deg = tokens.number()?;
                    let large_arc = tokens.number()? != 0.0;
                    let sweep = tokens.number()? != 0.0;
                    let to = tokens.coord_pair(cur, relative)?;
                    append_svg_arc(
                        &mut path,
                        cur,
                        rx,
                        ry,
                        rot_deg.to_radians(),
                        large_arc,
                        sweep,
                        to,
                    );
                    cur = to;
                }
                'Z' => {
                    cur = start;
                    path.segments.push(PathSegment::Close);
                }
                other => return Err(SvgPathError::UnsupportedCommand(other)),
            }
            prev_upper = c.to_ascii_uppercase();
        }
        Ok(path)
    }
}

/// Error returned by [`PathData::from_svg_d`] for malformed or unsupported
/// `d=` input. Importers should surface this as a skipped node, not a crash.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SvgPathError {
    #[error("expected a path command letter")]
    ExpectedCommand,
    #[error("expected a number in the path data")]
    ExpectedNumber,
    #[error("unsupported SVG path command '{0}' (M/L/H/V/Q/T/C/S/A/Z are handled)")]
    UnsupportedCommand(char),
}

/// Minimal tokenizer for the SVG path-data grammar subset we accept. Splits on
/// whitespace and commas, recognizes command letters, and parses floats. Kept
/// private — the public surface is [`PathData::from_svg_d`].
struct SvgTokenizer<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> SvgTokenizer<'a> {
    fn new(s: &'a str) -> Self {
        Self {
            bytes: s.as_bytes(),
            pos: 0,
        }
    }

    /// Skip whitespace and the single optional comma the SVG grammar allows
    /// between numbers.
    fn skip_separators(&mut self) {
        while self.pos < self.bytes.len() {
            match self.bytes[self.pos] {
                b' ' | b'\t' | b'\n' | b'\r' | b',' => self.pos += 1,
                _ => break,
            }
        }
    }

    fn at_end(&mut self) -> bool {
        self.skip_separators();
        self.pos >= self.bytes.len()
    }

    /// Look at the next non-separator byte; return it if it is a command letter.
    fn peek_command(&mut self) -> Option<char> {
        self.skip_separators();
        let b = *self.bytes.get(self.pos)?;
        let c = b as char;
        if c.is_ascii_alphabetic() && c != 'e' && c != 'E' {
            Some(c)
        } else {
            None
        }
    }

    fn next_command(&mut self) {
        self.pos += 1;
    }

    /// Parse one floating-point number (with optional sign, exponent, decimal).
    fn number(&mut self) -> Result<f64, SvgPathError> {
        self.skip_separators();
        let start = self.pos;
        let n = self.bytes.len();
        if self.pos < n && (self.bytes[self.pos] == b'+' || self.bytes[self.pos] == b'-') {
            self.pos += 1;
        }
        let mut seen_digit = false;
        while self.pos < n && self.bytes[self.pos].is_ascii_digit() {
            self.pos += 1;
            seen_digit = true;
        }
        if self.pos < n && self.bytes[self.pos] == b'.' {
            self.pos += 1;
            while self.pos < n && self.bytes[self.pos].is_ascii_digit() {
                self.pos += 1;
                seen_digit = true;
            }
        }
        // Exponent.
        if self.pos < n && (self.bytes[self.pos] == b'e' || self.bytes[self.pos] == b'E') {
            self.pos += 1;
            if self.pos < n && (self.bytes[self.pos] == b'+' || self.bytes[self.pos] == b'-') {
                self.pos += 1;
            }
            while self.pos < n && self.bytes[self.pos].is_ascii_digit() {
                self.pos += 1;
            }
        }
        if !seen_digit {
            return Err(SvgPathError::ExpectedNumber);
        }
        // `bytes` came from a `&str` and we only advanced over ASCII, so this
        // slice is valid UTF-8.
        let s = std::str::from_utf8(&self.bytes[start..self.pos])
            .map_err(|_| SvgPathError::ExpectedNumber)?;
        s.parse::<f64>().map_err(|_| SvgPathError::ExpectedNumber)
    }

    /// Parse an (x, y) pair, applying `cur` as the origin when `relative`.
    fn coord_pair(&mut self, cur: [f64; 2], relative: bool) -> Result<[f64; 2], SvgPathError> {
        let x = self.number()?;
        let y = self.number()?;
        Ok(if relative {
            [cur[0] + x, cur[1] + y]
        } else {
            [x, y]
        })
    }
}
