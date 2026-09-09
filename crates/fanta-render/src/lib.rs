//! The Skia raster backend for the Fanta engine.
//!
//! [`RasterRenderer`] takes a [`fanta_doc::Scene`] + [`fanta_doc::Viewport`]
//! and paints it two ways from one code path:
//!
//! 1. Into its **own CPU-backed Skia surface** — read back with
//!    [`RasterRenderer::copy_rgba`] / [`RasterRenderer::encode_png`]. This is
//!    the headless / export / golden-test path.
//! 2. Onto an **externally owned Skia canvas** via
//!    [`RasterRenderer::render_to_canvas`] — the seam a host's live-window
//!    present uses with a GPU-backed (e.g. Ganesh/Metal) surface. Both paths
//!    share the same scene walk, so they are pixel-identical.
//!
//! The scene model stays renderer-agnostic (`fanta-doc` has no Skia
//! dependency — ARCHITECTURE.md §2); this crate is where Skia is allowed.
//! Effects (drop/inner shadows, layer/background blurs), blend modes, masks,
//! component-instance expansion, variable resolution, culling, and the
//! image/instance caches live here (ARCHITECTURE.md §4–§5).
//!
//! There is no GPU-owned surface or wgpu pipeline in this crate today. Layer
//! caches and tile-based dirty regions are future work (ARCHITECTURE.md §4–§5).

#![allow(clippy::result_large_err)]
#![forbid(unsafe_code)]

pub mod asset;
mod bounds;
pub mod color;
pub mod image;
pub mod paint;
pub mod path;
pub mod raster;
pub mod transform;

pub use asset::{AssetResolver, DecodedImage, InMemoryAssetResolver, LazyAssetResolver};
pub use bounds::visual_world_bounds;
pub use color::{to_sk_color, to_sk_color4f};
pub use image::{
    FitRects, ImageCache, crop_to_pixels, draw_decoded_image, draw_image_cached, fit_src_dst,
};
pub use paint::{fill_to_paint, stroke_to_paint};
pub use path::to_sk_path;
// `vector_outline_sk_path`: track svg-prod — the effective vector outline,
// shared with the app's flatten/outline/simplify geometry ops.
pub use raster::{
    MediaPlayback, RasterRenderer, RenderError, RenderInputs, RenderMetrics, SvgRenderOptions,
    SvgRenderOutput, measure_text_node, solve_scene_layout, text_caret_rect, text_first_baseline,
    text_hit_test, text_line_height, text_node_outline, text_selection_rects,
    vector_outline_sk_path,
};
pub use transform::to_sk_matrix;
