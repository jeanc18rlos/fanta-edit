use super::*;
pub use crate::transition::{OverlayComposition, TransitionComposition};
use skia_safe::{AlphaType, BlendMode, Canvas, ColorType, Image, ImageInfo, Paint, Surface};

pub const DEFAULT_PRESENTATION_BUDGET_BYTES: usize = 128 << 20;

#[derive(Debug, thiserror::Error)]
pub enum GpuPresentationError {
    #[error("invalid presentation surface dimensions or display scale")]
    InvalidDimensions,
    #[error("presentation needs {required} surface bytes, exceeding its {budget} byte budget")]
    SurfaceBudget { required: usize, budget: usize },
    #[error("could not allocate a {width}x{height} compatible presentation surface")]
    SurfaceCreate { width: u32, height: u32 },
    #[error(transparent)]
    Render(#[from] RenderError),
    #[error("a presentation effect could not be rendered without reducing fidelity")]
    EffectFailed,
    #[error("presentation artwork could not be rendered from the supplied resources")]
    IncompleteArtwork,
}

impl GpuPresentationError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidDimensions => "InvalidDimensions",
            Self::SurfaceBudget { .. } => "SurfaceBudgetExceeded",
            Self::SurfaceCreate { .. } => "SurfaceAllocationFailed",
            Self::Render(_) => "RenderFailed",
            Self::EffectFailed => "EffectFailed",
            Self::IncompleteArtwork => "IncompleteArtwork",
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct GpuPresentationMetrics {
    pub render: RenderMetrics,
    pub rendered_layers: u32,
    pub peak_surface_bytes: usize,
    pub whole_frame_readbacks: u32,
}

pub struct GpuPresenter {
    budget_bytes: usize,
}

struct PresentationTarget<'a> {
    playback: &'a mut PlaybackSession,
    renderer: &'a mut RasterRenderer,
}

impl std::ops::Deref for PresentationTarget<'_> {
    type Target = PlaybackSession;

    fn deref(&self) -> &Self::Target {
        self.playback
    }
}

impl std::ops::DerefMut for PresentationTarget<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.playback
    }
}

impl Default for GpuPresenter {
    fn default() -> Self {
        Self::with_budget(DEFAULT_PRESENTATION_BUDGET_BYTES)
    }
}

impl GpuPresenter {
    pub fn with_budget(budget_bytes: usize) -> Self {
        Self { budget_bytes }
    }

    pub fn render(
        &mut self,
        session: &mut PresentSession,
        canvas: &Canvas,
        width: u32,
        height: u32,
        display_scale: f64,
    ) -> Result<GpuPresentationMetrics, GpuPresentationError> {
        surface_bytes(width, height, display_scale)?;
        session.synchronize_renderer()?;
        self.render_playback(
            &mut session.playback,
            &mut session.renderer,
            canvas,
            width,
            height,
            display_scale,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn render_playback(
        &mut self,
        playback: &mut PlaybackSession,
        renderer: &mut RasterRenderer,
        canvas: &Canvas,
        width: u32,
        height: u32,
        display_scale: f64,
    ) -> Result<GpuPresentationMetrics, GpuPresentationError> {
        let mut target = PresentationTarget { playback, renderer };
        self.render_target(&mut target, canvas, width, height, display_scale)
    }

    fn render_target(
        &mut self,
        session: &mut PresentationTarget<'_>,
        canvas: &Canvas,
        width: u32,
        height: u32,
        display_scale: f64,
    ) -> Result<GpuPresentationMetrics, GpuPresentationError> {
        let started = std::time::Instant::now();
        let readbacks_before = session.renderer.whole_frame_readback_count();
        let bytes = surface_bytes(width, height, display_scale)?;
        // The host destination stays live alongside the staging surface,
        // endpoint images, and a possible isolated variant-transition layer.
        let surfaces = if session.active_variant_transition.is_some() {
            5
        } else if session.active_transition.is_some() {
            4
        } else {
            3
        };
        let required = bytes
            .checked_mul(surfaces)
            .ok_or(GpuPresentationError::InvalidDimensions)?;
        if required > self.budget_bytes {
            return Err(GpuPresentationError::SurfaceBudget {
                required,
                budget: self.budget_bytes,
            });
        }
        let previous_scale = session.renderer.display_scale;
        session.renderer.display_scale = display_scale;
        let mut metrics = GpuPresentationMetrics {
            peak_surface_bytes: required,
            ..Default::default()
        };
        let result: Result<(), GpuPresentationError> = (|| {
            let mut surface = compatible_surface(canvas, width, height)?;
            if let Some(mut active) = session.active_variant_transition.take() {
                let result: Result<(), GpuPresentationError> = (|| {
                    let hidden: Vec<_> = active.layers.iter().map(|layer| layer.instance).collect();
                    render_backdrop(
                        session,
                        surface.canvas(),
                        width,
                        height,
                        &hidden,
                        &mut metrics,
                    )?;
                    let eased = ease(active.clock.easing, active.clock.progress());
                    for layer in &mut active.layers {
                        if matches!(active.clock.style, TransitionStyle::SmartAnimate)
                            && let Some(smart) = &mut layer.smart_animate
                        {
                            smart.apply(eased);
                            let mut scene = smart.scene.clone();
                            session.apply_destination_motion_to_materialized_root(
                                &mut scene,
                                smart.root,
                                &layer.target_instance,
                                layer.target_parent_world,
                                layer.target_master_opacity,
                            );
                            let image = render_scene(
                                session,
                                canvas,
                                width,
                                height,
                                &scene,
                                smart.root,
                                layer.viewport,
                                false,
                                &mut metrics,
                            )?;
                            draw_layer(
                                surface.canvas(),
                                &image,
                                (0.0, 0.0),
                                1.0,
                                BlendMode::SrcOver,
                            );
                        } else {
                            let outgoing = render_scene(
                                session,
                                canvas,
                                width,
                                height,
                                &layer.from_scene,
                                layer.from_root,
                                layer.viewport,
                                false,
                                &mut metrics,
                            )?;
                            let mut incoming_scene = layer.to_scene.clone();
                            session.apply_destination_motion_to_materialized_root(
                                &mut incoming_scene,
                                layer.to_root,
                                &layer.target_instance,
                                layer.target_parent_world,
                                layer.target_master_opacity,
                            );
                            let incoming = render_scene(
                                session,
                                canvas,
                                width,
                                height,
                                &incoming_scene,
                                layer.to_root,
                                layer.viewport,
                                false,
                                &mut metrics,
                            )?;
                            let count = surface
                                .canvas()
                                .save_layer(&skia_safe::canvas::SaveLayerRec::default());
                            draw_transition(
                                surface.canvas(),
                                active.clock.style,
                                eased,
                                width,
                                height,
                                &outgoing,
                                &incoming,
                            );
                            surface.canvas().restore_to_count(count);
                        }
                    }
                    Ok(())
                })();
                session.active_variant_transition = Some(active);
                result?;
            } else {
                render_base(session, surface.canvas(), width, height, &mut metrics)?;
                let overlays = session.overlays.clone();
                for (index, entry) in overlays.iter().enumerate() {
                    let entering = session
                        .active_overlay_transition
                        .filter(|active| active.root == entry.root && index + 1 == overlays.len());
                    let (style, visible) =
                        entering.map_or((TransitionStyle::Instant, 1.0), |active| {
                            (
                                active.clock.style,
                                ease(active.clock.easing, active.clock.progress()),
                            )
                        });
                    if entry.settings.background_dim {
                        draw_scrim(surface.canvas(), SCRIM_DIM * visible);
                    }
                    let overlay = render_frame(
                        session,
                        canvas,
                        width,
                        height,
                        entry.root,
                        entry.viewport,
                        Some(entry.layer),
                        true,
                        &[],
                        &mut metrics,
                    )?;
                    draw_overlay(surface.canvas(), style, visible, width, height, &overlay);
                }
                if let Some(exit) = session.active_overlay_exit.clone() {
                    let visible =
                        exit.start_visible * (1.0 - ease(exit.clock.easing, exit.clock.progress()));
                    if exit.entry.settings.background_dim {
                        draw_scrim(surface.canvas(), SCRIM_DIM * visible);
                    }
                    let image = render_frame(
                        session,
                        canvas,
                        width,
                        height,
                        exit.entry.root,
                        exit.entry.viewport,
                        Some(exit.entry.layer),
                        false,
                        &[],
                        &mut metrics,
                    )?;
                    draw_overlay(
                        surface.canvas(),
                        exit.clock.style,
                        visible,
                        width,
                        height,
                        &image,
                    );
                }
            }
            commit_presentation(canvas, &mut surface, &metrics)
        })();
        session.renderer.display_scale = previous_scale;
        result?;
        metrics.whole_frame_readbacks = session
            .renderer
            .whole_frame_readback_count()
            .saturating_sub(readbacks_before)
            .min(u64::from(u32::MAX)) as u32;
        metrics.render.frame_micros =
            started.elapsed().as_micros().min(u128::from(u64::MAX)) as u64;
        Ok(metrics)
    }
}

fn commit_presentation(
    canvas: &Canvas,
    surface: &mut Surface,
    metrics: &GpuPresentationMetrics,
) -> Result<(), GpuPresentationError> {
    if metrics.render.effect_failed {
        return Err(GpuPresentationError::EffectFailed);
    }
    if metrics.render.incomplete_artwork {
        return Err(GpuPresentationError::IncompleteArtwork);
    }
    let image = surface.image_snapshot();
    canvas.save();
    canvas.reset_matrix();
    draw_layer(canvas, &image, (0.0, 0.0), 1.0, BlendMode::Src);
    canvas.restore();
    Ok(())
}

fn surface_bytes(
    width: u32,
    height: u32,
    display_scale: f64,
) -> Result<usize, GpuPresentationError> {
    if width == 0
        || height == 0
        || width > i32::MAX as u32
        || height > i32::MAX as u32
        || !display_scale.is_finite()
        || display_scale <= 0.0
    {
        return Err(GpuPresentationError::InvalidDimensions);
    }
    (width as usize)
        .checked_mul(height as usize)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or(GpuPresentationError::InvalidDimensions)
}

impl PresentSession {
    pub fn release_graphics_resources(&mut self) {
        self.renderer.clear_caches();
    }

    pub fn set_render_cache_budgets(&mut self, image_bytes: usize, effect_bytes: usize) {
        self.renderer.set_image_cache_budget_bytes(image_bytes);
        self.renderer.set_layer_cache_budget_bytes(effect_bytes);
    }

    pub fn render_to_canvas(
        &mut self,
        canvas: &Canvas,
        width: u32,
        height: u32,
        display_scale: f64,
    ) -> Result<GpuPresentationMetrics, GpuPresentationError> {
        GpuPresenter::default().render(self, canvas, width, height, display_scale)
    }
}

fn compatible_surface(
    canvas: &Canvas,
    width: u32,
    height: u32,
) -> Result<Surface, GpuPresentationError> {
    let info = ImageInfo::new(
        (width as i32, height as i32),
        ColorType::RGBA8888,
        AlphaType::Premul,
        None,
    );
    canvas
        .new_surface(&info, None)
        .ok_or(GpuPresentationError::SurfaceCreate { width, height })
}

fn draw_layer(canvas: &Canvas, image: &Image, offset: (f64, f64), opacity: f64, blend: BlendMode) {
    let mut paint = Paint::default();
    paint
        .set_alpha_f(opacity.clamp(0.0, 1.0) as f32)
        .set_blend_mode(blend);
    canvas.draw_image(
        image,
        (offset.0.round() as f32, offset.1.round() as f32),
        Some(&paint),
    );
}

fn draw_scrim(canvas: &Canvas, dim: f64) {
    let factor = (1.0 - dim.clamp(0.0, 1.0)) as f32;
    // Modulate multiplies premultiplied RGB and leaves destination alpha
    // intact, including transparent edges and translucent prototypes.
    canvas.draw_color(
        skia_safe::Color4f::new(factor, factor, factor, 1.0),
        BlendMode::Modulate,
    );
}

fn draw_overlay(
    canvas: &Canvas,
    style: TransitionStyle,
    visible: f64,
    width: u32,
    height: u32,
    image: &Image,
) {
    let plan = OverlayComposition::new(style, visible, f64::from(width), f64::from(height));
    draw_layer(canvas, image, plan.offset, plan.opacity, BlendMode::SrcOver);
}

fn draw_transition(
    canvas: &Canvas,
    style: TransitionStyle,
    eased: f64,
    width: u32,
    height: u32,
    outgoing: &Image,
    incoming: &Image,
) {
    match TransitionComposition::new(style, eased, f64::from(width), f64::from(height)) {
        TransitionComposition::Incoming => {
            draw_layer(canvas, incoming, (0.0, 0.0), 1.0, BlendMode::Src)
        }
        TransitionComposition::Crossfade { incoming_weight } => {
            // Source-over would attenuate the outgoing endpoint twice. Sum
            // the two premultiplied weighted endpoints instead.
            draw_layer(
                canvas,
                outgoing,
                (0.0, 0.0),
                1.0 - incoming_weight,
                BlendMode::Src,
            );
            draw_layer(
                canvas,
                incoming,
                (0.0, 0.0),
                incoming_weight,
                BlendMode::Plus,
            );
        }
        TransitionComposition::Layers {
            outgoing_offset,
            incoming_offset,
            outgoing_on_top,
        } => {
            canvas.clear(skia_safe::Color::TRANSPARENT);
            if outgoing_on_top {
                draw_layer(canvas, incoming, incoming_offset, 1.0, BlendMode::SrcOver);
                draw_layer(canvas, outgoing, outgoing_offset, 1.0, BlendMode::SrcOver);
            } else {
                draw_layer(canvas, outgoing, outgoing_offset, 1.0, BlendMode::SrcOver);
                draw_layer(canvas, incoming, incoming_offset, 1.0, BlendMode::SrcOver);
            }
        }
    }
}

fn collect_metrics(metrics: &mut GpuPresentationMetrics, rendered: RenderMetrics) {
    metrics.rendered_layers += 1;
    metrics.render.nodes_visited += rendered.nodes_visited;
    metrics.render.nodes_drawn += rendered.nodes_drawn;
    metrics.render.nodes_culled += rendered.nodes_culled;
    metrics.render.paths_built += rendered.paths_built;
    metrics.render.instance_indexes_built += rendered.instance_indexes_built;
    metrics.render.effect_layers += rendered.effect_layers;
    metrics.render.opacity_folds += rendered.opacity_folds;
    metrics.render.layer_cache_hits += rendered.layer_cache_hits;
    metrics.render.layer_cache_misses += rendered.layer_cache_misses;
    metrics.render.incomplete_artwork |= rendered.incomplete_artwork;
    metrics.render.non_artwork_content |= rendered.non_artwork_content;
    metrics.render.effect_failed |= rendered.effect_failed;
}

#[allow(clippy::too_many_arguments)]
fn render_scene(
    session: &mut PresentationTarget<'_>,
    canvas: &Canvas,
    width: u32,
    height: u32,
    scene: &Scene,
    root: NodeId,
    viewport: Viewport,
    include_motion: bool,
    metrics: &mut GpuPresentationMetrics,
) -> Result<Image, GpuPresentationError> {
    let mut surface = compatible_surface(canvas, width, height)?;
    let motion = include_motion
        .then(|| session.prototype_motion_evaluation())
        .flatten();
    let playback = &session.playback;
    let inputs = RenderInputs {
        components: &playback.doc.components,
        variables: &playback.variables,
        active_modes: &playback.active_modes,
        mode_generation: playback.mode_generation,
        motion: motion.as_ref(),
        playback: None,
        video_fill_frames: None,
        dark_ui: false,
    };
    let rendered = session.renderer.render_to_canvas(
        surface.canvas(),
        width,
        height,
        scene,
        &viewport,
        Some(root),
        &inputs,
    );
    collect_metrics(metrics, rendered);
    Ok(surface.image_snapshot())
}

#[allow(clippy::too_many_arguments)]
fn render_frame(
    session: &mut PresentationTarget<'_>,
    canvas: &Canvas,
    width: u32,
    height: u32,
    root: NodeId,
    viewport: Viewport,
    layer: Option<u64>,
    include_motion: bool,
    hidden: &[NodeId],
    metrics: &mut GpuPresentationMetrics,
) -> Result<Image, GpuPresentationError> {
    let resolved = session.resolved_layer_with_identity(
        root,
        layer.unwrap_or_else(|| session.layer_identity(root)),
        include_motion,
    );
    let hidden_scene = if hidden.is_empty() {
        None
    } else {
        let mut scene = resolved.scene.clone();
        for hidden in hidden {
            if descends_from(&scene, *hidden, root)
                && let Some(node) = scene.get_mut(*hidden)
            {
                node.flags.insert(NodeFlags::HIDDEN);
            }
        }
        Some(scene)
    };
    let mut surface = compatible_surface(canvas, width, height)?;
    let playback = &session.playback;
    let inputs = RenderInputs {
        components: &playback.doc.components,
        variables: &playback.variables,
        active_modes: &playback.active_modes,
        mode_generation: playback.mode_generation,
        motion: None,
        playback: None,
        video_fill_frames: None,
        dark_ui: false,
    };
    let rendered = session.renderer.render_to_canvas(
        surface.canvas(),
        width,
        height,
        hidden_scene.as_ref().unwrap_or(&resolved.scene),
        &viewport,
        Some(root),
        &inputs,
    );
    collect_metrics(metrics, rendered);
    Ok(surface.image_snapshot())
}

fn render_backdrop(
    session: &mut PresentationTarget<'_>,
    canvas: &Canvas,
    width: u32,
    height: u32,
    hidden: &[NodeId],
    metrics: &mut GpuPresentationMetrics,
) -> Result<(), GpuPresentationError> {
    let image = render_frame(
        session,
        canvas,
        width,
        height,
        session.current,
        session.viewport,
        Some(session.current_layer),
        true,
        hidden,
        metrics,
    )?;
    draw_layer(canvas, &image, (0.0, 0.0), 1.0, BlendMode::Src);
    for entry in session.overlays.clone() {
        if entry.settings.background_dim {
            draw_scrim(canvas, SCRIM_DIM);
        }
        let image = render_frame(
            session,
            canvas,
            width,
            height,
            entry.root,
            entry.viewport,
            Some(entry.layer),
            true,
            hidden,
            metrics,
        )?;
        draw_layer(canvas, &image, (0.0, 0.0), 1.0, BlendMode::SrcOver);
    }
    Ok(())
}

fn render_base(
    session: &mut PresentationTarget<'_>,
    canvas: &Canvas,
    width: u32,
    height: u32,
    metrics: &mut GpuPresentationMetrics,
) -> Result<(), GpuPresentationError> {
    let Some(active) = session.active_transition else {
        let image = render_frame(
            session,
            canvas,
            width,
            height,
            session.current,
            session.viewport,
            Some(session.current_layer),
            true,
            &[],
            metrics,
        )?;
        draw_layer(canvas, &image, (0.0, 0.0), 1.0, BlendMode::Src);
        return Ok(());
    };
    let eased = ease(active.easing, active.progress());
    if matches!(active.style, TransitionStyle::SmartAnimate) {
        let smart_scene = session.smart_animate.as_mut().map(|smart| {
            smart.apply(eased);
            (smart.scene.clone(), smart.root)
        });
        if let Some((mut scene, root)) = smart_scene {
            PlaybackSession::apply_scroll_offsets_to_scene(&session.scroll_offsets, &mut scene);
            PlaybackSession::apply_variant_overrides_to_scene(
                &session.variant_overrides,
                &session.instance_variant_overrides,
                &session.doc.components,
                &mut scene,
                root,
            );
            let image = render_scene(
                session,
                canvas,
                width,
                height,
                &scene,
                root,
                active.to_viewport,
                true,
                metrics,
            )?;
            draw_layer(canvas, &image, (0.0, 0.0), 1.0, BlendMode::Src);
            return Ok(());
        }
    }
    let outgoing = render_frame(
        session,
        canvas,
        width,
        height,
        active.from_frame,
        active.from_viewport,
        None,
        false,
        &[],
        metrics,
    )?;
    let incoming = render_frame(
        session,
        canvas,
        width,
        height,
        active.to_frame,
        active.to_viewport,
        Some(session.current_layer),
        true,
        &[],
        metrics,
    )?;
    draw_transition(
        canvas,
        active.style,
        eased,
        width,
        height,
        &outgoing,
        &incoming,
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presentation_uses_external_canvas_without_whole_frame_readback()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut document = Doc::new();
        let frame = fanta_doc::CanvasNode::new(NodeData::Group(fanta_doc::GroupNode {
            clip_size: Some([16.0, 16.0]),
            background: Some(fanta_doc::Fill::solid(fanta_doc::Color::rgb(20, 40, 80))),
            ..Default::default()
        }));
        let root = frame.id;
        document.scene.insert(frame)?;
        let mut session = PresentSession::new(&document, Some(root), DVec2::new(16.0, 16.0))?;
        let expected = session.present_rgba();
        let mut surface = skia_safe::surfaces::raster_n32_premul((16, 16))
            .ok_or("test surface allocation failed")?;
        let metrics = session.render_to_canvas(surface.canvas(), 16, 16, 1.0)?;
        assert_eq!(metrics.whole_frame_readbacks, 0);
        assert_eq!(metrics.peak_surface_bytes, 16 * 16 * 4 * 3);
        assert_eq!(rgba(&mut surface)?, expected);
        let mut small_budget = GpuPresenter::with_budget(16 * 16 * 4 * 2);
        assert!(matches!(
            small_budget.render(&mut session, surface.canvas(), 16, 16, 1.0),
            Err(GpuPresentationError::SurfaceBudget { .. })
        ));
        assert_eq!(rgba(&mut surface)?, expected);
        let mut playback = PlaybackSession::new(&document, Some(root), DVec2::new(16.0, 16.0))?;
        let mut renderer = RasterRenderer::new(16, 16)?;
        let metrics = GpuPresenter::default().render_playback(
            &mut playback,
            &mut renderer,
            surface.canvas(),
            16,
            16,
            1.0,
        )?;
        assert_eq!(metrics.whole_frame_readbacks, 0);
        assert_eq!(metrics.rendered_layers, 1);
        assert_eq!(rgba(&mut surface)?, expected);
        Ok(())
    }

    fn rgba(surface: &mut Surface) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        let info = ImageInfo::new(
            (surface.width(), surface.height()),
            ColorType::RGBA8888,
            AlphaType::Unpremul,
            None,
        );
        let mut pixels = vec![0; info.min_row_bytes() * surface.height() as usize];
        if !surface.read_pixels(&info, &mut pixels, info.min_row_bytes(), (0, 0)) {
            return Err("test surface readback failed".into());
        }
        Ok(pixels)
    }

    #[test]
    fn failed_artwork_and_effects_preserve_the_previous_presentation()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut document = Doc::new();
        let bitmap = fanta_doc::CanvasNode::new(NodeData::Bitmap(fanta_doc::BitmapNode {
            asset: fanta_doc::AssetId::new(),
            natural_size: [16, 16],
            local_size: [16.0, 16.0],
            crop: None,
            fit: fanta_doc::ImageFitMode::Fill,
            tint: None,
        }));
        let root = bitmap.id;
        document.scene.insert(bitmap)?;
        let mut playback = PlaybackSession::new(&document, Some(root), DVec2::new(16.0, 16.0))?;
        let mut renderer = RasterRenderer::new(16, 16)?;
        let mut destination = skia_safe::surfaces::raster_n32_premul((16, 16))
            .ok_or("test surface allocation failed")?;
        destination.canvas().clear(skia_safe::Color::RED);
        let previous = rgba(&mut destination)?;
        assert!(matches!(
            GpuPresenter::default().render_playback(
                &mut playback,
                &mut renderer,
                destination.canvas(),
                16,
                16,
                1.0,
            ),
            Err(GpuPresentationError::IncompleteArtwork)
        ));
        assert_eq!(rgba(&mut destination)?, previous);
        assert_eq!(renderer.whole_frame_readback_count(), 0);
        let mut staged = compatible_surface(destination.canvas(), 16, 16)?;
        staged.canvas().clear(skia_safe::Color::BLUE);
        let mut metrics = GpuPresentationMetrics::default();
        metrics.render.effect_failed = true;
        assert!(matches!(
            commit_presentation(destination.canvas(), &mut staged, &metrics),
            Err(GpuPresentationError::EffectFailed)
        ));
        assert_eq!(rgba(&mut destination)?, previous);
        Ok(())
    }

    fn solid(width: i32, color: skia_safe::Color4f) -> Result<Surface, Box<dyn std::error::Error>> {
        let mut surface = skia_safe::surfaces::raster_n32_premul((width, 1))
            .ok_or("test surface allocation failed")?;
        surface.canvas().clear(color);
        Ok(surface)
    }

    #[test]
    fn canvas_transition_matches_cpu_composition_contracts()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut outgoing = solid(4, skia_safe::Color4f::new(1.0, 0.0, 0.0, 0.5))?;
        let mut incoming = solid(4, skia_safe::Color4f::new(0.0, 0.0, 1.0, 0.25))?;
        let outgoing_pixels = rgba(&mut outgoing)?;
        let incoming_pixels = rgba(&mut incoming)?;
        let outgoing_image = outgoing.image_snapshot();
        let incoming_image = incoming.image_snapshot();
        let mut result = solid(4, skia_safe::Color4f::new(0.0, 0.0, 0.0, 0.0))?;
        for style in [
            TransitionStyle::Instant,
            TransitionStyle::Dissolve,
            TransitionStyle::SmartAnimate,
            TransitionStyle::ScrollAnimate,
            TransitionStyle::MoveIn {
                direction: fanta_doc::Direction::Left,
            },
            TransitionStyle::Push {
                direction: fanta_doc::Direction::Up,
            },
            TransitionStyle::SlideIn {
                direction: fanta_doc::Direction::Right,
            },
            TransitionStyle::SlideOut {
                direction: fanta_doc::Direction::Left,
            },
            TransitionStyle::MoveOut {
                direction: fanta_doc::Direction::Down,
            },
        ] {
            for progress in [0.0, 0.25, 0.5, 1.0, 1.1] {
                draw_transition(
                    result.canvas(),
                    style,
                    progress,
                    4,
                    1,
                    &outgoing_image,
                    &incoming_image,
                );
                let expected = composite(style, progress, 4, 1, &outgoing_pixels, &incoming_pixels);
                let actual = rgba(&mut result)?;
                for (actual, expected) in actual.iter().zip(expected) {
                    assert!(
                        actual.abs_diff(expected) <= 2,
                        "{style:?} at {progress}: {actual} versus {expected}"
                    );
                }
            }
        }
        Ok(())
    }

    #[test]
    fn canvas_scrim_keeps_translucent_alpha() -> Result<(), Box<dyn std::error::Error>> {
        let mut surface = solid(1, skia_safe::Color4f::new(0.8, 0.4, 0.2, 0.5))?;
        let mut expected = rgba(&mut surface)?;
        apply_scrim(&mut expected, 0.4);
        draw_scrim(surface.canvas(), 0.4);
        let actual = rgba(&mut surface)?;
        assert_eq!(actual.get(3), expected.get(3));
        for (actual, expected) in actual.iter().zip(expected) {
            assert!(actual.abs_diff(expected) <= 2);
        }
        Ok(())
    }

    #[test]
    fn canvas_overlay_uses_reference_offsets_and_opacity() -> Result<(), Box<dyn std::error::Error>>
    {
        let mut backdrop = solid(4, skia_safe::Color4f::new(1.0, 0.0, 0.0, 1.0))?;
        let mut overlay = solid(4, skia_safe::Color4f::new(0.0, 1.0, 0.0, 1.0))?;
        let base_pixels = rgba(&mut backdrop)?;
        let top_pixels = rgba(&mut overlay)?;
        let image = overlay.image_snapshot();
        for style in [
            TransitionStyle::Dissolve,
            TransitionStyle::SlideIn {
                direction: fanta_doc::Direction::Left,
            },
            TransitionStyle::MoveOut {
                direction: fanta_doc::Direction::Right,
            },
        ] {
            for visible in [0.0, 0.25, 0.5, 1.0] {
                backdrop.canvas().clear(skia_safe::Color::RED);
                draw_overlay(backdrop.canvas(), style, visible, 4, 1, &image);
                let expected = composite_overlay(style, visible, 4, 1, &base_pixels, &top_pixels);
                let actual = rgba(&mut backdrop)?;
                for (actual, expected) in actual.iter().zip(expected) {
                    assert!(actual.abs_diff(expected) <= 1);
                }
            }
        }
        Ok(())
    }
}
