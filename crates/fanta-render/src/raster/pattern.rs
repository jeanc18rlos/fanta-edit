use super::{
    Bounds, ComponentLibrary, NodeData, NodeId, RenderCtx, Scene, render_node, to_sk_matrix,
};
use fanta_doc::{
    BlendMode, Fill, PatternFill, PatternHorizontalAlignment, PatternTileType, Transform2D,
};
use skia_safe::{
    Canvas, FilterMode, Matrix, Paint, Path, Picture, PictureRecorder, Rect, SamplingOptions,
    TileMode, canvas::SrcRectConstraint, surfaces,
};
use std::collections::HashMap;
use std::hash::{Hash, Hasher};

const MAX_SOURCE_DIMENSION: i32 = 1024;
const MAX_CACHED_SOURCES: usize = 16;
const MAX_CACHED_MOTIFS: usize = 64;
const MAX_PATTERN_DEPTH: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct SourceKey {
    id: NodeId,
    width: i32,
    height: i32,
}

struct CachedSource {
    image: skia_safe::Image,
    stamp: u64,
    scene_instance: u64,
    scene_revision: u64,
    mode_generation: u64,
    dark_ui: bool,
    external_dependencies: bool,
    component_stamp: u64,
    token: u64,
    last_used: u64,
}

#[derive(Clone)]
struct PatternSource {
    image: skia_safe::Image,
    token: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct MotifKey {
    source: SourceKey,
    token: u64,
    tile_type: PatternTileType,
    spacing_x: u32,
    spacing_y: u32,
}

struct CachedMotif {
    picture: Picture,
    last_used: u64,
}

#[derive(Default)]
pub(crate) struct PatternCache {
    sources: HashMap<SourceKey, CachedSource>,
    motifs: HashMap<MotifKey, CachedMotif>,
    next_token: u64,
    tick: u64,
}

impl PatternCache {
    pub(crate) fn clear(&mut self) {
        self.sources.clear();
        self.motifs.clear();
    }

    fn next_tick(&mut self) -> u64 {
        self.tick = self.tick.wrapping_add(1);
        self.tick
    }

    fn get_source(
        &mut self,
        key: SourceKey,
        scene: &Scene,
        mode_generation: u64,
        dark_ui: bool,
        components: &ComponentLibrary,
    ) -> Option<PatternSource> {
        let tick = self.next_tick();
        let cached = self.sources.get_mut(&key)?;
        if cached.mode_generation != mode_generation || cached.dark_ui != dark_ui {
            return None;
        }
        if cached.external_dependencies && cached.component_stamp != component_stamp(components) {
            return None;
        }
        if cached.scene_instance == scene.instance_id() && cached.scene_revision == scene.revision()
        {
            cached.last_used = tick;
            return Some(PatternSource {
                image: cached.image.clone(),
                token: cached.token,
            });
        }
        let current_stamp = scene.subtree_stamp(key.id);
        if current_stamp != cached.stamp || cached.external_dependencies {
            return None;
        }
        cached.scene_instance = scene.instance_id();
        cached.scene_revision = scene.revision();
        cached.last_used = tick;
        Some(PatternSource {
            image: cached.image.clone(),
            token: cached.token,
        })
    }

    fn put_source(
        &mut self,
        key: SourceKey,
        scene: &Scene,
        mode_generation: u64,
        dark_ui: bool,
        components: &ComponentLibrary,
        image: skia_safe::Image,
    ) -> PatternSource {
        if self.sources.len() >= MAX_CACHED_SOURCES
            && !self.sources.contains_key(&key)
            && let Some(oldest) = self
                .sources
                .iter()
                .min_by_key(|(_, entry)| entry.last_used)
                .map(|(key, _)| *key)
        {
            self.sources.remove(&oldest);
            self.motifs.retain(|motif, _| motif.source != oldest);
        }
        self.next_token = self.next_token.wrapping_add(1);
        let token = self.next_token;
        let last_used = self.next_tick();
        self.motifs.retain(|motif, _| motif.source.id != key.id);
        self.sources.insert(
            key,
            CachedSource {
                image: image.clone(),
                stamp: scene.subtree_stamp(key.id),
                scene_instance: scene.instance_id(),
                scene_revision: scene.revision(),
                mode_generation,
                dark_ui,
                external_dependencies: source_has_external_dependency(scene, key.id),
                component_stamp: component_stamp(components),
                token,
                last_used,
            },
        );
        PatternSource { image, token }
    }

    fn motif(&mut self, key: MotifKey, image: &skia_safe::Image) -> Option<Picture> {
        let last_used = self.next_tick();
        if let Some(motif) = self.motifs.get_mut(&key) {
            motif.last_used = last_used;
            return Some(motif.picture.clone());
        }
        let picture = make_motif(image, key.tile_type, key.spacing_x, key.spacing_y)?;
        if self.motifs.len() >= MAX_CACHED_MOTIFS
            && let Some(oldest) = self
                .motifs
                .iter()
                .min_by_key(|(_, entry)| entry.last_used)
                .map(|(key, _)| *key)
        {
            self.motifs.remove(&oldest);
        }
        self.motifs.insert(
            key,
            CachedMotif {
                picture: picture.clone(),
                last_used,
            },
        );
        Some(picture)
    }
}

fn fill_has_pattern(fill: &Fill) -> bool {
    matches!(fill, Fill::Pattern { .. })
}

fn component_stamp(components: &ComponentLibrary) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for (id, definition) in &components.defs {
        id.hash(&mut hasher);
        definition.rev.hash(&mut hasher);
        definition.preview_rev.hash(&mut hasher);
    }
    hasher.finish()
}

fn source_has_external_dependency(scene: &Scene, source: NodeId) -> bool {
    scene
        .descendants_of(source)
        .filter_map(|id| scene.get(id))
        .any(|node| match &node.data {
            NodeData::Group(group) => {
                group.background.as_ref().is_some_and(fill_has_pattern)
                    || group.background_fills.iter().any(fill_has_pattern)
                    || group
                        .strokes
                        .iter()
                        .any(|stroke| fill_has_pattern(&stroke.paint))
            }
            NodeData::Vector(vector) => {
                vector.fills.iter().any(fill_has_pattern)
                    || vector
                        .strokes
                        .iter()
                        .any(|stroke| fill_has_pattern(&stroke.paint))
            }
            NodeData::Boolean(boolean) => {
                boolean.fills.iter().any(fill_has_pattern)
                    || boolean
                        .strokes
                        .iter()
                        .any(|stroke| fill_has_pattern(&stroke.paint))
            }
            NodeData::Instance(_) => true,
            _ => false,
        })
}

fn inverse_transform(transform: Transform2D) -> Option<Transform2D> {
    let [a, b, c, d, tx, ty] = transform.to_components();
    let det = a * d - b * c;
    if !det.is_finite() || det.abs() < 1e-12 {
        return None;
    }
    let inverse = Transform2D::from_components([
        d / det,
        -b / det,
        -c / det,
        a / det,
        (c * ty - d * tx) / det,
        (b * tx - a * ty) / det,
    ]);
    inverse.is_finite().then_some(inverse)
}

fn source_pixel_size(bounds: Bounds, effective_scale: f32, pattern_scale: f32) -> Option<[i32; 2]> {
    let width = bounds.width();
    let height = bounds.height();
    if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
        return None;
    }
    let requested = f64::from(effective_scale.max(1.0)) * f64::from(pattern_scale.max(0.01));
    let scale = requested
        .min(4.0)
        .min(f64::from(MAX_SOURCE_DIMENSION) / width)
        .min(f64::from(MAX_SOURCE_DIMENSION) / height);
    let pixel_width = (width * scale)
        .ceil()
        .clamp(1.0, f64::from(MAX_SOURCE_DIMENSION)) as i32;
    let pixel_height = (height * scale)
        .ceil()
        .clamp(1.0, f64::from(MAX_SOURCE_DIMENSION)) as i32;
    Some([pixel_width, pixel_height])
}

fn render_source(
    pattern: &PatternFill,
    ctx: &mut RenderCtx,
) -> Option<(SourceKey, PatternSource, Bounds)> {
    let id = pattern.source_node_id;
    if ctx.pattern_stack.len() >= MAX_PATTERN_DEPTH || ctx.pattern_stack.contains(&id) {
        return None;
    }
    let source_node = ctx.scene.get(id)?;
    let bounds = ctx.scene.local_bounds(id)?;
    let [width, height] = source_pixel_size(bounds, ctx.effective_scale, pattern.scaling_factor)?;
    let key = SourceKey { id, width, height };
    let mode_generation = ctx.inputs.mode_generation;
    let dark_ui = ctx.inputs.dark_ui;
    if ctx.inputs.motion.is_none()
        && let Some(source) = ctx.pattern_cache.get_source(
            key,
            ctx.scene,
            mode_generation,
            dark_ui,
            ctx.inputs.components,
        )
    {
        return Some((key, source, bounds));
    }

    let inverse = inverse_transform(source_node.transform)?;
    let mut surface = surfaces::raster_n32_premul((width, height))?;
    let canvas = surface.canvas();
    canvas.clear(skia_safe::Color::TRANSPARENT);
    canvas.save();
    canvas.scale((
        width as f32 / bounds.width() as f32,
        height as f32 / bounds.height() as f32,
    ));
    canvas.translate((-bounds.min_x as f32, -bounds.min_y as f32));
    canvas.concat(&to_sk_matrix(&inverse));

    let old_visible = std::mem::replace(
        &mut ctx.visible,
        Bounds {
            min_x: -1e18,
            min_y: -1e18,
            max_x: 1e18,
            max_y: 1e18,
        },
    );
    let old_scale = std::mem::replace(
        &mut ctx.effective_scale,
        width as f32 / bounds.width() as f32,
    );
    let old_background = ctx.page_background_root.take();
    let old_layers = std::mem::replace(&mut ctx.supports_offscreen_layers, true);
    let old_lookup = std::mem::replace(&mut ctx.layer_cache_lookups, false);
    let old_populate = std::mem::replace(&mut ctx.layer_cache_populate, false);
    let old_alpha = std::mem::replace(&mut ctx.paint_alpha, 1.0);
    let old_volatile = std::mem::replace(&mut ctx.layer_volatile, false);
    ctx.pattern_stack.push(id);
    render_node(canvas, id, ctx);
    ctx.pattern_stack.pop();
    let volatile = ctx.layer_volatile;
    ctx.layer_volatile = old_volatile || volatile;
    ctx.paint_alpha = old_alpha;
    ctx.layer_cache_populate = old_populate;
    ctx.layer_cache_lookups = old_lookup;
    ctx.page_background_root = old_background;
    ctx.supports_offscreen_layers = old_layers;
    ctx.effective_scale = old_scale;
    ctx.visible = old_visible;
    canvas.restore();
    let image = surface.image_snapshot();
    let source = if !volatile && ctx.inputs.motion.is_none() {
        ctx.pattern_cache.put_source(
            key,
            ctx.scene,
            mode_generation,
            dark_ui,
            ctx.inputs.components,
            image,
        )
    } else {
        PatternSource { image, token: 0 }
    };
    Some((key, source, bounds))
}

fn hex_clip_path(width: f32, height: f32, tile_type: PatternTileType) -> Option<Path> {
    let points: &[(f32, f32)] = match tile_type {
        PatternTileType::Rectangular => return None,
        PatternTileType::HorizontalHexagonal => &[
            (width * 0.5, 0.0),
            (width, height * 0.25),
            (width, height * 0.75),
            (width * 0.5, height),
            (0.0, height * 0.75),
            (0.0, height * 0.25),
        ],
        PatternTileType::VerticalHexagonal => &[
            (width * 0.25, 0.0),
            (width * 0.75, 0.0),
            (width, height * 0.5),
            (width * 0.75, height),
            (width * 0.25, height),
            (0.0, height * 0.5),
        ],
    };
    let mut path = Path::new();
    if let Some(&(x, y)) = points.first() {
        path.move_to((x, y));
        for &(x, y) in &points[1..] {
            path.line_to((x, y));
        }
        path.close();
    }
    Some(path)
}

fn draw_motif_tile(
    canvas: &Canvas,
    image: &skia_safe::Image,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    clip_path: Option<&Path>,
) {
    canvas.save();
    canvas.translate((x, y));
    if let Some(path) = clip_path {
        canvas.clip_path(path, None, true);
    }
    let source = Rect::from_wh(width, height);
    let destination = Rect::from_wh(width, height);
    canvas.draw_image_rect_with_sampling_options(
        image,
        Some((&source, SrcRectConstraint::Strict)),
        destination,
        SamplingOptions::from(FilterMode::Linear),
        &Paint::default(),
    );
    canvas.restore();
}

fn make_motif(
    image: &skia_safe::Image,
    tile_type: PatternTileType,
    spacing_x_bits: u32,
    spacing_y_bits: u32,
) -> Option<Picture> {
    let width = image.width() as f32;
    let height = image.height() as f32;
    let spacing_x = f32::from_bits(spacing_x_bits).clamp(0.0, 1000.0);
    let spacing_y = f32::from_bits(spacing_y_bits).clamp(0.0, 1000.0);
    let step_x = width
        * (if tile_type == PatternTileType::VerticalHexagonal {
            0.75
        } else {
            1.0
        } + spacing_x);
    let step_y = height
        * (if tile_type == PatternTileType::HorizontalHexagonal {
            0.75
        } else {
            1.0
        } + spacing_y);
    let motif_width = step_x
        * if tile_type == PatternTileType::VerticalHexagonal {
            2.0
        } else {
            1.0
        };
    let motif_height = step_y
        * if tile_type == PatternTileType::HorizontalHexagonal {
            2.0
        } else {
            1.0
        };
    if !motif_width.is_finite()
        || !motif_height.is_finite()
        || motif_width <= 0.0
        || motif_height <= 0.0
    {
        return None;
    }
    let motif_bounds = Rect::from_wh(motif_width, motif_height);
    let mut recorder = PictureRecorder::new();
    let canvas = recorder.begin_recording(motif_bounds, None);
    let hex = hex_clip_path(width, height, tile_type);
    match tile_type {
        PatternTileType::Rectangular => {
            draw_motif_tile(canvas, image, 0.0, 0.0, width, height, None);
        }
        PatternTileType::HorizontalHexagonal => {
            for row in -2i32..=3 {
                let offset_x = if row.rem_euclid(2) == 0 {
                    0.0
                } else {
                    step_x * 0.5
                };
                for column in -2..=2 {
                    draw_motif_tile(
                        canvas,
                        image,
                        column as f32 * step_x + offset_x,
                        row as f32 * step_y,
                        width,
                        height,
                        hex.as_ref(),
                    );
                }
            }
        }
        PatternTileType::VerticalHexagonal => {
            for column in -2i32..=3 {
                let offset_y = if column.rem_euclid(2) == 0 {
                    0.0
                } else {
                    step_y * 0.5
                };
                for row in -2..=2 {
                    draw_motif_tile(
                        canvas,
                        image,
                        column as f32 * step_x,
                        row as f32 * step_y + offset_y,
                        width,
                        height,
                        hex.as_ref(),
                    );
                }
            }
        }
    }
    recorder.finish_recording_as_picture(Some(&motif_bounds))
}

pub(crate) fn pattern_paint(
    bounds: Bounds,
    pattern: &PatternFill,
    opacity: f32,
    blend: BlendMode,
    ctx: &mut RenderCtx,
) -> Option<Paint> {
    let (source_key, source, source_bounds) = render_source(pattern, ctx)?;
    let spacing_x = pattern.spacing.x.max(0.0).min(1000.0);
    let spacing_y = pattern.spacing.y.max(0.0).min(1000.0);
    let motif_key = MotifKey {
        source: source_key,
        token: source.token,
        tile_type: pattern.tile_type,
        spacing_x: spacing_x.to_bits(),
        spacing_y: spacing_y.to_bits(),
    };
    let picture = if source.token != 0 {
        ctx.pattern_cache.motif(motif_key, &source.image)
    } else {
        make_motif(
            &source.image,
            pattern.tile_type,
            spacing_x.to_bits(),
            spacing_y.to_bits(),
        )
    };
    let picture = picture?;
    let scaling_factor = if pattern.scaling_factor.is_finite() && pattern.scaling_factor > 0.0 {
        pattern.scaling_factor
    } else {
        1.0
    };
    let tile_width = source_bounds.width() as f32 * scaling_factor;
    let tile_height = source_bounds.height() as f32 * scaling_factor;
    if !tile_width.is_finite()
        || !tile_height.is_finite()
        || tile_width <= 0.0
        || tile_height <= 0.0
    {
        return None;
    }
    let scale_x = tile_width / source.image.width() as f32;
    let scale_y = tile_height / source.image.height() as f32;
    let anchor_x = match pattern.horizontal_alignment {
        PatternHorizontalAlignment::Start => bounds.min_x as f32,
        PatternHorizontalAlignment::Center => {
            (bounds.min_x + bounds.width() * 0.5) as f32 - tile_width * 0.5
        }
        PatternHorizontalAlignment::End => bounds.max_x as f32 - tile_width,
    };
    let mut matrix = Matrix::scale((scale_x, scale_y));
    matrix.post_translate((anchor_x, bounds.min_y as f32));
    let shader = picture.to_shader(
        (TileMode::Repeat, TileMode::Repeat),
        FilterMode::Linear,
        &matrix,
        None,
    );
    let mut paint = Paint::default();
    paint.set_anti_alias(true);
    paint.set_shader(shader);
    paint.set_alpha_f((opacity * ctx.paint_alpha).clamp(0.0, 1.0));
    if !blend.is_normal() {
        paint.set_blend_mode(super::to_sk_blend_mode(blend));
    }
    Some(paint)
}

pub(crate) fn draw_pattern_fill(
    canvas: &Canvas,
    path: &Path,
    bounds: Bounds,
    pattern: &PatternFill,
    opacity: f32,
    blend: BlendMode,
    ctx: &mut RenderCtx,
) -> bool {
    let Some(paint) = pattern_paint(bounds, pattern, opacity, blend, ctx) else {
        return false;
    };
    canvas.draw_path(path, &paint);
    ctx.metrics.nodes_drawn += 1;
    true
}
