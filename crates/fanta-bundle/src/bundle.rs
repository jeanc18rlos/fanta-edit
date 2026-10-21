//! Runtime bundles: the program format `fanta-runtime` loads.
//!
//! A bundle is a zip of the canonical document, its binary resources and the
//! fonts the player needs, with a manifest that pins every entry by SHA-256.
//! This crate reads and writes bundles and validates programs. It has no
//! source parser on purpose: the WebAssembly player depends on it, and
//! compiling FNX/FSS sources into a bundle is `fanta-format`'s job, shipped
//! through the `fanta-compile` CLI.

#![forbid(unsafe_code)]

use fanta_doc::motion::CompiledMotionLibrary;
use fanta_doc::{AssetId, Doc, NodeId};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::io::{Cursor, Read, Write};
use std::rc::Rc;
use zip::write::SimpleFileOptions;

pub const RUNTIME_BUNDLE_VERSION: u32 = 1;
const MAX_BUNDLE_BYTES: u64 = 512 * 1024 * 1024;

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

pub fn checksum(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn canonical_json(value: &Value) -> Result<Vec<u8>, RuntimeError> {
    let mut value = value.clone();
    value.sort_all_objects();
    Ok(serde_json::to_vec(&value)?)
}

pub fn source_error(path: &str, message: impl std::fmt::Display) -> RuntimeError {
    RuntimeError::Source {
        path: path.to_owned(),
        message: message.to_string(),
    }
}

pub fn required<'a>(
    files: &'a BTreeMap<String, Vec<u8>>,
    path: &str,
) -> Result<&'a [u8], RuntimeError> {
    files
        .get(path)
        .map(Vec::as_slice)
        .ok_or_else(|| source_error(path, "required resource is missing"))
}

pub fn validate_path(path: &str) -> Result<(), RuntimeError> {
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

pub fn strict_json(bytes: &[u8]) -> Result<Value, serde_json::Error> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use fanta_doc::{CanvasNode, GroupNode, NodeData};

    fn program() -> RuntimeProgram {
        let mut document = Doc::new();
        let mut page = CanvasNode::new(NodeData::Group(GroupNode::default()));
        page.name = "Home".into();
        let page = document.scene.insert(page).expect("insert page");
        document.add_page(page);
        RuntimeProgram::from_document(document, BTreeMap::new(), Vec::new(), BTreeMap::new())
            .expect("program")
    }

    #[test]
    fn bundles_are_deterministic_and_reload_without_a_parser() {
        let program = program();
        let bundle = program.to_bundle().expect("bundle");
        assert_eq!(bundle, program.to_bundle().expect("repeat bundle"));
        let loaded = RuntimeProgram::load_bundle(&bundle).expect("load");
        assert_eq!(loaded.to_bundle().expect("rebundle"), bundle);
        assert_eq!(loaded.document().pages, program.document().pages);
        assert!(loaded.diagnostics().is_empty());
    }

    #[test]
    fn tampered_and_undeclared_entries_are_rejected() {
        let bundle = program().to_bundle().expect("bundle");
        let mut archive = zip::ZipArchive::new(Cursor::new(bundle)).expect("archive");
        let mut tampered = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let options = SimpleFileOptions::default();
        for index in 0..archive.len() {
            let mut entry = archive.by_index(index).expect("entry");
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes).expect("read entry");
            if entry.name() == "doc.json" {
                bytes.extend_from_slice(b" ");
            }
            tampered
                .start_file(entry.name().to_owned(), options)
                .expect("start entry");
            tampered.write_all(&bytes).expect("write entry");
        }
        tampered
            .start_file("extra.bin", options)
            .expect("start extra");
        tampered.write_all(b"undeclared").expect("write extra");
        let bytes = tampered.finish().expect("finish").into_inner();
        let error = RuntimeProgram::load_bundle(&bytes).expect_err("tampered bundle");
        assert!(matches!(error, RuntimeError::Bundle(_)), "{error}");
    }
}
