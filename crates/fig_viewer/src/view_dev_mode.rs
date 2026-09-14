use super::*;

impl FigView {
    pub(crate) fn is_dev_mode(&self, cx: &App) -> bool {
        self.editor_mode(cx) == EditorMode::Dev
    }

    pub(super) fn is_design_canvas_mode(&self, cx: &App) -> bool {
        matches!(self.editor_mode(cx), EditorMode::Design | EditorMode::Dev)
    }

    pub(super) fn refuse_dev_transition(&self, cx: &mut Context<Self>) -> bool {
        if self.has_pending_authoring(cx)
            || self.comment_state.draft.is_some()
            || self.has_unsent_comment_reply(cx)
        {
            show_canvas_notice_deferred(
                "Finish or cancel the current edit before changing the Dev session. Its draft was kept.".into(),
                cx,
            );
            return true;
        }
        false
    }

    pub(super) fn set_dev_mode_boundary(&mut self, mode: EditorMode, cx: &mut Context<Self>) {
        if self.editor_mode(cx) == mode || self.refuse_dev_transition(cx) {
            return;
        }
        if mode == EditorMode::Dev
            && (self.item.read(cx).doc().is_none()
                || self.prototype_player.is_some()
                || self.editor_workspace(cx) == EditorWorkspace::Variables)
        {
            show_canvas_notice_deferred(
                "Return to a loaded canvas before entering Dev.".into(),
                cx,
            );
            return;
        }
        self.invalidate_local_media_origin();
        self.comment_state.clear_pending_motion_anchor();
        self.comment_state.close_thread();
        self.measurement_selection = None;
        self.annotation_state.selection = None;
        self.set_motion_auto_keyframe_state(false, cx);
        self.timeline_shell
            .update(cx, |timeline, cx| timeline.pause(cx));
        self.editor_session.update(cx, |session, cx| {
            session.set_mode(mode, cx);
            if mode == EditorMode::Dev {
                session.set_workspace(EditorWorkspace::Canvas, cx);
            }
        });
        let tool = if mode == EditorMode::Dev {
            ToolKind::Inspect
        } else {
            ToolKind::Select
        };
        self.tools.activate_without_context(tool);
        self.remember_tool_face(tool);
        self.hovered_node = None;
        self.hover_resize_handle = None;
        self.inspector_sidebar_visible = true;
        self.sync_measurement_edit_barrier(cx);
        self.sync_motion_timeline(cx);
        self.invalidate_canvas_cache();
        cx.notify();
    }

    pub(super) fn set_dev_workspace(&mut self, workspace: EditorWorkspace, cx: &mut Context<Self>) {
        if self.editor_workspace(cx) == workspace || self.refuse_dev_transition(cx) {
            return;
        }
        if workspace == EditorWorkspace::Variables {
            show_canvas_notice_deferred("Switch to Design to work with variables.".into(), cx);
            return;
        }
        self.invalidate_local_media_origin();
        self.measurement_selection = None;
        self.annotation_state.selection = None;
        self.tools.activate_without_context(ToolKind::Inspect);
        self.remember_tool_face(ToolKind::Inspect);
        self.editor_session
            .update(cx, |session, cx| session.set_workspace(workspace, cx));
        self.hovered_node = None;
        self.hover_resize_handle = None;
        self.sync_measurement_edit_barrier(cx);
        self.invalidate_canvas_cache();
        cx.notify();
    }

    pub(super) fn dev_history_allowed(&self, redo: bool, cx: &Context<Self>) -> bool {
        if self.is_inspecting()
            || self.has_pending_authoring(cx)
            || self.comment_state.draft.is_some()
            || self.has_unsent_comment_reply(cx)
        {
            return false;
        }
        let Some(page) = self.mark_page(cx) else {
            return false;
        };
        let item = self.item.read(cx);
        if !item.is_editable()
            || !item.can_preview_for_owner(cx.entity_id())
            || item.content_preview_active()
        {
            return false;
        }
        let Some(doc) = item.doc() else { return false };
        let transaction = if redo {
            doc.history.next_redo_transaction()
        } else {
            doc.history.next_undo_transaction()
        };
        transaction.is_some_and(|transaction| {
            crate::dev_history::transaction_is_dev_mark_edit(doc, page, transaction)
        })
    }

    pub(super) fn render_native_dev_toolbar(&self, cx: &mut Context<Self>) -> AnyElement {
        let tools = [
            ToolKind::Inspect,
            ToolKind::Hand,
            ToolKind::Measure,
            ToolKind::Annotation,
        ];
        h_flex()
            .debug_selector(|| "fanta-canvas-toolbar".to_owned())
            .absolute()
            .bottom(px(12.))
            .left_0()
            .right_0()
            .px_3()
            .justify_center()
            .child(
                h_flex()
                    .id("fanta-native-dev-toolbar")
                    .track_focus(&self.native_toolbar_focus)
                    .occlude()
                    .flex_wrap()
                    .gap_1()
                    .p_1p5()
                    .rounded_xl()
                    .border_1()
                    .border_color(cx.theme().colors().border)
                    .elevation_2(cx)
                    .children(tools.into_iter().enumerate().map(|(index, tool)| {
                        div()
                            .debug_selector(move || format!("native-dev-tool-{index}"))
                            .child(
                                IconButton::new(("native-dev-tool", index), tool.icon())
                                    .toggle_state(self.tools.kind() == tool)
                                    .disabled(
                                        matches!(tool, ToolKind::Measure | ToolKind::Annotation)
                                            && (!self.item.read(cx).is_editable()
                                                || self.mark_page(cx).is_none()),
                                    )
                                    .tooltip(Tooltip::text(tool.label()))
                                    .on_click(cx.listener(move |view, _, window, cx| {
                                        view.activate_tool_from_action(tool, window, cx)
                                    })),
                            )
                    }))
                    .child(div().debug_selector(|| "native-dev-code".to_owned()).child(
                        Button::new("native-dev-code", "Saved Code").on_click(cx.listener(
                            |view, _, _, cx| view.set_editor_workspace(EditorWorkspace::Code, cx),
                        )),
                    ))
                    .child(Button::new("native-dev-ready", "Readiness unavailable").disabled(true))
                    .child(
                        Button::new("native-dev-exit", "Design").on_click(cx.listener(
                            |view, _, window, cx| {
                                view.set_editor_mode_from_action(EditorMode::Design, window, cx)
                            },
                        )),
                    ),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::annotations::{DeveloperAnnotation, create_annotation_op};
    use fanta_doc::{Color as DocumentColor, GroupNode, Transform2D, VectorNode};
    use fs::FakeFs;
    use gpui::{TestAppContext, point, size};

    async fn fixture(
        cx: &mut TestAppContext,
    ) -> (Entity<Project>, Entity<FigItem>, NodeId, NodeId) {
        cx.update(|cx| {
            zlog::init_test();
            assets::Assets.load_test_fonts(cx);
            let store = settings::SettingsStore::test(cx);
            cx.set_global(store);
            theme_settings::init(theme::LoadThemes::JustBase, cx);
            release_channel::init(semver::Version::new(0, 0, 0), cx);
            editor::init(cx);
        });
        let project = Project::test(FakeFs::new(cx.executor()), [], cx).await;
        let mut doc = Doc::new();
        let page = CanvasNode::new(NodeData::Group(GroupNode::default()));
        let page_id = page.id;
        doc.apply(Operation::create_node(page)).expect("page");
        doc.add_page(page_id);
        doc.set_active_page(Some(page_id));
        let mut vector = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
            0.,
            0.,
            100.,
            100.,
            DocumentColor::BLACK,
        )));
        vector.parent = Some(page_id);
        let vector_id = vector.id;
        doc.apply(Operation::create_node(vector)).expect("artwork");
        doc.selection.select_only(vector_id);
        doc.history = Default::default();
        let item =
            crate::document::ready_item_for_test(&project, "/tmp/Dev-session.fig".into(), doc, cx);
        (project, item, page_id, vector_id)
    }

    fn activate_canvas(view: &Entity<FigView>, cx: &mut gpui::VisualTestContext) {
        cx.update(|window, cx| {
            window.activate_window();
            let bindings = settings::KeymapFile::load_asset_allow_partial_failure(
                settings::DEFAULT_KEYMAP_PATH,
                cx,
            )
            .expect("shipped bindings");
            cx.bind_keys(bindings);
        });
        cx.simulate_resize(size(px(1400.), px(1000.)));
        cx.run_until_parked();
        view.update_in(cx, |view, window, cx| {
            view.select_page(0, cx);
            view.focus_handle.focus(window, cx);
        });
        cx.run_until_parked();
    }

    fn visible_native_mode_tab(
        cx: &mut gpui::VisualTestContext,
        selector: &'static str,
    ) -> Bounds<Pixels> {
        // Newly mounted tabs reserve their expanded widths during the 150 ms
        // collapse animation, so a debug bound can temporarily lie outside
        // the clipped inspector after returning from Code.
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(150));
        cx.run_until_parked();
        cx.update(|window, cx| {
            window.refresh();
            window.draw(cx).clear();
        });
        let sidebar = cx
            .debug_bounds("fanta-inspector-sidebar")
            .expect("visible inspector");
        let tab = cx.debug_bounds(selector).expect("mode tab");
        assert!(
            tab.is_contained_within(&sidebar),
            "{selector} must be fully visible: {tab:?} in {sidebar:?}"
        );
        tab
    }

    fn snapshot(item: &Entity<FigItem>, cx: &mut gpui::VisualTestContext) -> serde_json::Value {
        item.read_with(cx, |item, _| {
            let doc = item.doc().expect("doc");
            serde_json::json!({
                "document": doc,
                "history": doc.history,
                "dirty": item.is_dirty(),
            })
        })
    }

    #[gpui::test]
    async fn dev_native_entry_shortcut_tools_code_and_artwork_barriers(cx: &mut TestAppContext) {
        let (project, item, _, vector) = fixture(cx).await;
        let (view, cx) = cx
            .add_window_view(|window, cx| FigView::new(item.clone(), project.clone(), window, cx));
        activate_canvas(&view, cx);
        let baseline = snapshot(&item, cx);
        cx.simulate_keystrokes("shift-d");
        cx.run_until_parked();
        view.read_with(cx, |view, cx| {
            assert_eq!(view.editor_mode(cx), EditorMode::Dev);
            assert_eq!(view.tools.kind(), ToolKind::Inspect);
            assert!(view.is_art_read_only(cx));
            assert!(!view.can_edit_annotations(cx));
            assert!(!view.can_edit_measurements(cx));
        });
        for key in [
            "shift-d",
            "v",
            "r",
            "p",
            "t",
            "delete",
            "backspace",
            "right",
            "enter",
        ] {
            cx.simulate_keystrokes(key);
            cx.run_until_parked();
            assert_eq!(
                snapshot(&item, cx),
                baseline,
                "Dev artwork barrier for {key}"
            );
            view.read_with(cx, |view, cx| {
                assert_eq!(view.editor_mode(cx), EditorMode::Dev)
            });
        }
        let hand = cx
            .debug_bounds("native-dev-tool-1")
            .expect("visible Hand control");
        cx.simulate_click(hand.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        view.read_with(cx, |view, cx| {
            assert_eq!(view.tools.kind(), ToolKind::Hand);
            assert!(!view.is_editable(cx));
        });
        view.update_in(cx, |view, window, cx| {
            view.delete_selection(&DeleteSelection, window, cx);
            view.duplicate_selection(&DuplicateSelection, window, cx);
            view.group_selection(&GroupSelection, window, cx);
            view.cut_selection(&CutSelection, window, cx);
            view.paste_selection(&PasteSelection, window, cx);
            view.play_prototype(&PlayPrototype, window, cx);
            assert!(view.prototype_player.is_none());
            view.set_editor_workspace(EditorWorkspace::Variables, cx);
            assert_eq!(view.editor_workspace(cx), EditorWorkspace::Canvas);
        });
        assert_eq!(snapshot(&item, cx), baseline);
        let code = cx
            .debug_bounds("native-dev-code")
            .expect("visible Saved Code control");
        cx.simulate_click(code.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        view.read_with(cx, |view, cx| {
            assert_eq!(view.editor_workspace(cx), EditorWorkspace::Code);
            assert_eq!(view.editor_mode(cx), EditorMode::Dev);
        });
        let canvas = cx
            .debug_bounds("fanta-collapsible-tab-fanta-editor-workspace-0")
            .expect("Canvas tab");
        cx.simulate_click(canvas.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        view.read_with(cx, |view, cx| {
            assert_eq!(view.editor_workspace(cx), EditorWorkspace::Canvas)
        });
        let design = visible_native_mode_tab(cx, "fanta-collapsible-tab-fanta-editor-mode-0");
        cx.simulate_click(design.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        view.read_with(cx, |view, cx| {
            assert_eq!(view.editor_mode(cx), EditorMode::Design);
            assert_eq!(view.tools.kind(), ToolKind::Select);
            assert!(view.is_editable(cx));
        });
        let dev = visible_native_mode_tab(cx, "fanta-collapsible-tab-fanta-editor-mode-4");
        cx.simulate_click(dev.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        assert_eq!(snapshot(&item, cx), baseline);
        let sibling =
            cx.update(|window, cx| cx.new(|cx| FigView::new(item.clone(), project, window, cx)));
        cx.run_until_parked();
        sibling.update_in(cx, |sibling, _, cx| {
            assert!(sibling.is_editable(cx), "Dev never locks the shared item");
            sibling.duplicate_selected_nodes(cx);
        });
        view.read_with(cx, |view, cx| {
            assert_eq!(view.editor_mode(cx), EditorMode::Dev);
            assert!(!view.is_editable(cx));
        });
        item.read_with(cx, |item, _| {
            assert!(item.is_editable());
            assert!(item.doc().expect("doc").scene.contains(vector));
            assert_eq!(item.doc().expect("doc").history.undo_depth(), 1);
        });
    }

    #[gpui::test]
    async fn dev_numeric_draft_refuses_mode_tab_without_committing(cx: &mut TestAppContext) {
        for invalid in [false, true] {
            let (project, item, _, _) = fixture(cx).await;
            let (view, cx) =
                cx.add_window_view(|window, cx| FigView::new(item.clone(), project, window, cx));
            activate_canvas(&view, cx);
            let inspector = view.read_with(cx, |view, _| view.inspector_sidebar.clone());
            let field = cx.debug_bounds("scrub-fanta-x-0").expect("legacy X field");
            cx.simulate_click(field.center(), gpui::Modifiers::none());
            cx.dispatch_action(editor::actions::SelectAll);
            cx.simulate_input("55.5");
            cx.run_until_parked();
            if invalid {
                cx.dispatch_action(editor::actions::SelectAll);
                cx.simulate_input("invalid");
                cx.run_until_parked();
            }
            let before = snapshot(&item, cx);
            let draft = inspector.read_with(cx, |panel, cx| {
                assert!(panel.editing_field.is_some());
                panel.field_editor.read(cx).text(cx)
            });
            let dev = visible_native_mode_tab(cx, "fanta-collapsible-tab-fanta-editor-mode-4");
            cx.simulate_mouse_down(dev.center(), MouseButton::Left, gpui::Modifiers::none());
            cx.run_until_parked();
            inspector.read_with(cx, |panel, cx| {
                assert!(
                    panel.editing_field.is_some(),
                    "mouse-down must preserve the draft before mode admission"
                );
                assert_eq!(panel.field_editor.read(cx).text(cx), draft);
            });
            assert_eq!(snapshot(&item, cx), before);
            cx.simulate_mouse_up(dev.center(), MouseButton::Left, gpui::Modifiers::none());
            cx.run_until_parked();
            assert_eq!(snapshot(&item, cx), before);
            view.read_with(cx, |view, cx| {
                assert_eq!(view.editor_mode(cx), EditorMode::Design)
            });
            inspector.read_with(cx, |panel, cx| {
                assert!(panel.editing_field.is_some());
                assert_eq!(panel.field_editor.read(cx).text(cx), draft);
            });
            assert!(item.read_with(cx, |item, _| item.content_preview_active()));
            inspector.update_in(cx, |panel, window, cx| {
                panel.field_editor.focus_handle(cx).focus(window, cx)
            });
            cx.dispatch_action(editor::actions::SelectAll);
            cx.simulate_input("60");
            cx.simulate_keystrokes("enter");
            cx.run_until_parked();
            view.update_in(cx, |view, window, cx| view.focus_handle.focus(window, cx));
            cx.simulate_keystrokes("shift-d");
            cx.run_until_parked();
            view.read_with(cx, |view, cx| {
                assert_eq!(view.editor_mode(cx), EditorMode::Dev)
            });
        }
    }

    #[gpui::test]
    async fn dev_annotation_editor_typing_and_exit_keep_unsent_draft(cx: &mut TestAppContext) {
        let (project, item, _, _) = fixture(cx).await;
        let (view, cx) =
            cx.add_window_view(|window, cx| FigView::new(item.clone(), project, window, cx));
        activate_canvas(&view, cx);
        cx.simulate_keystrokes("shift-d");
        cx.run_until_parked();
        let annotation = cx
            .debug_bounds("native-dev-tool-3")
            .expect("Annotation control");
        cx.simulate_click(annotation.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        view.update_in(cx, |view, window, cx| {
            assert_eq!(view.tools.kind(), ToolKind::Annotation);
            let bounds = view.container_bounds.expect("mounted canvas");
            view.handle_mouse_down(
                &MouseDownEvent {
                    button: MouseButton::Left,
                    position: bounds.center(),
                    modifiers: gpui::Modifiers::none(),
                    click_count: 1,
                    first_mouse: false,
                },
                window,
                cx,
            );
        });
        cx.run_until_parked();
        cx.simulate_keystrokes("shift-d");
        cx.run_until_parked();
        let draft = view.read_with(cx, |view, cx| {
            let draft = view
                .annotation_state
                .controller
                .draft()
                .expect("unsent draft")
                .clone();
            assert_eq!(draft.annotation().text, "D");
            assert!(view.close_blocker(cx).is_some());
            draft
        });
        let before = snapshot(&item, cx);
        let design = visible_native_mode_tab(cx, "fanta-collapsible-tab-fanta-editor-mode-0");
        cx.simulate_click(design.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        view.update_in(cx, |view, window, cx| {
            view.activate_tool(ToolKind::Hand, cx);
            view.play_prototype(&PlayPrototype, window, cx);
            assert!(view.prototype_player.is_none());
            view.set_editor_workspace(EditorWorkspace::Code, cx);
            assert_eq!(view.editor_mode(cx), EditorMode::Dev);
            assert_eq!(view.editor_workspace(cx), EditorWorkspace::Canvas);
            assert_eq!(view.tools.kind(), ToolKind::Annotation);
            assert_eq!(view.annotation_state.controller.draft(), Some(&draft));
            assert!(view.close_blocker(cx).is_some());
            assert!(view.cancel_annotation(None, cx));
            view.set_editor_mode(EditorMode::Design, cx);
            assert_eq!(view.editor_mode(cx), EditorMode::Design);
            assert!(view.close_blocker(cx).is_none());
        });
        assert_eq!(snapshot(&item, cx), before);
    }

    #[gpui::test]
    async fn dev_history_stops_at_artwork_and_preserves_source_permissions(
        cx: &mut TestAppContext,
    ) {
        let (project, item, page, vector) = fixture(cx).await;
        let (view, cx) =
            cx.add_window_view(|window, cx| FigView::new(item.clone(), project, window, cx));
        activate_canvas(&view, cx);
        item.update(cx, |item, cx| {
            item.with_document(cx, |document| {
                let node = document.doc.scene.get(vector).expect("vector");
                let old = node.transform;
                let new = Transform2D::translation(25., 35.);
                document
                    .doc
                    .apply(Operation::SetTransform {
                        id: vector,
                        old,
                        new,
                    })
                    .expect("artwork edit");
                let annotation =
                    DeveloperAnnotation::new([30., 50.], "Saved note".into(), "QA".into(), 10)
                        .expect("note");
                let operation =
                    create_annotation_op(&document.doc, page, &annotation).expect("note operation");
                document.doc.apply(operation).expect("add note");
                ((), DocChange::Content)
            });
        });
        view.update_in(cx, |view, window, cx| {
            view.set_editor_mode(EditorMode::Dev, cx);
            view.undo(&Undo, window, cx);
            assert_eq!(
                view.item.read(cx).doc().expect("doc").history.undo_depth(),
                2,
                "Inspect does not change marks"
            );
            view.activate_tool(ToolKind::Measure, cx);
            view.undo(&Undo, window, cx);
            assert_eq!(
                view.item.read(cx).doc().expect("doc").history.undo_depth(),
                1
            );
        });
        let art = snapshot(&item, cx);
        view.update_in(cx, |view, window, cx| view.undo(&Undo, window, cx));
        assert_eq!(
            snapshot(&item, cx),
            art,
            "refused art Undo leaves both stacks and scene intact"
        );
        view.update_in(cx, |view, window, cx| {
            view.redo(&Redo, window, cx);
            assert_eq!(
                view.item.read(cx).doc().expect("doc").history.undo_depth(),
                2
            );
        });
        let split = view
            .update_in(cx, |view, window, cx| view.clone_on_split(None, window, cx))
            .await
            .expect("split view");
        cx.run_until_parked();
        split.read_with(cx, |view, cx| {
            assert_eq!(view.editor_mode(cx), EditorMode::Dev);
            assert_eq!(view.tools.kind(), ToolKind::Inspect);
            assert!(!view.is_editable(cx));
        });
        item.update(cx, |item, cx| item.set_source_edit_locked(true, cx));
        cx.run_until_parked();
        view.update_in(cx, |view, window, cx| {
            view.undo(&Undo, window, cx);
            view.set_editor_mode(EditorMode::Design, cx);
            assert!(!view.is_editable(cx));
            view.set_editor_mode(EditorMode::Dev, cx);
            assert_eq!(
                view.editor_mode(cx),
                EditorMode::Dev,
                "locked source remains inspectable"
            );
            view.activate_tool(ToolKind::Annotation, cx);
            assert_eq!(view.tools.kind(), ToolKind::Inspect);
            assert_eq!(
                view.item.read(cx).doc().expect("doc").history.undo_depth(),
                2
            );
        });
    }

    #[gpui::test]
    async fn dev_measurement_pointer_commit_keeps_art_and_inspect_cannot_delete_it(
        cx: &mut TestAppContext,
    ) {
        let (project, item, page, vector) = fixture(cx).await;
        item.update(cx, |item, cx| {
            item.with_document(cx, |document| {
                document.doc.scene.get_mut(page).expect("page").transform =
                    Transform2D::translation(75., 35.);
                ((), DocChange::Selection)
            })
        });
        let (view, cx) =
            cx.add_window_view(|window, cx| FigView::new(item.clone(), project, window, cx));
        activate_canvas(&view, cx);
        let original = item.read_with(cx, |item, _| {
            item.doc()
                .expect("doc")
                .scene
                .get(vector)
                .expect("art")
                .clone()
        });
        cx.simulate_keystrokes("shift-d");
        cx.run_until_parked();
        let measure = cx
            .debug_bounds("native-dev-tool-2")
            .expect("Measurement control");
        cx.simulate_click(measure.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        let start = view.read_with(cx, |view, _| {
            view.container_bounds.expect("canvas").center()
        });
        let end = start + point(px(100.), px(40.));
        cx.simulate_mouse_down(start, MouseButton::Left, gpui::Modifiers::none());
        cx.simulate_mouse_move(end, Some(MouseButton::Left), gpui::Modifiers::none());
        cx.run_until_parked();
        view.read_with(cx, |view, cx| {
            assert!(view.measurement_controller.has_pending_authoring());
            assert!(view.page_measurements(cx).is_empty());
        });
        cx.simulate_mouse_up(end, MouseButton::Left, gpui::Modifiers::none());
        cx.run_until_parked();
        view.read_with(cx, |view, cx| {
            assert_eq!(view.editor_mode(cx), EditorMode::Dev);
            assert!(!view.is_editable(cx));
            assert!(!view.measurement_controller.has_pending_authoring());
            assert_eq!(view.page_measurements(cx).len(), 1);
            assert!(view.selected_measurement(cx).is_some());
        });
        let committed = snapshot(&item, cx);
        cx.simulate_keystrokes("v");
        cx.run_until_parked();
        cx.simulate_keystrokes("delete");
        cx.run_until_parked();
        assert_eq!(snapshot(&item, cx), committed);
        view.update_in(cx, |view, window, cx| {
            assert_eq!(view.tools.kind(), ToolKind::Inspect);
            view.activate_tool(ToolKind::Measure, cx);
            view.undo(&Undo, window, cx);
            assert!(view.page_measurements(cx).is_empty());
            view.redo(&Redo, window, cx);
            assert_eq!(view.page_measurements(cx).len(), 1);
        });
        item.read_with(cx, |item, _| {
            assert_eq!(item.doc().expect("doc").scene.get(vector), Some(&original));
            assert_eq!(item.doc().expect("doc").history.undo_depth(), 1);
        });
    }

    #[gpui::test]
    async fn dev_existing_comments_offer_read_and_copy_without_reply_editor(
        cx: &mut TestAppContext,
    ) {
        let (project, item, page, _) = fixture(cx).await;
        let thread = item.update(cx, |item, cx| {
            item.with_document(cx, |document| {
                let (id, operation) = crate::comments::add_comment_op(
                    &document.doc,
                    page,
                    [50., 50.],
                    "Keep this discussion",
                )
                .expect("comment");
                document.doc.apply(operation).expect("add comment");
                (id, DocChange::Content)
            })
            .expect("document")
        });
        let (view, cx) =
            cx.add_window_view(|window, cx| FigView::new(item.clone(), project, window, cx));
        activate_canvas(&view, cx);
        let before = snapshot(&item, cx);
        cx.simulate_keystrokes("shift-d");
        cx.run_until_parked();
        view.update_in(cx, |view, window, cx| {
            view.show_comment_thread(thread.clone(), window, cx);
            assert_eq!(
                view.comment_state.open_thread.as_deref(),
                Some(thread.as_str())
            );
            assert!(view.comment_state.reply_editor.is_none());
        });
        cx.run_until_parked();
        assert!(cx.debug_bounds("dev-comment-read-only").is_some());
        let copy = cx
            .debug_bounds("dev-comment-copy")
            .expect("Copy thread control");
        cx.simulate_click(copy.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        cx.update(|_, cx| {
            assert_eq!(
                cx.read_from_clipboard().and_then(|item| item.text()),
                Some("Keep this discussion".to_owned())
            )
        });
        assert_eq!(snapshot(&item, cx), before);
        view.update_in(cx, |view, window, cx| {
            view.toggle_comment_thread(thread, window, cx);
            assert!(view.comment_state.open_thread.is_none());
            assert!(view.comment_state.reply_editor.is_none());
            assert!(view.close_blocker(cx).is_none());
        });
    }

    #[gpui::test]
    async fn measurement_hidden_pages_refuse_keyboard_activation_and_armed_pointer_writes(
        cx: &mut TestAppContext,
    ) {
        for hidden_by_registry in [true, false] {
            let (project, initial_item, visible_page, _) = fixture(cx).await;
            let mut doc =
                initial_item.read_with(cx, |item, _| item.doc().expect("fixture doc").clone());
            let mut hidden = CanvasNode::new(NodeData::Group(GroupNode::default()));
            hidden.name = "Internal component library".into();
            if hidden_by_registry {
                hidden.meta = serde_json::json!({"hidden_page": true});
            } else {
                hidden.flags.insert(fanta_doc::NodeFlags::HIDDEN);
            }
            let hidden_page = hidden.id;
            doc.apply(Operation::create_node(hidden))
                .expect("hidden page");
            doc.add_page(hidden_page);
            doc.history = Default::default();
            let item = crate::document::ready_item_for_test(
                &project,
                "/tmp/Hidden-measurement-page.fig".into(),
                doc,
                cx,
            );
            item.read_with(cx, |item, _| {
                let document = item.document().expect("document");
                let hidden = document
                    .pages
                    .iter()
                    .find(|page| page.root == Some(hidden_page))
                    .expect("hidden library remains navigable");
                assert_eq!(hidden.hidden, hidden_by_registry);
            });
            let (view, cx) =
                cx.add_window_view(|window, cx| FigView::new(item.clone(), project, window, cx));
            activate_canvas(&view, cx);
            for mode in [EditorMode::Design, EditorMode::Dev] {
                view.update_in(cx, |view, window, cx| {
                    view.set_editor_mode(mode, cx);
                    view.activate_tool(ToolKind::Select, cx);
                    view.select_page(1, cx);
                    view.focus_handle.focus(window, cx);
                });
                cx.run_until_parked();
                let baseline = snapshot(&item, cx);
                let previous_tool = view.read_with(cx, |view, _| view.tools.kind());
                cx.simulate_keystrokes("shift-m");
                cx.run_until_parked();
                if mode == EditorMode::Dev {
                    let measure = cx
                        .debug_bounds("native-dev-tool-2")
                        .expect("disabled native Measurement control");
                    cx.simulate_click(measure.center(), gpui::Modifiers::none());
                    cx.run_until_parked();
                }
                view.update_in(cx, |view, _, cx| {
                    assert_eq!(view.editor_mode(cx), mode);
                    assert_eq!(
                        view.item.read(cx).doc().expect("doc").active_page(),
                        Some(hidden_page)
                    );
                    assert_eq!(
                        view.tools.kind(),
                        previous_tool,
                        "hidden page keyboard activation is refused"
                    );
                    view.activate_tool(ToolKind::Measure, cx);
                    assert_eq!(
                        view.tools.kind(),
                        previous_tool,
                        "direct activation uses the same eligibility"
                    );
                    assert!(view.mark_page(cx).is_none());
                    assert!(!view.can_edit_measurements(cx));
                    assert!(!view.can_edit_annotations(cx));
                    assert!(view.measurement_host_origin(cx).is_err());
                    assert!(view.annotation_origin(cx).is_err());
                });
                assert_eq!(snapshot(&item, cx), baseline);

                view.update_in(cx, |view, window, cx| {
                    view.select_page(0, cx);
                    view.focus_handle.focus(window, cx);
                });
                cx.run_until_parked();
                cx.simulate_keystrokes("shift-m");
                cx.run_until_parked();
                view.update_in(cx, |view, _, cx| {
                    assert_eq!(
                        view.tools.kind(),
                        ToolKind::Measure,
                        "visible page arms Measure"
                    );
                    assert_eq!(view.mark_page(cx), Some(visible_page));
                    view.select_page(1, cx);
                });
                cx.run_until_parked();
                let baseline = snapshot(&item, cx);
                let start = view.read_with(cx, |view, _| {
                    assert_eq!(
                        view.tools.kind(),
                        ToolKind::Measure,
                        "exercise a tool armed before navigation"
                    );
                    assert!(view.viewport.is_some());
                    view.container_bounds.expect("rendered canvas").center()
                });
                let end = start + point(px(80.), px(30.));
                cx.simulate_mouse_down(start, MouseButton::Left, gpui::Modifiers::none());
                cx.simulate_mouse_move(end, MouseButton::Left, gpui::Modifiers::none());
                cx.simulate_mouse_up(end, MouseButton::Left, gpui::Modifiers::none());
                cx.run_until_parked();
                view.read_with(cx, |view, cx| {
                    assert!(!view.measurement_controller.has_pending_authoring());
                    assert!(view.page_measurements(cx).is_empty());
                });
                assert_eq!(
                    snapshot(&item, cx),
                    baseline,
                    "hidden page pointer path must not write metadata or history"
                );

                view.update_in(cx, |view, window, cx| {
                    view.select_page(0, cx);
                    view.focus_handle.focus(window, cx);
                });
                cx.run_until_parked();
                let start = view.read_with(cx, |view, _| {
                    view.container_bounds.expect("visible canvas").center()
                });
                let end = start + point(px(80.), px(30.));
                cx.simulate_mouse_down(start, MouseButton::Left, gpui::Modifiers::none());
                cx.simulate_mouse_move(end, MouseButton::Left, gpui::Modifiers::none());
                cx.simulate_mouse_up(end, MouseButton::Left, gpui::Modifiers::none());
                cx.run_until_parked();
                view.update_in(cx, |view, window, cx| {
                    assert_eq!(
                        view.page_measurements(cx).len(),
                        1,
                        "visible ordinary page still accepts the real pointer gesture"
                    );
                    assert_eq!(
                        view.item.read(cx).doc().expect("doc").history.undo_depth(),
                        1
                    );
                    view.undo(&Undo, window, cx);
                    assert!(view.page_measurements(cx).is_empty());
                    assert_eq!(
                        view.item.read(cx).doc().expect("doc").history.undo_depth(),
                        0
                    );
                });
            }
        }
    }
}
