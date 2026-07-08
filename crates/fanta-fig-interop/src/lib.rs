//! `.fig` (Figma) interop for Fantaisa. Phase 4 (months 8–10).
//!
//! This crate reads Figma `.fig` files into the polymorphic Fantaisa [`Doc`].
//! It is built bottom-up in three layers:
//!
//! 1. [`kiwi`] — the **Kiwi binary codec** (github.com/evanw/kiwi), the
//!    serialization format underneath `.fig`. A faithful Rust port of the
//!    reference wire format: primitives (varint/zig-zag/compact-float/
//!    null-terminated UTF-8), the schema-driven compound model (enum/struct/
//!    message/array), the self-describing binary schema, and a dynamic
//!    [`KiwiValue`] tree so an arbitrary `.fig` can be decoded generically. This
//!    layer is exact and round-trip-proven.
//! 2. [`fig`] — the **`.fig` container**: an 8-byte `fig-kiwi` magic, a version,
//!    then length-prefixed zlib blocks (the embedded schema, then the encoded
//!    document). Figma publishes no spec, so structural choices are marked with
//!    `// ASSUMPTION:` and the writer is a test aid, not a product feature
//!    (`.fig` *export* is a non-goal — ARCHITECTURE.md §12).
//! 3. [`mapping`] — the semantic projection of the recognized Figma node kinds
//!    onto the doc: `CANVAS`/`FRAME`/`GROUP`/`SECTION` → group,
//!    `RECTANGLE`/`ROUNDED_RECTANGLE`/`ELLIPSE`/`TEXT` → vector/text, and the
//!    full design-system surface — `SYMBOL`/`COMPONENT` → component masters under
//!    a hidden Components page, state-group `SYMBOL`/`COMPONENT_SET` → component
//!    sets with parsed variant axes, `INSTANCE` → `NodeData::Instance` (virtual
//!    subtree), `VARIABLE_SET`/`VARIABLE` → collections + per-mode values,
//!    `variableConsumptionMap` → node bindings, and `prototypeInteractions`/
//!    `prototypeStartNodeID` → reactions + flow start. VECTOR-family geometry
//!    (`VECTOR`/`STAR`/`LINE`/`BOOLEAN_OPERATION`/`REGULAR_POLYGON`) has its
//!    **real path** decoded from the file's `fillGeometry`/`strokeGeometry`
//!    command blobs (see [`geometry`]); a node whose blob is missing or
//!    undecodable keeps a bbox-rect fallback. Partial by design; unsupported
//!    kinds are skipped with a count in [`MapReport`].
//!
//! ## What is proven vs. assumed
//!
//! The Kiwi codec is the rock-solid foundation — its byte-for-byte output is
//! pinned against the reference implementation's own test vectors and every
//! type round-trips. The container framing and the node mapping are grounded in
//! public reverse-engineering and the real Figma `fig.kiwi` schema (correct
//! field names and node-type enum), and they round-trip through this crate's
//! own writer, but they have not been validated against a file exported by
//! Figma itself. See the module docs for the precise assumptions.
//!
//! [`Doc`]: fanta_doc::Doc

#![forbid(unsafe_code)]

pub mod canvas;
pub mod error;
pub mod fig;
pub mod geometry;
pub mod kiwi;
pub mod mapping;

pub use canvas::figma_page_canvas_color;
pub use error::{FigError, FigResult};
pub use fig::{FigDocument, read_fig, write_fig};
pub use kiwi::{ByteReader, ByteWriter, Def, DefKind, Field, KiwiType, KiwiValue, Schema};
pub use mapping::{MapReport, fig_to_doc};
