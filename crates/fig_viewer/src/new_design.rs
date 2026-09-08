//! `New Design`: pick a folder, scaffold an empty Fanta project into it, and
//! open that folder as the window's project.
//!
//! Opening the FOLDER rather than the freshly written `fanta.json` is
//! deliberate — it is what gives the workspace an open project (which the
//! Agent Panel and the canvas's disk-sync watcher both require). The worktree
//! that appears is then picked up by [`crate::workspace_hooks`], which puts
//! the canvas tab on screen.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context as _, Result, bail};
use fanta_doc::{CanvasNode, Doc, GroupNode, NodeData, Operation};
use gpui::{AppContext as _, Context, TaskExt as _, Window};
use project::DirectoryLister;
use workspace::{MultiWorkspace, OpenMode, Workspace};

/// The single page a new design starts with. Matches the name the design
/// panel's "add page" gives the first page it creates, so a project made here
/// and one grown in the app read the same.
const FIRST_PAGE_NAME: &str = "Page 1";

/// The folder name the save panel proposes.
const SUGGESTED_NAME: &str = "Untitled Design";

/// Hook the `New Design` action up to a freshly created workspace. Called from
/// [`crate::workspace_hooks::init`]'s `observe_new` so every window gets it.
pub(crate) fn register(workspace: &mut Workspace) {
    workspace.register_action(|workspace, _: &zed_actions::fanta::NewDesign, window, cx| {
        prompt_and_create(workspace, window, cx);
    });
}

fn prompt_and_create(workspace: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) {
    let project = workspace.project().clone();
    let lister = if project.read(cx).is_local() {
        DirectoryLister::Local(project, workspace.app_state().fs.clone())
    } else {
        DirectoryLister::Project(project)
    };
    let chosen_path =
        workspace.prompt_for_new_path(lister, Some(SUGGESTED_NAME.to_owned()), window, cx);
    let multi_workspace = window.window_handle().downcast::<MultiWorkspace>();

    cx.spawn_in(window, async move |workspace, cx| {
        // A cancelled picker is not an error; a dropped sender means the
        // window went away mid-prompt, which is not one either.
        let Some(root) = chosen_path
            .await
            .ok()
            .flatten()
            .and_then(|paths| paths.into_iter().next())
        else {
            return anyhow::Ok(());
        };

        let created = cx
            .background_spawn({
                let root = root.clone();
                async move { create_project(&root) }
            })
            .await;
        if let Err(error) = created {
            log::error!(
                "creating a Fanta project at {} failed: {error:#}",
                root.display()
            );
            workspace.update_in(cx, |_, window, cx| {
                crate::view::show_canvas_notice(
                    format!("The design could not be created: {error:#}"),
                    window,
                    cx,
                );
            })?;
            return anyhow::Ok(());
        }

        if let Some(multi_workspace) = multi_workspace {
            multi_workspace
                .update(cx, |multi_workspace, window, cx| {
                    multi_workspace.open_project(vec![root], OpenMode::Activate, window, cx)
                })?
                .await?;
            return anyhow::Ok(());
        }
        // A window whose root is not a `MultiWorkspace` (tests, and the
        // single-workspace shell) still has to land somewhere.
        workspace
            .update_in(cx, |workspace, window, cx| {
                workspace.open_workspace_for_paths(OpenMode::NewWindow, vec![root], window, cx)
            })?
            .await?;
        anyhow::Ok(())
    })
    .detach_and_log_err(cx);
}

/// Scaffold a project tree at `root`, write a one-page document into it, and
/// make the folder a git repository (see [`crate::document::write_project`]),
/// so the design's history is reviewable from the first save.
///
/// Refuses when `root` already holds anything. [`fanta_format::scaffold_project_tree`]
/// uses `create_dir_all` and unconditionally rewrites `fanta.json` and
/// `.gitignore`, so pointing it at somebody's existing folder would tag that
/// folder as a Fanta project — and then `write_project_tree` prunes every
/// managed path the (nearly empty) projection does not claim. Nothing about
/// "create a new design" implies permission to do that, so the guard comes
/// first, before a single byte is written.
fn create_project(root: &Path) -> Result<()> {
    match std::fs::read_dir(root) {
        Ok(mut entries) => {
            if entries.next().is_some() {
                bail!(
                    "{} already exists and is not empty — pick a new folder for the design",
                    root.display()
                );
            }
        }
        // A path with nothing at it is exactly what we want. Anything else —
        // a file sitting at that path, a permissions failure — is a reason to
        // stop, not to scaffold.
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(error)
                .with_context(|| format!("{} is not an empty directory", root.display()));
        }
    }

    let doc = new_document(root)?;
    crate::document::write_project(root, &doc, &BTreeMap::new())
}

/// A document with exactly one empty page.
///
/// The page root is a plain [`NodeData::Group`], which is what both authorities
/// produce: the `.fig` importer maps a Figma `CANVAS` to
/// `NodeData::Group(GroupNode { .. })` and registers it with `Doc::add_page`,
/// and the design panel's own "add page" creates
/// `CanvasNode::new(NodeData::Group(GroupNode::default()))`. The importer
/// additionally stamps `meta.figma_type = "CANVAS"`; that is a Figma
/// correlation key for round-tripping an import, not something a natively
/// created page should claim.
fn new_document(root: &Path) -> Result<Doc> {
    let mut doc = Doc::new();
    doc.metadata.title = root
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("Untitled")
        .to_owned();

    let mut page = CanvasNode::new(NodeData::Group(GroupNode::default()));
    page.name = FIRST_PAGE_NAME.to_owned();
    page.index = doc.scene.next_root_index();
    let page_root = page.id;
    doc.apply(Operation::create_node(page))
        .context("creating the first page of a new design")?;
    doc.add_page(page_root);
    Ok(doc)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_design_round_trips_through_the_project_reader() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("My Design");
        create_project(&root).expect("creating the project");

        assert!(
            fanta_format::is_project_dir(&root),
            "the scaffolded directory must be recognized as a Fanta project"
        );

        let (doc, assets) = fanta_format::read_project_tree(&root).expect("reading the project");
        assert!(assets.is_empty(), "a new design ships no assets");
        assert_eq!(doc.metadata.title, "My Design");
        assert_eq!(doc.pages().len(), 1, "exactly one page");

        let page = doc.pages()[0];
        let node = doc
            .scene
            .get(page)
            .expect("the page root survives the write");
        assert_eq!(node.name, FIRST_PAGE_NAME);
        assert!(
            matches!(node.data, NodeData::Group(_)),
            "the page root is a group, matching the importer and the design panel"
        );
        assert!(
            doc.scene.children_of(Some(page)).is_empty(),
            "the page starts empty"
        );
    }

    #[test]
    fn creating_a_design_refuses_to_write_into_a_non_empty_directory() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("Taken");
        std::fs::create_dir_all(&root).expect("creating the directory");
        std::fs::write(root.join("notes.txt"), b"mine").expect("writing a file");

        let error = create_project(&root).expect_err("an occupied directory must be refused");
        assert!(
            error.to_string().contains("not empty"),
            "the refusal names the reason: {error:#}"
        );
        assert!(
            !root.join("fanta.json").exists(),
            "nothing was written into the user's directory"
        );
        assert_eq!(
            std::fs::read(root.join("notes.txt")).expect("the file survives"),
            b"mine".to_vec()
        );
    }

    #[test]
    fn creating_a_design_accepts_an_existing_empty_directory() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("Empty");
        std::fs::create_dir_all(&root).expect("creating the directory");

        create_project(&root).expect("an empty directory is a valid target");
        assert!(fanta_format::is_project_dir(&root));
    }
}
