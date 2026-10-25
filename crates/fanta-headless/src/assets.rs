//! Images ingested as project assets while a document is open: the decoded
//! upload, the stores it must reach (the raw bytes persisted on save and the
//! render resolver), and the overlay that makes a new image drawable without
//! re-decoding every embedded asset.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, PoisonError, RwLock, Weak};

use anyhow::{Context as _, Result, bail};
use base64::Engine as _;
use fanta_doc::AssetId;
use fanta_render::{AssetResolver, DecodeListener, DecodedImage, LevelLookup, LeveledImage};

/// Cap on the encoded bytes of one image ingested as a project asset (a paste,
/// a drop, an agent's `create_image`). Generous for any generated PNG while
/// keeping a bad payload from ballooning the document — and enforced before
/// decoding, since a decoder handed arbitrary bytes can allocate far more
/// than it was given.
pub const MAX_IMAGE_SOURCE_BYTES: usize = 32 * 1024 * 1024;

/// Somewhere an edit batch can ingest images and, when the batch fails, take
/// them out again. The editor's implementation also keeps its thumbnail cache
/// in step.
pub trait ImageStore {
    /// Ingest encoded image bytes. Returns the asset, its natural size, and
    /// whether this call inserted it, so a failed batch removes only what it
    /// added and never an existing duplicate.
    fn add_image_tracked(&mut self, bytes: Vec<u8>) -> Result<(AssetId, [u32; 2], bool)>;

    /// Drop an asset this store ingested.
    fn remove(&mut self, asset: AssetId);
}

/// Decode a `create_image` source: a `data:` URI or raw base64. URLs are
/// deliberately not fetched here — the surface is synchronous and network
/// access belongs to the tool layer (`place_generation`), which downloads and
/// re-issues the op with base64.
pub fn decode_image_source(source: &str) -> Result<Vec<u8>> {
    let source = source.trim();
    if source.starts_with("http://") || source.starts_with("https://") {
        bail!(
            "create_image does not fetch URLs; use the place_generation tool, or fetch the \
             bytes yourself and pass them as base64"
        );
    }
    let payload = match source.strip_prefix("data:") {
        Some(rest) => {
            let (header, payload) = rest
                .split_once(',')
                .context("the data: URI has no `,` separating the payload")?;
            if !header.ends_with(";base64") {
                bail!("only base64 data: URIs are supported");
            }
            payload
        }
        None => source,
    };
    // Models occasionally hard-wrap long base64 payloads; strip whitespace
    // before decoding so that doesn't fail the op.
    let compact: String = payload.chars().filter(|c| !c.is_whitespace()).collect();
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(compact.as_bytes())
        .context("the image source is not valid base64")?;
    if bytes.len() > MAX_IMAGE_SOURCE_BYTES {
        bail!(
            "the image is {} bytes; the limit is {MAX_IMAGE_SOURCE_BYTES}",
            bytes.len()
        );
    }
    Ok(bytes)
}

/// An image decoded once, ready to ingest: its content-hash id, the encoded
/// bytes that are persisted, and the pixels the renderer draws.
pub struct PreparedImage {
    id: AssetId,
    bytes: Vec<u8>,
    rgba: Vec<u8>,
    size: [u32; 2],
    format: Option<image::ImageFormat>,
}

impl PreparedImage {
    pub fn new(bytes: Vec<u8>) -> Result<Self> {
        anyhow::ensure!(
            bytes.len() <= MAX_IMAGE_SOURCE_BYTES,
            "the image is {} bytes; the limit is {MAX_IMAGE_SOURCE_BYTES} bytes",
            bytes.len()
        );
        let id = fanta_format::asset_id_for_bytes(&bytes);
        let decoded = image::load_from_memory(&bytes).context("decoding image bytes")?;
        let rgba = decoded.to_rgba8();
        let (width, height) = rgba.dimensions();
        anyhow::ensure!(width > 0 && height > 0, "the image has no pixels");
        let format = image::guess_format(&bytes).ok();
        Ok(Self {
            id,
            bytes,
            rgba: rgba.into_raw(),
            size: [width, height],
            format,
        })
    }

    /// The encoded bytes, as persisted.
    pub fn encoded(&self) -> &[u8] {
        &self.bytes
    }

    /// The encoded format, when it is one `image` recognizes.
    pub fn image_format(&self) -> Option<image::ImageFormat> {
        self.format
    }
}

/// Layers agent-ingested images over the resolver built at load, so placing
/// one image is O(1) instead of a re-decode of every embedded asset. Interior
/// mutability because the resolver is shared as an `Arc<dyn AssetResolver>`
/// with background renders; reads take the lock only on the overlay map.
pub struct OverlayAssetResolver {
    base: Option<Arc<dyn AssetResolver>>,
    added: RwLock<HashMap<AssetId, DecodedImage>>,
    added_bytes: RwLock<HashMap<AssetId, Arc<Vec<u8>>>>,
}

impl OverlayAssetResolver {
    /// Whether no ingested image is layered over the base resolver.
    pub fn is_empty(&self) -> bool {
        self.added
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .is_empty()
            && self
                .added_bytes
                .read()
                .unwrap_or_else(PoisonError::into_inner)
                .is_empty()
    }
}

impl AssetResolver for OverlayAssetResolver {
    fn resolve(&self, id: AssetId) -> Option<DecodedImage> {
        if let Some(image) = self
            .added
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&id)
        {
            return Some(image.clone());
        }
        self.base.as_ref()?.resolve(id)
    }

    fn resolve_bytes(&self, id: AssetId) -> Option<Arc<Vec<u8>>> {
        if let Some(bytes) = self
            .added_bytes
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&id)
        {
            return Some(bytes.clone());
        }
        self.base.as_ref()?.resolve_bytes(id)
    }

    fn image_size(&self, id: AssetId) -> Option<[u32; 2]> {
        if let Some(image) = self
            .added
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&id)
        {
            return Some([image.width, image.height]);
        }
        self.base.as_ref()?.image_size(id)
    }

    fn resolve_level_nonblocking(&self, id: AssetId, level: u32) -> LevelLookup {
        if let Some(image) = self
            .added
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&id)
        {
            return LevelLookup::Decoded(LeveledImage {
                image: image.clone(),
                level: 0,
            });
        }
        match &self.base {
            Some(base) => base.resolve_level_nonblocking(id, level),
            None => LevelLookup::Unavailable,
        }
    }

    fn decode_generation(&self) -> u64 {
        self.base
            .as_ref()
            .map_or(0, |base| base.decode_generation())
    }

    fn subscribe_decodes(&self, listener: Weak<dyn DecodeListener>) {
        if let Some(base) = &self.base {
            base.subscribe_decodes(listener);
        }
    }
}

/// Mutable views of every store an ingested image must reach: the raw bytes
/// (persisted on save) and the render resolver, with the overlay that layers
/// new images over it.
pub struct AssetStores<'a> {
    raw_assets: &'a mut Arc<BTreeMap<AssetId, Vec<u8>>>,
    asset_resolver: &'a mut Option<Arc<dyn AssetResolver>>,
    overlay: &'a mut Option<Arc<OverlayAssetResolver>>,
}

impl<'a> AssetStores<'a> {
    pub fn new(
        raw_assets: &'a mut Arc<BTreeMap<AssetId, Vec<u8>>>,
        asset_resolver: &'a mut Option<Arc<dyn AssetResolver>>,
        overlay: &'a mut Option<Arc<OverlayAssetResolver>>,
    ) -> Self {
        Self {
            raw_assets,
            asset_resolver,
            overlay,
        }
    }

    pub fn add_raw_bytes_tracked(&mut self, bytes: Vec<u8>) -> Result<(AssetId, bool)> {
        let id = fanta_format::asset_id_for_bytes(&bytes);
        if let Some(existing) = self.raw_assets.get(&id) {
            anyhow::ensure!(
                existing == &bytes,
                "asset {id} has a content hash collision"
            );
            return Ok((id, false));
        }
        Arc::make_mut(self.raw_assets).insert(id, bytes.clone());
        let overlay = self.overlay.get_or_insert_with(|| {
            Arc::new(OverlayAssetResolver {
                base: self.asset_resolver.clone(),
                added: RwLock::new(HashMap::default()),
                added_bytes: RwLock::new(HashMap::default()),
            })
        });
        overlay
            .added_bytes
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(id, Arc::new(bytes));
        *self.asset_resolver = Some(overlay.clone() as Arc<dyn AssetResolver>);
        Ok((id, true))
    }

    /// The final value says whether this call inserted a new asset, so a
    /// failed batch can roll it back without removing an existing duplicate.
    pub fn add_image_tracked(&mut self, bytes: Vec<u8>) -> Result<(AssetId, [u32; 2], bool)> {
        self.add_prepared_image_tracked(PreparedImage::new(bytes)?)
    }

    pub fn add_prepared_image_tracked(
        &mut self,
        prepared: PreparedImage,
    ) -> Result<(AssetId, [u32; 2], bool)> {
        let PreparedImage {
            id,
            bytes,
            rgba,
            size: [width, height],
            format: _,
        } = prepared;
        if let Some(existing) = self.raw_assets.get(&id) {
            anyhow::ensure!(
                existing == &bytes,
                "asset {id} has a content hash collision"
            );
            return Ok((id, [width, height], false));
        }

        self.add_raw_bytes_tracked(bytes)?;
        let overlay = self
            .overlay
            .as_ref()
            .context("ingested image has no resolver")?;
        overlay
            .added
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(id, DecodedImage::new(&rgba, width, height));
        *self.asset_resolver = Some(overlay.clone() as Arc<dyn AssetResolver>);
        Ok((id, [width, height], true))
    }

    /// Drop an ingested asset again — the rollback path when a batch fails
    /// after ingesting.
    pub fn remove(&mut self, id: AssetId) {
        if self.raw_assets.contains_key(&id) {
            Arc::make_mut(self.raw_assets).remove(&id);
        }
        if let Some(overlay) = self.overlay.as_ref() {
            overlay
                .added
                .write()
                .unwrap_or_else(PoisonError::into_inner)
                .remove(&id);
            overlay
                .added_bytes
                .write()
                .unwrap_or_else(PoisonError::into_inner)
                .remove(&id);
        }
    }
}

impl ImageStore for AssetStores<'_> {
    fn add_image_tracked(&mut self, bytes: Vec<u8>) -> Result<(AssetId, [u32; 2], bool)> {
        AssetStores::add_image_tracked(self, bytes)
    }

    fn remove(&mut self, asset: AssetId) {
        AssetStores::remove(self, asset);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = Vec::new();
        image::RgbaImage::from_pixel(width, height, image::Rgba([255, 0, 0, 255]))
            .write_to(
                &mut std::io::Cursor::new(&mut bytes),
                image::ImageFormat::Png,
            )
            .expect("encode png");
        bytes
    }

    #[test]
    fn ingested_images_resolve_through_the_overlay_and_roll_back() {
        let mut raw_assets = Arc::new(BTreeMap::new());
        let mut resolver = None;
        let mut overlay = None;
        let mut stores = AssetStores::new(&mut raw_assets, &mut resolver, &mut overlay);
        let bytes = png(3, 2);
        let (asset, size, inserted) = stores.add_image_tracked(bytes.clone()).expect("ingest");
        assert_eq!(size, [3, 2]);
        assert!(inserted);
        let (again, _, inserted_again) = stores.add_image_tracked(bytes).expect("dedupe");
        assert_eq!(again, asset);
        assert!(!inserted_again, "a duplicate is not a new insert");
        let drawable = resolver.as_ref().expect("resolver").resolve(asset);
        assert_eq!(
            drawable.map(|image| [image.width, image.height]),
            Some([3, 2])
        );

        let mut stores = AssetStores::new(&mut raw_assets, &mut resolver, &mut overlay);
        stores.remove(asset);
        assert!(raw_assets.is_empty());
        assert!(
            resolver
                .as_ref()
                .expect("resolver")
                .resolve(asset)
                .is_none()
        );
    }

    #[test]
    fn oversized_and_url_sources_are_refused() {
        let error = PreparedImage::new(vec![0; MAX_IMAGE_SOURCE_BYTES + 1])
            .err()
            .expect("oversized image refused");
        assert!(
            error
                .to_string()
                .contains(&MAX_IMAGE_SOURCE_BYTES.to_string())
        );
        assert!(decode_image_source("https://example.com/a.png").is_err());
        let encoded = base64::engine::general_purpose::STANDARD.encode(png(1, 1));
        let wrapped = format!("data:image/png;base64,{}\n{}", &encoded[..4], &encoded[4..]);
        assert_eq!(
            decode_image_source(&wrapped).expect("wrapped base64 decodes"),
            png(1, 1)
        );
    }
}
