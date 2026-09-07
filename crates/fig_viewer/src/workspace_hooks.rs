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
//! `FigView` is not a `SerializableItem`, so a restored session brings the
//! folder back but never the tab; this hook is what puts it on screen again.

use std::path::{Path, PathBuf};

use anyhow::Context as _;
use gpui::{App, AppContext as _, Context, Entity, TaskExt as _, Window};
use project::{Project, ProjectPath, WorktreeId};
use util::rel_path::RelPath;
use workspace::Workspace;

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

        let project = workspace.project().clone();
        cx.subscribe_in(
            &project,
            window,
            |workspace, _project, event, window, cx| {
                if let project::Event::WorktreeAdded(worktree_id) = event {
                    open_design_for_worktree(workspace, *worktree_id, window, cx);
                }
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
            open_design_for_worktree(workspace, worktree_id, window, cx);
        }
    })
    .detach();
}

/// Open the canvas for `worktree_id` when it is a Fanta project that is not
/// already on screen.
fn open_design_for_worktree(
    workspace: &mut Workspace,
    worktree_id: WorktreeId,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    let project = workspace.project().clone();
    let Some(root) = worktree_root(&project, worktree_id, cx) else {
        return;
    };
    // The cheap, synchronous half of the dedupe: bail before spawning anything
    // when this project is plainly already open.
    if design_is_open(workspace, &root, cx) {
        return;
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

        let open = workspace.update_in(cx, |workspace, window, cx| {
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
            anyhow::Ok(Some(workspace.open_path(path, None, true, window, cx)))
        })??;
        if let Some(open) = open {
            open.await?;
        }
        anyhow::Ok(())
    })
    .detach_and_log_err(cx);
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
