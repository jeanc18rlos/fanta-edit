//! The `.fant` zip container — the FantaFile API surface.
//!
//! Design rationale
//!
//! - **Open it once, hold it in memory.** A Fantaisa session opens a `.fant`
//!   once and edits live; we keep manifest + doc + asset blobs hot in a single
//!   `FantaFile` value, then atomically rewrite the file on save. This is the
//!   shape Sketch and Affinity use: zip-at-rest, struct-in-memory.
//! - **Atomic writes.** Every save goes to a sibling `.tmp` file and renames
//!   over the target. A crash mid-save leaves the original intact.
//! - **Content addressing.** Asset IDs are derived from the SHA-256 of the
//!   bytes (first 128 bits packed into `AssetId::from_u128`). Two `put_asset`
//!   calls with identical bytes return the same id and only store one blob.
//! - **Path layout inside the zip.** `manifest.json`, `doc.json`, and
//!   `assets/<hex>.bin` are the core entries this version writes. `vcs/` may
//!   hold an embedded Git bundle for agent/version history; `previews/` is
//!   read-through (we accept it on load) but not produced.

use crate::error::{FormatError, Result};
use crate::manifest::Manifest;
use crate::migrate::migrate;
use fanta_doc::{AssetId, Doc, SCHEMA_VERSION};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{Cursor, Read, Seek, Write};
use std::path::{Path, PathBuf};

/// Name of the manifest entry inside the zip.
const MANIFEST_NAME: &str = "manifest.json";
/// Name of the doc projection entry inside the zip.
const DOC_NAME: &str = "doc.json";
/// Path prefix for asset blob entries.
const ASSETS_PREFIX: &str = "assets/";
/// Embedded Git bundle for the document's portable version history.
const VCS_BUNDLE_NAME: &str = "vcs/repo.bundle";

/// An open `.fant` container.
///
/// Holds the on-disk path, the parsed manifest, the last-known doc JSON (so we
/// can rewrite on save without re-running serialization unnecessarily), and
/// the in-memory asset table keyed by content hash. The struct is intentionally
/// `Sync`-friendly only at the field level — multi-threaded mutation isn't
/// supported; wrap in a `Mutex` if you need it.
#[derive(Debug)]
pub struct FantaFile {
    path: PathBuf,
    manifest: Manifest,
    /// Cached doc JSON string. Populated after `save_doc` or `load_doc`. None
    /// when a freshly-created container has not yet had a doc written.
    doc_json: Option<String>,
    /// Asset payloads keyed by [`AssetId`]. Loaded lazily from the zip on
    /// `open` so re-opening a 500 MB project doesn't blow memory; cached
    /// thereafter so repeated reads are O(1).
    assets: HashMap<AssetId, Vec<u8>>,
    /// Tracks IDs whose blobs have not yet been hydrated from the on-disk zip.
    /// Allows lazy reads while still letting `list_assets` return the full set.
    asset_paths_pending: HashMap<AssetId, String>,
    /// Optional embedded Git bundle. Fanta's live/app layer owns the Git
    /// semantics; the format crate only persists the opaque bundle bytes.
    vcs_bundle: Option<Vec<u8>>,
}

impl FantaFile {
    /// Create a brand-new `.fant` at `path`. Writes a minimal zip containing
    /// only the manifest; the caller is expected to follow up with
    /// [`Self::save_doc`] and [`Self::put_asset`] before the file is useful.
    ///
    /// We commit the empty container to disk immediately so that crashes
    /// before the first explicit save still leave a well-formed (if empty)
    /// archive on disk — easier for tooling and the OS file picker.
    pub fn create(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        let manifest = Manifest::new(SCHEMA_VERSION, "");
        let mut me = Self {
            path,
            manifest,
            doc_json: None,
            assets: HashMap::new(),
            asset_paths_pending: HashMap::new(),
            vcs_bundle: None,
        };
        me.flush()?;
        Ok(me)
    }

    /// Open an existing `.fant`. Reads the manifest into memory and indexes the
    /// asset paths without decompressing the blobs themselves — the actual
    /// bytes are loaded on first [`Self::get_asset`] call. This keeps cold open
    /// cheap regardless of project size.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        let file = File::open(&path)?;
        let mut zip = zip::ZipArchive::new(file)?;

        // Read the manifest.
        let manifest = read_manifest(&mut zip)?;

        // We refuse to load files from a newer Fantaisa — the format may have
        // gained fields we don't understand and we don't want to silently
        // truncate them on save.
        if manifest.schema_version > SCHEMA_VERSION {
            return Err(FormatError::UnsupportedSchema {
                found: manifest.schema_version,
                supported: SCHEMA_VERSION,
            });
        }

        // Eagerly load the doc body — it's typically <1 MB even for large
        // projects (assets are stored separately) and having it in memory lets
        // us re-serialize quickly on save.
        let doc_json = read_optional_entry(&mut zip, DOC_NAME)?;
        let vcs_bundle = read_optional_binary_entry(&mut zip, VCS_BUNDLE_NAME)?;

        // Index assets without loading them. We trust the manifest's asset
        // table as the source of truth; entries on disk without a manifest
        // pointer are ignored (and dropped on next save).
        let mut asset_paths_pending = HashMap::new();
        for (id_str, path) in &manifest.assets {
            let id: AssetId = id_str.parse().map_err(|e: fanta_doc::IdParseError| {
                FormatError::InvalidManifest(format!(
                    "asset id {id_str:?} is not a valid AssetId: {e}"
                ))
            })?;
            asset_paths_pending.insert(id, path.clone());
        }

        Ok(Self {
            path,
            manifest,
            doc_json,
            assets: HashMap::new(),
            asset_paths_pending,
            vcs_bundle,
        })
    }

    /// Borrow the manifest. Callers reach for this to read app version,
    /// timestamps, or the asset table without forcing a doc load.
    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    /// Serialize a [`Doc`] into the container and persist atomically. Updates
    /// the manifest's `doc_id` and `modified_at` to match.
    pub fn save_doc(&mut self, doc: &Doc) -> Result<()> {
        let json = doc.to_json_string()?;
        self.doc_json = Some(json);
        self.manifest.schema_version = SCHEMA_VERSION;
        self.manifest.doc_id = doc.id.to_string();
        self.manifest.touch();
        self.flush()
    }

    /// Deserialize the stored doc, running schema migrations if needed.
    pub fn load_doc(&self) -> Result<Doc> {
        let raw = self
            .doc_json
            .as_deref()
            .ok_or_else(|| FormatError::MissingFile {
                name: DOC_NAME.to_string(),
            })?;

        let value: serde_json::Value = serde_json::from_str(raw)?;
        let migrated = migrate(value, self.manifest.schema_version, SCHEMA_VERSION)?;
        let migrated_str = serde_json::to_string(&migrated)?;
        // We funnel through `Doc::from_json_str` because it rebuilds the
        // child index and runs scene validation — we never want to hand a
        // malformed `Doc` to the rest of the app.
        Doc::from_json_str(&migrated_str)
            .map_err(|e| FormatError::InvalidManifest(format!("doc body invalid: {e}")))
    }

    /// Store a blob and return the deduplicated [`AssetId`]. The id is derived
    /// from the SHA-256 of the bytes (first 128 bits), so identical content
    /// always yields the same id and we only keep one copy.
    pub fn put_asset(&mut self, bytes: &[u8]) -> Result<AssetId> {
        let (id, hash_hex) = compute_asset_id(bytes);
        if !self.has_asset(id) {
            let path = format!("{ASSETS_PREFIX}{hash_hex}.bin");
            self.assets.insert(id, bytes.to_vec());
            self.manifest.set_asset(id, path);
            self.flush()?;
        }
        Ok(id)
    }

    /// Store a blob under a caller-supplied id. For assets whose ids are NOT
    /// content-addressed (e.g. minted by the `.fig` importer): the doc's
    /// `Fill::Image` references carry that exact id, so re-minting via
    /// [`Self::put_asset`] would orphan them. Content producers should prefer
    /// `put_asset`'s dedup.
    pub fn put_asset_as(&mut self, id: AssetId, bytes: &[u8]) -> Result<()> {
        if !self.has_asset(id) {
            let path = format!("{ASSETS_PREFIX}{id}.bin");
            self.assets.insert(id, bytes.to_vec());
            self.manifest.set_asset(id, path);
            self.flush()?;
        }
        Ok(())
    }

    /// Read an asset blob by id, decompressing it from the zip on first access.
    pub fn get_asset(&self, id: AssetId) -> Result<Vec<u8>> {
        // Hot path: already in memory.
        if let Some(b) = self.assets.get(&id) {
            return Ok(b.clone());
        }
        // Lazy load from disk.
        if let Some(entry_path) = self.asset_paths_pending.get(&id).cloned() {
            return self.read_asset_from_disk(&entry_path, id);
        }
        Err(FormatError::AssetNotFound(id))
    }

    /// All assets known to this container — both those already hydrated into
    /// memory and those still on disk. The set is the manifest's asset table.
    pub fn list_assets(&self) -> Vec<AssetId> {
        let mut out: Vec<AssetId> = self.assets.keys().copied().collect();
        for id in self.asset_paths_pending.keys() {
            if !self.assets.contains_key(id) {
                out.push(*id);
            }
        }
        out.sort_by_key(|a| a.to_u128());
        out
    }

    /// Borrow the embedded Git bundle, if this container carries one.
    pub fn vcs_bundle(&self) -> Option<&[u8]> {
        self.vcs_bundle.as_deref()
    }

    /// Replace (or clear) the embedded Git bundle and persist atomically.
    ///
    /// The app layer is responsible for ensuring the bundle was created from a
    /// valid repository; the format layer treats it as an opaque binary blob.
    pub fn set_vcs_bundle(&mut self, bundle: Option<Vec<u8>>) -> Result<()> {
        self.vcs_bundle = bundle;
        self.flush()
    }

    /// Path on disk for this container.
    pub fn path(&self) -> &Path {
        &self.path
    }

    // ---- internals -------------------------------------------------------

    /// Is the asset already accounted for (either hydrated or pending)?
    fn has_asset(&self, id: AssetId) -> bool {
        self.assets.contains_key(&id) || self.asset_paths_pending.contains_key(&id)
    }

    /// Lazily decompress an asset blob from the on-disk zip.
    fn read_asset_from_disk(&self, entry_path: &str, id: AssetId) -> Result<Vec<u8>> {
        let file = File::open(&self.path)?;
        let mut zip = zip::ZipArchive::new(file)?;
        let mut e = zip
            .by_name(entry_path)
            .map_err(|_| FormatError::AssetNotFound(id))?;
        let mut buf = Vec::with_capacity(e.size() as usize);
        e.read_to_end(&mut buf)?;
        Ok(buf)
    }

    /// Write the in-memory state back to disk atomically.
    ///
    /// We assemble the new archive in memory (typically tiny — manifest +
    /// small doc JSON + hashed assets) then write to a temp file and rename.
    /// This is the right call for v0: simple, atomic, and crash-safe; the
    /// "incremental rewrite" path described in §8 of ARCHITECTURE is a perf
    /// optimization for later.
    fn flush(&mut self) -> Result<()> {
        // Fully hydrate any lazy assets before we rebuild the zip — otherwise
        // we'd lose them on rewrite. This only fires for the (rare) case
        // where the caller mutated the container after open without ever
        // touching the asset; in the common case the set is empty.
        let pending_ids: Vec<AssetId> = self
            .asset_paths_pending
            .keys()
            .copied()
            .filter(|id| !self.assets.contains_key(id))
            .collect();
        for id in pending_ids {
            let entry = self
                .asset_paths_pending
                .get(&id)
                .expect("pending entry exists")
                .clone();
            let bytes = self.read_asset_from_disk(&entry, id)?;
            self.assets.insert(id, bytes);
        }

        let buf = self.encode_zip_bytes()?;

        // Atomic rename: write `tmp_path`, then rename onto `self.path`. The
        // rename is atomic on POSIX and on Windows when the target lives on
        // the same volume (it always does here — same directory).
        if let Some(parent) = self.path.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent)?;
        }
        let tmp_path = self.path.with_extension("fant.tmp");
        {
            let mut f = File::create(&tmp_path)?;
            f.write_all(&buf)?;
            f.sync_all()?;
        }
        fs::rename(&tmp_path, &self.path)?;
        Ok(())
    }

    /// Build the zip archive in memory, with manifest, doc, and assets.
    fn encode_zip_bytes(&self) -> Result<Vec<u8>> {
        let mut cursor = Cursor::new(Vec::<u8>::new());
        {
            let mut zw = zip::ZipWriter::new(&mut cursor);
            // Stored (uncompressed) for the manifest and doc — they're tiny
            // and the lookups should be cheap. Use deflate for assets where
            // the size win usually outweighs the CPU cost.
            let stored: zip::write::SimpleFileOptions = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            let deflated: zip::write::SimpleFileOptions = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);

            // Manifest first — by convention the "header" of a `.fant`.
            zw.start_file(MANIFEST_NAME, stored)?;
            let manifest_json = serde_json::to_vec_pretty(&self.manifest)?;
            zw.write_all(&manifest_json)?;

            // Doc projection (if any).
            if let Some(s) = &self.doc_json {
                zw.start_file(DOC_NAME, stored)?;
                zw.write_all(s.as_bytes())?;
            }

            // Assets, ordered by id so the archive bytes are deterministic.
            let mut ids: Vec<AssetId> = self.assets.keys().copied().collect();
            ids.sort_by_key(|a| a.to_u128());
            for id in ids {
                let path = self
                    .manifest
                    .asset_path(id)
                    .ok_or(FormatError::AssetNotFound(id))?
                    .to_string();
                let blob = self
                    .assets
                    .get(&id)
                    .expect("asset id from manifest must be present");
                zw.start_file(&path, deflated)?;
                zw.write_all(blob)?;
            }

            if let Some(bundle) = &self.vcs_bundle {
                zw.start_file(VCS_BUNDLE_NAME, deflated)?;
                zw.write_all(bundle)?;
            }

            zw.finish()?;
        }
        Ok(cursor.into_inner())
    }
}

/// Read and parse the manifest from a zip archive.
fn read_manifest<R: Read + Seek>(zip: &mut zip::ZipArchive<R>) -> Result<Manifest> {
    let mut entry = match zip.by_name(MANIFEST_NAME) {
        Ok(e) => e,
        Err(zip::result::ZipError::FileNotFound) => {
            return Err(FormatError::MissingFile {
                name: MANIFEST_NAME.to_string(),
            });
        }
        Err(e) => return Err(FormatError::Zip(e)),
    };
    let mut buf = String::new();
    entry.read_to_string(&mut buf)?;
    // Try a strict parse first; surface anything more specific than "Json"
    // as an `InvalidManifest` so callers can disambiguate "this zip has a
    // garbled manifest" from "this zip has a manifest with one field of the
    // wrong type".
    serde_json::from_str(&buf).map_err(|e| {
        if e.classify() == serde_json::error::Category::Data {
            FormatError::InvalidManifest(e.to_string())
        } else {
            FormatError::Json(e)
        }
    })
}

/// Read an optional entry as a UTF-8 string. Returns `None` if the entry is
/// missing (used for `doc.json` because a fresh container won't have one yet).
fn read_optional_entry<R: Read + Seek>(
    zip: &mut zip::ZipArchive<R>,
    name: &str,
) -> Result<Option<String>> {
    match zip.by_name(name) {
        Ok(mut e) => {
            let mut s = String::new();
            e.read_to_string(&mut s)?;
            Ok(Some(s))
        }
        Err(zip::result::ZipError::FileNotFound) => Ok(None),
        Err(e) => Err(FormatError::Zip(e)),
    }
}

/// Read an optional binary entry. Returns `None` if the entry is missing.
fn read_optional_binary_entry<R: Read + Seek>(
    zip: &mut zip::ZipArchive<R>,
    name: &str,
) -> Result<Option<Vec<u8>>> {
    match zip.by_name(name) {
        Ok(mut e) => {
            let mut bytes = Vec::with_capacity(e.size() as usize);
            e.read_to_end(&mut bytes)?;
            Ok(Some(bytes))
        }
        Err(zip::result::ZipError::FileNotFound) => Ok(None),
        Err(e) => Err(FormatError::Zip(e)),
    }
}

/// The content-addressed [`AssetId`] for `bytes` — the same id [`FantaFile::put_asset`]
/// would assign. Lets a caller mint an id (e.g. for an in-memory image about to
/// be placed) that will match the container on save, so the asset round-trips
/// without an id-remap.
pub fn asset_id_for_bytes(bytes: &[u8]) -> AssetId {
    compute_asset_id(bytes).0
}

/// SHA-256 the bytes and turn the digest into both an [`AssetId`] (top 128
/// bits packed into `from_u128`) and the hex string used as the on-disk
/// filename.
fn compute_asset_id(bytes: &[u8]) -> (AssetId, String) {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    let mut first_16 = [0u8; 16];
    first_16.copy_from_slice(&digest[..16]);
    let id = AssetId::from_u128(u128::from_be_bytes(first_16));
    let hex = digest
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    (id, hex)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compute_asset_id_is_deterministic() {
        let (a, ha) = compute_asset_id(b"hello world");
        let (b, hb) = compute_asset_id(b"hello world");
        assert_eq!(a, b);
        assert_eq!(ha, hb);
    }

    #[test]
    fn compute_asset_id_distinguishes_bytes() {
        let (a, _) = compute_asset_id(b"hello world");
        let (b, _) = compute_asset_id(b"goodbye world");
        assert_ne!(a, b);
    }
}
