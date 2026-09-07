//! The `manifest.json` entry inside a `.fant` archive.
//!
//! The manifest is the entry point: anyone — Fantaisa itself, an external
//! agent, a file picker that wants a thumbnail — can read it without touching
//! the doc body or unzipping assets. Keeping it separate from `doc.json` means
//! cheap metadata reads and lets us evolve the manifest shape independently of
//! the scene graph schema.

use fanta_doc::AssetId;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Manifest stored at the top of every `.fant` container.
///
/// The `BTreeMap` for `assets` is deliberate: deterministic ordering means two
/// otherwise-identical containers produce byte-identical manifests, which
/// matters for version control diffs and content-addressed deployment.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    /// Mirrors [`fanta_doc::SCHEMA_VERSION`] at the moment the file was written.
    /// Loaders compare this to their own constant and either accept, migrate,
    /// or reject.
    pub schema_version: u32,

    /// Identifier of the document stored in `doc.json`. Lets a Manifest reader
    /// know what document a sidecar (e.g. a `.fant.preview.png`) belongs to.
    pub doc_id: String,

    /// Fantaisa application version that wrote the file. Diagnostic only —
    /// loaders never branch on this. Tracking it lets bug reports correlate
    /// container shape with shipped builds.
    pub app_version: String,

    /// Map from [`AssetId`] (stringified) to the path of the blob inside the
    /// zip — typically `assets/<sha256_hex>.bin`. Keeping the mapping in the
    /// manifest rather than scanning the zip's central directory makes asset
    /// lookups O(1) without a directory walk.
    #[serde(default)]
    pub assets: BTreeMap<String, String>,

    /// Unix epoch seconds at which the file was first created.
    pub created_at: i64,

    /// Unix epoch seconds at which the file was last saved.
    pub modified_at: i64,
}

impl Manifest {
    /// Construct an empty manifest for a brand-new container. The asset index
    /// starts empty; `put_asset` adds entries as blobs are written.
    pub fn new(schema_version: u32, doc_id: impl Into<String>) -> Self {
        let now = unix_seconds_now();
        Self {
            schema_version,
            doc_id: doc_id.into(),
            app_version: env!("CARGO_PKG_VERSION").to_string(),
            assets: BTreeMap::new(),
            created_at: now,
            modified_at: now,
        }
    }

    /// Insert / update the relative path for an asset.
    pub fn set_asset(&mut self, id: AssetId, path: impl Into<String>) {
        self.assets.insert(id.to_string(), path.into());
    }

    /// Look up the relative path for an asset by id. Returns `None` if the
    /// asset isn't in the index — callers should map that to
    /// [`crate::FormatError::AssetNotFound`].
    pub fn asset_path(&self, id: AssetId) -> Option<&str> {
        self.assets.get(&id.to_string()).map(String::as_str)
    }

    /// Stamp the modified-at time to now. Called by the container on every save.
    pub fn touch(&mut self) {
        self.modified_at = unix_seconds_now();
    }
}

fn unix_seconds_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_round_trips_through_json() {
        let mut m = Manifest::new(1, "d_test");
        let id = AssetId::from_u128(42);
        m.set_asset(id, "assets/foo.bin");
        let s = serde_json::to_string(&m).unwrap();
        let back: Manifest = serde_json::from_str(&s).unwrap();
        assert_eq!(back, m);
        assert_eq!(back.asset_path(id), Some("assets/foo.bin"));
    }

    #[test]
    fn touch_updates_modified_at() {
        let mut m = Manifest::new(1, "d_test");
        let original = m.modified_at;
        m.modified_at = 0;
        m.touch();
        // We can't rely on `> original` because seconds resolution may equal,
        // but we can verify it moved off the manually-set 0.
        assert!(m.modified_at >= original);
    }
}
