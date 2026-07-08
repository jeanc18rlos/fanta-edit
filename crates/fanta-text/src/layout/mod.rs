//! Shaping and layout via Skia's `textlayout` (Paragraph) API.
//!
//! This module is the bridge from the editing model ([`crate::buffer::TextBuffer`])
//! to pixels. It is the *only* place in the crate that touches Skia: the buffer,
//! caret, and style types are renderer-agnostic on purpose (ARCHITECTURE.md §2 —
//! the doc model must survive a future Skia → Vello swap), and confining the
//! Skia dependency to one module is what keeps that door open.
//!
//! ## Why Skia's Paragraph
//!
//! Text is where every design tool's schedule blows up (ULTRA_PLAN.md §13 risk
//! register). Skia's `Paragraph` already does shaping, BiDi, line breaking,
//! per-run styling, hit-testing, and caret geometry — reimplementing any of that
//! is the wrong use of the wedge's time. We wrap it behind a small, owned
//! [`TextLayout`] so the rest of Fantaisa depends on our surface, not Skia's.
//!
//! ## Font resolution
//!
//! We don't ship the proprietary faces a `.fig` references (Figma's defaults,
//! Adobe Clean / Adobe Clean Serif, etc.). Rather than blanket-overriding every
//! missing family with one bundled face — which made text laid out for the real
//! font overflow its boxes — the engine resolves each run's *requested* family
//! to the closest face it can actually obtain, exactly like Figma downloading
//! the document's real fonts. The chain (installed → bundled → Google-Fonts
//! download → known-proprietary→open substitute → generic class) lives in
//! [`crate::font_resolver::FontResolver`]; this module just consumes its ordered
//! family list. The two invariants the layout layer preserves:
//!
//! 1. **Weight + italic are honored.** [`crate::style::TextStyle::weight`] (an
//!    OpenType 100–900 number) becomes a `skia_safe::font_style::Weight`, and
//!    [`crate::style::TextStyle::italic`] becomes a `Slant`. A 700 heading
//!    therefore renders bolder than a 400 body run, and italic renders slanted.
//!    The bundled families register multiple weights (variable Source faces are
//!    instanced per weight), so Skia picks the closest registered face for the
//!    run's style instead of synthesizing.
//! 2. **One resolved face for measure *and* paint.** The same family list the
//!    resolver returns is baked into the `TextStyle` that both line-breaking/
//!    measurement and painting consume, so layout geometry never disagrees with
//!    what is drawn.
//!
//! ## The font collection
//!
//! The engine installs the resolver's [`TypefaceFontProvider`] (carrying the
//! bundled Inter + Source families and anything downloaded this run) as the
//! collection's **asset** manager, probed before the system default. The asset
//! provider only answers for the families it actually carries, so a genuinely-
//! installed family (a real "Adobe Clean") still wins via the requested-name-
//! leads-the-list rule routing through the system [`FontMgr`]. `FontMgr::default()`
//! is the collection's default *and* fallback manager, so glyphs no listed family
//! covers (emoji, CJK) still shape.
//!
//! [`TypefaceFontProvider`]: skia_safe::textlayout::TypefaceFontProvider
//! [`FontMgr`]: skia_safe::FontMgr

mod align;
mod engine;
mod text_layout;

pub use align::Align;
pub use engine::{LayoutEngine, LayoutOptions};
pub use text_layout::{LineMetrics, TextLayout};
