//! Workspace-level wiring for the Fanta canvas.
//!
//! Fanta's unit of work is a *directory* — a `fanta-project` tree holding
//! `fanta.json`, `pages/`, `components/` and `assets/` — but every route into
//! the app (the folder picker, a path on the command line, a session restore,
//! the `New Design` action) hands the workspace a folder, not a file. Without
//! the hook below, opening a Fanta project folder shows a project panel and an
//! empty pane: the design never reaches the canvas.
//!
//! So: watch each workspace's project for worktrees, and whenever one turns
//! out to be a Fanta project that has no canvas tab yet, open its `fanta.json`.
//! That path is enough, because `FigItem::try_open` accepts the manifest and
//! `ProjectItemRegistry::open_path` walks its registrations in reverse —
//! `fig_viewer::init` runs after `editor::init`, so `fanta.json` resolves to
//! the canvas rather than to a JSON buffer.
//!
//! Saved canvas tabs restore through `SerializableItem`. This hook also opens
//! newly added project folders and sessions saved before canvas serialization.

use std::path::{Path, PathBuf};

use anyhow::Context as _;
use gpui::{App, AppContext as _, Context, Entity, Task, TaskExt as _, Window};
use project::{Project, ProjectItem as _, ProjectPath, WorktreeId};
use util::rel_path::RelPath;
use workspace::Workspace;
use worktree::PathChange;

use crate::view::FigView;

/// The manifest at the root of every Fanta project; opening it opens the
/// design.
const MANIFEST: &str = "fanta.json";

pub fn init(cx: &mut App) {
    cx.observe_new(|workspace: &mut Workspace, window, cx| {
        let Some(window) = window else {
            return;
        };
        crate::new_design::register(workspace);
        workspace.register_action(
            |workspace, _: &zed_actions::fanta::RevealProjectInFileManager, window, cx| {
                let root = workspace
                    .active_item_as::<FigView>(cx)
                    .and_then(|view| {
                        view.read(cx)
                            .item()
                            .read(cx)
                            .project_root()
                            .map(Path::to_path_buf)
                    })
                    .or_else(|| {
                        workspace
                            .project()
                            .read(cx)
                            .visible_worktrees(cx)
                            .next()
                            .map(|worktree| worktree.read(cx).abs_path().to_path_buf())
                    });
                if let Some(root) = root {
                    cx.reveal_path(&root);
                } else {
                    crate::view::show_canvas_notice(
                        "Open or save a project before revealing its folder.".into(),
                        window,
                        cx,
                    );
                }
            },
        );
        crate::generation_workspace::register(workspace);
        crate::live_mcp::register(workspace);

        let project = workspace.project().clone();
        cx.subscribe_in(
            &project,
            window,
            |workspace, _project, event, window, cx| match event {
                project::Event::WorktreeAdded(worktree_id) => {
                    open_design_for_worktree(workspace, *worktree_id, window, cx)
                        .detach_and_log_err(cx);
                }
                project::Event::WorktreeUpdatedEntries(worktree_id, changes)
                    if changes.iter().any(|(path, _, change)| {
                        path.as_std_path() == Path::new(MANIFEST)
                            && !matches!(change, PathChange::Loaded)
                    }) =>
                {
                    open_design_for_worktree(workspace, *worktree_id, window, cx)
                        .detach_and_log_err(cx);
                }
                _ => {}
            },
        )
        .detach();

        // A restored session (and any window opened straight onto a folder)
        // already has its worktrees by the time this observer runs, so those
        // never emit `WorktreeAdded` here. Sweep them once; the dedupe below
        // makes the overlap with the subscription harmless.
        let existing: Vec<WorktreeId> = project
            .read(cx)
            .visible_worktrees(cx)
            .map(|worktree| worktree.read(cx).id())
            .collect();
        for worktree_id in existing {
            open_design_for_worktree(workspace, worktree_id, window, cx).detach_and_log_err(cx);
        }
    })
    .detach();
}

/// Open the canvas for `worktree_id` when it is a Fanta project that is not
/// already on screen.
pub(crate) fn open_design_for_worktree(
    workspace: &mut Workspace,
    worktree_id: WorktreeId,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) -> Task<anyhow::Result<()>> {
    let project = workspace.project().clone();
    let Some(root) = worktree_root(&project, worktree_id, cx) else {
        return Task::ready(Ok(()));
    };
    // The cheap, synchronous half of the dedupe: bail before spawning anything
    // when this project is plainly already open.
    if design_is_open(workspace, &root, cx) {
        return Task::ready(Ok(()));
    }

    cx.spawn_in(window, async move |workspace, cx| {
        // `is_project_dir` reads and parses `fanta.json`; keep that off the
        // UI thread, since a worktree is added while the user is waiting to
        // see their window.
        let is_project = cx
            .background_spawn({
                let root = root.clone();
                async move { fanta_format::is_project_dir(&root) }
            })
            .await;
        if !is_project {
            return anyhow::Ok(());
        }

        let open = workspace.update_in(cx, |workspace, _, cx| {
            // Re-check after the await. Two worktree events for the same
            // project (the initial sweep racing the subscription, or a
            // `.fig` load adopting the directory it just materialized) both
            // reach this point, and without the second check each one opens
            // its own tab.
            if design_is_open(workspace, &root, cx) {
                return anyhow::Ok(None);
            }
            let path = ProjectPath {
                worktree_id,
                path: RelPath::unix(MANIFEST)
                    .context("building the fanta.json project path")?
                    .into(),
            };
            anyhow::Ok(crate::FigItem::try_open(&project, &path, cx).map(|open| (path, open)))
        })??;
        if let Some((path, open)) = open {
            let item = open.await?;
            workspace.update_in(cx, |workspace, window, cx| {
                if design_is_open(workspace, &root, cx)
                    || worktree_root(&project, worktree_id, cx).as_ref() != Some(&root)
                {
                    crate::document::take_pending_view_descriptor(cx);
                    return;
                }
                let entry_id = project
                    .read(cx)
                    .entry_for_path(&path, cx)
                    .map(|entry| entry.id);
                crate::document::set_pending_view_descriptor(
                    crate::document::FigViewDescriptor {
                        entry_id,
                        scope: None,
                    },
                    cx,
                );
                let view = cx.new(|cx| FigView::new(item, project.clone(), window, cx));
                let pane = workspace.active_pane().clone();
                pane.update(cx, |pane, cx| {
                    // Resolve activation after loading: restored tabs or an
                    // explicit Open may have become active during the await.
                    let activate = pane.active_item().is_none();
                    pane.add_item_inner(
                        Box::new(view),
                        activate,
                        activate,
                        activate,
                        None,
                        window,
                        cx,
                    );
                });
            })?;
        }
        anyhow::Ok(())
    })
}

fn worktree_root(project: &Entity<Project>, worktree_id: WorktreeId, cx: &App) -> Option<PathBuf> {
    Some(
        project
            .read(cx)
            .worktree_for_id(worktree_id, cx)?
            .read(cx)
            .abs_path()
            .to_path_buf(),
    )
}

/// Whether some pane in `workspace` already shows the design rooted at `root`.
///
/// This is the whole defense against a duplicate canvas tab, and it has to
/// key on the project root rather than the opened path: a user who opened
/// `Design.fig` gets a canvas whose `FigItem` materialized `Design/` and then
/// added it as a worktree, which lands right back here — with a completely
/// different `ProjectPath` naming the very same design.
fn design_is_open(workspace: &Workspace, root: &Path, cx: &App) -> bool {
    let open_roots: Vec<PathBuf> = workspace
        .items_of_type::<FigView>(cx)
        .filter_map(|view| {
            view.read(cx)
                .item()
                .read(cx)
                .project_root()
                .map(Path::to_path_buf)
        })
        .collect();
    if open_roots
        .iter()
        .any(|open_root| open_root.as_path() == root)
    {
        return true;
    }
    // Only worth the syscalls once the literal compare has missed: a worktree
    // path and the path a `FigItem` derived from the file it opened can
    // disagree over symlinks (`/tmp` vs `/private/tmp` on macOS) while naming
    // the same directory.
    let canonical_root = canonical(root);
    open_roots
        .iter()
        .any(|open_root| canonical(open_root) == canonical_root)
}

fn canonical(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}
