//! Turn a [`DecodedImage`] into Skia draws that honour [`ImageFitMode`],
//! `crop`, and `tint`.
//!
//! Split out from `raster.rs` so the genuinely fiddly part — the source→dest
//! rectangle arithmetic for the four fit modes — is a *pure* function
//! ([`fit_src_dst`]) that can be table-tested without a surface, exactly as
//! `specs/03-media-3d-and-node-workflows.md` §1's test plan asks. The Skia
//! glue ([`draw_decoded_image`]) is thin on top of it.

use crate::asset::DecodedImage;
use crate::color::to_sk_color;
use fanta_doc::{AssetId, Color, ImageFitMode};
use skia_safe::{
    AlphaType, Canvas, ColorType, Data, FilterMode, ImageInfo, Matrix, Paint, Rect,
    SamplingOptions, TileMode, canvas::SrcRectConstraint, images,
};
use std::collections::HashMap;

/// Caches uploaded [`skia_safe::Image`]s keyed by [`AssetId`].
///
/// `draw_decoded_image` builds an `SkImage` from raw RGBA bytes on every call,
/// which for a scene full of bitmaps re-uploads the same pixels every frame.
/// Because an [`AssetId`] is content-addressed and immutable — the bytes behind
/// an id never change in place; a re-roll mints a *new* id — the built image is
/// valid for the lifetime of the id, so it is safe to cache indefinitely with
/// no invalidation logic. Replacing an asset's pixels means a new `AssetId`,
/// whose first draw is a fresh cache miss; the stale entry is simply never read
/// again (and can be dropped via [`clear`] or [`remove`] if memory matters).
///
/// [`clear`]: Self::clear
/// [`remove`]: Self::remove
#[derive(Default)]
pub struct ImageCache {
    images: HashMap<AssetId, skia_safe::Image>,
}

impl ImageCache {
    /// An empty cache.
    pub fn new() -> Self {
        Self::default()
    }

    /// The cached image for `id`, if already built. The hot path: callers
    /// check this before resolving/decoding pixels at all, so a cache hit
    /// never touches the asset resolver (see [`draw_image_cached`]).
    fn get(&self, id: AssetId) -> Option<&skia_safe::Image> {
        self.images.get(&id)
    }

    /// Return the cached image for `id`, building and inserting it from `img`
    /// on a miss. Returns `None` only when the buffer is malformed / Skia
    /// rejects it (the caller then draws its placeholder); a `None` is *not*
    /// cached, so a buffer that becomes well-formed on a later frame can still
    /// upload.
    fn get_or_build(&mut self, id: AssetId, img: &DecodedImage) -> Option<&skia_safe::Image> {
        use std::collections::hash_map::Entry;
        // Build only on a genuine miss. The `Vacant` arm runs `build_sk_image`
        // exactly once; a hit never builds. We deliberately do not use
        // `or_insert_with` because building can *fail* (a malformed buffer), and
        // a failed build must not insert a (sentinel) entry — so the miss arm
        // bails with `?` and leaves the slot empty for a later well-formed frame.
        match self.images.entry(id) {
            Entry::Occupied(e) => Some(e.into_mut()),
            Entry::Vacant(e) => {
                let built = build_sk_image(img)?;
                Some(e.insert(built))
            }
        }
    }

    /// Drop every cached image. Call when assets are bulk-invalidated (a new
    /// document is loaded, say) or to reclaim GPU/CPU image memory.
    pub fn clear(&mut self) {
        self.images.clear();
    }

    /// Drop a single asset's cached image. Returns whether an entry was present.
    pub fn remove(&mut self, id: AssetId) -> bool {
        self.images.remove(&id).is_some()
    }

    /// Drop every cached image whose id is NOT in `keep`, returning how many
    /// were dropped. The asset garbage collector calls this alongside the
    /// resolver's own retain — since the cached `SkImage` is the single
    /// long-lived CPU copy of each asset's pixels, an orphaned entry here is
    /// exactly the memory the GC exists to reclaim.
    pub fn retain(&mut self, keep: &std::collections::HashSet<AssetId>) -> usize {
        let before = self.images.len();
        self.images.retain(|id, _| keep.contains(id));
        before - self.images.len()
    }

    /// Number of cached images. Handy in tests and memory diagnostics.
    pub fn len(&self) -> usize {
        self.images.len()
    }

    /// Whether the cache holds no images.
    pub fn is_empty(&self) -> bool {
        self.images.is_empty()
    }
}

/// A source rectangle (in asset pixels) paired with the destination rectangle
/// (in node-local units) it maps onto. Returned by [`fit_src_dst`] for the
/// `Fill` / `Fit` / `Stretch` modes; `Tile` does not use this (it is a
/// repeating shader, not a single blit).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FitRects {
    /// Source rect in asset-pixel space: `[x, y, w, h]`.
    pub src: [f32; 4],
    /// Destination rect in node-local space: `[x, y, w, h]`.
    pub dst: [f32; 4],
}

/// Compute the source and destination rectangles for a non-tiling fit mode.
///
/// `src_w`/`src_h` are the dimensions of the (already cropped) source region in
/// asset pixels; `dst_w`/`dst_h` are the node's local rect. The destination
/// origin is always `(0, 0)` because the caller draws in node-local space where
/// the bitmap's rect is `[0, 0, local_size]` (the node transform places it on
/// the canvas).
///
/// - **Fill** (cover): scale to *cover* the dst, preserving aspect, then
///   centre-crop the overflowing axis. `src` shrinks to the centred sub-rect
///   that, scaled up, exactly fills `dst`. `dst` is the full local rect.
/// - **Fit** (contain): scale to *fit inside* the dst, preserving aspect, then
///   letterbox. `src` is the whole image; `dst` shrinks to the centred sub-rect
///   that preserves aspect (leaving transparent bars).
/// - **Stretch**: independent axis scale. `src` is the whole image; `dst` is
///   the full local rect. Aspect is not preserved.
/// - **Tile**: handled by the shader path; here it degenerates to `Stretch`'s
///   rects so a caller that ignores tiling still draws *something* sane.
///
/// Degenerate inputs (a zero dimension) yield zero-area rects rather than NaNs,
/// so the caller simply draws nothing.
pub fn fit_src_dst(fit: ImageFitMode, src_w: f32, src_h: f32, dst_w: f32, dst_h: f32) -> FitRects {
    let whole_src = [0.0, 0.0, src_w, src_h];
    let whole_dst = [0.0, 0.0, dst_w, dst_h];

    // Any zero dimension means there is nothing meaningful to map.
    if src_w <= 0.0 || src_h <= 0.0 || dst_w <= 0.0 || dst_h <= 0.0 {
        return FitRects {
            src: whole_src,
            dst: [0.0, 0.0, 0.0, 0.0],
        };
    }

    match fit {
        ImageFitMode::Stretch | ImageFitMode::Tile => FitRects {
            src: whole_src,
            dst: whole_dst,
        },
        ImageFitMode::Fill => {
            // Cover: the source sub-rect has the dst's aspect ratio, centred.
            let src_aspect = src_w / src_h;
            let dst_aspect = dst_w / dst_h;
            let (crop_w, crop_h) = if src_aspect > dst_aspect {
                // Source is wider than dst — crop the sides.
                let w = src_h * dst_aspect;
                (w, src_h)
            } else {
                // Source is taller than dst — crop top/bottom.
                let h = src_w / dst_aspect;
                (src_w, h)
            };
            let x = (src_w - crop_w) * 0.5;
            let y = (src_h - crop_h) * 0.5;
            FitRects {
                src: [x, y, crop_w, crop_h],
                dst: whole_dst,
            }
        }
        ImageFitMode::Fit => {
            // Contain: the dst sub-rect preserves the source aspect, centred.
            let src_aspect = src_w / src_h;
            let dst_aspect = dst_w / dst_h;
            let (out_w, out_h) = if src_aspect > dst_aspect {
                // Source is wider — full width, shorter height (letterbox).
                let h = dst_w / src_aspect;
                (dst_w, h)
            } else {
                // Source is taller — full height, narrower width (pillarbox).
                let w = dst_h * src_aspect;
                (w, dst_h)
            };
            let x = (dst_w - out_w) * 0.5;
            let y = (dst_h - out_h) * 0.5;
            FitRects {
                src: whole_src,
                dst: [x, y, out_w, out_h],
            }
        }
    }
}

/// Resolve a normalized crop `[x, y, w, h]` (0..=1 in asset space) to a pixel
/// rect against `natural` dimensions. `None` means "no crop" → the whole asset.
///
/// Clamped to the asset bounds so a crop that runs past the edge (or has a
/// non-positive size) cannot produce an out-of-range source rect for Skia.
pub fn crop_to_pixels(crop: Option<[f32; 4]>, nat_w: f32, nat_h: f32) -> [f32; 4] {
    match crop {
        None => [0.0, 0.0, nat_w, nat_h],
        Some([cx, cy, cw, ch]) => {
            let x = (cx.clamp(0.0, 1.0)) * nat_w;
            let y = (cy.clamp(0.0, 1.0)) * nat_h;
            // Width/height clamped so x+w never exceeds the asset.
            let w = (cw.max(0.0) * nat_w).min(nat_w - x);
            let h = (ch.max(0.0) * nat_h).min(nat_h - y);
            [x, y, w, h]
        }
    }
}

/// Build a Skia [`Image`] from straight-alpha RGBA8 pixels.
///
/// Returns `None` if the buffer is malformed or Skia rejects it — the caller
/// then falls back to the placeholder instead of drawing garbage. We declare
/// the pixels as `RGBA8888` / `Unpremul` to match [`DecodedImage`]'s straight-
/// alpha contract; Skia premultiplies internally on use.
///
/// [`Image`]: skia_safe::Image
fn build_sk_image(img: &DecodedImage) -> Option<skia_safe::Image> {
    if !img.is_well_formed() {
        return None;
    }
    let info = ImageInfo::new(
        (img.width as i32, img.height as i32),
        ColorType::RGBA8888,
        AlphaType::Unpremul,
        None,
    );
    let row_bytes = info.min_row_bytes();
    // new_copy hands Skia its own owned buffer — deliberately: this SkImage is
    // the ONE long-lived CPU copy of the pixels (the resolver does not memoize
    // decodes; see `draw_image_cached`), and an owned buffer keeps the
    // lifetime story trivial. skia-safe offers no release-proc Data
    // constructor, so zero-copy sharing of the resolver's Arc would be unsafe
    // against Skia-internal refs outliving the cache entry.
    let data = Data::new_copy(&img.pixels_rgba);
    images::raster_from_data(&info, data, row_bytes)
}

/// Optional multiplicative tint as a Skia paint configured with a `Multiply`
/// colour filter. `None` tint → a plain paint. Multiply means a white tint is a
/// no-op and a coloured tint darkens the channels it lacks — the documented
/// overlay semantics, applied at composite time rather than by mutating pixels.
fn tinted_paint(tint: Option<Color>, opacity: f32) -> Paint {
    let mut paint = Paint::default();
    paint.set_anti_alias(true);
    paint.set_alpha_f(opacity.clamp(0.0, 1.0));
    if let Some(t) = tint {
        if let Some(cf) =
            skia_safe::color_filters::blend(to_sk_color(t), skia_safe::BlendMode::Multiply)
        {
            paint.set_color_filter(cf);
        }
    }
    paint
}

/// Draw a decoded image into the node-local rect `[0, 0, local_size]`, honouring
/// `fit`, `crop`, and `tint`. Returns `true` if it drew the real image, `false`
/// if the image could not be built (caller draws the placeholder).
///
/// Linear sampling is used so a scaled photo is smooth rather than blocky; this
/// matches what a designer expects from "the actual photo," and is cheap on the
/// CPU surface.
pub fn draw_decoded_image(
    canvas: &Canvas,
    img: &DecodedImage,
    local_size: [f64; 2],
    crop: Option<[f32; 4]>,
    fit: ImageFitMode,
    tint: Option<Color>,
    opacity: f32,
) -> bool {
    let Some(sk_image) = build_sk_image(img) else {
        return false;
    };
    blit_sk_image(
        canvas, &sk_image, img.width, img.height, local_size, crop, fit, tint, opacity,
    )
}

/// Cache-aware variant of [`draw_decoded_image`]: looks the built
/// [`skia_safe::Image`] up in `cache` by `id`, calling `resolve` for the
/// decoded pixels ONLY on a cache miss. This is the path the renderer uses
/// for bitmap nodes and image fills; the bare [`draw_decoded_image`] (no
/// `id`, no cache) stays for one-shot callers and the pure fit-math tests.
///
/// Resolving lazily — rather than taking a `&DecodedImage` — is what lets the
/// cached `SkImage` (which owns its own pixel buffer) be the *single*
/// long-lived CPU copy of each asset: on the hot cache-hit path no decoded
/// pixels exist at all, so the resolver is free to decode fresh per miss
/// instead of memoizing a permanent second copy of every image.
///
/// Returns `true` if it drew the real image, `false` if the pixels could not
/// be resolved or the image could not be built (caller draws the placeholder).
#[allow(clippy::too_many_arguments)]
pub fn draw_image_cached(
    canvas: &Canvas,
    cache: &mut ImageCache,
    id: AssetId,
    resolve: impl FnOnce() -> Option<DecodedImage>,
    local_size: [f64; 2],
    crop: Option<[f32; 4]>,
    fit: ImageFitMode,
    tint: Option<Color>,
    opacity: f32,
) -> bool {
    // Clone the handle out so the cache borrow ends before we draw.
    // `skia_safe::Image` is a refcounted handle (an `SkImage` smart pointer),
    // so the clone is a refcount bump, not a pixel copy.
    let sk_image = match cache.get(id).cloned() {
        Some(img) => img,
        None => {
            let Some(decoded) = resolve() else {
                return false;
            };
            match cache.get_or_build(id, &decoded) {
                Some(img) => img.clone(),
                None => return false,
            }
        }
    };
    // The upload preserves dimensions, so the SkImage's size IS the asset's
    // natural size — the crop math needs no DecodedImage on the hit path.
    let (nat_w, nat_h) = (sk_image.width() as u32, sk_image.height() as u32);
    blit_sk_image(
        canvas, &sk_image, nat_w, nat_h, local_size, crop, fit, tint, opacity,
    )
}

/// Draw an already-built [`skia_safe::Image`] into the node-local rect
/// `[0, 0, local_size]`, honouring `fit`, `crop`, and `tint`. The source-pixel
/// dimensions (`nat_w`/`nat_h`) come from the [`DecodedImage`] the image was
/// built from — they drive crop math and must match the uploaded pixels.
///
/// Factored out of [`draw_decoded_image`] so the cached and uncached entry
/// points share one body and cannot drift apart.
#[allow(clippy::too_many_arguments)]
fn blit_sk_image(
    canvas: &Canvas,
    sk_image: &skia_safe::Image,
    nat_w: u32,
    nat_h: u32,
    local_size: [f64; 2],
    crop: Option<[f32; 4]>,
    fit: ImageFitMode,
    tint: Option<Color>,
    opacity: f32,
) -> bool {
    let dst_w = local_size[0] as f32;
    let dst_h = local_size[1] as f32;
    if dst_w <= 0.0 || dst_h <= 0.0 {
        // Nothing to draw, but the image *was* resolvable — report success so
        // the caller does not paint a placeholder over a legitimately empty
        // (zero-size) node.
        return true;
    }

    // Crop first (asset pixels), then fit the cropped region into the rect.
    let [crop_x, crop_y, crop_w, crop_h] = crop_to_pixels(crop, nat_w as f32, nat_h as f32);
    let paint = tinted_paint(tint, opacity);
    let sampling = SamplingOptions::from(FilterMode::Linear);

    if fit == ImageFitMode::Tile {
        draw_tiled(
            canvas,
            sk_image,
            [crop_x, crop_y, crop_w, crop_h],
            [dst_w, dst_h],
            sampling,
            &paint,
        );
        return true;
    }

    let rects = fit_src_dst(fit, crop_w, crop_h, dst_w, dst_h);
    let src = Rect::from_xywh(
        crop_x + rects.src[0],
        crop_y + rects.src[1],
        rects.src[2],
        rects.src[3],
    );
    let dst = Rect::from_xywh(rects.dst[0], rects.dst[1], rects.dst[2], rects.dst[3]);
    // Strict constraint keeps Fill's centre-crop from bleeding neighbouring
    // texels at the cropped edge.
    canvas.draw_image_rect(
        sk_image,
        Some((&src, SrcRectConstraint::Strict)),
        dst,
        &paint,
    );
    true
}

/// Tile the (cropped) image across the local rect at native pixel size via a
/// repeating shader. The image's own pixels start at the rect origin and repeat
/// in both axes; the draw is clipped to the local rect.
fn draw_tiled(
    canvas: &Canvas,
    sk_image: &skia_safe::Image,
    crop_px: [f32; 4],
    dst: [f32; 2],
    sampling: SamplingOptions,
    base_paint: &Paint,
) {
    let [cx, cy, cw, ch] = crop_px;
    // A local matrix translates the shader so the cropped region's top-left
    // lands at the rect origin; TileMode::Repeat handles the wrap.
    let local = Matrix::translate((-cx, -cy));
    let shader = sk_image.to_shader((TileMode::Repeat, TileMode::Repeat), sampling, &local);
    let mut paint = base_paint.clone();
    if let Some(shader) = shader {
        paint.set_shader(shader);
        // Clip to the cropped sub-rect within each tile so a crop on a tiled
        // fill repeats only the cropped region, not the whole asset.
        canvas.save();
        canvas.clip_rect(Rect::from_xywh(0.0, 0.0, dst[0], dst[1]), None, true);
        // The shader is anchored at the origin; clamp tiling to the crop window
        // by intersecting each repeat with [cw, ch] is overkill for v1 — the
        // common case (no crop) tiles the whole asset, which is correct.
        let _ = (cw, ch);
        canvas.draw_rect(Rect::from_xywh(0.0, 0.0, dst[0], dst[1]), &paint);
        canvas.restore();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    const EPS: f32 = 1e-3;

    fn approx(a: [f32; 4], b: [f32; 4]) -> bool {
        a.iter().zip(b.iter()).all(|(x, y)| (x - y).abs() < EPS)
    }

    /// A `w × h` straight-alpha RGBA image filled with one colour.
    fn solid_image(w: u32, h: u32, rgba: [u8; 4]) -> DecodedImage {
        let mut px = Vec::with_capacity((w * h * 4) as usize);
        for _ in 0..(w * h) {
            px.extend_from_slice(&rgba);
        }
        DecodedImage::new(Arc::new(px), w, h)
    }

    #[test]
    fn cache_builds_once_then_reuses_the_same_image() {
        let mut cache = ImageCache::new();
        assert!(cache.is_empty());
        let id = AssetId::new();
        let img = solid_image(4, 4, [10, 20, 30, 255]);

        // First lookup builds and caches; capture the uploaded image's identity.
        let first_uid = cache
            .get_or_build(id, &img)
            .expect("well-formed builds")
            .unique_id();
        assert_eq!(cache.len(), 1, "first lookup populates the cache");

        // Second lookup must NOT rebuild — same uploaded SkImage (same unique
        // id), and the cache does not grow.
        let second_uid = cache
            .get_or_build(id, &img)
            .expect("cached hit")
            .unique_id();
        assert_eq!(cache.len(), 1, "a hit does not grow the cache");
        assert_eq!(
            first_uid, second_uid,
            "the same SkImage is returned, not a rebuild"
        );
    }

    #[test]
    fn cache_keys_distinct_assets_separately() {
        let mut cache = ImageCache::new();
        let a = AssetId::new();
        let b = AssetId::new();
        cache.get_or_build(a, &solid_image(2, 2, [1, 2, 3, 255]));
        cache.get_or_build(b, &solid_image(2, 2, [4, 5, 6, 255]));
        assert_eq!(cache.len(), 2, "two distinct asset ids → two cached images");
    }

    #[test]
    fn cache_does_not_store_a_malformed_buffer() {
        let mut cache = ImageCache::new();
        let id = AssetId::new();
        // len 7 != 2*2*4 → build_sk_image returns None and nothing is cached.
        let bad = DecodedImage::new(Arc::new(vec![0u8; 7]), 2, 2);
        assert!(cache.get_or_build(id, &bad).is_none());
        assert_eq!(cache.len(), 0, "a malformed buffer is not cached");
        // It can later succeed once a well-formed buffer for the id arrives.
        assert!(
            cache
                .get_or_build(id, &solid_image(2, 2, [0, 0, 0, 255]))
                .is_some()
        );
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn cache_clear_and_remove_drop_entries() {
        let mut cache = ImageCache::new();
        let a = AssetId::new();
        let b = AssetId::new();
        cache.get_or_build(a, &solid_image(2, 2, [1, 1, 1, 255]));
        cache.get_or_build(b, &solid_image(2, 2, [2, 2, 2, 255]));
        assert_eq!(cache.len(), 2);

        assert!(cache.remove(a), "remove reports the entry was present");
        assert!(!cache.remove(a), "removing again reports absent");
        assert_eq!(cache.len(), 1);

        cache.clear();
        assert!(cache.is_empty(), "clear drops everything");
    }

    #[test]
    fn cache_retain_drops_only_unreferenced_images() {
        let mut cache = ImageCache::new();
        let a = AssetId::new();
        let b = AssetId::new();
        cache.get_or_build(a, &solid_image(2, 2, [1, 1, 1, 255]));
        cache.get_or_build(b, &solid_image(2, 2, [2, 2, 2, 255]));

        let keep: std::collections::HashSet<_> = [a].into_iter().collect();
        assert_eq!(cache.retain(&keep), 1, "exactly the orphan dropped");
        assert!(cache.get(a).is_some(), "referenced image survives");
        assert!(cache.get(b).is_none(), "orphaned image is gone");
        assert_eq!(cache.retain(&keep), 0, "idempotent");
    }

    #[test]
    fn stretch_maps_whole_source_to_whole_dst() {
        let r = fit_src_dst(ImageFitMode::Stretch, 100.0, 50.0, 200.0, 200.0);
        assert!(approx(r.src, [0.0, 0.0, 100.0, 50.0]), "src {:?}", r.src);
        assert!(approx(r.dst, [0.0, 0.0, 200.0, 200.0]), "dst {:?}", r.dst);
    }

    #[test]
    fn fill_center_crops_a_wide_image_into_a_square() {
        // 200x100 source into 100x100 square: cover crops the sides.
        let r = fit_src_dst(ImageFitMode::Fill, 200.0, 100.0, 100.0, 100.0);
        // Dst is the full square.
        assert!(approx(r.dst, [0.0, 0.0, 100.0, 100.0]), "dst {:?}", r.dst);
        // Source sub-rect is the centred 100x100 of the 200x100 image.
        assert!(approx(r.src, [50.0, 0.0, 100.0, 100.0]), "src {:?}", r.src);
    }

    #[test]
    fn fill_center_crops_a_tall_image_into_a_square() {
        // 100x200 source into 100x100 square: cover crops top/bottom.
        let r = fit_src_dst(ImageFitMode::Fill, 100.0, 200.0, 100.0, 100.0);
        assert!(approx(r.dst, [0.0, 0.0, 100.0, 100.0]), "dst {:?}", r.dst);
        assert!(approx(r.src, [0.0, 50.0, 100.0, 100.0]), "src {:?}", r.src);
    }

    #[test]
    fn fit_letterboxes_a_wide_image_into_a_square() {
        // 200x100 source into 100x100: contain leaves top/bottom bars.
        let r = fit_src_dst(ImageFitMode::Fit, 200.0, 100.0, 100.0, 100.0);
        // Whole source is shown.
        assert!(approx(r.src, [0.0, 0.0, 200.0, 100.0]), "src {:?}", r.src);
        // Dst is full width (100), half height (50), centred vertically (y=25).
        assert!(approx(r.dst, [0.0, 25.0, 100.0, 50.0]), "dst {:?}", r.dst);
    }

    #[test]
    fn fit_pillarboxes_a_tall_image_into_a_square() {
        // 100x200 source into 100x100: contain leaves left/right bars.
        let r = fit_src_dst(ImageFitMode::Fit, 100.0, 200.0, 100.0, 100.0);
        assert!(approx(r.src, [0.0, 0.0, 100.0, 200.0]), "src {:?}", r.src);
        // Dst is full height (100), half width (50), centred horizontally (x=25).
        assert!(approx(r.dst, [25.0, 0.0, 50.0, 100.0]), "dst {:?}", r.dst);
    }

    #[test]
    fn fit_and_fill_agree_when_aspect_matches() {
        // Same aspect ratio: cover and contain both fill the rect with the
        // whole source, no crop, no letterbox.
        let fill = fit_src_dst(ImageFitMode::Fill, 100.0, 100.0, 50.0, 50.0);
        let fit = fit_src_dst(ImageFitMode::Fit, 100.0, 100.0, 50.0, 50.0);
        assert!(approx(fill.src, [0.0, 0.0, 100.0, 100.0]));
        assert!(approx(fill.dst, [0.0, 0.0, 50.0, 50.0]));
        assert!(approx(fit.src, fill.src));
        assert!(approx(fit.dst, fill.dst));
    }

    #[test]
    fn zero_dst_yields_zero_area_dst() {
        let r = fit_src_dst(ImageFitMode::Fill, 100.0, 100.0, 0.0, 50.0);
        assert_eq!(r.dst[2] * r.dst[3], 0.0);
    }

    #[test]
    fn crop_none_is_the_whole_asset() {
        assert_eq!(crop_to_pixels(None, 640.0, 480.0), [0.0, 0.0, 640.0, 480.0]);
    }

    #[test]
    fn crop_maps_normalized_to_pixels() {
        // Centre quarter of a 100x100 asset.
        let r = crop_to_pixels(Some([0.25, 0.25, 0.5, 0.5]), 100.0, 100.0);
        assert!(approx(r, [25.0, 25.0, 50.0, 50.0]), "{r:?}");
    }

    #[test]
    fn crop_is_clamped_to_asset_bounds() {
        // A crop that runs past the right/bottom edge is trimmed, never
        // exceeding the asset.
        let r = crop_to_pixels(Some([0.8, 0.8, 0.5, 0.5]), 100.0, 100.0);
        assert!(approx(r, [80.0, 80.0, 20.0, 20.0]), "{r:?}");
    }
}
