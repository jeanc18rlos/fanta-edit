//! Media-type sniffing and integrity for the `assets/` tree.
//!
//! Assets are sorted into human-browsable family folders
//! (`assets/images/…`, `assets/models/…`) with a real extension, purely from
//! their **magic bytes** — the project tree is the deliverable, so a game-team
//! recipient should be able to browse it without Fantaisa (spec 09 user
//! stories). Folder and extension are *projections*: on read the id encoded in
//! the filename is the only thing that matters, so a mis-sniffed (or manually
//! moved) asset still round-trips losslessly.

use crate::error::{FormatError, Result};
use crate::formats::FormatCapabilities;
use fanta_doc::AssetId;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

/// A project created before the integrity index may contain non-hash asset
/// ids from an importer. An absent index remains readable; the next save
/// records digests without changing ids referenced by the design source.
pub(crate) const ASSET_INDEX_FILE: &str = "index.json";
const ASSET_INDEX_VERSION: u32 = 1;

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AssetIndexFile {
    version: u32,
    assets: BTreeMap<String, AssetRecord>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AssetRecord {
    size: u64,
    sha256: String,
}

/// Full digests for all binary files in a project. The first 128 SHA-256 bits
/// are used for newly minted `AssetId`s, while the full digest detects damage
/// to existing files and preserves legacy ids on migration.
#[derive(Debug)]
pub(crate) struct AssetIndex {
    assets: BTreeMap<AssetId, AssetRecord>,
}

impl AssetIndex {
    /// Check one asset as it is read, so a corrupt file never reaches the doc.
    pub(crate) fn verify_entry(&self, id: AssetId, bytes: &[u8]) -> Result<()> {
        let record = self.assets.get(&id).ok_or_else(|| {
            FormatError::InvalidProjectTree(format!("asset {id} is missing from assets/index.json"))
        })?;
        if record.size != bytes.len() as u64 || record.sha256 != sha256_hex(bytes) {
            return Err(FormatError::InvalidProjectTree(format!(
                "asset {id} does not match assets/index.json"
            )));
        }
        Ok(())
    }

    /// Check membership after all asset files have been collected. Callers
    /// should have already called `verify_entry` for each file.
    pub(crate) fn verify_complete(&self, assets: &BTreeMap<AssetId, Vec<u8>>) -> Result<()> {
        if let Some(id) = self.assets.keys().find(|id| !assets.contains_key(id)) {
            return Err(FormatError::InvalidProjectTree(format!(
                "asset {id} is listed in assets/index.json but its file is missing"
            )));
        }
        Ok(())
    }
}

/// Deterministic index bytes for the project's assets. `BTreeMap` ordering
/// and a trailing newline keep Git diffs stable across machines.
pub(crate) fn asset_index_bytes(assets: &BTreeMap<AssetId, Vec<u8>>) -> Result<Vec<u8>> {
    let file = AssetIndexFile {
        version: ASSET_INDEX_VERSION,
        assets: assets
            .iter()
            .map(|(id, bytes)| {
                (
                    id.to_string(),
                    AssetRecord {
                        size: bytes.len() as u64,
                        sha256: sha256_hex(bytes),
                    },
                )
            })
            .collect(),
    };
    let mut json = serde_json::to_vec_pretty(&file)?;
    json.push(b'\n');
    Ok(json)
}

/// Parse and validate the index structure before reading any asset bodies.
pub(crate) fn read_asset_index(bytes: &[u8]) -> Result<AssetIndex> {
    let file: AssetIndexFile = serde_json::from_slice(bytes).map_err(|error| {
        FormatError::InvalidProjectTree(format!("assets/index.json is invalid: {error}"))
    })?;
    if file.version != ASSET_INDEX_VERSION {
        return Err(FormatError::InvalidProjectTree(format!(
            "unsupported assets/index.json version: {}",
            file.version
        )));
    }
    let mut assets = BTreeMap::new();
    for (name, record) in file.assets {
        let id = name.parse::<AssetId>().map_err(|error| {
            FormatError::InvalidProjectTree(format!(
                "assets/index.json contains invalid asset id {name:?}: {error}"
            ))
        })?;
        if record.sha256.len() != 64
            || !record
                .sha256
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(FormatError::InvalidProjectTree(format!(
                "assets/index.json contains invalid SHA-256 for {id}"
            )));
        }
        if assets.insert(id, record).is_some() {
            return Err(FormatError::InvalidProjectTree(format!(
                "assets/index.json contains duplicate asset id {id}"
            )));
        }
    }
    Ok(AssetIndex { assets })
}

fn sha256_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let digest = Sha256::digest(bytes);
    let mut result = String::with_capacity(64);
    for byte in digest {
        result.push(HEX[usize::from(byte >> 4)] as char);
        result.push(HEX[usize::from(byte & 0x0f)] as char);
    }
    result
}

/// Family folders under `assets/`, in the order the scaffold creates them.
pub(crate) const MEDIA_DIRS: [&str; 7] = [
    "images", "video", "audio", "models", "svg", "fonts", "other",
];

/// A binary format that can be recognized from a bounded header probe.
/// Additional formats can register a probe without changing project I/O.
#[derive(Debug, Clone, Copy)]
pub struct MediaFormat {
    pub id: &'static str,
    pub family: &'static str,
    pub extension: &'static str,
    pub capabilities: FormatCapabilities,
    pub probe: fn(&[u8]) -> bool,
}

#[derive(Debug, thiserror::Error)]
pub enum MediaRegistryError {
    #[error("media format id {0:?} is already registered")]
    DuplicateId(&'static str),
}

/// Ordered probes. A specific signature must register before a broader one.
pub struct MediaRegistry {
    formats: Vec<MediaFormat>,
}

impl MediaRegistry {
    pub fn new() -> Self {
        Self {
            formats: Vec::new(),
        }
    }

    pub fn with_builtins() -> Self {
        Self {
            formats: BUILTIN_MEDIA.to_vec(),
        }
    }

    pub fn register(&mut self, format: MediaFormat) -> std::result::Result<(), MediaRegistryError> {
        if self.formats.iter().any(|existing| existing.id == format.id) {
            return Err(MediaRegistryError::DuplicateId(format.id));
        }
        self.formats.push(format);
        Ok(())
    }

    pub fn formats(&self) -> impl Iterator<Item = MediaFormat> + '_ {
        self.formats.iter().copied()
    }

    pub fn sniff(&self, bytes: &[u8]) -> Option<MediaFormat> {
        sniff_in(&self.formats, bytes)
    }
}

impl Default for MediaRegistry {
    fn default() -> Self {
        Self::with_builtins()
    }
}

const fn capabilities(display: bool) -> FormatCapabilities {
    FormatCapabilities {
        import: true,
        export: true,
        display,
        edit: false,
        version: true,
    }
}

const fn media_format(
    id: &'static str,
    family: &'static str,
    extension: &'static str,
    display: bool,
    probe: fn(&[u8]) -> bool,
) -> MediaFormat {
    MediaFormat {
        id,
        family,
        extension,
        capabilities: capabilities(display),
        probe,
    }
}

const BUILTIN_MEDIA: &[MediaFormat] = &[
    media_format("png", "images", "png", true, is_png),
    media_format("jpeg", "images", "jpg", true, is_jpeg),
    media_format("gif", "images", "gif", true, is_gif),
    media_format("webp", "images", "webp", true, is_webp),
    media_format("wav", "audio", "wav", true, is_wav),
    media_format("quicktime", "video", "mov", false, is_quicktime),
    media_format("mp4", "video", "mp4", false, is_mp4),
    media_format("webm", "video", "webm", false, is_webm),
    media_format("mp3-id3", "audio", "mp3", false, is_id3_mp3),
    media_format("ogg", "audio", "ogg", false, is_ogg),
    media_format("flac", "audio", "flac", false, is_flac),
    media_format("glb", "models", "glb", false, is_glb),
    media_format("ttf", "fonts", "ttf", false, is_ttf),
    media_format("otf", "fonts", "otf", false, is_otf),
    media_format("woff", "fonts", "woff", false, is_woff),
    media_format("woff2", "fonts", "woff2", false, is_woff2),
    media_format("svg", "svg", "svg", false, is_svg),
    media_format("gltf", "models", "gltf", false, is_gltf),
    media_format("mp3-raw", "audio", "mp3", false, is_raw_mp3),
];

fn sniff_in(formats: &[MediaFormat], bytes: &[u8]) -> Option<MediaFormat> {
    let header = &bytes[..bytes.len().min(512)];
    formats
        .iter()
        .copied()
        .find(|format| (format.probe)(header))
}

/// Sniff `(family folder, extension)` from the leading bytes of an asset.
/// Unknown content falls back to `("other", "bin")`.
pub(crate) fn sniff_media(bytes: &[u8]) -> (&'static str, &'static str) {
    sniff_in(BUILTIN_MEDIA, bytes)
        .map(|format| (format.family, format.extension))
        .unwrap_or(("other", "bin"))
}

fn is_png(bytes: &[u8]) -> bool {
    bytes.starts_with(b"\x89PNG\r\n\x1a\n")
}

fn is_jpeg(bytes: &[u8]) -> bool {
    bytes.starts_with(&[0xFF, 0xD8, 0xFF])
}

fn is_gif(bytes: &[u8]) -> bool {
    bytes.starts_with(b"GIF8")
}

fn riff_kind(bytes: &[u8], kind: &[u8; 4]) -> bool {
    bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == kind
}

fn is_webp(bytes: &[u8]) -> bool {
    riff_kind(bytes, b"WEBP")
}

fn is_wav(bytes: &[u8]) -> bool {
    riff_kind(bytes, b"WAVE")
}

fn is_mp4_family(bytes: &[u8]) -> bool {
    bytes.len() >= 12 && &bytes[4..8] == b"ftyp"
}

fn is_quicktime(bytes: &[u8]) -> bool {
    is_mp4_family(bytes) && &bytes[8..10] == b"qt"
}

fn is_mp4(bytes: &[u8]) -> bool {
    is_mp4_family(bytes)
}

fn is_webm(bytes: &[u8]) -> bool {
    bytes.starts_with(&[0x1A, 0x45, 0xDF, 0xA3])
}

fn is_id3_mp3(bytes: &[u8]) -> bool {
    bytes.starts_with(b"ID3")
}

fn is_ogg(bytes: &[u8]) -> bool {
    bytes.starts_with(b"OggS")
}

fn is_flac(bytes: &[u8]) -> bool {
    bytes.starts_with(b"fLaC")
}

fn is_glb(bytes: &[u8]) -> bool {
    bytes.starts_with(b"glTF")
}

fn is_ttf(bytes: &[u8]) -> bool {
    bytes.starts_with(&[0x00, 0x01, 0x00, 0x00])
}

fn is_otf(bytes: &[u8]) -> bool {
    bytes.starts_with(b"OTTO")
}

fn is_woff(bytes: &[u8]) -> bool {
    bytes.starts_with(b"wOFF")
}

fn is_woff2(bytes: &[u8]) -> bool {
    bytes.starts_with(b"wOF2")
}

fn is_svg(bytes: &[u8]) -> bool {
    let head = String::from_utf8_lossy(bytes);
    let trimmed = head.trim_start();
    trimmed.starts_with("<svg") || (trimmed.starts_with("<?xml") && head.contains("<svg"))
}

fn is_gltf(bytes: &[u8]) -> bool {
    let head = String::from_utf8_lossy(bytes);
    head.trim_start().starts_with('{') && head.contains("\"asset\"")
}

fn is_raw_mp3(bytes: &[u8]) -> bool {
    bytes.len() >= 2 && bytes[0] == 0xFF && bytes[1] & 0xE0 == 0xE0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn is_test_asset(header: &[u8]) -> bool {
        assert!(header.len() <= 512);
        header.starts_with(b"TEST")
    }

    #[test]
    fn media_registry_supports_added_format_with_bounded_probe() {
        let mut registry = MediaRegistry::with_builtins();
        registry
            .register(MediaFormat {
                id: "test-format",
                family: "other",
                extension: "test",
                capabilities: capabilities(false),
                probe: is_test_asset,
            })
            .unwrap();
        let mut bytes = vec![0; 4096];
        bytes[..4].copy_from_slice(b"TEST");
        let format = registry.sniff(&bytes).unwrap();
        assert_eq!(format.id, "test-format");
        assert_eq!(format.extension, "test");
        assert!(format.capabilities.import);
        assert!(format.capabilities.version);
        assert!(matches!(
            registry.register(format),
            Err(MediaRegistryError::DuplicateId("test-format"))
        ));
    }

    #[test]
    fn index_is_deterministic_and_preserves_legacy_ids() {
        let first = AssetId::from_u128(7);
        let second = AssetId::from_u128(3);
        let assets = BTreeMap::from([(first, b"first".to_vec()), (second, b"second".to_vec())]);
        let bytes = asset_index_bytes(&assets).unwrap();
        assert_eq!(bytes, asset_index_bytes(&assets).unwrap());
        assert_eq!(bytes.last(), Some(&b'\n'));
        let index = read_asset_index(&bytes).unwrap();
        for (id, payload) in &assets {
            index.verify_entry(*id, payload).unwrap();
        }
        index.verify_complete(&assets).unwrap();
    }

    #[test]
    fn index_rejects_same_length_corruption() {
        let id = AssetId::from_u128(23);
        let assets = BTreeMap::from([(id, b"asset".to_vec())]);
        let index = read_asset_index(&asset_index_bytes(&assets).unwrap()).unwrap();
        assert!(matches!(
            index.verify_entry(id, b"other"),
            Err(FormatError::InvalidProjectTree(_))
        ));
    }

    #[test]
    fn index_rejects_extra_or_missing_assets() {
        let id = AssetId::from_u128(23);
        let assets = BTreeMap::from([(id, b"asset".to_vec())]);
        let index = read_asset_index(&asset_index_bytes(&assets).unwrap()).unwrap();
        assert!(matches!(
            index.verify_entry(AssetId::from_u128(24), b"unlisted"),
            Err(FormatError::InvalidProjectTree(_))
        ));
        assert!(matches!(
            index.verify_complete(&BTreeMap::new()),
            Err(FormatError::InvalidProjectTree(_))
        ));
    }

    #[test]
    fn index_rejects_malformed_digest_and_unknown_version() {
        let id = AssetId::from_u128(23);
        let assets = BTreeMap::from([(id, b"asset".to_vec())]);
        let bytes = asset_index_bytes(&assets).unwrap();
        let mut value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        value["assets"][id.to_string().as_str()]["sha256"] = "not a digest".into();
        assert!(matches!(
            read_asset_index(&serde_json::to_vec(&value).unwrap()),
            Err(FormatError::InvalidProjectTree(_))
        ));
        value["version"] = 2.into();
        assert!(matches!(
            read_asset_index(&serde_json::to_vec(&value).unwrap()),
            Err(FormatError::InvalidProjectTree(_))
        ));
    }

    #[test]
    fn sniffs_every_family() {
        let cases: &[(&[u8], (&str, &str))] = &[
            (b"\x89PNG\r\n\x1a\n....", ("images", "png")),
            (&[0xFF, 0xD8, 0xFF, 0xE0, 0x00], ("images", "jpg")),
            (b"GIF89a..", ("images", "gif")),
            (b"RIFF\x10\x00\x00\x00WEBPVP8 ", ("images", "webp")),
            (b"RIFF\x10\x00\x00\x00WAVEfmt ", ("audio", "wav")),
            (b"\x00\x00\x00\x18ftypisom....", ("video", "mp4")),
            (b"\x00\x00\x00\x14ftypqt  ....", ("video", "mov")),
            (&[0x1A, 0x45, 0xDF, 0xA3, 0x01], ("video", "webm")),
            (b"ID3\x03\x00....", ("audio", "mp3")),
            (&[0xFF, 0xFB, 0x90, 0x00], ("audio", "mp3")),
            (b"OggS....", ("audio", "ogg")),
            (b"fLaC....", ("audio", "flac")),
            (b"glTF\x02\x00\x00\x00", ("models", "glb")),
            (br#"{"asset":{"version":"2.0"}}"#, ("models", "gltf")),
            (b"<svg xmlns='x'></svg>", ("svg", "svg")),
            (b"<?xml version=\"1.0\"?><svg/>", ("svg", "svg")),
            (&[0x00, 0x01, 0x00, 0x00, 0x00], ("fonts", "ttf")),
            (b"OTTO....", ("fonts", "otf")),
            (b"wOFF....", ("fonts", "woff")),
            (b"wOF2....", ("fonts", "woff2")),
            (b"plain unknown bytes", ("other", "bin")),
            (b"", ("other", "bin")),
        ];
        for (bytes, expected) in cases {
            assert_eq!(sniff_media(bytes), *expected, "bytes: {bytes:?}");
        }
    }
}
