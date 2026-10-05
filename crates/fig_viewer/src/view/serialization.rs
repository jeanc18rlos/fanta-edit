use std::path::PathBuf;

use db::{
    query,
    sqlez::{domain::Domain, thread_safe_connection::ThreadSafeConnection},
    sqlez_macros::sql,
};
use gpui::WeakEntity;
use project::{ProjectItem as _, ProjectPath};
use serde::{Deserialize, Serialize};
use workspace::{ItemId, Workspace, WorkspaceDb, WorkspaceId, item::SerializableItem};

use super::*;

#[derive(Serialize, Deserialize)]
enum SavedScope {
    Page(NodeId),
    Component(fanta_doc::ComponentId),
    Variables,
}

impl From<FigScope> for SavedScope {
    fn from(scope: FigScope) -> Self {
        match scope {
            FigScope::Page(root) => Self::Page(root),
            FigScope::Component(component) => Self::Component(component),
            FigScope::Variables => Self::Variables,
        }
    }
}

impl From<SavedScope> for FigScope {
    fn from(scope: SavedScope) -> Self {
        match scope {
            SavedScope::Page(root) => Self::Page(root),
            SavedScope::Component(component) => Self::Component(component),
            SavedScope::Variables => Self::Variables,
        }
    }
}

struct FigViewerDb(ThreadSafeConnection);

impl Domain for FigViewerDb {
    const NAME: &str = stringify!(FigViewerDb);
    const MIGRATIONS: &[&str] = &[sql!(
        CREATE TABLE fig_viewers (
            workspace_id INTEGER,
            item_id INTEGER,
            abs_path BLOB NOT NULL,
            source_path BLOB,
            scope TEXT NOT NULL,
            PRIMARY KEY(workspace_id, item_id),
            FOREIGN KEY(workspace_id) REFERENCES workspaces(workspace_id) ON DELETE CASCADE
        ) STRICT;
    )];
}

db::static_connection!(FigViewerDb, [WorkspaceDb]);

impl FigViewerDb {
    query! {
        async fn save_view(
            item_id: ItemId,
            workspace_id: WorkspaceId,
            abs_path: PathBuf,
            source_path: Option<PathBuf>,
            scope: String
        ) -> Result<()> {
            INSERT OR REPLACE INTO fig_viewers(item_id, workspace_id, abs_path, source_path, scope)
            VALUES (?, ?, ?, ?, ?)
        }
    }

    query! {
        fn get_view(item_id: ItemId, workspace_id: WorkspaceId) -> Result<Option<(PathBuf, Option<PathBuf>, String)>> {
            SELECT abs_path, source_path, scope FROM fig_viewers
            WHERE item_id = ? AND workspace_id = ?
        }
    }
}

impl SerializableItem for FigView {
    fn serialized_item_kind() -> &'static str {
        "FigView"
    }

    fn cleanup(
        workspace_id: WorkspaceId,
        alive_items: Vec<ItemId>,
        _window: &mut Window,
        cx: &mut App,
    ) -> Task<Result<()>> {
        workspace::delete_unloaded_items(
            alive_items,
            workspace_id,
            "fig_viewers",
            &FigViewerDb::global(cx),
            cx,
        )
    }

    fn deserialize(
        project: Entity<Project>,
        _workspace: WeakEntity<Workspace>,
        workspace_id: WorkspaceId,
        item_id: ItemId,
        window: &mut Window,
        cx: &mut App,
    ) -> Task<Result<Entity<Self>>> {
        let database = FigViewerDb::global(cx);
        window.spawn(cx, async move |cx| {
            let (abs_path, source_path, scope) = database
                .get_view(item_id, workspace_id)?
                .context("No saved canvas path found")?;
            let scope: Option<SavedScope> = serde_json::from_str(&scope)?;
            let scope = scope.map(FigScope::from);
            let project_root = abs_path
                .parent()
                .filter(|_| abs_path.ends_with("fanta.json"))
                .map(std::path::Path::to_path_buf);
            let path = cx
                .background_spawn(async move {
                    anyhow::ensure!(
                        abs_path.is_file(),
                        "The saved design is missing: {}",
                        abs_path.display()
                    );
                    Ok::<_, anyhow::Error>(
                        source_path
                            .filter(|path| path.is_file())
                            .or_else(|| {
                                // Save As invalidates the old source entry. Keep
                                // scoped tabs distinct instead of opening every
                                // copied tab from the same manifest entry.
                                let root = abs_path
                                    .parent()
                                    .filter(|_| abs_path.ends_with("fanta.json"))?;
                                match scope? {
                                    FigScope::Page(page) => {
                                        fanta_format::locate_page_source(root, page)
                                    }
                                    FigScope::Component(component) => {
                                        fanta_format::locate_master_source(root, component)
                                    }
                                    FigScope::Variables => Some(root.join("doc/variables.json")),
                                }
                                .filter(|path| path.is_file())
                            })
                            .unwrap_or(abs_path),
                    )
                })
                .await?;
            // A source-only worktree can disappear when the project folder
            // loads later, invalidating the restored tab's entry identity.
            if let Some(root) = project_root {
                project
                    .update(cx, |project, cx| {
                        project.find_or_create_worktree(root, true, cx)
                    })
                    .await?;
            }
            let (worktree, relative_path) = project
                .update(cx, |project, cx| {
                    project.find_or_create_worktree(path, false, cx)
                })
                .await?;
            let refresh = worktree.update(cx, |worktree, cx| {
                if worktree.entry_for_path(&relative_path).is_some() {
                    None
                } else {
                    worktree
                        .as_local()
                        .map(|local| local.refresh_entry(relative_path.clone(), None, cx))
                }
            });
            if let Some(refresh) = refresh {
                refresh.await?;
            }
            let path = ProjectPath {
                worktree_id: worktree.read_with(cx, |worktree, _| worktree.id()),
                path: relative_path,
            };
            let open = cx
                .update(|_, cx| FigItem::try_open(&project, &path, cx))?
                .context("The saved path is not a supported design")?;
            let item = open.await?;
            cx.update(|window, cx| {
                // Restored tabs load concurrently, so the global descriptor
                // left by try_open may belong to a different tab by now.
                let entry_id = project
                    .read(cx)
                    .entry_for_path(&path, cx)
                    .map(|entry| entry.id);
                crate::document::set_pending_view_descriptor(
                    crate::document::FigViewDescriptor { entry_id, scope },
                    cx,
                );
                cx.new(|cx| Self::new(item, project, window, cx))
            })
        })
    }

    fn serialize(
        &mut self,
        workspace: &mut Workspace,
        item_id: ItemId,
        closing: bool,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Task<Result<()>>> {
        // This saves navigation only. Returning a task for unsaved content
        // during close would opt into hot exit and suppress the save prompt.
        if closing
            && (self.is_dirty(cx)
                || self.item.read(cx).source_edit_locked()
                || self.item.read(cx).content_preview_active()
                || self.text_edit.is_some()
                || self.pending_text_edit.is_some()
                || self.close_blocker(cx).is_some())
        {
            return None;
        }
        let workspace_id = workspace.database_id()?;
        let item = self.item.read(cx);
        let abs_path = item
            .project_root()
            .map(|root| root.join("fanta.json"))
            .unwrap_or_else(|| item.abs_path().to_path_buf());
        let source_path = self
            .opened_entry_id
            .and_then(|entry| self.project.read(cx).path_for_entry(entry, cx))
            .and_then(|path| self.project.read(cx).absolute_path(&path, cx))
            .filter(|path| {
                item.project_root()
                    .is_none_or(|root| path.starts_with(root))
            });
        let scope = self
            .scope
            .or_else(|| self.selected_page_root.map(FigScope::Page))
            .map(SavedScope::from);
        let scope = match serde_json::to_string(&scope) {
            Ok(scope) => scope,
            Err(error) => return Some(Task::ready(Err(error.into()))),
        };
        let database = FigViewerDb::global(cx);
        Some(cx.background_spawn(async move {
            database
                .save_view(item_id, workspace_id, abs_path, source_path, scope)
                .await
        }))
    }

    fn should_serialize(&self, event: &Self::Event) -> bool {
        matches!(event, FigViewEvent::TitleChanged | FigViewEvent::Edited)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{TestAppContext, WindowHandle};
    use project::FakeFs;

    struct Fixture {
        directory: tempfile::TempDir,
        project: Entity<Project>,
        file_system: Arc<FakeFs>,
        workspace: Entity<Workspace>,
        window: WindowHandle<Empty>,
        workspace_id: WorkspaceId,
        pages: [NodeId; 2],
    }

    async fn fixture(cx: &mut TestAppContext) -> Fixture {
        cx.update(|cx| {
            zlog::init_test();
            assets::Assets.load_test_fonts(cx);
            let settings_store = settings::SettingsStore::test(cx);
            cx.set_global(settings_store);
            theme_settings::init(theme::LoadThemes::JustBase, cx);
            release_channel::init(semver::Version::new(0, 0, 0), cx);
            editor::init(cx);
            #[cfg(feature = "fanta-gpui-ui")]
            {
                gpui_component::init(cx);
                fanta_gpui::init(cx);
                crate::theme_bridge::init(cx);
            }
            workspace::register_project_item::<FigView>(cx);
            workspace::register_serializable_item::<FigView>(cx);
        });
        let directory = tempfile::tempdir().expect("session projects");
        let mut doc = Doc::new();
        let pages = std::array::from_fn(|index| {
            let mut page = CanvasNode::new(NodeData::Group(Default::default()));
            page.name = format!("Page {}", index + 1);
            let id = page.id;
            doc.scene.insert(page).expect("insert page");
            doc.add_page(id);
            id
        });
        doc.set_active_page(pages.first().copied());
        for name in ["Original", "Other"] {
            crate::document::write_project(&directory.path().join(name), &doc, &BTreeMap::new())
                .expect("write fixture project");
        }
        std::fs::create_dir(directory.path().join("SavedCopy")).expect("empty copy destination");
        let file_system = FakeFs::new(cx.executor());
        file_system
            .insert_tree_from_real_fs(directory.path(), directory.path())
            .await;
        let roots = [
            directory.path().join("Original"),
            directory.path().join("Other"),
        ];
        let project =
            Project::test(file_system.clone(), roots.iter().map(PathBuf::as_path), cx).await;
        let workspace_id = cx
            .update(|cx| WorkspaceDb::global(cx))
            .next_id()
            .await
            .expect("workspace id");
        let window = cx.add_window(|_, _| Empty);
        let workspace = window
            .update(cx, |_, window, cx| {
                let temporary = cx.new(|cx| Workspace::test_new(project.clone(), window, cx));
                let app_state = temporary.read(cx).app_state().clone();
                cx.new(|cx| {
                    Workspace::new(Some(workspace_id), project.clone(), app_state, window, cx)
                })
            })
            .expect("workspace");
        Fixture {
            directory,
            project,
            file_system,
            workspace,
            window,
            workspace_id,
            pages,
        }
    }

    async fn open(fixture: &Fixture, name: &str, cx: &mut TestAppContext) -> Entity<FigView> {
        open_path(
            fixture,
            fixture.directory.path().join(name).join("fanta.json"),
            cx,
        )
        .await
    }

    async fn open_path(
        fixture: &Fixture,
        absolute_path: PathBuf,
        cx: &mut TestAppContext,
    ) -> Entity<FigView> {
        let path = fixture.project.read_with(cx, |project, cx| {
            project
                .find_project_path(absolute_path, cx)
                .expect("project path")
        });
        let task = fixture
            .window
            .update(cx, |_, window, cx| {
                fixture.workspace.update(cx, |workspace, cx| {
                    workspace.open_path(path, None, true, window, cx)
                })
            })
            .expect("open canvas");
        let view = task
            .await
            .expect("canvas opened")
            .downcast::<FigView>()
            .expect("canvas item");
        cx.run_until_parked();
        assert!(view.read_with(cx, |view, cx| view.item.read(cx).has_ready_document()));
        view
    }

    async fn auto_open(fixture: &Fixture, name: &str, cx: &mut TestAppContext) {
        let root = fixture.directory.path().join(name);
        let (worktree, _) = fixture
            .project
            .update(cx, |project, cx| {
                project.find_or_create_worktree(root, true, cx)
            })
            .await
            .expect("project worktree");
        let worktree_id = worktree.read_with(cx, |worktree, _| worktree.id());
        fixture
            .window
            .update(cx, |_, window, cx| {
                fixture.workspace.update(cx, |workspace, cx| {
                    crate::workspace_hooks::open_design_for_worktree(
                        workspace,
                        worktree_id,
                        window,
                        cx,
                    )
                })
            })
            .expect("auto-open task")
            .await
            .expect("auto-open canvas");
        cx.run_until_parked();
    }

    fn active_root(workspace: &Entity<Workspace>, cx: &TestAppContext) -> PathBuf {
        workspace.read_with(cx, |workspace, cx| {
            workspace
                .active_item_as::<FigView>(cx)
                .expect("active canvas")
                .read(cx)
                .item
                .read(cx)
                .project_root()
                .expect("project root")
                .to_path_buf()
        })
    }

    #[gpui::test]
    async fn canvas_session_save_as_restores_destination_and_page_without_focus_theft(
        cx: &mut TestAppContext,
    ) {
        let fixture = fixture(cx).await;
        let view = open(&fixture, "Original", cx).await;
        view.update(cx, |view, cx| view.select_page(1, cx));
        let destination = fixture.directory.path().join("SavedCopy");
        let (worktree, path) = fixture
            .project
            .update(cx, |project, cx| {
                project.find_or_create_worktree(destination.clone(), true, cx)
            })
            .await
            .expect("destination worktree");
        let path = ProjectPath {
            worktree_id: worktree.read_with(cx, |worktree, _| worktree.id()),
            path,
        };
        fixture
            .window
            .update(cx, |_, window, cx| {
                view.update(cx, |view, cx| {
                    Item::save_as(view, fixture.project.clone(), path, window, cx)
                })
            })
            .expect("Save As")
            .await
            .expect("save copied project");
        fixture
            .file_system
            .insert_tree_from_real_fs(&destination, &destination)
            .await;
        cx.run_until_parked();
        let item = view.read_with(cx, |view, _| view.item.clone());
        item.update(cx, |item, cx| {
            item.apply(
                Operation::SetName {
                    id: fixture.pages[1],
                    old: "Page 2".into(),
                    new: "Edited in copied project".into(),
                },
                cx,
            )
            .expect("edit copied page");
            item.save(SaveKind::Explicit, cx)
        })
        .await
        .expect("save copied edit");
        fixture
            .file_system
            .insert_tree_from_real_fs(&destination, &destination)
            .await;
        cx.run_until_parked();
        fixture
            .window
            .update(cx, |_, window, cx| {
                fixture.workspace.update(cx, |workspace, cx| {
                    view.update(cx, |view, cx| {
                        view.serialize(workspace, cx.entity_id().as_u64(), false, window, cx)
                    })
                })
            })
            .expect("serialize view")
            .expect("serializable canvas")
            .await
            .expect("persist canvas identity");
        for page_index in [0, 1] {
            view.update(cx, |view, cx| view.select_page(page_index, cx));
            cx.run_until_parked();
            cx.executor()
                .advance_clock(workspace::SERIALIZATION_THROTTLE_TIME * 2);
            cx.run_until_parked();
            let (_, _, scope) = cx
                .update(|cx| FigViewerDb::global(cx))
                .get_view(view.entity_id().as_u64(), fixture.workspace_id)
                .expect("read automatically persisted scope")
                .expect("saved view");
            assert_eq!(
                serde_json::from_str::<Option<SavedScope>>(&scope)
                    .expect("saved scope")
                    .map(FigScope::from),
                Some(FigScope::Page(fixture.pages[page_index])),
                "clean page navigation must update the saved view"
            );
        }
        fixture
            .window
            .update(cx, |_, window, cx| {
                fixture.workspace.update(cx, |workspace, cx| {
                    workspace.flush_serialization(window, cx)
                })
            })
            .expect("flush session")
            .await;
        let app_state = fixture
            .workspace
            .read_with(cx, |workspace, _| workspace.app_state().clone());
        let weak_item = item.downgrade();
        let directory = fixture.directory;
        fixture
            .window
            .update(cx, |_, window, _| window.remove_window())
            .expect("close old window");
        drop(view);
        drop(item);
        cx.update(|_| drop(fixture.workspace));
        cx.run_until_parked();
        assert!(
            weak_item.upgrade().is_none(),
            "restore must reload the copied project from disk"
        );
        let restored_window = cx
            .update(|cx| workspace::open_workspace_by_id(fixture.workspace_id, app_state, None, cx))
            .await
            .expect("restore saved workspace");
        let restored = restored_window
            .update(cx, |window, _, _| window.workspace().clone())
            .expect("restored workspace");
        cx.run_until_parked();
        assert_eq!(active_root(&restored, cx), destination);
        let restored_view = restored.read_with(cx, |workspace, cx| {
            workspace
                .active_item_as::<FigView>(cx)
                .expect("restored canvas")
        });
        restored_view.read_with(cx, |view, cx| {
            assert_eq!(view.scope, Some(FigScope::Page(fixture.pages[1])));
            assert_eq!(
                view.item
                    .read(cx)
                    .doc()
                    .expect("restored doc")
                    .scene
                    .get(fixture.pages[1])
                    .expect("copied page")
                    .name,
                "Edited in copied project"
            );
        });
        let project = restored.read_with(cx, |workspace, _| workspace.project().clone());
        let original = directory.path().join("Original");
        let (worktree, _) = project
            .update(cx, |project, cx| {
                project.find_or_create_worktree(original.clone(), true, cx)
            })
            .await
            .expect("restore original worktree");
        let worktree_id = worktree.read_with(cx, |worktree, _| worktree.id());
        restored_window
            .update(cx, |_, window, cx| {
                restored.update(cx, |workspace, cx| {
                    crate::workspace_hooks::open_design_for_worktree(
                        workspace,
                        worktree_id,
                        window,
                        cx,
                    )
                })
            })
            .expect("late automatic open")
            .await
            .expect("late original canvas");
        assert_eq!(
            active_root(&restored, cx),
            destination,
            "a late original-project open must not hide the saved copy"
        );
        let original_path = project.read_with(cx, |project, cx| {
            project
                .find_project_path(original.join("fanta.json"), cx)
                .expect("original path")
        });
        restored_window
            .update(cx, |_, window, cx| {
                restored.update(cx, |workspace, cx| {
                    workspace.open_path(original_path, None, true, window, cx)
                })
            })
            .expect("explicit Open")
            .await
            .expect("open original explicitly");
        assert_eq!(
            active_root(&restored, cx),
            original,
            "explicit Open still activates its target"
        );
    }

    #[gpui::test]
    async fn canvas_session_save_as_preserves_two_page_tabs_and_active_scope(
        cx: &mut TestAppContext,
    ) {
        let fixture = fixture(cx).await;
        let original = fixture.directory.path().join("Original");
        let mut views = Vec::new();
        for page in fixture.pages {
            let source = fanta_format::locate_page_source(&original, page).expect("page source");
            views.push(open_path(&fixture, source, cx).await);
        }
        let view = views.last().expect("active second page");
        let item = view.read_with(cx, |view, _| view.item.clone());
        assert!(views.iter().all(|view| {
            view.read_with(cx, |view, _| view.item.entity_id()) == item.entity_id()
        }));
        let destination = fixture.directory.path().join("SavedCopy");
        let (worktree, path) = fixture
            .project
            .update(cx, |project, cx| {
                project.find_or_create_worktree(destination.clone(), true, cx)
            })
            .await
            .expect("destination worktree");
        let path = ProjectPath {
            worktree_id: worktree.read_with(cx, |worktree, _| worktree.id()),
            path,
        };
        fixture
            .window
            .update(cx, |_, window, cx| {
                view.update(cx, |view, cx| {
                    Item::save_as(view, fixture.project.clone(), path, window, cx)
                })
            })
            .expect("Save As")
            .await
            .expect("save copied project");
        fixture
            .file_system
            .insert_tree_from_real_fs(&destination, &destination)
            .await;
        cx.run_until_parked();
        assert!(
            views
                .iter()
                .all(|view| { view.read_with(cx, |view, _| view.opened_entry_id.is_none()) })
        );
        cx.executor()
            .advance_clock(workspace::SERIALIZATION_THROTTLE_TIME * 2);
        cx.run_until_parked();
        fixture
            .window
            .update(cx, |_, window, cx| {
                fixture.workspace.update(cx, |workspace, cx| {
                    workspace.flush_serialization(window, cx)
                })
            })
            .expect("flush copied session")
            .await;
        let app_state = fixture
            .workspace
            .read_with(cx, |workspace, _| workspace.app_state().clone());
        let _directory = fixture.directory;
        let weak_views = views.iter().map(Entity::downgrade).collect::<Vec<_>>();
        let weak_workspace = fixture.workspace.downgrade();
        fixture
            .window
            .update(cx, |_, window, _| window.remove_window())
            .expect("close original window");
        drop(views);
        drop(item);
        cx.update(|_| {
            drop(fixture.workspace);
            drop(fixture.project);
        });
        cx.run_until_parked();
        assert!(weak_workspace.upgrade().is_none(), "old workspace retained");
        assert!(
            weak_views.iter().all(|view| view.upgrade().is_none()),
            "old scoped view retained"
        );
        let restored_window = cx
            .update(|cx| workspace::open_workspace_by_id(fixture.workspace_id, app_state, None, cx))
            .await
            .expect("restore copied workspace");
        let restored = restored_window
            .update(cx, |window, _, _| window.workspace().clone())
            .expect("restored workspace");
        cx.run_until_parked();
        restored.read_with(cx, |workspace, cx| {
            let views = workspace.items_of_type::<FigView>(cx).collect::<Vec<_>>();
            assert_eq!(views.len(), 2, "Save As must preserve both scoped tabs");
            let scopes = views
                .iter()
                .map(|view| view.read(cx).scope)
                .collect::<Vec<_>>();
            for page in fixture.pages {
                assert!(scopes.contains(&Some(FigScope::Page(page))));
            }
            let entries = views
                .iter()
                .map(|view| {
                    let view = view.read(cx);
                    assert_eq!(
                        view.item.read(cx).project_root(),
                        Some(destination.as_path())
                    );
                    let Some(FigScope::Page(page)) = view.scope else {
                        panic!("restored page scope");
                    };
                    let source = fanta_format::locate_page_source(&destination, page)
                        .expect("copied page source");
                    let project = workspace.project().read(cx);
                    let path = project
                        .find_project_path(&source, cx)
                        .expect("copied source path");
                    let entry = project
                        .entry_for_path(&path, cx)
                        .expect("copied source indexed");
                    assert_eq!(
                        view.opened_entry_id,
                        Some(entry.id),
                        "restored source {source:?}"
                    );
                    entry.id
                })
                .collect::<std::collections::HashSet<_>>();
            assert_eq!(
                entries.len(),
                2,
                "tabs need distinct deduplication identities"
            );
            let active = workspace
                .active_item_as::<FigView>(cx)
                .expect("active canvas");
            assert_eq!(
                active.read(cx).scope,
                Some(FigScope::Page(fixture.pages[1]))
            );
            assert_eq!(
                active
                    .read(cx)
                    .item
                    .read(cx)
                    .doc()
                    .expect("document")
                    .active_page(),
                Some(fixture.pages[1]),
                "the restored active tab must apply its own page"
            );
        });
    }

    #[gpui::test]
    async fn canvas_session_immediate_flush_restores_latest_clean_page(cx: &mut TestAppContext) {
        let fixture = fixture(cx).await;
        let view = open(&fixture, "Original", cx).await;
        view.update(cx, |view, cx| view.select_page(0, cx));
        fixture
            .window
            .update(cx, |_, window, cx| {
                fixture.workspace.update(cx, |workspace, cx| {
                    view.update(cx, |view, cx| {
                        view.serialize(workspace, cx.entity_id().as_u64(), false, window, cx)
                    })
                })
            })
            .expect("initial serialization")
            .expect("serializable canvas")
            .await
            .expect("persist initial page");
        cx.run_until_parked();
        view.update(cx, |view, cx| view.select_page(1, cx));
        cx.run_until_parked();
        assert!(!view.read_with(cx, |view, cx| view.is_dirty(cx)));
        // Quit flushes immediately, without waiting for the item queue's
        // serialization throttle to expire.
        fixture
            .window
            .update(cx, |_, window, cx| {
                fixture.workspace.update(cx, |workspace, cx| {
                    workspace.flush_serialization(window, cx)
                })
            })
            .expect("immediate session flush")
            .await;
        let restored = fixture
            .window
            .update(cx, |_, window, cx| {
                FigView::deserialize(
                    fixture.project.clone(),
                    fixture.workspace.downgrade(),
                    fixture.workspace_id,
                    view.entity_id().as_u64(),
                    window,
                    cx,
                )
            })
            .expect("restore immediately flushed canvas")
            .await
            .expect("restored canvas");
        assert_eq!(
            restored.read_with(cx, |view, _| view.scope),
            Some(FigScope::Page(fixture.pages[1])),
            "Quit must persist the latest clean navigation before returning"
        );
    }

    #[gpui::test]
    async fn canvas_session_auto_open_activates_first_project_and_preserves_existing_focus(
        cx: &mut TestAppContext,
    ) {
        let fixture = fixture(cx).await;
        auto_open(&fixture, "Original", cx).await;
        assert_eq!(
            active_root(&fixture.workspace, cx),
            fixture.directory.path().join("Original")
        );
        auto_open(&fixture, "Other", cx).await;
        auto_open(&fixture, "Original", cx).await;
        assert_eq!(
            active_root(&fixture.workspace, cx),
            fixture.directory.path().join("Original")
        );
        assert_eq!(
            fixture.workspace.read_with(cx, |workspace, cx| workspace
                .items_of_type::<FigView>(cx)
                .count()),
            2
        );
    }

    #[gpui::test]
    async fn canvas_session_dirty_closing_never_claims_to_preserve_unsaved_content(
        cx: &mut TestAppContext,
    ) {
        let fixture = fixture(cx).await;
        let view = open(&fixture, "Original", cx).await;
        let serialize = |cx: &mut TestAppContext| {
            fixture
                .window
                .update(cx, |_, window, cx| {
                    fixture.workspace.update(cx, |workspace, cx| {
                        view.update(cx, |view, cx| {
                            view.serialize(workspace, cx.entity_id().as_u64(), true, window, cx)
                        })
                    })
                })
                .expect("closing serialization")
        };
        serialize(cx)
            .expect("clean canvas can restore its path")
            .await
            .expect("save clean path");
        let item = view.read_with(cx, |view, _| view.item.clone());
        let code = view.read_with(cx, |view, _| view.code_workspace.clone());
        fixture
            .window
            .update(cx, |_, window, cx| {
                code.update(cx, |code, cx| {
                    code.refresh_page(Some(fixture.pages[0]), window, cx);
                })
            })
            .expect("load source editor");
        cx.run_until_parked();
        let source = fanta_format::locate_page_source(
            &fixture.directory.path().join("Original"),
            fixture.pages[0],
        )
        .expect("page source");
        let buffer = fixture
            .project
            .update(cx, |project, cx| project.open_local_buffer(&source, cx))
            .await
            .expect("source buffer");
        buffer.update(cx, |buffer, cx| {
            buffer.edit([(0..0, "<unfinished")], None, cx)
        });
        cx.run_until_parked();
        assert!(code.read_with(cx, |code, cx| code.source_is_dirty(cx)));
        assert!(
            !item.read_with(cx, |item, _| item.is_dirty()),
            "the unsaved draft belongs to the source editor"
        );
        assert!(
            serialize(cx).is_none(),
            "unsaved source must retain the normal close prompt"
        );
        code.update(cx, |code, cx| code.discard_source_edit(cx))
            .await
            .expect("discard source draft");
        cx.run_until_parked();
        item.update(cx, |item, cx| {
            item.apply(
                Operation::SetName {
                    id: fixture.pages[0],
                    old: "Page 1".into(),
                    new: "Unsaved content".into(),
                },
                cx,
            )
        })
        .expect("unsaved canvas edit");
        assert!(
            serialize(cx).is_none(),
            "path-only restore cannot preserve canvas edits"
        );
        assert!(item.read_with(cx, |item, _| item.is_dirty()));
    }

    #[gpui::test]
    async fn canvas_session_concurrent_scoped_tabs_keep_their_own_source_entries(
        cx: &mut TestAppContext,
    ) {
        let fixture = fixture(cx).await;
        let root = fixture.directory.path().join("Original");
        let database = cx.update(|cx| FigViewerDb::global(cx));
        let mut sources = Vec::new();
        for (index, page) in fixture.pages.iter().enumerate() {
            let source = fanta_format::locate_page_source(&root, *page).expect("page source");
            database
                .save_view(
                    index as u64,
                    fixture.workspace_id,
                    root.join("fanta.json"),
                    Some(source.clone()),
                    serde_json::to_string(&Some(SavedScope::Page(*page))).expect("scope"),
                )
                .await
                .expect("persist scoped tab");
            sources.push(source);
        }
        let restores = fixture
            .window
            .update(cx, |_, window, cx| {
                (0..2)
                    .map(|index| {
                        FigView::deserialize(
                            fixture.project.clone(),
                            fixture.workspace.downgrade(),
                            fixture.workspace_id,
                            index,
                            window,
                            cx,
                        )
                    })
                    .collect::<Vec<_>>()
            })
            .expect("concurrent restores");
        let views = futures::future::join_all(restores).await;
        cx.run_until_parked();
        for (index, result) in views.into_iter().enumerate() {
            let view = result.expect("restored scoped tab");
            let entry = fixture.project.read_with(cx, |project, cx| {
                let path = project
                    .find_project_path(&sources[index], cx)
                    .expect("source path");
                project.entry_for_path(&path, cx).expect("source entry").id
            });
            view.read_with(cx, |view, _| {
                assert_eq!(view.scope, Some(FigScope::Page(fixture.pages[index])));
                assert_eq!(
                    view.opened_entry_id,
                    Some(entry),
                    "each tab must retain its own deduplication identity"
                );
            });
        }
    }

    #[gpui::test]
    async fn canvas_session_missing_project_returns_restore_error(cx: &mut TestAppContext) {
        let fixture = fixture(cx).await;
        cx.update(|cx| FigViewerDb::global(cx))
            .save_view(
                123,
                fixture.workspace_id,
                fixture.directory.path().join("Missing/fanta.json"),
                None,
                "null".into(),
            )
            .await
            .expect("store missing project path");
        let restore = fixture
            .window
            .update(cx, |_, window, cx| {
                FigView::deserialize(
                    fixture.project.clone(),
                    fixture.workspace.downgrade(),
                    fixture.workspace_id,
                    123,
                    window,
                    cx,
                )
            })
            .expect("restore task");
        let error = restore
            .await
            .expect_err("missing design must propagate an error");
        assert!(error.to_string().contains("saved design is missing"));
    }
}
