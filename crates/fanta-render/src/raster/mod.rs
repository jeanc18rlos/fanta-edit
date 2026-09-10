//! [`RasterRenderer`] — render a [`Scene`] to a CPU-backed Skia surface.
//!
//! This is the v0 renderer: enough to exercise the conversion pipeline, drive
//! snapshot tests, and produce PNG exports. The shape of its API
//! (`(Scene, Viewport) → pixels`) anticipates the GPU-backed version: when
//! that lands, callers swap `RasterRenderer` for `GpuRasterRenderer` without
//! touching the render call site.
//!
//! ## What it does now
//!
//! - Walks the scene root-children in z-order, depth-first.
//! - Composes parent → child transforms via the Skia canvas save/restore stack.
//! - Renders [`NodeData::Vector`] paths (fills + strokes, including image fills
//!   clipped to the path) and [`NodeData::Group`] children.
//! - Renders [`NodeData::Bitmap`] through an [`AssetResolver`]: the decoded
//!   image is fitted into the node's local rect per its [`ImageFitMode`],
//!   cropped, and tinted. A missing/undecoded asset falls back to a debug
//!   placeholder so it stays visible and never panics.
//! - Renders [`NodeData::Instance`] (component instances) by expanding the
//!   master subtree via [`fanta_doc::expand_instance`], reconstructing the
//!   transient parent→child tree from the fresh clone ids, and drawing it
//!   clipped to the instance's `local_size` (like a frame). Nested instances
//!   recurse. The expansion is memoized across frames (see [`RenderInputs`]).
//! - Applies the **variable-resolution overlay**: before painting a node with
//!   `bindings`, every bound `(BoundProp, VariableId)` is resolved in the
//!   node's effective mode ([`fanta_doc::resolve_effective_mode`]) and written
//!   onto a SCRATCH COPY of the node, which is what gets painted — the live
//!   scene is never mutated, so a Light/Dark toggle re-paints without touching
//!   the document. Requires the app to pass [`RenderInputs`] via
//!   [`RasterRenderer::render_with`] / [`RasterRenderer::render_page_with`]; the
//!   legacy [`RasterRenderer::render`] / [`RasterRenderer::render_page`] entry
//!   points pass an empty context (no components/variables) and behave exactly
//!   as before.
//! - Draws a debug placeholder for the still-unimplemented variants.
//! - Honors per-node `opacity` via a save-layer when < 1.0 — except for a leaf
//!   vector emitting a single Solid/Gradient draw, whose opacity is folded into
//!   that draw's paint alpha (pixel-identical, no layer; see
//!   `opacity_folds_into_paint`).
//! - Renders node-level **drop shadows** (`CanvasNode.effects`) and **blend
//!   modes** (`CanvasNode.blend_mode`): when a node has ≥1 drop shadow or a
//!   non-`Normal` blend mode, its drawing is wrapped in a save-layer whose
//!   `Paint` carries a chained drop-shadow `ImageFilter` (so the shadow follows
//!   the whole composited node + subtree silhouette) and the mapped Skia blend
//!   mode. Nodes with neither pay nothing — no extra layer is allocated.
//! - Honors per-node `flags.HIDDEN`.
//! - **Culls off-screen nodes**: derives the visible world rect (the inverse of
//!   the viewport transform) and skips any node — and, for a group, its whole
//!   subtree — whose world AABB misses it. Reported as `nodes_culled`.
//! - **Caches uploaded images**: bitmaps and image fills build their Skia
//!   `Image` once and reuse it across frames, keyed by content-addressed
//!   `AssetId` (see [`ImageCache`]).
//! - **Caches rendered effect layers** across frames: a node's blur / shadow /
//!   blend / opacity save-layer is rendered once per zoom into an offscreen
//!   image and re-drawn, pixel-identically, on every whole-device-pixel pan
//!   until the next content edit (see `layer_cache`;
//!   [`RasterRenderer::set_layer_cache_enabled`] /
//!   [`RasterRenderer::set_pixel_snap_pan`]).
//!
//! [`AssetResolver`]: crate::asset::AssetResolver
//! [`ImageFitMode`]: fanta_doc::ImageFitMode
//! [`ImageCache`]: crate::image::ImageCache
//!
//! ## What it does not do yet
//!
//! - Video / 3D / AI artifact / Audio / Embed variants (rendered as
//!   placeholders for now — they need frame decode, GPU compute, or async
//!   generation). Their pixels ultimately composite through the same image
//!   path bitmap now uses (spec §1).
//! - Tile-based dirty regions and a renderer-owned GPU surface (hosts pass a
//!   Ganesh canvas to [`RasterRenderer::render_to_canvas`] today).
//!
//! (Drop shadows, **inner shadows** — a clip-to-shape + offset/blur/`SrcOut`
//! filter DAG, see [`draw_inner_shadows`] — plus layer/background blurs and
//! blend modes ARE rendered.)
//!
//! Those land per ARCHITECTURE.md §4–§5 in subsequent commits.

// ---------------------------------------------------------------------------
// Submodules (pure split of the former single `raster.rs`)
//
// `mod.rs` stays thin: it owns the shared import surface and the submodule +
// re-export wiring. Each submodule does `use super::*;` to pick up both the
// external imports re-exported below and its sibling `pub(crate)` items, so a
// move is a pure relocation — no item was renamed and no public path changed.
// ---------------------------------------------------------------------------

mod boolean;
mod content;
mod cull;
mod effects;
mod instance;
mod layer_cache;
mod media;
mod renderer;
mod text;
mod text_path;
mod vector;
mod walk;

#[cfg(test)]
mod tests;

// Public surface — preserves the exact paths `crate::raster::*` (and, via
// `lib.rs`, `fanta_render::*`) resolved before the split.
pub use instance::{measure_text_node, solve_scene_layout};
pub use renderer::{
    MediaPlayback, RasterRenderer, RenderError, RenderInputs, RenderMetrics, SvgRenderOptions,
    SvgRenderOutput,
};
pub use text::{
    text_caret_rect, text_first_baseline, text_hit_test, text_line_height, text_node_outline,
    text_selection_rects,
};
pub use vector::vector_outline_sk_path; // track svg-prod

// Crate-internal items shared across the submodules (and exercised by the
// co-located `tests`). `use super::*;` in each submodule resolves these. Only
// items referenced from a *different* submodule (or the tests) are re-exported
// here; helpers used solely within their defining submodule are reached
// directly there and stay unlisted.
pub(crate) use boolean::paint_boolean;
pub(crate) use content::{frame_box_bounds, paint_node_content, paint_node_foreground};
pub(crate) use cull::visible_world_rect;
pub(crate) use effects::{
    apply_background_blur, begin_effects_layer, draw_inner_shadows, effects_layer_bounds,
    opacity_folds_into_paint, padded_layer_rect, shadow_expanded_local_bounds, visible_effects,
};
pub(crate) use instance::render_instance;
pub(crate) use layer_cache::{LayerCache, LayerEpoch};
pub(crate) use renderer::{
    BooleanCache, CachedVectorPaths, InstanceCache, InstanceCacheKey, PathCache,
};
pub(crate) use text::{draw_text_node, with_shaped_layout};
pub(crate) use text_path::{draw_text_path_node, text_path_bounds, text_path_outline};
pub(crate) use vector::{
    bounds_to_f32, draw_placeholder, draw_unresolved_outline, draw_vector, path_is_rect,
    rounded_rect_path, stroke_box_path,
};
pub(crate) use walk::{RenderCtx, paint_child_sequence, render_node, resolve_overlay};

// Internals exercised ONLY by the co-located `tests` module (and otherwise used
// solely within their defining submodule). Re-exported under `#[cfg(test)]` so
// the non-test lib build doesn't see them as dead re-exports.
#[cfg(test)]
pub(crate) use cull::CULL_MARGIN_WORLD;
#[cfg(test)]
pub(crate) use effects::{
    SIGMA_SCREEN_MAX, SIGMA_SCREEN_MIN, VisibleEffects, capped_render_sigma,
    shadow_expanded_world_bounds,
};
#[cfg(test)]
pub(crate) use renderer::hash_overrides;
#[cfg(test)]
pub(crate) use text::{clear_layout_cache, layout_cache_len};

// Shared external import surface, re-exported `pub(crate)` so every submodule's
// `use super::*;` resolves the same names the monolith's top-of-file `use`s did.
pub(crate) use crate::asset::AssetResolver;
pub(crate) use crate::color::to_sk_color;
pub(crate) use crate::image::{ImageCache, ImageFillMods, draw_image_cached};
pub(crate) use crate::paint::{
    fill_to_paint, scale_paint_alpha, stroke_to_paint, to_sk_blend_mode,
};
pub(crate) use crate::path::{to_sk_fill_path, to_sk_path};
pub(crate) use crate::transform::to_sk_matrix;
pub(crate) use fanta_doc::{
    AudioNode, BlendMode, Bounds, CanvasNode, Color, ComponentLibrary, ExpandedNode, Fill,
    GroupNode, IdHashMap, InstanceNode, MaskType, ModeId, Model3dNode, NodeData, NodeFlags, NodeId,
    Scene, Shadow, ShadowKind, TextAlign, TextAutoResize, TextNode, Transform2D, VAlign,
    VariableCollectionId, VariableRegistry, VideoNode, Viewport, resolve_bound_value,
};
// `Blur`/`BlurKind` are reached via the (public) `style` module rather than a
// crate-root re-export, since the doc crate's `lib.rs` is owned by another
// agent — the module path keeps this additive without touching it.
pub(crate) use fanta_doc::style::{Blur, BlurKind};
// `Align` is reached via its module path rather than a crate-root re-export so
// this integration touches `fanta-text`'s sanctioned layout module only.
pub(crate) use fanta_text::layout::Align;
pub(crate) use fanta_text::{LayoutEngine, TextBuffer, TextStyle};
pub(crate) use skia_safe::{
    AlphaType, Canvas, ColorType, EncodedImageFormat, ImageInfo, Paint, Rect, Surface, surfaces,
};
pub(crate) use std::cell::RefCell;
pub(crate) use std::collections::{BTreeMap, HashMap};
pub(crate) use std::hash::{Hash, Hasher};
pub(crate) use std::sync::Arc;
pub(crate) use std::time::Instant;
