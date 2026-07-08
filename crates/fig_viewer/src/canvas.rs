//! The canvas element: paints the rendered scene (through Skia Metal on
//! macOS, with a CPU fallback), then the interaction overlays — hover and
//! selection outlines, resize handles, and the active tool's render hints.

use std::sync::Arc;

use anyhow::{Context as _, Result, anyhow};
#[cfg(target_os = "macos")]
use core_foundation::{
    base::{CFType, TCFType},
    boolean::CFBoolean,
    dictionary::CFDictionary,
    string::CFString,
};
#[cfg(target_os = "macos")]
use core_video::{
    metal_texture::CVMetalTextureGetTexture,
    metal_texture_cache::CVMetalTextureCache,
    pixel_buffer::{CVPixelBuffer, CVPixelBufferKeys, kCVPixelFormatType_32BGRA},
};
use fanta_canvas::ResizeHandle;
use fanta_doc::{NodeId, Viewport};
use fanta_render::{RasterRenderer, RenderInputs};
use fanta_tools::{SnapGuideAxis, ToolOverlay};
#[cfg(target_os = "macos")]
use foreign_types::ForeignType;
use glam::DVec2;
use gpui::{
    App, BorderStyle, Bounds, DispatchPhase, Element, ElementId, Entity, Font, GlobalElementId,
    Hsla, InspectorElementId, IntoElement, LayoutId, MouseMoveEvent, MouseUpEvent, PathBuilder,
    Pixels, Point, RenderImage, ShapedLine, TextAlign, TextRun, Window, font, point, px, relative,
    size,
};
use image::{Frame, RgbaImage};
#[cfg(target_os = "macos")]
use skia_safe::{
    ColorType,
    gpu::{self, SurfaceOrigin, backend_render_targets, direct_contexts, mtl},
};
use smallvec::SmallVec;
use ui::prelude::*;

use crate::document::FigDocument;
use crate::view::FigView;

const HANDLE_SIZE: f32 = 7.0;

/// Screen-space font size (logical px) for the on-canvas frame name labels and
/// the badge/measurement readouts. Fixed regardless of zoom so the chrome stays
/// legible, matching Figma.
const LABEL_FONT_SIZE: f32 = 11.0;

/// Figma's measurement-guide red (`#F24B4B`) — distinct from the accent-blue
/// selection chrome so spacing reads unambiguously over either canvas.
const MEASURE_RED: (u8, u8, u8) = (0xF2, 0x4B, 0x4B);

/// Screen-space height (logical px) of the badge/measurement pills.
const PILL_HEIGHT: Pixels = px(16.0);

/// Extra logical pixels rendered beyond each element edge. Reprojected frames
/// can then cover pans (and modest zoom-outs) without exposing unrendered
/// edges before the next fresh render lands.
#[cfg(target_os = "macos")]
const RENDER_MARGIN: f32 = 160.0;

/// Renders one page of the document plus interaction overlays, filling the
/// available space.
pub(crate) struct CanvasElement {
    view: Entity<FigView>,
}

impl CanvasElement {
    pub(crate) fn new(view: Entity<FigView>) -> Self {
        Self { view }
    }
}

pub(crate) struct RenderedCanvas {
    image: Arc<RenderImage>,
    size: (u32, u32),
    viewport: Viewport,
    revision: u64,
    page_root: Option<NodeId>,
}

enum PaintCanvas {
    #[cfg(target_os = "macos")]
    Surface {
        frame: GpuFrame,
        viewport: Viewport,
    },
    Image(Arc<RenderImage>),
}

/// Where a frame rendered at `cached` must be painted so its world content
/// lines up under the `current` viewport: the cached image covers the element
/// rect at its own zoom, so scale it by the zoom ratio and shift it by the
/// screen-space distance between the two centers.
#[cfg(target_os = "macos")]
fn reprojected_bounds(
    bounds: Bounds<Pixels>,
    cached: Viewport,
    current: Viewport,
) -> Bounds<Pixels> {
    let (width, height) = bounds_size(bounds);
    let scale = current.zoom / cached.zoom.max(f64::EPSILON);
    let center_x = (cached.center[0] - current.center[0]) * current.zoom + width * 0.5;
    let center_y = (cached.center[1] - current.center[1]) * current.zoom + height * 0.5;
    let projected_width = width * scale;
    let projected_height = height * scale;
    Bounds {
        origin: point(
            bounds.origin.x + px((center_x - projected_width * 0.5) as f32),
            bounds.origin.y + px((center_y - projected_height * 0.5) as f32),
        ),
        size: size(px(projected_width as f32), px(projected_height as f32)),
    }
}

/// Key identifying one rendered frame. While it matches, repaints reuse the
/// previous surface without touching the GPU.
#[cfg(target_os = "macos")]
#[derive(PartialEq, Clone, Copy)]
struct SurfaceKey {
    size: (u32, u32),
    viewport_center: [f64; 2],
    viewport_zoom: f64,
    page_root: Option<NodeId>,
    revision: u64,
}

#[cfg(target_os = "macos")]
struct CachedSurface {
    buffer: CVPixelBuffer,
    key: SurfaceKey,
}

#[cfg(target_os = "macos")]
pub(crate) struct MacGpuRenderer {
    raster_renderer: RasterRenderer,
    direct_context: skia_safe::gpu::DirectContext,
    texture_cache: CVMetalTextureCache,
    _device: metal::Device,
    _command_queue: metal::CommandQueue,
    size: (u32, u32),
    /// Recycled pixel buffers matching `size`. Reusing them avoids a
    /// multi-megabyte IOSurface allocation per frame.
    buffer_pool: Vec<CVPixelBuffer>,
    /// Buffers handed to the compositor recently. They only graduate to the
    /// pool after `IN_FLIGHT_FRAMES` newer frames were presented, because the
    /// window server may still be scanning them out — rendering into one too
    /// early tears or corrupts the displayed frame.
    retired: std::collections::VecDeque<CVPixelBuffer>,
    cached: Option<CachedSurface>,
    last_render_at: Option<std::time::Instant>,
}

/// One frame's worth of pixels for the canvas.
#[cfg(target_os = "macos")]
pub(crate) enum GpuFrame {
    /// Rendered at the requested viewport; paint it 1:1 over the element.
    Fresh(CVPixelBuffer),
    /// The cached frame from an earlier `viewport`, reused because a fresh
    /// render is throttled mid-interaction. The caller scales/offsets it to
    /// approximate the requested viewport and repaints soon after.
    Reprojected {
        buffer: CVPixelBuffer,
        viewport: Viewport,
    },
}

#[cfg(target_os = "macos")]
impl MacGpuRenderer {
    const BUFFER_POOL_LIMIT: usize = 3;
    const IN_FLIGHT_FRAMES: usize = 2;
    /// Cap on how often the scene is re-rendered while the viewport is
    /// animating. Frames in between reuse the previous render reprojected,
    /// which keeps pans and zooms at display rate no matter the scene cost —
    /// the same blurry-then-sharp trade design tools like Figma make.
    const MIN_RENDER_INTERVAL: std::time::Duration = std::time::Duration::from_millis(33);
    /// Beyond this zoom ratio a reprojected frame is too blurry or too sparse
    /// to be useful; render fresh even mid-interaction.
    const MAX_REPROJECT_SCALE: f64 = 3.0;

    pub(crate) fn new(size: (u32, u32)) -> Result<Self> {
        let device = metal::Device::system_default()
            .ok_or_else(|| anyhow!("Metal system default device is unavailable"))?;
        let command_queue = device.new_command_queue();
        let backend = unsafe {
            // Ganesh retains the Objective-C device and queue handles while the
            // DirectContext lives; this renderer stores both owners alongside it.
            mtl::BackendContext::new(
                device.as_ptr() as mtl::Handle,
                command_queue.as_ptr() as mtl::Handle,
            )
        };
        let direct_context = direct_contexts::make_metal(&backend, None)
            .ok_or_else(|| anyhow!("creating Skia Metal context failed"))?;
        let texture_cache = CVMetalTextureCache::new(None, device.clone(), None)
            .map_err(|status| anyhow!("creating CoreVideo Metal texture cache failed: {status}"))?;
        let raster_renderer =
            RasterRenderer::new(size.0, size.1).context("creating Skia renderer state")?;

        Ok(Self {
            raster_renderer,
            direct_context,
            texture_cache,
            _device: device,
            _command_queue: command_queue,
            size,
            buffer_pool: Vec::new(),
            retired: std::collections::VecDeque::new(),
            cached: None,
            last_render_at: None,
        })
    }

    pub(crate) fn invalidate(&mut self) {
        if let Some(cached) = self.cached.take() {
            self.retire(cached.buffer);
        }
    }

    /// Queue a previously presented buffer for reuse once enough newer frames
    /// have shipped that the compositor cannot still be reading it.
    fn retire(&mut self, buffer: CVPixelBuffer) {
        self.retired.push_back(buffer);
        while self.retired.len() > Self::IN_FLIGHT_FRAMES {
            if let Some(buffer) = self.retired.pop_front() {
                self.recycle(buffer);
            }
        }
    }

    fn recycle(&mut self, buffer: CVPixelBuffer) {
        if self.buffer_pool.len() < Self::BUFFER_POOL_LIMIT {
            self.buffer_pool.push(buffer);
        }
    }

    fn resize(&mut self, size: (u32, u32)) -> Result<()> {
        if self.size == size {
            return Ok(());
        }

        // `RasterRenderer::resize` keeps the image and instance caches warm,
        // unlike rebuilding the renderer.
        self.raster_renderer
            .resize(size.0, size.1)
            .context("resizing Skia renderer state")?;
        self.size = size;
        self.buffer_pool.clear();
        self.retired.clear();
        self.cached = None;
        Ok(())
    }

    fn take_buffer(&mut self, size: (u32, u32)) -> Result<CVPixelBuffer> {
        if let Some(buffer) = self.buffer_pool.pop() {
            return Ok(buffer);
        }
        create_bgra_pixel_buffer(size.0, size.1)
    }

    fn render(
        &mut self,
        document: &FigDocument,
        page_root: Option<NodeId>,
        size: (u32, u32),
        frame_logical: (f64, f64),
        visible_logical: (f64, f64),
        viewport: Viewport,
        scale_factor: f32,
    ) -> Result<GpuFrame> {
        let key = SurfaceKey {
            size,
            viewport_center: viewport.center,
            viewport_zoom: viewport.zoom,
            page_root,
            revision: document.doc.scene.revision(),
        };
        if let Some(cached) = &self.cached
            && cached.key == key
        {
            return Ok(GpuFrame::Fresh(cached.buffer.clone()));
        }

        // Mid-interaction, reuse the previous frame reprojected instead of
        // paying for a full scene render on every viewport tick — but only
        // when the cached frame's world coverage still contains everything
        // the element must show. A partially covering frame would flash
        // background at the leading edges until the next fresh render, which
        // reads as edge jumping while panning or zooming out.
        if let Some(cached) = &self.cached
            && cached.key.size == size
            && cached.key.page_root == page_root
            && cached.key.revision == key.revision
            && self
                .last_render_at
                .is_some_and(|at| at.elapsed() < Self::MIN_RENDER_INTERVAL)
        {
            let cached_zoom = cached.key.viewport_zoom.max(f64::EPSILON);
            let scale = viewport.zoom / cached_zoom;
            let covered_width = frame_logical.0 / cached_zoom;
            let covered_height = frame_logical.1 / cached_zoom;
            let needed_width = visible_logical.0 / viewport.zoom.max(f64::EPSILON);
            let needed_height = visible_logical.1 / viewport.zoom.max(f64::EPSILON);
            let slack_x = (covered_width - needed_width) * 0.5;
            let slack_y = (covered_height - needed_height) * 0.5;
            let offset_x = (cached.key.viewport_center[0] - viewport.center[0]).abs();
            let offset_y = (cached.key.viewport_center[1] - viewport.center[1]).abs();
            let covers_view = offset_x <= slack_x && offset_y <= slack_y;
            if covers_view
                && (1.0 / Self::MAX_REPROJECT_SCALE..=Self::MAX_REPROJECT_SCALE).contains(&scale)
            {
                return Ok(GpuFrame::Reprojected {
                    buffer: cached.buffer.clone(),
                    viewport: Viewport {
                        center: cached.key.viewport_center,
                        zoom: cached.key.viewport_zoom,
                    },
                });
            }
        }

        self.resize(size)?;
        if let Some(asset_resolver) = document.asset_resolver.clone() {
            self.raster_renderer.set_asset_resolver(asset_resolver);
        }

        let pixel_buffer = self.take_buffer(size)?;
        let color_texture = self
            .texture_cache
            .create_texture_from_image(
                pixel_buffer.as_concrete_TypeRef(),
                None,
                metal::MTLPixelFormat::BGRA8Unorm,
                size.0 as usize,
                size.1 as usize,
                0,
            )
            .map_err(|status| anyhow!("creating CoreVideo Metal texture failed: {status}"))?;
        let texture = unsafe { CVMetalTextureGetTexture(color_texture.as_concrete_TypeRef()) };
        if texture.is_null() {
            self.recycle(pixel_buffer);
            return Err(anyhow!("CoreVideo returned a null Metal texture"));
        }

        let texture_info = unsafe { mtl::TextureInfo::new(texture as mtl::Handle) };
        let backend_render_target =
            backend_render_targets::make_mtl((size.0 as i32, size.1 as i32), &texture_info);
        let mut surface = gpu::surfaces::wrap_backend_render_target(
            &mut self.direct_context,
            &backend_render_target,
            SurfaceOrigin::TopLeft,
            ColorType::BGRA8888,
            None,
            None,
        )
        .ok_or_else(|| anyhow!("wrapping Metal texture as Skia surface failed"))?;

        let render_viewport = Viewport {
            center: viewport.center,
            zoom: viewport.zoom * f64::from(scale_factor),
        };
        let inputs = RenderInputs {
            components: &document.doc.components,
            variables: &document.doc.variables,
            active_modes: &document.doc.active_modes,
            mode_generation: 0,
            playback: None,
            dark_ui: false,
        };
        self.raster_renderer.render_to_canvas(
            surface.canvas(),
            size.0,
            size.1,
            &document.doc.scene,
            &render_viewport,
            page_root,
            &inputs,
        );
        // The compositor samples the IOSurface as soon as we hand it to
        // `paint_surface`, so the GPU work must be complete by then.
        self.direct_context.flush_submit_and_sync_cpu();
        drop(surface);

        if let Some(previous) = self.cached.take() {
            self.retire(previous.buffer);
        }
        self.cached = Some(CachedSurface {
            buffer: pixel_buffer.clone(),
            key,
        });
        self.last_render_at = Some(std::time::Instant::now());
        Ok(GpuFrame::Fresh(pixel_buffer))
    }
}

#[cfg(target_os = "macos")]
fn create_bgra_pixel_buffer(width: u32, height: u32) -> Result<CVPixelBuffer> {
    let io_surface_options = CFDictionary::<CFString, CFType>::from_CFType_pairs(&[]);
    let options = CFDictionary::<CFString, CFType>::from_CFType_pairs(&[
        (
            CFString::from(CVPixelBufferKeys::IOSurfaceProperties),
            io_surface_options.as_CFType(),
        ),
        (
            CFString::from(CVPixelBufferKeys::MetalCompatibility),
            CFBoolean::true_value().as_CFType(),
        ),
    ]);
    CVPixelBuffer::new(
        kCVPixelFormatType_32BGRA,
        width as usize,
        height as usize,
        Some(&options),
    )
    .map_err(|status| anyhow!("creating BGRA CVPixelBuffer failed: {status}"))
}

fn render_fig_canvas(
    document: &FigDocument,
    page_root: Option<NodeId>,
    width: u32,
    height: u32,
    viewport: Viewport,
    scale_factor: f32,
) -> Result<Arc<RenderImage>> {
    let mut renderer =
        RasterRenderer::new(width, height).context("creating Skia raster surface")?;
    if let Some(asset_resolver) = document.asset_resolver.clone() {
        renderer.set_asset_resolver(asset_resolver);
    }

    let render_viewport = Viewport {
        center: viewport.center,
        zoom: viewport.zoom * f64::from(scale_factor),
    };
    let inputs = RenderInputs {
        components: &document.doc.components,
        variables: &document.doc.variables,
        active_modes: &document.doc.active_modes,
        mode_generation: 0,
        playback: None,
        dark_ui: false,
    };
    renderer.render_page_with(&document.doc.scene, &render_viewport, page_root, &inputs);
    render_image_from_rgba(width, height, renderer.copy_rgba(), scale_factor)
}

fn render_image_from_rgba(
    width: u32,
    height: u32,
    mut pixels: Vec<u8>,
    scale_factor: f32,
) -> Result<Arc<RenderImage>> {
    for pixel in pixels.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    let image = RgbaImage::from_raw(width, height, pixels)
        .ok_or_else(|| anyhow!("Skia returned an invalid RGBA buffer"))?;
    Ok(Arc::new(
        RenderImage::new(SmallVec::from_elem(Frame::new(image), 1)).with_scale_factor(scale_factor),
    ))
}

impl FigView {
    pub(crate) fn render_cpu_canvas(
        &mut self,
        document: &FigDocument,
        page_root: Option<NodeId>,
        size: (u32, u32),
        viewport: Viewport,
        scale_factor: f32,
    ) -> Result<Arc<RenderImage>> {
        let revision = document.doc.scene.revision();
        if let Some(rendered) = &self.rendered_canvas
            && rendered.size == size
            && rendered.page_root == page_root
            && rendered.revision == revision
            && same_viewport(rendered.viewport, viewport)
        {
            return Ok(rendered.image.clone());
        }

        let image = render_fig_canvas(document, page_root, size.0, size.1, viewport, scale_factor)?;
        self.rendered_canvas = Some(RenderedCanvas {
            image: image.clone(),
            size,
            viewport,
            revision,
            page_root,
        });
        Ok(image)
    }
}

impl IntoElement for CanvasElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

/// Which window-level drag listeners the paint pass must install for the
/// gestures currently in flight.
pub(crate) struct DragListeners {
    panning: bool,
    primary_drag: bool,
}

impl Element for CanvasElement {
    type RequestLayoutState = ();
    type PrepaintState = Option<DragListeners>;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        (
            window.request_layout(
                gpui::Style {
                    size: size(relative(1.).into(), relative(1.).into()),
                    ..Default::default()
                },
                [],
                cx,
            ),
            (),
        )
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let logical_size = bounds_size(bounds);
        let (viewport, drag_listeners) = {
            let view = self.view.read(cx);
            let item = view.item().read(cx);
            let Some(document) = item.document() else {
                return None;
            };
            let Some(page) = document.page(view.selected_page_index()) else {
                return None;
            };
            let viewport = view.viewport().unwrap_or_else(|| {
                crate::document::fit_bounds(
                    page.bounds,
                    logical_size,
                    crate::view::RENDER_PADDING,
                    crate::view::MIN_ZOOM,
                    crate::view::MAX_ZOOM,
                )
            });
            (
                viewport,
                DragListeners {
                    panning: view.is_panning(),
                    primary_drag: view.primary_pressed(),
                },
            )
        };

        self.view.update(cx, |this, _| {
            this.set_container_bounds(bounds);
            this.set_viewport_silent(viewport);
        });
        Some(drag_listeners)
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let Some(listeners) = prepaint.take() else {
            return;
        };

        if listeners.panning {
            let view = self.view.downgrade();
            window.on_mouse_event(move |_event: &MouseUpEvent, phase, _window, cx| {
                if phase == DispatchPhase::Bubble
                    && let Some(view) = view.upgrade()
                {
                    view.update(cx, |this, cx| {
                        this.end_panning(cx);
                    });
                }
            });
        }

        // While a primary drag is live, window-level listeners own the
        // move/release stream: element listeners stop firing once the cursor
        // leaves the canvas, which would strand the tool mid-gesture.
        if listeners.primary_drag {
            let view = self.view.downgrade();
            window.on_mouse_event(move |event: &MouseMoveEvent, phase, _window, cx| {
                if phase == DispatchPhase::Bubble
                    && let Some(view) = view.upgrade()
                {
                    view.update(cx, |this, cx| {
                        this.handle_window_mouse_move(event, cx);
                    });
                }
            });
            let view = self.view.downgrade();
            window.on_mouse_event(move |event: &MouseUpEvent, phase, _window, cx| {
                if phase == DispatchPhase::Bubble
                    && let Some(view) = view.upgrade()
                {
                    view.update(cx, |this, cx| {
                        this.handle_window_mouse_up(event, cx);
                    });
                }
            });
        }

        let scale_factor = window.scale_factor();
        // The GPU frame is rendered with a margin beyond the element so
        // reprojection during pans has real pixels at the edges.
        #[cfg(target_os = "macos")]
        let frame_bounds = {
            let margin = px(RENDER_MARGIN);
            Bounds {
                origin: point(bounds.origin.x - margin, bounds.origin.y - margin),
                size: size(
                    bounds.size.width + margin * 2.,
                    bounds.size.height + margin * 2.,
                ),
            }
        };
        #[cfg(not(target_os = "macos"))]
        let frame_bounds = bounds;
        let render_size = render_size_for_bounds(frame_bounds, scale_factor);
        let paint_started = std::time::Instant::now();
        let paint_canvas = self.view.update(cx, |this, cx| {
            let viewport = this
                .viewport()
                .ok_or_else(|| anyhow!("Figma canvas viewport was not initialized"))?;
            let item = this.item().clone();
            let item = item.read(cx);
            let document = item
                .document()
                .ok_or_else(|| anyhow!("Figma document is not ready"))?;
            let page_root = document
                .page(this.selected_page_index())
                .map(|page| page.root)
                .ok_or_else(|| anyhow!("Figma document has no renderable pages"))?;

            #[cfg(target_os = "macos")]
            {
                let mut gpu_renderer = match this.take_gpu_renderer() {
                    Some(renderer) => renderer,
                    None => MacGpuRenderer::new(render_size)?,
                };
                let gpu_result = gpu_renderer.render(
                    document,
                    page_root,
                    render_size,
                    bounds_size(frame_bounds),
                    bounds_size(bounds),
                    viewport,
                    scale_factor,
                );
                this.store_gpu_renderer(gpu_renderer);
                match gpu_result {
                    Ok(frame) => {
                        this.clear_rendered_canvas();
                        if matches!(frame, GpuFrame::Reprojected { .. }) {
                            // Repaint immediately so a fresh render lands as
                            // soon as the throttle window opens; the chain
                            // stops once the cache matches the viewport.
                            cx.notify();
                        }
                        return Ok(PaintCanvas::Surface { frame, viewport });
                    }
                    Err(error) => {
                        log::warn!(
                            "failed to render .fig canvas with Skia Metal, falling back to CPU: {error:#}"
                        );
                    }
                }
            }

            this.render_cpu_canvas(document, page_root, render_size, viewport, scale_factor)
                .map(PaintCanvas::Image)
        });
        crate::report_slow("canvas scene render", paint_started);

        match paint_canvas {
            #[cfg(target_os = "macos")]
            Ok(PaintCanvas::Surface { frame, viewport }) => match frame {
                GpuFrame::Fresh(surface) => {
                    window.with_content_mask(Some(gpui::ContentMask { bounds }), |window| {
                        window.paint_surface(frame_bounds, surface);
                    });
                }
                GpuFrame::Reprojected {
                    buffer,
                    viewport: cached_viewport,
                } => {
                    let projected = reprojected_bounds(frame_bounds, cached_viewport, viewport);
                    window.with_content_mask(Some(gpui::ContentMask { bounds }), |window| {
                        window.paint_surface(projected, buffer);
                    });
                }
            },
            Ok(PaintCanvas::Image(image)) => {
                if let Err(error) = window
                    .with_content_mask(Some(gpui::ContentMask { bounds }), |window| {
                        window.paint_image(frame_bounds, Default::default(), image, 0, false)
                    })
                {
                    log::warn!("failed to paint .fig CPU canvas: {error:#}");
                }
            }
            Err(error) => {
                log::warn!("failed to render .fig canvas: {error:#}");
            }
        }

        self.paint_overlays(bounds, window, cx);
    }
}

/// A top-level frame's name label to paint above its top-left corner.
struct FrameLabel {
    /// The frame's world bounds; its projected top-left anchors the label.
    world: fanta_doc::Bounds,
    name: String,
    /// The selected frame's label is drawn in the accent color; others muted.
    selected: bool,
}

/// Owned overlay data gathered while the document is borrowed, so the paint
/// pass (which needs `&mut App` for text) no longer holds that borrow.
struct OverlayData {
    frame_labels: Vec<FrameLabel>,
    /// Union of every selected node's world bounds, for the size badge.
    selection_union: Option<fanta_doc::Bounds>,
    /// The selection's world size shown in the badge, once nodes are selected.
    selection_size: Option<(f64, f64)>,
    /// Measurement gap segments between the single selection and the hovered
    /// node, when the alt-hover-style measure condition holds.
    measure_segments: Vec<GapSegment>,
}

impl CanvasElement {
    /// Gather the frame-label, selection-badge, and measurement geometry from
    /// the document into owned values. Done up front so the borrow of `cx` (via
    /// the view/document) is released before the paint pass, which needs a
    /// mutable `cx` to shape and paint text.
    fn collect_overlay_data(&self, cx: &App) -> OverlayData {
        let mut data = OverlayData {
            frame_labels: Vec::new(),
            selection_union: None,
            selection_size: None,
            measure_segments: Vec::new(),
        };
        let view = self.view.read(cx);
        let item = view.item().read(cx);
        let Some(document) = item.document() else {
            return data;
        };
        let doc = &document.doc;

        // Frame name labels: only frame-surface groups that are direct children
        // of the active page (top-level frames/sections, like Figma) — nested
        // frames would be noise.
        let page_children: &[NodeId] = doc.scene.children_of(doc.active_page());
        for &id in page_children {
            let Some(node) = doc.scene.get(id) else {
                continue;
            };
            let is_frame =
                matches!(&node.data, fanta_doc::NodeData::Group(group) if group.is_frame_surface());
            if !is_frame {
                continue;
            }
            let Some(world) = doc.scene.world_bounds(id) else {
                continue;
            };
            let name = node.name.trim();
            let name = if name.is_empty() { "Frame" } else { name };
            data.frame_labels.push(FrameLabel {
                world,
                name: name.to_string(),
                selected: doc.selection.contains(id),
            });
        }

        // Selection union + size badge.
        for &id in doc.selection.iter() {
            if let Some(world) = doc.scene.world_bounds(id) {
                data.selection_union = Some(match data.selection_union {
                    Some(existing) => existing.union(&world),
                    None => world,
                });
            }
        }
        if let Some(union) = data.selection_union {
            data.selection_size = Some((union.width(), union.height()));
        }

        // Measurements: exactly one node selected and a different, non-related
        // node hovered. The ancestor/descendant guard keeps the guides from
        // firing between a node and its own container (pure noise).
        if let (&[selected_id], Some(hovered_id)) = (doc.selection.as_slice(), view.hovered_node())
            && hovered_id != selected_id
            && !is_related(&doc.scene, selected_id, hovered_id)
            && let Some(selected_world) = doc.scene.world_bounds(selected_id)
            && let Some(hovered_world) = doc.scene.world_bounds(hovered_id)
        {
            data.measure_segments = edge_gaps(selected_world, hovered_world);
        }

        data
    }

    fn paint_overlays(&self, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut App) {
        let overlay_data = self.collect_overlay_data(cx);

        let view = self.view.read(cx);
        let Some(viewport) = view.viewport() else {
            return;
        };
        let item = view.item().read(cx);
        let Some(document) = item.document() else {
            return;
        };

        let accent = cx.theme().players().local().cursor;
        let selection_fill = cx.theme().players().local().selection;
        let label_muted = cx.theme().colors().text_muted;
        let screen_size = bounds_size(bounds);
        let project = |world: DVec2| -> Point<Pixels> {
            let screen = fanta_canvas::world_to_screen(
                world,
                &viewport,
                DVec2::new(screen_size.0, screen_size.1),
            );
            point(
                bounds.origin.x + px(screen.x as f32),
                bounds.origin.y + px(screen.y as f32),
            )
        };
        let project_bounds = |world: fanta_doc::Bounds| -> Bounds<Pixels> {
            let min = project(DVec2::new(world.min_x, world.min_y));
            let max = project(DVec2::new(world.max_x, world.max_y));
            Bounds::from_corners(min, max)
        };

        window.with_content_mask(Some(gpui::ContentMask { bounds }), |window| {
            // Hover highlight under the selection so the selected outline wins.
            if let Some(hovered) = view.hovered_node()
                && !document.doc.selection.contains(hovered)
                && let Some(world) = document.doc.scene.world_bounds(hovered)
            {
                window.paint_quad(gpui::outline(
                    project_bounds(world),
                    accent.opacity(0.55),
                    BorderStyle::Solid,
                ));
            }

            let selected: Vec<NodeId> = document.doc.selection.iter().copied().collect();
            let mut selection_union: Option<fanta_doc::Bounds> = None;
            for id in &selected {
                let Some(world) = document.doc.scene.world_bounds(*id) else {
                    continue;
                };
                selection_union = Some(match selection_union {
                    Some(existing) => existing.union(&world),
                    None => world,
                });
                window.paint_quad(gpui::outline(
                    project_bounds(world),
                    accent,
                    BorderStyle::Solid,
                ));
            }

            // Resize handles on the selection box, Figma-style, only for the
            // select tool.
            if view.tools().kind() == crate::tools::ToolKind::Select
                && let Some(union) = selection_union
            {
                let handle_px = px(HANDLE_SIZE);
                for handle in ResizeHandle::ALL {
                    let world = handle.handle_world(union);
                    let center = project(world);
                    let handle_bounds = Bounds {
                        origin: point(center.x - handle_px / 2., center.y - handle_px / 2.),
                        size: size(handle_px, handle_px),
                    };
                    window.paint_quad(gpui::quad(
                        handle_bounds,
                        px(1.),
                        gpui::white(),
                        px(1.),
                        accent,
                        BorderStyle::Solid,
                    ));
                }
            }

            for overlay in &view.tools().overlays {
                match overlay {
                    ToolOverlay::Marquee { screen_rect } => {
                        let marquee = Bounds::from_corners(
                            point(
                                bounds.origin.x + px(screen_rect.min_x as f32),
                                bounds.origin.y + px(screen_rect.min_y as f32),
                            ),
                            point(
                                bounds.origin.x + px(screen_rect.max_x as f32),
                                bounds.origin.y + px(screen_rect.max_y as f32),
                            ),
                        );
                        window.paint_quad(gpui::fill(marquee, selection_fill.opacity(0.15)));
                        window.paint_quad(gpui::outline(marquee, accent, BorderStyle::Solid));
                    }
                    ToolOverlay::SnapGuide(guide) => {
                        let guide_bounds = match guide.axis {
                            SnapGuideAxis::Vertical => {
                                let x = project(DVec2::new(guide.world_position, 0.0)).x;
                                Bounds {
                                    origin: point(x, bounds.origin.y),
                                    size: size(px(1.), bounds.size.height),
                                }
                            }
                            SnapGuideAxis::Horizontal => {
                                let y = project(DVec2::new(0.0, guide.world_position)).y;
                                Bounds {
                                    origin: point(bounds.origin.x, y),
                                    size: size(bounds.size.width, px(1.)),
                                }
                            }
                        };
                        window.paint_quad(gpui::fill(guide_bounds, accent.opacity(0.8)));
                    }
                    ToolOverlay::PreviewRect { world_rect } => {
                        let preview = project_bounds(*world_rect);
                        window.paint_quad(gpui::fill(preview, selection_fill.opacity(0.1)));
                        window.paint_quad(gpui::outline(preview, accent, BorderStyle::Solid));
                    }
                    ToolOverlay::PreviewEllipse { world_rect } => {
                        paint_ellipse_outline(project_bounds(*world_rect), accent, window);
                    }
                    ToolOverlay::PreviewLine {
                        world_start,
                        world_end,
                    } => {
                        let start = project(DVec2::new(world_start[0], world_start[1]));
                        let end = project(DVec2::new(world_end[0], world_end[1]));
                        paint_line(start, end, accent, window);
                    }
                    ToolOverlay::PathAnchor { world, selected } => {
                        let center = project(DVec2::new(world[0], world[1]));
                        let anchor_px = px(6.);
                        let anchor_bounds = Bounds {
                            origin: point(center.x - anchor_px / 2., center.y - anchor_px / 2.),
                            size: size(anchor_px, anchor_px),
                        };
                        let fill: Hsla = if *selected { accent } else { gpui::white() };
                        window.paint_quad(gpui::quad(
                            anchor_bounds,
                            px(1.),
                            fill,
                            px(1.),
                            accent,
                            BorderStyle::Solid,
                        ));
                    }
                    ToolOverlay::PathHandle {
                        world_anchor,
                        world_ctrl,
                    } => {
                        let anchor = project(DVec2::new(world_anchor[0], world_anchor[1]));
                        let ctrl = project(DVec2::new(world_ctrl[0], world_ctrl[1]));
                        paint_line(anchor, ctrl, accent.opacity(0.7), window);
                        let dot = px(4.);
                        window.paint_quad(gpui::quad(
                            Bounds {
                                origin: point(ctrl.x - dot / 2., ctrl.y - dot / 2.),
                                size: size(dot, dot),
                            },
                            dot / 2.,
                            gpui::white(),
                            px(1.),
                            accent,
                            BorderStyle::Solid,
                        ));
                    }
                    ToolOverlay::PathInsertHint { world } => {
                        let center = project(DVec2::new(world[0], world[1]));
                        let dot = px(5.);
                        window.paint_quad(gpui::quad(
                            Bounds {
                                origin: point(center.x - dot / 2., center.y - dot / 2.),
                                size: size(dot, dot),
                            },
                            dot / 2.,
                            accent,
                            px(0.),
                            gpui::transparent_black(),
                            BorderStyle::Solid,
                        ));
                    }
                }
            }
        });

        // Text-carrying overlays run in a second masked pass: shaping and
        // painting glyphs needs a mutable `App`, which the first pass could not
        // hold while the document was borrowed for the geometry above.
        self.paint_text_overlays(
            bounds,
            &overlay_data,
            accent,
            label_muted,
            project,
            project_bounds,
            window,
            cx,
        );
    }

    /// Frame name labels, the selection size badge, and Figma-style measurement
    /// guides — every overlay that carries text. Split from `paint_overlays`
    /// because these need `&mut App` for text shaping/painting.
    #[allow(clippy::too_many_arguments)]
    fn paint_text_overlays(
        &self,
        bounds: Bounds<Pixels>,
        data: &OverlayData,
        accent: Hsla,
        label_muted: Hsla,
        project: impl Fn(DVec2) -> Point<Pixels>,
        project_bounds: impl Fn(fanta_doc::Bounds) -> Bounds<Pixels>,
        window: &mut Window,
        cx: &mut App,
    ) {
        let ui_font = font(".SystemUIFont");
        let measure_red = {
            let (r, g, b) = MEASURE_RED;
            gpui::rgb(((r as u32) << 16) | ((g as u32) << 8) | b as u32).into()
        };

        // Everything here paints inside the canvas content mask so overlays
        // never spill past the canvas edges. The closure captures `cx` (a
        // distinct `&mut App` from the `window` it receives) so `ShapedLine`
        // painting works while the mask is on the stack.
        window.with_content_mask(Some(gpui::ContentMask { bounds }), |window| {
            for label in &data.frame_labels {
                let top_left = project_bounds(label.world).origin;
                let color = if label.selected { accent } else { label_muted };
                let line = shape_label(&label.name, color, &ui_font, window);
                // Float the name just above the frame's top-left corner (Figma's
                // loose frame title), at a fixed screen size.
                let origin = point(top_left.x, top_left.y - px(LABEL_FONT_SIZE + 5.0));
                if let Err(error) = line.paint(
                    origin,
                    px(LABEL_FONT_SIZE + 4.0),
                    TextAlign::Left,
                    None,
                    window,
                    cx,
                ) {
                    log::warn!("failed to paint canvas frame label: {error:#}");
                }
            }

            // Selection size badge, centered just below the selection bounds.
            if let (Some(union), Some((width, height))) =
                (data.selection_union, data.selection_size)
            {
                let rect = project_bounds(union);
                let label = format!("{} × {}", width.round() as i64, height.round() as i64);
                let line = shape_label(&label, gpui::white(), &ui_font, window);
                let top_center = point(
                    rect.origin.x + rect.size.width / 2.0,
                    rect.bottom() + px(8.0),
                );
                paint_pill(top_center, &line, accent, window, cx);
            }

            // Measurement guides: red line + centered px label per gap segment.
            for segment in &data.measure_segments {
                let start = project(segment.start);
                let end = project(segment.end);
                paint_line(start, end, measure_red, window);
                let label = format!("{}", segment.dist.round() as i64);
                let line = shape_label(&label, gpui::white(), &ui_font, window);
                // Center the pill on the guide's midpoint: shift up by half the
                // pill height so its top-center anchor lands the box on the line.
                let mid = point(
                    (start.x + end.x) / 2.0,
                    (start.y + end.y) / 2.0 - PILL_HEIGHT / 2.0,
                );
                paint_pill(mid, &line, measure_red, window, cx);
            }
        });
    }
}

/// Whether `a` and `b` are on the same ancestor chain — one contains the other.
/// Used to suppress measurement guides between a node and its own container.
fn is_related(scene: &fanta_doc::Scene, a: NodeId, b: NodeId) -> bool {
    scene.ancestors_of(a).any(|node| node.id == b) || scene.ancestors_of(b).any(|node| node.id == a)
}

fn paint_line(start: Point<Pixels>, end: Point<Pixels>, color: Hsla, window: &mut Window) {
    let mut builder = PathBuilder::stroke(px(1.));
    builder.move_to(start);
    builder.line_to(end);
    match builder.build() {
        Ok(path) => window.paint_path(path, color),
        Err(error) => log::warn!("failed to build canvas overlay line: {error:#}"),
    }
}

fn paint_ellipse_outline(bounds: Bounds<Pixels>, color: Hsla, window: &mut Window) {
    const SEGMENTS: usize = 48;
    let center_x = f32::from(bounds.origin.x) + f32::from(bounds.size.width) / 2.0;
    let center_y = f32::from(bounds.origin.y) + f32::from(bounds.size.height) / 2.0;
    let radius_x = f32::from(bounds.size.width) / 2.0;
    let radius_y = f32::from(bounds.size.height) / 2.0;

    let mut builder = PathBuilder::stroke(px(1.));
    for segment in 0..=SEGMENTS {
        let angle = segment as f32 / SEGMENTS as f32 * std::f32::consts::TAU;
        let position = point(
            px(center_x + radius_x * angle.cos()),
            px(center_y + radius_y * angle.sin()),
        );
        if segment == 0 {
            builder.move_to(position);
        } else {
            builder.line_to(position);
        }
    }
    match builder.build() {
        Ok(path) => window.paint_path(path, color),
        Err(error) => log::warn!("failed to build canvas ellipse preview: {error:#}"),
    }
}

/// The axis a [`GapSegment`] measures along.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MeasureAxis {
    Horizontal,
    Vertical,
}

/// One measured edge-to-edge gap between the selected box and the hovered box,
/// in world coordinates. `start`/`end` are the endpoints of the guide line;
/// `dist` is the non-negative world-space gap shown as the px label.
#[derive(Debug, Clone, Copy, PartialEq)]
struct GapSegment {
    axis: MeasureAxis,
    start: DVec2,
    end: DVec2,
    dist: f64,
}

/// Cross-axis coordinate at which to anchor a gap guide: the centre of the
/// boxes' overlap on that axis when they overlap, else the midpoint between the
/// two box centres (so the connector still reads for diagonally offset boxes,
/// like Figma's measure line). Mirrors fanta's `overlays::measure::cross_anchor`.
fn cross_anchor(a_lo: f64, a_hi: f64, b_lo: f64, b_hi: f64) -> f64 {
    let lo = a_lo.max(b_lo);
    let hi = a_hi.min(b_hi);
    if lo <= hi {
        (lo + hi) * 0.5
    } else {
        ((a_lo + a_hi) * 0.5 + (b_lo + b_hi) * 0.5) * 0.5
    }
}

/// Compute the gap guide segments between the `selected` box and the `hovered`
/// box, both world-space AABBs. Returns up to two segments — one per axis —
/// each present only when the boxes are *separated* on that axis (a clear gap
/// to measure). Boxes that overlap on an axis contribute no segment for it; two
/// fully-overlapping boxes therefore yield an empty list. Mirrors fanta's
/// `overlays::measure::gap_segments` (same distances, same shared-span anchor).
fn edge_gaps(selected: fanta_doc::Bounds, hovered: fanta_doc::Bounds) -> Vec<GapSegment> {
    let mut out = Vec::with_capacity(2);

    let y = cross_anchor(selected.min_y, selected.max_y, hovered.min_y, hovered.max_y);
    if selected.max_x <= hovered.min_x {
        out.push(GapSegment {
            axis: MeasureAxis::Horizontal,
            start: DVec2::new(selected.max_x, y),
            end: DVec2::new(hovered.min_x, y),
            dist: hovered.min_x - selected.max_x,
        });
    } else if hovered.max_x <= selected.min_x {
        out.push(GapSegment {
            axis: MeasureAxis::Horizontal,
            start: DVec2::new(hovered.max_x, y),
            end: DVec2::new(selected.min_x, y),
            dist: selected.min_x - hovered.max_x,
        });
    }

    let x = cross_anchor(selected.min_x, selected.max_x, hovered.min_x, hovered.max_x);
    if selected.max_y <= hovered.min_y {
        out.push(GapSegment {
            axis: MeasureAxis::Vertical,
            start: DVec2::new(x, selected.max_y),
            end: DVec2::new(x, hovered.min_y),
            dist: hovered.min_y - selected.max_y,
        });
    } else if hovered.max_y <= selected.min_y {
        out.push(GapSegment {
            axis: MeasureAxis::Vertical,
            start: DVec2::new(x, hovered.max_y),
            end: DVec2::new(x, selected.min_y),
            dist: selected.min_y - hovered.max_y,
        });
    }

    out
}

/// Shape a single line of overlay text in the UI font at a fixed screen size.
/// The whole overlay pass shapes only a handful of these (top-level frame names
/// plus one badge and up to two measurement labels), so no caching is needed.
fn shape_label(text: &str, color: Hsla, ui_font: &Font, window: &Window) -> ShapedLine {
    let text: gpui::SharedString = text.to_string().into();
    let run = TextRun {
        len: text.len(),
        font: ui_font.clone(),
        color,
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    window
        .text_system()
        .shape_line(text, px(LABEL_FONT_SIZE), &[run], None)
}

/// Paint a small rounded badge (accent or red) with the given shaped white text
/// centered inside it, its top-center anchored at `top_center`. Mirrors the
/// selection dimension pill / measurement label treatment in the original
/// fanta app.
fn paint_pill(
    top_center: Point<Pixels>,
    line: &ShapedLine,
    background: Hsla,
    window: &mut Window,
    cx: &mut App,
) {
    let pad_x = px(6.0);
    let pill_width = line.width() + pad_x * 2.0;
    let origin = point(top_center.x - pill_width / 2.0, top_center.y);
    let pill = Bounds {
        origin,
        size: size(pill_width, PILL_HEIGHT),
    };
    window.paint_quad(gpui::quad(
        pill,
        px(3.0),
        background,
        px(0.0),
        gpui::transparent_black(),
        BorderStyle::Solid,
    ));
    // Center the glyphs in the plate: pad_x from the left edge, and vertically
    // centered by painting into a line box the height of the pill.
    let text_origin = point(origin.x + pad_x, origin.y);
    if let Err(error) = line.paint(text_origin, PILL_HEIGHT, TextAlign::Left, None, window, cx) {
        log::warn!("failed to paint canvas overlay pill text: {error:#}");
    }
}

pub(crate) fn same_viewport(left: Viewport, right: Viewport) -> bool {
    (left.center[0] - right.center[0]).abs() < 0.001
        && (left.center[1] - right.center[1]).abs() < 0.001
        && (left.zoom - right.zoom).abs() < 0.0001
}

pub(crate) fn bounds_size(bounds: Bounds<Pixels>) -> (f64, f64) {
    (
        f64::from(f32::from(bounds.size.width)),
        f64::from(f32::from(bounds.size.height)),
    )
}

pub(crate) fn screen_position_in_bounds(position: Point<Pixels>, bounds: Bounds<Pixels>) -> DVec2 {
    DVec2::new(
        f64::from(f32::from(position.x - bounds.origin.x)),
        f64::from(f32::from(position.y - bounds.origin.y)),
    )
}

pub(crate) fn render_size_for_bounds(bounds: Bounds<Pixels>, scale_factor: f32) -> (u32, u32) {
    let width = (f32::from(bounds.size.width) * scale_factor)
        .round()
        .max(1.0) as u32;
    let height = (f32::from(bounds.size.height) * scale_factor)
        .round()
        .max(1.0) as u32;
    (width, height)
}

#[cfg(test)]
mod geometry_tests {
    use super::*;
    use fanta_doc::Bounds as WorldBounds;

    #[test]
    fn side_by_side_boxes_have_one_horizontal_gap() {
        // A: [0,0..10,10], B: [30,0..40,10] → a clear 20px horizontal gap, no
        // vertical gap (they share the full y-range).
        let a = WorldBounds::from_xywh(0.0, 0.0, 10.0, 10.0);
        let b = WorldBounds::from_xywh(30.0, 0.0, 10.0, 10.0);
        let segments = edge_gaps(a, b);
        assert_eq!(segments.len(), 1);
        let segment = segments[0];
        assert_eq!(segment.axis, MeasureAxis::Horizontal);
        assert_eq!(segment.dist, 20.0);
        // Corridor runs from A's right edge (10) to B's left edge (30), anchored
        // at the shared y-overlap centre (5).
        assert_eq!(segment.start.x, 10.0);
        assert_eq!(segment.end.x, 30.0);
        assert_eq!(segment.start.y, 5.0);
    }

    #[test]
    fn stacked_boxes_have_one_vertical_gap() {
        let a = WorldBounds::from_xywh(0.0, 0.0, 10.0, 10.0);
        let b = WorldBounds::from_xywh(0.0, 15.0, 10.0, 10.0);
        let segments = edge_gaps(a, b);
        assert_eq!(segments.len(), 1);
        let segment = segments[0];
        assert_eq!(segment.axis, MeasureAxis::Vertical);
        assert_eq!(segment.dist, 5.0);
        assert_eq!(segment.start.y, 10.0);
        assert_eq!(segment.end.y, 15.0);
        assert_eq!(segment.start.x, 5.0);
    }

    #[test]
    fn diagonally_offset_boxes_have_both_gaps() {
        let a = WorldBounds::from_xywh(0.0, 0.0, 10.0, 10.0);
        let b = WorldBounds::from_xywh(20.0, 30.0, 10.0, 10.0);
        let segments = edge_gaps(a, b);
        assert_eq!(segments.len(), 2);
        let horizontal = segments
            .iter()
            .find(|s| s.axis == MeasureAxis::Horizontal)
            .expect("a horizontal gap");
        let vertical = segments
            .iter()
            .find(|s| s.axis == MeasureAxis::Vertical)
            .expect("a vertical gap");
        assert_eq!(horizontal.dist, 10.0);
        assert_eq!(vertical.dist, 20.0);
    }

    #[test]
    fn overlapping_boxes_have_no_gap() {
        let a = WorldBounds::from_xywh(0.0, 0.0, 10.0, 10.0);
        assert!(edge_gaps(a, a).is_empty());
        // Partial overlap on both axes is also no-gap.
        let b = WorldBounds::from_xywh(5.0, 5.0, 10.0, 10.0);
        assert!(edge_gaps(a, b).is_empty());
    }

    #[test]
    fn gap_is_symmetric_in_argument_order() {
        let a = WorldBounds::from_xywh(0.0, 0.0, 10.0, 10.0);
        let b = WorldBounds::from_xywh(30.0, 0.0, 10.0, 10.0);
        let ab = edge_gaps(a, b);
        let ba = edge_gaps(b, a);
        assert_eq!(ab.len(), 1);
        assert_eq!(ba.len(), 1);
        assert_eq!(ab[0].dist, ba[0].dist);
        assert_eq!(
            ab[0].start.x.min(ab[0].end.x),
            ba[0].start.x.min(ba[0].end.x)
        );
    }

    #[test]
    fn touching_edges_yield_a_zero_distance_gap() {
        let a = WorldBounds::from_xywh(0.0, 0.0, 10.0, 10.0);
        let b = WorldBounds::from_xywh(10.0, 0.0, 10.0, 10.0);
        let segments = edge_gaps(a, b);
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].axis, MeasureAxis::Horizontal);
        assert_eq!(segments[0].dist, 0.0);
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    fn viewport(center: [f64; 2], zoom: f64) -> Viewport {
        Viewport { center, zoom }
    }

    #[test]
    fn reprojection_shifts_content_against_a_pan() {
        let bounds = Bounds {
            origin: point(px(0.), px(0.)),
            size: size(px(100.), px(100.)),
        };
        let cached = viewport([0.0, 0.0], 1.0);
        let current = viewport([10.0, 0.0], 1.0);
        let projected = reprojected_bounds(bounds, cached, current);
        // Panning the viewport 10 world units right moves the old frame's
        // content 10 px left at zoom 1.
        assert!((f32::from(projected.origin.x) - -10.0).abs() < 1e-4);
        assert!((f32::from(projected.origin.y) - 0.0).abs() < 1e-4);
        assert!((f32::from(projected.size.width) - 100.0).abs() < 1e-4);
    }

    #[test]
    fn reprojection_scales_content_around_the_shared_center() {
        let bounds = Bounds {
            origin: point(px(0.), px(0.)),
            size: size(px(100.), px(100.)),
        };
        let cached = viewport([0.0, 0.0], 1.0);
        let current = viewport([0.0, 0.0], 2.0);
        let projected = reprojected_bounds(bounds, cached, current);
        // Doubling the zoom doubles the old frame around the element center.
        assert!((f32::from(projected.size.width) - 200.0).abs() < 1e-4);
        assert!((f32::from(projected.origin.x) - -50.0).abs() < 1e-4);
    }
}
