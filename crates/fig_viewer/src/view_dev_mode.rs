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
