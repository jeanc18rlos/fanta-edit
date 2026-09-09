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
//! [`LazyAssetResolver`] keeps that rule too: it owns the *encoded* bytes and
//! the decoded-pixel budget, but the decode itself is a function the app
//! injects.

use fanta_doc::AssetId;
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, PoisonError};

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

/// Default cap on decoded RGBA held by a [`LazyAssetResolver`]: 1.5 GiB.
/// Above it the least-recently-resolved images are dropped and re-decoded on
/// their next use.
pub const DEFAULT_DECODED_BUDGET_BYTES: usize = 1536 * 1024 * 1024;

/// Turns one asset's encoded bytes into straight-alpha RGBA8, or `None` when
/// the bytes are not a decodable image (the resolver remembers the failure so
/// a corrupt asset is not re-attempted on every frame). The app supplies this
/// — see the module docs on why no decoder lives in this crate.
pub type ImageDecoder = dyn Fn(AssetId, &[u8]) -> Option<DecodedImage> + Send + Sync;

/// An [`AssetResolver`] over a document's encoded assets that decodes each
/// image the first time something asks for it and keeps the decoded pixels
/// under a byte budget.
///
/// Eagerly decoding every embedded asset at load was the bulk of a large
/// document's resident memory (a 10 MB `.fig` settled at several GiB of
/// RGBA), most of it for images on pages the user never opened. Here only
/// the assets actually drawn — plus whatever the loader [`prewarm`]s — cost
/// pixels; the encoded bytes stay shared with the save path.
///
/// Shared between the render thread and the UI thread (screenshots, export),
/// so it is `Send + Sync`, and the lock is never held across a decode: a
/// miss decodes outside the lock and inserts afterwards (a concurrent decode
/// of the same asset keeps whichever landed first).
///
/// [`prewarm`]: Self::prewarm
pub struct LazyAssetResolver {
    encoded: Arc<BTreeMap<AssetId, Vec<u8>>>,
    decoder: Arc<ImageDecoder>,
    budget_bytes: usize,
    cache: Mutex<DecodedCache>,
}

#[derive(Default)]
struct DecodedCache {
    entries: HashMap<AssetId, CacheEntry>,
    /// Recency order: tick → asset. The smallest tick is the least recently
    /// used entry; every hit re-keys its asset under a fresh tick.
    recency: BTreeMap<u64, AssetId>,
    next_tick: u64,
    /// Bytes held by successfully decoded entries (failures weigh nothing).
    decoded_bytes: usize,
    /// Encoded bytes already handed out through [`AssetResolver::resolve_bytes`],
    /// so a caller asking every frame (the audio waveform painter) gets a
    /// refcount bump, not a copy of the whole file. Not budgeted: each entry is
    /// the one copy of an asset the document already holds.
    shared_bytes: HashMap<AssetId, Arc<Vec<u8>>>,
}

struct CacheEntry {
    /// `None` records a decode failure so the asset is not retried per frame.
    image: Option<DecodedImage>,
    tick: u64,
}

impl DecodedCache {
    fn touch(&mut self, id: AssetId) -> Option<Option<DecodedImage>> {
        let entry = self.entries.get_mut(&id)?;
        self.recency.remove(&entry.tick);
        entry.tick = self.next_tick;
        self.next_tick += 1;
        self.recency.insert(entry.tick, id);
        Some(entry.image.clone())
    }

    fn insert(&mut self, id: AssetId, image: Option<DecodedImage>, budget_bytes: usize) {
        if self.entries.contains_key(&id) {
            return;
        }
        let tick = self.next_tick;
        self.next_tick += 1;
        self.decoded_bytes += image.as_ref().map_or(0, |image| image.pixels_rgba.len());
        self.entries.insert(id, CacheEntry { image, tick });
        self.recency.insert(tick, id);
        // The entry just inserted is the most recent and is never evicted
        // here, even if it alone exceeds the budget: an image the renderer
        // needs right now that can never be cached would otherwise be
        // re-decoded on every single frame.
        while self.decoded_bytes > budget_bytes {
            let Some((&oldest_tick, &oldest)) = self.recency.iter().next() else {
                break;
            };
            if oldest == id {
                break;
            }
            self.recency.remove(&oldest_tick);
            if let Some(evicted) = self.entries.remove(&oldest) {
                self.decoded_bytes -= evicted
                    .image
                    .as_ref()
                    .map_or(0, |image| image.pixels_rgba.len());
            }
        }
    }
}

impl LazyAssetResolver {
    /// Resolver over `encoded` with the [`DEFAULT_DECODED_BUDGET_BYTES`] cap.
    pub fn new(encoded: Arc<BTreeMap<AssetId, Vec<u8>>>, decoder: Arc<ImageDecoder>) -> Self {
        Self::with_budget(encoded, decoder, DEFAULT_DECODED_BUDGET_BYTES)
    }

    /// Resolver over `encoded` keeping at most `budget_bytes` of decoded RGBA
    /// (the most recently used image is always kept, budget or not).
    pub fn with_budget(
        encoded: Arc<BTreeMap<AssetId, Vec<u8>>>,
        decoder: Arc<ImageDecoder>,
        budget_bytes: usize,
    ) -> Self {
        Self {
            encoded,
            decoder,
            budget_bytes,
            cache: Mutex::new(DecodedCache::default()),
        }
    }

    /// Decode `ids` now (skipping ones already decoded or unknown), so the
    /// first frame that draws them does not pay the decode. Meant for a
    /// background load task, e.g. with the assets of the page that opens.
    pub fn prewarm(&self, ids: impl IntoIterator<Item = AssetId>) {
        for id in ids {
            let cached = self.lock().entries.contains_key(&id);
            if cached {
                continue;
            }
            self.decode_and_insert(id);
        }
    }

    /// Bytes of decoded RGBA currently held.
    pub fn decoded_bytes(&self) -> usize {
        self.lock().decoded_bytes
    }

    /// The decoded-pixel cap this resolver evicts down to.
    pub fn budget_bytes(&self) -> usize {
        self.budget_bytes
    }

    /// Number of encoded assets this resolver knows about.
    pub fn len(&self) -> usize {
        self.encoded.len()
    }

    /// Whether there are no encoded assets at all.
    pub fn is_empty(&self) -> bool {
        self.encoded.is_empty()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, DecodedCache> {
        // The cache holds no invariant a panicking holder could break
        // half-way (each mutation is a single insert/evict step), so a
        // poisoned lock is still safe to keep using.
        self.cache.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn decode_and_insert(&self, id: AssetId) -> Option<DecodedImage> {
        let bytes = self.encoded.get(&id)?;
        let image = (self.decoder)(id, bytes);
        let mut cache = self.lock();
        cache.insert(id, image, self.budget_bytes);
        // Another thread may have raced us to the same asset; serve the
        // entry that won so both callers hold the same allocation.
        cache.touch(id).flatten()
    }
}

impl AssetResolver for LazyAssetResolver {
    fn resolve(&self, id: AssetId) -> Option<DecodedImage> {
        if let Some(cached) = self.lock().touch(id) {
            return cached;
        }
        self.decode_and_insert(id)
    }

    fn resolve_bytes(&self, id: AssetId) -> Option<Arc<Vec<u8>>> {
        if let Some(shared) = self.lock().shared_bytes.get(&id) {
            return Some(Arc::clone(shared));
        }
        let bytes = Arc::new(self.encoded.get(&id)?.clone());
        // The copy happened outside the lock; a racing caller's copy is
        // equally valid, so whichever landed first is the one kept.
        let mut cache = self.lock();
        let shared = cache
            .shared_bytes
            .entry(id)
            .or_insert_with(|| Arc::clone(&bytes));
        Some(Arc::clone(shared))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

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

    // ---- LazyAssetResolver -----------------------------------------------

    /// A fake codec: the first byte is the square image's side length, a
    /// zero-length payload is "corrupt". Counts decodes so tests can tell a
    /// cache hit from a re-decode.
    fn counting_decoder(decodes: Arc<AtomicUsize>) -> Arc<ImageDecoder> {
        Arc::new(move |_id, bytes: &[u8]| {
            decodes.fetch_add(1, Ordering::SeqCst);
            let side = u32::from(*bytes.first()?);
            Some(solid(side, side, [1, 2, 3, 4]))
        })
    }

    fn lazy(assets: &[(AssetId, Vec<u8>)], budget: usize) -> (LazyAssetResolver, Arc<AtomicUsize>) {
        let decodes = Arc::new(AtomicUsize::new(0));
        let encoded: BTreeMap<AssetId, Vec<u8>> = assets.iter().cloned().collect();
        let resolver = LazyAssetResolver::with_budget(
            Arc::new(encoded),
            counting_decoder(decodes.clone()),
            budget,
        );
        (resolver, decodes)
    }

    #[test]
    fn lazy_resolver_decodes_on_first_use_and_then_serves_the_cache() {
        let id = AssetId::new();
        let (resolver, decodes) = lazy(&[(id, vec![2])], usize::MAX);
        assert_eq!(
            decodes.load(Ordering::SeqCst),
            0,
            "nothing decoded at construction"
        );
        assert_eq!(resolver.decoded_bytes(), 0);

        let first = resolver.resolve(id).unwrap();
        let second = resolver.resolve(id).unwrap();
        assert_eq!(decodes.load(Ordering::SeqCst), 1);
        assert!(Arc::ptr_eq(&first.pixels_rgba, &second.pixels_rgba));
        assert_eq!(resolver.decoded_bytes(), 2 * 2 * 4);
        assert!(resolver.resolve(AssetId::new()).is_none());
    }

    #[test]
    fn lazy_resolver_evicts_least_recently_used_over_budget() {
        let a = AssetId::from_u128(1);
        let b = AssetId::from_u128(2);
        let c = AssetId::from_u128(3);
        // Each 2x2 image is 16 bytes; the budget fits exactly two.
        let (resolver, decodes) = lazy(&[(a, vec![2]), (b, vec![2]), (c, vec![2])], 32);
        resolver.resolve(a);
        resolver.resolve(b);
        // Touch `a` so `b` becomes the least recently used.
        resolver.resolve(a);
        resolver.resolve(c);
        assert_eq!(resolver.decoded_bytes(), 32);
        assert_eq!(decodes.load(Ordering::SeqCst), 3);

        resolver.resolve(a);
        assert_eq!(decodes.load(Ordering::SeqCst), 3, "a survived eviction");
        resolver.resolve(b);
        assert_eq!(
            decodes.load(Ordering::SeqCst),
            4,
            "b was evicted and re-decoded"
        );
        assert_eq!(resolver.decoded_bytes(), 32);
    }

    #[test]
    fn lazy_resolver_keeps_an_image_larger_than_the_whole_budget() {
        let big = AssetId::new();
        let (resolver, decodes) = lazy(&[(big, vec![8])], 16);
        assert!(resolver.resolve(big).is_some());
        assert!(resolver.resolve(big).is_some());
        assert_eq!(
            decodes.load(Ordering::SeqCst),
            1,
            "not re-decoded every frame"
        );
        assert_eq!(resolver.decoded_bytes(), 8 * 8 * 4);
    }

    #[test]
    fn lazy_resolver_remembers_a_failed_decode() {
        let corrupt = AssetId::new();
        let (resolver, decodes) = lazy(&[(corrupt, Vec::new())], usize::MAX);
        assert!(resolver.resolve(corrupt).is_none());
        assert!(resolver.resolve(corrupt).is_none());
        assert_eq!(decodes.load(Ordering::SeqCst), 1);
        assert_eq!(resolver.decoded_bytes(), 0);
    }

    #[test]
    fn lazy_resolver_prewarms_only_what_is_not_yet_decoded() {
        let a = AssetId::from_u128(1);
        let b = AssetId::from_u128(2);
        let (resolver, decodes) = lazy(&[(a, vec![1]), (b, vec![1])], usize::MAX);
        resolver.resolve(a);
        resolver.prewarm([a, b, AssetId::from_u128(99)]);
        assert_eq!(decodes.load(Ordering::SeqCst), 2);
        resolver.resolve(b);
        assert_eq!(decodes.load(Ordering::SeqCst), 2, "prewarmed b is a hit");
    }

    #[test]
    fn lazy_resolver_hands_out_encoded_bytes() {
        let id = AssetId::new();
        let (resolver, decodes) = lazy(&[(id, vec![3, 9, 9])], usize::MAX);
        assert_eq!(resolver.resolve_bytes(id).unwrap().as_slice(), &[3, 9, 9]);
        assert_eq!(decodes.load(Ordering::SeqCst), 0, "bytes never decode");
        assert_eq!(resolver.len(), 1);
        assert!(!resolver.is_empty());
    }

    #[test]
    fn lazy_resolver_shares_one_allocation_of_the_encoded_bytes() {
        let id = AssetId::new();
        let (resolver, _decodes) = lazy(&[(id, vec![3, 9, 9])], usize::MAX);
        let first = resolver.resolve_bytes(id).unwrap();
        let second = resolver.resolve_bytes(id).unwrap();
        assert!(
            Arc::ptr_eq(&first, &second),
            "a repeat request must not copy the asset again"
        );
        assert!(resolver.resolve_bytes(AssetId::from_u128(99)).is_none());
    }
}
