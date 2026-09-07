//! Text editing and layout for the Fanta engine.
//!
//! A standalone editing + layout model for rich text. It is deliberately *not*
//! wired into `fanta-doc`'s `CanvasNode` enum: the engine is built on its own
//! types so the tool layer (the text tool, ULTRA_PLAN.md §2.2) and a future
//! `TextNode` can both consume it without the doc crate taking a Skia
//! dependency before that integration is designed.
//!
//! ## The three layers
//!
//! 1. **Style** ([`TextStyle`], [`FontWeight`]) — per-run character formatting.
//!    Reuses `fanta_doc::Color` so text color shares the document's color path.
//! 2. **Editing** ([`TextBuffer`], [`StyleRun`], [`Caret`], [`Selection`]) — the
//!    mutable model. Content is a UTF-8 `String`; styling is a normalized list
//!    of byte-range runs. Caret/selection movement is grapheme-aware. None of
//!    this layer depends on Skia, so it is trivially testable and renderer-free.
//! 3. **Layout** ([`LayoutEngine`], [`TextLayout`], [`LineMetrics`]) — the only
//!    Skia-touching layer. Wraps Skia's `Paragraph` for shaping, line breaking,
//!    hit-testing, and caret geometry.
//!
//! ## Coordinates
//!
//! Byte offsets (UTF-8) are the universal position currency: the buffer slices
//! on them, the caret stores one, and Skia's hit-test / rect queries speak them.
//! Geometry is in `f64` logical pixels to match the doc's coordinate decision
//! (ARCHITECTURE.md §13), narrowing to `f32` only at the Skia boundary.
//!
//! ## Quick tour
//!
//! ```
//! use fanta_text::{TextBuffer, TextStyle, FontWeight, LayoutEngine, Caret};
//!
//! // Build styled content.
//! let mut buf = TextBuffer::from_str("Hello, world", TextStyle::default());
//! buf.set_style(0..5, TextStyle::default().with_weight(FontWeight::Bold)).unwrap();
//!
//! // Move a caret one grapheme at a time.
//! let caret = Caret::new(0).move_right(buf.text());
//! assert_eq!(caret.byte, 1);
//!
//! // Lay it out and query geometry.
//! let engine = LayoutEngine::new();
//! let layout = engine.layout(&buf, 1000.0);
//! assert!(layout.height() > 0.0);
//! assert_eq!(layout.hit_test([0.5, 2.0]), 0);
//! ```

#![forbid(unsafe_code)]

pub mod buffer;
pub mod caret;
pub mod font_resolver;
pub mod layout;
pub mod style;

// Flat re-exports for ergonomic call sites, matching the pattern in
// `fanta-doc`'s `lib.rs`.
pub use buffer::{StyleRun, TextBuffer, TextError};
pub use caret::{Caret, Selection};
pub use font_resolver::{
    ADOBE_CLEAN_SANS_METRIC_RATIO, ADOBE_CLEAN_SERIF_METRIC_RATIO, FontResolver, GenericFamily,
    INTER_FAMILY, SOURCE_CODE_FAMILY, SOURCE_SANS_FAMILY, SOURCE_SERIF_FAMILY,
    bundled_family_names, bundled_preview_bytes, prewarm_font_downloads,
};
pub use layout::{LayoutEngine, LayoutOptions, LineMetrics, TextLayout};
pub use style::{FontWeight, TextStyle};
