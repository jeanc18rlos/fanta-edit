use fanta_doc::motion::CompiledMotionLibrary;
use fanta_doc::{
    AssetId, Doc, NodeId, StyleAssignment, StyleDeclaration, StyleId, StyleProperty, StyleRegistry,
    StyleRule, VariableId,
};
use fanta_fnx::{FnxElement, FnxSidecar, IdEntry, RefTable};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::io::{Cursor, Read, Write};
use std::rc::Rc;
use zip::write::SimpleFileOptions;

pub const RUNTIME_BUNDLE_VERSION: u32 = 1;
const MAX_BUNDLE_BYTES: u64 = 512 * 1024 * 1024;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SourceBundle {
    pub files: BTreeMap<String, Vec<u8>>,
    #[serde(default)]
    pub fonts: Vec<RuntimeFont>,
    #[serde(default)]
    pub font_substitutions: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeFont {
    pub family: String,
    #[serde(default = "default_font_weight")]
    pub weight: u16,
    #[serde(default)]
    pub italic: bool,
    #[serde(default)]
    pub face_index: u32,
    pub bytes: Vec<u8>,
}

fn default_font_weight() -> u16 {
    400
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeDiagnostic {
    pub code: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node: Option<NodeId>,
}

#[derive(Debug, thiserror::Error)]
pub enum RuntimeError {
    #[error("{path}: {message}")]
    Source { path: String, message: String },
    #[error("invalid runtime bundle: {0}")]
    Bundle(String),
    #[error("{0}")]
    Json(#[from] serde_json::Error),
    #[error("{0}")]
    Zip(#[from] zip::result::ZipError),
    #[error("{0}")]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Clone)]
pub struct RuntimeProgram {
    inner: Rc<ProgramData>,
}

#[derive(Debug)]
struct ProgramData {
    document: Doc,
    motion: Rc<CompiledMotionLibrary>,
    resources: BTreeMap<AssetId, Vec<u8>>,
    fonts: Vec<RuntimeFont>,
    font_substitutions: BTreeMap<String, String>,
    diagnostics: Vec<RuntimeDiagnostic>,
}

#[derive(Debug, Serialize, Deserialize)]
struct BundleManifest {
    format: String,
    version: u32,
    schema_version: u32,
    document: ResourceDigest,
    assets: BTreeMap<AssetId, ResourceDigest>,
    fonts: Vec<FontDescriptor>,
    font_substitutions: BTreeMap<String, String>,
    #[serde(default)]
    pending_layout: BTreeSet<NodeId>,
}

#[derive(Debug, Serialize, Deserialize)]
struct FontDescriptor {
    family: String,
    weight: u16,
    italic: bool,
    #[serde(default)]
    face_index: u32,
    resource: ResourceDigest,
}

#[derive(Debug, Serialize, Deserialize)]
struct ResourceDigest {
    path: String,
    byte_length: u64,
    sha256: String,
}

impl ResourceDigest {
    fn new(path: String, bytes: &[u8]) -> Self {
        Self {
            path,
            byte_length: bytes.len() as u64,
            sha256: checksum(bytes),
        }
    }
}

impl RuntimeProgram {
    pub fn compile(source: &SourceBundle) -> Result<Self, RuntimeError> {
        for path in source.files.keys() {
            validate_path(path)?;
        }
        if source.files.contains_key("doc.json") && source.files.contains_key("fanta.json") {
            return Err(source_error(
                "doc.json",
                "source bundle cannot contain both a canonical document and a project source manifest",
            ));
        }
        let document = if let Some(bytes) = source.files.get("doc.json") {
            let mut value = strict_json(bytes).map_err(|error| source_error("doc.json", error))?;
            if let Some(object) = value.as_object_mut() {
                object
                    .entry("metadata")
                    .or_insert_with(|| json!({"title":"Untitled","created_at":0,"modified_at":0}));
            }
            Doc::from_json_str(&value.to_string())
                .map_err(|error| source_error("doc.json", error))?
        } else {
            compile_project(source)?
        };
        let mut resources = BTreeMap::new();
        for (path, bytes) in &source.files {
            if !path.starts_with("assets/") || path == "assets/index.json" {
                continue;
            }
            let filename = path
                .rsplit('/')
                .next()
                .ok_or_else(|| source_error(path, "missing asset filename"))?;
            let stem = filename
                .split('.')
                .next()
                .ok_or_else(|| source_error(path, "missing asset id"))?;
            let id = stem
                .parse::<AssetId>()
                .map_err(|error| source_error(path, error))?;
            if resources.insert(id, bytes.clone()).is_some() {
                return Err(source_error(path, "duplicate asset identity"));
            }
        }
        verify_source_asset_index(&source.files, &resources)?;
        Self::from_parts(
            document,
            resources,
            source.fonts.clone(),
            source.font_substitutions.clone(),
        )
    }

    pub fn from_document(
        document: Doc,
        resources: BTreeMap<AssetId, Vec<u8>>,
        fonts: Vec<RuntimeFont>,
        font_substitutions: BTreeMap<String, String>,
    ) -> Result<Self, RuntimeError> {
        Self::from_parts(document, resources, fonts, font_substitutions)
    }

    fn from_parts(
        document: Doc,
        resources: BTreeMap<AssetId, Vec<u8>>,
        mut fonts: Vec<RuntimeFont>,
        font_substitutions: BTreeMap<String, String>,
    ) -> Result<Self, RuntimeError> {
        let pending_layout = document.pending_layout.clone();
        let mut document = Doc::from_json_str(&document.clone_for_persist().to_json_string()?)
            .map_err(|error| source_error("doc.json", error))?;
        document.pending_layout = pending_layout;
        validate_document(&document, &resources)?;
        fonts.sort_by(|left, right| {
            (&left.family, left.weight, left.italic).cmp(&(
                &right.family,
                right.weight,
                right.italic,
            ))
        });
        let mut previous = None;
        for font in &fonts {
            if font.family.trim().is_empty()
                || !(1..=1000).contains(&font.weight)
                || font.bytes.is_empty()
            {
                return Err(source_error(
                    "fonts",
                    "font registrations require a family, weight in 1..=1000, and font bytes",
                ));
            }
            let key = (&font.family, font.weight, font.italic);
            if previous == Some(key) {
                return Err(source_error("fonts", "duplicate font registration"));
            }
            previous = Some(key);
        }
        for (requested, provided) in &font_substitutions {
            if requested.trim().is_empty() || !fonts.iter().any(|font| &font.family == provided) {
                return Err(source_error(
                    "fonts",
                    format!(
                        "font substitution {requested:?} requires registered family {provided:?}"
                    ),
                ));
            }
        }
        let mut diagnostics = Vec::new();
        for id in document
            .scene
            .roots()
            .iter()
            .flat_map(|root| document.scene.descendants_of(*root))
            .collect::<BTreeSet<_>>()
        {
            let node = document
                .scene
                .get(id)
                .ok_or_else(|| source_error("scene", "missing node"))?;
            if matches!(node.data, fanta_doc::NodeData::Video(_))
                || contains_video_paint(&serde_json::to_value(&node.data)?)
            {
                diagnostics.push(RuntimeDiagnostic {
                    code: "poster_only_video".into(),
                    message: "Video uses its authored poster; live video playback is unavailable"
                        .into(),
                    node: Some(id),
                });
            } else if matches!(node.data, fanta_doc::NodeData::Audio(_)) {
                diagnostics.push(RuntimeDiagnostic {
                    code: "audio_unavailable".into(),
                    message: "Live audio playback is unavailable".into(),
                    node: Some(id),
                });
            }
        }
        let motion = Rc::new(document.motion.compile());
        Ok(Self {
            inner: Rc::new(ProgramData {
                document,
                motion,
                resources,
                fonts,
                font_substitutions,
                diagnostics,
            }),
        })
    }

    pub fn document(&self) -> &Doc {
        &self.inner.document
    }
    /// Compiled once from [`Self::document`]'s motion so every player of this
    /// program samples clips through one shared index.
    pub fn compiled_motion(&self) -> &Rc<CompiledMotionLibrary> {
        &self.inner.motion
    }
    pub fn resources(&self) -> &BTreeMap<AssetId, Vec<u8>> {
        &self.inner.resources
    }
    pub fn fonts(&self) -> &[RuntimeFont] {
        &self.inner.fonts
    }
    pub fn font_substitutions(&self) -> &BTreeMap<String, String> {
        &self.inner.font_substitutions
    }
    pub fn diagnostics(&self) -> &[RuntimeDiagnostic] {
        &self.inner.diagnostics
    }

    pub fn to_bundle(&self) -> Result<Vec<u8>, RuntimeError> {
        let document = canonical_json(
            &self
                .document()
                .clone_for_persist()
                .to_authored_json_value()?,
        )?;
        let mut entries = BTreeMap::from([("doc.json".to_owned(), document.clone())]);
        let mut assets = BTreeMap::new();
        for (id, bytes) in self.resources() {
            let path = format!("assets/{id}.bin");
            assets.insert(*id, ResourceDigest::new(path.clone(), bytes));
            entries.insert(path, bytes.clone());
        }
        let mut fonts = Vec::new();
        for (index, font) in self.fonts().iter().enumerate() {
            let path = format!("fonts/{index:04}-{}.bin", checksum(&font.bytes));
            fonts.push(FontDescriptor {
                family: font.family.clone(),
                weight: font.weight,
                italic: font.italic,
                face_index: font.face_index,
                resource: ResourceDigest::new(path.clone(), &font.bytes),
            });
            entries.insert(path, font.bytes.clone());
        }
        let manifest = BundleManifest {
            format: "fanta-runtime".into(),
            version: RUNTIME_BUNDLE_VERSION,
            schema_version: fanta_doc::SCHEMA_VERSION,
            document: ResourceDigest::new("doc.json".into(), &document),
            assets,
            fonts,
            font_substitutions: self.font_substitutions().clone(),
            pending_layout: self.document().pending_layout.clone(),
        };
        entries.insert(
            "manifest.json".into(),
            canonical_json(&serde_json::to_value(manifest)?)?,
        );
        let mut archive = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let options = SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .last_modified_time(zip::DateTime::default())
            .unix_permissions(0o644);
        for (path, bytes) in entries {
            archive.start_file(path, options)?;
            archive.write_all(&bytes)?;
        }
        Ok(archive.finish()?.into_inner())
    }

    pub fn load_bundle(bytes: &[u8]) -> Result<Self, RuntimeError> {
        Self::load_bundle_with_limit(bytes, MAX_BUNDLE_BYTES)
    }

    pub fn load_bundle_with_limit(bytes: &[u8], maximum_bytes: u64) -> Result<Self, RuntimeError> {
        if maximum_bytes == 0 || bytes.len() as u64 > maximum_bytes {
            return Err(RuntimeError::Bundle("archive exceeds size limit".into()));
        }
        let mut archive = zip::ZipArchive::new(Cursor::new(bytes))?;
        let mut entries = BTreeMap::new();
        let mut total_size = 0u64;
        for index in 0..archive.len() {
            let entry = archive.by_index(index)?;
            total_size = total_size
                .checked_add(entry.size())
                .ok_or_else(|| RuntimeError::Bundle("archive size overflows".into()))?;
            if total_size > maximum_bytes {
                return Err(RuntimeError::Bundle(
                    "expanded archive exceeds size limit".into(),
                ));
            }
        }
        for index in 0..archive.len() {
            let mut entry = archive.by_index(index)?;
            let path = entry.name().to_owned();
            validate_path(&path)?;
            if entry.is_dir()
                || entry
                    .unix_mode()
                    .is_some_and(|mode| mode & 0o170000 == 0o120000)
            {
                return Err(RuntimeError::Bundle(
                    "only regular resource entries are supported".into(),
                ));
            }
            let mut contents = Vec::new();
            entry
                .by_ref()
                .take(maximum_bytes.saturating_add(1))
                .read_to_end(&mut contents)?;
            if contents.len() as u64 != entry.size() {
                return Err(RuntimeError::Bundle(
                    "archive resource length mismatch".into(),
                ));
            }
            if entries.insert(path.clone(), contents).is_some() {
                return Err(RuntimeError::Bundle(format!(
                    "duplicate archive path {path}"
                )));
            }
        }
        let manifest_value = strict_json(required(&entries, "manifest.json")?)?;
        if let Some(assets) = manifest_value.get("assets").and_then(Value::as_object) {
            let mut identities = BTreeSet::new();
            for key in assets.keys() {
                let identity: AssetId = serde_json::from_value(Value::String(key.clone()))?;
                if !identities.insert(identity) {
                    return Err(RuntimeError::Bundle(
                        "duplicate canonical asset identity".into(),
                    ));
                }
            }
        }
        let manifest: BundleManifest = serde_json::from_value(manifest_value)?;
        if manifest.format != "fanta-runtime"
            || manifest.version != RUNTIME_BUNDLE_VERSION
            || !(1..=fanta_doc::SCHEMA_VERSION).contains(&manifest.schema_version)
        {
            return Err(RuntimeError::Bundle("unsupported format or version".into()));
        }
        let mut expected_paths = BTreeSet::from(["manifest.json".to_owned()]);
        let document_bytes = verified(&entries, &manifest.document, &mut expected_paths)?;
        let document_value = strict_json(document_bytes)?;
        if document_value.get("metadata").is_none() {
            return Err(RuntimeError::Bundle(
                "authored document metadata is missing".into(),
            ));
        }
        if document_value
            .get("schema_version")
            .and_then(Value::as_u64)
            .unwrap_or(1)
            != u64::from(manifest.schema_version)
        {
            return Err(RuntimeError::Bundle(
                "manifest schema disagrees with the authored document schema".into(),
            ));
        }
        let mut document = Doc::from_json_str(&document_value.to_string())
            .map_err(|error| RuntimeError::Bundle(error.to_string()))?;
        if manifest
            .pending_layout
            .iter()
            .any(|id| !document.scene.contains(*id))
        {
            return Err(RuntimeError::Bundle(
                "pending layout references a missing node".into(),
            ));
        }
        document.pending_layout = manifest.pending_layout;
        let mut resources = BTreeMap::new();
        for (id, resource) in manifest.assets {
            resources.insert(
                id,
                verified(&entries, &resource, &mut expected_paths)?.to_vec(),
            );
        }
        let mut fonts = Vec::new();
        for font in manifest.fonts {
            fonts.push(RuntimeFont {
                family: font.family,
                weight: font.weight,
                italic: font.italic,
                face_index: font.face_index,
                bytes: verified(&entries, &font.resource, &mut expected_paths)?.to_vec(),
            });
        }
        if expected_paths.len() != entries.len() {
            return Err(RuntimeError::Bundle(
                "archive contains undeclared resources".into(),
            ));
        }
        Self::from_parts(document, resources, fonts, manifest.font_substitutions)
    }
}

fn verified<'a>(
    entries: &'a BTreeMap<String, Vec<u8>>,
    resource: &ResourceDigest,
    expected: &mut BTreeSet<String>,
) -> Result<&'a [u8], RuntimeError> {
    if !expected.insert(resource.path.clone()) {
        return Err(RuntimeError::Bundle(
            "resource path is claimed twice".into(),
        ));
    }
    let bytes = required(entries, &resource.path)?;
    if bytes.len() as u64 != resource.byte_length || checksum(bytes) != resource.sha256 {
        return Err(RuntimeError::Bundle(format!(
            "resource checksum mismatch at {}",
            resource.path
        )));
    }
    Ok(bytes)
}

fn checksum(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn canonical_json(value: &Value) -> Result<Vec<u8>, RuntimeError> {
    let mut value = value.clone();
    value.sort_all_objects();
    Ok(serde_json::to_vec(&value)?)
}
fn source_error(path: &str, message: impl std::fmt::Display) -> RuntimeError {
    RuntimeError::Source {
        path: path.to_owned(),
        message: message.to_string(),
    }
}
fn text<'a>(path: &str, bytes: &'a [u8]) -> Result<&'a str, RuntimeError> {
    std::str::from_utf8(bytes).map_err(|error| source_error(path, error))
}
fn required<'a>(
    files: &'a BTreeMap<String, Vec<u8>>,
    path: &str,
) -> Result<&'a [u8], RuntimeError> {
    files
        .get(path)
        .map(Vec::as_slice)
        .ok_or_else(|| source_error(path, "required resource is missing"))
}
fn json_file(
    files: &BTreeMap<String, Vec<u8>>,
    path: &str,
    default: Option<Value>,
) -> Result<Value, RuntimeError> {
    match files.get(path) {
        Some(bytes) => strict_json(bytes).map_err(|error| source_error(path, error)),
        None => default.ok_or_else(|| source_error(path, "required source is missing")),
    }
}

fn validate_path(path: &str) -> Result<(), RuntimeError> {
    if path.is_empty()
        || path.starts_with('/')
        || path.contains('\\')
        || path.contains(':')
        || path.contains('\0')
        || path
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err(source_error(
            path,
            "paths must be normalized project-relative paths",
        ));
    }
    Ok(())
}

fn verify_source_asset_index(
    files: &BTreeMap<String, Vec<u8>>,
    resources: &BTreeMap<AssetId, Vec<u8>>,
) -> Result<(), RuntimeError> {
    let Some(bytes) = files.get("assets/index.json") else {
        return Ok(());
    };
    let index = strict_json(bytes).map_err(|error| source_error("assets/index.json", error))?;
    if index.get("version").and_then(Value::as_u64) != Some(1) {
        return Err(source_error(
            "assets/index.json",
            "unsupported asset index version",
        ));
    }
    let records = index
        .get("assets")
        .and_then(Value::as_object)
        .ok_or_else(|| source_error("assets/index.json", "asset records are missing"))?;
    if records.len() != resources.len() {
        return Err(source_error(
            "assets/index.json",
            "indexed resources do not match supplied resources",
        ));
    }
    for (name, record) in records {
        let id: AssetId = name
            .parse()
            .map_err(|error| source_error("assets/index.json", error))?;
        let bytes = resources
            .get(&id)
            .ok_or_else(|| source_error("assets/index.json", format!("asset {id} is missing")))?;
        if record.get("size").and_then(Value::as_u64) != Some(bytes.len() as u64)
            || record.get("sha256").and_then(Value::as_str) != Some(checksum(bytes).as_str())
        {
            return Err(source_error(
                "assets/index.json",
                format!("asset {id} checksum or length differs"),
            ));
        }
    }
    Ok(())
}

fn resolve_import(source_path: &str, import: &str) -> Result<String, RuntimeError> {
    if import.starts_with('/')
        || import.contains('\\')
        || import.contains(':')
        || import.contains('\0')
    {
        return Err(source_error(
            source_path,
            "style import escapes the source bundle",
        ));
    }
    let mut parts: Vec<&str> = source_path.split('/').collect();
    parts.pop();
    for part in import.split('/') {
        match part {
            "." | "" => {}
            ".." => {
                if parts.pop().is_none() {
                    return Err(source_error(
                        source_path,
                        "style import escapes the source bundle",
                    ));
                }
            }
            other => parts.push(other),
        }
    }
    let path = parts.join("/");
    validate_path(&path)?;
    Ok(path)
}

fn compile_project(source: &SourceBundle) -> Result<Doc, RuntimeError> {
    let files = &source.files;
    if let Some(path) = files.keys().find(|path| {
        path.ends_with(".fnx")
            && (path.starts_with("motion/")
                || path.starts_with("prototypes/")
                || path.starts_with("audio/"))
    }) {
        return Err(source_error(
            path,
            "this artifact's FNX schema is not implemented; supply authored prototype configuration and a MotionLibrary in doc/ JSON",
        ));
    }
    let manifest = json_file(files, "fanta.json", None)?;
    if manifest.get("format").and_then(Value::as_str) != Some("fanta-project") {
        return Err(source_error("fanta.json", "expected fanta-project format"));
    }
    let version = manifest
        .get("version")
        .and_then(Value::as_u64)
        .ok_or_else(|| source_error("fanta.json", "project version is missing"))?;
    if !(1..=6).contains(&version) {
        return Err(source_error("fanta.json", "unsupported project version"));
    }
    let schema_version = manifest
        .get("schema_version")
        .and_then(Value::as_u64)
        .ok_or_else(|| source_error("fanta.json", "schema version is missing"))?;
    if !(1..=u64::from(fanta_doc::SCHEMA_VERSION)).contains(&schema_version) {
        return Err(source_error("fanta.json", "unsupported document schema"));
    }
    let document_id: fanta_doc::DocId = manifest
        .get("project_id")
        .and_then(Value::as_str)
        .ok_or_else(|| source_error("fanta.json", "project identity is missing"))?
        .parse()
        .map_err(|error| source_error("fanta.json", error))?;
    let mut root = Map::new();
    root.insert("id".into(), serde_json::to_value(document_id)?);
    root.insert("schema_version".into(), Value::from(schema_version));
    root.insert(
        "metadata".into(),
        json_file(files, "doc/metadata.json", None)?,
    );
    for (name, fallback) in [
        ("asset_library", json!({})),
        ("variables", json!({})),
        ("active_modes", json!({})),
        ("motion", json!({})),
        ("flows", json!([])),
        ("flow_start", Value::Null),
        ("presentation", Value::Null),
    ] {
        root.insert(
            name.into(),
            json_file(files, &format!("doc/{name}.json"), Some(fallback))?,
        );
    }
    let mut defs = Map::new();
    let mut sets = Map::new();
    for path in files.keys() {
        if path.starts_with("components/") && path.ends_with("/def.json") {
            let definition = json_file(files, path, None)?;
            insert_identified(&mut defs, definition, path)?;
        }
        if path.starts_with("components/") && path.ends_with("/set.json") {
            let definition = json_file(files, path, None)?;
            insert_identified(&mut sets, definition, path)?;
        }
    }
    if let Some(legacy) = json_file(files, "components/sets.json", Some(Value::Null))?.as_object() {
        for (id, set) in legacy {
            if sets.insert(id.clone(), set.clone()).is_some() {
                return Err(source_error(
                    "components/sets.json",
                    "duplicate component set",
                ));
            }
        }
    }
    let mut refs = RefTable::new(false);
    for (id, definition) in &defs {
        if let Some(name) = definition.get("name").and_then(Value::as_str) {
            refs.insert_component(name, id);
        }
    }
    if let Some(variables) = root.get("variables") {
        if let Some(entries) = variables.get("variables").and_then(Value::as_object) {
            for (id, variable) in entries {
                if let (Some(name), Some(collection)) = (
                    variable.get("name").and_then(Value::as_str),
                    variable.get("collection").and_then(Value::as_str),
                ) {
                    if let Some(collection_name) = variables
                        .get("collections")
                        .and_then(|collections| collections.get(collection))
                        .and_then(|collection| collection.get("name"))
                        .and_then(Value::as_str)
                    {
                        refs.insert_variable(&format!("{collection_name}/{name}"), id);
                    }
                }
            }
        }
    }
    root.insert("components".into(), json!({"defs":defs,"sets":sets}));
    let mut styles: StyleRegistry =
        serde_json::from_value(json_file(files, "doc/styles.json", Some(json!({})))?)?;
    let mut imported_styles = BTreeSet::new();
    let mut nodes = Map::new();
    let mut pages = Vec::new();
    let mut pending_layout = BTreeSet::new();
    for path in files.keys().filter(|path| {
        path.ends_with(".fnx")
            && (path.starts_with("pages/")
                || path.starts_with("components/")
                || path.starts_with("graphics/"))
    }) {
        let source_text = text(path, required(files, path)?)?;
        for import in
            fanta_fnx::stylesheet_imports(source_text).map_err(|error| source_error(path, error))?
        {
            let imported = resolve_import(path, &import)?;
            if imported_styles.insert(imported.clone()) {
                let fss = text(&imported, required(files, &imported)?)?;
                for rule in
                    fanta_fnx::parse_fss(fss).map_err(|error| source_error(&imported, error))?
                {
                    let id: StyleId = rule
                        .selector
                        .parse()
                        .map_err(|error| source_error(&imported, error))?;
                    let mut declarations = BTreeMap::new();
                    for (property, declaration) in rule.declarations {
                        let property = StyleProperty::parse(&property)
                            .map_err(|error| source_error(&imported, error))?;
                        let binding = declaration
                            .binding
                            .map(|binding| resolve_variable_reference(&refs, &binding, &imported))
                            .transpose()?;
                        if declarations
                            .insert(
                                property,
                                StyleDeclaration {
                                    value: declaration.value,
                                    binding,
                                },
                            )
                            .is_some()
                        {
                            return Err(source_error(
                                &imported,
                                "duplicate normalized style property",
                            ));
                        }
                    }
                    if styles
                        .rules
                        .insert(id, StyleRule { id, declarations })
                        .is_some()
                    {
                        return Err(source_error(
                            &imported,
                            format!("duplicate style identity {id}"),
                        ));
                    }
                }
            }
        }
        let tree = fanta_fnx::parse_doc_with(source_text, &refs)
            .map_err(|error| source_error(path, error))?;
        let sidecar_path = path
            .strip_suffix(".fnx")
            .map(|base| format!("{base}.ids.json"))
            .ok_or_else(|| source_error(path, "invalid FNX path"))?;
        let sidecar = match files.get(&sidecar_path) {
            Some(bytes) => {
                let persisted: FnxSidecar = serde_json::from_value(
                    strict_json(bytes).map_err(|error| source_error(&sidecar_path, error))?,
                )
                .map_err(|error| source_error(&sidecar_path, error))?;
                validate_sidecar(&tree, &persisted, path)?;
                persisted
            }
            None => explicit_sidecar(&tree, path)?,
        };
        let mut decoded =
            fanta_fnx::nodes_from_tree(&tree, &sidecar.ids, sidecar.root_parent.as_deref())
                .map_err(|error| source_error(path, error))?;
        for node in &mut decoded {
            if !node.get("local_size").is_some_and(|size| !size.is_null())
                && matches!(
                    node.get("type").and_then(Value::as_str),
                    Some("text" | "instance")
                )
            {
                let id = normalize_node_id(
                    node.get("id")
                        .ok_or_else(|| source_error(path, "missing node identity"))?,
                    path,
                )?;
                pending_layout.insert(serde_json::from_value(id)?);
                if node.get("type").and_then(Value::as_str) == Some("text")
                    && node.get("auto_resize").is_none()
                {
                    node.as_object_mut()
                        .ok_or_else(|| source_error(path, "node must be an object"))?
                        .insert(
                            "auto_resize".into(),
                            Value::String("width_and_height".into()),
                        );
                }
            }
            compile_node_styles(node, &styles, path)?;
            hydrate_geometry(node);
        }
        if path.ends_with("/page.fnx") {
            let directory = path
                .rsplit_once('/')
                .map(|(directory, _)| directory)
                .ok_or_else(|| source_error(path, "invalid page directory"))?;
            let header = json_file(files, &format!("{directory}/page.json"), None)?;
            let root_id = decoded
                .first()
                .and_then(|node| node.get("id"))
                .cloned()
                .ok_or_else(|| source_error(path, "missing FNX root"))?;
            if let Some(header_id) = header.get("id") {
                let header_id = normalize_node_id(header_id, path)?;
                if header_id != root_id {
                    return Err(source_error(
                        path,
                        "page header identity disagrees with FNX root",
                    ));
                }
            }
            let order = header
                .get("order")
                .and_then(Value::as_u64)
                .ok_or_else(|| source_error(path, "page order is missing"))?;
            if let Some(root) = decoded.first_mut().and_then(Value::as_object_mut) {
                if root.get("type").and_then(Value::as_str) != Some("group") {
                    return Err(source_error(path, "page root must be a Frame"));
                }
                root.remove("clip_size");
                root.remove("local_size");
            }
            pages.push((order, path.clone(), root_id));
        }
        for node in decoded {
            insert_identified(&mut nodes, node, path)?;
        }
    }
    for path in files
        .keys()
        .filter(|path| path.contains("/nodes/") && path.ends_with(".json"))
    {
        // A design's legacy node folder is only authoritative when it has no FNX.
        let Some((directory, _)) = path.split_once("/nodes/") else {
            continue;
        };
        if files.keys().any(|candidate| {
            candidate.starts_with(&format!("{directory}/")) && candidate.ends_with(".fnx")
        }) {
            continue;
        }
        let mut node = json_file(files, path, None)?;
        compile_node_styles(&mut node, &styles, path)?;
        hydrate_geometry(&mut node);
        insert_identified(&mut nodes, node, path)?;
    }
    for path in files
        .keys()
        .filter(|path| path.starts_with("pages/") && path.ends_with("/page.json"))
    {
        let directory = path
            .strip_suffix("/page.json")
            .ok_or_else(|| source_error(path, "invalid page header path"))?;
        if files.contains_key(&format!("{directory}/page.fnx")) {
            continue;
        }
        let header = json_file(files, path, None)?;
        let identity = match header.get("id") {
            Some(identity) => normalize_node_id(identity, path)?,
            None => {
                let name = directory
                    .rsplit('/')
                    .next()
                    .ok_or_else(|| source_error(path, "page identity is missing"))?;
                let id: NodeId = name.parse().map_err(|error| source_error(path, error))?;
                serde_json::to_value(id)?
            }
        };
        let key = identity
            .as_str()
            .ok_or_else(|| source_error(path, "invalid page identity"))?;
        let node = nodes
            .get_mut(key)
            .and_then(Value::as_object_mut)
            .ok_or_else(|| source_error(path, "page design source is missing"))?;
        if node.get("type").and_then(Value::as_str) != Some("group") {
            return Err(source_error(path, "page root must be a Frame"));
        }
        node.remove("local_size");
        node.remove("clip_size");
        let order = header
            .get("order")
            .and_then(Value::as_u64)
            .ok_or_else(|| source_error(path, "page order is missing"))?;
        pages.push((order, path.clone(), identity));
    }
    pages.sort_by(|left, right| (left.0, &left.1).cmp(&(right.0, &right.1)));
    root.insert(
        "pages".into(),
        Value::Array(pages.into_iter().map(|(_, _, id)| id).collect()),
    );
    root.insert("styles".into(), serde_json::to_value(styles)?);
    root.insert("scene".into(), json!({"nodes":nodes}));
    for path in files
        .keys()
        .filter(|path| path.starts_with("motion/") && path.ends_with("/motion.json"))
    {
        let value = json_file(files, path, None)?;
        let library: fanta_doc::MotionLibrary =
            serde_json::from_value(value).map_err(|error| source_error(path, error))?;
        let motion = root
            .get_mut("motion")
            .ok_or_else(|| source_error(path, "motion library missing"))?;
        let mut existing: fanta_doc::MotionLibrary = serde_json::from_value(motion.clone())
            .map_err(|error| source_error("doc/motion.json", error))?;
        for (id, clip) in library.clips {
            if existing.clips.insert(id, clip).is_some() {
                return Err(source_error(path, "duplicate animation clip"));
            }
        }
        *motion = serde_json::to_value(existing)?;
    }
    let mut document = Doc::from_json_str(&Value::Object(root).to_string())
        .map_err(|error| source_error("fanta.json", error))?;
    document.pending_layout = pending_layout;
    Ok(document)
}

fn normalize_node_id(value: &Value, path: &str) -> Result<Value, RuntimeError> {
    let string = value
        .as_str()
        .ok_or_else(|| source_error(path, "identity must be a string"))?;
    let id: NodeId = if string.starts_with("n_") {
        string.parse().map_err(|error| source_error(path, error))?
    } else {
        serde_json::from_value(value.clone()).map_err(|error| source_error(path, error))?
    };
    Ok(serde_json::to_value(id)?)
}

fn insert_identified(
    entries: &mut Map<String, Value>,
    value: Value,
    path: &str,
) -> Result<(), RuntimeError> {
    let id = value
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| source_error(path, "authoritative identity is missing"))?
        .to_owned();
    if entries.insert(id.clone(), value).is_some() {
        return Err(source_error(path, format!("duplicate identity {id}")));
    }
    Ok(())
}

fn explicit_sidecar(tree: &FnxElement, path: &str) -> Result<FnxSidecar, RuntimeError> {
    fn visit(
        tree: &FnxElement,
        parent_index: Option<u32>,
        index: usize,
        entries: &mut Vec<IdEntry>,
        path: &str,
    ) -> Result<(), RuntimeError> {
        let id = normalize_node_id(
            tree.attrs.get("id").ok_or_else(|| {
                source_error(
                    path,
                    "node is missing authoritative id and no identity sidecar was supplied",
                )
            })?,
            path,
        )?;
        let position =
            u32::try_from(entries.len()).map_err(|_| source_error(path, "too many nodes"))?;
        entries.push(IdEntry {
            id: id
                .as_str()
                .ok_or_else(|| source_error(path, "invalid node identity"))?
                .to_owned(),
            index: Value::from(index as u64),
            tag: Some(tree.tag.clone()),
            name: tree
                .attrs
                .get("name")
                .and_then(Value::as_str)
                .map(str::to_owned),
            parent_index,
        });
        for (index, child) in tree.children.iter().enumerate() {
            visit(child, Some(position), index + 1, entries, path)?;
        }
        Ok(())
    }
    let mut ids = Vec::new();
    visit(tree, None, 1, &mut ids, path)?;
    let mut seen = BTreeSet::new();
    for entry in &ids {
        if !seen.insert(&entry.id) {
            return Err(source_error(path, "duplicate node identity"));
        }
    }
    Ok(FnxSidecar {
        root_parent: None,
        ids,
    })
}

fn validate_sidecar(
    tree: &FnxElement,
    sidecar: &FnxSidecar,
    path: &str,
) -> Result<(), RuntimeError> {
    let mut seen = BTreeSet::new();
    for entry in &sidecar.ids {
        let id: NodeId = serde_json::from_value(Value::String(entry.id.clone()))
            .map_err(|error| source_error(path, error))?;
        if !seen.insert(id) {
            return Err(source_error(path, "duplicate sidecar identity"));
        }
    }
    fanta_fnx::nodes_from_tree(tree, &sidecar.ids, sidecar.root_parent.as_deref())
        .map_err(|error| source_error(path, error))?;
    Ok(())
}

fn resolve_variable_reference(
    refs: &RefTable,
    reference: &str,
    path: &str,
) -> Result<VariableId, RuntimeError> {
    let mut tree = FnxElement::new("Frame");
    tree.attrs
        .insert("bindings".into(), json!({"opacity":reference}));
    fanta_fnx::desugar_refs(&mut tree, refs).map_err(|error| source_error(path, error))?;
    let id = tree
        .attrs
        .get("bindings")
        .and_then(Value::as_array)
        .and_then(|pairs| pairs.first())
        .and_then(Value::as_array)
        .and_then(|pair| pair.get(1))
        .cloned()
        .ok_or_else(|| source_error(path, "invalid variable reference"))?;
    serde_json::from_value(id).map_err(|error| source_error(path, error))
}

fn compile_node_styles(
    value: &mut Value,
    styles: &StyleRegistry,
    path: &str,
) -> Result<(), RuntimeError> {
    let object = value
        .as_object_mut()
        .ok_or_else(|| source_error(path, "node must be an object"))?;
    let has_authored_assignment = object.contains_key("style_assignment");
    let mut assignment: StyleAssignment = serde_json::from_value(
        object
            .remove("style_assignment")
            .unwrap_or_else(|| json!({})),
    )?;
    if has_authored_assignment && object.contains_key("class") {
        return Err(source_error(
            path,
            "class and style_assignment cannot both own the style list",
        ));
    }
    let authored_bindings = object.get("bindings").cloned().unwrap_or_else(|| json!([]));
    let binding_pairs: Vec<(fanta_doc::BoundProp, VariableId)> = authored_bindings
        .as_array()
        .ok_or_else(|| source_error(path, "bindings must be a pair array"))?
        .iter()
        .map(|pair| {
            let pair = pair
                .as_array()
                .filter(|pair| pair.len() == 2)
                .ok_or_else(|| source_error(path, "invalid binding pair"))?;
            Ok((
                serde_json::from_value(
                    pair.first()
                        .cloned()
                        .ok_or_else(|| source_error(path, "binding property missing"))?,
                )?,
                serde_json::from_value(
                    pair.get(1)
                        .cloned()
                        .ok_or_else(|| source_error(path, "binding variable missing"))?,
                )?,
            ))
        })
        .collect::<Result<_, RuntimeError>>()?;
    let mut binding_map = BTreeMap::new();
    for (property, variable) in binding_pairs {
        if binding_map.insert(property, variable).is_some() {
            return Err(source_error(path, "duplicate binding property"));
        }
    }
    if let Some(classes) = object.remove("class") {
        let classes = classes
            .as_str()
            .ok_or_else(|| source_error(path, "class must be a whitespace-separated string"))?;
        for selector in classes.split_whitespace() {
            assignment.style_ids.push(
                selector
                    .parse()
                    .map_err(|error| source_error(path, error))?,
            );
        }
    }
    if !assignment.style_ids.is_empty() && !has_authored_assignment {
        assignment.binding_overrides.extend(binding_map.clone());
    }
    if !assignment.style_ids.is_empty() && !has_authored_assignment {
        for (name, value) in object.iter() {
            if let Ok(property) = StyleProperty::parse(name) {
                assignment
                    .local_patch
                    .entry(property)
                    .or_insert_with(|| StyleDeclaration {
                        value: value.clone(),
                        binding: property
                            .binding()
                            .and_then(|bound| binding_map.get(&bound).copied()),
                    });
            }
        }
    }
    if let Some(overrides) = object.remove("style_overrides") {
        for (name, value) in overrides
            .as_object()
            .ok_or_else(|| source_error(path, "style_overrides must be an object"))?
        {
            let property = StyleProperty::parse(name).map_err(|error| source_error(path, error))?;
            if let Some(bound) = property.binding() {
                assignment.binding_overrides.remove(&bound);
            }
            assignment.local_patch.insert(
                property,
                StyleDeclaration {
                    value: value.clone(),
                    binding: None,
                },
            );
        }
    }
    if !assignment.is_empty() {
        hydrate_geometry(value);
        styles
            .compile_node_value(value, &assignment)
            .map_err(|error| source_error(path, error))?;
    }
    Ok(())
}

fn hydrate_geometry(value: &mut Value) {
    let Some(object) = value.as_object_mut() else {
        return;
    };
    let kind = object
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if !matches!(
        kind,
        "text"
            | "bitmap"
            | "video"
            | "audio"
            | "node_graph"
            | "model3d"
            | "ai_artifact"
            | "embed"
            | "instance"
    ) || object.contains_key("local_size")
    {
        return;
    }
    let size = object.get("natural_size").cloned().unwrap_or_else(|| {
        if kind == "text" {
            json!([0.0, 0.0])
        } else {
            json!([200.0, 100.0])
        }
    });
    object.insert("local_size".into(), size);
}

fn validate_document(
    document: &Doc,
    resources: &BTreeMap<AssetId, Vec<u8>>,
) -> Result<(), RuntimeError> {
    if document
        .flow_start
        .is_some_and(|id| !document.scene.contains(id))
        || document
            .flows
            .iter()
            .any(|flow| !document.scene.contains(flow.start))
    {
        return Err(source_error(
            "flows",
            "prototype flow references a missing starting frame",
        ));
    }
    for (id, rule) in &document.styles.rules {
        if *id != rule.id {
            return Err(source_error(
                "styles",
                "style map key disagrees with rule identity",
            ));
        }
        for (property, declaration) in &rule.declarations {
            property
                .validate_literal(&declaration.value)
                .map_err(|error| source_error("styles", error))?;
            if let Some(variable) = declaration.binding {
                let property = property.binding().ok_or_else(|| {
                    source_error("styles", "style property cannot be variable-bound")
                })?;
                let variable =
                    document.variables.variables.get(&variable).ok_or_else(|| {
                        source_error("styles", "style binding variable is missing")
                    })?;
                if !property.accepts_variable_type(variable.ty) {
                    return Err(source_error(
                        "styles",
                        "style binding variable has an incompatible type",
                    ));
                }
            }
        }
    }
    for (id, definition) in &document.components.defs {
        if *id != definition.id || !document.scene.contains(definition.root) {
            return Err(source_error(
                "components",
                "component identity or master root is invalid",
            ));
        }
        if document.components.sets.contains_key(id) {
            return Err(source_error(
                "components",
                "one identity cannot name both a component definition and a set",
            ));
        }
    }
    for (id, set) in &document.components.sets {
        if *id != set.id
            || !set.members.contains(&set.default_variant)
            || set
                .members
                .iter()
                .any(|member| !document.components.defs.contains_key(member))
        {
            return Err(source_error(
                "components",
                "component set identity or variant membership is invalid",
            ));
        }
    }
    for (id, variable) in &document.variables.variables {
        if *id != variable.id
            || !document
                .variables
                .collections
                .contains_key(&variable.collection)
        {
            return Err(source_error(
                "variables",
                "variable identity or collection is invalid",
            ));
        }
        let collection = document
            .variables
            .collections
            .get(&variable.collection)
            .ok_or_else(|| source_error("variables", "variable collection is missing"))?;
        if variable
            .values_by_mode
            .keys()
            .any(|mode| !collection.has_mode(*mode))
        {
            return Err(source_error(
                "variables",
                "variable value references an undeclared mode",
            ));
        }
        for value in variable.values_by_mode.values() {
            if let fanta_doc::VarValue::Alias { variable: target } = value {
                let target =
                    document.variables.variables.get(target).ok_or_else(|| {
                        source_error("variables", "variable alias target is missing")
                    })?;
                if target.ty != variable.ty {
                    return Err(source_error(
                        "variables",
                        "variable alias target has a different type",
                    ));
                }
            } else if value.variable_type() != Some(variable.ty) {
                return Err(source_error(
                    "variables",
                    "variable value does not match its declared type",
                ));
            }
        }
    }
    for (id, collection) in &document.variables.collections {
        let mut modes = BTreeSet::new();
        if *id != collection.id
            || collection.modes.iter().any(|mode| !modes.insert(mode.id))
            || (!collection.modes.is_empty() && !modes.contains(&collection.default_mode))
        {
            return Err(source_error(
                "variables",
                "collection identity, modes, or default mode is invalid",
            ));
        }
    }
    validate_mode_pins(document, &document.active_modes)?;
    for id in document
        .scene
        .roots()
        .iter()
        .flat_map(|root| document.scene.descendants_of(*root))
    {
        let node = document
            .scene
            .get(id)
            .ok_or_else(|| source_error("scene", "missing node"))?;
        if let fanta_doc::NodeData::Group(group) = &node.data {
            validate_mode_pins(document, &group.explicit_modes)?;
        }
        if let fanta_doc::NodeData::Instance(instance) = &node.data {
            if !document.components.defs.contains_key(&instance.component)
                && !document.components.sets.contains_key(&instance.component)
            {
                return Err(source_error(
                    "scene",
                    format!("missing component {}", instance.component),
                ));
            }
        }
        for (property, variable) in &node.bindings {
            let variable = document
                .variables
                .variables
                .get(variable)
                .ok_or_else(|| source_error("scene", format!("missing variable {variable}")))?;
            if !property.applies_to(node) || !property.accepts_variable_type(variable.ty) {
                return Err(source_error(
                    "scene",
                    format!("incompatible binding {property:?} on {}", node.id),
                ));
            }
        }
        let mut value = serde_json::to_value(node)?;
        if let Some(object) = value.as_object_mut() {
            object.remove("meta");
        }
        validate_asset_references(&value, resources)?;
    }
    for (id, clip) in &document.motion.clips {
        if *id != clip.id {
            return Err(source_error(
                "motion",
                "clip map key disagrees with clip identity",
            ));
        }
        let mut targets = BTreeSet::new();
        for (track_id, track) in &clip.tracks {
            if *track_id != track.id || !document.scene.contains(track.target.node) {
                return Err(source_error(
                    "motion",
                    "track identity or target is invalid",
                ));
            }
            if !targets.insert(track.target) {
                return Err(source_error(
                    "motion",
                    "clip has duplicate property targets",
                ));
            }
            for (keyframe_id, keyframe) in &track.keyframes {
                if *keyframe_id != keyframe.id
                    || keyframe.time_ms > clip.duration_ms
                    || !track.target.property.accepts_value(&keyframe.value)
                {
                    return Err(source_error(
                        "motion",
                        "invalid keyframe identity, time, or value type",
                    ));
                }
            }
        }
    }
    Ok(())
}

fn validate_mode_pins(
    document: &Doc,
    pins: &BTreeMap<fanta_doc::VariableCollectionId, fanta_doc::ModeId>,
) -> Result<(), RuntimeError> {
    for (collection, mode) in pins {
        if document
            .variables
            .collections
            .get(collection)
            .is_none_or(|collection| !collection.has_mode(*mode))
        {
            return Err(source_error(
                "variables",
                "mode selection references an undeclared collection or mode",
            ));
        }
    }
    Ok(())
}

fn contains_video_paint(value: &Value) -> bool {
    match value {
        Value::Object(object) => {
            object.get("kind").and_then(Value::as_str) == Some("video")
                || object.values().any(contains_video_paint)
        }
        Value::Array(values) => values.iter().any(contains_video_paint),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fanta_doc::{CanvasNode, Color, GroupNode, NodeData, TextNode};

    fn sources() -> SourceBundle {
        SourceBundle { files: BTreeMap::from([
            ("fanta.json".into(), br#"{"format":"fanta-project","version":6,"schema_version":4,"project_id":"d_00000000000000000000000001"}"#.to_vec()),
            ("doc/metadata.json".into(), br#"{"title":"Test","created_at":0,"modified_at":0}"#.to_vec()),
            ("pages/home/page.json".into(), br#"{"id":"n_00000000000000000000000002","order":0}"#.to_vec()),
            ("pages/home/page.fnx".into(), br#"import "../../styles/project.fss";
<Frame id="00000000000000000000000002" name="Home">
  <Frame id="00000000000000000000000003" name="Card" class="s_00000000000000000000000004" width={100} height={50} />
</Frame>"#.to_vec()),
            ("styles/project.fss".into(), b".s_00000000000000000000000004 { fill: #AABBCC; corner-radius: 12; layout: vertical; gap: 4; padding: 8; }".to_vec()),
        ]), fonts: Vec::new(), font_substitutions: BTreeMap::new() }
    }

    #[test]
    fn source_bundle_round_trip_preserves_style_identity_and_is_deterministic() {
        let source = sources();
        let program = RuntimeProgram::compile(&source).expect("compile source");
        let card = program
            .document()
            .scene
            .get(NodeId::from_u128(3))
            .expect("card");
        assert_eq!(card.style_assignment.style_ids, vec![StyleId::from_u128(4)]);
        let NodeData::Group(group) = &card.data else {
            panic!("expected frame");
        };
        assert_eq!(
            group
                .background
                .as_ref()
                .and_then(fanta_doc::Fill::solid_color),
            Some(Color::rgb(0xAA, 0xBB, 0xCC))
        );
        assert_eq!(
            group.auto_layout.as_ref().expect("layout").padding,
            [8.0; 4]
        );
        let archive = program.to_bundle().expect("bundle");
        assert_eq!(archive, program.to_bundle().expect("repeat bundle"));
        let loaded = RuntimeProgram::load_bundle(&archive).expect("load bundle");
        assert_eq!(loaded.to_bundle().expect("rebundle"), archive);
        assert_eq!(
            loaded
                .document()
                .scene
                .get(card.id)
                .expect("loaded card")
                .style_assignment,
            card.style_assignment
        );
    }

    #[test]
    fn missing_identity_is_an_error_without_allocation() {
        let mut source = sources();
        source.files.insert(
            "pages/home/page.fnx".into(),
            b"<Frame name=\"Home\"/>".to_vec(),
        );
        let error = RuntimeProgram::compile(&source).expect_err("missing identity");
        assert!(error.to_string().contains("authoritative id"));
    }

    #[test]
    fn malformed_motion_is_never_replaced_with_empty_library() {
        let mut source = sources();
        source.files.insert(
            "doc/motion.json".into(),
            br#"{"clips":{"00000000000000000000000001":{"id":7}}}"#.to_vec(),
        );
        assert!(RuntimeProgram::compile(&source).is_err());
    }

    #[test]
    fn missing_imports_and_escaping_paths_are_errors() {
        let mut source = sources();
        source.files.remove("styles/project.fss");
        assert!(
            RuntimeProgram::compile(&source)
                .expect_err("missing import")
                .to_string()
                .contains("styles/project.fss")
        );
        source.files.insert("../outside.fss".into(), Vec::new());
        assert!(RuntimeProgram::compile(&source).is_err());
    }

    #[test]
    fn conflicting_style_owners_and_duplicate_binding_addresses_are_rejected() {
        let mut source = sources();
        let text = String::from_utf8(
            source
                .files
                .get("pages/home/page.fnx")
                .expect("FNX")
                .clone(),
        )
        .expect("source text");
        source.files.insert(
            "pages/home/page.fnx".into(),
            text.replace(
                "width={100}",
                "style_assignment={{\"style_ids\":[]}} width={100}",
            )
            .into_bytes(),
        );
        assert!(
            RuntimeProgram::compile(&source)
                .expect_err("conflicting style ownership")
                .to_string()
                .contains("cannot both own")
        );
        let mut source = sources();
        let text = String::from_utf8(
            source
                .files
                .get("pages/home/page.fnx")
                .expect("FNX")
                .clone(),
        )
        .expect("source text");
        source.files.insert("pages/home/page.fnx".into(), text.replace("width={100}", "bindings={[[{\"prop\":\"opacity\"},\"00000000000000000000000005\"],[{\"prop\":\"opacity\"},\"00000000000000000000000005\"]]} width={100}").into_bytes());
        assert!(
            RuntimeProgram::compile(&source)
                .expect_err("duplicate bindings")
                .to_string()
                .contains("duplicate binding property")
        );
    }

    #[test]
    fn node_adapters_reject_properties_that_the_payload_does_not_render() {
        let registry = StyleRegistry {
            rules: BTreeMap::from([(
                StyleId::from_u128(1),
                StyleRule {
                    id: StyleId::from_u128(1),
                    declarations: BTreeMap::from([(
                        StyleProperty::Align,
                        StyleDeclaration {
                            value: json!("center"),
                            binding: None,
                        },
                    )]),
                },
            )]),
        };
        let node = fanta_doc::CanvasNode::new(fanta_doc::NodeData::TextPath(
            fanta_doc::TextPathNode::new(fanta_doc::PathData::rect(0.0, 0.0, 100.0, 20.0), "Label"),
        ));
        let mut value = serde_json::to_value(node).expect("text path");
        assert!(
            registry
                .compile_node_value(
                    &mut value,
                    &StyleAssignment {
                        style_ids: vec![StyleId::from_u128(1)],
                        ..StyleAssignment::default()
                    }
                )
                .expect_err("text box alignment on path")
                .to_string()
                .contains("incompatible")
        );
        let registry = StyleRegistry {
            rules: BTreeMap::from([(
                StyleId::from_u128(1),
                StyleRule {
                    id: StyleId::from_u128(1),
                    declarations: BTreeMap::from([(
                        StyleProperty::Fills,
                        StyleDeclaration {
                            value: json!([]),
                            binding: None,
                        },
                    )]),
                },
            )]),
        };
        let mut value = json!({"id":"00000000000000000000000002","type":"bitmap","parent":null,"index":1,"name":"Image","asset":"00000000000000000000000003","natural_size":[2,2],"local_size":[20,20]});
        assert!(
            registry
                .compile_node_value(
                    &mut value,
                    &StyleAssignment {
                        style_ids: vec![StyleId::from_u128(1)],
                        ..StyleAssignment::default()
                    }
                )
                .expect_err("vector paints on bitmap")
                .to_string()
                .contains("incompatible")
        );
    }

    #[test]
    fn canonical_id_aliases_are_rejected_before_typed_maps_can_overwrite_them() {
        let mut document = Doc::new();
        let mut node = CanvasNode::new(NodeData::Group(GroupNode::default()));
        node.id = NodeId::from_u128(42);
        document.scene.insert(node).expect("node");
        let mut value = document.to_authored_json_value().expect("document");
        let nodes = value
            .get_mut("scene")
            .and_then(|scene| scene.get_mut("nodes"))
            .and_then(Value::as_object_mut)
            .expect("nodes");
        let uppercase = NodeId::from_u128(42).0.to_string();
        let mut alias = nodes.get(&uppercase).expect("original node").clone();
        alias
            .as_object_mut()
            .expect("node object")
            .insert("id".into(), Value::String(uppercase.to_ascii_lowercase()));
        nodes.insert(uppercase.to_ascii_lowercase(), alias);
        let source = SourceBundle {
            files: BTreeMap::from([("doc.json".into(), serde_json::to_vec(&value).expect("JSON"))]),
            ..SourceBundle::default()
        };
        assert!(
            RuntimeProgram::compile(&source)
                .expect_err("duplicate canonical ID")
                .to_string()
                .contains("duplicate canonical identity")
        );
        let mut value = document.to_authored_json_value().expect("document");
        let nodes = value
            .get_mut("scene")
            .and_then(|scene| scene.get_mut("nodes"))
            .and_then(Value::as_object_mut)
            .expect("nodes");
        let node = nodes.remove(&uppercase).expect("original node");
        nodes.insert(NodeId::from_u128(43).0.to_string(), node);
        assert!(
            Doc::from_json_str(&value.to_string())
                .expect_err("identity mismatch")
                .to_string()
                .contains("map key disagrees")
        );
    }

    #[test]
    fn redundant_root_documents_and_conflicting_bundle_metadata_are_rejected() {
        fn rewrite_manifest(bytes: &[u8], edit: impl FnOnce(&mut Value)) -> Vec<u8> {
            let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).expect("archive");
            let mut files = BTreeMap::new();
            for index in 0..archive.len() {
                let mut entry = archive.by_index(index).expect("entry");
                let mut bytes = Vec::new();
                entry.read_to_end(&mut bytes).expect("entry bytes");
                files.insert(entry.name().to_owned(), bytes);
            }
            let mut manifest: Value =
                serde_json::from_slice(files.get("manifest.json").expect("manifest"))
                    .expect("manifest JSON");
            edit(&mut manifest);
            files.insert(
                "manifest.json".into(),
                serde_json::to_vec(&manifest).expect("edited manifest"),
            );
            let mut archive = zip::ZipWriter::new(Cursor::new(Vec::new()));
            for (path, bytes) in files {
                archive
                    .start_file(path, SimpleFileOptions::default())
                    .expect("start file");
                archive.write_all(&bytes).expect("write bytes");
            }
            archive.finish().expect("finish archive").into_inner()
        }
        let mut source = sources();
        source.files.insert(
            "doc.json".into(),
            serde_json::to_vec(&Doc::new()).expect("document"),
        );
        assert!(
            RuntimeProgram::compile(&source)
                .expect_err("two roots")
                .to_string()
                .contains("both a canonical document")
        );
        let program = RuntimeProgram::compile(&sources()).expect("program");
        let conflicting = rewrite_manifest(&program.to_bundle().expect("bundle"), |manifest| {
            manifest["schema_version"] = json!(3);
        });
        assert!(
            RuntimeProgram::load_bundle(&conflicting)
                .expect_err("schema mismatch")
                .to_string()
                .contains("manifest schema disagrees")
        );
        let program = RuntimeProgram::from_document(
            Doc::new(),
            BTreeMap::from([(AssetId::from_u128(42), vec![1, 2, 3])]),
            Vec::new(),
            BTreeMap::new(),
        )
        .expect("asset program");
        let conflicting = rewrite_manifest(&program.to_bundle().expect("bundle"), |manifest| {
            let assets = manifest
                .get_mut("assets")
                .and_then(Value::as_object_mut)
                .expect("assets");
            let identity = AssetId::from_u128(42).0.to_string();
            let value = assets.get(&identity).expect("asset").clone();
            assets.insert(identity.to_ascii_lowercase(), value);
        });
        assert!(
            RuntimeProgram::load_bundle(&conflicting)
                .expect_err("aliased asset ID")
                .to_string()
                .contains("duplicate canonical asset identity")
        );
    }

    #[test]
    fn omitted_source_metadata_has_a_deterministic_default() {
        let source = SourceBundle {
            files: BTreeMap::from([(
                "doc.json".into(),
                br#"{"id":"00000000000000000000000001","schema_version":4,"scene":{"nodes":{}}}"#
                    .to_vec(),
            )]),
            ..SourceBundle::default()
        };
        let first = RuntimeProgram::compile(&source).expect("first compilation");
        let second = RuntimeProgram::compile(&source).expect("second compilation");
        assert_eq!(first.document().metadata.created_at, 0);
        assert_eq!(first.document().metadata.modified_at, 0);
        assert_eq!(
            first.to_bundle().expect("first bundle"),
            second.to_bundle().expect("second bundle")
        );
    }

    #[test]
    fn configured_bundle_limit_checks_archive_and_expanded_size_before_loading() {
        let program = RuntimeProgram::compile(&sources()).expect("program");
        let bytes = program.to_bundle().expect("bundle");
        assert!(
            RuntimeProgram::load_bundle_with_limit(&bytes, 1)
                .expect_err("archive limit")
                .to_string()
                .contains("size limit")
        );
        let archive = zip::ZipArchive::new(Cursor::new(&bytes)).expect("archive");
        let expanded_size = archive.decompressed_size().expect("decompressed size") as u64;
        assert!(expanded_size > bytes.len() as u64);
        assert!(
            RuntimeProgram::load_bundle_with_limit(&bytes, expanded_size - 1)
                .expect_err("expanded limit")
                .to_string()
                .contains("expanded archive")
        );
        RuntimeProgram::load_bundle_with_limit(&bytes, expanded_size)
            .expect("exact expanded limit");
    }

    #[test]
    fn duplicate_json_identity_and_conflicting_style_files_are_rejected() {
        let mut source = sources();
        source.files.insert(
            "doc/metadata.json".into(),
            br#"{"title":"A","title":"B","created_at":0,"modified_at":0}"#.to_vec(),
        );
        assert!(
            RuntimeProgram::compile(&source)
                .expect_err("duplicate JSON keys")
                .to_string()
                .contains("duplicate JSON key")
        );
        let mut source = sources();
        let stylesheet = source
            .files
            .get("styles/project.fss")
            .expect("style bytes")
            .clone();
        source.files.insert("styles/other.fss".into(), stylesheet);
        source.files.insert(
            "pages/home/page.fnx".into(),
            br#"import "../../styles/project.fss";
import "../../styles/other.fss";
<Frame id="00000000000000000000000002" name="Home"/>"#
                .to_vec(),
        );
        assert!(
            RuntimeProgram::compile(&source)
                .expect_err("duplicate style identity")
                .to_string()
                .contains("duplicate style identity")
        );
    }

    #[test]
    fn authored_style_projection_omits_inherited_cache_and_retains_same_value_claim() {
        let mut source = sources();
        let old = source.files.get("pages/home/page.fnx").expect("FNX");
        let source_text = String::from_utf8(old.clone()).expect("source text");
        source.files.insert(
            "pages/home/page.fnx".into(),
            source_text
                .replace("width={100}", "corner_radius={12} width={100}")
                .into_bytes(),
        );
        let program = RuntimeProgram::compile(&source).expect("program");
        let authored = program
            .document()
            .to_authored_json_value()
            .expect("authored");
        let card = &authored["scene"]["nodes"]["00000000000000000000000003"];
        assert!(card.get("background").is_none());
        assert!(card.get("corner_radius").is_none());
        assert_eq!(
            card["style_assignment"]["local_patch"]["corner_radius"]["value"],
            12
        );
        let loaded =
            RuntimeProgram::load_bundle(&program.to_bundle().expect("bundle")).expect("load");
        assert_eq!(
            loaded
                .document()
                .to_authored_json_value()
                .expect("loaded authored"),
            authored
        );
    }

    #[test]
    fn tampering_resource_bytes_fails_checksum_validation() {
        let program = RuntimeProgram::compile(&sources()).expect("program");
        let original = program.to_bundle().expect("bundle");
        let mut archive = zip::ZipArchive::new(Cursor::new(original)).expect("archive");
        let mut tampered = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for index in 0..archive.len() {
            let mut entry = archive.by_index(index).expect("entry");
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes).expect("entry bytes");
            if entry.name() == "doc.json" {
                bytes.push(b' ');
            }
            tampered
                .start_file(entry.name(), SimpleFileOptions::default())
                .expect("start file");
            tampered.write_all(&bytes).expect("write");
        }
        let bytes = tampered.finish().expect("finish").into_inner();
        assert!(
            RuntimeProgram::load_bundle(&bytes)
                .expect_err("checksum mismatch")
                .to_string()
                .contains("checksum mismatch")
        );
    }

    #[test]
    fn fonts_assets_and_substitutions_survive_bundle_loading() {
        let mut document = Doc::new();
        document.id = fanta_doc::DocId::from_u128(1);
        document.metadata.created_at = 0;
        document.metadata.modified_at = 0;
        let mut node = CanvasNode::new(NodeData::Group(GroupNode::default()));
        node.id = NodeId::from_u128(2);
        document.scene.insert(node).expect("insert");
        let resources = BTreeMap::from([(AssetId::from_u128(5), vec![1, 2, 3])]);
        let fonts = vec![RuntimeFont {
            family: "Test".into(),
            weight: 400,
            italic: false,
            face_index: 0,
            bytes: vec![9, 8, 7],
        }];
        let program = RuntimeProgram::from_document(
            document,
            resources.clone(),
            fonts.clone(),
            BTreeMap::from([("Inter".into(), "Test".into())]),
        )
        .expect("program");
        let loaded =
            RuntimeProgram::load_bundle(&program.to_bundle().expect("bundle")).expect("load");
        assert_eq!(loaded.resources(), &resources);
        assert_eq!(loaded.fonts(), &fonts);
        assert_eq!(
            loaded.font_substitutions().get("Inter").map(String::as_str),
            Some("Test")
        );
    }

    #[test]
    fn every_program_path_compiles_motion_once_and_shares_it_between_clones() {
        use fanta_doc::{
            AnimationClip, AnimationClipId, AnimationTrack, AnimationTrackId, Keyframe, KeyframeId,
            MotionProperty, MotionTarget, ResolvedVarValue,
        };
        let mut document = Doc::new();
        document.id = fanta_doc::DocId::from_u128(1);
        document.metadata.created_at = 0;
        document.metadata.modified_at = 0;
        let mut node = CanvasNode::new(NodeData::Group(GroupNode::default()));
        node.id = NodeId::from_u128(2);
        document.scene.insert(node).expect("insert");
        let clip_id = AnimationClipId::from_u128(3);
        let mut clip = AnimationClip::new(clip_id, "Slide", 100);
        let mut track = AnimationTrack::new(
            AnimationTrackId::from_u128(4),
            MotionTarget::new(NodeId::from_u128(2), MotionProperty::PositionX),
        );
        for (identity, time_ms, value) in [(5, 0, 0.0), (6, 100, 40.0)] {
            let keyframe = Keyframe::new(
                KeyframeId::from_u128(identity),
                time_ms,
                ResolvedVarValue::Float { value },
            );
            track.keyframes.insert(keyframe.id, keyframe);
        }
        clip.tracks.insert(track.id, track);
        document.motion.clips.insert(clip_id, clip);

        let from_document =
            RuntimeProgram::from_document(document, BTreeMap::new(), Vec::new(), BTreeMap::new())
                .expect("program");
        let reloaded =
            RuntimeProgram::load_bundle(&from_document.to_bundle().expect("bundle")).expect("load");
        let compiled = RuntimeProgram::compile(&SourceBundle {
            files: BTreeMap::from([(
                "doc.json".to_owned(),
                from_document
                    .document()
                    .to_json_string()
                    .expect("document json")
                    .into_bytes(),
            )]),
            ..SourceBundle::default()
        })
        .expect("compile source");
        for program in [&from_document, &reloaded, &compiled] {
            for playhead_ms in [0, 50, 100] {
                let sample = program.compiled_motion().evaluate(clip_id, playhead_ms);
                assert!(sample.is_some(), "clip samples at {playhead_ms}ms");
                assert_eq!(
                    sample,
                    program.document().motion.evaluate(clip_id, playhead_ms)
                );
            }
            assert!(Rc::ptr_eq(
                program.clone().compiled_motion(),
                program.compiled_motion()
            ));
        }
        assert!(!Rc::ptr_eq(
            from_document.compiled_motion(),
            reloaded.compiled_motion()
        ));
    }

    #[test]
    fn composite_token_does_not_erase_local_typography_leaf() {
        let registry = StyleRegistry {
            rules: BTreeMap::from([(
                StyleId::from_u128(1),
                StyleRule {
                    id: StyleId::from_u128(1),
                    declarations: BTreeMap::from([(
                        StyleProperty::Typography,
                        StyleDeclaration {
                            value: json!({"font_family":"Inter","size_px":16}),
                            binding: Some(VariableId::from_u128(9)),
                        },
                    )]),
                },
            )]),
        };
        let mut node = CanvasNode::new(NodeData::Text(TextNode::new("Hi", 100.0, 50.0)));
        node.style_assignment = StyleAssignment {
            style_ids: vec![StyleId::from_u128(1)],
            local_patch: BTreeMap::from([(
                StyleProperty::FontSize,
                StyleDeclaration {
                    value: Value::from(24),
                    binding: None,
                },
            )]),
            ..StyleAssignment::default()
        };
        if let NodeData::Text(text) = &mut node.data {
            text.style.font_family = "Mode B".into();
            text.style.size_px = 32.0;
        }
        let resolved = registry
            .apply_local_patch_after_bindings(&node)
            .expect("claim overlay");
        let NodeData::Text(text) = resolved.data else {
            panic!("text");
        };
        assert_eq!(text.style.font_family, "Mode B");
        assert_eq!(text.style.size_px, 24.0);
    }

    #[test]
    fn style_binding_and_equal_local_override_keep_distinct_ownership() {
        use fanta_doc::{
            Mode, ModeId, VarValue, Variable, VariableCollection, VariableCollectionId,
            VariableRegistry, VariableType,
        };
        let collection = VariableCollectionId::from_u128(10);
        let mode = ModeId::from_u128(11);
        let variable = VariableId::from_u128(12);
        let registry = VariableRegistry {
            collections: BTreeMap::from([(
                collection,
                VariableCollection {
                    id: collection,
                    name: "Theme".into(),
                    modes: vec![Mode {
                        id: mode,
                        name: "Default".into(),
                    }],
                    default_mode: mode,
                    variable_order: Vec::new(),
                },
            )]),
            variables: BTreeMap::from([(
                variable,
                Variable {
                    id: variable,
                    collection,
                    name: "opacity".into(),
                    ty: VariableType::Float,
                    values_by_mode: BTreeMap::from([(mode, VarValue::Float { value: 0.75 })]),
                    scopes: Vec::new(),
                    options: Vec::new(),
                },
            )]),
        };
        let mut source = sources();
        source.files.insert(
            "doc/variables.json".into(),
            serde_json::to_vec(&registry).expect("variable source"),
        );
        source.files.insert(
            "styles/project.fss".into(),
            b".s_00000000000000000000000004 { opacity: bind(\"$Theme/opacity\", 0.5); }".to_vec(),
        );
        let program = RuntimeProgram::compile(&source).expect("bound source");
        let node = program
            .document()
            .scene
            .get(NodeId::from_u128(3))
            .expect("card");
        assert_eq!(
            node.bindings.get(&fanta_doc::BoundProp::Opacity),
            Some(&variable)
        );
        let loaded =
            RuntimeProgram::load_bundle(&program.to_bundle().expect("bundle")).expect("load");
        assert_eq!(
            loaded
                .document()
                .scene
                .get(node.id)
                .expect("loaded card")
                .bindings,
            node.bindings
        );
        let source_text = String::from_utf8(
            source
                .files
                .get("pages/home/page.fnx")
                .expect("FNX")
                .clone(),
        )
        .expect("UTF8");
        source.files.insert(
            "pages/home/page.fnx".into(),
            source_text
                .replace("width={100}", "opacity={0.5} width={100}")
                .into_bytes(),
        );
        let overridden = RuntimeProgram::compile(&source).expect("equal override");
        let node = overridden
            .document()
            .scene
            .get(NodeId::from_u128(3))
            .expect("card");
        assert!(node.bindings.is_empty());
        assert_eq!(
            node.style_assignment
                .local_patch
                .get(&StyleProperty::Opacity)
                .expect("local claim")
                .value,
            0.5
        );
        let mut source = sources();
        source.files.insert(
            "doc/variables.json".into(),
            serde_json::to_vec(&registry).expect("variables"),
        );
        source.files.insert(
            "styles/project.fss".into(),
            b".s_00000000000000000000000004 { opacity: bind(\"$Theme/opacity\", 0.5); }".to_vec(),
        );
        let text = String::from_utf8(
            source
                .files
                .get("pages/home/page.fnx")
                .expect("FNX")
                .clone(),
        )
        .expect("source text");
        source.files.insert("pages/home/page.fnx".into(), text.replace("width={100}", "bindings={{\"opacity\":\"$Theme/opacity\"}} style_overrides={{\"opacity\":0.2}} width={100}").into_bytes());
        let overridden = RuntimeProgram::compile(&source).expect("override explicit binding");
        let node = overridden
            .document()
            .scene
            .get(NodeId::from_u128(3))
            .expect("card");
        assert!(node.bindings.is_empty());
        assert!(node.style_assignment.binding_overrides.is_empty());
        assert_eq!(node.opacity.get(), 0.2);
    }

    #[cfg(feature = "native")]
    #[test]
    fn desktop_project_projection_and_portable_compiler_agree() {
        fn collect(
            directory: &std::path::Path,
            root: &std::path::Path,
            files: &mut BTreeMap<String, Vec<u8>>,
        ) -> std::io::Result<()> {
            for entry in std::fs::read_dir(directory)? {
                let path = entry?.path();
                if path.is_dir() {
                    collect(&path, root, files)?;
                } else {
                    let relative = path.strip_prefix(root).map_err(std::io::Error::other)?;
                    files.insert(
                        relative.to_string_lossy().replace('\\', "/"),
                        std::fs::read(path)?,
                    );
                }
            }
            Ok(())
        }
        let program = RuntimeProgram::compile(&sources()).expect("source program");
        let directory = tempfile::tempdir().expect("project directory");
        crate::write_project_tree(directory.path(), program.document(), program.resources())
            .expect("desktop projection");
        let (desktop, _) = crate::read_project_tree(directory.path()).expect("desktop reload");
        let mut source = SourceBundle::default();
        collect(directory.path(), directory.path(), &mut source.files).expect("source bytes");
        let portable = RuntimeProgram::compile(&source).expect("portable compiler");
        assert_eq!(
            portable
                .document()
                .to_authored_json_value()
                .expect("portable JSON"),
            desktop.to_authored_json_value().expect("desktop JSON")
        );
        assert_eq!(portable.document().styles, program.document().styles);
        assert_eq!(
            portable
                .document()
                .scene
                .get(NodeId::from_u128(3))
                .expect("card")
                .style_assignment,
            program
                .document()
                .scene
                .get(NodeId::from_u128(3))
                .expect("original card")
                .style_assignment
        );
    }
}

fn validate_asset_references(
    value: &Value,
    resources: &BTreeMap<AssetId, Vec<u8>>,
) -> Result<(), RuntimeError> {
    match value {
        Value::Object(object) => {
            for (key, value) in object {
                if matches!(key.as_str(), "asset" | "poster" | "preview" | "output")
                    && value.is_string()
                {
                    let asset: AssetId = serde_json::from_value(value.clone())
                        .map_err(|error| source_error("assets", error))?;
                    if !resources.contains_key(&asset) {
                        return Err(source_error(
                            "assets",
                            format!("required asset {asset} is missing"),
                        ));
                    }
                } else {
                    validate_asset_references(value, resources)?;
                }
            }
        }
        Value::Array(values) => {
            for value in values {
                validate_asset_references(value, resources)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn strict_json(bytes: &[u8]) -> Result<Value, serde_json::Error> {
    struct StrictValue(Value);
    impl<'de> Deserialize<'de> for StrictValue {
        fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
            struct Visitor;
            impl<'de> serde::de::Visitor<'de> for Visitor {
                type Value = StrictValue;
                fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                    formatter.write_str("JSON with unique object keys")
                }
                fn visit_bool<E: serde::de::Error>(self, value: bool) -> Result<Self::Value, E> {
                    Ok(StrictValue(Value::Bool(value)))
                }
                fn visit_i64<E: serde::de::Error>(self, value: i64) -> Result<Self::Value, E> {
                    Ok(StrictValue(Value::from(value)))
                }
                fn visit_u64<E: serde::de::Error>(self, value: u64) -> Result<Self::Value, E> {
                    Ok(StrictValue(Value::from(value)))
                }
                fn visit_f64<E: serde::de::Error>(self, value: f64) -> Result<Self::Value, E> {
                    serde_json::Number::from_f64(value)
                        .map(|number| StrictValue(Value::Number(number)))
                        .ok_or_else(|| E::custom("non-finite number"))
                }
                fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<Self::Value, E> {
                    Ok(StrictValue(Value::String(value.into())))
                }
                fn visit_string<E: serde::de::Error>(
                    self,
                    value: String,
                ) -> Result<Self::Value, E> {
                    Ok(StrictValue(Value::String(value)))
                }
                fn visit_unit<E: serde::de::Error>(self) -> Result<Self::Value, E> {
                    Ok(StrictValue(Value::Null))
                }
                fn visit_none<E: serde::de::Error>(self) -> Result<Self::Value, E> {
                    Ok(StrictValue(Value::Null))
                }
                fn visit_seq<A: serde::de::SeqAccess<'de>>(
                    self,
                    mut sequence: A,
                ) -> Result<Self::Value, A::Error> {
                    let mut values = Vec::new();
                    while let Some(value) = sequence.next_element::<StrictValue>()? {
                        values.push(value.0);
                    }
                    Ok(StrictValue(Value::Array(values)))
                }
                fn visit_map<A: serde::de::MapAccess<'de>>(
                    self,
                    mut map: A,
                ) -> Result<Self::Value, A::Error> {
                    use serde::de::Error as _;
                    let mut values = Map::new();
                    while let Some((key, value)) = map.next_entry::<String, StrictValue>()? {
                        if values.insert(key.clone(), value.0).is_some() {
                            return Err(A::Error::custom(format!("duplicate JSON key {key}")));
                        }
                    }
                    Ok(StrictValue(Value::Object(values)))
                }
            }
            deserializer.deserialize_any(Visitor)
        }
    }
    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    let value = StrictValue::deserialize(&mut deserializer)?.0;
    deserializer.end()?;
    Ok(value)
}
