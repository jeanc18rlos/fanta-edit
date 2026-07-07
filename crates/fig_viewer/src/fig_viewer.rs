use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

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
use fanta_doc::{Doc, NodeId, Viewport};
use fanta_fig_interop::{MapReport, fig_to_doc, read_fig};
use fanta_render::{
    AssetResolver, DecodedImage, InMemoryAssetResolver, RasterRenderer, RenderInputs,
    solve_scene_layout,
};
use file_icons::FileIcons;
#[cfg(target_os = "macos")]
use foreign_types::ForeignType;
use gpui::{
    AnyElement, App, Bounds, Context, DispatchPhase, Element, ElementId, Entity, EventEmitter,
    FocusHandle, Focusable, GlobalElementId, InspectorElementId, InteractiveElement, IntoElement,
    LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement, PinchEvent,
    Pixels, Point, Render, RenderImage, ScrollDelta, ScrollWheelEvent, SharedString, Styled, Task,
    Window, actions, div, px, relative, size,
};
use image::{Frame, RgbaImage};
use language::Capability;
use project::{Project, ProjectPath};
use settings::Settings;
#[cfg(target_os = "macos")]
use skia_safe::{
    ColorType,
    gpu::{self, SurfaceOrigin, backend_render_targets, direct_contexts, mtl},
};
use smallvec::SmallVec;
use ui::prelude::*;
use util::paths::PathExt;
use workspace::{
    ItemSettings, Pane,
    item::{Item, ProjectItem, TabContentParams},
};
use worktree::ProjectEntryId;

actions!(
    fig_viewer,
    [
        /// Zoom in the Figma preview.
        ZoomIn,
        /// Zoom out the Figma preview.
        ZoomOut,
        /// Reset preview zoom to 100%.
        ResetZoom,
        /// Fit the Figma preview to the pane.
        FitToView
    ]
);

const MIN_ZOOM: f32 = 0.1;
const MAX_ZOOM: f32 = 20.0;
const ZOOM_STEP: f32 = 1.1;
const SCROLL_LINE_MULTIPLIER: f32 = 20.0;
const RENDER_PADDING: f64 = 48.0;

pub struct FigItem {
    path: ProjectPath,
    abs_path: PathBuf,
    entry_id: Option<ProjectEntryId>,
    document: Result<FigDocument, Arc<anyhow::Error>>,
}

struct FigDocument {
    doc: Doc,
    page_root: Option<NodeId>,
    page_bounds: fanta_doc::Bounds,
    asset_resolver: Option<Arc<dyn AssetResolver>>,
    report: MapReport,
}

impl project::ProjectItem for FigItem {
    fn try_open(
        project: &Entity<Project>,
        path: &ProjectPath,
        cx: &mut App,
    ) -> Option<Task<Result<Entity<Self>>>> {
        let abs_path = project.read(cx).absolute_path(path, cx);
        if !is_fig_file(path) && !abs_path.as_deref().is_some_and(path_has_fig_extension) {
            return None;
        }

        let path = path.clone();
        let entry_id = project
            .read(cx)
            .entry_for_path(&path, cx)
            .map(|entry| entry.id);

        Some(cx.spawn(async move |cx| {
            let abs_path = abs_path.context("Figma viewer only supports local .fig files")?;
            let document = std::fs::read(&abs_path)
                .with_context(|| format!("reading {}", abs_path.display()))
                .and_then(|bytes| load_fig_document(&bytes));

            let document = document.map_err(Arc::new);
            let item = cx.new(|_| Self {
                path,
                abs_path,
                entry_id,
                document,
            });
            Ok(item)
        }))
    }

    fn entry_id(&self, _: &App) -> Option<ProjectEntryId> {
        self.entry_id
    }

    fn project_path(&self, _: &App) -> Option<ProjectPath> {
        Some(self.path.clone())
    }

    fn is_dirty(&self) -> bool {
        false
    }
}

fn is_fig_file(path: &ProjectPath) -> bool {
    path_has_fig_extension(path.path.as_std_path())
}

fn path_has_fig_extension(path: &Path) -> bool {
    path.extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("fig"))
}

fn load_fig_document(bytes: &[u8]) -> Result<FigDocument> {
    let fig = read_fig(bytes).context("parsing .fig")?;
    let (mut doc, report, assets) = fig_to_doc(&fig).context("mapping .fig to Fanta document")?;
    let page_root = doc.active_page().or_else(|| doc.pages().first().copied());
    if let Some(page_root) = page_root {
        solve_scene_layout(&mut doc.scene, page_root);
    }
    let resolver = decode_assets(assets);
    let asset_resolver =
        (!resolver.is_empty()).then(|| Arc::new(resolver) as Arc<dyn AssetResolver>);
    let page_bounds = page_bounds(&doc, page_root);

    Ok(FigDocument {
        doc,
        page_root,
        page_bounds,
        asset_resolver,
        report,
    })
}

fn render_fig_canvas(
    document: &FigDocument,
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
    renderer.render_page_with(
        &document.doc.scene,
        &render_viewport,
        document.page_root,
        &inputs,
    );
    render_image_from_rgba(width, height, renderer.copy_rgba(), scale_factor)
}

fn page_bounds(doc: &Doc, page_root: Option<NodeId>) -> fanta_doc::Bounds {
    let mut bounds: Option<fanta_doc::Bounds> = None;
    let mut include = |node_id| {
        if let Some(node_bounds) = doc.scene.world_bounds(node_id)
            && node_bounds.is_finite()
            && node_bounds.width() > 0.0
            && node_bounds.height() > 0.0
        {
            bounds = Some(match bounds {
                Some(bounds) => bounds.union(&node_bounds),
                None => node_bounds,
            });
        }
    };

    if let Some(page_root) = page_root {
        for node_id in doc.scene.descendants_of(page_root) {
            include(node_id);
        }
    } else {
        for root in doc.scene.roots() {
            for node_id in doc.scene.descendants_of(*root) {
                include(node_id);
            }
        }
    }

    bounds.unwrap_or_else(|| fanta_doc::Bounds::from_xywh(0.0, 0.0, 1024.0, 768.0))
}

fn fit_bounds(bounds: fanta_doc::Bounds, screen_size: (f64, f64), padding: f64) -> Viewport {
    let usable_width = (screen_size.0 - padding * 2.0).max(1.0);
    let usable_height = (screen_size.1 - padding * 2.0).max(1.0);
    let bounds_width = bounds.width().max(1.0);
    let bounds_height = bounds.height().max(1.0);
    let zoom = (usable_width / bounds_width)
        .min(usable_height / bounds_height)
        .clamp(f64::from(MIN_ZOOM), f64::from(MAX_ZOOM));
    Viewport {
        center: [
            bounds.min_x + bounds.width() * 0.5,
            bounds.min_y + bounds.height() * 0.5,
        ],
        zoom,
    }
}

fn pan_viewport(viewport: Viewport, screen_delta: (f64, f64)) -> Viewport {
    let inv_zoom = 1.0 / viewport.zoom.max(f64::EPSILON);
    Viewport {
        center: [
            viewport.center[0] - screen_delta.0 * inv_zoom,
            viewport.center[1] - screen_delta.1 * inv_zoom,
        ],
        zoom: viewport.zoom,
    }
}

fn screen_to_world(screen: (f64, f64), viewport: Viewport, screen_size: (f64, f64)) -> (f64, f64) {
    let inv_zoom = 1.0 / viewport.zoom.max(f64::EPSILON);
    (
        (screen.0 - screen_size.0 * 0.5) * inv_zoom + viewport.center[0],
        (screen.1 - screen_size.1 * 0.5) * inv_zoom + viewport.center[1],
    )
}

fn zoom_viewport_at(
    viewport: Viewport,
    screen_anchor: (f64, f64),
    factor: f64,
    screen_size: (f64, f64),
) -> Viewport {
    let world_before = screen_to_world(screen_anchor, viewport, screen_size);
    let provisional = Viewport {
        center: viewport.center,
        zoom: (viewport.zoom * factor).clamp(f64::from(MIN_ZOOM), f64::from(MAX_ZOOM)),
    };
    let world_after = screen_to_world(screen_anchor, provisional, screen_size);
    Viewport {
        center: [
            provisional.center[0] - (world_after.0 - world_before.0),
            provisional.center[1] - (world_after.1 - world_before.1),
        ],
        zoom: provisional.zoom,
    }
}

fn same_viewport(left: Viewport, right: Viewport) -> bool {
    (left.center[0] - right.center[0]).abs() < 0.001
        && (left.center[1] - right.center[1]).abs() < 0.001
        && (left.zoom - right.zoom).abs() < 0.0001
}

fn bounds_size(bounds: Bounds<Pixels>) -> (f64, f64) {
    (
        f64::from(f32::from(bounds.size.width)),
        f64::from(f32::from(bounds.size.height)),
    )
}

fn screen_position_in_bounds(position: Point<Pixels>, bounds: Bounds<Pixels>) -> (f64, f64) {
    (
        f64::from(f32::from(position.x - bounds.origin.x)),
        f64::from(f32::from(position.y - bounds.origin.y)),
    )
}

fn render_size_for_bounds(bounds: Bounds<Pixels>, scale_factor: f32) -> (u32, u32) {
    let width = (f32::from(bounds.size.width) * scale_factor)
        .round()
        .max(1.0) as u32;
    let height = (f32::from(bounds.size.height) * scale_factor)
        .round()
        .max(1.0) as u32;
    (width, height)
}

fn decode_assets(
    assets: std::collections::HashMap<fanta_doc::AssetId, Vec<u8>>,
) -> InMemoryAssetResolver {
    let mut resolver = InMemoryAssetResolver::new();
    for (asset_id, bytes) in assets {
        match image::load_from_memory(&bytes) {
            Ok(image) => {
                let image = image.to_rgba8();
                let (width, height) = image.dimensions();
                resolver.insert(
                    asset_id,
                    DecodedImage::new(Arc::new(image.into_raw()), width, height),
                );
            }
            Err(error) => {
                log::warn!("failed to decode embedded .fig image asset {asset_id}: {error}");
            }
        }
    }
    resolver
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

pub struct FigView {
    item: Entity<FigItem>,
    project: Entity<Project>,
    focus_handle: FocusHandle,
    viewport: Option<Viewport>,
    last_mouse_position: Option<Point<Pixels>>,
    container_bounds: Option<Bounds<Pixels>>,
    rendered_canvas: Option<RenderedCanvas>,
    #[cfg(target_os = "macos")]
    gpu_renderer: Option<MacGpuRenderer>,
}

struct RenderedCanvas {
    image: Arc<RenderImage>,
    size: (u32, u32),
    viewport: Viewport,
}

enum PaintCanvas {
    #[cfg(target_os = "macos")]
    Surface(CVPixelBuffer),
    Image(Arc<RenderImage>),
}

#[cfg(target_os = "macos")]
struct MacGpuRenderer {
    raster_renderer: RasterRenderer,
    direct_context: skia_safe::gpu::DirectContext,
    texture_cache: CVMetalTextureCache,
    _device: metal::Device,
    _command_queue: metal::CommandQueue,
    size: (u32, u32),
}

#[cfg(target_os = "macos")]
impl MacGpuRenderer {
    fn new(size: (u32, u32)) -> Result<Self> {
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
        })
    }

    fn resize(&mut self, size: (u32, u32)) -> Result<()> {
        if self.size == size {
            return Ok(());
        }

        self.raster_renderer =
            RasterRenderer::new(size.0, size.1).context("resizing Skia renderer state")?;
        self.size = size;
        Ok(())
    }

    fn render(
        &mut self,
        document: &FigDocument,
        size: (u32, u32),
        viewport: Viewport,
        scale_factor: f32,
    ) -> Result<CVPixelBuffer> {
        self.resize(size)?;
        if let Some(asset_resolver) = document.asset_resolver.clone() {
            self.raster_renderer.set_asset_resolver(asset_resolver);
        }

        let pixel_buffer = create_bgra_pixel_buffer(size.0, size.1)?;
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
            document.page_root,
            &inputs,
        );
        self.direct_context.flush_submit_and_sync_cpu();
        drop(surface);

        Ok(pixel_buffer)
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

impl FigView {
    fn new(
        item: Entity<FigItem>,
        project: Entity<Project>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            item,
            project,
            focus_handle: cx.focus_handle(),
            viewport: None,
            last_mouse_position: None,
            container_bounds: None,
            rendered_canvas: None,
            #[cfg(target_os = "macos")]
            gpu_renderer: None,
        }
    }

    fn is_dragging(&self) -> bool {
        self.last_mouse_position.is_some()
    }

    fn reset_rendered_canvas(&mut self) {
        self.rendered_canvas = None;
    }

    fn render_cpu_canvas(
        &mut self,
        document: &FigDocument,
        size: (u32, u32),
        viewport: Viewport,
        scale_factor: f32,
    ) -> Result<Arc<RenderImage>> {
        if let Some(rendered) = &self.rendered_canvas
            && rendered.size == size
            && same_viewport(rendered.viewport, viewport)
        {
            return Ok(rendered.image.clone());
        }

        let image = render_fig_canvas(document, size.0, size.1, viewport, scale_factor)?;
        self.rendered_canvas = Some(RenderedCanvas {
            image: image.clone(),
            size,
            viewport,
        });
        Ok(image)
    }

    fn set_viewport(&mut self, viewport: Viewport, cx: &mut Context<Self>) {
        self.viewport = Some(viewport);
        self.reset_rendered_canvas();
        cx.notify();
    }

    fn zoom_in(&mut self, _: &ZoomIn, _window: &mut Window, cx: &mut Context<Self>) {
        self.zoom_by(f64::from(ZOOM_STEP), None, cx);
    }

    fn zoom_out(&mut self, _: &ZoomOut, _window: &mut Window, cx: &mut Context<Self>) {
        self.zoom_by(1.0 / f64::from(ZOOM_STEP), None, cx);
    }

    fn reset_zoom(&mut self, _: &ResetZoom, _window: &mut Window, cx: &mut Context<Self>) {
        let viewport = self
            .container_bounds
            .zip(self.item.read(cx).document.as_ref().ok())
            .map(|(bounds, document)| {
                fit_bounds(document.page_bounds, bounds_size(bounds), RENDER_PADDING)
            });
        if let Some(viewport) = viewport {
            self.set_viewport(viewport, cx);
        }
    }

    fn fit_to_view(&mut self, _: &FitToView, _window: &mut Window, cx: &mut Context<Self>) {
        let viewport = self
            .container_bounds
            .zip(self.item.read(cx).document.as_ref().ok())
            .map(|(bounds, document)| {
                fit_bounds(document.page_bounds, bounds_size(bounds), RENDER_PADDING)
            });
        if let Some(viewport) = viewport {
            self.set_viewport(viewport, cx);
        }
    }

    fn zoom_by(&mut self, factor: f64, anchor: Option<Point<Pixels>>, cx: &mut Context<Self>) {
        let Some((viewport, bounds)) = self.viewport.zip(self.container_bounds) else {
            return;
        };
        let anchor = anchor
            .map(|point| screen_position_in_bounds(point, bounds))
            .unwrap_or_else(|| {
                let size = bounds_size(bounds);
                (size.0 * 0.5, size.1 * 0.5)
            });
        self.set_viewport(
            zoom_viewport_at(viewport, anchor, factor, bounds_size(bounds)),
            cx,
        );
    }

    fn handle_scroll_wheel(
        &mut self,
        event: &ScrollWheelEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.modifiers.control || event.modifiers.platform {
            let delta: f32 = match event.delta {
                ScrollDelta::Pixels(pixels) => pixels.y.into(),
                ScrollDelta::Lines(lines) => lines.y * SCROLL_LINE_MULTIPLIER,
            };
            let zoom_factor = if delta > 0.0 {
                1.0 + delta.abs() * 0.01
            } else {
                1.0 / (1.0 + delta.abs() * 0.01)
            };
            self.zoom_by(f64::from(zoom_factor), Some(event.position), cx);
        } else {
            let delta = match event.delta {
                ScrollDelta::Pixels(pixels) => pixels,
                ScrollDelta::Lines(lines) => lines.map(|line| px(line * SCROLL_LINE_MULTIPLIER)),
            };
            if let Some(viewport) = self.viewport {
                let delta_x = f64::from(f32::from(delta.x));
                let delta_y = f64::from(f32::from(delta.y));
                self.set_viewport(pan_viewport(viewport, (delta_x, delta_y)), cx);
            }
        }
    }

    fn handle_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.button == MouseButton::Left || event.button == MouseButton::Middle {
            self.last_mouse_position = Some(event.position);
            cx.notify();
        }
    }

    fn handle_mouse_up(
        &mut self,
        _event: &MouseUpEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.last_mouse_position = None;
        cx.notify();
    }

    fn handle_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.is_dragging() {
            if let Some(last_position) = self.last_mouse_position {
                let delta = event.position - last_position;
                if let Some(viewport) = self.viewport {
                    let delta_x = f64::from(f32::from(delta.x));
                    let delta_y = f64::from(f32::from(delta.y));
                    self.set_viewport(pan_viewport(viewport, (delta_x, delta_y)), cx);
                }
            }
            self.last_mouse_position = Some(event.position);
        }
    }

    fn handle_pinch(&mut self, event: &PinchEvent, _window: &mut Window, cx: &mut Context<Self>) {
        self.zoom_by(f64::from(1.0 + event.delta), Some(event.position), cx);
    }
}

struct FigContentElement {
    view: Entity<FigView>,
}

impl FigContentElement {
    fn new(view: Entity<FigView>) -> Self {
        Self { view }
    }
}

impl IntoElement for FigContentElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for FigContentElement {
    type RequestLayoutState = ();
    type PrepaintState = Option<bool>;

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
        let (viewport, is_dragging) = {
            let view = self.view.read(cx);
            let item = view.item.read(cx);
            let Ok(document) = item.document.as_ref() else {
                return None;
            };
            let viewport = view
                .viewport
                .unwrap_or_else(|| fit_bounds(document.page_bounds, logical_size, RENDER_PADDING));
            (viewport, view.is_dragging())
        };

        self.view.update(cx, |this, _| {
            this.container_bounds = Some(bounds);
            this.viewport = Some(viewport);
        });
        Some(is_dragging)
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
        let Some(is_dragging) = prepaint.take() else {
            return;
        };

        if is_dragging {
            let view = self.view.downgrade();
            window.on_mouse_event(move |_event: &MouseUpEvent, phase, _window, cx| {
                if phase == DispatchPhase::Bubble
                    && let Some(view) = view.upgrade()
                {
                    view.update(cx, |this, cx| {
                        this.last_mouse_position = None;
                        cx.notify();
                    });
                }
            });
        }

        let scale_factor = window.scale_factor();
        let render_size = render_size_for_bounds(bounds, scale_factor);
        let paint_canvas = self.view.update(cx, |this, cx| {
            let viewport = this
                .viewport
                .ok_or_else(|| anyhow!("Figma canvas viewport was not initialized"))?;
            let item = this.item.clone();
            let item = item.read(cx);
            let document = item
                .document
                .as_ref()
                .map_err(|error| anyhow!(error.to_string()))?;

            #[cfg(target_os = "macos")]
            {
                let mut gpu_renderer = match this.gpu_renderer.take() {
                    Some(renderer) => renderer,
                    None => MacGpuRenderer::new(render_size)?,
                };
                let gpu_result = gpu_renderer.render(document, render_size, viewport, scale_factor);
                this.gpu_renderer = Some(gpu_renderer);
                match gpu_result {
                    Ok(surface) => {
                        this.rendered_canvas = None;
                        return Ok(PaintCanvas::Surface(surface));
                    }
                    Err(error) => {
                        log::warn!(
                            "failed to render .fig canvas with Skia Metal, falling back to CPU: {error:#}"
                        );
                    }
                }
            }

            this.render_cpu_canvas(document, render_size, viewport, scale_factor)
                .map(PaintCanvas::Image)
        });

        match paint_canvas {
            #[cfg(target_os = "macos")]
            Ok(PaintCanvas::Surface(surface)) => {
                window.paint_surface(bounds, surface);
            }
            Ok(PaintCanvas::Image(image)) => {
                if let Err(error) = window.paint_image(bounds, Default::default(), image, 0, false)
                {
                    log::warn!("failed to paint .fig CPU canvas: {error:#}");
                }
            }
            Err(error) => {
                log::warn!("failed to render .fig canvas: {error:#}");
            }
        }
    }
}

pub enum FigViewEvent {}

impl EventEmitter<FigViewEvent> for FigView {}

impl Item for FigView {
    type Event = FigViewEvent;

    fn for_each_project_item(
        &self,
        cx: &App,
        f: &mut dyn FnMut(gpui::EntityId, &dyn project::ProjectItem),
    ) {
        f(self.item.entity_id(), self.item.read(cx));
    }

    fn tab_content_text(&self, _: usize, cx: &App) -> SharedString {
        self.item
            .read(cx)
            .abs_path
            .file_name()
            .and_then(|file_name| file_name.to_str())
            .unwrap_or("Figma")
            .into()
    }

    fn tab_tooltip_text(&self, cx: &App) -> Option<SharedString> {
        Some(
            self.item
                .read(cx)
                .abs_path
                .compact()
                .to_string_lossy()
                .into_owned()
                .into(),
        )
    }

    fn tab_icon(&self, _: &Window, cx: &App) -> Option<Icon> {
        let path = &self.item.read(cx).abs_path;
        ItemSettings::get_global(cx)
            .file_icons
            .then(|| FileIcons::get_icon(path, cx))
            .flatten()
            .map(Icon::from_path)
    }

    fn buffer_kind(&self, _: &App) -> workspace::item::ItemBufferKind {
        workspace::item::ItemBufferKind::Singleton
    }

    fn capability(&self, _: &App) -> Capability {
        Capability::ReadOnly
    }

    fn can_split(&self) -> bool {
        true
    }

    fn clone_on_split(
        &self,
        _workspace_id: Option<workspace::WorkspaceId>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> Task<Option<Entity<Self>>>
    where
        Self: Sized,
    {
        Task::ready(Some(cx.new(|cx| Self {
            item: self.item.clone(),
            project: self.project.clone(),
            focus_handle: cx.focus_handle(),
            viewport: self.viewport,
            last_mouse_position: None,
            container_bounds: None,
            rendered_canvas: None,
            #[cfg(target_os = "macos")]
            gpu_renderer: None,
        })))
    }

    fn tab_content(&self, params: TabContentParams, _window: &Window, cx: &App) -> AnyElement {
        Label::new(self.tab_content_text(params.detail.unwrap_or_default(), cx))
            .single_line()
            .color(params.text_color())
            .when(params.preview, |this| this.italic())
            .into_any_element()
    }
}

impl ProjectItem for FigView {
    type Item = FigItem;

    fn for_project_item(
        project: Entity<Project>,
        _: Option<&Pane>,
        item: Entity<Self::Item>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self
    where
        Self: Sized,
    {
        Self::new(item, project, window, cx)
    }
}

impl Focusable for FigView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for FigView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let report_text = self.item.read(cx).document.as_ref().ok().map(|document| {
            let skipped: usize = document.report.skipped_by_type.values().sum();
            let page_name = document
                .page_root
                .and_then(|page| document.doc.page_name(page))
                .unwrap_or("Page");
            format!(
                "{page_name} · {} mapped, {skipped} skipped",
                document.report.mapped
            )
        });
        let error = self.item.read(cx).document.as_ref().err().cloned();
        let has_error = error.is_some();

        div()
            .track_focus(&self.focus_handle(cx))
            .key_context("FigViewer")
            .on_action(cx.listener(Self::zoom_in))
            .on_action(cx.listener(Self::zoom_out))
            .on_action(cx.listener(Self::reset_zoom))
            .on_action(cx.listener(Self::fit_to_view))
            .size_full()
            .relative()
            .bg(cx.theme().colors().editor_background)
            .when_some(error, |this, error| {
                this.child(
                    v_flex()
                        .size_full()
                        .items_center()
                        .justify_center()
                        .gap_2()
                        .child(Label::new("Could not open Figma file").size(LabelSize::Large))
                        .child(Label::new(error.to_string()).color(Color::Muted)),
                )
            })
            .when(!has_error, |this| {
                this.child(
                    div()
                        .id("fig-container")
                        .size_full()
                        .overflow_hidden()
                        .cursor(if self.is_dragging() {
                            gpui::CursorStyle::ClosedHand
                        } else {
                            gpui::CursorStyle::OpenHand
                        })
                        .on_scroll_wheel(cx.listener(Self::handle_scroll_wheel))
                        .on_pinch(cx.listener(Self::handle_pinch))
                        .on_mouse_down(MouseButton::Left, cx.listener(Self::handle_mouse_down))
                        .on_mouse_down(MouseButton::Middle, cx.listener(Self::handle_mouse_down))
                        .on_mouse_up(MouseButton::Left, cx.listener(Self::handle_mouse_up))
                        .on_mouse_up(MouseButton::Middle, cx.listener(Self::handle_mouse_up))
                        .on_mouse_move(cx.listener(Self::handle_mouse_move))
                        .child(FigContentElement::new(cx.entity())),
                )
            })
            .when_some(report_text, |this, report_text| {
                this.child(
                    div()
                        .absolute()
                        .right_2()
                        .bottom_2()
                        .px_2()
                        .py_1()
                        .bg(cx.theme().colors().panel_background)
                        .border_1()
                        .border_color(cx.theme().colors().border)
                        .child(Label::new(report_text).size(LabelSize::Small)),
                )
            })
    }
}

pub fn init(cx: &mut App) {
    workspace::register_project_item::<FigView>(cx);
}
