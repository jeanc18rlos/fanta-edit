//! The canvas element: paints the rendered scene (through Skia Metal on
//! macOS, with a CPU fallback), then the interaction overlays — hover and
//! selection outlines, resize handles, and the active tool's render hints.

use std::rc::Rc;
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
use fanta_doc::{Action, AnimationClipId, MotionEvaluation, NodeId, Viewport};
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
use crate::editor_session::EditorMode;
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
    motion_frame: Option<MotionFrameKey>,
    page_root: Option<NodeId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MotionFrameKey {
    clip: AnimationClipId,
    playhead_ms: u32,
}

impl From<&MotionEvaluation> for MotionFrameKey {
    fn from(evaluation: &MotionEvaluation) -> Self {
        Self {
            clip: evaluation.clip,
            playhead_ms: evaluation.playhead_ms,
        }
    }
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
    motion_frame: Option<MotionFrameKey>,
}

#[cfg(target_os = "macos")]
struct CachedSurface {
    buffer: CVPixelBuffer,
    key: SurfaceKey,
}

/// What to do with the cached frame for a newly requested one.
#[cfg(target_os = "macos")]
#[derive(Debug, PartialEq, Eq)]
enum FrameDecision {
    /// The cached frame matches the request exactly; hand it back as fresh.
    ReuseCached,
    /// The cached frame covers the requested view at a tolerable zoom ratio
    /// and a fresh render is throttled; scale/offset it over the element.
    Reproject,
    /// Render the scene anew.
    RenderFresh,
}

/// Decide whether `cached` satisfies `requested` as-is, can be reprojected
/// over it, or a fresh render is due. Pure so the reproject-vs-fresh policy is
/// unit-testable without a Metal device.
///
/// Mid-interaction (`within_render_interval`), the previous frame is reused
/// reprojected instead of paying for a full scene render on every viewport
/// tick — but only when the cached frame's world coverage still contains
/// everything the element must show. A partially covering frame would flash
/// background at the leading edges until the next fresh render, which reads as
/// edge jumping while panning or zooming out.
#[cfg(target_os = "macos")]
fn frame_decision(
    cached: &SurfaceKey,
    requested: &SurfaceKey,
    frame_logical: (f64, f64),
    visible_logical: (f64, f64),
    within_render_interval: bool,
) -> FrameDecision {
    if *cached == *requested {
        return FrameDecision::ReuseCached;
    }
    if cached.size == requested.size
        && cached.page_root == requested.page_root
        && cached.revision == requested.revision
        && cached.motion_frame == requested.motion_frame
        && within_render_interval
    {
        let cached_zoom = cached.viewport_zoom.max(f64::EPSILON);
        let scale = requested.viewport_zoom / cached_zoom;
        let covered_width = frame_logical.0 / cached_zoom;
        let covered_height = frame_logical.1 / cached_zoom;
        let needed_width = visible_logical.0 / requested.viewport_zoom.max(f64::EPSILON);
        let needed_height = visible_logical.1 / requested.viewport_zoom.max(f64::EPSILON);
        let slack_x = (covered_width - needed_width) * 0.5;
        let slack_y = (covered_height - needed_height) * 0.5;
        let offset_x = (cached.viewport_center[0] - requested.viewport_center[0]).abs();
        let offset_y = (cached.viewport_center[1] - requested.viewport_center[1]).abs();
        let covers_view = offset_x <= slack_x && offset_y <= slack_y;
        // Zooming OUT quickly leaves the cached frame's coverage (its margin
        // is thin), which used to force a FULL scene render on every input
        // event — the exact "zooming degrades" cliff. A shrinking reprojected
        // frame with briefly exposed background at the edges is the same
        // blurry-then-sharp trade zoom-in already makes, so accept it while
        // the throttle is closed instead of stalling the gesture.
        let zooming_out = scale < 1.0;
        if (covers_view || zooming_out)
            && (1.0 / MacGpuRenderer::MAX_REPROJECT_SCALE..=MacGpuRenderer::MAX_REPROJECT_SCALE)
                .contains(&scale)
        {
            return FrameDecision::Reproject;
        }
    }
    FrameDecision::RenderFresh
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
        motion: Option<&MotionEvaluation>,
    ) -> Result<GpuFrame> {
        let key = SurfaceKey {
            size,
            viewport_center: viewport.center,
            viewport_zoom: viewport.zoom,
            page_root,
            revision: document.render_generation(),
            motion_frame: motion.map(MotionFrameKey::from),
        };
        if let Some(cached) = &self.cached {
            let within_render_interval = self
                .last_render_at
                .is_some_and(|at| at.elapsed() < Self::MIN_RENDER_INTERVAL);
            match frame_decision(
                &cached.key,
                &key,
                frame_logical,
                visible_logical,
                within_render_interval,
            ) {
                FrameDecision::ReuseCached => {
                    return Ok(GpuFrame::Fresh(cached.buffer.clone()));
                }
                FrameDecision::Reproject => {
                    return Ok(GpuFrame::Reprojected {
                        buffer: cached.buffer.clone(),
                        viewport: Viewport {
                            center: cached.key.viewport_center,
                            zoom: cached.key.viewport_zoom,
                        },
                    });
                }
                FrameDecision::RenderFresh => {}
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
            mode_generation: document.render_generation(),
            motion,
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
    motion: Option<&MotionEvaluation>,
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
        mode_generation: document.render_generation(),
        motion,
        playback: None,
        dark_ui: false,
    };
    renderer.render_page_with(&document.doc.scene, &render_viewport, page_root, &inputs);
    render_image_from_rgba(width, height, renderer.copy_rgba(), scale_factor)
}

pub(crate) fn render_image_from_rgba(
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
        motion: Option<&MotionEvaluation>,
    ) -> Result<Arc<RenderImage>> {
        let revision = document.render_generation();
        let motion_frame = motion.map(MotionFrameKey::from);
        if let Some(rendered) = &self.rendered_canvas
            && rendered.size == size
            && rendered.page_root == page_root
            && rendered.revision == revision
            && rendered.motion_frame == motion_frame
            && same_viewport(rendered.viewport, viewport)
        {
            return Ok(rendered.image.clone());
        }

        let image = render_fig_canvas(
            document,
            page_root,
            size.0,
            size.1,
            viewport,
            scale_factor,
            motion,
        )?;
        self.rendered_canvas = Some(RenderedCanvas {
            image: image.clone(),
            size,
            viewport,
            revision,
            motion_frame,
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
        if self.view.read(cx).is_presenting_prototype() {
            self.view.update(cx, |this, _| {
                this.set_container_bounds(bounds);
            });
            return Some(DragListeners {
                panning: false,
                primary_drag: false,
            });
        }
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
            // A component-scoped view renders a master root that is not a
            // listed page; the initial fit must frame the master's own bounds,
            // not the fallback page the paint path never shows.
            let fit = document
                .doc
                .active_page()
                .filter(|root| document.doc.is_component_root(*root))
                .map(|root| crate::document::page_bounds(&document.doc, Some(root)))
                .unwrap_or(page.bounds);
            let viewport = view.viewport().unwrap_or_else(|| {
                crate::document::fit_bounds(
                    fit,
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

        if self.view.read(cx).is_presenting_prototype() {
            let (logical_width, logical_height) = bounds_size(bounds);
            let size = (
                logical_width.round().max(1.0) as u32,
                logical_height.round().max(1.0) as u32,
            );
            let scale_factor = window.scale_factor();
            let image = self.view.update(cx, |this, _| {
                this.render_prototype_image(size, scale_factor)
            });
            match image {
                Ok(image) => {
                    if let Err(error) = window
                        .with_content_mask(Some(gpui::ContentMask { bounds }), |window| {
                            window.paint_image(bounds, Default::default(), image, 0, false)
                        })
                    {
                        log::warn!("failed to paint prototype presentation: {error:#}");
                    }
                }
                Err(error) => log::warn!("failed to render prototype presentation: {error:#}"),
            }
            return;
        }

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
            // Render the SAME page the tools parent into, hit-testing scopes to,
            // and the selection overlay walks — `doc.active_page()`. Deriving the
            // render root from `selected_page_index` instead let the two diverge
            // (e.g. after a reload, or when the importer's active design page is
            // not the first visible page), so a freshly-created shape was parented
            // under `active_page` while the canvas rendered a different page —
            // making new shapes and text render invisible. Fall back to the
            // selected page's root only when no active page is set.
            let page_root = document
                .doc
                .active_page()
                .map(Some)
                .or_else(|| document.page(this.selected_page_index()).map(|page| page.root))
                .ok_or_else(|| anyhow!("Figma document has no renderable pages"))?;
            let motion = this.motion_evaluation(document, cx);

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
                    motion.as_ref(),
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

            this.render_cpu_canvas(
                document,
                page_root,
                render_size,
                viewport,
                scale_factor,
                motion.as_ref(),
            )
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

pub(crate) fn evaluated_world_transform(
    scene: &fanta_doc::Scene,
    id: NodeId,
    motion: Option<&MotionEvaluation>,
) -> Option<fanta_doc::Transform2D> {
    // Motion-free (the Design-mode norm): identical composition through the
    // scene's memoized cache — O(1) warm instead of an ancestor `Vec`
    // allocation and walk per call. The chrome scan calls this per selected
    // node per frame, so with a select-all on a large page the uncached walk
    // was O(selection × depth) every paint.
    let Some(motion) = motion else {
        return scene.world_transform(id);
    };
    let node = scene.get(id)?;
    let mut ancestors: Vec<_> = scene.ancestors_of(id).collect();
    ancestors.reverse();

    let mut world = fanta_doc::Transform2D::IDENTITY;
    for ancestor in ancestors.into_iter().chain(std::iter::once(node)) {
        let local = motion.apply_to_node(ancestor).transform;
        world = local.then(&world);
    }
    Some(world)
}

pub(crate) fn evaluated_world_bounds(
    scene: &fanta_doc::Scene,
    id: NodeId,
    motion: Option<&MotionEvaluation>,
) -> Option<fanta_doc::Bounds> {
    let node = scene.get(id)?;
    if let Some(motion) = motion {
        let evaluated = motion.apply_to_node(node);
        if let Some(local) = evaluated.data.local_bounds() {
            return local.try_transformed(&evaluated_world_transform(scene, id, Some(motion))?);
        }
    } else {
        // Motion-free fast paths: the same math as the motion branch, but
        // through the scene's memoized caches. Without these, every call
        // re-ran a vector's `rough_bounds` (a full path-segment walk) and an
        // ancestor-chain transform fold — per selected node, per frame.
        match &node.data {
            // A group's data-level box (clip/local size) can diverge from the
            // scene's local bounds when the `clip_content` meta is off, so
            // keep reading the data box (an O(1) field read); only the
            // transform goes through the cache.
            fanta_doc::NodeData::Group(_) => {
                if let Some(local) = node.data.local_bounds() {
                    return local.try_transformed(&scene.world_transform(id)?);
                }
                // Sizeless group: union children below, like the motion path.
            }
            // A boolean's scene-level local bounds union its operands in
            // LOCAL space, which differs from the world-space union below
            // under rotation. Keep the world-space union for parity.
            fanta_doc::NodeData::Boolean(_) => {}
            // For every other kind the scene's local bounds ARE
            // `data.local_bounds()` — memoized, so a vector's path walk runs
            // once per edit instead of once per call.
            _ => {
                if let Some(local) = scene.local_bounds(id) {
                    return local.try_transformed(&scene.world_transform(id)?);
                }
                // No intrinsic bounds: union children below.
            }
        }
    }

    let mut bounds: Option<fanta_doc::Bounds> = None;
    for &child in scene.children_of(Some(id)) {
        if let Some(child_bounds) = evaluated_world_bounds(scene, child, motion) {
            bounds = Some(match bounds {
                Some(bounds) => bounds.union(&child_bounds),
                None => child_bounds,
            });
        }
    }
    bounds
}

pub(crate) fn evaluated_hit_test_screen(
    scene: &fanta_doc::Scene,
    motion: &MotionEvaluation,
    viewport: &Viewport,
    screen_size: DVec2,
    screen_point: DVec2,
    precision: fanta_canvas::HitPrecision,
    active_page: Option<NodeId>,
) -> Option<NodeId> {
    let world_point = fanta_canvas::screen_to_world(screen_point, viewport, screen_size);
    evaluated_hit_test(scene, motion, world_point, precision, active_page)
}

fn evaluated_hit_test(
    scene: &fanta_doc::Scene,
    motion: &MotionEvaluation,
    world_point: DVec2,
    precision: fanta_canvas::HitPrecision,
    active_page: Option<NodeId>,
) -> Option<NodeId> {
    if let Some(page) = active_page {
        let exclude_root = scene.get(page).is_some_and(|node| node.parent.is_none());
        return evaluated_hit_test_subtree(
            scene,
            motion,
            page,
            world_point,
            precision,
            exclude_root,
        );
    }

    scene.roots().iter().rev().find_map(|root| {
        evaluated_hit_test_subtree(scene, motion, *root, world_point, precision, false)
    })
}

fn evaluated_hit_test_subtree(
    scene: &fanta_doc::Scene,
    motion: &MotionEvaluation,
    id: NodeId,
    world_point: DVec2,
    precision: fanta_canvas::HitPrecision,
    exclude_self: bool,
) -> Option<NodeId> {
    let committed = scene.get(id)?;
    let node = motion.apply_to_node(committed);
    if node
        .flags
        .intersects(fanta_doc::NodeFlags::HIDDEN | fanta_doc::NodeFlags::LOCKED)
    {
        return None;
    }
    let bounds = evaluated_world_bounds(scene, id, Some(motion))?;
    if !bounds.contains_point(world_point) {
        return None;
    }

    if matches!(node.data, fanta_doc::NodeData::Boolean(_)) {
        return (!exclude_self).then_some(id);
    }
    for &child in scene.children_of(Some(id)).iter().rev() {
        if let Some(hit) =
            evaluated_hit_test_subtree(scene, motion, child, world_point, precision, false)
        {
            return Some(hit);
        }
    }
    if exclude_self {
        return None;
    }
    if let fanta_doc::NodeData::Group(group) = &node.data {
        return group.is_frame_surface().then_some(id);
    }
    if precision == fanta_canvas::HitPrecision::Path
        && let fanta_doc::NodeData::Vector(vector) = &node.data
    {
        let transform = evaluated_world_transform(scene, id, Some(motion))?;
        let [a, b, c, d, _, _] = transform.to_components();
        let determinant = a * d - b * c;
        if !determinant.is_finite() || determinant.abs() <= f64::EPSILON {
            return None;
        }
        let local_point = transform.inverse().transform_point(world_point);
        return fanta_canvas::point_in_path(&vector.path, local_point).then_some(id);
    }
    Some(id)
}

/// A top-level frame's name label to paint above its top-left corner.
pub(crate) struct FrameLabel {
    /// The frame's world bounds; its projected top-left anchors the label.
    world: fanta_doc::Bounds,
    name: String,
    /// The selected frame's label is drawn in the accent color; others muted.
    selected: bool,
}

/// Selection-dependent overlay geometry memoized across paint frames. The
/// scan behind it is O(selection) (with a select-all, O(page)), yet its
/// inputs only change on document edits or selection changes — a pan or zoom
/// repaints every frame with both untouched. Owned by [`FigView`] behind a
/// `RefCell` because overlay collection runs during element paint with only
/// `&App`; cleared alongside the rendered-canvas cache on reloads, whose
/// restarted revision counter could otherwise collide with a stale entry.
///
/// [`FigView`]: crate::view::FigView
pub(crate) struct ChromeCache {
    scene_revision: u64,
    page_root: Option<NodeId>,
    selection: Vec<NodeId>,
    editing_node: Option<NodeId>,
    frame_labels: Rc<Vec<FrameLabel>>,
    selected_bounds: Rc<Vec<fanta_doc::Bounds>>,
    selection_union: Option<fanta_doc::Bounds>,
    text_baselines: Rc<Vec<(DVec2, DVec2)>>,
}

/// Owned overlay data gathered while the document is borrowed, so the paint
/// pass (which needs `&mut App` for text) no longer holds that borrow. The
/// selection-scaled pieces are shared `Rc`s with the [`ChromeCache`] so a
/// cache hit clones a pointer, not a 29k-element `Vec`.
struct OverlayData {
    frame_labels: Rc<Vec<FrameLabel>>,
    hovered_bounds: Option<fanta_doc::Bounds>,
    selected_bounds: Rc<Vec<fanta_doc::Bounds>>,
    /// A single selection follows its transformed local box instead of drawing
    /// an axis-aligned world AABB. This keeps the box and handles attached to a
    /// rotated node, including beneath transformed parents.
    oriented_selection: Option<OrientedSelection>,
    text_baselines: Rc<Vec<(DVec2, DVec2)>>,
    /// Union of every selected node's world bounds, for the size badge.
    selection_union: Option<fanta_doc::Bounds>,
    /// The selection's world size shown in the badge, once nodes are selected.
    selection_size: Option<(f64, f64)>,
    /// Measurement gap segments between the single selection and the hovered
    /// node, when the alt-hover-style measure condition holds.
    measure_segments: Vec<GapSegment>,
    /// Comment pins on the active page, in stored (oldest-first) order.
    comment_pins: Vec<CommentPin>,
    prototype_connections: Vec<(DVec2, DVec2)>,
    prototype_handle: Option<DVec2>,
    prototype_start: Option<DVec2>,
}

struct OrientedSelection {
    corners: [DVec2; 4],
    handles: [DVec2; 8],
}

fn oriented_selection(
    local: fanta_doc::Bounds,
    transform: fanta_doc::Transform2D,
) -> OrientedSelection {
    let corners = [
        DVec2::new(local.min_x, local.min_y),
        DVec2::new(local.max_x, local.min_y),
        DVec2::new(local.max_x, local.max_y),
        DVec2::new(local.min_x, local.max_y),
    ]
    .map(|point| transform.transform_point(point));
    let handles =
        ResizeHandle::ALL.map(|handle| transform.transform_point(handle.handle_world(local)));
    OrientedSelection { corners, handles }
}

fn authored_selection_bounds(doc: &fanta_doc::Doc, id: NodeId) -> Option<fanta_doc::Bounds> {
    let node = doc.scene.get(id)?;
    match &node.data {
        fanta_doc::NodeData::Group(group) => group
            .clip_size
            .or(group.local_size)
            .map(|[width, height]| fanta_doc::Bounds::from_xywh(0.0, 0.0, width, height))
            .or_else(|| doc.scene.local_bounds(id)),
        _ => doc.scene.local_bounds(id),
    }
}

/// Prepaint snapshot of one comment pin (owned, so paint holds no doc borrow).
pub(crate) struct CommentPin {
    pub(crate) world: DVec2,
    pub(crate) author: String,
    pub(crate) resolved: bool,
    pub(crate) from_agent: bool,
    pub(crate) message_count: usize,
}

impl CanvasElement {
    /// Gather the frame-label, selection-badge, and measurement geometry from
    /// the document into owned values. Done up front so the borrow of `cx` (via
    /// the view/document) is released before the paint pass, which needs a
    /// mutable `cx` to shape and paint text.
    fn collect_overlay_data(&self, cx: &App) -> OverlayData {
        let collect_started = std::time::Instant::now();
        let mut data = OverlayData {
            frame_labels: Rc::new(Vec::new()),
            hovered_bounds: None,
            selected_bounds: Rc::new(Vec::new()),
            oriented_selection: None,
            text_baselines: Rc::new(Vec::new()),
            selection_union: None,
            selection_size: None,
            measure_segments: Vec::new(),
            comment_pins: Vec::new(),
            prototype_connections: Vec::new(),
            prototype_handle: None,
            prototype_start: None,
        };
        let view = self.view.read(cx);
        let item = view.item().read(cx);
        let Some(document) = item.document() else {
            return data;
        };
        let doc = &document.doc;
        let motion = view.motion_evaluation(document, cx);
        let editing_node = view.text_edit.as_ref().map(|edit| edit.session.node_id());
        let page_root = doc.active_page();

        // The selection-scaled scans (frame labels, per-node selection bounds,
        // text baselines) only depend on scene content, the active page, the
        // selection, and the text-editing node. Reuse the memoized geometry
        // when none of those changed — a pan/zoom repaints every frame with
        // all of them untouched, so this turns an O(selection) walk into an
        // `Rc` clone. Motion mode samples the timeline per frame and bypasses
        // the cache entirely.
        let cached = motion.is_none().then(|| {
            let cache = view.chrome_cache.borrow();
            cache
                .as_ref()
                .filter(|cache| {
                    cache.scene_revision == doc.scene.revision()
                        && cache.page_root == page_root
                        && cache.editing_node == editing_node
                        && cache.selection.as_slice() == doc.selection.as_slice()
                })
                .map(|cache| {
                    (
                        cache.frame_labels.clone(),
                        cache.selected_bounds.clone(),
                        cache.selection_union,
                        cache.text_baselines.clone(),
                    )
                })
        });
        if let Some(Some((frame_labels, selected_bounds, selection_union, text_baselines))) = cached
        {
            data.frame_labels = frame_labels;
            data.selected_bounds = selected_bounds;
            data.selection_union = selection_union;
            data.text_baselines = text_baselines;
        } else {
            // Frame name labels: only frame-surface groups that are direct
            // children of the active page (top-level frames/sections, like
            // Figma) — nested frames would be noise.
            let mut frame_labels = Vec::new();
            let page_children: &[NodeId] = doc.scene.children_of(page_root);
            for &id in page_children {
                let Some(node) = doc.scene.get(id) else {
                    continue;
                };
                let is_frame = matches!(&node.data, fanta_doc::NodeData::Group(group) if group.is_frame_surface());
                if !is_frame {
                    continue;
                }
                let Some(world) = evaluated_world_bounds(&doc.scene, id, motion.as_ref()) else {
                    continue;
                };
                let name = node.name.trim();
                let name = if name.is_empty() { "Frame" } else { name };
                frame_labels.push(FrameLabel {
                    world,
                    name: name.to_string(),
                    selected: doc.selection.contains(id),
                });
            }

            // Selection union + size badge.
            let mut selected_bounds = Vec::new();
            let mut text_baselines = Vec::new();
            for &id in doc.selection.iter() {
                if let Some(world) = evaluated_world_bounds(&doc.scene, id, motion.as_ref()) {
                    selected_bounds.push(world);
                    data.selection_union = Some(match data.selection_union {
                        Some(existing) => existing.union(&world),
                        None => world,
                    });
                }
                if editing_node != Some(id)
                    && let Some(node) = doc.scene.get(id)
                    && let fanta_doc::NodeData::Text(text) = &node.data
                    && let Some(transform) =
                        evaluated_world_transform(&doc.scene, id, motion.as_ref())
                {
                    let baseline = fanta_render::text_first_baseline(text);
                    text_baselines.push((
                        transform.transform_point(DVec2::new(0.0, baseline)),
                        transform
                            .transform_point(DVec2::new(text.local_size[0].max(1.0), baseline)),
                    ));
                }
            }
            data.frame_labels = Rc::new(frame_labels);
            data.selected_bounds = Rc::new(selected_bounds);
            data.text_baselines = Rc::new(text_baselines);
            if motion.is_none() {
                *view.chrome_cache.borrow_mut() = Some(ChromeCache {
                    scene_revision: doc.scene.revision(),
                    page_root,
                    selection: doc.selection.as_slice().to_vec(),
                    editing_node,
                    frame_labels: data.frame_labels.clone(),
                    selected_bounds: data.selected_bounds.clone(),
                    selection_union: data.selection_union,
                    text_baselines: data.text_baselines.clone(),
                });
            }
        }
        if let &[id] = doc.selection.as_slice()
            && let (Some(local), Some(transform)) = (
                authored_selection_bounds(doc, id),
                evaluated_world_transform(&doc.scene, id, motion.as_ref()),
            )
        {
            let oriented = oriented_selection(local, transform);
            data.selection_size = Some((
                (oriented.corners[1] - oriented.corners[0]).length(),
                (oriented.corners[3] - oriented.corners[0]).length(),
            ));
            data.oriented_selection = Some(oriented);
        } else if let Some(union) = data.selection_union {
            data.selection_size = Some((union.width(), union.height()));
        }

        data.hovered_bounds = view
            .hovered_node()
            .filter(|hovered| !doc.selection.contains(*hovered))
            .and_then(|hovered| evaluated_world_bounds(&doc.scene, hovered, motion.as_ref()));

        // Comment pins for the active page (annotation overlay, not scene
        // content, so they paint above the rendered canvas like the badges).
        if let Some(page) = doc.active_page() {
            data.comment_pins = crate::comments::read_comments(doc, page)
                .into_iter()
                .map(|comment| CommentPin {
                    world: DVec2::new(comment.world[0], comment.world[1]),
                    author: comment.author.clone(),
                    resolved: comment.resolved,
                    from_agent: comment.from_agent,
                    message_count: comment.message_count(),
                })
                .collect();
        }

        if view.editor_mode(cx) == EditorMode::Prototype {
            if let Some(start) = doc.flow_start()
                && doc.scene.get(start).is_some()
                && let Some(bounds) = evaluated_world_bounds(&doc.scene, start, motion.as_ref())
            {
                data.prototype_start = Some(DVec2::new(bounds.min_x, bounds.min_y));
            }
            if let &[source] = doc.selection.as_slice()
                && let Some(source_bounds) =
                    evaluated_world_bounds(&doc.scene, source, motion.as_ref())
            {
                let source_point = DVec2::new(
                    source_bounds.max_x,
                    (source_bounds.min_y + source_bounds.max_y) * 0.5,
                );
                data.prototype_handle = Some(source_point);
                if let Some(node) = doc.scene.get(source) {
                    for reaction in &node.reactions {
                        let target = match &reaction.action {
                            Action::Navigate { to } => Some(*to),
                            Action::OpenOverlay { frame, .. } => Some(*frame),
                            Action::ScrollTo { target } => Some(*target),
                            Action::Back
                            | Action::Close
                            | Action::SetVariable { .. }
                            | Action::UpdateVariant { .. }
                            | Action::OpenLink { .. } => None,
                        };
                        let Some(target_bounds) = target.and_then(|target| {
                            evaluated_world_bounds(&doc.scene, target, motion.as_ref())
                        }) else {
                            continue;
                        };
                        let target_point = DVec2::new(
                            target_bounds.min_x,
                            (target_bounds.min_y + target_bounds.max_y) * 0.5,
                        );
                        data.prototype_connections
                            .push((source_point, target_point));
                    }
                }
            }
        }

        // Measurements: exactly one node selected and a different, non-related
        // node hovered. The ancestor/descendant guard keeps the guides from
        // firing between a node and its own container (pure noise).
        if let (&[selected_id], Some(hovered_id)) = (doc.selection.as_slice(), view.hovered_node())
            && hovered_id != selected_id
            && !is_related(&doc.scene, selected_id, hovered_id)
            && let Some(selected_world) =
                evaluated_world_bounds(&doc.scene, selected_id, motion.as_ref())
            && let Some(hovered_world) =
                evaluated_world_bounds(&doc.scene, hovered_id, motion.as_ref())
        {
            data.measure_segments = edge_gaps(selected_world, hovered_world);
        }

        crate::report_slow("canvas chrome scan", collect_started);
        data
    }

    fn paint_overlays(&self, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut App) {
        if self.view.read(cx).is_presenting_prototype() {
            return;
        }
        let overlay_data = self.collect_overlay_data(cx);

        let view = self.view.read(cx);
        let Some(viewport) = view.viewport() else {
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
            if let Some(world) = overlay_data.hovered_bounds {
                window.paint_quad(gpui::outline(
                    project_bounds(world),
                    accent.opacity(0.55),
                    BorderStyle::Solid,
                ));
            }

            if let Some(oriented) = &overlay_data.oriented_selection {
                for edge in 0..oriented.corners.len() {
                    paint_line(
                        project(oriented.corners[edge]),
                        project(oriented.corners[(edge + 1) % oriented.corners.len()]),
                        accent,
                        window,
                    );
                }
            } else {
                for world in overlay_data.selected_bounds.iter() {
                    window.paint_quad(gpui::outline(
                        project_bounds(*world),
                        accent,
                        BorderStyle::Solid,
                    ));
                }
            }
            for (start, end) in overlay_data.text_baselines.iter() {
                paint_line(project(*start), project(*end), accent, window);
            }

            for (start, end) in &overlay_data.prototype_connections {
                paint_line(project(*start), project(*end), accent, window);
                let endpoint = project(*end);
                let radius = px(4.);
                window.paint_quad(gpui::quad(
                    Bounds {
                        origin: point(endpoint.x - radius, endpoint.y - radius),
                        size: size(radius * 2., radius * 2.),
                    },
                    radius,
                    accent,
                    px(0.),
                    gpui::transparent_black(),
                    BorderStyle::Solid,
                ));
            }
            if let Some(world) = overlay_data.prototype_handle {
                let center = project(world);
                let radius = px(5.);
                window.paint_quad(gpui::quad(
                    Bounds {
                        origin: point(center.x - radius, center.y - radius),
                        size: size(radius * 2., radius * 2.),
                    },
                    radius,
                    gpui::white(),
                    px(2.),
                    accent,
                    BorderStyle::Solid,
                ));
            }
            if let Some(world) = overlay_data.prototype_start {
                let anchor = project(world);
                let diameter = px(16.);
                let marker = Bounds {
                    origin: point(anchor.x - diameter - px(6.), anchor.y - diameter - px(6.)),
                    size: size(diameter, diameter),
                };
                window.paint_quad(gpui::quad(
                    marker,
                    diameter / 2.,
                    accent,
                    px(0.),
                    gpui::transparent_black(),
                    BorderStyle::Solid,
                ));
                let dot = px(4.);
                window.paint_quad(gpui::quad(
                    Bounds {
                        origin: point(
                            marker.origin.x + diameter / 2. - dot / 2.,
                            marker.origin.y + diameter / 2. - dot / 2.,
                        ),
                        size: size(dot, dot),
                    },
                    dot / 2.,
                    gpui::white(),
                    px(0.),
                    gpui::transparent_black(),
                    BorderStyle::Solid,
                ));
            }

            // Resize handles on the selection box, Figma-style, only for the
            // select tool.
            if view.tools().kind() == crate::tools::ToolKind::Select {
                let handle_px = px(HANDLE_SIZE);
                let handles = overlay_data
                    .oriented_selection
                    .as_ref()
                    .map(|oriented| oriented.handles.to_vec())
                    .or_else(|| {
                        overlay_data.selection_union.map(|union| {
                            ResizeHandle::ALL
                                .map(|handle| handle.handle_world(union))
                                .to_vec()
                        })
                    })
                    .unwrap_or_default();
                for world in handles {
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
            for label in data.frame_labels.iter() {
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

            // Comment pins: the original fanta's teardrop — squared tail at
            // the anchor's bottom-left, author-colored body, white monogram,
            // and a message-count chip. Screen-fixed size across zoom.
            for pin in &data.comment_pins {
                let anchor = project(pin.world);
                let fill = if pin.from_agent {
                    accent
                } else {
                    avatar_color(&pin.author)
                };
                let fill = if pin.resolved {
                    fill.opacity(0.5)
                } else {
                    fill
                };
                paint_comment_pin(
                    anchor,
                    fill,
                    &pin.author,
                    pin.message_count,
                    &ui_font,
                    window,
                    cx,
                );
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
/// Screen-fixed comment pin size, constant across zoom (matches the original).
const COMMENT_PIN_SIZE: f32 = 30.0;

/// The teardrop outline the original fanta uses: a 24-unit circle with three
/// rounded corners and a SQUARED bottom-left tail (the anchor point). Cubics
/// sampled at 10 steps each and normalized into the unit square, cached.
fn pin_unit_polygon() -> &'static [(f32, f32)] {
    use std::sync::OnceLock;
    static POLY: OnceLock<Vec<(f32, f32)>> = OnceLock::new();
    POLY.get_or_init(|| {
        fn cubic(
            points: &mut Vec<(f32, f32)>,
            p0: (f32, f32),
            p1: (f32, f32),
            p2: (f32, f32),
            p3: (f32, f32),
        ) {
            for step in 1..=10 {
                let t = step as f32 / 10.0;
                let u = 1.0 - t;
                let x = u * u * u * p0.0
                    + 3.0 * u * u * t * p1.0
                    + 3.0 * u * t * t * p2.0
                    + t * t * t * p3.0;
                let y = u * u * u * p0.1
                    + 3.0 * u * u * t * p1.1
                    + 3.0 * u * t * t * p2.1
                    + t * t * t * p3.1;
                points.push((x, y));
            }
        }
        let mut points: Vec<(f32, f32)> = Vec::with_capacity(44);
        points.push((24.0, 12.098));
        cubic(
            &mut points,
            (24.0, 12.098),
            (24.0, 18.725),
            (18.627, 24.098),
            (12.0, 24.098),
        );
        points.push((1.146, 24.098));
        cubic(
            &mut points,
            (1.146, 24.098),
            (0.513, 24.098),
            (0.0, 23.585),
            (0.0, 22.952),
        );
        points.push((0.0, 12.098));
        cubic(
            &mut points,
            (0.0, 12.098),
            (0.0, 5.471),
            (5.373, 0.098),
            (12.0, 0.098),
        );
        cubic(
            &mut points,
            (12.0, 0.098),
            (18.627, 0.098),
            (24.0, 5.471),
            (24.0, 12.098),
        );
        points
            .into_iter()
            .map(|(x, y)| (x / 24.0, (y - 0.098) / 24.0))
            .collect()
    })
}

/// Deterministic per-author pin/avatar color: FNV-1a over the normalized name
/// hashed onto the hue wheel at the original's fixed saturation/value.
fn avatar_color(author: &str) -> Hsla {
    let normalized = author.trim().to_lowercase();
    let mut hash: u32 = 0x811c_9dc5;
    for byte in normalized.bytes() {
        hash ^= u32::from(byte);
        hash = hash.wrapping_mul(0x0100_0193);
    }
    let hue = (hash % 360) as f32 / 360.0;
    // HSV(h, 0.52, 0.72) converted to HSL.
    let value = 0.72;
    let saturation_v = 0.52;
    let lightness = value * (1.0 - saturation_v / 2.0);
    let saturation = if lightness <= 0.0 || lightness >= 1.0 {
        0.0
    } else {
        (value - lightness) / f32::min(lightness, 1.0 - lightness)
    };
    gpui::hsla(hue, saturation, lightness, 1.0)
}

/// One teardrop comment pin: drop shadow, author-colored body, hairline white
/// stroke, monogram, and (for threads) a count chip at the top-right.
fn paint_comment_pin(
    anchor: Point<Pixels>,
    fill: Hsla,
    author: &str,
    message_count: usize,
    ui_font: &Font,
    window: &mut Window,
    cx: &mut App,
) {
    let size = COMMENT_PIN_SIZE;
    // The anchor is the squared tail: the pin rect's bottom-left corner.
    let origin = point(anchor.x, anchor.y - px(size));
    let polygon = pin_unit_polygon();
    let build = |offset_y: f32| -> Option<gpui::Path<Pixels>> {
        let mut points = polygon
            .iter()
            .map(|(x, y)| point(origin.x + px(x * size), origin.y + px(y * size + offset_y)));
        let first = points.next()?;
        let mut builder = PathBuilder::fill();
        builder.move_to(first);
        for p in points {
            builder.line_to(p);
        }
        builder.close();
        builder.build().ok()
    };
    if let Some(shadow) = build(1.5) {
        window.paint_path(shadow, gpui::black().opacity(0.18));
    }
    if let Some(body) = build(0.0) {
        window.paint_path(body, fill);
    }
    // Hairline outline: re-trace the polygon as a stroke path.
    {
        let mut points = polygon
            .iter()
            .map(|(x, y)| point(origin.x + px(x * size), origin.y + px(y * size)));
        if let Some(first) = points.next() {
            let mut builder = PathBuilder::stroke(px(1.));
            builder.move_to(first);
            for p in points {
                builder.line_to(p);
            }
            builder.close();
            if let Ok(path) = builder.build() {
                window.paint_path(path, gpui::white().opacity(0.86));
            }
        }
    }
    // Monogram: first grapheme of the author, uppercased; nudged up because
    // the tail eats the bottom-left.
    let monogram: String = author
        .trim()
        .chars()
        .next()
        .map(|c| c.to_uppercase().to_string())
        .unwrap_or_else(|| "?".to_string());
    let line = shape_label(&monogram, gpui::white(), ui_font, window);
    let text_x = origin.x + px(size / 2.0) - line.width / 2.0;
    let text_y = origin.y + px(size * 0.43) - px(LABEL_FONT_SIZE / 2.0);
    if let Err(error) = line.paint(
        point(text_x, text_y),
        px(LABEL_FONT_SIZE * 1.2),
        TextAlign::Left,
        None,
        window,
        cx,
    ) {
        log::warn!("failed to paint comment pin monogram: {error:#}");
    }
    // Thread size chip, only when there are replies.
    if message_count > 1 {
        let chip_center = point(origin.x + px(size * 0.92), origin.y + px(size * 0.08));
        let chip_radius = px(size * 0.27);
        let underlay = Bounds {
            origin: point(
                chip_center.x - chip_radius - px(1.6),
                chip_center.y - chip_radius - px(1.6),
            ),
            size: gpui::size((chip_radius + px(1.6)) * 2.0, (chip_radius + px(1.6)) * 2.0),
        };
        window.paint_quad(
            gpui::fill(underlay, gpui::white())
                .corner_radii(gpui::Corners::all(chip_radius + px(1.6))),
        );
        let chip = Bounds {
            origin: point(chip_center.x - chip_radius, chip_center.y - chip_radius),
            size: gpui::size(chip_radius * 2.0, chip_radius * 2.0),
        };
        window.paint_quad(
            gpui::fill(chip, gpui::rgb(0x33343A)).corner_radii(gpui::Corners::all(chip_radius)),
        );
        let count = shape_label(&format!("{message_count}"), gpui::white(), ui_font, window);
        if let Err(error) = count.paint(
            point(
                chip_center.x - count.width / 2.0,
                chip_center.y - px(LABEL_FONT_SIZE / 2.0),
            ),
            px(LABEL_FONT_SIZE),
            TextAlign::Left,
            None,
            window,
            cx,
        ) {
            log::warn!("failed to paint comment pin count: {error:#}");
        }
    }
}

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
    use fanta_doc::{
        AnimationClipId, Bounds as WorldBounds, CanvasNode, Color, GroupNode, MotionProperty,
        MotionTarget, NodeData, ResolvedVarValue, Scene, Transform2D, VectorNode,
    };
    use std::collections::BTreeMap;

    fn position_evaluation(node: NodeId, x: f64) -> MotionEvaluation {
        MotionEvaluation {
            clip: AnimationClipId::from_u128(1),
            playhead_ms: 500,
            overrides: BTreeMap::from([(
                MotionTarget::new(node, MotionProperty::PositionX),
                ResolvedVarValue::Float { value: x },
            )]),
        }
    }

    #[test]
    fn evaluated_bounds_compose_motion_through_animated_ancestors()
    -> Result<(), fanta_doc::SceneError> {
        let mut scene = Scene::new();
        let mut parent = CanvasNode::new(NodeData::Group(GroupNode::default()));
        parent.transform = Transform2D::translation(10.0, 20.0);
        let parent_id = parent.id;
        scene.insert(parent)?;

        let mut child = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
            0.0,
            0.0,
            10.0,
            10.0,
            Color::WHITE,
        )));
        child.parent = Some(parent_id);
        child.transform = Transform2D::translation(5.0, 6.0);
        let child_id = child.id;
        scene.insert(child)?;

        let motion = position_evaluation(parent_id, 100.0);
        assert_eq!(
            evaluated_world_bounds(&scene, child_id, Some(&motion)),
            Some(WorldBounds::from_xywh(105.0, 26.0, 10.0, 10.0))
        );
        assert_eq!(
            evaluated_world_bounds(&scene, parent_id, Some(&motion)),
            Some(WorldBounds::from_xywh(105.0, 26.0, 10.0, 10.0))
        );
        Ok(())
    }

    /// The pre-cache implementation of [`evaluated_world_transform`] with
    /// `motion: None`: an explicit root→leaf fold over the ancestor chain.
    /// The fast path must stay bit-identical to it.
    fn reference_world_transform(scene: &Scene, id: NodeId) -> Option<Transform2D> {
        let node = scene.get(id)?;
        let mut ancestors: Vec<_> = scene.ancestors_of(id).collect();
        ancestors.reverse();
        let mut world = Transform2D::IDENTITY;
        for ancestor in ancestors.into_iter().chain(std::iter::once(node)) {
            world = ancestor.transform.then(&world);
        }
        Some(world)
    }

    /// The pre-cache implementation of [`evaluated_world_bounds`] with
    /// `motion: None`: data-level local bounds through the uncached transform
    /// fold, world-space child union otherwise.
    fn reference_world_bounds(scene: &Scene, id: NodeId) -> Option<WorldBounds> {
        let node = scene.get(id)?;
        if let Some(local) = node.data.local_bounds() {
            return local.try_transformed(&reference_world_transform(scene, id)?);
        }
        let mut bounds: Option<WorldBounds> = None;
        for &child in scene.children_of(Some(id)) {
            if let Some(child_bounds) = reference_world_bounds(scene, child) {
                bounds = Some(match bounds {
                    Some(bounds) => bounds.union(&child_bounds),
                    None => child_bounds,
                });
            }
        }
        bounds
    }

    /// A scene exercising every fast-path branch: a clipped frame, a sizeless
    /// group (child union), a rotated vector, and a loose root vector.
    fn fast_path_scene() -> (Scene, Vec<NodeId>) {
        let mut scene = Scene::new();
        let mut frame = CanvasNode::new(NodeData::Group(GroupNode {
            clip_size: Some([200.0, 100.0]),
            background: Some(fanta_doc::Fill::solid(Color::WHITE)),
            ..GroupNode::default()
        }));
        frame.transform = Transform2D::translation(10.0, 20.0);
        let frame_id = frame.id;
        scene.insert(frame).expect("insert frame");

        let mut sizeless = CanvasNode::new(NodeData::Group(GroupNode::default()));
        sizeless.parent = Some(frame_id);
        sizeless.transform = Transform2D::rotation(0.3).then(&Transform2D::translation(5.0, 7.0));
        let sizeless_id = sizeless.id;
        scene.insert(sizeless).expect("insert group");

        let mut rect = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
            0.0,
            0.0,
            30.0,
            40.0,
            Color::BLACK,
        )));
        rect.parent = Some(sizeless_id);
        rect.transform = Transform2D::rotation(-0.7).then(&Transform2D::translation(3.0, 4.0));
        let rect_id = rect.id;
        scene.insert(rect).expect("insert rect");

        let mut loose = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
            0.0,
            0.0,
            12.0,
            8.0,
            Color::WHITE,
        )));
        loose.transform = Transform2D::translation(-40.0, 9.0);
        let loose_id = loose.id;
        scene.insert(loose).expect("insert loose rect");

        (scene, vec![frame_id, sizeless_id, rect_id, loose_id])
    }

    #[test]
    fn motion_free_world_transform_matches_the_uncached_fold() {
        let (scene, ids) = fast_path_scene();
        for &id in &ids {
            // Twice per node: a cold cache fill, then the memoized answer.
            for _ in 0..2 {
                assert_eq!(
                    evaluated_world_transform(&scene, id, None),
                    reference_world_transform(&scene, id),
                    "world transform diverged for {id:?}"
                );
            }
        }
    }

    #[test]
    fn motion_free_world_bounds_match_the_uncached_walk() {
        let (scene, ids) = fast_path_scene();
        for &id in &ids {
            for _ in 0..2 {
                assert_eq!(
                    evaluated_world_bounds(&scene, id, None),
                    reference_world_bounds(&scene, id),
                    "world bounds diverged for {id:?}"
                );
            }
        }
    }

    #[test]
    fn motion_free_world_bounds_track_edits_through_the_cache() {
        let (mut scene, ids) = fast_path_scene();
        let rect_id = ids[2];
        // Warm the caches, then move the node; the memoized fast path must
        // observe the invalidation and re-agree with the uncached walk.
        let _ = evaluated_world_bounds(&scene, rect_id, None);
        scene
            .set_transform(rect_id, Transform2D::translation(500.0, 600.0))
            .expect("set transform");
        assert_eq!(
            evaluated_world_bounds(&scene, rect_id, None),
            reference_world_bounds(&scene, rect_id),
        );
        assert_eq!(
            evaluated_world_bounds(&scene, ids[0], None),
            reference_world_bounds(&scene, ids[0]),
        );
    }

    #[test]
    fn evaluated_hit_testing_uses_the_sampled_position_and_paint_order()
    -> Result<(), fanta_doc::SceneError> {
        let mut scene = Scene::new();
        let lower = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
            0.0,
            0.0,
            10.0,
            10.0,
            Color::BLACK,
        )));
        let lower_id = lower.id;
        scene.insert(lower)?;

        let mut animated = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
            0.0,
            0.0,
            10.0,
            10.0,
            Color::WHITE,
        )));
        animated.index = scene.next_root_index();
        let animated_id = animated.id;
        scene.insert(animated)?;
        let motion = position_evaluation(animated_id, 100.0);

        assert_eq!(
            evaluated_hit_test(
                &scene,
                &motion,
                DVec2::new(5.0, 5.0),
                fanta_canvas::HitPrecision::Bounds,
                None,
            ),
            Some(lower_id)
        );
        assert_eq!(
            evaluated_hit_test(
                &scene,
                &motion,
                DVec2::new(105.0, 5.0),
                fanta_canvas::HitPrecision::Bounds,
                None,
            ),
            Some(animated_id)
        );
        Ok(())
    }

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

    #[test]
    fn rotated_selection_corners_and_handles_follow_the_node_box() {
        let local = WorldBounds::from_xywh(0.0, 0.0, 40.0, 20.0);
        let transform = Transform2D::rotation(std::f64::consts::FRAC_PI_2)
            .then(&Transform2D::translation(100.0, 50.0));
        let oriented = oriented_selection(local, transform);
        let expected_corners = [
            DVec2::new(100.0, 50.0),
            DVec2::new(100.0, 90.0),
            DVec2::new(80.0, 90.0),
            DVec2::new(80.0, 50.0),
        ];
        for (actual, expected) in oriented.corners.iter().zip(expected_corners) {
            assert!((*actual - expected).length() < 1e-9);
        }
        assert_eq!(oriented.handles[0], expected_corners[0]);
        assert_eq!(oriented.handles[4], expected_corners[2]);
    }

    #[test]
    fn group_selection_handles_use_authored_box_not_overflow_bounds() {
        let mut doc = fanta_doc::Doc::new();
        let group = CanvasNode::new(NodeData::Group(GroupNode {
            local_size: Some([40.0, 20.0]),
            ..Default::default()
        }));
        let group_id = group.id;
        doc.scene.insert(group).unwrap();
        let mut child = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
            0.0,
            0.0,
            10.0,
            10.0,
            Color::WHITE,
        )));
        child.parent = Some(group_id);
        child.transform = Transform2D::translation(100.0, 0.0);
        doc.scene.insert(child).unwrap();

        assert_eq!(
            authored_selection_bounds(&doc, group_id),
            Some(WorldBounds::from_xywh(0.0, 0.0, 40.0, 20.0))
        );
        assert_eq!(
            doc.scene.local_bounds(group_id),
            Some(WorldBounds::from_xywh(0.0, 0.0, 110.0, 20.0))
        );
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

    fn surface_key(center: [f64; 2], zoom: f64, revision: u64) -> SurfaceKey {
        SurfaceKey {
            size: (1000, 1000),
            viewport_center: center,
            viewport_zoom: zoom,
            page_root: None,
            revision,
            motion_frame: None,
        }
    }

    /// The frame is rendered with a 160-logical-px margin on each side of a
    /// 1000×1000 element, so at zoom 1 the cached frame has 160 world units of
    /// pan slack per axis.
    const FRAME_LOGICAL: (f64, f64) = (1320.0, 1320.0);
    const VISIBLE_LOGICAL: (f64, f64) = (1000.0, 1000.0);

    #[test]
    fn identical_key_reuses_the_cached_frame_even_outside_the_interval() {
        let key = surface_key([0.0, 0.0], 1.0, 7);
        assert_eq!(
            frame_decision(&key, &key, FRAME_LOGICAL, VISIBLE_LOGICAL, false),
            FrameDecision::ReuseCached
        );
    }

    #[test]
    fn a_revision_bump_always_renders_fresh() {
        // Same viewport, mid-interaction: an edit landed, so a reprojected
        // frame would show stale content.
        let cached = surface_key([0.0, 0.0], 1.0, 7);
        let requested = surface_key([0.0, 0.0], 1.0, 8);
        assert_eq!(
            frame_decision(&cached, &requested, FRAME_LOGICAL, VISIBLE_LOGICAL, true),
            FrameDecision::RenderFresh
        );
    }

    #[test]
    fn a_new_motion_sample_always_renders_fresh() {
        let mut cached = surface_key([0.0, 0.0], 1.0, 7);
        cached.motion_frame = Some(MotionFrameKey {
            clip: AnimationClipId::from_u128(11),
            playhead_ms: 100,
        });
        let mut requested = cached;
        requested.motion_frame = Some(MotionFrameKey {
            clip: AnimationClipId::from_u128(11),
            playhead_ms: 116,
        });
        assert_eq!(
            frame_decision(&cached, &requested, FRAME_LOGICAL, VISIBLE_LOGICAL, true),
            FrameDecision::RenderFresh
        );
    }

    #[test]
    fn a_small_pan_within_the_interval_reprojects() {
        // 100 world units of pan at zoom 1 stays inside the 160-unit slack.
        let cached = surface_key([0.0, 0.0], 1.0, 7);
        let requested = surface_key([100.0, 0.0], 1.0, 7);
        assert_eq!(
            frame_decision(&cached, &requested, FRAME_LOGICAL, VISIBLE_LOGICAL, true),
            FrameDecision::Reproject
        );
    }

    #[test]
    fn a_pan_beyond_the_margin_slack_renders_fresh() {
        // 200 world units exceeds the 160-unit slack: the cached frame no
        // longer covers the leading edge.
        let cached = surface_key([0.0, 0.0], 1.0, 7);
        let requested = surface_key([200.0, 0.0], 1.0, 7);
        assert_eq!(
            frame_decision(&cached, &requested, FRAME_LOGICAL, VISIBLE_LOGICAL, true),
            FrameDecision::RenderFresh
        );
    }

    #[test]
    fn pan_slack_scales_with_the_cached_zoom() {
        // At zoom 2 the cached frame covers 1320/2 = 660 world units against a
        // needed 1000/2 = 500, leaving 80 units of slack per side.
        let cached = surface_key([0.0, 0.0], 2.0, 7);
        let just_inside = surface_key([0.0, 79.0], 2.0, 7);
        let just_outside = surface_key([0.0, 81.0], 2.0, 7);
        assert_eq!(
            frame_decision(&cached, &just_inside, FRAME_LOGICAL, VISIBLE_LOGICAL, true),
            FrameDecision::Reproject
        );
        assert_eq!(
            frame_decision(&cached, &just_outside, FRAME_LOGICAL, VISIBLE_LOGICAL, true),
            FrameDecision::RenderFresh
        );
    }

    #[test]
    fn a_zoom_ratio_beyond_the_reproject_cap_renders_fresh() {
        // Zooming IN shrinks the needed world coverage, so the cached frame
        // still covers the view — only the 3× scale cap forces the fresh
        // render here.
        let cached = surface_key([0.0, 0.0], 1.0, 7);
        let at_cap = surface_key([0.0, 0.0], MacGpuRenderer::MAX_REPROJECT_SCALE, 7);
        let beyond_cap = surface_key([0.0, 0.0], MacGpuRenderer::MAX_REPROJECT_SCALE * 1.01, 7);
        assert_eq!(
            frame_decision(&cached, &at_cap, FRAME_LOGICAL, VISIBLE_LOGICAL, true),
            FrameDecision::Reproject
        );
        assert_eq!(
            frame_decision(&cached, &beyond_cap, FRAME_LOGICAL, VISIBLE_LOGICAL, true),
            FrameDecision::RenderFresh
        );
    }

    #[test]
    fn zooming_out_reprojects_within_the_throttle_despite_thin_coverage() {
        // Zooming out grows the needed world coverage past the cached frame's
        // margin almost immediately. That must NOT fall off the throttle onto
        // a full render per input event (the old degradation cliff): while the
        // interval is closed the shrinking frame reprojects with briefly
        // exposed edges, and the sharp frame lands when the throttle opens.
        let cached = surface_key([0.0, 0.0], 1.0, 7);
        let zoomed_out = surface_key([0.0, 0.0], 0.5, 7);
        assert_eq!(
            frame_decision(&cached, &zoomed_out, FRAME_LOGICAL, VISIBLE_LOGICAL, true),
            FrameDecision::Reproject
        );
        // Past the reproject scale cap the frame is too sparse to be useful.
        let far_out = surface_key(
            [0.0, 0.0],
            1.0 / (MacGpuRenderer::MAX_REPROJECT_SCALE * 1.01),
            7,
        );
        assert_eq!(
            frame_decision(&cached, &far_out, FRAME_LOGICAL, VISIBLE_LOGICAL, true),
            FrameDecision::RenderFresh
        );
        // And with the throttle open, the sharp frame renders.
        assert_eq!(
            frame_decision(&cached, &zoomed_out, FRAME_LOGICAL, VISIBLE_LOGICAL, false),
            FrameDecision::RenderFresh
        );
    }

    #[test]
    fn outside_the_render_interval_a_pan_renders_fresh() {
        // The throttle window has passed: pay for the sharp frame.
        let cached = surface_key([0.0, 0.0], 1.0, 7);
        let requested = surface_key([1.0, 0.0], 1.0, 7);
        assert_eq!(
            frame_decision(&cached, &requested, FRAME_LOGICAL, VISIBLE_LOGICAL, false),
            FrameDecision::RenderFresh
        );
    }

    #[test]
    fn an_element_resize_renders_fresh() {
        let cached = surface_key([0.0, 0.0], 1.0, 7);
        let mut requested = surface_key([0.0, 0.0], 1.0, 7);
        requested.size = (1200, 1000);
        assert_eq!(
            frame_decision(&cached, &requested, FRAME_LOGICAL, VISIBLE_LOGICAL, true),
            FrameDecision::RenderFresh
        );
    }
}
