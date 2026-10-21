//! Static website export of a Fanta document. Every design page gets its own
//! read-only URL, rendered in the browser by the `@fanta/runtime` WASM engine
//! from one deterministic runtime bundle, so any static host can serve it.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use fanta_doc::{AnimationClipId, AssetId, Doc, NodeId};
use fanta_format::runtime::{RuntimeFont, RuntimeProgram, SourceBundle};
use fanta_runtime::Runtime;
use fanta_text::{FontRegistration, FontResolver, GenericFamily};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use unicode_normalization::UnicodeNormalization;

pub const SITE_FORMAT: &str = "fanta-site";
pub const SITE_VERSION: u32 = 1;

const ASSET_DIRECTORY: &str = "fanta";
const RUNTIME_MODULE: &str = "fanta-runtime.mjs";
const RUNTIME_WASM: &str = "fanta_runtime_wasm.wasm";
const FONT_EXTENSIONS: [&str; 4] = ["ttf", "otf", "ttc", "otc"];
const PROJECT_FILES: [&str; 2] = ["fanta.json", "workspace.fnx"];
const PROJECT_DIRECTORIES: [&str; 9] = [
    "pages",
    "components",
    "doc",
    "assets",
    "styles",
    "graphics",
    "prototypes",
    "motion",
    "audio",
];
/// Creating a player validates a page's fonts and resources; layout does not
/// depend on the surface size, so any positive size proves the page renders.
const VALIDATION_SIZE: f64 = 1024.0;

#[derive(Debug, thiserror::Error)]
pub enum SiteError {
    #[error("{}: {source}", path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("could not build the runtime bundle: {0}")]
    Bundle(#[from] fanta_format::runtime::RuntimeError),
    #[error("the runtime rejected the bundle: {0}")]
    Runtime(#[from] fanta_runtime::RuntimeError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error("the document has no pages to export")]
    NoPages,
    #[error("page {page:?} cannot be displayed: {message}")]
    Page { page: String, message: String },
    #[error(
        "no embeddable face renders {family:?} at weight {weight} (italic: {italic}); add a font directory that contains it"
    )]
    MissingFace {
        family: String,
        weight: u16,
        italic: bool,
    },
    #[error(
        "{} is not empty and is not a previous Fanta site export",
        .0.display()
    )]
    OccupiedOutput(PathBuf),
}

/// How fonts are chosen for an export and what the site is called.
#[derive(Debug, Clone, Default)]
pub struct SiteOptions {
    /// Site title; the document title when absent.
    pub title: Option<String>,
    /// Directories of font files whose licenses allow publishing them.
    pub font_directories: Vec<PathBuf>,
    /// Families rendered with another family's faces, keyed by requested name.
    pub substitutions: BTreeMap<String, String>,
    /// Whether open-licensed families may be downloaded when no local file
    /// has them.
    pub download_fonts: bool,
}

/// The compiled `@fanta/runtime` module and WebAssembly binary the site loads.
pub struct RuntimeAssets {
    pub module: Vec<u8>,
    pub wasm: Vec<u8>,
    pub api: Vec<u8>,
    pub viewer_script: Vec<u8>,
    pub viewer_style: Vec<u8>,
}

impl RuntimeAssets {
    /// Read a built `@fanta/runtime` package from Fanta-JS.
    pub fn read(directory: &Path) -> Result<Self, SiteError> {
        Ok(Self {
            module: read(&directory.join("dist").join(RUNTIME_MODULE))?,
            wasm: read(&directory.join("dist").join(RUNTIME_WASM))?,
            api: read(&directory.join("src/index.js"))?,
            viewer_script: read(&directory.join("site/viewer.js"))?,
            viewer_style: read(&directory.join("site/viewer.css"))?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum FontSource {
    File {
        path: PathBuf,
    },
    Bundled,
    Downloaded,
    /// Another family's face, registered under the requested name: chosen
    /// explicitly, or because no publishable face of the family exists.
    Substituted {
        family: String,
    },
}

impl std::fmt::Display for FontSource {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::File { path } => write!(formatter, "file {}", path.display()),
            Self::Bundled => formatter.write_str("bundled"),
            Self::Downloaded => formatter.write_str("downloaded"),
            Self::Substituted { family } => write!(formatter, "substituted with {family}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EmbeddedFace {
    pub family: String,
    pub weight: u16,
    pub italic: bool,
    pub source: FontSource,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SitePage {
    pub id: NodeId,
    pub name: String,
    pub slug: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SiteReport {
    pub title: String,
    pub pages: Vec<SitePage>,
    pub fonts: Vec<EmbeddedFace>,
    pub bundle_bytes: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Manifest {
    format: &'static str,
    version: u32,
    title: String,
    pages: Vec<SitePage>,
    flows: Vec<ManifestFlow>,
    clips: Vec<ManifestClip>,
    fonts: Vec<EmbeddedFace>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ManifestFlow {
    name: String,
    start: NodeId,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ManifestClip {
    id: AnimationClipId,
    name: String,
    duration_ms: u32,
}

#[derive(Debug, Deserialize)]
struct PreviousManifest {
    format: String,
    #[serde(default)]
    pages: Vec<PreviousPage>,
}

#[derive(Debug, Deserialize)]
struct PreviousPage {
    slug: String,
}

/// Export the project directory at `project` into `output`, compiling its
/// sources with the same portable compiler the browser runtime uses.
pub fn export_project(
    project: &Path,
    output: &Path,
    assets: &RuntimeAssets,
    options: &SiteOptions,
) -> Result<SiteReport, SiteError> {
    let compiled = project_source(project)?.compile()?;
    export_document(
        compiled.document().clone(),
        compiled.resources().clone(),
        output,
        assets,
        options,
    )
}

/// The project files the portable compiler reads. Symlinks are skipped so a
/// project cannot pull files from outside its directory into a public site.
fn project_source(project: &Path) -> Result<SourceBundle, SiteError> {
    fn collect(
        project: &Path,
        directory: &Path,
        files: &mut BTreeMap<String, Vec<u8>>,
    ) -> Result<(), SiteError> {
        let entries = std::fs::read_dir(directory).map_err(|source| SiteError::Io {
            path: directory.to_owned(),
            source,
        })?;
        for entry in entries {
            let entry = entry.map_err(|source| SiteError::Io {
                path: directory.to_owned(),
                source,
            })?;
            let path = entry.path();
            let file_type = entry.file_type().map_err(|source| SiteError::Io {
                path: path.clone(),
                source,
            })?;
            if file_type.is_dir() {
                collect(project, &path, files)?;
            } else if file_type.is_file() {
                let relative = path
                    .strip_prefix(project)
                    .map(|relative| {
                        relative
                            .components()
                            .map(|component| component.as_os_str().to_string_lossy())
                            .collect::<Vec<_>>()
                            .join("/")
                    })
                    .map_err(|_| SiteError::Io {
                        path: path.clone(),
                        source: std::io::Error::other("file is outside the project"),
                    })?;
                files.insert(relative, read(&path)?);
            }
        }
        Ok(())
    }
    let mut files = BTreeMap::new();
    for name in PROJECT_FILES {
        let path = project.join(name);
        if std::fs::symlink_metadata(&path).is_ok_and(|metadata| metadata.is_file()) {
            files.insert(name.to_owned(), read(&path)?);
        }
    }
    for name in PROJECT_DIRECTORIES {
        let path = project.join(name);
        if std::fs::symlink_metadata(&path).is_ok_and(|metadata| metadata.is_dir()) {
            collect(project, &path, &mut files)?;
        }
    }
    Ok(SourceBundle {
        files,
        fonts: Vec::new(),
        font_substitutions: BTreeMap::new(),
    })
}

/// Export an already loaded document, such as the one open in the editor.
pub fn export_document(
    document: Doc,
    resources: BTreeMap<AssetId, Vec<u8>>,
    output: &Path,
    assets: &RuntimeAssets,
    options: &SiteOptions,
) -> Result<SiteReport, SiteError> {
    let compiled = compile_site(document, resources, options)?;
    let document = compiled.program.document();
    let manifest = Manifest {
        format: SITE_FORMAT,
        version: SITE_VERSION,
        title: compiled.title.clone(),
        pages: compiled.pages.clone(),
        flows: document
            .flows
            .iter()
            .map(|flow| ManifestFlow {
                name: flow.name.clone(),
                start: flow.start,
            })
            .collect(),
        clips: document
            .motion
            .clips
            .values()
            .map(|clip| ManifestClip {
                id: clip.id,
                name: clip.name.clone(),
                duration_ms: clip.duration_ms,
            })
            .collect(),
        fonts: compiled.fonts.clone(),
    };
    write_site(output, assets, &compiled.bundle, &manifest)?;
    Ok(compiled.report())
}

/// Compile the project directory at `project` into one runtime bundle written
/// to `output`. Fonts are embedded exactly as for a website, so the file is
/// self-contained: `fanta-runtime` loads it with `loadBundle` and has no
/// compiler of its own.
pub fn export_bundle(
    project: &Path,
    output: &Path,
    options: &SiteOptions,
) -> Result<SiteReport, SiteError> {
    let compiled = project_source(project)?.compile()?;
    let compiled = compile_site(
        compiled.document().clone(),
        compiled.resources().clone(),
        options,
    )?;
    if let Some(parent) = output
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        create_directory(parent)?;
    }
    write(output, &compiled.bundle)?;
    Ok(compiled.report())
}

/// Split a `--substitute FAMILY=REPLACEMENT` argument into its trimmed parts.
pub fn parse_substitution(argument: &str) -> Option<(String, String)> {
    let (family, replacement) = argument.split_once('=')?;
    Some((family.trim().to_owned(), replacement.trim().to_owned()))
}

struct CompiledSite {
    title: String,
    pages: Vec<SitePage>,
    fonts: Vec<EmbeddedFace>,
    program: RuntimeProgram,
    bundle: Vec<u8>,
}

impl CompiledSite {
    fn report(&self) -> SiteReport {
        SiteReport {
            title: self.title.clone(),
            pages: self.pages.clone(),
            fonts: self.fonts.clone(),
            bundle_bytes: self.bundle.len(),
        }
    }
}

/// Embed the fonts, build the deterministic bundle and prove every page loads
/// in the native runtime before anything is written.
fn compile_site(
    document: Doc,
    resources: BTreeMap<AssetId, Vec<u8>>,
    options: &SiteOptions,
) -> Result<CompiledSite, SiteError> {
    let title = [
        options.title.as_deref(),
        Some(document.metadata.title.as_str()),
    ]
    .into_iter()
    .flatten()
    .map(str::trim)
    .find(|title| !title.is_empty())
    .unwrap_or("Untitled")
    .to_owned();
    let pages = site_pages(&document)?;
    let (registrations, fonts) = FontLibrary::new(options)?.embed(&requested_faces(&document)?)?;
    let program =
        RuntimeProgram::from_document(document, resources, registrations, BTreeMap::new())?;
    let bundle = program.to_bundle()?;
    validate_pages(&bundle, &pages)?;
    Ok(CompiledSite {
        title,
        pages,
        fonts,
        program,
        bundle,
    })
}

fn site_pages(document: &Doc) -> Result<Vec<SitePage>, SiteError> {
    let mut taken = BTreeSet::from([ASSET_DIRECTORY.to_owned()]);
    let mut pages = Vec::new();
    for (index, id) in document.pages().iter().enumerate() {
        let name = document
            .scene
            .get(*id)
            .map(|node| node.name.trim().to_owned())
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| format!("Page {}", index + 1));
        let mut base = slug(&name);
        if base.is_empty() {
            base = format!("page-{}", index + 1);
        }
        let mut candidate = base.clone();
        let mut suffix = 2;
        while !taken.insert(candidate.clone()) {
            candidate = format!("{base}-{suffix}");
            suffix += 1;
        }
        pages.push(SitePage {
            id: *id,
            name,
            slug: candidate,
        });
    }
    if pages.is_empty() {
        return Err(SiteError::NoPages);
    }
    Ok(pages)
}

/// A lowercase ASCII URL segment: accents fold to their base letters and any
/// other run of characters becomes one hyphen.
fn slug(name: &str) -> String {
    let mut slug = String::new();
    for character in name.nfkd() {
        if character.is_ascii_alphanumeric() {
            slug.push(character.to_ascii_lowercase());
        } else if unicode_normalization::char::is_combining_mark(character) {
            continue;
        } else if !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
    }
    slug.trim_end_matches('-').to_owned()
}

fn is_slug(candidate: &str) -> bool {
    !candidate.is_empty()
        && candidate != ASSET_DIRECTORY
        && candidate.chars().all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
        })
}

#[derive(Debug, Clone)]
struct Face {
    family: String,
    weight: u16,
    italic: bool,
}

/// Every distinct text face the document can render. Node styles and runs,
/// variable values, instance overrides and motion keyframes all serialize
/// text styles with a `font_family`, so one walk finds them all.
fn requested_faces(document: &Doc) -> Result<Vec<Face>, SiteError> {
    fn collect(value: &Value, faces: &mut BTreeMap<(String, u16, bool), Face>) {
        match value {
            Value::Object(object) => {
                if let Some(family) = object
                    .get("font_family")
                    .and_then(Value::as_str)
                    .filter(|family| !family.trim().is_empty())
                {
                    let weight = object
                        .get("weight")
                        .and_then(Value::as_u64)
                        .and_then(|weight| u16::try_from(weight).ok())
                        .unwrap_or(400);
                    let italic = object
                        .get("italic")
                        .and_then(Value::as_bool)
                        .unwrap_or(false);
                    // The runtime matches families ignoring ASCII case and
                    // rejects two registrations of one face.
                    faces
                        .entry((family.to_ascii_lowercase(), weight, italic))
                        .or_insert_with(|| Face {
                            family: family.to_owned(),
                            weight,
                            italic,
                        });
                }
                for child in object.values() {
                    collect(child, faces);
                }
            }
            Value::Array(values) => {
                for child in values {
                    collect(child, faces);
                }
            }
            _ => {}
        }
    }
    let mut faces = BTreeMap::new();
    collect(&serde_json::to_value(document)?, &mut faces);
    Ok(faces.into_values().collect())
}

struct FontFile {
    path: PathBuf,
    bytes: Rc<Vec<u8>>,
    face_index: u32,
}

struct FontLibrary<'a> {
    options: &'a SiteOptions,
    files: BTreeMap<String, Vec<FontFile>>,
    downloads: BTreeMap<String, Option<Vec<u8>>>,
}

impl<'a> FontLibrary<'a> {
    fn new(options: &'a SiteOptions) -> Result<Self, SiteError> {
        let mut files = BTreeMap::new();
        for directory in &options.font_directories {
            scan_fonts(directory, &mut files)?;
        }
        Ok(Self {
            options,
            files,
            downloads: BTreeMap::new(),
        })
    }

    fn embed(mut self, faces: &[Face]) -> Result<(Vec<RuntimeFont>, Vec<EmbeddedFace>), SiteError> {
        let mut registrations = Vec::new();
        let mut embedded = Vec::new();
        for face in faces {
            let (bytes, face_index, source) = self.resolve(face)?;
            registrations.push(RuntimeFont {
                family: face.family.clone(),
                weight: face.weight,
                italic: face.italic,
                face_index,
                bytes,
            });
            embedded.push(EmbeddedFace {
                family: face.family.clone(),
                weight: face.weight,
                italic: face.italic,
                source,
            });
        }
        Ok((registrations, embedded))
    }

    fn resolve(&mut self, face: &Face) -> Result<(Vec<u8>, u32, FontSource), SiteError> {
        let explicit = self
            .options
            .substitutions
            .iter()
            .find(|(requested, _)| requested.eq_ignore_ascii_case(&face.family))
            .map(|(_, replacement)| replacement.clone());
        if let Some(replacement) = explicit {
            return self
                .find(&replacement, face)
                .map(|(bytes, face_index, _)| {
                    (
                        bytes,
                        face_index,
                        FontSource::Substituted {
                            family: replacement.clone(),
                        },
                    )
                })
                .ok_or_else(|| missing(face));
        }
        if let Some(found) = self.find(&face.family, face) {
            return Ok(found);
        }
        // Mirrors the editor's class substitute for unavailable families; system
        // fonts usually cannot be published, so they land here too.
        let fallback = GenericFamily::classify(&face.family).source_substitute();
        bundled_face(fallback, face)
            .map(|(bytes, face_index, _)| {
                (
                    bytes,
                    face_index,
                    FontSource::Substituted {
                        family: fallback.to_owned(),
                    },
                )
            })
            .ok_or_else(|| missing(face))
    }

    fn find(&mut self, family: &str, face: &Face) -> Option<(Vec<u8>, u32, FontSource)> {
        let key = family.to_ascii_lowercase();
        if let Some(file) = self.files.get(&key).and_then(|files| {
            files
                .iter()
                .find(|file| registers(face, &file.bytes, file.face_index))
        }) {
            return Some((
                file.bytes.as_ref().clone(),
                file.face_index,
                FontSource::File {
                    path: file.path.clone(),
                },
            ));
        }
        if let Some(found) = bundled_face(family, face) {
            return Some(found);
        }
        if !self.options.download_fonts {
            return None;
        }
        let downloaded = self
            .downloads
            .entry(key)
            .or_insert_with(|| fanta_text::fetch_downloadable_font(family));
        downloaded
            .as_ref()
            .filter(|bytes| registers(face, bytes, 0))
            .map(|bytes| (bytes.clone(), 0, FontSource::Downloaded))
    }
}

fn bundled_face(family: &str, face: &Face) -> Option<(Vec<u8>, u32, FontSource)> {
    fanta_text::bundled_font_files(family, face.italic)
        .iter()
        .find(|bytes| registers(face, bytes, 0))
        .map(|bytes| (bytes.to_vec(), 0, FontSource::Bundled))
}

/// Whether the runtime would accept `bytes` as this face: its registration
/// checks the slant and instances variable fonts at the requested weight.
fn registers(face: &Face, bytes: &[u8], face_index: u32) -> bool {
    FontResolver::from_fonts(
        [FontRegistration {
            family: &face.family,
            weight: face.weight,
            italic: face.italic,
            face_index,
            bytes,
        }],
        &BTreeMap::new(),
    )
    .is_ok()
}

fn missing(face: &Face) -> SiteError {
    SiteError::MissingFace {
        family: face.family.clone(),
        weight: face.weight,
        italic: face.italic,
    }
}

fn scan_fonts(
    directory: &Path,
    files: &mut BTreeMap<String, Vec<FontFile>>,
) -> Result<(), SiteError> {
    let entries = std::fs::read_dir(directory).map_err(|source| SiteError::Io {
        path: directory.to_owned(),
        source,
    })?;
    let mut paths = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|source| SiteError::Io {
            path: directory.to_owned(),
            source,
        })?;
        // The entry's own type: a symlinked directory is not followed, so a
        // link cycle cannot recurse forever.
        let file_type = entry.file_type().map_err(|source| SiteError::Io {
            path: entry.path(),
            source,
        })?;
        paths.push((entry.path(), file_type.is_dir()));
    }
    // Directory order is filesystem-dependent; sorting keeps exports deterministic.
    paths.sort();
    for (path, is_directory) in paths {
        if is_directory {
            scan_fonts(&path, files)?;
            continue;
        }
        let is_font = path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| {
                FONT_EXTENSIONS
                    .iter()
                    .any(|known| extension.eq_ignore_ascii_case(known))
            });
        if !is_font {
            continue;
        }
        let bytes = Rc::new(read(&path)?);
        for (face_index, family) in fanta_text::font_family_names(&bytes) {
            files
                .entry(family.to_ascii_lowercase())
                .or_default()
                .push(FontFile {
                    path: path.clone(),
                    bytes: bytes.clone(),
                    face_index,
                });
        }
    }
    Ok(())
}

fn validate_pages(bundle: &[u8], pages: &[SitePage]) -> Result<(), SiteError> {
    let mut runtime = Runtime::default();
    let program = runtime.load_bundle(bundle)?;
    for page in pages {
        let player = runtime
            .create_player(program, VALIDATION_SIZE, VALIDATION_SIZE, Some(page.id))
            .map_err(|error| SiteError::Page {
                page: page.name.clone(),
                message: error.0,
            })?;
        runtime.dispose_player(player);
    }
    Ok(())
}

fn write_site(
    output: &Path,
    assets: &RuntimeAssets,
    bundle: &[u8],
    manifest: &Manifest,
) -> Result<(), SiteError> {
    let previous = previous_export(output)?;
    let directory = output.join(ASSET_DIRECTORY);
    create_directory(&directory)?;
    write(&directory.join("index.js"), &assets.api)?;
    write(&directory.join("viewer.js"), &assets.viewer_script)?;
    write(&directory.join("viewer.css"), &assets.viewer_style)?;
    write(&directory.join(RUNTIME_MODULE), &assets.module)?;
    write(&directory.join(RUNTIME_WASM), &assets.wasm)?;
    write(&directory.join("program.fanta"), bundle)?;
    write(
        &directory.join("manifest.json"),
        &serde_json::to_vec_pretty(manifest)?,
    )?;
    write(&output.join("index.html"), index_html(manifest).as_bytes())?;
    for page in &manifest.pages {
        let page_directory = output.join(&page.slug);
        create_directory(&page_directory)?;
        write(
            &page_directory.join("index.html"),
            page_html(manifest, page)?.as_bytes(),
        )?;
    }
    for slug in previous {
        if manifest.pages.iter().any(|page| page.slug == slug) {
            continue;
        }
        let page_directory = output.join(&slug);
        let page_file = page_directory.join("index.html");
        removed(std::fs::remove_file(&page_file), &page_file)?;
        // Hosting files a person added beside the page keep its directory.
        removed(std::fs::remove_dir(&page_directory), &page_directory)?;
    }
    Ok(())
}

/// Page slugs of an earlier export in `output`, so a re-export can remove
/// pages that no longer exist. Any other non-empty directory is refused rather
/// than mixed with site files.
fn previous_export(output: &Path) -> Result<Vec<String>, SiteError> {
    let mut entries = match std::fs::read_dir(output) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => {
            return Err(SiteError::Io {
                path: output.to_owned(),
                source,
            });
        }
    };
    if entries.next().is_none() {
        return Ok(Vec::new());
    }
    let manifest_path = output.join(ASSET_DIRECTORY).join("manifest.json");
    let manifest = match std::fs::read(&manifest_path) {
        Ok(bytes) => serde_json::from_slice::<PreviousManifest>(&bytes)
            .ok()
            .filter(|manifest| manifest.format == SITE_FORMAT),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(source) => {
            return Err(SiteError::Io {
                path: manifest_path,
                source,
            });
        }
    };
    let manifest = manifest.ok_or_else(|| SiteError::OccupiedOutput(output.to_owned()))?;
    // A manifest is user-editable; only slugs this exporter could have written
    // may name a directory to clean up.
    Ok(manifest
        .pages
        .into_iter()
        .map(|page| page.slug)
        .filter(|slug| is_slug(slug))
        .collect())
}

fn removed(result: std::io::Result<()>, path: &Path) -> Result<(), SiteError> {
    match result {
        Ok(()) => Ok(()),
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::DirectoryNotEmpty
            ) =>
        {
            Ok(())
        }
        Err(source) => Err(SiteError::Io {
            path: path.to_owned(),
            source,
        }),
    }
}

fn read(path: &Path) -> Result<Vec<u8>, SiteError> {
    std::fs::read(path).map_err(|source| SiteError::Io {
        path: path.to_owned(),
        source,
    })
}

fn write(path: &Path, bytes: &[u8]) -> Result<(), SiteError> {
    std::fs::write(path, bytes).map_err(|source| SiteError::Io {
        path: path.to_owned(),
        source,
    })
}

fn create_directory(path: &Path) -> Result<(), SiteError> {
    std::fs::create_dir_all(path).map_err(|source| SiteError::Io {
        path: path.to_owned(),
        source,
    })
}

fn escape_html(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#39;"),
            _ => escaped.push(character),
        }
    }
    escaped
}

fn index_html(manifest: &Manifest) -> String {
    let title = escape_html(&manifest.title);
    let items: String = manifest
        .pages
        .iter()
        .map(|page| {
            format!(
                "<li><a href=\"{}/\">{}</a></li>\n",
                page.slug,
                escape_html(&page.name)
            )
        })
        .collect();
    format!(
        r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>{title}</title>
<link rel="stylesheet" href="fanta/viewer.css">
</head>
<body class="index">
<main class="contents">
<h1>{title}</h1>
<ol class="page-list">
{items}</ol>
</main>
</body>
</html>
"#
    )
}

fn page_html(manifest: &Manifest, page: &SitePage) -> Result<String, SiteError> {
    let title = escape_html(&manifest.title);
    let name = escape_html(&page.name);
    let options: String = manifest
        .pages
        .iter()
        .map(|option| {
            format!(
                "<option value=\"../{}/\"{}>{}</option>",
                option.slug,
                if option.slug == page.slug {
                    " selected"
                } else {
                    ""
                },
                escape_html(&option.name)
            )
        })
        .collect();
    let identity = serde_json::to_string(&page.id)?;
    Ok(format!(
        r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>{name} · {title}</title>
<link rel="stylesheet" href="../fanta/viewer.css">
</head>
<body>
<header class="bar">
<a class="title" href="../">{title}</a>
<nav class="pages" aria-label="Pages"><select data-fanta-pages aria-label="Page">{options}</select></nav>
<div class="camera" role="toolbar" aria-label="Zoom">
<button type="button" data-fanta-zoom="out" aria-label="Zoom out">−</button>
<output data-fanta-zoom-level aria-live="polite">100%</output>
<button type="button" data-fanta-zoom="in" aria-label="Zoom in">+</button>
<button type="button" data-fanta-fit>Fit</button>
</div>
</header>
<main class="stage">
<canvas data-fanta-canvas role="img" aria-label="{name}"></canvas>
<p class="status" data-fanta-status role="status">Loading…</p>
</main>
<script type="module">
import {{mountPage}} from "../fanta/viewer.js";
mountPage({{page:{identity}}});
</script>
</body>
</html>
"#
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use fanta_doc::{CanvasNode, GroupNode, NodeData, TextNode};

    fn assets() -> RuntimeAssets {
        RuntimeAssets {
            module: b"export default async () => ({});".to_vec(),
            wasm: b"\0asm".to_vec(),
            api: b"export const createRuntime = () => {};".to_vec(),
            viewer_script: b"export const mountPage = () => {};".to_vec(),
            viewer_style: b"canvas { display: block; }".to_vec(),
        }
    }

    #[test]
    fn runtime_assets_are_read_from_the_external_package_and_missing_files_are_reported()
    -> Result<(), Box<dyn std::error::Error>> {
        let package = tempfile::tempdir()?;
        let assets = assets();
        for (name, contents) in [
            (format!("dist/{RUNTIME_MODULE}"), &assets.module),
            (format!("dist/{RUNTIME_WASM}"), &assets.wasm),
            ("src/index.js".into(), &assets.api),
            ("site/viewer.js".into(), &assets.viewer_script),
            ("site/viewer.css".into(), &assets.viewer_style),
        ] {
            let path = package.path().join(name);
            std::fs::create_dir_all(path.parent().ok_or("missing parent")?)?;
            std::fs::write(path, contents)?;
        }
        let read = RuntimeAssets::read(package.path())?;
        assert_eq!(read.module, assets.module);
        assert_eq!(read.wasm, assets.wasm);
        assert_eq!(read.api, assets.api);
        assert_eq!(read.viewer_script, assets.viewer_script);
        assert_eq!(read.viewer_style, assets.viewer_style);
        let missing = package.path().join("site/viewer.js");
        std::fs::remove_file(&missing)?;
        assert!(matches!(
            RuntimeAssets::read(package.path()),
            Err(SiteError::Io { path, .. }) if path == missing
        ));
        Ok(())
    }

    fn page(document: &mut Doc, name: &str) -> Result<NodeId, Box<dyn std::error::Error>> {
        let mut node = CanvasNode::new(NodeData::Group(GroupNode::default()));
        node.name = name.to_owned();
        let id = document.scene.insert(node)?;
        document.add_page(id);
        Ok(id)
    }

    fn text(
        document: &mut Doc,
        parent: NodeId,
        content: &str,
        family: &str,
        weight: u16,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut frame = CanvasNode::new(NodeData::Group(GroupNode {
            clip_size: Some([320.0, 200.0]),
            ..GroupNode::default()
        }));
        frame.parent = Some(parent);
        let frame = document.scene.insert(frame)?;
        let mut label = TextNode::new(content, 300.0, 40.0);
        label.style.font_family = family.to_owned();
        label.style.weight = weight;
        let mut node = CanvasNode::new(NodeData::Text(label));
        node.name = content.to_owned();
        node.parent = Some(frame);
        document.scene.insert(node)?;
        Ok(())
    }

    fn project(document: &Doc) -> Result<tempfile::TempDir, Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        fanta_format::write_project_tree(directory.path(), document, &BTreeMap::new())?;
        Ok(directory)
    }

    fn options() -> SiteOptions {
        SiteOptions {
            download_fonts: false,
            ..SiteOptions::default()
        }
    }

    #[test]
    fn bundle_export_writes_one_loadable_file_with_embedded_fonts()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut document = Doc::new();
        document.metadata.title = "Bundle".into();
        let home = page(&mut document, "Home")?;
        text(&mut document, home, "Welcome", "Inter", 400)?;
        let source = project(&document)?;
        let output = tempfile::tempdir()?;
        let target = output.path().join("dist").join("program.fanta");
        let report = export_bundle(source.path(), &target, &options())?;
        assert_eq!(report.title, "Bundle");
        assert_eq!(report.pages.len(), 1);
        let bundle = std::fs::read(&target)?;
        assert_eq!(report.bundle_bytes, bundle.len());
        assert_eq!(std::fs::read_dir(output.path().join("dist"))?.count(), 1);
        let program = RuntimeProgram::load_bundle(&bundle)?;
        assert_eq!(program.document().metadata.title, "Bundle");
        assert!(program.fonts().iter().any(|font| font.family == "Inter"));
        assert_eq!(
            report
                .fonts
                .iter()
                .map(|face| face.family.as_str())
                .collect::<Vec<_>>(),
            ["Inter"]
        );
        Ok(())
    }

    #[test]
    fn pages_get_escaped_names_unique_slugs_and_a_loadable_bundle_with_their_fonts()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut document = Doc::new();
        document.metadata.title = "Launch <Kit> & \"Notes\"".into();
        let home = page(&mut document, "Home")?;
        let duplicate = page(&mut document, "Home")?;
        let accented = page(&mut document, "Página Ñandú")?;
        let reserved = page(&mut document, "Fanta")?;
        let symbols = page(&mut document, "★★★")?;
        text(&mut document, home, "Welcome", "Inter", 400)?;
        text(&mut document, home, "Bold welcome", "Inter", 700)?;
        text(
            &mut document,
            duplicate,
            "System text",
            "Helvetica Neue",
            400,
        )?;
        text(&mut document, accented, "Brand text", "Brand Grotesk", 600)?;
        let source = project(&document)?;
        let output = tempfile::tempdir()?;
        let mut options = options();
        options
            .substitutions
            .insert("brand grotesk".into(), "Source Sans 3".into());
        let report = export_project(source.path(), output.path(), &assets(), &options)?;

        let slugs: Vec<_> = report.pages.iter().map(|page| page.slug.as_str()).collect();
        assert_eq!(
            slugs,
            ["home", "home-2", "pagina-nandu", "fanta-2", "page-5"]
        );
        assert_eq!(report.pages[2].id, accented);
        assert_eq!(report.pages[3].id, reserved);
        assert_eq!(report.pages[4].id, symbols);
        let source_of = |family: &str, weight: u16| {
            report
                .fonts
                .iter()
                .find(|face| face.family == family && face.weight == weight)
                .map(|face| face.source.clone())
        };
        assert_eq!(source_of("Inter", 400), Some(FontSource::Bundled));
        assert_eq!(source_of("Inter", 700), Some(FontSource::Bundled));
        assert_eq!(
            source_of("Helvetica Neue", 400),
            Some(FontSource::Substituted {
                family: "Source Sans 3".into()
            })
        );
        assert_eq!(
            source_of("Brand Grotesk", 600),
            Some(FontSource::Substituted {
                family: "Source Sans 3".into()
            })
        );

        let site = output.path();
        let index = std::fs::read_to_string(site.join("index.html"))?;
        assert!(index.contains("Launch &lt;Kit&gt; &amp; &quot;Notes&quot;"));
        assert!(index.contains("<a href=\"pagina-nandu/\">Página Ñandú</a>"));
        let home_page = std::fs::read_to_string(site.join("home/index.html"))?;
        assert!(home_page.contains(&serde_json::to_string(&home)?));
        assert!(home_page.contains("<option value=\"../home/\" selected>Home</option>"));
        for asset in [
            "index.js",
            "viewer.js",
            "viewer.css",
            RUNTIME_MODULE,
            RUNTIME_WASM,
        ] {
            assert!(site.join(ASSET_DIRECTORY).join(asset).is_file(), "{asset}");
        }
        let bundle = std::fs::read(site.join("fanta/program.fanta"))?;
        assert_eq!(bundle.len(), report.bundle_bytes);
        let program = RuntimeProgram::load_bundle(&bundle)?;
        let mut registered: Vec<_> = program
            .fonts()
            .iter()
            .map(|font| (font.family.as_str(), font.weight, font.italic))
            .collect();
        registered.sort_unstable();
        assert_eq!(
            registered,
            [
                ("Brand Grotesk", 600, false),
                ("Helvetica Neue", 400, false),
                ("Inter", 400, false),
                ("Inter", 700, false),
            ]
        );
        let manifest: Value =
            serde_json::from_slice(&std::fs::read(site.join("fanta/manifest.json"))?)?;
        assert_eq!(manifest["format"], SITE_FORMAT);
        assert_eq!(manifest["pages"][0]["slug"], "home");
        Ok(())
    }

    #[test]
    fn re_export_removes_renamed_pages_keeps_hosting_files_and_refuses_unrelated_directories()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut document = Doc::new();
        let first = page(&mut document, "Overview")?;
        text(&mut document, first, "Overview", "Inter", 400)?;
        let output = tempfile::tempdir()?;
        let source = project(&document)?;
        export_project(source.path(), output.path(), &assets(), &options())?;
        std::fs::write(output.path().join("CNAME"), "designs.example.com")?;
        std::fs::write(output.path().join("overview/notes.txt"), "kept")?;

        if let Some(node) = document.scene.get_mut(first) {
            node.name = "Summary".into();
        }
        let renamed = project(&document)?;
        export_project(renamed.path(), output.path(), &assets(), &options())?;
        assert!(output.path().join("summary/index.html").is_file());
        assert!(!output.path().join("overview/index.html").exists());
        assert!(output.path().join("overview/notes.txt").is_file());
        assert!(output.path().join("CNAME").is_file());

        let unrelated = tempfile::tempdir()?;
        std::fs::write(unrelated.path().join("index.html"), "mine")?;
        assert!(matches!(
            export_project(renamed.path(), unrelated.path(), &assets(), &options()),
            Err(SiteError::OccupiedOutput(_))
        ));
        assert_eq!(
            std::fs::read_to_string(unrelated.path().join("index.html"))?,
            "mine"
        );
        Ok(())
    }

    #[test]
    fn text_the_embedded_faces_cannot_render_fails_the_export_naming_the_text()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut document = Doc::new();
        let localized = page(&mut document, "Localized")?;
        text(&mut document, localized, "日本語", "Inter", 400)?;
        let source = project(&document)?;
        let output = tempfile::tempdir()?;
        let error = export_project(source.path(), output.path(), &assets(), &options())
            .expect_err("Inter has no Japanese glyphs");
        let message = error.to_string();
        assert!(
            matches!(error, SiteError::Runtime(_))
                && message.contains("Text \"日本語\"")
                && message.contains("U+65E5"),
            "{message}"
        );
        assert!(!output.path().join("index.html").exists());
        Ok(())
    }

    #[test]
    fn missing_weights_of_an_explicit_substitute_are_reported()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut document = Doc::new();
        let heading = page(&mut document, "Heading")?;
        text(&mut document, heading, "Light", "Display", 300)?;
        let source = project(&document)?;
        let output = tempfile::tempdir()?;
        let mut options = options();
        options
            .substitutions
            .insert("Display".into(), "Inter".into());
        let error = export_project(source.path(), output.path(), &assets(), &options)
            .expect_err("bundled Inter has no 300 weight");
        assert!(matches!(
            error,
            SiteError::MissingFace { family, weight: 300, italic: false } if family == "Display"
        ));
        Ok(())
    }
}
