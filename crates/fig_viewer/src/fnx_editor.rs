use std::ffi::OsStr;
use std::path::Path;
use std::{cell::Cell, rc::Rc};

use anyhow::{Context as _, Result};
use editor::Editor;
use gpui::{App, Context, Entity};
use language::{Buffer, Capability};

pub(crate) fn init(cx: &mut App) {
    cx.observe_new(|editor: &mut Editor, _window, cx: &mut Context<Editor>| {
        let multi_buffer = editor.buffer().clone();
        let migration_pending = Rc::new(Cell::new(true));
        cx.observe(&multi_buffer, {
            let migration_pending = migration_pending.clone();
            move |editor, _, cx| {
                if !migration_pending.get() {
                    return;
                }
                if editor.read_only(cx) {
                    migration_pending.set(false);
                    return;
                }
                let buffer = editor.buffer().read(cx).as_singleton();
                if let Some(buffer) = buffer {
                    if let Err(error) = upgrade_legacy_fnx_buffer(&buffer, cx) {
                        log::error!("preparing the FNX editor failed: {error:#}");
                    }
                    if upgrade_attempt_is_complete(&buffer, cx) {
                        migration_pending.set(false);
                    }
                }
            }
        })
        .detach();
        if editor.read_only(cx) {
            migration_pending.set(false);
        } else {
            let buffer = editor.buffer().read(cx).as_singleton();
            if let Some(buffer) = buffer {
                if let Err(error) = upgrade_legacy_fnx_buffer(&buffer, cx) {
                    log::error!("preparing the FNX editor failed: {error:#}");
                }
                if upgrade_attempt_is_complete(&buffer, cx) {
                    migration_pending.set(false);
                }
            }
        }
    })
    .detach();
}

fn upgrade_attempt_is_complete(buffer: &Entity<Buffer>, cx: &App) -> bool {
    let buffer = buffer.read(cx);
    if buffer.is_dirty() {
        return true;
    }
    buffer.file().is_some() && !buffer.text().is_empty()
}

pub(crate) fn upgrade_legacy_fnx_buffer(buffer: &Entity<Buffer>, cx: &mut App) -> Result<bool> {
    let (path, source) = {
        let buffer = buffer.read(cx);
        if buffer.is_dirty() || !matches!(buffer.capability(), Capability::ReadWrite) {
            return Ok(false);
        }
        let Some(path) = buffer
            .file()
            .and_then(|file| file.as_local())
            .map(|file| file.abs_path(cx))
        else {
            return Ok(false);
        };
        (path, buffer.text())
    };
    if source.is_empty() {
        return Ok(false);
    }

    if let Some(canonical) = canonicalize_project_source(&path, &source)? {
        buffer.update(cx, |buffer, cx| {
            buffer.set_text(canonical, cx);
        });
        return Ok(true);
    }
    Ok(false)
}

fn canonicalize_project_source(path: &Path, source: &str) -> Result<Option<String>> {
    let Some(project_root) = project_root_for_fnx_source(path) else {
        return Ok(None);
    };
    if !fanta_format::is_project_dir(project_root) {
        return Ok(None);
    }
    fanta_format::ensure_project_editor_support(project_root)
        .with_context(|| format!("seeding editor support in {}", project_root.display()))?;
    Ok(fanta_format::canonicalize_legacy_source(source))
}

fn project_root_for_fnx_source(path: &Path) -> Option<&Path> {
    let source_name = path.file_name()?;
    let id_directory = path.parent()?;
    let source_directory = id_directory.parent()?;
    let expected_directory = match source_name {
        name if name == OsStr::new("page.fnx") => OsStr::new("pages"),
        name if name == OsStr::new("master.fnx") => OsStr::new("components"),
        _ => return None,
    };
    if source_directory.file_name()? != expected_directory {
        return None;
    }
    source_directory.parent()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_root_only_matches_page_and_component_sources() {
        let root = Path::new("/project");
        assert_eq!(
            project_root_for_fnx_source(Path::new("/project/pages/n_page/page.fnx")),
            Some(root)
        );
        assert_eq!(
            project_root_for_fnx_source(Path::new("/project/components/c_button/master.fnx")),
            Some(root)
        );
        for path in [
            "/project/workspace.fnx",
            "/project/pages/n_page/master.fnx",
            "/project/components/c_button/page.fnx",
            "/project/other/n_page/page.fnx",
        ] {
            assert_eq!(project_root_for_fnx_source(Path::new(path)), None);
        }
    }

    #[test]
    fn project_source_upgrade_seeds_support_and_returns_a_dirty_buffer_candidate() {
        let project = tempfile::tempdir().expect("temporary project");
        fanta_format::scaffold_project_tree(project.path()).expect("scaffold project");
        std::fs::remove_file(project.path().join("fnx.d.ts")).expect("remove types seed");
        std::fs::remove_file(project.path().join(".prettierrc.json"))
            .expect("remove prettier seed");
        let path = project.path().join("pages/n_page/page.fnx");
        let legacy = "// @generated fanta source\nexport default () => <Frame background={{color: #ffffff}} />;\n";

        let canonical = canonicalize_project_source(&path, legacy)
            .expect("upgrade source")
            .expect("legacy source needs an upgrade");

        assert!(canonical.contains("@jsxRuntime classic"));
        assert!(canonical.contains("import { AiArtifact"));
        assert!(canonical.contains("fnxColor(\"#FFFFFF\")"));
        assert!(project.path().join("fnx.d.ts").is_file());
        assert!(project.path().join(".prettierrc.json").is_file());
        assert_eq!(
            canonicalize_project_source(&path, &canonical).unwrap(),
            None
        );
    }
}
