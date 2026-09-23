//! Document format routing. A new format supplies one `FormatHandler` module
//! and registers it at the application boundary; the loader and save action
//! use the same capability and path matching rules for every format.

use crate::{FantaFile, read_project_tree, write_project_tree};
use fanta_doc::{AssetId, Doc};
use std::collections::BTreeMap;
use std::error::Error;
use std::path::{Path, PathBuf};

pub type FormatHandlerResult<T> = std::result::Result<T, Box<dyn Error + Send + Sync>>;
pub type FormatRegistryResult<T> = std::result::Result<T, FormatRegistryError>;

/// A decoded design and its original encoded binary assets.
pub struct ImportedDesign {
    pub doc: Doc,
    pub assets: BTreeMap<AssetId, Vec<u8>>,
}

/// The affordances a format supports directly. `edit` and `version` refer to
/// the original path, before any import into a native project directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FormatCapabilities {
    pub import: bool,
    pub export: bool,
    pub display: bool,
    pub edit: bool,
    pub version: bool,
}

/// A path a format handler recognizes. Extensions omit the leading period.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormatPath {
    Extension(&'static str),
    DirectoryMarker(&'static str),
}

impl FormatPath {
    fn matches(self, path: &Path) -> bool {
        match self {
            Self::Extension(extension) => path
                .extension()
                .and_then(|value| value.to_str())
                .is_some_and(|value| value.eq_ignore_ascii_case(extension)),
            Self::DirectoryMarker(marker) => {
                (path.is_dir() && path.join(marker).is_file())
                    || (path.is_file()
                        && path.file_name().and_then(|name| name.to_str()) == Some(marker))
            }
        }
    }

    fn conflicts_with(self, other: Self) -> bool {
        match (self, other) {
            (Self::Extension(left), Self::Extension(right)) => left.eq_ignore_ascii_case(right),
            (Self::DirectoryMarker(left), Self::DirectoryMarker(right)) => left == right,
            _ => false,
        }
    }
}

/// Static metadata for a registered handler. `id` is stable across releases
/// so user preferences can name a format independently of its display name.
#[derive(Debug, Clone, Copy)]
pub struct FormatDescriptor {
    pub id: &'static str,
    pub name: &'static str,
    pub paths: &'static [FormatPath],
    pub capabilities: FormatCapabilities,
}

pub trait FormatHandler: Send + Sync {
    fn descriptor(&self) -> FormatDescriptor;

    fn import(&self, _path: &Path) -> FormatHandlerResult<ImportedDesign> {
        Err(Box::new(FormatRegistryError::UnsupportedOperation))
    }

    fn export(
        &self,
        _path: &Path,
        _doc: &Doc,
        _assets: &BTreeMap<AssetId, Vec<u8>>,
    ) -> FormatHandlerResult<()> {
        Err(Box::new(FormatRegistryError::UnsupportedOperation))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormatOperation {
    Import,
    Export,
}

#[derive(Debug, thiserror::Error)]
pub enum FormatRegistryError {
    #[error("no registered format can {operation:?} {}", path.display())]
    NoHandler {
        operation: FormatOperation,
        path: PathBuf,
    },
    #[error("unknown format id {0:?}")]
    UnknownFormat(String),
    #[error("format id {0:?} is already registered")]
    DuplicateId(&'static str),
    #[error("formats {first:?} and {second:?} claim the same path and operation")]
    ConflictingHandlers {
        first: &'static str,
        second: &'static str,
    },
    #[error("handler does not support this operation")]
    UnsupportedOperation,
    #[error("{format} failed: {source}")]
    Handler {
        format: &'static str,
        #[source]
        source: Box<dyn Error + Send + Sync>,
    },
}

#[derive(Default)]
pub struct FormatRegistry {
    handlers: Vec<Box<dyn FormatHandler>>,
}

impl FormatRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Native project folders and portable `.fant` snapshots. Applications
    /// register additional handlers such as `.fig` alongside these.
    pub fn with_native_formats() -> Self {
        Self {
            handlers: vec![Box::new(ProjectFormat), Box::new(FantSnapshotFormat)],
        }
    }

    pub fn register(&mut self, handler: impl FormatHandler + 'static) -> FormatRegistryResult<()> {
        let descriptor = handler.descriptor();
        for existing in &self.handlers {
            let registered = existing.descriptor();
            if registered.id == descriptor.id {
                return Err(FormatRegistryError::DuplicateId(descriptor.id));
            }
            let overlapping_operation = (registered.capabilities.import
                && descriptor.capabilities.import)
                || (registered.capabilities.export && descriptor.capabilities.export);
            if overlapping_operation
                && registered.paths.iter().copied().any(|first| {
                    descriptor
                        .paths
                        .iter()
                        .copied()
                        .any(|second| first.conflicts_with(second))
                })
            {
                return Err(FormatRegistryError::ConflictingHandlers {
                    first: registered.id,
                    second: descriptor.id,
                });
            }
        }
        self.handlers.push(Box::new(handler));
        Ok(())
    }

    pub fn formats(&self) -> impl Iterator<Item = FormatDescriptor> + '_ {
        self.handlers.iter().map(|handler| handler.descriptor())
    }

    pub fn can_import_path(&self, path: &Path) -> bool {
        self.handler_for(path, FormatOperation::Import).is_ok()
    }

    pub fn import_path(&self, path: &Path) -> FormatRegistryResult<ImportedDesign> {
        let handler = self.handler_for(path, FormatOperation::Import)?;
        Self::import_with(handler, path)
    }

    /// Route by stable format id when a file picker or command supplied the
    /// format, including an output path that does not exist yet.
    pub fn import_as(&self, format_id: &str, path: &Path) -> FormatRegistryResult<ImportedDesign> {
        let handler = self.handler_by_id(format_id, FormatOperation::Import)?;
        Self::import_with(handler, path)
    }

    pub fn export_path(
        &self,
        path: &Path,
        doc: &Doc,
        assets: &BTreeMap<AssetId, Vec<u8>>,
    ) -> FormatRegistryResult<()> {
        let handler = self.handler_for(path, FormatOperation::Export)?;
        Self::export_with(handler, path, doc, assets)
    }

    pub fn export_as(
        &self,
        format_id: &str,
        path: &Path,
        doc: &Doc,
        assets: &BTreeMap<AssetId, Vec<u8>>,
    ) -> FormatRegistryResult<()> {
        let handler = self.handler_by_id(format_id, FormatOperation::Export)?;
        Self::export_with(handler, path, doc, assets)
    }

    fn import_with(
        handler: &dyn FormatHandler,
        path: &Path,
    ) -> FormatRegistryResult<ImportedDesign> {
        handler
            .import(path)
            .map_err(|source| FormatRegistryError::Handler {
                format: handler.descriptor().id,
                source,
            })
    }

    fn export_with(
        handler: &dyn FormatHandler,
        path: &Path,
        doc: &Doc,
        assets: &BTreeMap<AssetId, Vec<u8>>,
    ) -> FormatRegistryResult<()> {
        handler
            .export(path, doc, assets)
            .map_err(|source| FormatRegistryError::Handler {
                format: handler.descriptor().id,
                source,
            })
    }

    fn handler_by_id(
        &self,
        format_id: &str,
        operation: FormatOperation,
    ) -> FormatRegistryResult<&dyn FormatHandler> {
        let handler = self
            .handlers
            .iter()
            .find(|handler| handler.descriptor().id == format_id)
            .ok_or_else(|| FormatRegistryError::UnknownFormat(format_id.to_owned()))?;
        let capabilities = handler.descriptor().capabilities;
        let supported = match operation {
            FormatOperation::Import => capabilities.import,
            FormatOperation::Export => capabilities.export,
        };
        if !supported {
            return Err(FormatRegistryError::UnsupportedOperation);
        }
        Ok(handler.as_ref())
    }

    fn handler_for(
        &self,
        path: &Path,
        operation: FormatOperation,
    ) -> FormatRegistryResult<&dyn FormatHandler> {
        self.handlers
            .iter()
            .find(|handler| {
                let descriptor = handler.descriptor();
                let supported = match operation {
                    FormatOperation::Import => descriptor.capabilities.import,
                    FormatOperation::Export => descriptor.capabilities.export,
                };
                supported && descriptor.paths.iter().any(|pattern| pattern.matches(path))
            })
            .map(|handler| handler.as_ref())
            .ok_or_else(|| FormatRegistryError::NoHandler {
                operation,
                path: path.to_path_buf(),
            })
    }
}

struct ProjectFormat;

fn project_root(path: &Path) -> &Path {
    if path.file_name().and_then(|name| name.to_str()) == Some("fanta.json") {
        path.parent().unwrap_or(path)
    } else {
        path
    }
}

impl FormatHandler for ProjectFormat {
    fn descriptor(&self) -> FormatDescriptor {
        FormatDescriptor {
            id: "fanta-project",
            name: "Fanta project",
            paths: &[FormatPath::DirectoryMarker("fanta.json")],
            capabilities: FormatCapabilities {
                import: true,
                export: true,
                display: true,
                edit: true,
                version: true,
            },
        }
    }

    fn import(&self, path: &Path) -> FormatHandlerResult<ImportedDesign> {
        let (doc, assets) = read_project_tree(project_root(path))?;
        Ok(ImportedDesign { doc, assets })
    }

    fn export(
        &self,
        path: &Path,
        doc: &Doc,
        assets: &BTreeMap<AssetId, Vec<u8>>,
    ) -> FormatHandlerResult<()> {
        write_project_tree(project_root(path), doc, assets)?;
        Ok(())
    }
}

struct FantSnapshotFormat;

impl FormatHandler for FantSnapshotFormat {
    fn descriptor(&self) -> FormatDescriptor {
        FormatDescriptor {
            id: "fant-snapshot",
            name: "Fanta snapshot",
            paths: &[FormatPath::Extension("fant")],
            capabilities: FormatCapabilities {
                import: true,
                export: true,
                display: true,
                edit: false,
                version: false,
            },
        }
    }

    fn import(&self, path: &Path) -> FormatHandlerResult<ImportedDesign> {
        let file = FantaFile::open(path)?;
        let doc = file.load_doc()?;
        let mut assets = BTreeMap::new();
        for id in file.list_assets() {
            assets.insert(id, file.get_asset(id)?);
        }
        Ok(ImportedDesign { doc, assets })
    }

    fn export(
        &self,
        path: &Path,
        doc: &Doc,
        assets: &BTreeMap<AssetId, Vec<u8>>,
    ) -> FormatHandlerResult<()> {
        let mut file = FantaFile::create(path)?;
        file.save_doc(doc)?;
        for (id, bytes) in assets {
            file.put_asset_as(*id, bytes)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    struct ExampleFormat;

    impl FormatHandler for ExampleFormat {
        fn descriptor(&self) -> FormatDescriptor {
            FormatDescriptor {
                id: "example",
                name: "Example",
                paths: &[FormatPath::Extension("example")],
                capabilities: FormatCapabilities {
                    import: true,
                    export: false,
                    display: true,
                    edit: false,
                    version: false,
                },
            }
        }

        fn import(&self, _path: &Path) -> FormatHandlerResult<ImportedDesign> {
            Ok(ImportedDesign {
                doc: Doc::new(),
                assets: BTreeMap::new(),
            })
        }
    }

    #[test]
    fn external_handler_routes_case_insensitive_extension() {
        let mut registry = FormatRegistry::with_native_formats();
        registry.register(ExampleFormat).unwrap();
        let imported = registry.import_path(Path::new("design.EXAMPLE")).unwrap();
        assert!(imported.assets.is_empty());
        assert!(matches!(
            registry.export_path(Path::new("design.example"), &imported.doc, &imported.assets),
            Err(FormatRegistryError::NoHandler { .. })
        ));
    }

    #[test]
    fn registration_rejects_ambiguous_importers() {
        let mut registry = FormatRegistry::new();
        registry.register(ExampleFormat).unwrap();
        struct DuplicateExtension;
        impl FormatHandler for DuplicateExtension {
            fn descriptor(&self) -> FormatDescriptor {
                FormatDescriptor {
                    id: "another",
                    name: "Another",
                    paths: &[FormatPath::Extension("EXAMPLE")],
                    capabilities: FormatCapabilities {
                        import: true,
                        export: false,
                        display: true,
                        edit: false,
                        version: false,
                    },
                }
            }
        }
        assert!(matches!(
            registry.register(DuplicateExtension),
            Err(FormatRegistryError::ConflictingHandlers { .. })
        ));
    }

    #[test]
    fn native_project_handler_reads_directory() {
        let directory = tempdir().unwrap();
        let doc = Doc::new();
        let assets = BTreeMap::new();
        write_project_tree(directory.path(), &doc, &assets).unwrap();
        let registry = FormatRegistry::with_native_formats();
        let imported = registry.import_path(directory.path()).unwrap();
        assert_eq!(imported.doc.id, doc.id);
        assert!(imported.assets.is_empty());
    }

    #[test]
    fn explicit_format_exports_new_project_directory() {
        let directory = tempdir().unwrap();
        let project_path = directory.path().join("new-project");
        let doc = Doc::new();
        let registry = FormatRegistry::with_native_formats();
        registry
            .export_as("fanta-project", &project_path, &doc, &BTreeMap::new())
            .unwrap();
        let imported = registry.import_path(&project_path).unwrap();
        assert_eq!(imported.doc.id, doc.id);
    }
}
