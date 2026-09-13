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
use crate::paint::to_sk_blend_mode;
use fanta_doc::{AssetId, BlendMode, Color, ImageAdjust, ImageFitMode};
use skia_safe::{
    AlphaType, Canvas, ColorFilter, ColorType, Data, FilterMode, ImageInfo, Matrix, MipmapMode,
    Paint, Rect, SamplingOptions, TileMode, canvas::SrcRectConstraint, images,
};
use std::collections::HashMap;
use std::sync::OnceLock;

/// The per-paint modifiers a [`Fill::Image`](fanta_doc::Fill::Image) carries
/// beyond fit/crop/tint — bundled so the image-draw entry points don't grow an
/// argument per Figma paint field. [`Default`] (no scale, no rotation, normal
/// blend) reproduces the plain draw exactly, and is what non-fill callers
/// (bitmap nodes, video posters) pass.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ImageFillMods {
    /// Tile scaling factor for [`ImageFitMode::Tile`] (Figma
    /// `ImagePaint.scalingFactor`): each tile paints at `natural × scale`.
    /// `None` ⇒ native size. Ignored for non-tile modes.
    pub scale: Option<f32>,
    /// Clockwise rotation of the image within the fill, in degrees (Figma
    /// exposes 90° steps; the draw snaps to the nearest quarter turn).
    pub rotation: Option<f32>,
    /// Blend mode of this paint against the paints below it / the backdrop
    /// (Figma's per-paint `blendMode`, distinct from the node-level blend).
    pub blend: BlendMode,
    /// Non-destructive image adjustments (exposure / contrast / …). Default
    /// (all zero) is a no-op — no colour filter is built.
    pub adjust: ImageAdjust,
}

/// Caches uploaded [`skia_safe::Image`]s keyed by [`AssetId`], plus a lazily
/// built zoom-out **display pyramid** per asset (see [`CachedImage`]).
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
    images: HashMap<AssetId, CachedImage>,
}

/// One asset's cached images: the full-resolution upload plus its display
/// pyramid.
///
/// ## Why a pyramid (image zoom-out LOD)
///
/// A GPU canvas draws a raster `SkImage` by uploading it (with its mip chain)
/// into Ganesh's resource cache, keyed by the image's unique id. A photo-heavy
/// page zoomed out draws every asset at a small fraction of its pixels, yet
/// the FULL-resolution mipped texture is what gets uploaded — on the Agency
/// template Design page, 22 assets = 85 Mpx ≈ 454 MB of mipped RGBA, which
/// blows the 256 MB default budget, so every frame purged and re-uploaded
/// (and re-mipped) them all: ~200 ms of a 458 ms frame at 10% zoom.
///
/// `levels[i]` is the full image's own mip level `i+1`, copied out as a
/// standalone image (see [`build_pyramid_level`]) with its own mip chain — so a
/// level's chain IS the full image's chain from that level down, texel for
/// texel. A minified draw picks the shallowest level
/// whose texel density still meets the device's (see [`minification_level`])
/// and samples that instead: the same texels a trilinear lookup into the full
/// image's chain would read, but the upload is `1/4^level` the size, and the
/// full-resolution texture is never touched while zoomed out. Levels are built
/// on first need (a cheap CPU raster draw, `1/4^level` of the asset's pixels)
/// and kept: total extra CPU memory ≤ 1/3 of the full image, like a mip chain.
/// Zooming back in walks up to level 0 — the untouched full upload — so export
/// / zoom-in fidelity is unchanged.
struct CachedImage {
    /// The full-resolution image (with default mipmaps).
    full: skia_safe::Image,
    /// Display pyramid, contiguous from level 1: `levels[i]` = level `i + 1`.
    levels: Vec<skia_safe::Image>,
}

impl ImageCache {
    /// An empty cache.
    pub fn new() -> Self {
        Self::default()
    }

    /// The cached full-resolution image for `id`, if already built. The hot
    /// path: callers check this before resolving/decoding pixels at all, so a
    /// cache hit never touches the asset resolver (see [`draw_image_cached`]).
    fn get(&self, id: AssetId) -> Option<&skia_safe::Image> {
        self.images.get(&id).map(|c| &c.full)
    }

    /// Return the cached full-resolution image for `id`, building and
    /// inserting it from `img` on a miss. Returns `None` only when the buffer
    /// is malformed / Skia rejects it (the caller then draws its placeholder);
    /// a `None` is *not* cached, so a buffer that becomes well-formed on a
    /// later frame can still upload.
    fn get_or_build(&mut self, id: AssetId, img: &DecodedImage) -> Option<&skia_safe::Image> {
        use std::collections::hash_map::Entry;
        // Build only on a genuine miss. The `Vacant` arm runs `build_sk_image`
        // exactly once; a hit never builds. We deliberately do not use
        // `or_insert_with` because building can *fail* (a malformed buffer), and
        // a failed build must not insert a (sentinel) entry — so the miss arm
        // bails with `?` and leaves the slot empty for a later well-formed frame.
        match self.images.entry(id) {
            Entry::Occupied(e) => Some(&e.into_mut().full),
            Entry::Vacant(e) => {
                let full = build_sk_image(img)?;
                Some(
                    &e.insert(CachedImage {
                        full,
                        levels: Vec::new(),
                    })
                    .full,
                )
            }
        }
    }

    /// The display-pyramid image for `id` at `level` (`0` = the full image;
    /// level `k` = the full image box-downscaled by `2^k`), building the
    /// missing levels on demand. A `level` deeper than the pyramid can go
    /// (a dimension would drop below [`IMAGE_LOD_MIN_LEVEL_PX`]) returns the
    /// deepest level that exists — always at least the device resolution the
    /// caller asked for, since the caller only asks for a level whose texel
    /// density still meets the device's. `None` when `id` is not cached.
    ///
    /// The returned handle is a refcount bump (`skia_safe::Image` is a shared
    /// pointer), so the borrow of the cache ends here.
    fn lod_level(&mut self, id: AssetId, level: u32) -> Option<skia_safe::Image> {
        let cached = self.images.get_mut(&id)?;
        if level == 0 {
            return Some(cached.full.clone());
        }
        while cached.levels.len() < level as usize {
            let Some(next) = build_pyramid_level(&cached.full, cached.levels.len() as u32 + 1)
            else {
                break;
            };
            cached.levels.push(next);
        }
        let idx = (level as usize).min(cached.levels.len());
        Some(if idx == 0 {
            cached.full.clone()
        } else {
            cached.levels[idx - 1].clone()
        })
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
    let image = images::raster_from_data(&info, data, row_bytes)?;
    // Attach the default mip chain so a zoomed-out draw can sample trilinearly
    // (see `blit_sk_image`) instead of aliasing over the full-res pixels. Built
    // once here because the image is cached; ~1/3 extra memory per image. If
    // Skia declines (`None`), the un-mipped image still draws — the minified
    // path checks `has_mipmaps` before asking for mip sampling.
    let mipped = image.with_default_mipmaps();
    Some(mipped.unwrap_or(image))
}

/// The pyramid stops halving once either dimension would drop below this
/// many pixels — a 1-px level is the floor of the chain, exactly like Skia's
/// own mip chain (`max(1, dim >> 1)`).
pub(crate) const IMAGE_LOD_MIN_LEVEL_PX: i32 = 1;

/// Display-pyramid level `level` (≥ 1) of the full image `full`: the full
/// image's OWN mip level `level`, copied out texel-for-texel — a `Nearest` /
/// `MipmapMode::Nearest` draw of `full` at exactly `2^-level` scale selects
/// mip level `level` and lands every device pixel centre on that level's texel
/// centres (for even dimensions; the odd remainder is at most a one-texel
/// drift in the last row/column of a deep level, as Skia's own chain shrinks
/// with `dim >> 1`). The level then carries its own default mip chain, which
/// Skia builds from those very texels — so a minified draw of the level reads
/// exactly the texels a trilinear lookup into `full`'s chain would. `None`
/// when the level's dimensions would fall below [`IMAGE_LOD_MIN_LEVEL_PX`], or
/// if Skia cannot allocate the surface.
fn build_pyramid_level(full: &skia_safe::Image, level: u32) -> Option<skia_safe::Image> {
    let (w, h) = (full.width(), full.height());
    let (lw, lh) = ((w >> level).max(1), (h >> level).max(1));
    // Stop at the floor: once a dimension is already at the floor there is no
    // shallower texel grid to build.
    if (w >> (level - 1)).max(1) <= IMAGE_LOD_MIN_LEVEL_PX
        || (h >> (level - 1)).max(1) <= IMAGE_LOD_MIN_LEVEL_PX
    {
        return None;
    }
    // A raster surface in the image's own pixel geometry (RGBA8888 unpremul on
    // upload; the surface is premul internally, as every draw is — the level
    // is only ever drawn, never read back as straight alpha).
    let info = ImageInfo::new((lw, lh), ColorType::RGBA8888, AlphaType::Premul, None);
    let mut surface = skia_safe::surfaces::raster(&info, None, None)?;
    let canvas = surface.canvas();
    canvas.clear(skia_safe::Color::TRANSPARENT);
    let mut paint = Paint::default();
    // Src (not SrcOver) so a translucent asset's texels are copied, not
    // composited over the cleared transparent surface — identical here, but
    // explicit.
    paint.set_blend_mode(skia_safe::BlendMode::Src);
    let scale = 1.0 / (1u32 << level) as f32;
    canvas.scale((scale, scale));
    canvas.draw_image_rect_with_sampling_options(
        full,
        None,
        Rect::from_wh(w as f32, h as f32),
        SamplingOptions::new(FilterMode::Nearest, MipmapMode::Nearest),
        &paint,
    );
    let level = surface.image_snapshot();
    Some(level.with_default_mipmaps().unwrap_or(level))
}

/// Skia's mip-level bias: `SkMipmap::ComputeLevel` selects the level pair
/// for scale `s` from `L = log2(1/s) − 0.5` ("to emulate GPU's sharpen mipmap
/// option"), i.e. it reads one half-level SHARPER than the geometric level.
/// The pyramid must never be deeper than the shallowest level Skia would read,
/// so the level choice applies the same bias — and stays valid for a GPU
/// sampler with no bias (which reads deeper still).
const MIP_SHARPEN_BIAS: f32 = 0.5;

/// The display-pyramid level a minified image draw should sample: the level
/// Skia's own (sharpen-biased) trilinear lookup would read as its UPPER
/// (sharper) level at `device_scale` (device pixels per source texel, `< 1`
/// when minifying) — `floor(log2(1/s) − MIP_SHARPEN_BIAS)`, at least 0. Level
/// 0 (the full image) for any draw at or above `2^-1.5 ≈ 0.354`× — magnified,
/// 1:1, and mildly minified draws never touch the pyramid; a degenerate
/// (`NaN`/zero) scale also yields 0.
///
/// Because level `k`'s mip chain IS the full image's chain from level `k`
/// down (see [`build_pyramid_level`]), a trilinear draw of level `k` at
/// `s·2^k` reads exactly the texels, at exactly the lerp weight, that the same
/// draw of the full image reads (`log2(1/(s·2^k)) − bias = L − k` — the same
/// fractional level, shifted) — byte-identical output, `1/4^k` the upload.
pub(crate) fn minification_level(device_scale: f32) -> u32 {
    if device_scale <= 0.0 || device_scale >= 1.0 || device_scale.is_nan() {
        return 0;
    }
    let biased = (1.0 / device_scale).log2() - MIP_SHARPEN_BIAS;
    if biased < 1.0 {
        0
    } else {
        biased.floor() as u32
    }
}

/// The largest scale factor the canvas CTM applies to a local unit vector —
/// how many device pixels one local pixel spans. Used to detect minification
/// (`< 1.0`: the image is drawn smaller than its pixels). A degenerate matrix
/// yields `NaN`, which fails every `< 1.0` test — safely selecting the
/// non-mipmapped path.
fn max_device_scale(canvas: &Canvas) -> f32 {
    let m = canvas.local_to_device_as_3x3();
    let x = (m.scale_x() * m.scale_x() + m.skew_y() * m.skew_y()).sqrt();
    let y = (m.skew_x() * m.skew_x() + m.scale_y() * m.scale_y()).sqrt();
    x.max(y)
}

/// Optional multiplicative tint as a Skia paint configured with a `Multiply`
/// colour filter. `None` tint → a plain paint. Multiply means a white tint is a
/// no-op and a coloured tint darkens the channels it lacks — the documented
/// overlay semantics, applied at composite time rather than by mutating pixels.
/// The per-paint `blend` (Figma's paint-level blend mode) rides on the same
/// paint so the image composites against whatever is below it with that mode.
fn tinted_paint(
    tint: Option<Color>,
    opacity: f32,
    blend: BlendMode,
    adjust: &ImageAdjust,
) -> Paint {
    let mut paint = Paint::default();
    paint.set_anti_alias(true);
    paint.set_alpha_f(opacity.clamp(0.0, 1.0));

    // Adjustments run on the source pixels first; the tint multiply composites on
    // top of the adjusted colour.
    let adjust_cf = image_adjust_filter(adjust);
    let tint_cf = tint.and_then(|t| {
        skia_safe::color_filters::blend(to_sk_color(t), skia_safe::BlendMode::Multiply)
    });
    let color_filter = match (tint_cf, adjust_cf) {
        (Some(tint), Some(adjust)) => skia_safe::color_filters::compose(tint, adjust),
        (Some(tint), None) => Some(tint),
        (None, Some(adjust)) => Some(adjust),
        (None, None) => None,
    };
    if let Some(cf) = color_filter {
        paint.set_color_filter(cf);
    }
    if !blend.is_normal() {
        paint.set_blend_mode(to_sk_blend_mode(blend));
    }
    paint
}

/// Build the Skia colour filter for a set of [`ImageAdjust`]ments, or `None`
/// when they are all at rest. The tonal adjustments (exposure / contrast /
/// highlights / shadows) become a per-channel tone-curve LUT; the chromatic ones
/// (saturation / temperature / tint) become a colour matrix; the two compose
/// (tone first, then chroma).
///
/// Results are cached by the bit pattern of the adjust values. A design rarely
/// uses more than a handful of distinct adjustment presets, so this turns
/// repeated 256-entry LUT + matrix construction into a fast lookup.
fn image_adjust_filter(adjust: &ImageAdjust) -> Option<ColorFilter> {
    if adjust.is_default() {
        return None;
    }

    // Content-address the adjust by its raw bytes. f32s from the UI are
    // deterministic; transmuting gives a stable key without float hashing issues.
    let key = adjust_key(adjust);

    static CACHE: OnceLock<std::sync::Mutex<HashMap<[u8; 28], Option<ColorFilter>>>> =
        OnceLock::new();
    let cache = CACHE.get_or_init(|| std::sync::Mutex::new(HashMap::new()));

    let mut guard = cache.lock().unwrap();
    if let Some(cached) = guard.get(&key) {
        return cached.clone();
    }

    let tone = tone_curve_filter(adjust);
    let chroma = chroma_matrix_filter(adjust);
    let result = match (chroma, tone) {
        (Some(chroma), Some(tone)) => skia_safe::color_filters::compose(chroma, tone),
        (Some(chroma), None) => Some(chroma),
        (None, Some(tone)) => Some(tone),
        (None, None) => None,
    };
    guard.insert(key, result.clone());
    result
}

/// 7 f32 fields = 28 bytes. Used as a stable cache key.
fn adjust_key(adjust: &ImageAdjust) -> [u8; 28] {
    let mut b = [0u8; 28];
    b[0..4].copy_from_slice(&adjust.exposure.to_ne_bytes());
    b[4..8].copy_from_slice(&adjust.contrast.to_ne_bytes());
    b[8..12].copy_from_slice(&adjust.saturation.to_ne_bytes());
    b[12..16].copy_from_slice(&adjust.temperature.to_ne_bytes());
    b[16..20].copy_from_slice(&adjust.tint.to_ne_bytes());
    b[20..24].copy_from_slice(&adjust.highlights.to_ne_bytes());
    b[24..28].copy_from_slice(&adjust.shadows.to_ne_bytes());
    b
}

/// A per-channel tone curve encoding exposure (multiplicative), shadows /
/// highlights (weighted tone lifts), and contrast (scale around mid-grey),
/// applied identically to R, G, B. `None` when all four are zero.
fn tone_curve_filter(adjust: &ImageAdjust) -> Option<ColorFilter> {
    if adjust.exposure == 0.0
        && adjust.contrast == 0.0
        && adjust.highlights == 0.0
        && adjust.shadows == 0.0
    {
        return None;
    }
    let exposure_gain = 2.0f32.powf(adjust.exposure);
    let contrast = 1.0 + adjust.contrast;
    let mut table = [0u8; 256];
    for (i, slot) in table.iter_mut().enumerate() {
        let mut v = i as f32 / 255.0;
        v *= exposure_gain;
        // Shadows act mostly on the dark end, highlights on the bright end.
        let shadow_weight = (1.0 - v).clamp(0.0, 1.0);
        let highlight_weight = v.clamp(0.0, 1.0);
        v += adjust.shadows * 0.5 * shadow_weight * shadow_weight;
        v += adjust.highlights * 0.5 * highlight_weight * highlight_weight;
        v = (v - 0.5) * contrast + 0.5;
        *slot = (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    }
    skia_safe::color_filters::table_argb(None, Some(&table), Some(&table), Some(&table))
}

/// A colour matrix for saturation (luma-weighted) plus temperature / tint
/// per-channel white-balance scaling. `None` when all three are zero.
fn chroma_matrix_filter(adjust: &ImageAdjust) -> Option<ColorFilter> {
    if adjust.saturation == 0.0 && adjust.temperature == 0.0 && adjust.tint == 0.0 {
        return None;
    }
    let s = 1.0 + adjust.saturation;
    // Rec. 709 luma weights — the greyscale a fully-desaturated pixel collapses to.
    let (lr, lg, lb) = (0.213f32, 0.715, 0.072);
    let (ir, ig, ib) = ((1.0 - s) * lr, (1.0 - s) * lg, (1.0 - s) * lb);
    let mut m = [
        ir + s,
        ig,
        ib,
        0.0,
        0.0, //
        ir,
        ig + s,
        ib,
        0.0,
        0.0, //
        ir,
        ig,
        ib + s,
        0.0,
        0.0, //
        0.0,
        0.0,
        0.0,
        1.0,
        0.0, //
    ];
    // White balance: warm/cool scales R vs B, tint scales G. Applied by scaling
    // each output row (post-saturation) by its channel gain.
    let k = 0.5;
    let r_gain = 1.0 + adjust.temperature * k;
    let b_gain = 1.0 - adjust.temperature * k;
    let g_gain = 1.0 + adjust.tint * k;
    for value in &mut m[0..5] {
        *value *= r_gain;
    }
    for value in &mut m[5..10] {
        *value *= g_gain;
    }
    for value in &mut m[10..15] {
        *value *= b_gain;
    }
    Some(skia_safe::color_filters::matrix_row_major(
        &m,
        skia_safe::color_filters::Clamp::Yes,
    ))
}

/// Draw a decoded image into the node-local rect `[0, 0, local_size]`, honouring
/// `fit`, `crop`, and `tint`. Returns `true` if it drew the real image, `false`
/// if the image could not be built (caller draws the placeholder).
///
/// A whole-image draw that lands SMALLER than its pixels (zoomed out) samples
/// trilinearly through the image's mip chain so it averages instead of
/// aliasing; magnified and cropped draws keep Skia's strict-constraint
/// sampling (see `blit_sk_image`).
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
        canvas,
        &sk_image,
        img.width,
        img.height,
        local_size,
        crop,
        fit,
        tint,
        opacity,
        ImageFillMods::default(),
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
    mods: ImageFillMods,
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
    // Zoom-out LOD: a minified draw samples the display-pyramid level whose
    // texel density still meets the device's instead of the full upload (see
    // `CachedImage`). The crop/fit math is normalized, so the level's own
    // dimensions stand in for the natural size below.
    let level = draw_device_scale(canvas, nat_w, nat_h, local_size, crop, fit, mods)
        .map_or(0, minification_level);
    let (sk_image, nat_w, nat_h) = match level {
        0 => (sk_image, nat_w, nat_h),
        level => match cache.lod_level(id, level) {
            Some(lod) => {
                let (w, h) = (lod.width() as u32, lod.height() as u32);
                (lod, w, h)
            }
            None => (sk_image, nat_w, nat_h),
        },
    };
    blit_sk_image(
        canvas, &sk_image, nat_w, nat_h, local_size, crop, fit, tint, opacity, mods,
    )
}

/// Device pixels per SOURCE texel for a draw of a `nat_w × nat_h` image into
/// the node-local `local_size` rect under `fit` / `crop` / `mods` on `canvas`:
/// the CTM's span of one local unit times this blit's dst/src stretch — the
/// same quantity [`blit_sk_image`] derives to pick mip sampling, computed up
/// front so [`draw_image_cached`] can choose a pyramid level. `None` for a
/// tiled draw (its tiles are sized from the natural resolution, so it always
/// samples the full image) or a degenerate destination.
fn draw_device_scale(
    canvas: &Canvas,
    nat_w: u32,
    nat_h: u32,
    local_size: [f64; 2],
    crop: Option<[f32; 4]>,
    fit: ImageFitMode,
    mods: ImageFillMods,
) -> Option<f32> {
    if fit == ImageFitMode::Tile {
        return None;
    }
    // A quarter-turn rotation fits against the swapped frame (see
    // `blit_sk_image`); the CTM's scale magnitude is rotation-invariant.
    let size = if quarter_turns(mods.rotation) % 2 == 1 {
        [local_size[1], local_size[0]]
    } else {
        local_size
    };
    let (dst_w, dst_h) = (size[0] as f32, size[1] as f32);
    if dst_w <= 0.0 || dst_h <= 0.0 || dst_w.is_nan() || dst_h.is_nan() {
        return None;
    }
    let [_, _, crop_w, crop_h] = crop_to_pixels(crop, nat_w as f32, nat_h as f32);
    let rects = fit_src_dst(fit, crop_w, crop_h, dst_w, dst_h);
    let stretch = if rects.src[2] > 0.0 && rects.src[3] > 0.0 {
        (rects.dst[2] / rects.src[2]).max(rects.dst[3] / rects.src[3])
    } else {
        1.0
    };
    Some(max_device_scale(canvas) * stretch)
}

/// Draw an already-built [`skia_safe::Image`] into the node-local rect
/// `[0, 0, local_size]`, honouring `fit`, `crop`, and `tint`. The source-pixel
/// dimensions (`nat_w`/`nat_h`) come from the [`DecodedImage`] the image was
/// built from — they drive crop math and must match the uploaded pixels.
///
/// Factored out of [`draw_decoded_image`] so the cached and uncached entry
/// points share one body and cannot drift apart.
///
/// A quarter-turn `mods.rotation` (Figma rotates image fills in 90° steps) is
/// realized here as a canvas rotation mapping the rotated frame onto the dst
/// rect — the fit then runs against the frame's SWAPPED dimensions for 90/270°
/// so cover/contain crops exactly like Figma's rotated fill.
#[allow(clippy::too_many_arguments)]
pub(crate) fn blit_sk_image(
    canvas: &Canvas,
    sk_image: &skia_safe::Image,
    nat_w: u32,
    nat_h: u32,
    local_size: [f64; 2],
    crop: Option<[f32; 4]>,
    fit: ImageFitMode,
    tint: Option<Color>,
    opacity: f32,
    mods: ImageFillMods,
) -> bool {
    let quarter = quarter_turns(mods.rotation);
    if quarter != 0 {
        let (w, h) = (local_size[0] as f32, local_size[1] as f32);
        canvas.save();
        // Map the rotated drawing frame onto [0, 0, w, h]: rotate clockwise
        // (y-down), then translate the rotated frame's origin back into the
        // rect. 90°/270° swap the frame's width and height.
        match quarter {
            1 => {
                canvas.translate((w, 0.0));
                canvas.rotate(90.0, None);
            }
            2 => {
                canvas.translate((w, h));
                canvas.rotate(180.0, None);
            }
            _ => {
                canvas.translate((0.0, h));
                canvas.rotate(270.0, None);
            }
        }
        let rotated_size = if quarter % 2 == 1 {
            [local_size[1], local_size[0]]
        } else {
            local_size
        };
        let drawn = blit_sk_image(
            canvas,
            sk_image,
            nat_w,
            nat_h,
            rotated_size,
            crop,
            fit,
            tint,
            opacity,
            ImageFillMods {
                rotation: None,
                ..mods
            },
        );
        canvas.restore();
        return drawn;
    }

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
    let paint = tinted_paint(tint, opacity, mods.blend, &mods.adjust);

    if fit == ImageFitMode::Tile {
        // Trilinear when the CTM minifies the native-size tiles (zoomed out),
        // so a repeated texture doesn't shimmer; the plain linear tile
        // sampling is kept at magnification, where mip level 0 is all a
        // trilinear lookup would read anyway.
        let sampling = if sk_image.has_mipmaps() && max_device_scale(canvas) < 1.0 {
            SamplingOptions::new(FilterMode::Linear, MipmapMode::Linear)
        } else {
            SamplingOptions::from(FilterMode::Linear)
        };
        draw_tiled(
            canvas,
            sk_image,
            [crop_x, crop_y, crop_w, crop_h],
            [dst_w, dst_h],
            sampling,
            &paint,
            mods.scale,
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

    // The effective image→device scale: the CTM's span of a local unit times
    // the dst/src stretch of this blit.
    let stretch = if rects.src[2] > 0.0 && rects.src[3] > 0.0 {
        (rects.dst[2] / rects.src[2]).max(rects.dst[3] / rects.src[3])
    } else {
        1.0
    };
    let device_scale = max_device_scale(canvas) * stretch;
    // Whether this draw samples the ENTIRE image (no crop, no cover
    // centre-crop). Exact float compares are fine: the whole-image case is
    // produced by identity arithmetic in `crop_to_pixels`/`fit_src_dst`, and a
    // false negative merely keeps the conservative path.
    let src_is_whole_image = src == Rect::from_wh(nat_w as f32, nat_h as f32);

    if src_is_whole_image && sk_image.has_mipmaps() && device_scale < 1.0 {
        // Minified whole-image draw: sample trilinearly so a zoomed-out photo
        // averages its pixels instead of aliasing. Skia's raster pipeline
        // ignores mip levels under a Strict constraint (verified empirically),
        // and Fast is only safe when there is no sub-rect to bleed across —
        // which is exactly this whole-image case (the image edge clamps).
        canvas.draw_image_rect_with_sampling_options(
            sk_image,
            Some((&src, SrcRectConstraint::Fast)),
            dst,
            SamplingOptions::new(FilterMode::Linear, MipmapMode::Linear),
            &paint,
        );
        return true;
    }
    // Strict constraint keeps Fill's centre-crop from bleeding neighbouring
    // texels at the cropped edge. Skia's raster pipeline ignores mip levels
    // under Strict, so a minified cropped draw samples the base level: with
    // the display pyramid feeding this path a level within 2× of the device
    // resolution (see `draw_image_cached`), bilinear filtering averages the
    // ≤2×2 texels under each device pixel instead of point-sampling one of
    // them (the nearest default), which is what keeps a zoomed-out cover-fit
    // photo from sparkling as it pans. Magnified / 1:1 draws keep the default
    // (nearest) sampling they always had.
    let sampling = if device_scale < 1.0 {
        SamplingOptions::from(FilterMode::Linear)
    } else {
        SamplingOptions::default()
    };
    canvas.draw_image_rect_with_sampling_options(
        sk_image,
        Some((&src, SrcRectConstraint::Strict)),
        dst,
        sampling,
        &paint,
    );
    true
}

/// Snap an optional rotation (degrees clockwise) to whole quarter turns in
/// `0..=3`. Figma authors image-fill rotation in 90° steps; snapping keeps a
/// slightly-off import faithful and a `None`/0° fill on the untouched path.
fn quarter_turns(rotation: Option<f32>) -> i32 {
    let Some(deg) = rotation else {
        return 0;
    };
    if !deg.is_finite() {
        return 0;
    }
    ((deg / 90.0).round() as i32).rem_euclid(4)
}

/// Tile the (cropped) image across the local rect via a repeating shader, each
/// tile at `natural × scale` pixels (`None` scale = native size — Figma's
/// `scalingFactor`). The image's own pixels start at the rect origin and repeat
/// in both axes; the draw is clipped to the local rect.
fn draw_tiled(
    canvas: &Canvas,
    sk_image: &skia_safe::Image,
    crop_px: [f32; 4],
    dst: [f32; 2],
    sampling: SamplingOptions,
    base_paint: &Paint,
    scale: Option<f32>,
) {
    let [cx, cy, cw, ch] = crop_px;
    // A local matrix translates the shader so the cropped region's top-left
    // lands at the rect origin, scaled so one tile spans `natural × scale`;
    // TileMode::Repeat handles the wrap. Degenerate scales fall back to 1.0.
    let tile_scale = scale.filter(|s| s.is_finite() && *s > 0.0).unwrap_or(1.0);
    let mut local = Matrix::scale((tile_scale, tile_scale));
    local.pre_translate((-cx, -cy));
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
    fn cached_image_carries_mipmaps_for_minified_sampling() {
        let mut cache = ImageCache::new();
        let id = AssetId::new();
        let img = solid_image(4, 4, [10, 20, 30, 255]);
        let has_mips = cache
            .get_or_build(id, &img)
            .expect("well-formed builds")
            .has_mipmaps();
        assert!(has_mips, "the cache should attach a default mip chain");
    }

    #[test]
    fn minified_whole_image_draw_samples_through_the_mip_chain() {
        // 64x64 checker of 2px black/white squares: a trilinear lookup at ~1/9
        // scale reads a deep mip level, which has averaged to uniform mid-gray;
        // base-level sampling would keep near-pure black/white pixels (the
        // aliasing this path exists to remove).
        let size = 64u32;
        let mut px = Vec::with_capacity((size * size * 4) as usize);
        for y in 0..size {
            for x in 0..size {
                let on = ((x / 2) + (y / 2)) % 2 == 0;
                let v = if on { 255 } else { 0 };
                px.extend_from_slice(&[v, v, v, 255]);
            }
        }
        let img = DecodedImage::new(Arc::new(px), size, size);

        let dst = 7;
        let mut surface = skia_safe::surfaces::raster_n32_premul((dst, dst)).expect("surface");
        let canvas = surface.canvas();
        canvas.scale((dst as f32 / size as f32, dst as f32 / size as f32));
        assert!(draw_decoded_image(
            canvas,
            &img,
            [size as f64, size as f64],
            None,
            ImageFitMode::Stretch,
            None,
            1.0,
        ));

        let info = ImageInfo::new((dst, dst), ColorType::RGBA8888, AlphaType::Unpremul, None);
        let row_bytes = info.min_row_bytes();
        let mut buf = vec![0u8; row_bytes * dst as usize];
        assert!(surface.read_pixels(&info, &mut buf, row_bytes, (0, 0)));
        for i in 0..(dst * dst) as usize {
            let r = buf[i * 4];
            assert!(
                (64..=192).contains(&r),
                "pixel {i} should be an averaged mid-gray, got {r}"
            );
        }
    }

    // -------------------------------------------------------------------
    // Zoom-out LOD: the display pyramid
    // -------------------------------------------------------------------

    #[test]
    fn minification_level_is_the_sharpen_biased_upper_mip_level() {
        // Above 2^-1.5 ≈ 0.354: the full image (magnified, 1:1, mild
        // minification — Skia's biased lookup still reads the base level).
        for s in [4.0f32, 1.0, 0.75, 0.5, 0.36] {
            assert_eq!(minification_level(s), 0, "scale {s}");
        }
        // 2^-1.5 → level 1 exactly; the app's 10% zoom on Retina (0.2) → 1.
        assert_eq!(minification_level(0.35), 1);
        assert_eq!(minification_level(0.2), 1);
        assert_eq!(minification_level(0.18), 1);
        // 2^-2.5 ≈ 0.177 → level 2; 10% zoom on a 1× display → 2.
        assert_eq!(minification_level(0.17), 2);
        assert_eq!(minification_level(0.1), 2);
        assert_eq!(minification_level(0.08), 3);
        assert_eq!(minification_level(0.02), 5);
        // Level k is never deeper than the geometric level floor(log2(1/s)),
        // so an unbiased (GPU hardware) sampler reading the level's chain
        // still finds every level it wants.
        for s in [0.49f32, 0.3, 0.2, 0.13, 0.07, 0.01] {
            let k = minification_level(s);
            assert!(
                k <= (1.0 / s).log2().floor() as u32,
                "scale {s} → level {k}"
            );
        }
        // Degenerate scales never touch the pyramid.
        assert_eq!(minification_level(0.0), 0);
        assert_eq!(minification_level(-1.0), 0);
        assert_eq!(minification_level(f32::NAN), 0);
    }

    #[test]
    fn pyramid_levels_are_built_lazily_halved_and_clamped_to_the_floor() {
        let mut cache = ImageCache::new();
        let id = AssetId::new();
        cache.get_or_build(id, &solid_image(40, 24, [10, 20, 30, 255]));
        let full_uid = cache.get(id).unwrap().unique_id();
        assert!(
            cache.images[&id].levels.is_empty(),
            "no level built up front"
        );

        // Level 0 is the full upload itself (same SkImage), builds nothing.
        assert_eq!(cache.lod_level(id, 0).unwrap().unique_id(), full_uid);
        assert!(cache.images[&id].levels.is_empty());

        // Level 2 builds levels 1 and 2 (contiguous), each halved.
        let l2 = cache.lod_level(id, 2).unwrap();
        assert_eq!((l2.width(), l2.height()), (10, 6));
        assert_eq!(cache.images[&id].levels.len(), 2);
        let l1 = cache.lod_level(id, 1).unwrap();
        assert_eq!((l1.width(), l1.height()), (20, 12));
        assert!(
            l1.has_mipmaps() && l2.has_mipmaps(),
            "levels carry their own mip chain"
        );
        // A repeat request is a hit: same handle, nothing rebuilt.
        assert_eq!(cache.lod_level(id, 2).unwrap().unique_id(), l2.unique_id());
        assert_eq!(cache.images[&id].levels.len(), 2);

        // Deeper than the pyramid can go: 40x24 → 20x12 → 10x6 → 5x3 → 2x1 →
        // floor; asking for level 9 yields the deepest that exists.
        let deepest = cache.lod_level(id, 9).unwrap();
        assert_eq!((deepest.width(), deepest.height()), (2, 1));
        assert_eq!(cache.images[&id].levels.len(), 4);
        // Unknown asset: nothing to level.
        assert!(cache.lod_level(AssetId::new(), 1).is_none());
        // Cache maintenance still counts assets, not levels.
        assert_eq!(cache.len(), 1);
    }

    /// A `size × size` image whose colour varies smoothly across it (a
    /// two-axis gradient), so a downscaled draw has real texel averaging to do.
    fn gradient_image(size: u32) -> DecodedImage {
        let mut px = Vec::with_capacity((size * size * 4) as usize);
        for y in 0..size {
            for x in 0..size {
                let r = (x * 255 / (size - 1)) as u8;
                let g = (y * 255 / (size - 1)) as u8;
                let b = (((x + y) * 255) / (2 * (size - 1))) as u8;
                px.extend_from_slice(&[r, g, b, 255]);
            }
        }
        DecodedImage::new(Arc::new(px), size, size)
    }

    /// Draw `img` (asset `id`, cached in `cache` if `Some`, else the uncached
    /// full-resolution path) into a `dst × dst` surface at CTM scale
    /// `dst / size` with `fit` + `crop`, returning the RGBA bytes.
    fn render_minified(
        cache: Option<&mut ImageCache>,
        id: AssetId,
        img: &DecodedImage,
        dst: i32,
        fit: ImageFitMode,
        crop: Option<[f32; 4]>,
        local: [f64; 2],
    ) -> Vec<u8> {
        let mut surface = skia_safe::surfaces::raster_n32_premul((dst, dst)).expect("surface");
        let canvas = surface.canvas();
        canvas.clear(skia_safe::Color::WHITE);
        canvas.scale((dst as f32 / local[0] as f32, dst as f32 / local[1] as f32));
        let drawn = match cache {
            Some(cache) => draw_image_cached(
                canvas,
                cache,
                id,
                || Some(img.clone()),
                local,
                crop,
                fit,
                None,
                1.0,
                ImageFillMods::default(),
            ),
            None => draw_decoded_image(canvas, img, local, crop, fit, None, 1.0),
        };
        assert!(drawn);
        let info = ImageInfo::new((dst, dst), ColorType::RGBA8888, AlphaType::Unpremul, None);
        let row_bytes = info.min_row_bytes();
        let mut buf = vec![0u8; row_bytes * dst as usize];
        assert!(surface.read_pixels(&info, &mut buf, row_bytes, (0, 0)));
        buf
    }

    /// FIDELITY OF THE PYRAMID (the LOD gate): whole-image draws minified to
    /// 1/5 (the app's 10% zoom on Retina → level 1) and 1/10 (10% on a 1×
    /// display → level 2) of the asset through the cache — sampling the
    /// pyramid — match the full-resolution trilinear draw within
    /// `IMAGE_LOD_MAX_DIFF_LSB` on every pixel, on both a smooth gradient and
    /// a 2-px checker (the aliasing case). A level is the full image's own
    /// mip level and its chain continues the full chain, so the lookup reads
    /// the same texels at the same lerp weight (see `minification_level`).
    #[test]
    fn pyramid_level_draw_matches_the_full_resolution_trilinear_draw() {
        /// The pyramid draw is the same texels and lerp as the full draw; the
        /// only slack is float rounding of the level fraction. Measured 0 LSB;
        /// 1 LSB is the contract this test enforces (invisible at 8 bits).
        const IMAGE_LOD_MAX_DIFF_LSB: u8 = 1;
        let size = 160u32;
        for (dst, level) in [(32, 1u32), (16, 2)] {
            for (name, img) in [
                ("gradient", gradient_image(size)),
                ("checker", {
                    let mut px = Vec::with_capacity((size * size * 4) as usize);
                    for y in 0..size {
                        for x in 0..size {
                            let v = if ((x / 2) + (y / 2)) % 2 == 0 { 255 } else { 0 };
                            px.extend_from_slice(&[v, v, v, 255]);
                        }
                    }
                    DecodedImage::new(Arc::new(px), size, size)
                }),
            ] {
                let id = AssetId::new();
                let mut cache = ImageCache::new();
                let local = [size as f64, size as f64];
                let lod = render_minified(
                    Some(&mut cache),
                    id,
                    &img,
                    dst,
                    ImageFitMode::Stretch,
                    None,
                    local,
                );
                assert_eq!(
                    cache.images[&id].levels.len() as u32,
                    level,
                    "{name}: a {}× draw must build and use pyramid level {level}",
                    dst as f32 / size as f32
                );
                let full = render_minified(None, id, &img, dst, ImageFitMode::Stretch, None, local);
                let worst = lod
                    .iter()
                    .zip(&full)
                    .map(|(a, b)| a.abs_diff(*b))
                    .max()
                    .unwrap();
                assert!(
                    worst <= IMAGE_LOD_MAX_DIFF_LSB,
                    "{name} @ level {level}: pyramid draw must match the full trilinear draw \
                 within {IMAGE_LOD_MAX_DIFF_LSB} LSB, worst pixel differs by {worst}"
                );
            }
        }
    }

    /// A cover-fit (centre-cropped) draw — Skia's Strict-constraint path,
    /// which cannot use mips — sampled from the pyramid keeps the crop
    /// geometry: on a two-tone image the tone boundary lands on the same
    /// device column as the full-resolution draw (±1 px of AA), and every
    /// pixel away from it is exact. Also pins that the level is what got
    /// sampled.
    #[test]
    fn pyramid_cropped_draw_keeps_the_crop_geometry() {
        let (w, h) = (200u32, 100u32);
        let mut px = Vec::with_capacity((w * h * 4) as usize);
        for _y in 0..h {
            for x in 0..w {
                // Left 60% red, right 40% blue (boundary at x = 120 of 200).
                let c = if x < 120 {
                    [255, 0, 0, 255]
                } else {
                    [0, 0, 255, 255]
                };
                px.extend_from_slice(&c);
            }
        }
        let img = DecodedImage::new(Arc::new(px), w, h);
        // Cover-fit into a 100×100 local square: the crop takes the middle
        // 100 px of the 200 wide asset (x 50..150), so the boundary sits at
        // 70% of the destination. Drawn at 0.2×: a 20×20 device square, the
        // boundary at device x = 14.
        let local = [100.0, 100.0];
        let dst = 20;
        let id = AssetId::new();
        let mut cache = ImageCache::new();
        let lod = render_minified(
            Some(&mut cache),
            id,
            &img,
            dst,
            ImageFitMode::Fill,
            None,
            local,
        );
        assert_eq!(
            cache.images[&id].levels.len(),
            1,
            "0.2× → level 1 was built"
        );
        let full = render_minified(None, id, &img, dst, ImageFitMode::Fill, None, local);
        let px_at = |buf: &[u8], x: usize, y: usize| {
            let i = (y * dst as usize + x) * 4;
            [buf[i], buf[i + 1], buf[i + 2], buf[i + 3]]
        };
        for y in 0..dst as usize {
            for x in 0..dst as usize {
                let (a, b) = (px_at(&lod, x, y), px_at(&full, x, y));
                if (13..=14).contains(&x) {
                    // The AA'd boundary column: red or blue or a mix of the two.
                    assert!(a[1] == 0 && a[3] == 255, "boundary pixel ({x},{y}) = {a:?}");
                    continue;
                }
                assert_eq!(a, b, "pixel ({x},{y}) away from the boundary must be exact");
                assert_eq!(
                    a,
                    if x < 13 {
                        [255, 0, 0, 255]
                    } else {
                        [0, 0, 255, 255]
                    }
                );
            }
        }
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
