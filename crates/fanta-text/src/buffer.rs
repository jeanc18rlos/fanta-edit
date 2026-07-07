//! [`TextBuffer`] — editable text content plus styled runs over byte ranges.
//!
//! The buffer is the source of truth for *what the text says* and *how each
//! span looks*. It is intentionally separate from layout (`layout.rs`): editing
//! must be cheap and synchronous on every keystroke, whereas layout is a
//! comparatively expensive shaping pass we only re-run when content changes.
//! Splitting them keeps a 5,000-character paragraph responsive while typing.
//!
//! ## Why byte ranges
//!
//! Runs are keyed by byte offset into the UTF-8 `String`, not by `char` index.
//! Byte offsets are what Skia's text layout consumes for hit-testing and rect
//! queries, and what `String` slicing uses natively, so byte ranges avoid an
//! O(n) char→byte translation on the layout hot path. The cost is that every
//! public mutation has to defend the UTF-8 invariant — an offset must land on a
//! `char` boundary — which this module does explicitly via [`TextError`].
//!
//! ## Run invariants (upheld after every mutation)
//!
//! 1. **Covering** — the runs tile `0..len` with no gaps. An empty buffer has
//!    no runs (there is nothing to cover); a non-empty buffer is fully covered.
//! 2. **Sorted & non-overlapping** — runs are ordered by `start`, and run *i*'s
//!    `end` equals run *i+1*'s `start`.
//! 3. **Merged** — no two adjacent runs share an identical [`TextStyle`]; they
//!    are coalesced so the run list stays minimal. This keeps `set_style`
//!    idempotent and the layout builder's `push_style` count bounded.

use crate::style::TextStyle;
use serde::{Deserialize, Serialize};
use std::ops::Range;

/// Errors from invalid buffer edits.
///
/// Every variant corresponds to a precondition the caller can check, so the
/// editor / tool layer can surface a precise reason rather than a generic
/// "edit failed". UTF-8 violations are the common case and get their own
/// variant carrying the offending byte index for diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum TextError {
    /// A byte index fell past the end of the text.
    #[error("byte index {index} is out of bounds (len {len})")]
    OutOfBounds { index: usize, len: usize },
    /// A byte index landed in the middle of a multi-byte UTF-8 character.
    /// Editing there would corrupt the string, so the op is rejected.
    #[error("byte index {index} is not on a UTF-8 character boundary")]
    NotCharBoundary { index: usize },
    /// A range had `start > end`.
    #[error("invalid range: start {start} > end {end}")]
    InvalidRange { start: usize, end: usize },
}

/// A contiguous span of text sharing one [`TextStyle`], addressed by byte range.
///
/// `start..end` is a half-open byte range into the owning [`TextBuffer`]'s
/// string. Kept as a flat struct (rather than `(Range, Style)`) so it can derive
/// `serde` cleanly for the `.fant.json` projection a future `TextNode` will emit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StyleRun {
    /// Inclusive byte offset where the run begins.
    pub start: usize,
    /// Exclusive byte offset where the run ends.
    pub end: usize,
    /// Formatting applied to every character in the run.
    pub style: TextStyle,
}

impl StyleRun {
    /// The half-open byte range this run covers.
    pub fn range(&self) -> Range<usize> {
        self.start..self.end
    }

    /// Number of bytes the run spans.
    pub fn len(&self) -> usize {
        self.end - self.start
    }

    /// Whether the run covers zero bytes. Zero-length runs are never retained
    /// in a normalized buffer; this predicate exists to detect and drop them.
    pub fn is_empty(&self) -> bool {
        self.start == self.end
    }
}

/// Editable styled text: a UTF-8 string plus a normalized list of [`StyleRun`]s.
///
/// Construct with [`TextBuffer::new`] (empty) or [`TextBuffer::from_str`]
/// (initial content in one style). Mutate with [`insert`](Self::insert),
/// [`delete_range`](Self::delete_range), and [`set_style`](Self::set_style);
/// each maintains the run invariants documented at the module level. Read with
/// [`text`](Self::text) and [`runs`](Self::runs).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextBuffer {
    text: String,
    runs: Vec<StyleRun>,
    /// Style applied to newly-inserted text when it cannot inherit from an
    /// existing neighbor (i.e. the very first insert into an empty buffer).
    /// Stored so an empty buffer still "remembers" what the next typed
    /// character should look like — the editor sets this from the active tool
    /// style before the user types.
    default_style: TextStyle,
}

impl TextBuffer {
    /// An empty buffer whose future typed text will use `TextStyle::default()`.
    pub fn new() -> Self {
        Self {
            text: String::new(),
            runs: Vec::new(),
            default_style: TextStyle::default(),
        }
    }

    /// A buffer pre-filled with `text`, entirely in `style`.
    ///
    /// The single all-covering run is the normalized representation of
    /// "uniformly styled text"; subsequent `set_style` calls split it as
    /// needed. `default_style` is seeded from `style` so text typed at the end
    /// continues in the same look.
    pub fn from_str(text: impl Into<String>, style: TextStyle) -> Self {
        let text = text.into();
        let runs = if text.is_empty() {
            Vec::new()
        } else {
            vec![StyleRun {
                start: 0,
                end: text.len(),
                style: style.clone(),
            }]
        };
        Self {
            text,
            runs,
            default_style: style,
        }
    }

    /// Total length of the text in bytes (not characters). Named `len` to match
    /// `String`/`str` convention; pair with [`is_empty`](Self::is_empty).
    pub fn len(&self) -> usize {
        self.text.len()
    }

    /// Whether the buffer holds no text.
    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// The full text content.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// The normalized style runs (sorted, gap-free, merged). For a non-empty
    /// buffer these tile `0..len`; for an empty buffer this is empty.
    pub fn runs(&self) -> &[StyleRun] {
        &self.runs
    }

    /// The style that the next insert into otherwise-empty space will use.
    /// Exposed so the editor can show "what will I type in" before any text
    /// exists, and set it from the active tool style.
    pub fn default_style(&self) -> &TextStyle {
        &self.default_style
    }

    /// Replace the style applied to future inserts that cannot inherit a
    /// neighbor. Does not retroactively restyle existing text.
    pub fn set_default_style(&mut self, style: TextStyle) {
        self.default_style = style;
    }

    /// Insert `s` at byte offset `byte_idx`.
    ///
    /// The inserted text inherits the style of the run it lands inside (or the
    /// run immediately to its left at a boundary, matching the universal editor
    /// behavior where typing continues the preceding character's style). An
    /// insert into an empty buffer uses [`default_style`](Self::default_style).
    ///
    /// Returns [`TextError`] without mutating if `byte_idx` is out of bounds or
    /// not on a `char` boundary. Inserting an empty string is a no-op.
    pub fn insert(&mut self, byte_idx: usize, s: &str) -> Result<(), TextError> {
        self.check_boundary(byte_idx)?;
        if s.is_empty() {
            return Ok(());
        }
        let n = s.len();

        // Choose the style the inserted span adopts, *before* we shift offsets.
        let inserted_style = self.style_for_insert(byte_idx);

        // Splice into the string.
        self.text.insert_str(byte_idx, s);

        // Shift every run boundary at or after the insertion point right by `n`.
        // A run that strictly contains the point (start < idx < end) is widened;
        // its start stays, its end moves. A boundary exactly at `byte_idx` is
        // treated as "belongs to the left run", so only offsets strictly greater
        // than the insertion start of the right neighbor move — handled below by
        // expanding the run whose range now contains the inserted bytes.
        for run in &mut self.runs {
            if run.start >= byte_idx {
                run.start += n;
            }
            if run.end >= byte_idx {
                // A run whose end == byte_idx grows only if the inserted text is
                // meant to extend it; we decide that by style match after the
                // shift. For now extend any run ending at/after the point.
                run.end += n;
            }
        }

        // After the uniform shift the inserted bytes `[byte_idx, byte_idx+n)`
        // are covered by whichever run now spans them (or none, if we inserted
        // at a seam or into an empty buffer). Reassert coverage with the chosen
        // style, then normalize so an identical neighbor merges back.
        self.stamp_style(byte_idx..byte_idx + n, inserted_style);
        self.normalize();
        Ok(())
    }

    /// Delete the text in `range` (byte offsets).
    ///
    /// Runs overlapping the range are trimmed; runs fully inside it are removed;
    /// runs after it shift left. Deleting an empty range is a no-op. Returns
    /// [`TextError`] without mutating on a malformed or non-boundary range.
    pub fn delete_range(&mut self, range: Range<usize>) -> Result<(), TextError> {
        let Range { start, end } = range;
        if start > end {
            return Err(TextError::InvalidRange { start, end });
        }
        self.check_boundary(start)?;
        self.check_boundary(end)?;
        if start == end {
            return Ok(());
        }
        let n = end - start;

        self.text.replace_range(start..end, "");

        // Rebuild runs: clip each to the surviving text, dropping empties.
        let mut rebuilt: Vec<StyleRun> = Vec::with_capacity(self.runs.len());
        for run in self.runs.drain(..) {
            // Clamp the run's endpoints around the deleted interval.
            let new_start = clamp_after_delete(run.start, start, end, n);
            let new_end = clamp_after_delete(run.end, start, end, n);
            if new_end > new_start {
                rebuilt.push(StyleRun {
                    start: new_start,
                    end: new_end,
                    style: run.style,
                });
            }
        }
        self.runs = rebuilt;
        self.normalize();
        Ok(())
    }

    /// Apply `style` to every character in `range`, splitting and merging runs
    /// as needed so the invariants hold afterward.
    ///
    /// This is the operation behind "select some text and make it bold". A
    /// partial overlap of an existing run splits that run into a styled middle
    /// and unstyled remainder(s); adjacent runs that end up identical are merged
    /// by [`normalize`](Self::normalize). An empty range is a no-op. Returns
    /// [`TextError`] on a malformed or non-boundary range.
    pub fn set_style(&mut self, range: Range<usize>, style: TextStyle) -> Result<(), TextError> {
        let Range { start, end } = range;
        if start > end {
            return Err(TextError::InvalidRange { start, end });
        }
        self.check_boundary(start)?;
        self.check_boundary(end)?;
        if start == end {
            return Ok(());
        }
        self.stamp_style(start..end, style);
        self.normalize();
        Ok(())
    }

    /// The style in effect at byte offset `byte_idx`.
    ///
    /// Returns the style of the run containing the offset. At a run seam the
    /// left run wins (consistent with [`insert`](Self::insert)). For an empty
    /// buffer, or an offset at the very end, returns the default style. Useful
    /// for the properties panel showing "the caret is in bold text".
    pub fn style_at(&self, byte_idx: usize) -> &TextStyle {
        for run in &self.runs {
            if byte_idx >= run.start && byte_idx < run.end {
                return &run.style;
            }
        }
        // At end-of-text or empty: continue the last run's style if present.
        self.runs
            .last()
            .map(|r| &r.style)
            .unwrap_or(&self.default_style)
    }

    // -- internals ---------------------------------------------------------

    /// Reject indices that are out of bounds or mid-`char`. `len` itself is a
    /// valid boundary (end-of-text insertions are legal).
    fn check_boundary(&self, idx: usize) -> Result<(), TextError> {
        if idx > self.text.len() {
            return Err(TextError::OutOfBounds {
                index: idx,
                len: self.text.len(),
            });
        }
        if !self.text.is_char_boundary(idx) {
            return Err(TextError::NotCharBoundary { index: idx });
        }
        Ok(())
    }

    /// Decide which style an insert at `byte_idx` adopts: the containing run, or
    /// the run ending exactly at `byte_idx` (left-bias), or the default style if
    /// no run applies (empty buffer / leading insert with no left neighbor).
    fn style_for_insert(&self, byte_idx: usize) -> TextStyle {
        // Prefer the run that strictly contains the point.
        for run in &self.runs {
            if byte_idx > run.start && byte_idx < run.end {
                return run.style.clone();
            }
        }
        // Then the run ending at the point (typing continues the left style).
        for run in &self.runs {
            if run.end == byte_idx {
                return run.style.clone();
            }
        }
        // Then the run starting at the point (insert at very beginning).
        for run in &self.runs {
            if run.start == byte_idx {
                return run.style.clone();
            }
        }
        self.default_style.clone()
    }

    /// Force `range` to be covered by exactly `style`, splitting any runs that
    /// straddle the range edges. Leaves the run list sorted but possibly with
    /// mergeable neighbors — callers must follow with [`normalize`](Self::normalize).
    ///
    /// Implemented as a rebuild rather than in-place surgery because the edge
    /// cases (range inside one run, range spanning many, range past the current
    /// run set after an insert) are far easier to get correct as a single linear
    /// pass than as a tangle of index arithmetic.
    fn stamp_style(&mut self, range: Range<usize>, style: TextStyle) {
        let Range { start, end } = range;
        let mut rebuilt: Vec<StyleRun> = Vec::with_capacity(self.runs.len() + 2);

        for run in self.runs.drain(..) {
            // Left remainder: the part of the run before the stamped range.
            if run.start < start {
                rebuilt.push(StyleRun {
                    start: run.start,
                    end: run.end.min(start),
                    style: run.style.clone(),
                });
            }
            // Right remainder: the part of the run after the stamped range.
            if run.end > end {
                rebuilt.push(StyleRun {
                    start: run.start.max(end),
                    end: run.end,
                    style: run.style.clone(),
                });
            }
            // The overlapping middle is dropped — it gets replaced by the
            // single stamped run pushed below.
        }

        // Insert the stamped run, then sort by start so it lands in order.
        rebuilt.push(StyleRun { start, end, style });
        rebuilt.retain(|r| !r.is_empty());
        rebuilt.sort_by_key(|r| r.start);
        self.runs = rebuilt;
    }

    /// Restore the invariants: drop empties, merge adjacent identical-style
    /// runs, and assert full coverage of `0..len` for non-empty text.
    ///
    /// This is the single chokepoint that every mutating method funnels
    /// through, which is why those methods can be written for clarity rather
    /// than for maintaining the invariants inline.
    fn normalize(&mut self) {
        self.runs.retain(|r| !r.is_empty());
        self.runs.sort_by_key(|r| r.start);

        // Merge adjacent runs that touch and share a style.
        let mut merged: Vec<StyleRun> = Vec::with_capacity(self.runs.len());
        for run in self.runs.drain(..) {
            if let Some(last) = merged.last_mut() {
                if last.end == run.start && last.style == run.style {
                    last.end = run.end;
                    continue;
                }
            }
            merged.push(run);
        }
        self.runs = merged;

        // Guarantee coverage. After well-behaved edits the runs already tile
        // `0..len`; this backstop fills any seam left by an insert at a gap
        // (e.g. first character into an empty buffer) so callers can rely on
        // total coverage unconditionally.
        if self.text.is_empty() {
            self.runs.clear();
            return;
        }
        self.fill_gaps();
    }

    /// Patch any uncovered byte spans with the nearest available style so the
    /// runs fully tile `0..len`. Gaps only arise transiently from an insert at
    /// a seam; this keeps the public invariant absolute.
    fn fill_gaps(&mut self) {
        let len = self.text.len();
        let mut filled: Vec<StyleRun> = Vec::with_capacity(self.runs.len() + 1);
        let mut cursor = 0usize;
        let fallback = self.default_style.clone();

        for run in self.runs.drain(..) {
            if run.start > cursor {
                // Uncovered span [cursor, run.start): adopt this run's style so
                // the gap blends into the text that follows it.
                filled.push(StyleRun {
                    start: cursor,
                    end: run.start,
                    style: run.style.clone(),
                });
            }
            cursor = cursor.max(run.end);
            filled.push(run);
        }
        if cursor < len {
            // Trailing gap: adopt the previous run's style, else default.
            let style = filled.last().map(|r| r.style.clone()).unwrap_or(fallback);
            filled.push(StyleRun {
                start: cursor,
                end: len,
                style,
            });
        }

        // The gap-fill above can produce adjacent identical runs again; do one
        // more merge pass so the result is fully normalized.
        let mut merged: Vec<StyleRun> = Vec::with_capacity(filled.len());
        for run in filled {
            if let Some(last) = merged.last_mut() {
                if last.end == run.start && last.style == run.style {
                    last.end = run.end;
                    continue;
                }
            }
            merged.push(run);
        }
        self.runs = merged;
    }
}

impl Default for TextBuffer {
    fn default() -> Self {
        Self::new()
    }
}

/// Map a byte offset through a deletion of `[del_start, del_end)` (length `n`).
/// Offsets before the cut are unchanged; offsets inside it collapse to the cut
/// start; offsets after it shift left by `n`.
fn clamp_after_delete(offset: usize, del_start: usize, del_end: usize, n: usize) -> usize {
    if offset <= del_start {
        offset
    } else if offset >= del_end {
        offset - n
    } else {
        del_start
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::style::FontWeight;
    use fanta_doc::Color;

    fn body() -> TextStyle {
        TextStyle::default()
    }

    fn bold() -> TextStyle {
        TextStyle::default().with_weight(FontWeight::Bold)
    }

    fn red() -> TextStyle {
        TextStyle::default().with_color(Color::rgb(255, 0, 0))
    }

    /// The runs must tile `0..len` exactly, in order, with no overlap.
    fn assert_covers(buf: &TextBuffer) {
        if buf.is_empty() {
            assert!(buf.runs().is_empty(), "empty buffer must have no runs");
            return;
        }
        let mut cursor = 0;
        for run in buf.runs() {
            assert_eq!(
                run.start,
                cursor,
                "gap or overlap at {cursor}: {:?}",
                buf.runs()
            );
            assert!(run.start < run.end, "zero/negative run: {run:?}");
            cursor = run.end;
        }
        assert_eq!(cursor, buf.len(), "runs do not reach end of text");
    }

    /// No two adjacent runs may share a style (they should have merged).
    fn assert_merged(buf: &TextBuffer) {
        for pair in buf.runs().windows(2) {
            assert!(
                pair[0].style != pair[1].style,
                "adjacent identical runs not merged: {:?}",
                buf.runs()
            );
        }
    }

    #[test]
    fn new_buffer_is_empty() {
        let b = TextBuffer::new();
        assert!(b.is_empty());
        assert_eq!(b.len(), 0);
        assert_eq!(b.text(), "");
        assert!(b.runs().is_empty());
    }

    #[test]
    fn from_str_has_one_covering_run() {
        let b = TextBuffer::from_str("hello", body());
        assert_eq!(b.text(), "hello");
        assert_eq!(b.len(), 5);
        assert_eq!(b.runs().len(), 1);
        assert_eq!(b.runs()[0].range(), 0..5);
        assert_covers(&b);
    }

    #[test]
    fn from_empty_str_has_no_runs() {
        let b = TextBuffer::from_str("", body());
        assert!(b.is_empty());
        assert!(b.runs().is_empty());
    }

    #[test]
    fn insert_into_empty_uses_default_style() {
        let mut b = TextBuffer::new();
        b.insert(0, "hi").unwrap();
        assert_eq!(b.text(), "hi");
        assert_eq!(b.runs().len(), 1);
        assert_eq!(&b.runs()[0].style, &body());
        assert_covers(&b);
    }

    #[test]
    fn insert_at_end_appends() {
        let mut b = TextBuffer::from_str("ab", body());
        b.insert(2, "cd").unwrap();
        assert_eq!(b.text(), "abcd");
        assert_covers(&b);
        assert_merged(&b);
    }

    #[test]
    fn insert_in_middle_shifts_runs() {
        let mut b = TextBuffer::from_str("aXb", body());
        b.insert(1, "123").unwrap();
        assert_eq!(b.text(), "a123Xb");
        assert_covers(&b);
    }

    #[test]
    fn insert_empty_string_is_noop() {
        let mut b = TextBuffer::from_str("ab", body());
        b.insert(1, "").unwrap();
        assert_eq!(b.text(), "ab");
        assert_eq!(b.runs().len(), 1);
    }

    #[test]
    fn insert_inherits_containing_run_style() {
        // "AABB": A's bold, B's body. Insert inside the bold region.
        let mut b = TextBuffer::from_str("AABB", body());
        b.set_style(0..2, bold()).unwrap();
        b.insert(1, "x").unwrap(); // inside bold run
        // The inserted x must be bold; "AxA" all bold then "BB" body.
        assert_eq!(b.text(), "AxABB");
        assert_eq!(&b.style_at(1), &&bold());
        assert_covers(&b);
        assert_merged(&b);
    }

    #[test]
    fn delete_round_trip_text() {
        let mut b = TextBuffer::from_str("hello world", body());
        b.delete_range(5..11).unwrap(); // remove " world"
        assert_eq!(b.text(), "hello");
        assert_covers(&b);
        b.insert(5, " world").unwrap();
        assert_eq!(b.text(), "hello world");
        assert_covers(&b);
    }

    #[test]
    fn delete_entire_buffer_clears_runs() {
        let mut b = TextBuffer::from_str("gone", body());
        b.delete_range(0..4).unwrap();
        assert!(b.is_empty());
        assert!(b.runs().is_empty());
    }

    #[test]
    fn delete_empty_range_is_noop() {
        let mut b = TextBuffer::from_str("abc", body());
        b.delete_range(1..1).unwrap();
        assert_eq!(b.text(), "abc");
    }

    #[test]
    fn delete_inverted_range_errors() {
        let mut b = TextBuffer::from_str("abc", body());
        // Construct the inverted range via `Range` fields so the literal does
        // not trip clippy's reversed-empty-ranges lint — the point of the test
        // is exactly that the method rejects start > end at runtime.
        let inverted = std::ops::Range { start: 2, end: 1 };
        assert_eq!(
            b.delete_range(inverted),
            Err(TextError::InvalidRange { start: 2, end: 1 })
        );
        assert_eq!(b.text(), "abc"); // unchanged
    }

    #[test]
    fn set_style_partial_splits_run() {
        let mut b = TextBuffer::from_str("hello", body());
        b.set_style(1..3, bold()).unwrap(); // "h[el]lo"
        assert_covers(&b);
        assert_merged(&b);
        assert_eq!(b.runs().len(), 3); // body | bold | body
        assert_eq!(b.runs()[0].range(), 0..1);
        assert_eq!(b.runs()[1].range(), 1..3);
        assert_eq!(&b.runs()[1].style, &bold());
        assert_eq!(b.runs()[2].range(), 3..5);
    }

    #[test]
    fn set_style_prefix_splits_into_two() {
        let mut b = TextBuffer::from_str("hello", body());
        b.set_style(0..2, bold()).unwrap();
        assert_eq!(b.runs().len(), 2);
        assert_eq!(b.runs()[0].range(), 0..2);
        assert_eq!(&b.runs()[0].style, &bold());
        assert_eq!(b.runs()[1].range(), 2..5);
        assert_covers(&b);
    }

    #[test]
    fn set_style_whole_range_yields_single_run() {
        let mut b = TextBuffer::from_str("hello", body());
        b.set_style(0..5, bold()).unwrap();
        assert_eq!(b.runs().len(), 1);
        assert_eq!(&b.runs()[0].style, &bold());
        assert_covers(&b);
    }

    #[test]
    fn set_style_merges_adjacent_identical_runs() {
        let mut b = TextBuffer::from_str("hello", body());
        // Split into three, then restyle the middle back to body so all three
        // become identical and must coalesce to one.
        b.set_style(1..3, bold()).unwrap();
        assert_eq!(b.runs().len(), 3);
        b.set_style(1..3, body()).unwrap();
        assert_eq!(b.runs().len(), 1, "runs should merge back: {:?}", b.runs());
        assert_merged(&b);
        assert_covers(&b);
    }

    #[test]
    fn set_style_overlapping_existing_runs_is_clean() {
        let mut b = TextBuffer::from_str("abcdef", body());
        b.set_style(1..2, bold()).unwrap();
        b.set_style(3..4, red()).unwrap();
        // Now restyle a span that crosses both: 1..4 -> bold.
        b.set_style(1..4, bold()).unwrap();
        assert_covers(&b);
        assert_merged(&b);
        assert_eq!(&b.style_at(1), &&bold());
        assert_eq!(&b.style_at(3), &&bold());
    }

    #[test]
    fn set_style_empty_range_is_noop() {
        let mut b = TextBuffer::from_str("abc", body());
        b.set_style(1..1, bold()).unwrap();
        assert_eq!(b.runs().len(), 1);
        assert_eq!(&b.runs()[0].style, &body());
    }

    // -- UTF-8 safety ------------------------------------------------------

    #[test]
    fn insert_at_multibyte_boundary_is_allowed() {
        // "é" is 2 bytes (0xC3 0xA9). Valid boundaries: 0 and 2.
        let mut b = TextBuffer::from_str("é", body());
        assert_eq!(b.len(), 2);
        b.insert(2, "!").unwrap(); // after the é — on a boundary
        assert_eq!(b.text(), "é!");
        b.insert(0, "¡").unwrap(); // before the é — on a boundary
        assert_eq!(b.text(), "¡é!");
        assert_covers(&b);
    }

    #[test]
    fn insert_mid_char_is_rejected() {
        let mut b = TextBuffer::from_str("é", body()); // bytes 0..2
        assert_eq!(
            b.insert(1, "x"),
            Err(TextError::NotCharBoundary { index: 1 })
        );
        assert_eq!(b.text(), "é"); // unchanged
    }

    #[test]
    fn delete_mid_char_is_rejected() {
        let mut b = TextBuffer::from_str("naïve", body());
        // "ï" is 2 bytes; find its interior index.
        let i = b.text().find('ï').unwrap();
        assert_eq!(
            b.delete_range(i..i + 1),
            Err(TextError::NotCharBoundary { index: i + 1 })
        );
        assert_eq!(b.text(), "naïve");
    }

    #[test]
    fn insert_out_of_bounds_is_rejected() {
        let mut b = TextBuffer::from_str("ab", body());
        assert_eq!(
            b.insert(5, "x"),
            Err(TextError::OutOfBounds { index: 5, len: 2 })
        );
    }

    #[test]
    fn emoji_insert_and_delete_round_trips() {
        // Crab emoji is 4 bytes.
        let mut b = TextBuffer::new();
        b.insert(0, "a🦀b").unwrap();
        assert_eq!(b.len(), 6);
        assert_covers(&b);
        // Delete the emoji (bytes 1..5).
        b.delete_range(1..5).unwrap();
        assert_eq!(b.text(), "ab");
        assert_covers(&b);
    }

    #[test]
    fn styled_multibyte_text_keeps_coverage() {
        let mut b = TextBuffer::from_str("aéb", body()); // bytes: a=0, é=1..3, b=3
        b.set_style(1..3, bold()).unwrap(); // style just the é
        assert_covers(&b);
        assert_eq!(&b.style_at(1), &&bold());
        assert_eq!(&b.style_at(0), &&body());
    }

    #[test]
    fn buffer_round_trips_through_json() {
        let mut b = TextBuffer::from_str("hello", body());
        b.set_style(0..2, bold()).unwrap();
        let json = serde_json::to_string(&b).unwrap();
        let back: TextBuffer = serde_json::from_str(&json).unwrap();
        assert_eq!(b, back);
    }
}
