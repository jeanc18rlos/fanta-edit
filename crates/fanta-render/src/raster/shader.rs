use fanta_doc::style::{ShaderFill, ShaderPropertyValue};
use fanta_doc::{BlendMode, Bounds, Color};
use skia_safe::{
    AlphaType, Canvas, ColorType, Data, FilterMode, Image, ImageInfo, Matrix, MipmapMode, Paint,
    Path, SamplingOptions, TileMode,
};
use std::cell::RefCell;
use std::collections::HashMap;

const TILE_SIZE: usize = 256;
const CACHE_CAPACITY: usize = 64;

#[derive(Clone, Copy, Eq, Hash, PartialEq)]
enum ShaderPreset {
    FractalNoise {
        frequency_bits: u32,
        octaves_bits: u32,
        color_a: Color,
        color_b: Color,
    },
    Halftone {
        columns_bits: u32,
        rows_bits: u32,
        radius_bits: u32,
        ink_color: Color,
        paper_color: Color,
    },
}

struct CachedImage {
    image: Image,
    last_used: u64,
}

#[derive(Default)]
struct ShaderCache {
    images: HashMap<ShaderPreset, CachedImage>,
    tick: u64,
}

thread_local! {
    static SHADER_CACHE: RefCell<ShaderCache> = RefCell::new(ShaderCache::default());
}

fn number_property(shader: &ShaderFill, id: &str, default: f32) -> f32 {
    shader
        .properties
        .iter()
        .find(|property| property.definition_id == id)
        .and_then(|property| match &property.value {
            ShaderPropertyValue::Number(value) if value.is_finite() => Some(*value),
            _ => None,
        })
        .unwrap_or(default)
}

fn color_property(shader: &ShaderFill, id: &str, default: Color) -> Color {
    shader
        .properties
        .iter()
        .find(|property| property.definition_id == id)
        .and_then(|property| match &property.value {
            ShaderPropertyValue::Color(value) => Some(*value),
            _ => None,
        })
        .unwrap_or(default)
}

fn preset(shader: &ShaderFill, bounds: Bounds) -> Option<ShaderPreset> {
    match shader.shader_id.as_str() {
        "fanta:shader:fractal-noise" => Some(ShaderPreset::FractalNoise {
            frequency_bits: number_property(shader, "frequency", 4.0)
                .clamp(0.5, 32.0)
                .to_bits(),
            octaves_bits: number_property(shader, "octaves", 4.0)
                .clamp(1.0, 6.0)
                .to_bits(),
            color_a: color_property(shader, "color_a", Color::rgb(37, 31, 72)),
            color_b: color_property(shader, "color_b", Color::rgb(125, 227, 214)),
        }),
        "fanta:shader:halftone" => {
            let columns = number_property(shader, "columns", 18.0).clamp(2.0, 80.0);
            let rows = (columns as f64 * bounds.height() / bounds.width()).clamp(1.0, 256.0);
            Some(ShaderPreset::Halftone {
                columns_bits: columns.to_bits(),
                rows_bits: (rows as f32).to_bits(),
                radius_bits: number_property(shader, "radius", 1.0)
                    .clamp(0.0, 20.0)
                    .to_bits(),
                ink_color: color_property(shader, "ink_color", Color::rgb(30, 41, 59)),
                paper_color: color_property(shader, "paper_color", Color::rgb(248, 250, 252)),
            })
        }
        _ => None,
    }
}

fn hash_lattice(x: i32, y: i32) -> f32 {
    let mut hash = (x as u32)
        .wrapping_mul(0x9E37_79B9)
        .wrapping_add((y as u32).wrapping_mul(0x85EB_CA6B));
    hash ^= hash >> 16;
    hash = hash.wrapping_mul(0x7FEB_352D);
    hash ^= hash >> 15;
    hash = hash.wrapping_mul(0x846C_A68B);
    hash ^= hash >> 16;
    hash as f32 / u32::MAX as f32
}

fn value_noise(x: f32, y: f32) -> f32 {
    let left = x.floor() as i32;
    let top = y.floor() as i32;
    let horizontal = x - left as f32;
    let vertical = y - top as f32;
    let horizontal = horizontal * horizontal * (3.0 - 2.0 * horizontal);
    let vertical = vertical * vertical * (3.0 - 2.0 * vertical);
    let top_left = hash_lattice(left, top);
    let top_right = hash_lattice(left + 1, top);
    let bottom_left = hash_lattice(left, top + 1);
    let bottom_right = hash_lattice(left + 1, top + 1);
    let top = top_left + (top_right - top_left) * horizontal;
    let bottom = bottom_left + (bottom_right - bottom_left) * horizontal;
    top + (bottom - top) * vertical
}

fn fractal_noise(u: f32, v: f32, frequency: f32, octaves: f32) -> f32 {
    let mut frequency = frequency;
    let mut amplitude = 1.0;
    let mut sum = 0.0;
    let mut weight = 0.0;
    for level in 0..6 {
        let contribution = (octaves - level as f32).clamp(0.0, 1.0);
        if contribution == 0.0 {
            break;
        }
        sum += value_noise(u * frequency, v * frequency) * amplitude * contribution;
        weight += amplitude * contribution;
        amplitude *= 0.5;
        frequency *= 2.0;
        if frequency > TILE_SIZE as f32 * 0.5 {
            break;
        }
    }
    if weight > 0.0 { sum / weight } else { 0.5 }
}

fn smoothstep(edge_start: f32, edge_end: f32, value: f32) -> f32 {
    let fraction = ((value - edge_start) / (edge_end - edge_start)).clamp(0.0, 1.0);
    fraction * fraction * (3.0 - 2.0 * fraction)
}

fn halftone_coverage(u: f32, v: f32, columns: f32, rows: f32, radius: f32) -> f32 {
    if radius == 0.0 {
        return 0.0;
    }
    let x = (u * columns).fract() - 0.5;
    let y = (v * rows).fract() - 0.5;
    let distance = x.hypot(y);
    let edge = 0.25 * (columns.max(rows) / TILE_SIZE as f32).min(1.0);
    let radius = 0.5 * radius / (1.0 + radius);
    1.0 - smoothstep(radius - edge, radius + edge, distance)
}

fn mix_color(a: Color, b: Color, amount: f32) -> [u8; 4] {
    let mix = |first: u8, second: u8| {
        (first as f32 + (second as f32 - first as f32) * amount)
            .round()
            .clamp(0.0, 255.0) as u8
    };
    [mix(a.r, b.r), mix(a.g, b.g), mix(a.b, b.b), mix(a.a, b.a)]
}

fn bake_pixels(preset: ShaderPreset) -> Vec<u8> {
    let mut pixels = vec![0; TILE_SIZE * TILE_SIZE * 4];
    for row in 0..TILE_SIZE {
        let v = (row as f32 + 0.5) / TILE_SIZE as f32;
        for column in 0..TILE_SIZE {
            let u = (column as f32 + 0.5) / TILE_SIZE as f32;
            let color = match preset {
                ShaderPreset::FractalNoise {
                    frequency_bits,
                    octaves_bits,
                    color_a,
                    color_b,
                } => mix_color(
                    color_a,
                    color_b,
                    fractal_noise(
                        u,
                        v,
                        f32::from_bits(frequency_bits),
                        f32::from_bits(octaves_bits),
                    ),
                ),
                ShaderPreset::Halftone {
                    columns_bits,
                    rows_bits,
                    radius_bits,
                    ink_color,
                    paper_color,
                } => mix_color(
                    paper_color,
                    ink_color,
                    halftone_coverage(
                        u,
                        v,
                        f32::from_bits(columns_bits),
                        f32::from_bits(rows_bits),
                        f32::from_bits(radius_bits),
                    ),
                ),
            };
            let offset = (row * TILE_SIZE + column) * 4;
            pixels[offset..offset + 4].copy_from_slice(&color);
        }
    }
    pixels
}

fn bake_image(preset: ShaderPreset) -> Option<Image> {
    let pixels = bake_pixels(preset);
    let info = ImageInfo::new(
        (TILE_SIZE as i32, TILE_SIZE as i32),
        ColorType::RGBA8888,
        AlphaType::Unpremul,
        None,
    );
    skia_safe::images::raster_from_data(&info, Data::new_copy(&pixels), info.min_row_bytes())
}

fn cached_image(preset: ShaderPreset) -> Option<Image> {
    let hit = SHADER_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        cache.tick = cache.tick.wrapping_add(1);
        let tick = cache.tick;
        cache.images.get_mut(&preset).map(|entry| {
            entry.last_used = tick;
            entry.image.clone()
        })
    });
    if hit.is_some() {
        return hit;
    }

    let image = bake_image(preset)?;
    SHADER_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        cache.tick = cache.tick.wrapping_add(1);
        if cache.images.len() >= CACHE_CAPACITY
            && let Some(oldest) = cache
                .images
                .iter()
                .min_by_key(|(_, entry)| entry.last_used)
                .map(|(key, _)| *key)
        {
            cache.images.remove(&oldest);
        }
        let tick = cache.tick;
        cache.images.insert(
            preset,
            CachedImage {
                image: image.clone(),
                last_used: tick,
            },
        );
    });
    Some(image)
}

pub(crate) fn shader_paint(
    bounds: Bounds,
    shader: &ShaderFill,
    opacity: f32,
    blend: BlendMode,
    paint_alpha: f32,
) -> Option<Paint> {
    let [x, y, width, height] = [
        bounds.min_x as f32,
        bounds.min_y as f32,
        bounds.width() as f32,
        bounds.height() as f32,
    ];
    if ![x, y, width, height, opacity, paint_alpha]
        .iter()
        .all(|value| value.is_finite())
        || width <= 0.0
        || height <= 0.0
    {
        return None;
    }
    let preset = preset(shader, bounds)?;
    let image = cached_image(preset)?;
    let matrix = Matrix::new_all(
        width / TILE_SIZE as f32,
        0.0,
        x,
        0.0,
        height / TILE_SIZE as f32,
        y,
        0.0,
        0.0,
        1.0,
    );
    let sampling = SamplingOptions::new(FilterMode::Linear, MipmapMode::None);
    let skia_shader = image.to_shader((TileMode::Clamp, TileMode::Clamp), sampling, &matrix)?;
    let mut paint = Paint::default();
    paint.set_anti_alias(true);
    paint.set_shader(skia_shader);
    paint.set_alpha_f((opacity * paint_alpha).clamp(0.0, 1.0));
    if !blend.is_normal() {
        paint.set_blend_mode(crate::paint::to_sk_blend_mode(blend));
    }
    Some(paint)
}

pub(crate) fn draw_shader_fill(
    canvas: &Canvas,
    path: &Path,
    bounds: Bounds,
    shader: &ShaderFill,
    opacity: f32,
    blend: BlendMode,
    paint_alpha: f32,
) -> bool {
    let Some(paint) = shader_paint(bounds, shader, opacity, blend, paint_alpha) else {
        return false;
    };
    canvas.draw_path(path, &paint);
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use fanta_doc::style::ShaderPropertyAssignment;
    use fanta_doc::{CanvasNode, Doc, Fill, NodeData, Operation, VectorNode};

    fn shader(id: &str, properties: &[(&str, ShaderPropertyValue)]) -> ShaderFill {
        ShaderFill {
            shader_id: id.to_owned(),
            name: id.to_owned(),
            properties: properties
                .iter()
                .map(|(definition_id, value)| ShaderPropertyAssignment {
                    definition_id: (*definition_id).to_owned(),
                    value: value.clone(),
                })
                .collect(),
        }
    }

    #[test]
    fn fractal_noise_is_deterministic_and_color_properties_change_pixels() {
        let initial = shader("fanta:shader:fractal-noise", &[]);
        let initial_preset =
            preset(&initial, Bounds::from_xywh(0.0, 0.0, 100.0, 100.0)).expect("bundled shader");
        let first = bake_pixels(initial_preset);
        let second = bake_pixels(initial_preset);
        assert_eq!(first, second);
        assert!(first.chunks_exact(4).any(|pixel| pixel[0] != first[0]));

        let recolored = shader(
            "fanta:shader:fractal-noise",
            &[("color_b", ShaderPropertyValue::Color(Color::rgb(255, 0, 0)))],
        );
        let recolored = bake_pixels(
            preset(&recolored, Bounds::from_xywh(0.0, 0.0, 100.0, 100.0)).expect("bundled shader"),
        );
        assert_ne!(first, recolored);
    }

    #[test]
    fn halftone_keeps_dots_round_on_non_square_bounds() {
        let shader = shader("fanta:shader:halftone", &[]);
        let wide =
            preset(&shader, Bounds::from_xywh(0.0, 0.0, 200.0, 100.0)).expect("bundled shader");
        let tall =
            preset(&shader, Bounds::from_xywh(0.0, 0.0, 100.0, 200.0)).expect("bundled shader");
        assert!(matches!(
            wide,
            ShaderPreset::Halftone {
                columns_bits,
                rows_bits,
                ..
            } if f32::from_bits(columns_bits) == 18.0 && f32::from_bits(rows_bits) == 9.0
        ));
        assert!(matches!(
            tall,
            ShaderPreset::Halftone {
                columns_bits,
                rows_bits,
                ..
            } if f32::from_bits(columns_bits) == 18.0 && f32::from_bits(rows_bits) == 36.0
        ));
        assert!(halftone_coverage(0.5 / 18.0, 0.5 / 9.0, 18.0, 9.0, 1.0) > 0.95);
        assert!(halftone_coverage(0.0, 0.0, 18.0, 9.0, 1.0) < 0.05);
    }

    #[test]
    fn invalid_inputs_do_not_panic_or_allocate_unbounded_images() {
        let shader = shader(
            "fanta:shader:halftone",
            &[
                ("columns", ShaderPropertyValue::Number(f32::INFINITY)),
                ("radius", ShaderPropertyValue::Number(f32::NAN)),
            ],
        );
        let bounded = preset(&shader, Bounds::from_xywh(0.0, 0.0, 100.0, 100.0));
        assert!(bounded.is_some());
        assert!(
            shader_paint(
                Bounds::from_xywh(0.0, 0.0, 0.0, 100.0),
                &shader,
                1.0,
                BlendMode::Normal,
                1.0,
            )
            .is_none()
        );
        assert!(
            shader_paint(
                Bounds::from_xywh(0.0, 0.0, 100.0, 100.0),
                &shader,
                1.0,
                BlendMode::Normal,
                1.0,
            )
            .is_some()
        );
    }

    #[test]
    fn every_numeric_property_changes_the_baked_pixels_at_one_ui_step() {
        let bounds = Bounds::from_xywh(0.0, 0.0, 100.0, 100.0);
        let base_noise = shader("fanta:shader:fractal-noise", &[]);
        let base_noise = bake_pixels(preset(&base_noise, bounds).expect("noise shader"));
        for (property, value) in [("frequency", 4.25), ("octaves", 4.25)] {
            let edited = shader(
                "fanta:shader:fractal-noise",
                &[(property, ShaderPropertyValue::Number(value))],
            );
            assert_ne!(
                base_noise,
                bake_pixels(preset(&edited, bounds).expect("edited noise shader")),
                "{property} must affect the first +0.25 click"
            );
        }

        let base_halftone = shader("fanta:shader:halftone", &[]);
        let base_halftone = bake_pixels(preset(&base_halftone, bounds).expect("halftone shader"));
        for (property, value) in [("columns", 18.25), ("radius", 1.25)] {
            let edited = shader(
                "fanta:shader:halftone",
                &[(property, ShaderPropertyValue::Number(value))],
            );
            assert_ne!(
                base_halftone,
                bake_pixels(preset(&edited, bounds).expect("edited halftone shader")),
                "{property} must affect the first +0.25 click"
            );
        }
    }

    #[test]
    fn shader_fill_renders_on_a_vector_node() {
        let mut vector = VectorNode::rect_solid(-20.0, -20.0, 40.0, 40.0, Color::BLACK);
        vector.fills.clear();
        vector.fills.push(Fill::Shader {
            shader: Box::new(shader("fanta:shader:fractal-noise", &[])),
            opacity: 1.0,
            blend: BlendMode::Normal,
        });
        let mut doc = Doc::new();
        doc.apply(Operation::create_node(CanvasNode::new(NodeData::Vector(
            vector,
        ))))
        .expect("create shader filled vector");
        let mut renderer = super::super::RasterRenderer::new(64, 64).expect("raster surface");
        renderer.render(&doc.scene, &doc.viewport);
        let pixels = renderer.copy_rgba();
        let mut seen = std::collections::BTreeSet::new();
        for row in 20..44 {
            for column in 20..44 {
                let index = (row * 64 + column) * 4;
                seen.insert([
                    pixels[index],
                    pixels[index + 1],
                    pixels[index + 2],
                    pixels[index + 3],
                ]);
            }
        }
        assert!(
            seen.len() > 12,
            "procedural shader must vary across the shape"
        );
        assert!(
            seen.iter().all(|pixel| pixel[3] == 255),
            "opaque shader must cover the shape"
        );
    }
}
