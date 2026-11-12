//! The project's agent-editable source files: the listing `read_fnx_source`
//! returns and the validation behind `validate_fnx_source`. Shared by the
//! app's live MCP server and the headless stdio server, so both answer
//! identically (spec 0008 R7).

use std::path::Path;

use anyhow::{Context as _, Result, bail};
use fanta_doc::{
    AssetId, CanvasNode, ComponentId, ModeId, NodeId, ProjectAsset, VariableCollectionId,
};
use serde_json::{Value, json};

pub fn validate_source_candidate(
    active_root: Option<&Path>,
    source_path: &Path,
    source: &str,
) -> Result<Value> {
    let path = if source_path.is_absolute() {
        match source_path.canonicalize() {
            Ok(path) => path,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => source_path.to_path_buf(),
            Err(error) => return Err(error).context("resolving the agent source candidate"),
        }
    } else {
        source_path.to_path_buf()
    };
    let mut inferred_root = None;
    if path.is_absolute() {
        for ancestor in path.parent().into_iter().flat_map(Path::ancestors) {
            if ancestor.join("fanta.json").try_exists()? {
                inferred_root = Some(ancestor);
                break;
            }
        }
    }
    let Some(root) = inferred_root.or(active_root) else {
        return Ok(json!({ "applicable": false }));
    };
    validate_agent_source_edit(root, &path, source)
}

pub fn validate_agent_source_edit(
    project_root: &Path,
    source_path: &Path,
    source: &str,
) -> Result<Value> {
    use std::path::Component;
    let canonical_root = project_root
        .canonicalize()
        .with_context(|| format!("resolving Fanta project {}", project_root.display()))?;
    let requested = if source_path.is_absolute() {
        source_path
            .strip_prefix(project_root)
            .map(|relative| canonical_root.join(relative))
            .unwrap_or_else(|_| source_path.to_path_buf())
    } else {
        canonical_root.join(source_path)
    };
    let source_path = match requested.canonicalize() {
        Ok(path) => path,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => requested,
        Err(error) => return Err(error).context("resolving the design source candidate"),
    };
    let Ok(relative) = source_path.strip_prefix(&canonical_root) else {
        return Ok(json!({ "applicable": false }));
    };
    let components = relative.components().collect::<Vec<_>>();
    if validate_managed_json(relative, source)
        .with_context(|| format!("Invalid managed JSON in {}", source_path.display()))?
    {
        let scope = if source_path.is_file() {
            fanta_format::validate_project_json_edit(&canonical_root, &source_path, source)
                .context("the JSON candidate cannot be materialized; the existing source has not been overwritten")?;
            "project"
        } else {
            "typed_json"
        };
        return Ok(
            json!({"applicable":true,"validated":true,"scope":scope,"path":source_path.display().to_string(),"diagnostics":[]}),
        );
    }
    let managed = matches!(components.as_slice(), [
        Component::Normal(directory), Component::Normal(_), Component::Normal(file)
    ] if (*directory == "pages" && *file == "page.fnx")
        || (*directory == "components" && *file == "master.fnx"))
        // A variant's master, inside its component set's folder.
        || matches!(components.as_slice(), [
            Component::Normal(directory), Component::Normal(_), Component::Normal(_),
            Component::Normal(file)
        ] if *directory == "components" && *file == "master.fnx");
    if !managed {
        return Ok(json!({ "applicable": false }));
    }
    fanta_fnx::parse_doc(source).with_context(|| format!(
        "Invalid FNX in {}. FNX attributes require quoted JSON object keys and complete closing tags; repair the candidate before saving",
        source_path.display()
    ))?;
    let (scope, diagnostics) = if source_path.is_file() {
        let (_, diagnostics) = fanta_format::validate_project_source_edit_with_diagnostics(
            &canonical_root, &source_path, source,
        ).context("the FNX candidate cannot be materialized; the existing source has not been overwritten")?;
        ("project", serde_json::to_value(diagnostics)?)
    } else {
        ("syntax", json!([]))
    };
    Ok(
        json!({ "applicable": true, "validated": true, "scope": scope,
        "path": source_path.display().to_string(), "diagnostics": diagnostics }),
    )
}

fn validate_typed_json<T: serde::de::DeserializeOwned>(source: &str) -> Result<()> {
    let value: Value = serde_json::from_str(source)?;
    if !value.is_object() && !value.is_null() {
        bail!("this managed JSON source requires an object");
    }
    serde_json::from_value::<T>(value)?;
    Ok(())
}

fn validate_json_value<T: serde::de::DeserializeOwned>(source: &str) -> Result<()> {
    serde_json::from_str::<T>(source)?;
    Ok(())
}

fn validate_managed_json(relative: &Path, source: &str) -> Result<bool> {
    use std::collections::BTreeMap;
    use std::path::Component;

    let components = relative.components().collect::<Vec<_>>();
    match components.as_slice() {
        [Component::Normal(file)] if *file == "fanta.json" => {
            validate_typed_json::<fanta_format::ProjectManifest>(source)?;
            let manifest: fanta_format::ProjectManifest = serde_json::from_str(source)?;
            if manifest.format != "fanta-project" {
                bail!("the project manifest requires the fanta-project format tag");
            }
            manifest.project_id.parse::<fanta_doc::DocId>()?;
        }
        [Component::Normal(directory), Component::Normal(file)] if *directory == "doc" => {
            match file.to_str() {
                Some("metadata.json") => validate_typed_json::<fanta_doc::DocMetadata>(source)?,
                Some("asset_library.json") => {
                    validate_typed_json::<BTreeMap<AssetId, ProjectAsset>>(source)?
                }
                Some("variables.json") => {
                    validate_typed_json::<fanta_doc::VariableRegistry>(source)?
                }
                Some("active_modes.json") => {
                    validate_typed_json::<BTreeMap<VariableCollectionId, ModeId>>(source)?
                }
                Some("motion.json") => validate_typed_json::<fanta_doc::MotionLibrary>(source)?,
                Some("flow_start.json") => validate_json_value::<Option<NodeId>>(source)?,
                Some("flows.json") => validate_json_value::<Vec<fanta_doc::Flow>>(source)?,
                Some("presentation.json") => {
                    validate_typed_json::<Option<fanta_doc::PresentationConfig>>(source)?
                }
                _ => return Ok(false),
            }
        }
        [Component::Normal(directory), Component::Normal(file)]
            if *directory == "components" && *file == "sets.json" =>
        {
            validate_typed_json::<BTreeMap<ComponentId, fanta_doc::ComponentSet>>(source)?;
        }
        [
            Component::Normal(directory),
            Component::Normal(_),
            Component::Normal(file),
        ] if *directory == "components" && *file == "set.json" => {
            validate_typed_json::<fanta_doc::ComponentSet>(source)?;
        }
        [
            Component::Normal(directory),
            Component::Normal(_),
            Component::Normal(_),
            Component::Normal(file),
        ] if *directory == "components" && *file == "def.json" => {
            validate_typed_json::<fanta_doc::ComponentDef>(source)?;
        }
        [Component::Normal(directory), Component::Normal(file)]
            if *directory == "assets" && *file == "index.json" =>
        {
            let index: Value = serde_json::from_str(source)?;
            if index.get("version").and_then(Value::as_u64) != Some(1) {
                bail!("the asset index requires version 1");
            }
            for (name, record) in index
                .get("assets")
                .and_then(Value::as_object)
                .context("the asset index requires an assets object")?
            {
                name.parse::<AssetId>()?;
                record
                    .get("size")
                    .and_then(Value::as_u64)
                    .context("asset size must be an unsigned integer")?;
                let digest = record
                    .get("sha256")
                    .and_then(Value::as_str)
                    .context("asset SHA-256 must be a string")?;
                if digest.len() != 64
                    || !digest
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
                {
                    bail!("asset {name} requires a lowercase SHA-256 digest");
                }
            }
        }
        [
            Component::Normal(directory),
            Component::Normal(_),
            Component::Normal(file),
        ] if *directory == "pages" && *file == "page.json" => {
            let header: Value = serde_json::from_str(source)?;
            header
                .get("order")
                .and_then(Value::as_u64)
                .context("page order must be an unsigned integer")?;
            if let Some(id) = header.get("id") {
                id.as_str()
                    .context("page identity must be a string")?
                    .parse::<NodeId>()?;
            }
            if let Some(name) = header.get("name") {
                name.as_str().context("page name must be a string")?;
            }
        }
        [
            Component::Normal(directory),
            Component::Normal(_),
            Component::Normal(file),
        ] if *directory == "components" && *file == "def.json" => {
            validate_typed_json::<fanta_doc::ComponentDef>(source)?;
        }
        [
            Component::Normal(directory),
            Component::Normal(_),
            Component::Normal(file),
        ]
        | [
            Component::Normal(directory),
            Component::Normal(_),
            Component::Normal(_),
            Component::Normal(file),
        ] if (*directory == "pages" && *file == "page.ids.json")
            || (*directory == "components" && *file == "master.ids.json") =>
        {
            validate_typed_json::<fanta_fnx::FnxSidecar>(source)?;
            let sidecar: fanta_fnx::FnxSidecar = serde_json::from_str(source)?;
            for entry in sidecar.ids {
                serde_json::from_value::<NodeId>(json!(entry.id))?;
                if !entry.index.is_null() && !entry.index.is_number() {
                    bail!("sidecar sibling order must be a number");
                }
            }
            if let Some(parent) = sidecar.root_parent {
                serde_json::from_value::<NodeId>(json!(parent))?;
            }
        }
        [
            Component::Normal(directory),
            Component::Normal(_),
            Component::Normal(nodes),
            Component::Normal(file),
        ] if (*directory == "pages" || *directory == "components")
            && *nodes == "nodes"
            && Path::new(file)
                .extension()
                .is_some_and(|extension| extension == "json") =>
        {
            validate_typed_json::<CanvasNode>(source)?;
        }
        _ => return Ok(false),
    }
    Ok(true)
}

pub fn list_source_files(root: &Path) -> Vec<String> {
    let mut files = Vec::new();
    for source in [
        "fanta.json",
        "fanta.md",
        "design.css",
        "doc/motion.json",
        "doc/variables.json",
        "doc/active_modes.json",
        "components/sets.json",
    ] {
        if root.join(source).is_file() {
            files.push(source.into());
        }
    }
    // The `.fnx` names cover projects written before the web-file formats.
    for (directory, file_names) in [
        (
            "pages",
            &["page.svg", "page.css", "page.json", "nodes.json", "page.fnx"][..],
        ),
        (
            "components",
            &["master.svg", "master.css", "def.json", "nodes.json", "master.fnx"][..],
        ),
        ("motion", &["motion.json"][..]),
    ] {
        let Ok(entries) = std::fs::read_dir(root.join(directory)) else {
            continue;
        };
        let design_sources = |path: &Path, prefix: &str, sources: &mut Vec<String>| {
            for file_name in file_names {
                if path.join(file_name).is_file() {
                    sources.push(format!("{prefix}/{file_name}"));
                }
            }
        };
        let mut found: Vec<String> = entries
            .flatten()
            .flat_map(|entry| {
                let folder = entry.file_name().to_string_lossy().into_owned();
                let mut sources = Vec::new();
                design_sources(
                    &entry.path(),
                    &format!("{directory}/{folder}"),
                    &mut sources,
                );
                // A component set's folder: its set.json and its variants.
                if directory == "components" && entry.path().join("set.json").is_file() {
                    sources.push(format!("{directory}/{folder}/set.json"));
                    let mut variants = Vec::new();
                    for variant in std::fs::read_dir(entry.path()).into_iter().flatten().flatten()
                    {
                        design_sources(
                            &variant.path(),
                            &format!(
                                "{directory}/{folder}/{}",
                                variant.file_name().to_string_lossy()
                            ),
                            &mut variants,
                        );
                    }
                    variants.sort();
                    sources.append(&mut variants);
                }
                sources
            })
            .collect();
        found.sort();
        files.append(&mut found);
    }
    files
}
