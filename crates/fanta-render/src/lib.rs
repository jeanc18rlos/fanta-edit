//! Skia + wgpu rendering for Fantaisa.
//!
//! Two pipelines, composited:
//!
//! 1. **Skia** owns the 2D canvas — vector, raster, video frames, text.
//!    Wraps the entire scene's compositing tree.
//! 2. **wgpu** (later — phase 2) renders 3D and custom compute into off-screen
//!    textures that Skia composites as image fills.
//!
//! GPUI hosts the resulting surface in the app shell.
//!
//! ## What's here in v0
//!
//! [`RasterRenderer`] — a CPU-backed Skia surface that takes a
//! [`fanta_doc::Scene`] + [`fanta_doc::Viewport`] and produces an RGBA pixmap
//! and/or a PNG. Enough to exercise the conversion pipeline, drive snapshot
//! tests, and produce PNG exports without wiring a window/GPU yet.
//!
//! GPU-backed surfaces, layer caches, tile-based dirty regions, and the
//! wgpu compositor land in subsequent commits (see ARCHITECTURE.md §4–§5).

#![allow(clippy::result_large_err)]
#![forbid(unsafe_code)]

pub mod asset;
pub mod color;
pub mod image;
pub mod paint;
pub mod path;
pub mod raster;
pub mod transform;

pub use asset::{AssetResolver, DecodedImage, InMemoryAssetResolver};
pub use color::{to_sk_color, to_sk_color4f};
pub use image::{
    FitRects, ImageCache, crop_to_pixels, draw_decoded_image, draw_image_cached, fit_src_dst,
};
pub use paint::{fill_to_paint, stroke_to_paint};
pub use path::to_sk_path;
// `vector_outline_sk_path`: track svg-prod — the effective vector outline,
// shared with the app's flatten/outline/simplify geometry ops.
pub use raster::{
    MediaPlayback, RasterRenderer, RenderError, RenderInputs, RenderMetrics, measure_text_node,
    solve_scene_layout, text_caret_rect, text_hit_test, text_line_height, text_node_outline,
    text_selection_rects, vector_outline_sk_path,
};
pub use transform::to_sk_matrix;
