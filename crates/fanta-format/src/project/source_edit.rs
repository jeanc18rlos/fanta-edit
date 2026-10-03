use crate::error::{FormatError, Result};
use fanta_doc::{AssetId, Doc};
use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Component, Path, PathBuf};

use super::read::{
    FnxSourceOverride, JsonSourceOverride, persisted_fnx_sidecar, read_fnx_sidecar,
    read_project_tree_with_json_override, read_project_tree_with_source_override,
};
use super::session::SourceDiagnostic;
use super::session::diagnostics::unknown_attribute_diagnostics;

/// A validated `.fnx` edit and the complete project state it regenerates.
/// Callers can replace their live document only after receiving this value,
/// leaving the last good canvas intact when parsing or semantic validation
/// fails.
#[derive(Debug)]
pub struct ProjectSourceEdit {
    pub document: Doc,
    pub assets: BTreeMap<AssetId, Vec<u8>>,
    pub source_path: PathBuf,
}

/// Parse one edited page/component source in the context of all its unchanged
/// project siblings without touching disk.
pub fn validate_project_source_edit(
    project_root: &Path,
    source_path: &Path,
    source: &str,
) -> Result<ProjectSourceEdit> {
    validate_project_source_edit_with_diagnostics(project_root, source_path, source)
        .map(|(edit, _)| edit)
}

/// [`validate_project_source_edit`] plus the non-fatal authoring diagnostics
/// (e.g. `source.unknown_attribute` typo warnings) for the edited buffer.
/// Diagnostics never fail validation — unknown future fields must keep riding
/// through — callers decide whether a policy promotes them.
pub fn validate_project_source_edit_with_diagnostics(
    project_root: &Path,
    source_path: &Path,
    source: &str,
) -> Result<(ProjectSourceEdit, Vec<SourceDiagnostic>)> {
    let (project_root, source_path) = checked_source_path(project_root, source_path)?;
    let sidecar_path = sidecar_path(&source_path)?;
    let sidecar = read_fnx_sidecar(&source_path, source, &sidecar_path)?;
    let source_override = FnxSourceOverride {
        path: &source_path,
        source,
        sidecar: Some(&sidecar),
    };
    let (document, assets) =
        read_project_tree_with_source_override(&project_root, Some(&source_override))?;
    let diagnostics = edited_source_diagnostics(&document, source, &sidecar);
    Ok((
        ProjectSourceEdit {
            document,
            assets,
            source_path,
        },
        diagnostics,
    ))
}

/// Typo/unknown-attribute warnings for one validated buffer. The regenerated
/// document supplies the name-resolution vocabulary; validation already
/// succeeded, so a decode failure here degrades to no diagnostics rather than
/// inventing a new error path.
fn edited_source_diagnostics(
    document: &Doc,
    source: &str,
    sidecar: &fanta_fnx::FnxSidecar,
) -> Vec<SourceDiagnostic> {
    let refs = super::refs_ctx::build_ref_table(&document.components, &document.variables, false);
    fanta_fnx::decode_subtree_with(source, sidecar, &refs)
        .map(|nodes| unknown_attribute_diagnostics(&nodes))
        .unwrap_or_default()
}

/// Validate and atomically persist one `.fnx` buffer edit. The returned
/// document is the exact regenerated state callers should install in the live
/// viewer. Invalid source is never written.
pub fn apply_project_source_edit(
    project_root: &Path,
    source_path: &Path,
    source: &str,
) -> Result<ProjectSourceEdit> {
    apply_project_source_edit_with_diagnostics(project_root, source_path, source)
        .map(|(edit, _)| edit)
}

/// [`apply_project_source_edit`] plus the buffer's non-fatal authoring
/// diagnostics (see [`validate_project_source_edit_with_diagnostics`]).
/// Diagnostics never block the write.
pub fn apply_project_source_edit_with_diagnostics(
    project_root: &Path,
    source_path: &Path,
    source: &str,
) -> Result<(ProjectSourceEdit, Vec<SourceDiagnostic>)> {
    let checked_source_path = source_path_for_read(project_root, source_path)?;
    let original = fs::read(&checked_source_path)?;
    let checked_sidecar_path = sidecar_path(&checked_source_path)?;
    let original_sidecar = read_optional(&checked_sidecar_path)?;
    let sidecar = persisted_fnx_sidecar(source, original_sidecar.as_deref())?;
    let persisted_sidecar = original_sidecar
        .as_ref()
        .and_then(|bytes| serde_json::from_slice::<fanta_fnx::FnxSidecar>(bytes).ok());
    let sidecar_changed = persisted_sidecar.as_ref() != Some(&sidecar);
    let (edit, diagnostics) =
        validate_project_source_edit_with_diagnostics(project_root, source_path, source)?;
    let current = fs::read(&edit.source_path)?;
    if current != original {
        return Err(FormatError::InvalidProjectTree(format!(
            "{} changed while the FNX edit was being validated",
            edit.source_path.display()
        )));
    }
    if read_optional(&checked_sidecar_path)? != original_sidecar {
        return Err(FormatError::InvalidProjectTree(format!(
            "{} changed while the FNX edit was being validated",
            checked_sidecar_path.display()
        )));
    }
    let source_changed = current != source.as_bytes();
    if !source_changed && !sidecar_changed {
        return Ok((edit, diagnostics));
    }

    if sidecar_changed {
        let sidecar_bytes = super::layout::json_bytes(&serde_json::to_value(&sidecar)?)?;
        atomic_write(&checked_sidecar_path, &sidecar_bytes)?;
    }
    if source_changed && let Err(source_error) = atomic_write(&edit.source_path, source.as_bytes())
    {
        if sidecar_changed
            && let Err(rollback_error) = match original_sidecar {
                Some(bytes) => atomic_write(&checked_sidecar_path, &bytes),
                None => fs::remove_file(&checked_sidecar_path).map_err(FormatError::Io),
            }
        {
            return Err(FormatError::InvalidProjectTree(format!(
                "writing {} failed: {source_error}; restoring {} also failed: {rollback_error}",
                edit.source_path.display(),
                checked_sidecar_path.display()
            )));
        }
        return Err(source_error);
    }
    Ok((edit, diagnostics))
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().ok_or_else(|| {
        FormatError::InvalidProjectTree(format!("{} has no parent directory", path.display()))
    })?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    match fs::metadata(path) {
        Ok(metadata) => temporary
            .as_file()
            .set_permissions(metadata.permissions())?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    temporary.write_all(bytes)?;
    temporary.flush()?;
    temporary.as_file().sync_all()?;
    temporary
        .persist(path)
        .map_err(|error| FormatError::Io(error.error))?;
    Ok(())
}

fn read_optional(path: &Path) -> Result<Option<Vec<u8>>> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

pub fn validate_project_json_edit(
    project_root: &Path,
    source_path: &Path,
    source: &str,
) -> Result<ProjectSourceEdit> {
    let project_root = project_root.canonicalize()?;
    let source_path = if source_path.is_absolute() {
        source_path.to_path_buf()
    } else {
        project_root.join(source_path)
    }
    .canonicalize()?;
    if !source_path.starts_with(&project_root)
        || source_path
            .extension()
            .is_none_or(|extension| extension != "json")
    {
        return Err(FormatError::InvalidProjectTree(format!(
            "{} is not a JSON source within {}",
            source_path.display(),
            project_root.display()
        )));
    }
    let value: serde_json::Value = serde_json::from_str(source).map_err(|error| {
        FormatError::InvalidProjectTree(format!("{}: {error}", source_path.display()))
    })?;
    if source_path
        .file_name()
        .is_some_and(|name| name == "page.ids.json" || name == "master.ids.json")
    {
        super::read::validate_project_json_shape(&source_path, &value)?;
        let sidecar: fanta_fnx::FnxSidecar = serde_json::from_value(value.clone())?;
        for entry in &sidecar.ids {
            super::layout::id_from_key::<fanta_doc::NodeId>(&entry.id).map_err(|error| {
                FormatError::InvalidProjectTree(format!("invalid sidecar node ID: {error}"))
            })?;
            if !entry.index.is_null() && !entry.index.is_number() {
                return Err(FormatError::InvalidProjectTree(
                    "sidecar sibling order must be a number".into(),
                ));
            }
        }
        if let Some(parent) = &sidecar.root_parent {
            super::layout::id_from_key::<fanta_doc::NodeId>(parent)?;
        }
        let fnx_name = if source_path
            .file_name()
            .is_some_and(|name| name == "page.ids.json")
        {
            "page.fnx"
        } else {
            "master.fnx"
        };
        let fnx_path = source_path.with_file_name(fnx_name);
        super::read::reconcile_fnx_sidecar(&fnx_path, &fs::read_to_string(&fnx_path)?, &sidecar)?;
    }
    let source_override = JsonSourceOverride {
        path: &source_path,
        value: &value,
    };
    let (document, assets) = read_project_tree_with_json_override(&project_root, &source_override)?;
    Ok(ProjectSourceEdit {
        document,
        assets,
        source_path,
    })
}

pub fn apply_project_json_edit(
    project_root: &Path,
    source_path: &Path,
    source: &str,
) -> Result<ProjectSourceEdit> {
    let original_path = if source_path.is_absolute() {
        source_path.to_path_buf()
    } else {
        project_root.join(source_path)
    };
    let original = fs::read(&original_path)?;
    let edit = validate_project_json_edit(project_root, source_path, source)?;
    if fs::read(&edit.source_path)? != original {
        return Err(FormatError::InvalidProjectTree(format!(
            "{} changed while the JSON edit was being validated",
            edit.source_path.display()
        )));
    }
    atomic_write(&edit.source_path, source.as_bytes())?;
    Ok(edit)
}

fn sidecar_path(source_path: &Path) -> Result<PathBuf> {
    let file_name = match source_path.file_name().and_then(|name| name.to_str()) {
        Some("page.fnx") => "page.ids.json",
        Some("master.fnx") => "master.ids.json",
        _ => {
            return Err(FormatError::InvalidProjectTree(format!(
                "{} is not an FNX project source",
                source_path.display()
            )));
        }
    };
    let parent = source_path.parent().ok_or_else(|| {
        FormatError::InvalidProjectTree(format!(
            "{} has no parent directory",
            source_path.display()
        ))
    })?;
    Ok(parent.join(file_name))
}

fn source_path_for_read(project_root: &Path, source_path: &Path) -> Result<PathBuf> {
    let (_, source_path) = checked_source_path(project_root, source_path)?;
    Ok(source_path)
}

fn checked_source_path(project_root: &Path, source_path: &Path) -> Result<(PathBuf, PathBuf)> {
    let project_root = project_root.canonicalize()?;
    if !super::layout::is_project_dir(&project_root) {
        return Err(FormatError::NotAProject { path: project_root });
    }
    let source_path = if source_path.is_absolute() {
        source_path.to_path_buf()
    } else {
        project_root.join(source_path)
    }
    .canonicalize()?;
    let relative = source_path.strip_prefix(&project_root).map_err(|_| {
        FormatError::InvalidProjectTree(format!(
            "FNX source {} is outside project {}",
            source_path.display(),
            project_root.display()
        ))
    })?;
    let components: Vec<_> = relative.components().collect();
    let is_page = matches!(
        components.as_slice(),
        [
            Component::Normal(root),
            Component::Normal(_),
            Component::Normal(file)
        ] if *root == "pages" && *file == "page.fnx"
    );
    let is_component = matches!(
        components.as_slice(),
        [
            Component::Normal(root),
            Component::Normal(_),
            Component::Normal(file)
        ] if *root == "components" && *file == "master.fnx"
    );
    if !is_page && !is_component {
        return Err(FormatError::InvalidProjectTree(format!(
            "{} is not a page.fnx or master.fnx project source",
            source_path.display()
        )));
    }
    Ok((project_root, source_path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use fanta_doc::{CanvasNode, Doc, GroupNode, NodeData};

    fn project() -> (tempfile::TempDir, PathBuf) {
        let mut document = Doc::new();
        let mut page = CanvasNode::new(NodeData::Group(GroupNode::default()));
        page.name = "Original".to_owned();
        let page_id = page.id;
        document.scene.insert(page).expect("insert page");
        document.add_page(page_id);

        let directory = tempfile::tempdir().expect("temporary project");
        super::super::layout::scaffold_project_tree(directory.path()).expect("scaffold project");
        super::super::write::write_project_tree(directory.path(), &document, &BTreeMap::new())
            .expect("write project");
        let source_path = super::super::read::locate_page_source(directory.path(), page_id)
            .expect("locate page source");
        assert_eq!(
            source_path,
            directory
                .path()
                .join("pages")
                .join("original")
                .join("page.fnx"),
            "v3 names the page dir by the page name's slug"
        );
        (directory, source_path)
    }

    #[test]
    fn validation_regenerates_without_writing() {
        let (directory, source_path) = project();
        let original = fs::read_to_string(&source_path).expect("read source");
        let changed = original.replace("name=\"Original\"", "name=\"Changed\"");
        let edit = validate_project_source_edit(directory.path(), &source_path, &changed)
            .expect("validate edit");
        let page_id = edit.document.pages()[0];
        assert_eq!(edit.document.scene.get(page_id).unwrap().name, "Changed");
        assert_eq!(fs::read_to_string(source_path).unwrap(), original);
    }

    #[test]
    fn invalid_source_is_never_persisted() {
        let (directory, source_path) = project();
        let original = fs::read_to_string(&source_path).expect("read source");
        let error = apply_project_source_edit(directory.path(), &source_path, "<Frame>")
            .expect_err("invalid source must fail");
        assert!(matches!(error, FormatError::InvalidProjectTree(_)));
        assert_eq!(fs::read_to_string(source_path).unwrap(), original);
    }

    #[test]
    fn source_save_repairs_missing_and_damaged_modern_sidecars() {
        let (directory, source_path) = project();
        let original = fs::read_to_string(&source_path).expect("read source");
        let sidecar_path = source_path.with_file_name("page.ids.json");
        fs::remove_file(&sidecar_path).expect("remove sidecar");
        let changed = original.replace("name=\"Original\"", "name=\"Changed\"");
        let edit = apply_project_source_edit(directory.path(), &source_path, &changed)
            .expect("save with missing sidecar");
        let page = edit.document.pages().first().copied().expect("page");
        assert_eq!(edit.document.scene.get(page).expect("root").name, "Changed");
        let repaired = fs::read(&sidecar_path).expect("recreated sidecar");
        fs::write(&sidecar_path, b"{\"ids\": [").expect("damage sidecar");
        apply_project_source_edit(directory.path(), &source_path, &changed)
            .expect("repair damaged sidecar");
        assert_eq!(fs::read(&sidecar_path).expect("repaired sidecar"), repaired);
    }

    #[test]
    fn typed_json_edits_are_validated_before_any_write() {
        let (directory, source_path) = project();
        let header_path = source_path.with_file_name("page.json");
        let original = fs::read_to_string(&header_path).expect("read page header");
        let mut header: serde_json::Value = serde_json::from_str(&original).expect("header JSON");
        header["order"] = serde_json::json!("later");
        assert!(
            apply_project_json_edit(directory.path(), &header_path, &header.to_string()).is_err()
        );
        assert_eq!(
            fs::read_to_string(&header_path).expect("untouched header"),
            original
        );
        header["order"] = serde_json::json!(3);
        apply_project_json_edit(directory.path(), &header_path, &header.to_string())
            .expect("save validated JSON");
        assert_eq!(
            fs::read_to_string(&header_path).expect("saved JSON"),
            header.to_string()
        );
        super::super::read::read_project_tree(directory.path()).expect("project still loads");
    }

    #[test]
    fn document_json_edits_materialize_and_bad_types_preserve_the_file() {
        let (directory, _) = project();
        let path = directory.path().join("doc/metadata.json");
        let original = fs::read_to_string(&path).expect("metadata");
        let mut metadata: serde_json::Value = serde_json::from_str(&original).expect("JSON");
        metadata["title"] = serde_json::json!("Updated document");
        let edit = validate_project_json_edit(directory.path(), &path, &metadata.to_string())
            .expect("validate metadata");
        assert_eq!(edit.document.metadata.title, "Updated document");
        assert_eq!(
            fs::read_to_string(&path).expect("unchanged metadata"),
            original
        );
        metadata["created_at"] = serde_json::json!("yesterday");
        let error = apply_project_json_edit(directory.path(), &path, &metadata.to_string())
            .expect_err("invalid metadata type");
        assert!(error.to_string().contains("metadata.created_at"));
        assert_eq!(
            fs::read_to_string(&path).expect("unchanged metadata"),
            original
        );
    }

    #[test]
    fn variable_registry_arrays_are_rejected_by_edit_validation_and_disk_loading() {
        let (directory, _) = project();
        let path = directory.path().join("doc/variables.json");
        let original = fs::read_to_string(&path).expect("variables");
        let error = validate_project_json_edit(directory.path(), &path, "[]")
            .expect_err("an array cannot replace the variable registry");
        assert!(
            error
                .to_string()
                .contains("variables.json must contain an object")
        );
        apply_project_json_edit(directory.path(), &path, "[]")
            .expect_err("invalid registry must not be saved");
        assert_eq!(
            fs::read_to_string(&path).expect("untouched registry"),
            original
        );

        fs::write(&path, "[]").expect("simulate external invalid edit");
        let error = super::super::read::read_project_tree(directory.path())
            .expect_err("disk loading enforces the same registry schema");
        assert!(
            error
                .to_string()
                .contains("variables.json must contain an object")
        );
        fs::write(&path, original).expect("restore registry");
        super::super::read::read_project_tree(directory.path()).expect("repaired registry loads");
    }

    #[test]
    fn invalid_asset_ids_in_valid_json_are_rejected_before_source_is_written() {
        let (directory, source_path) = project();
        let original = fs::read_to_string(&source_path).expect("source");
        let invalid_asset = "OKUCWRFB73B2XMEHY2MVAS5W34";
        let index_path = directory.path().join("assets/index.json");
        let original_index = fs::read_to_string(&index_path).expect("asset index");
        let index = serde_json::json!({
            "version": 1,
            "assets": {format!("a_{invalid_asset}"): {
                "size": 1,
                "sha256": "0".repeat(64),
            }},
        });
        let error = apply_project_json_edit(directory.path(), &index_path, &index.to_string())
            .expect_err("valid JSON with an invalid asset identity");
        assert!(error.to_string().contains("assets/index.json"));
        assert!(error.to_string().contains(invalid_asset));
        assert_eq!(
            fs::read_to_string(&index_path).expect("index"),
            original_index
        );

        let source = original.replace(
            " />\n  );\n}",
            &format!(
                ">\n      <Frame name=\"Photo\" background={{{{\"kind\":\"image\",\"asset\":\"{invalid_asset}\",\"mode\":\"fill\"}}}} />\n    </Frame>\n  );\n}}"
            ),
        );
        assert_ne!(source, original);
        let error = apply_project_source_edit(directory.path(), &source_path, &source)
            .expect_err("invalid image reference");
        assert!(error.to_string().contains("scene.nodes"));
        assert_eq!(fs::read_to_string(&source_path).expect("source"), original);
    }

    #[test]
    fn valid_source_is_atomically_applied_and_readable() {
        let (directory, source_path) = project();
        let changed = fs::read_to_string(&source_path)
            .expect("read source")
            .replace("name=\"Original\"", "name=\"Changed\"");
        let edit = apply_project_source_edit(directory.path(), &source_path, &changed)
            .expect("apply source edit");
        assert_eq!(fs::read_to_string(&source_path).unwrap(), changed);
        let page_id = edit.document.pages()[0];
        assert_eq!(edit.document.scene.get(page_id).unwrap().name, "Changed");
        let (reloaded, _) = super::super::read::read_project_tree(directory.path()).unwrap();
        assert_eq!(reloaded.scene.get(page_id).unwrap().name, "Changed");
    }

    #[test]
    fn page_root_tag_replacement_is_rejected_cleanly() {
        let (directory, source_path) = project();
        let original = fs::read_to_string(&source_path).expect("read source");
        let changed = original.replace("<Frame", "<Vector");
        assert_ne!(changed, original);

        let error = validate_project_source_edit(directory.path(), &source_path, &changed)
            .expect_err("a page root type flip must fail validation, not load garbage");
        match error {
            FormatError::InvalidProjectTree(message) => assert!(
                message.contains("root element of page.fnx must be a <Frame>"),
                "the error must name the invariant, got: {message}"
            ),
            other => panic!("expected InvalidProjectTree, got {other:?}"),
        }
        assert_eq!(fs::read_to_string(source_path).unwrap(), original);
    }

    #[test]
    fn adding_and_removing_fnx_elements_reconciles_the_sidecar() {
        let (directory, source_path) = project();
        let original = fs::read_to_string(&source_path).expect("read source");
        let added = original.replace(
            " />\n  );\n}",
            ">\n      <Vector name=\"Added\" path={{\"segments\": []}} />\n    </Frame>\n  );\n}",
        );
        assert_ne!(added, original);

        let first_validation = validate_project_source_edit(directory.path(), &source_path, &added)
            .expect("validate added vector");
        let page_id = first_validation.document.pages()[0];
        let added_id = first_validation.document.scene.children_of(Some(page_id))[0];
        let second_validation =
            validate_project_source_edit(directory.path(), &source_path, &added)
                .expect("validate added vector again");
        assert_eq!(
            second_validation.document.scene.children_of(Some(page_id)),
            &[added_id],
            "live validation must keep generated ids stable"
        );

        let ids_path = sidecar_path(&source_path).expect("sidecar path");
        let original_sidecar: fanta_fnx::FnxSidecar =
            serde_json::from_slice(&fs::read(&ids_path).expect("read original sidecar"))
                .expect("parse original sidecar");
        assert_eq!(original_sidecar.ids.len(), 1);

        fs::write(&source_path, &added).expect("simulate an ordinary editor save");
        let (mismatched_document, _) = super::super::read::read_project_tree(directory.path())
            .expect("read project with a stale sidecar");
        assert_eq!(
            mismatched_document.scene.children_of(Some(page_id)),
            &[added_id]
        );

        apply_project_source_edit(directory.path(), &source_path, &added)
            .expect("repair the stale sidecar");
        let added_sidecar: fanta_fnx::FnxSidecar =
            serde_json::from_slice(&fs::read(&ids_path).expect("read added sidecar"))
                .expect("parse added sidecar");
        assert_eq!(added_sidecar.ids.len(), 2);
        let (added_document, _) = super::super::read::read_project_tree(directory.path())
            .expect("read project with added vector");
        assert_eq!(added_document.scene.children_of(Some(page_id)), &[added_id]);

        apply_project_source_edit(directory.path(), &source_path, &original)
            .expect("remove added vector");
        let removed_sidecar: fanta_fnx::FnxSidecar =
            serde_json::from_slice(&fs::read(&ids_path).expect("read removed sidecar"))
                .expect("parse removed sidecar");
        assert_eq!(removed_sidecar.ids.len(), 1);
        let (removed_document, _) = super::super::read::read_project_tree(directory.path())
            .expect("read project after removing vector");
        assert!(removed_document.scene.children_of(Some(page_id)).is_empty());
    }

    #[test]
    fn source_edit_sidecar_stays_canonical_across_full_project_saves() {
        let (directory, source_path) = project();
        let original = fs::read_to_string(&source_path).expect("read source");
        let added = original.replace(
            " />\n  );\n}",
            ">\n      <Vector name=\"Added\" path={{\"segments\": []}} />\n    </Frame>\n  );\n}",
        );
        assert_ne!(added, original);
        let ids_path = sidecar_path(&source_path).expect("sidecar path");
        for (source, expected_count) in [(&added, 2), (&original, 1)] {
            let edit = apply_project_source_edit(directory.path(), &source_path, source)
                .expect("apply structural source edit");
            let bytes = fs::read(&ids_path).expect("edited sidecar");
            let value: serde_json::Value = serde_json::from_slice(&bytes).expect("sidecar JSON");
            let sidecar: fanta_fnx::FnxSidecar =
                serde_json::from_slice(&bytes).expect("sidecar identity");
            assert_eq!(sidecar.ids.len(), expected_count);
            assert_eq!(
                bytes,
                super::super::layout::json_bytes(&value).expect("canonical sidecar"),
                "code edits must use the same JSON ordering as full project saves"
            );
            assert_eq!(
                fs::read_to_string(&source_path).expect("authored source"),
                *source
            );
            super::super::write::write_project_tree(directory.path(), &edit.document, &edit.assets)
                .expect("save regenerated document");
            assert_eq!(fs::read(&ids_path).expect("saved sidecar"), bytes);
        }
    }

    #[test]
    fn source_edits_cannot_escape_or_target_project_metadata() {
        let (directory, source_path) = project();
        let source = fs::read_to_string(&source_path).unwrap();

        let outside = tempfile::NamedTempFile::new().unwrap();
        let outside_error = validate_project_source_edit(directory.path(), outside.path(), &source)
            .expect_err("outside source must fail");
        assert!(matches!(outside_error, FormatError::InvalidProjectTree(_)));

        let manifest_error =
            validate_project_source_edit(directory.path(), Path::new("fanta.json"), &source)
                .expect_err("manifest is not an FNX source");
        assert!(matches!(manifest_error, FormatError::InvalidProjectTree(_)));
    }
}
