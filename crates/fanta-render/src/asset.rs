//! Asset resolution — the seam between `fanta-render` and whoever owns the
//! decoded image bytes (the app's in-memory cache, `fanta-format`'s `.fant`
//! store, a node-graph's `ImageHandle`, …).
//!
//! ## Why a trait instead of a concrete store
//!
//! `fanta-render` must not depend on `fanta-format`: the dependency direction
//! is doc → render, and reversing it (render → format) would drag the zip /
//! sha256 / codec machinery into every consumer of the renderer and make the
//! "swap the renderer" promise (lib.rs) a lie. So the renderer declares the
//! one capability it needs — "turn this [`AssetId`] into pixels Skia can
//! draw" — as [`AssetResolver`], and the app wires the concrete impl. Tests
//! inject [`InMemoryAssetResolver`] and never touch a file. See
//! `specs/03-media-3d-and-node-workflows.md` §1.
//!
//! ## Why straight (un-premultiplied) alpha
//!
//! [`DecodedImage::pixels_rgba`] is straight-alpha RGBA8 to match
//! [`RasterRenderer::copy_rgba`] (which reads back as `Unpremul`) and the
//! node-graph contract (`RemoveBackground` preserves RGB under a zeroed
//! alpha channel — only possible with straight alpha). Skia premultiplies on
//! upload; the buffer stays canonical so the same bytes round-trip through
//! decode → render → readback without a colour shift.
//!
//! ## Why dependency-light
//!
//! No image-decoding crate lives here. Callers hand in *already-decoded* RGBA
//! (PNG/JPG decode is owned by the app / `fanta-format`). That keeps this
//! crate's compile graph small and lets the node-graph path — whose bytes are
//! already decoded `ImageHandle` RGBA8 — skip a redundant decode entirely.

use fanta_doc::AssetId;
use std::collections::HashMap;
use std::sync::Arc;

/// Decoded, ready-to-upload image pixels.
///
/// `pixels_rgba` is straight-alpha RGBA8, row-major, with
/// `len == width * height * 4`. The buffer is behind an [`Arc`] so the cache,
/// the resolver, and the Skia image can share one allocation rather than
/// copying megabytes per frame; it also gives the renderer a cheap identity
/// (the `Arc` pointer) to key a future `SkImage` upload cache on, so a re-roll
/// that swaps the buffer invalidates the stale GPU texture for free.
#[derive(Clone)]
pub struct DecodedImage {
    /// Straight-alpha RGBA8, row-major. `len == width * height * 4`.
    pub pixels_rgba: Arc<Vec<u8>>,
    pub width: u32,
    pub height: u32,
}

impl DecodedImage {
    /// Construct from raw parts. Convenience for callers and tests; does not
    /// validate `len` (the renderer tolerates a malformed buffer by falling
    /// back to the placeholder rather than panicking).
    pub fn new(pixels_rgba: Arc<Vec<u8>>, width: u32, height: u32) -> Self {
        Self {
            pixels_rgba,
            width,
            height,
        }
    }

    /// Whether the buffer length matches `width * height * 4`. The renderer
    /// checks this before handing the bytes to Skia: a mismatched buffer would
    /// otherwise read out of bounds inside Skia, so a bad asset degrades to the
    /// placeholder instead of unsafety.
    pub fn is_well_formed(&self) -> bool {
        self.pixels_rgba.len() == (self.width as usize) * (self.height as usize) * 4
    }
}

/// Resolves an [`AssetId`] to decoded pixels.
///
/// Implemented by the app over a `FantaFile`; faked in renderer tests. The
/// renderer never touches `fanta-format` directly — it only knows this trait.
///
/// `resolve` is **non-blocking** by contract: a cache miss should kick off a
/// background decode and return `None` *this* frame (the node draws a
/// placeholder), with the pixels available on a later frame. Returning `None`
/// for an unknown or still-decoding asset is the single deferred-output
/// pattern the rest of the media pipeline (video frames, waveforms, AI
/// outputs) reuses — bitmap is its simplest instance.
///
/// `Send + Sync` so the resolver can be shared across the render thread and a
/// background decode pool behind an [`Arc`].
pub trait AssetResolver: Send + Sync {
    /// Decoded RGBA8 pixels for `id`, or `None` if unknown / still decoding.
    fn resolve(&self, id: AssetId) -> Option<DecodedImage>;

    /// Raw, undecoded source bytes for `id` (WAV / glTF / MP4 / …), or `None`.
    /// Used by the non-image visualizers (audio waveform, 3D, video) that parse
    /// the source directly. Defaults to `None` so image-only resolvers — and
    /// renderer tests — need not implement it.
    fn resolve_bytes(&self, _id: AssetId) -> Option<std::sync::Arc<Vec<u8>>> {
        None
    }
}

/// A trivial [`AssetResolver`] backed by an in-memory map.
///
/// Two roles: the substrate of the renderer's bitmap tests (register a known
/// buffer, assert the rendered pixels) and a usable hot cache for the app
/// before the `.fant`-backed LRU lands. Cloning a stored [`DecodedImage`] is
/// cheap — only the `Arc` refcount bumps, never the pixel buffer.
#[derive(Default)]
pub struct InMemoryAssetResolver {
    images: HashMap<AssetId, DecodedImage>,
}

impl InMemoryAssetResolver {
    /// Empty resolver.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register (or replace) the decoded image for `id`. Returns the previous
    /// entry if one was present, mirroring [`HashMap::insert`].
    pub fn insert(&mut self, id: AssetId, image: DecodedImage) -> Option<DecodedImage> {
        self.images.insert(id, image)
    }

    /// Number of registered assets. Handy in tests.
    pub fn len(&self) -> usize {
        self.images.len()
    }

    /// Whether the resolver holds no assets.
    pub fn is_empty(&self) -> bool {
        self.images.is_empty()
    }
}

impl AssetResolver for InMemoryAssetResolver {
    fn resolve(&self, id: AssetId) -> Option<DecodedImage> {
        // Clone is an Arc bump on the pixel buffer plus two u32s — the whole
        // point of the Arc is that this stays O(1) regardless of image size.
        self.images.get(&id).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(w: u32, h: u32, rgba: [u8; 4]) -> DecodedImage {
        let mut px = Vec::with_capacity((w * h * 4) as usize);
        for _ in 0..(w * h) {
            px.extend_from_slice(&rgba);
        }
        DecodedImage::new(Arc::new(px), w, h)
    }

    #[test]
    fn insert_then_resolve_round_trips_the_same_buffer() {
        let mut r = InMemoryAssetResolver::new();
        let id = AssetId::new();
        let img = solid(4, 4, [255, 0, 0, 255]);
        let original_ptr = Arc::as_ptr(&img.pixels_rgba);
        assert!(r.insert(id, img).is_none());

        let got = r.resolve(id).expect("registered asset resolves");
        assert_eq!(got.width, 4);
        assert_eq!(got.height, 4);
        // The resolve clone shares the Arc — no pixel copy.
        assert_eq!(Arc::as_ptr(&got.pixels_rgba), original_ptr);
        assert!(got.is_well_formed());
    }

    #[test]
    fn unknown_asset_resolves_to_none() {
        let r = InMemoryAssetResolver::new();
        assert!(r.resolve(AssetId::new()).is_none());
    }

    #[test]
    fn insert_replaces_and_returns_previous() {
        let mut r = InMemoryAssetResolver::new();
        let id = AssetId::new();
        r.insert(id, solid(2, 2, [1, 2, 3, 4]));
        let prev = r.insert(id, solid(2, 2, [9, 9, 9, 9]));
        assert!(prev.is_some(), "replacing returns the old entry");
        assert_eq!(r.len(), 1, "replacing does not grow the map");
        let now = r.resolve(id).unwrap();
        assert_eq!(now.pixels_rgba[0], 9);
    }

    #[test]
    fn well_formed_detects_length_mismatch() {
        let good = solid(2, 2, [0, 0, 0, 255]);
        assert!(good.is_well_formed());
        let bad = DecodedImage::new(Arc::new(vec![0u8; 3]), 2, 2);
        assert!(!bad.is_well_formed());
    }

    #[test]
    fn empty_resolver_reports_empty() {
        let r = InMemoryAssetResolver::new();
        assert!(r.is_empty());
        assert_eq!(r.len(), 0);
    }
}
