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
//! [`DecodedImage::pixels`] is straight-alpha RGBA8 to match
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
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, PoisonError, Weak};

/// Decoded, ready-to-upload image pixels.
///
/// The pixels are straight-alpha RGBA8, row-major, with
/// `len == width * height * 4`. They live in a refcounted Skia buffer, so the
/// resolver's cache, every clone, and the renderer's `SkImage` share one
/// allocation: the renderer wraps these bytes instead of copying them. The
/// buffer's address doubles as the decode's identity (see
/// [`Self::shares_pixels`]), so a re-roll or re-decode is told apart for free.
#[derive(Clone)]
pub struct DecodedImage {
    pixels: skia_safe::Data,
    pub width: u32,
    pub height: u32,
}

impl DecodedImage {
    /// Construct from raw parts. Does not validate the length (the renderer
    /// tolerates a malformed buffer by falling back to the placeholder rather
    /// than panicking).
    pub fn new(pixels_rgba: &[u8], width: u32, height: u32) -> Self {
        Self::from_data(skia_safe::Data::new_copy(pixels_rgba), width, height)
    }

    /// Construct over pixels already in a Skia buffer, without copying them.
    pub fn from_data(pixels: skia_safe::Data, width: u32, height: u32) -> Self {
        Self {
            pixels,
            width,
            height,
        }
    }

    /// Straight-alpha RGBA8, row-major.
    pub fn pixels(&self) -> &[u8] {
        self.pixels.as_bytes()
    }

    /// The pixels' shared Skia buffer.
    pub fn data(&self) -> &skia_safe::Data {
        &self.pixels
    }

    /// Whether both images are the same decode, sharing one pixel buffer.
    pub fn shares_pixels(&self, other: &Self) -> bool {
        std::ptr::eq(self.pixels().as_ptr(), other.pixels().as_ptr())
            && self.pixels().len() == other.pixels().len()
    }

    /// Whether the buffer length matches `width * height * 4`. The renderer
    /// checks this before handing the bytes to Skia: a mismatched buffer would
    /// otherwise read out of bounds inside Skia, so a bad asset degrades to the
    /// placeholder instead of unsafety.
    pub fn is_well_formed(&self) -> bool {
        (self.width as usize)
            .checked_mul(self.height as usize)
            .and_then(|pixels| pixels.checked_mul(4))
            == Some(self.pixels().len())
    }
}

pub fn encoded_image_dimensions(bytes: &[u8]) -> Option<[u32; 2]> {
    let image = skia_safe::Image::from_encoded(skia_safe::Data::new_copy(bytes))?;
    Some([
        u32::try_from(image.width()).ok()?,
        u32::try_from(image.height()).ok()?,
    ])
}

pub fn decode_image_bytes(bytes: &[u8]) -> Option<DecodedImage> {
    let image = skia_safe::Image::from_encoded(skia_safe::Data::new_copy(bytes))?;
    let width = u32::try_from(image.width()).ok()?;
    let height = u32::try_from(image.height()).ok()?;
    let info = skia_safe::ImageInfo::new(
        (image.width(), image.height()),
        skia_safe::ColorType::RGBA8888,
        skia_safe::AlphaType::Unpremul,
        None,
    );
    let row_bytes = (width as usize).checked_mul(4)?;
    let mut pixels = vec![0u8; row_bytes.checked_mul(height as usize)?];
    if !image.read_pixels(
        &info,
        &mut pixels,
        row_bytes,
        (0, 0),
        skia_safe::image::CachingHint::Allow,
    ) {
        return None;
    }
    Some(DecodedImage::new(&pixels, width, height))
}

/// Pixels decoded at one level of an asset's display pyramid.
#[derive(Clone)]
pub struct LeveledImage {
    pub image: DecodedImage,
    /// `0` is the natural resolution; level `k` is box-downscaled by `2^k`.
    pub level: u32,
}

/// What a non-blocking lookup found for an asset.
pub enum LevelLookup {
    /// The finest decode on hand. It can be coarser than the level asked for
    /// while a finer one decodes in the background.
    Decoded(LeveledImage),
    /// Nothing decoded yet; a background decode is under way.
    Pending,
    /// The asset is unknown, or its bytes are not a decodable image.
    Unavailable,
}

/// Told when a background decode lands, so a view that drew a placeholder
/// can draw again.
pub trait DecodeListener: Send + Sync {
    /// Runs on the decoding thread before the decode counts as finished, so
    /// it must return quickly and never resolve images itself.
    fn images_decoded(&self);
}

/// Resolves an [`AssetId`] to decoded pixels.
///
/// Implemented by the app over a `FantaFile`; faked in renderer tests. The
/// renderer never touches `fanta-format` directly — it only knows this trait.
///
/// [`Self::resolve`] decodes on the calling thread when it must, for callers
/// that need complete output (export, sampling, thumbnails). Interactive
/// drawing uses [`Self::resolve_level_nonblocking`] instead: a miss starts a
/// background decode and the frame draws a placeholder, with the pixels
/// available on a later frame. Returning nothing for a still-decoding asset is
/// the deferred-output pattern the rest of the media pipeline (video frames,
/// waveforms, AI outputs) reuses — bitmap is its simplest instance.
///
/// `Send + Sync` so the resolver can be shared across the render thread and a
/// background decode pool behind an [`Arc`].
pub trait AssetResolver: Send + Sync {
    /// Decoded RGBA8 pixels for `id` at natural resolution, or `None` if
    /// unknown or undecodable.
    fn resolve(&self, id: AssetId) -> Option<DecodedImage>;

    /// Raw, undecoded source bytes for `id` (WAV / glTF / MP4 / …), or `None`.
    /// Used by the non-image visualizers (audio waveform, 3D, video) that parse
    /// the source directly. Defaults to `None` so image-only resolvers — and
    /// renderer tests — need not implement it.
    fn resolve_bytes(&self, _id: AssetId) -> Option<std::sync::Arc<Vec<u8>>> {
        None
    }

    /// Bytes this resolver holds beyond the document's own encoded assets, for
    /// memory diagnostics. `None` when it does not track them.
    fn held_bytes(&self) -> Option<usize> {
        None
    }

    /// The asset's natural size in pixels, read from its header when that
    /// avoids a decode.
    fn image_size(&self, id: AssetId) -> Option<[u32; 2]> {
        self.resolve(id).map(|image| [image.width, image.height])
    }

    /// The finest decode of `id` this resolver holds, without decoding on the
    /// calling thread. When nothing as fine as `level` is held, a background
    /// decode of `level` starts, and [`Self::decode_generation`] moves once it
    /// lands.
    ///
    /// The default suits resolvers whose pixels are already in memory. A
    /// resolver that decodes must override it, or drawing threads decode.
    fn resolve_level_nonblocking(&self, id: AssetId, _level: u32) -> LevelLookup {
        match self.resolve(id) {
            Some(image) => LevelLookup::Decoded(LeveledImage { image, level: 0 }),
            None => LevelLookup::Unavailable,
        }
    }

    /// Moves whenever a decode lands. A frame drawn before the change may show
    /// a placeholder or a coarser level, so it belongs to the frame's identity.
    fn decode_generation(&self) -> u64 {
        0
    }

    /// Wake `listener` after each decode lands, for as long as it is alive.
    fn subscribe_decodes(&self, _listener: Weak<dyn DecodeListener>) {}
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

/// Default cap on decoded RGBA held by a [`LazyAssetResolver`]: 256 MiB.
/// Above it the least-recently-resolved images are dropped and re-decoded on
/// their next use. A renderer keeps the images it draws in its own cache, which
/// shares these buffers, so this mostly bounds decodes nobody has drawn yet:
/// room for the pages being opened, not for every image ever opened.
pub const DEFAULT_DECODED_BUDGET_BYTES: usize = 256 * 1024 * 1024;

/// Turns one asset's encoded bytes into straight-alpha RGBA8, or `None` when
/// the bytes are not a decodable image (the resolver remembers the failure so
/// a corrupt asset is not re-attempted on every frame). The app supplies this
/// — see the module docs on why no decoder lives in this crate.
pub type ImageDecoder = dyn Fn(AssetId, &[u8]) -> Option<DecodedImage> + Send + Sync;

/// Reads an image's natural size from its encoded header without decoding its
/// pixels, or `None` for a format it does not know.
pub type ImageSizeReader = dyn Fn(&[u8]) -> Option<[u32; 2]> + Send + Sync;

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
/// Each asset is held at one pyramid level: the finest any caller needed so
/// far. A canvas asks for the level it draws, so a photo shown as a thumbnail
/// costs a thumbnail's pixels rather than the full image's. Before this, a page
/// of 24 full-size photos needed more than the budget, and every frame evicted
/// and re-decoded the photos it was about to draw.
///
/// Shared between the render thread, its own background decode workers,
/// prewarm threads and the UI thread (screenshots, export), so it is
/// `Send + Sync`, and the lock is never held across a decode: a miss decodes
/// outside the lock and inserts afterwards. A blocking resolve that finds the
/// asset already being decoded waits for that decode instead of repeating it.
///
/// [`prewarm`]: Self::prewarm
pub struct LazyAssetResolver {
    shared: Arc<LazyShared>,
}

struct LazyShared {
    encoded: Arc<BTreeMap<AssetId, Vec<u8>>>,
    decoder: Arc<ImageDecoder>,
    size_reader: Arc<ImageSizeReader>,
    budget_bytes: usize,
    cache: Mutex<DecodedCache>,
    /// Notified whenever an in-flight decode finishes (or unwinds).
    decoded: Condvar,
    queue: Mutex<DecodeQueue>,
    /// Wakes idle background workers.
    work: Condvar,
    generation: AtomicU64,
    listeners: Mutex<Vec<Weak<dyn DecodeListener>>>,
}

#[derive(Default)]
struct DecodeQueue {
    /// Newest request last. Workers take from the end, so the frame that asked
    /// most recently is served first while a zoom keeps changing the level.
    jobs: Vec<(AssetId, u32)>,
    workers: usize,
    shutdown: bool,
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
    /// refcount bump, not a copy of the whole file. Each entry is a SECOND copy
    /// of bytes `LazyAssetResolver::encoded` already holds — a 200 MB embedded
    /// audio asset costs 200 MB here — so it counts against the same budget as
    /// the decoded pixels and is swept when the cache runs over.
    shared_bytes: HashMap<AssetId, Arc<Vec<u8>>>,
    /// Bytes held by [`Self::shared_bytes`].
    shared_bytes_total: usize,
    /// Decodes queued or running, by asset and pyramid level.
    in_flight: HashSet<(AssetId, u32)>,
    /// Natural sizes, from headers or from finished decodes.
    sizes: HashMap<AssetId, [u32; 2]>,
}

struct CacheEntry {
    /// `None` records a decode failure so the asset is not retried per frame.
    image: Option<DecodedImage>,
    level: u32,
    tick: u64,
}

impl DecodedCache {
    fn touch(&mut self, id: AssetId) -> Option<(Option<DecodedImage>, u32)> {
        let entry = self.entries.get_mut(&id)?;
        self.recency.remove(&entry.tick);
        entry.tick = self.next_tick;
        self.next_tick += 1;
        self.recency.insert(entry.tick, id);
        Some((entry.image.clone(), entry.level))
    }

    /// Everything the cache holds on top of the document's own copy: decoded
    /// pixels plus the duplicated encoded bytes handed out by `resolve_bytes`.
    fn held_bytes(&self) -> usize {
        self.decoded_bytes + self.shared_bytes_total
    }

    /// Whether a decode at `level` or finer is queued or running for `id`.
    fn decoding(&self, id: AssetId, level: u32) -> bool {
        (0..=level).any(|finer| self.in_flight.contains(&(id, finer)))
    }

    /// Record a decode of `id` at `level`, keeping whatever is already finer.
    /// Returns whether the cache changed.
    fn insert(
        &mut self,
        id: AssetId,
        image: Option<DecodedImage>,
        level: u32,
        budget_bytes: usize,
    ) -> bool {
        if let Some(existing) = self.entries.get(&id) {
            let keep = match (&existing.image, &image) {
                (None, _) | (Some(_), None) => true,
                (Some(_), Some(_)) => existing.level <= level,
            };
            if keep {
                return false;
            }
            self.remove(id);
        }
        let tick = self.next_tick;
        self.next_tick += 1;
        self.decoded_bytes += image.as_ref().map_or(0, |image| image.pixels().len());
        self.entries.insert(id, CacheEntry { image, level, tick });
        self.recency.insert(tick, id);
        self.evict_to(budget_bytes, Some(id));
        true
    }

    fn remove(&mut self, id: AssetId) {
        if let Some(removed) = self.entries.remove(&id) {
            self.recency.remove(&removed.tick);
            self.decoded_bytes -= removed
                .image
                .as_ref()
                .map_or(0, |image| image.pixels().len());
        }
    }

    /// Drop cached bytes until the cache is back inside `budget_bytes`,
    /// keeping `keep` whatever happens.
    ///
    /// Shared encoded bytes go first, and only the ones no caller still holds:
    /// they are a pure duplicate of what the document already owns, so dropping
    /// one costs a single re-copy on the next `resolve_bytes`, while dropping a
    /// decoded image costs a full re-decode. `keep` is the entry the caller
    /// needs right now, never evicted even if it alone exceeds the budget —
    /// otherwise an oversized image the renderer is drawing would be re-decoded
    /// on every single frame.
    fn evict_to(&mut self, budget_bytes: usize, keep: Option<AssetId>) {
        if self.held_bytes() > budget_bytes {
            self.shared_bytes.retain(|id, bytes| {
                let in_use = Arc::strong_count(bytes) > 1 || Some(*id) == keep;
                if !in_use {
                    self.shared_bytes_total = self.shared_bytes_total.saturating_sub(bytes.len());
                }
                in_use
            });
        }
        while self.held_bytes() > budget_bytes {
            let Some((_, &oldest)) = self.recency.iter().next() else {
                break;
            };
            if Some(oldest) == keep {
                break;
            }
            self.remove(oldest);
        }
    }
}

impl LazyShared {
    fn lock(&self) -> std::sync::MutexGuard<'_, DecodedCache> {
        // The cache holds no invariant a panicking holder could break
        // half-way (each mutation is a single insert/evict step), so a
        // poisoned lock is still safe to keep using.
        self.cache.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn lock_queue(&self) -> std::sync::MutexGuard<'_, DecodeQueue> {
        self.queue.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Decoding is CPU-bound; leave cores for the render and UI threads.
    fn max_workers() -> usize {
        std::thread::available_parallelism()
            .map_or(1, std::num::NonZeroUsize::get)
            .saturating_sub(2)
            .clamp(1, 4)
    }

    /// Queue a background decode the caller already marked in flight.
    fn enqueue(shared: &Arc<Self>, id: AssetId, level: u32) {
        let mut queue = shared.lock_queue();
        queue.jobs.push((id, level));
        if queue.workers < Self::max_workers() {
            let worker = Arc::clone(shared);
            match std::thread::Builder::new()
                .name("fanta-image-decode".into())
                .spawn(move || worker.work())
            {
                Ok(_) => queue.workers += 1,
                Err(error) => {
                    tracing::warn!(%error, "starting an image decode thread failed");
                    if queue.workers == 0 {
                        // Nobody would ever take the job: forget it, so a
                        // later draw asks again instead of waiting forever.
                        queue.jobs.clear();
                        drop(queue);
                        shared.lock().in_flight.retain(|job| *job != (id, level));
                        shared.decoded.notify_all();
                        return;
                    }
                }
            }
        }
        drop(queue);
        shared.work.notify_one();
    }

    fn work(self: Arc<Self>) {
        loop {
            let (id, level) = {
                let mut queue = self.lock_queue();
                loop {
                    if queue.shutdown {
                        return;
                    }
                    if let Some(job) = queue.jobs.pop() {
                        break job;
                    }
                    queue = self
                        .work
                        .wait(queue)
                        .unwrap_or_else(PoisonError::into_inner);
                }
            };
            self.decode_in_background(id, level);
        }
    }

    fn decode_in_background(&self, id: AssetId, level: u32) {
        let in_flight = InFlightDecode {
            shared: self,
            id,
            level,
        };
        // A decode at least this fine may have landed while the job waited.
        let held = self
            .lock()
            .entries
            .get(&id)
            .map(|entry| (entry.image.is_some(), entry.level));
        if held.is_some_and(|(decoded, held)| !decoded || held <= level) {
            return;
        }
        let Some(bytes) = self.encoded.get(&id) else {
            return;
        };
        let decoded = (self.decoder)(id, bytes).map(|full| {
            let natural = [full.width, full.height];
            let leveled =
                crate::image::decoded_pyramid_level(&full, level).unwrap_or(LeveledImage {
                    image: full,
                    level: 0,
                });
            (natural, leveled)
        });
        // A recorded failure counts as a change too: the frame that drew the
        // pending wash must redraw to show the missing-image placeholder.
        let changed = {
            let mut cache = self.lock();
            match decoded {
                Some((natural, leveled)) => {
                    cache.sizes.insert(id, natural);
                    cache.insert(id, Some(leveled.image), leveled.level, self.budget_bytes)
                }
                None => cache.insert(id, None, 0, self.budget_bytes),
            }
        };
        // Waiters see the decode finish only after the generation moved, so
        // whoever waited also finds the listeners woken.
        if changed {
            self.landed();
        }
        drop(in_flight);
    }

    /// A decode changed what drawing threads can find: move the generation
    /// and wake the listeners that drew without it.
    fn landed(&self) {
        self.generation.fetch_add(1, Ordering::AcqRel);
        let live: Vec<Arc<dyn DecodeListener>> = {
            let mut listeners = self
                .listeners
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            listeners.retain(|listener| listener.strong_count() > 0);
            listeners.iter().filter_map(Weak::upgrade).collect()
        };
        for listener in live {
            listener.images_decoded();
        }
    }
}

impl LazyAssetResolver {
    /// Resolver over `encoded` with the [`DEFAULT_DECODED_BUDGET_BYTES`] cap,
    /// reading natural sizes through Skia's codecs.
    pub fn new(encoded: Arc<BTreeMap<AssetId, Vec<u8>>>, decoder: Arc<ImageDecoder>) -> Self {
        Self::with_budget(encoded, decoder, DEFAULT_DECODED_BUDGET_BYTES)
    }

    /// Like [`Self::new`], reading natural sizes with `size_reader`. A
    /// background draw picks the level to decode from the natural size, so a
    /// decoder that handles formats this crate's Skia build cannot parse
    /// (WebP) needs a reader for them too, or those images decode at full
    /// resolution.
    pub fn with_size_reader(
        encoded: Arc<BTreeMap<AssetId, Vec<u8>>>,
        decoder: Arc<ImageDecoder>,
        size_reader: Arc<ImageSizeReader>,
    ) -> Self {
        Self::build(encoded, decoder, size_reader, DEFAULT_DECODED_BUDGET_BYTES)
    }

    /// Resolver over `encoded` keeping at most `budget_bytes` of decoded RGBA
    /// (the most recently used image is always kept, budget or not).
    pub fn with_budget(
        encoded: Arc<BTreeMap<AssetId, Vec<u8>>>,
        decoder: Arc<ImageDecoder>,
        budget_bytes: usize,
    ) -> Self {
        Self::build(
            encoded,
            decoder,
            Arc::new(encoded_image_dimensions),
            budget_bytes,
        )
    }

    fn build(
        encoded: Arc<BTreeMap<AssetId, Vec<u8>>>,
        decoder: Arc<ImageDecoder>,
        size_reader: Arc<ImageSizeReader>,
        budget_bytes: usize,
    ) -> Self {
        Self {
            shared: Arc::new(LazyShared {
                encoded,
                decoder,
                size_reader,
                budget_bytes,
                cache: Mutex::new(DecodedCache::default()),
                decoded: Condvar::new(),
                queue: Mutex::new(DecodeQueue::default()),
                work: Condvar::new(),
                generation: AtomicU64::new(0),
                listeners: Mutex::new(Vec::new()),
            }),
        }
    }

    /// Decode `ids` at natural resolution now (skipping ones already decoded,
    /// being decoded, or unknown), so the first blocking frame that draws them
    /// does not pay the decode. Meant for background tasks, e.g. with the
    /// assets of a page that opens.
    pub fn prewarm(&self, ids: impl IntoIterator<Item = AssetId>) {
        for id in ids {
            self.decode_full(id, false);
        }
    }

    /// Queue background decodes of `ids` at the coarsest level whose longer
    /// side still reaches `max_side` pixels, and return at once. A canvas then
    /// has something to draw on its first frame and asks for finer levels
    /// only where it shows an image larger than that.
    pub fn prewarm_previews(&self, ids: impl IntoIterator<Item = AssetId>, max_side: u32) {
        for id in ids {
            let level = self.image_size(id).map_or(0, |[width, height]| {
                preview_level(width.max(height), max_side)
            });
            self.lookup(id, level);
        }
    }

    /// Block until no decode is queued or running. Tests and benchmarks use it
    /// to measure settled frames.
    pub fn wait_for_background_decodes(&self) {
        let mut cache = self.shared.lock();
        while !cache.in_flight.is_empty() {
            cache = self
                .shared
                .decoded
                .wait(cache)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }

    /// Bytes of decoded RGBA currently held.
    pub fn decoded_bytes(&self) -> usize {
        self.shared.lock().decoded_bytes
    }

    /// Everything the cache holds on top of the document's own copy of the
    /// assets: decoded pixels plus the encoded bytes `resolve_bytes` shares.
    /// This is what [`Self::budget_bytes`] caps.
    pub fn held_bytes(&self) -> usize {
        self.shared.lock().held_bytes()
    }

    /// The cap this resolver evicts down to.
    pub fn budget_bytes(&self) -> usize {
        self.shared.budget_bytes
    }

    /// Number of encoded assets this resolver knows about.
    pub fn len(&self) -> usize {
        self.shared.encoded.len()
    }

    /// Whether there are no encoded assets at all.
    pub fn is_empty(&self) -> bool {
        self.shared.encoded.is_empty()
    }

    /// Decode `id` at natural resolution unless it is cached. When another
    /// thread is already decoding it, `wait` waits for that decode; otherwise
    /// this returns `None` at once (a prewarm has nothing to add).
    fn decode_full(&self, id: AssetId, wait: bool) -> Option<DecodedImage> {
        let shared = &*self.shared;
        let bytes = shared.encoded.get(&id)?;
        {
            let mut cache = shared.lock();
            loop {
                match cache.touch(id) {
                    Some((None, _)) => return None,
                    Some((Some(image), 0)) => return Some(image),
                    _ => {}
                }
                if !cache.in_flight.contains(&(id, 0)) {
                    break;
                }
                if !wait {
                    return None;
                }
                cache = shared
                    .decoded
                    .wait(cache)
                    .unwrap_or_else(PoisonError::into_inner);
            }
            cache.in_flight.insert((id, 0));
        }
        let in_flight = InFlightDecode {
            shared,
            id,
            level: 0,
        };
        let image = (shared.decoder)(id, bytes);
        let mut cache = shared.lock();
        if let Some(image) = &image {
            cache.sizes.insert(id, [image.width, image.height]);
        }
        let changed = cache.insert(id, image, 0, shared.budget_bytes);
        let image = match cache.touch(id) {
            Some((image, 0)) => image,
            _ => None,
        };
        drop(cache);
        if changed {
            shared.landed();
        }
        drop(in_flight);
        image
    }

    /// The finest decode of `id` on hand, queuing a background decode of
    /// `level` when nothing that fine is held or coming.
    fn lookup(&self, id: AssetId, level: u32) -> LevelLookup {
        if !self.shared.encoded.contains_key(&id) {
            return LevelLookup::Unavailable;
        }
        let mut cache = self.shared.lock();
        let mut held = match cache.touch(id) {
            Some((None, _)) => return LevelLookup::Unavailable,
            Some((Some(image), held)) => Some(LeveledImage { image, level: held }),
            None => None,
        };
        if let Some(fine_enough) = held.take_if(|held| held.level <= level) {
            return LevelLookup::Decoded(fine_enough);
        }
        let queue = !cache.decoding(id, level);
        if queue {
            cache.in_flight.insert((id, level));
        }
        drop(cache);
        if queue {
            LazyShared::enqueue(&self.shared, id, level);
        }
        match held {
            Some(held) => LevelLookup::Decoded(held),
            None => LevelLookup::Pending,
        }
    }
}

impl Drop for LazyAssetResolver {
    fn drop(&mut self) {
        self.shared.lock_queue().shutdown = true;
        self.shared.work.notify_all();
    }
}

/// The coarsest pyramid level of an image whose longer side is `side` that
/// still keeps at least `max_side` pixels on that side.
fn preview_level(side: u32, max_side: u32) -> u32 {
    let mut level = 0;
    while max_side > 0 && (side >> (level + 1)) >= max_side {
        level += 1;
    }
    level
}

/// Clears a decode's in-flight mark when it ends — including by unwinding out
/// of a panicking decoder, so waiters never block forever.
struct InFlightDecode<'a> {
    shared: &'a LazyShared,
    id: AssetId,
    level: u32,
}

impl Drop for InFlightDecode<'_> {
    fn drop(&mut self) {
        self.shared.lock().in_flight.remove(&(self.id, self.level));
        self.shared.decoded.notify_all();
    }
}

impl AssetResolver for LazyAssetResolver {
    fn resolve(&self, id: AssetId) -> Option<DecodedImage> {
        self.decode_full(id, true)
    }

    fn resolve_bytes(&self, id: AssetId) -> Option<Arc<Vec<u8>>> {
        let shared = &*self.shared;
        if let Some(bytes) = shared.lock().shared_bytes.get(&id) {
            return Some(Arc::clone(bytes));
        }
        let bytes = Arc::new(shared.encoded.get(&id)?.clone());
        // The copy happened outside the lock; a racing caller's copy is
        // equally valid, so whichever landed first is the one kept.
        let mut cache = shared.lock();
        let handed_out = match cache.shared_bytes.get(&id) {
            Some(existing) => Arc::clone(existing),
            None => {
                cache.shared_bytes_total += bytes.len();
                cache.shared_bytes.insert(id, Arc::clone(&bytes));
                bytes
            }
        };
        cache.evict_to(shared.budget_bytes, Some(id));
        Some(handed_out)
    }

    fn held_bytes(&self) -> Option<usize> {
        Some(LazyAssetResolver::held_bytes(self))
    }

    fn image_size(&self, id: AssetId) -> Option<[u32; 2]> {
        if let Some(size) = self.shared.lock().sizes.get(&id) {
            return Some(*size);
        }
        let size = (self.shared.size_reader)(self.shared.encoded.get(&id)?)?;
        self.shared.lock().sizes.insert(id, size);
        Some(size)
    }

    fn resolve_level_nonblocking(&self, id: AssetId, level: u32) -> LevelLookup {
        self.lookup(id, level)
    }

    fn decode_generation(&self) -> u64 {
        self.shared.generation.load(Ordering::Acquire)
    }

    fn subscribe_decodes(&self, listener: Weak<dyn DecodeListener>) {
        self.shared
            .listeners
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(listener);
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
        DecodedImage::new(&px, w, h)
    }

    #[test]
    fn insert_then_resolve_round_trips_the_same_buffer() {
        let mut r = InMemoryAssetResolver::new();
        let id = AssetId::new();
        let img = solid(4, 4, [255, 0, 0, 255]);
        assert!(r.insert(id, img.clone()).is_none());

        let got = r.resolve(id).expect("registered asset resolves");
        assert_eq!(got.width, 4);
        assert_eq!(got.height, 4);
        // The resolve clone shares the buffer — no pixel copy.
        assert!(got.shares_pixels(&img));
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
        assert_eq!(now.pixels()[0], 9);
    }

    #[test]
    fn well_formed_detects_length_mismatch() {
        let good = solid(2, 2, [0, 0, 0, 255]);
        assert!(good.is_well_formed());
        let bad = DecodedImage::new(&[0u8; 3], 2, 2);
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
        assert!(first.shares_pixels(&second));
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
    fn lazy_resolver_waits_for_an_in_flight_decode_instead_of_repeating_it() {
        let id = AssetId::new();
        let decodes = Arc::new(AtomicUsize::new(0));
        let released = Arc::new((Mutex::new(false), Condvar::new()));
        let decoder: Arc<ImageDecoder> = Arc::new({
            let decodes = decodes.clone();
            let released = released.clone();
            move |_id, bytes: &[u8]| {
                decodes.fetch_add(1, Ordering::SeqCst);
                let (flag, signal) = &*released;
                let mut open = flag.lock().unwrap_or_else(PoisonError::into_inner);
                while !*open {
                    open = signal.wait(open).unwrap_or_else(PoisonError::into_inner);
                }
                let side = u32::from(*bytes.first()?);
                Some(solid(side, side, [1, 2, 3, 4]))
            }
        });
        let encoded: BTreeMap<AssetId, Vec<u8>> = [(id, vec![2])].into_iter().collect();
        let resolver = Arc::new(LazyAssetResolver::with_budget(
            Arc::new(encoded),
            decoder,
            usize::MAX,
        ));
        let first = std::thread::spawn({
            let resolver = resolver.clone();
            move || resolver.resolve(id)
        });
        while decodes.load(Ordering::SeqCst) == 0 {
            std::thread::yield_now();
        }
        resolver.prewarm([id]);
        let second = std::thread::spawn(move || resolver.resolve(id));
        // Give the second resolve time to find the decode in flight; if it
        // arrives late it finds the cached result, which is equally valid.
        std::thread::sleep(std::time::Duration::from_millis(50));
        {
            let (flag, signal) = &*released;
            *flag.lock().unwrap_or_else(PoisonError::into_inner) = true;
            signal.notify_all();
        }
        let first = first.join().expect("first resolve").expect("decoded");
        let second = second.join().expect("second resolve").expect("decoded");
        assert_eq!(
            decodes.load(Ordering::SeqCst),
            1,
            "one decode serves every caller"
        );
        assert!(first.shares_pixels(&second));
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

    #[test]
    fn shared_encoded_bytes_are_budgeted_and_released_once_nobody_holds_them() {
        // `resolve_bytes` hands out a COPY of bytes the document already owns,
        // so an unbudgeted memo of them doubles a media-heavy document's
        // resident memory for good. Every visible audio node asks once a frame.
        let ids: Vec<AssetId> = (1..=4).map(AssetId::from_u128).collect();
        let assets: Vec<(AssetId, Vec<u8>)> = ids.iter().map(|id| (*id, vec![1u8; 100])).collect();
        let (resolver, _decodes) = lazy(&assets, 250);

        for id in &ids {
            // Dropped immediately, as the waveform painter does once it has
            // read the samples for the frame.
            assert_eq!(resolver.resolve_bytes(*id).unwrap().len(), 100);
        }
        assert!(
            resolver.held_bytes() <= 250,
            "shared encoded bytes must be swept back inside the budget, held {}",
            resolver.held_bytes()
        );

        // Bytes a caller is still holding are never swept out from under it.
        let held: Vec<Arc<Vec<u8>>> = ids
            .iter()
            .map(|id| resolver.resolve_bytes(*id).expect("asset bytes"))
            .collect();
        for (bytes, id) in held.iter().zip(&ids) {
            assert!(
                Arc::ptr_eq(bytes, &resolver.resolve_bytes(*id).expect("asset bytes")),
                "a live handout must stay the one shared allocation"
            );
        }
    }

    // ---- Background decoding -------------------------------------------------

    /// A real PNG, so header reads and Skia decoding work: a `side`-pixel
    /// square of horizontal opaque stripes.
    fn png(side: u32) -> Vec<u8> {
        let mut surface =
            skia_safe::surfaces::raster_n32_premul((side as i32, side as i32)).expect("surface");
        let canvas = surface.canvas();
        canvas.clear(skia_safe::Color::from_rgb(40, 120, 200));
        let mut paint = skia_safe::Paint::default();
        paint.set_color(skia_safe::Color::from_rgb(250, 200, 30));
        for row in (0..side).step_by(4) {
            canvas.draw_rect(
                skia_safe::Rect::from_xywh(0.0, row as f32, side as f32, 2.0),
                &paint,
            );
        }
        surface
            .image_snapshot()
            .encode(None, skia_safe::EncodedImageFormat::PNG, 100)
            .expect("png")
            .as_bytes()
            .to_vec()
    }

    /// Decodes through Skia, recording each decode's thread.
    fn recording_decoder(threads: Arc<Mutex<Vec<std::thread::ThreadId>>>) -> Arc<ImageDecoder> {
        Arc::new(move |_id, bytes: &[u8]| {
            threads
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(std::thread::current().id());
            decode_image_bytes(bytes)
        })
    }

    #[derive(Default)]
    struct CountingListener(AtomicUsize);

    impl DecodeListener for CountingListener {
        fn images_decoded(&self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn background(
        assets: &[(AssetId, Vec<u8>)],
    ) -> (LazyAssetResolver, Arc<Mutex<Vec<std::thread::ThreadId>>>) {
        let threads = Arc::new(Mutex::new(Vec::new()));
        let encoded: BTreeMap<AssetId, Vec<u8>> = assets.iter().cloned().collect();
        let resolver =
            LazyAssetResolver::new(Arc::new(encoded), recording_decoder(threads.clone()));
        (resolver, threads)
    }

    fn decoded_at(lookup: LevelLookup) -> Option<(u32, [u32; 2])> {
        match lookup {
            LevelLookup::Decoded(found) => {
                Some((found.level, [found.image.width, found.image.height]))
            }
            LevelLookup::Pending | LevelLookup::Unavailable => None,
        }
    }

    #[test]
    fn background_lookup_decodes_off_the_calling_thread_and_keeps_only_the_level() {
        let id = AssetId::new();
        let (resolver, threads) = background(&[(id, png(64))]);
        let listener = Arc::new(CountingListener::default());
        let weak: Weak<CountingListener> = Arc::downgrade(&listener);
        resolver.subscribe_decodes(weak);
        assert_eq!(
            resolver.image_size(id),
            Some([64, 64]),
            "read from the header"
        );
        assert!(matches!(
            resolver.resolve_level_nonblocking(id, 2),
            LevelLookup::Pending
        ));
        resolver.wait_for_background_decodes();

        let decode_threads = threads
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        assert_eq!(decode_threads.len(), 1);
        assert_ne!(
            decode_threads[0],
            std::thread::current().id(),
            "the caller never decodes"
        );
        assert!(
            listener.0.load(Ordering::SeqCst) >= 1,
            "the landing wakes listeners"
        );
        assert!(resolver.decode_generation() > 0);
        assert_eq!(
            decoded_at(resolver.resolve_level_nonblocking(id, 2)),
            Some((2, [16, 16]))
        );
        assert_eq!(
            resolver.decoded_bytes(),
            16 * 16 * 4,
            "only the drawn level is held, not the 64-pixel image"
        );
    }

    #[test]
    fn background_lookup_serves_a_coarser_level_while_a_finer_one_decodes() {
        let id = AssetId::new();
        let (resolver, threads) = background(&[(id, png(64))]);
        resolver.resolve_level_nonblocking(id, 3);
        resolver.wait_for_background_decodes();
        assert_eq!(
            decoded_at(resolver.resolve_level_nonblocking(id, 1)),
            Some((3, [8, 8])),
            "the coarser level answers at once"
        );
        resolver.wait_for_background_decodes();
        assert_eq!(
            decoded_at(resolver.resolve_level_nonblocking(id, 1)),
            Some((1, [32, 32]))
        );
        assert_eq!(
            decoded_at(resolver.resolve_level_nonblocking(id, 3)),
            Some((1, [32, 32])),
            "a finer decode serves coarser draws without another decode"
        );
        assert_eq!(
            threads.lock().unwrap_or_else(PoisonError::into_inner).len(),
            2
        );
        assert_eq!(
            resolver.decoded_bytes(),
            32 * 32 * 4,
            "the finer level replaced the coarser"
        );
    }

    #[test]
    fn blocking_resolve_upgrades_a_background_level_to_natural_resolution() {
        let id = AssetId::new();
        let (resolver, _threads) = background(&[(id, png(64))]);
        resolver.resolve_level_nonblocking(id, 2);
        resolver.wait_for_background_decodes();
        let full = resolver.resolve(id).expect("decodes at natural size");
        assert_eq!([full.width, full.height], [64, 64]);
        assert_eq!(
            decoded_at(resolver.resolve_level_nonblocking(id, 2)),
            Some((0, [64, 64]))
        );
    }

    #[test]
    fn a_failed_background_decode_is_remembered() {
        let corrupt = AssetId::new();
        let (resolver, threads) = background(&[(corrupt, vec![1, 2, 3])]);
        assert!(matches!(
            resolver.resolve_level_nonblocking(corrupt, 0),
            LevelLookup::Pending
        ));
        resolver.wait_for_background_decodes();
        assert!(matches!(
            resolver.resolve_level_nonblocking(corrupt, 0),
            LevelLookup::Unavailable
        ));
        resolver.wait_for_background_decodes();
        assert_eq!(
            threads.lock().unwrap_or_else(PoisonError::into_inner).len(),
            1
        );
        assert!(matches!(
            resolver.resolve_level_nonblocking(AssetId::new(), 0),
            LevelLookup::Unavailable
        ));
    }

    #[test]
    fn an_injected_size_reader_sizes_formats_skia_cannot_parse() {
        let id = AssetId::new();
        let encoded = Arc::new(BTreeMap::from([(id, b"RIFF\0\0\0\0WEBPVP8L".to_vec())]));
        let decoder: Arc<ImageDecoder> = Arc::new(|_, _| None);
        let skia_only = LazyAssetResolver::new(encoded.clone(), decoder.clone());
        assert_eq!(skia_only.image_size(id), None);
        let resolver = LazyAssetResolver::with_size_reader(
            encoded,
            decoder,
            Arc::new(|_bytes: &[u8]| Some([2048, 1024])),
        );
        assert_eq!(resolver.image_size(id), Some([2048, 1024]));
    }

    #[test]
    fn preview_prewarm_queues_the_coarsest_level_that_keeps_the_preview_size() {
        assert_eq!(preview_level(2048, 512), 2);
        assert_eq!(preview_level(300, 512), 0);
        assert_eq!(preview_level(1024, 0), 0);

        let id = AssetId::new();
        let (resolver, threads) = background(&[(id, png(64))]);
        resolver.prewarm_previews([id], 16);
        resolver.wait_for_background_decodes();
        assert_eq!(
            decoded_at(resolver.resolve_level_nonblocking(id, 2)),
            Some((2, [16, 16]))
        );
        assert_eq!(
            threads.lock().unwrap_or_else(PoisonError::into_inner).len(),
            1
        );
    }
}
