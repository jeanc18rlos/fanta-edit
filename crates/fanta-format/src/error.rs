//! Errors surfaced by the `.fant` container.
//!
//! Split into its own module so call sites can `use fanta_format::FormatError`
//! without pulling in the rest of the crate. Every error path the public API
//! exposes is enumerated here — anything new must be added to this enum, never
//! flattened to `Io(_)` or boxed.

use fanta_doc::AssetId;

/// One enum for every failure shape `fanta-format` can produce.
///
/// We deliberately distinguish I/O, ZIP, JSON, and semantic errors so callers
/// can react meaningfully — a UI can offer "open as read-only" for a
/// `Zip(_)` mid-write crash but "report a bug" for `InvalidManifest`.
#[derive(Debug, thiserror::Error)]
pub enum FormatError {
    /// Wrapped `std::io::Error`. Filesystem permission denied, disk full, etc.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    /// The underlying zip library produced an error — corrupt archive,
    /// unsupported compression, truncated central directory.
    #[error("zip error: {0}")]
    Zip(#[from] zip::result::ZipError),

    /// JSON parse error while reading the manifest or the doc projection.
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    /// Container was written by a newer Fantaisa than this one understands.
    /// We refuse to load rather than silently corrupting on save.
    #[error("unsupported schema version: found {found}, this build supports {supported}")]
    UnsupportedSchema { found: u32, supported: u32 },

    /// A required entry was missing from the archive (e.g. no `manifest.json`).
    #[error("required file missing from container: {name}")]
    MissingFile { name: String },

    /// An asset was requested that isn't present in the container.
    #[error("asset not found: {0}")]
    AssetNotFound(AssetId),

    /// The manifest parsed as JSON but failed semantic validation — wrong
    /// field types, missing required keys, etc.
    #[error("invalid manifest: {0}")]
    InvalidManifest(String),

    /// The directory handed to `read_project_tree` has no `fanta.json` carrying
    /// the `fanta-project` format tag — it is not a Fantaisa project tree.
    #[error("not a fanta project directory: {}", path.display())]
    NotAProject { path: std::path::PathBuf },

    /// The project tree's `fanta.json` declares a *layout* version newer than
    /// this build understands (distinct from the doc `schema_version`, which is
    /// gated by [`FormatError::UnsupportedSchema`]).
    #[error("unsupported project format version: found {found}, this build supports {supported}")]
    UnsupportedProjectVersion { found: u32, supported: u32 },

    /// A project tree was structurally malformed — an id-named file or
    /// directory that doesn't parse, a `page.json` without an order, a doc that
    /// fails to reassemble from its parts.
    #[error("invalid project tree: {0}")]
    InvalidProjectTree(String),
}

/// Result alias used throughout the public API.
pub type Result<T> = std::result::Result<T, FormatError>;
