use std::{collections::VecDeque, io::Cursor};

use anyhow::{Context as _, Result, ensure};
use fanta_doc::{BitmapNode, Bounds, ImageFitMode, Transform2D};
use fanta_tools::select::DrawSelectionShape;
use glam::DVec2;
use image::{Rgba, RgbaImage};

const MAX_DECODED_PIXELS: u64 = 32_000_000;
const MAX_MASK_RUNS: usize = 4096;

pub(crate) fn decode_wand_thumbnail(bytes: &[u8]) -> Result<RgbaImage> {
    let mut reader = image::ImageReader::new(Cursor::new(bytes)).with_guessed_format()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(16_384);
    limits.max_image_height = Some(16_384);
    limits.max_alloc = Some(MAX_DECODED_PIXELS * 8);
    reader.limits(limits);
    let image = reader
        .decode()
        .context("Could not decode the selected image")?;
    ensure!(
        u64::from(image.width()) * u64::from(image.height()) <= MAX_DECODED_PIXELS,
        "The selected image is too large for pixel selection"
    );
    Ok(image.thumbnail(512, 512).to_rgba8())
}

pub(crate) fn bitmap_wand_region(
    pixels: &RgbaImage,
    bitmap: &BitmapNode,
    world_transform: Transform2D,
    world_click: DVec2,
    tolerance: u8,
    contiguous: bool,
) -> Result<Option<DrawSelectionShape>> {
    ensure!(
        bitmap
            .local_size
            .into_iter()
            .all(|size| size.is_finite() && size > 0.0),
        "The image has no selectable area"
    );
    let [a, b, c, d, _, _] = world_transform.to_components();
    ensure!(
        (a * d - b * c).is_finite() && (a * d - b * c).abs() > 1e-12,
        "The image transform is flattened"
    );
    ensure!(
        bitmap.fit != ImageFitMode::Tile,
        "Pixel selection is unavailable for tiled images"
    );
    let local_click = world_transform.inverse().transform_point(world_click);
    if local_click.x < 0.0
        || local_click.y < 0.0
        || local_click.x >= bitmap.local_size[0]
        || local_click.y >= bitmap.local_size[1]
    {
        return Ok(None);
    }
    let mapping = ImageMapping::new(bitmap, pixels)?;
    let Some(seed_color) = mapping.sample(local_click) else {
        return Ok(None);
    };
    for resolution in [256_u32, 128, 64] {
        let columns = pixels.width().min(resolution).max(1) as usize;
        let rows = pixels.height().min(resolution).max(1) as usize;
        let sample_width = bitmap.local_size[0] / columns as f64;
        let sample_height = bitmap.local_size[1] / rows as f64;
        let samples: Vec<_> = (0..rows)
            .flat_map(|row| {
                let mapping = &mapping;
                (0..columns).map(move |column| {
                    mapping.sample(DVec2::new(
                        (column as f64 + 0.5) * sample_width,
                        (row as f64 + 0.5) * sample_height,
                    ))
                })
            })
            .collect();
        let mut selected = vec![false; samples.len()];
        if contiguous {
            let seed_column = (local_click.x / sample_width).floor() as usize;
            let seed_row = (local_click.y / sample_height).floor() as usize;
            let seed = seed_row.min(rows - 1) * columns + seed_column.min(columns - 1);
            let mut queue = VecDeque::from([seed]);
            selected[seed] = true;
            while let Some(index) = queue.pop_front() {
                let column = index % columns;
                let row = index / columns;
                let neighbors = [
                    (column > 0).then_some(index.saturating_sub(1)),
                    (column + 1 < columns).then_some(index + 1),
                    (row > 0).then_some(index.saturating_sub(columns)),
                    (row + 1 < rows).then_some(index + columns),
                ];
                for neighbor in neighbors.into_iter().flatten() {
                    if !selected[neighbor]
                        && samples[neighbor]
                            .is_some_and(|color| similar_pixel(color, seed_color, tolerance))
                    {
                        selected[neighbor] = true;
                        queue.push_back(neighbor);
                    }
                }
            }
        } else {
            for (index, sample) in samples.iter().enumerate() {
                selected[index] =
                    sample.is_some_and(|color| similar_pixel(color, seed_color, tolerance));
            }
        }
        let mut quads = Vec::new();
        let mut world_minimum = DVec2::splat(f64::INFINITY);
        let mut world_maximum = DVec2::splat(f64::NEG_INFINITY);
        for row in 0..rows {
            let mut column = 0;
            while column < columns {
                if !selected[row * columns + column] {
                    column += 1;
                    continue;
                }
                let start = column;
                while column < columns && selected[row * columns + column] {
                    column += 1;
                }
                let x0 = start as f64 * sample_width;
                let x1 = column as f64 * sample_width;
                let y0 = row as f64 * sample_height;
                let y1 = (row + 1) as f64 * sample_height;
                let quad = [
                    DVec2::new(x0, y0),
                    DVec2::new(x1, y0),
                    DVec2::new(x1, y1),
                    DVec2::new(x0, y1),
                ]
                .map(|point| world_transform.transform_point(point));
                for point in quad {
                    world_minimum = world_minimum.min(point);
                    world_maximum = world_maximum.max(point);
                }
                quads.push(quad.map(|point| point.to_array()));
            }
        }
        if quads.len() > MAX_MASK_RUNS {
            continue;
        }
        return Ok((!quads.is_empty()).then(|| DrawSelectionShape::RasterRuns {
            quads,
            bounds: Bounds::from_min_max(world_minimum, world_maximum),
        }));
    }
    Ok(None)
}

fn similar_pixel(candidate: Rgba<u8>, seed: Rgba<u8>, tolerance: u8) -> bool {
    candidate
        .0
        .into_iter()
        .zip(seed.0)
        .all(|(candidate, seed)| candidate.abs_diff(seed) <= tolerance)
}

struct ImageMapping<'a> {
    pixels: &'a RgbaImage,
    natural_size: [f32; 2],
    source: [f32; 4],
    destination: [f32; 4],
}

impl<'a> ImageMapping<'a> {
    fn new(bitmap: &BitmapNode, pixels: &'a RgbaImage) -> Result<Self> {
        let natural_size = [bitmap.natural_size[0] as f32, bitmap.natural_size[1] as f32];
        ensure!(
            natural_size[0] > 0.0 && natural_size[1] > 0.0,
            "The image has no pixels"
        );
        let crop = fanta_render::crop_to_pixels(bitmap.crop, natural_size[0], natural_size[1]);
        let fit = fanta_render::fit_src_dst(
            bitmap.fit,
            crop[2],
            crop[3],
            bitmap.local_size[0] as f32,
            bitmap.local_size[1] as f32,
        );
        Ok(Self {
            pixels,
            natural_size,
            source: [
                crop[0] + fit.src[0],
                crop[1] + fit.src[1],
                fit.src[2],
                fit.src[3],
            ],
            destination: fit.dst,
        })
    }

    fn sample(&self, local: DVec2) -> Option<Rgba<u8>> {
        let [x, y, width, height] = self.destination;
        if width <= 0.0 || height <= 0.0 {
            return None;
        }
        let unit_x = (local.x as f32 - x) / width;
        let unit_y = (local.y as f32 - y) / height;
        if !(0.0..1.0).contains(&unit_x) || !(0.0..1.0).contains(&unit_y) {
            return None;
        }
        let source_x = self.source[0] + unit_x * self.source[2];
        let source_y = self.source[1] + unit_y * self.source[3];
        let sample_x = (source_x / self.natural_size[0] * self.pixels.width() as f32).floor();
        let sample_y = (source_y / self.natural_size[1] * self.pixels.height() as f32).floor();
        if !sample_x.is_finite() || !sample_y.is_finite() {
            return None;
        }
        let sample_x = (sample_x as u32).min(self.pixels.width().saturating_sub(1));
        let sample_y = (sample_y as u32).min(self.pixels.height().saturating_sub(1));
        self.pixels.get_pixel_checked(sample_x, sample_y).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bitmap() -> BitmapNode {
        BitmapNode {
            asset: fanta_doc::AssetId::new(),
            natural_size: [4, 4],
            local_size: [4.0, 4.0],
            crop: None,
            fit: ImageFitMode::Stretch,
            tint: None,
        }
    }

    #[test]
    fn contiguous_pixel_wand_keeps_only_the_connected_color_area() {
        let pixels = RgbaImage::from_fn(4, 4, |x, _| {
            if x == 0 || x == 3 {
                Rgba([200, 10, 10, 255])
            } else {
                Rgba([10, 10, 200, 255])
            }
        });
        let connected = bitmap_wand_region(
            &pixels,
            &bitmap(),
            Transform2D::IDENTITY,
            DVec2::new(0.5, 0.5),
            0,
            true,
        )
        .expect("wand")
        .expect("connected pixels");
        assert!(
            matches!(&connected, DrawSelectionShape::RasterRuns { quads, bounds } if quads.len() == 4 && bounds.max_x == 1.0)
        );
        let all = bitmap_wand_region(
            &pixels,
            &bitmap(),
            Transform2D::IDENTITY,
            DVec2::new(0.5, 0.5),
            0,
            false,
        )
        .expect("wand")
        .expect("matching pixels");
        assert!(
            matches!(&all, DrawSelectionShape::RasterRuns { quads, bounds } if quads.len() == 8 && bounds.max_x == 4.0)
        );
    }

    #[test]
    fn pixel_wand_maps_image_crop_and_transform_to_world() {
        let pixels = RgbaImage::from_fn(4, 4, |x, _| {
            if x < 2 {
                Rgba([200, 0, 0, 255])
            } else {
                Rgba([0, 0, 200, 255])
            }
        });
        let mut bitmap = bitmap();
        bitmap.crop = Some([0.5, 0.0, 0.5, 1.0]);
        let shape = bitmap_wand_region(
            &pixels,
            &bitmap,
            Transform2D::translation(10.0, 20.0),
            DVec2::new(10.5, 20.5),
            0,
            true,
        )
        .expect("wand")
        .expect("pixels");
        assert_eq!(
            shape.bounds(),
            Some(Bounds::from_xywh(10.0, 20.0, 4.0, 4.0))
        );
    }
}
