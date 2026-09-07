//! Caret and selection model with grapheme-aware movement.
//!
//! Positions are byte offsets into the text (the same coordinate space as
//! [`crate::buffer::TextBuffer`]), so a caret can be handed straight to the
//! buffer for an insert/delete without translation. Movement, however, must be
//! *grapheme*-aware, not byte- or `char`-aware: a user pressing the right arrow
//! over an emoji, a flag, or an accented `e` expects the caret to jump the whole
//! perceived character, even though that "character" may be several `char`s and
//! many bytes. This module uses `unicode-segmentation`'s extended grapheme
//! cluster boundaries to get that right.

use std::ops::Range;
use unicode_segmentation::UnicodeSegmentation;

/// A text insertion point, as a byte offset.
///
/// A bare `usize` would work, but a named type documents intent at call sites
/// ("this `usize` is a caret, not a length") and gives movement a natural place
/// to live. The offset is always a `char` boundary for any well-formed caret;
/// the movement helpers preserve that, and callers constructing one by hand are
/// responsible for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Caret {
    /// Byte offset into the text.
    pub byte: usize,
}

impl Caret {
    /// A caret at the given byte offset.
    pub fn new(byte: usize) -> Self {
        Self { byte }
    }

    /// Move one grapheme cluster left, clamping at the start of the text.
    ///
    /// Walks the cluster boundaries of `text` and lands on the boundary
    /// immediately before the current position. Returns a new caret; carets are
    /// `Copy` and movement is pure so the editor can compute candidate positions
    /// without committing them.
    #[must_use]
    pub fn move_left(self, text: &str) -> Self {
        let prev = prev_boundary(text, self.byte);
        Self { byte: prev }
    }

    /// Move one grapheme cluster right, clamping at the end of the text.
    #[must_use]
    pub fn move_right(self, text: &str) -> Self {
        let next = next_boundary(text, self.byte);
        Self { byte: next }
    }

    /// Move to the start of the current line (the byte after the preceding
    /// `\n`, or 0). "Line" here is a hard newline-delimited line, matching what
    /// the Home key does in every editor; soft-wrap-aware Home is a layout-level
    /// concern handled in `layout.rs`.
    #[must_use]
    pub fn move_home(self, text: &str) -> Self {
        let byte = line_start(text, self.byte);
        Self { byte }
    }

    /// Move to the end of the current line (the byte before the next `\n`, or
    /// the end of the text).
    #[must_use]
    pub fn move_end(self, text: &str) -> Self {
        let byte = line_end(text, self.byte);
        Self { byte }
    }
}

/// A directed text selection: a fixed `anchor` and a moving `head`.
///
/// Anchor/head (rather than start/end) is the model every editor uses for
/// shift-arrow selection: the anchor is where selection began, the head follows
/// the caret, and `head` may be *before* `anchor` when selecting leftward. The
/// normalized, start-before-end form is recovered on demand via
/// [`range`](Self::range) — storing the directed form is what lets the UI know
/// which end the user is actively dragging.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Selection {
    /// Byte offset where the selection was anchored (fixed end).
    pub anchor: usize,
    /// Byte offset of the moving end (follows the caret).
    pub head: usize,
}

impl Selection {
    /// A selection spanning `anchor..head` (either order).
    pub fn new(anchor: usize, head: usize) -> Self {
        Self { anchor, head }
    }

    /// A collapsed (zero-width) selection — a plain caret — at `byte`.
    pub fn caret(byte: usize) -> Self {
        Self {
            anchor: byte,
            head: byte,
        }
    }

    /// Whether the selection is collapsed (anchor == head), i.e. just a caret
    /// with nothing selected. The editor uses this to decide whether a delete
    /// removes the selection or one grapheme.
    pub fn is_collapsed(&self) -> bool {
        self.anchor == self.head
    }

    /// The selection as a normalized half-open byte range `start..end` with
    /// `start <= end`, regardless of drag direction.
    ///
    /// This is the form the buffer's [`delete_range`](crate::buffer::TextBuffer::delete_range)
    /// and [`set_style`](crate::buffer::TextBuffer::set_style) want, so selecting
    /// right-to-left and then hitting delete works without the caller untangling
    /// the direction.
    pub fn range(&self) -> Range<usize> {
        let start = self.anchor.min(self.head);
        let end = self.anchor.max(self.head);
        start..end
    }

    /// The lower byte offset of the selection.
    pub fn start(&self) -> usize {
        self.anchor.min(self.head)
    }

    /// The upper byte offset of the selection.
    pub fn end(&self) -> usize {
        self.anchor.max(self.head)
    }

    /// Collapse the selection to its head, returning a caret there. What
    /// pressing an arrow key (without shift) does to an active selection in most
    /// editors collapses to the head; some collapse to an edge, but head is the
    /// least surprising default.
    pub fn collapse_to_head(&self) -> Caret {
        Caret { byte: self.head }
    }
}

// ---------------------------------------------------------------------------
// Grapheme / line boundary helpers
// ---------------------------------------------------------------------------

/// The grapheme-cluster boundary immediately before `byte`, or 0.
///
/// `grapheme_indices(true)` yields extended (user-perceived) clusters with
/// their starting byte; the predecessor of `byte` is the last cluster start
/// strictly less than `byte`. Falls back to 0 when at or before the first
/// cluster. Robust to a `byte` that is not itself a cluster boundary: it lands
/// on the cluster *containing* it, which is the sane behavior for a clamped or
/// externally-set caret.
fn prev_boundary(text: &str, byte: usize) -> usize {
    if byte == 0 {
        return 0;
    }
    let mut last = 0;
    for (idx, _) in text.grapheme_indices(true) {
        if idx >= byte {
            break;
        }
        last = idx;
    }
    last
}

/// The grapheme-cluster boundary immediately after `byte`, or `text.len()`.
fn next_boundary(text: &str, byte: usize) -> usize {
    if byte >= text.len() {
        return text.len();
    }
    for (idx, _) in text.grapheme_indices(true) {
        if idx > byte {
            return idx;
        }
    }
    text.len()
}

/// Start of the hard line containing `byte`: one past the nearest preceding
/// `\n`, or 0 if there is none.
fn line_start(text: &str, byte: usize) -> usize {
    let clamped = byte.min(text.len());
    match text[..clamped].rfind('\n') {
        Some(nl) => nl + 1,
        None => 0,
    }
}

/// End of the hard line containing `byte`: the offset of the next `\n`, or the
/// end of the text if the caret is on the last line.
fn line_end(text: &str, byte: usize) -> usize {
    let clamped = byte.min(text.len());
    match text[clamped..].find('\n') {
        Some(rel) => clamped + rel,
        None => text.len(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caret_moves_one_ascii_char() {
        let t = "abc";
        let c = Caret::new(0);
        assert_eq!(c.move_right(t).byte, 1);
        assert_eq!(c.move_right(t).move_right(t).byte, 2);
    }

    #[test]
    fn caret_clamps_at_ends() {
        let t = "ab";
        assert_eq!(Caret::new(0).move_left(t).byte, 0);
        assert_eq!(Caret::new(2).move_right(t).byte, 2);
    }

    #[test]
    fn caret_moves_over_accented_char_as_one() {
        // "é" as a single precomposed char is 2 bytes; one right-move clears it.
        let t = "éx";
        assert_eq!(t.len(), 3);
        let c = Caret::new(0);
        let after = c.move_right(t);
        assert_eq!(after.byte, 2, "should skip the whole 2-byte é");
        assert_eq!(after.move_left(t).byte, 0);
    }

    #[test]
    fn caret_moves_over_emoji_as_one() {
        // Crab emoji is 4 bytes; a single grapheme.
        let t = "🦀!";
        assert_eq!(t.len(), 5);
        let c = Caret::new(0);
        assert_eq!(
            c.move_right(t).byte,
            4,
            "should skip the whole 4-byte emoji"
        );
    }

    #[test]
    fn caret_moves_over_combining_sequence_as_one() {
        // "e" + combining acute accent (U+0301, 2 bytes) = one grapheme cluster,
        // 3 bytes total. Grapheme-awareness (not char-awareness) is what makes
        // this move as a single unit.
        let t = "e\u{0301}z";
        assert_eq!(t.len(), 4);
        let c = Caret::new(0);
        let after = c.move_right(t);
        assert_eq!(after.byte, 3, "combining sequence must move as one cluster");
    }

    #[test]
    fn caret_moves_over_flag_emoji_as_one() {
        // Regional indicator pair forms one flag grapheme (8 bytes).
        let t = "\u{1F1FA}\u{1F1F8}end"; // US flag + "end"
        let c = Caret::new(0);
        let after = c.move_right(t);
        assert_eq!(after.byte, 8, "flag (2 regional indicators) is one cluster");
    }

    #[test]
    fn home_and_end_on_single_line() {
        let t = "hello";
        let c = Caret::new(3);
        assert_eq!(c.move_home(t).byte, 0);
        assert_eq!(c.move_end(t).byte, 5);
    }

    #[test]
    fn home_and_end_respect_hard_newlines() {
        let t = "ab\ncde\nf";
        // Caret in the middle line ("cde", bytes 3..6).
        let c = Caret::new(4);
        assert_eq!(c.move_home(t).byte, 3, "home goes to after the first \\n");
        assert_eq!(c.move_end(t).byte, 6, "end goes to before the next \\n");
    }

    #[test]
    fn end_on_last_line_goes_to_text_end() {
        let t = "ab\ncd";
        let c = Caret::new(4);
        assert_eq!(c.move_end(t).byte, 5);
    }

    #[test]
    fn selection_range_normalizes_when_anchor_after_head() {
        // Selected right-to-left: anchor 5, head 2.
        let s = Selection::new(5, 2);
        assert_eq!(s.range(), 2..5);
        assert_eq!(s.start(), 2);
        assert_eq!(s.end(), 5);
        assert!(!s.is_collapsed());
    }

    #[test]
    fn selection_range_normalizes_when_anchor_before_head() {
        let s = Selection::new(1, 4);
        assert_eq!(s.range(), 1..4);
    }

    #[test]
    fn collapsed_selection_is_a_caret() {
        let s = Selection::caret(3);
        assert!(s.is_collapsed());
        assert_eq!(s.range(), 3..3);
        assert_eq!(s.collapse_to_head().byte, 3);
    }

    #[test]
    fn selection_collapse_to_head_takes_moving_end() {
        let s = Selection::new(5, 2); // head is 2
        assert_eq!(s.collapse_to_head().byte, 2);
    }
}
