use super::*;
use crate::annotations::{
    AnnotationCommit, AnnotationController, AnnotationRecord, annotation_pin_hit_test,
    delete_annotation_op, read_annotations,
};
use editor::{Editor, EditorEvent};

#[derive(Default)]
pub(super) struct AnnotationHostState {
    pub(super) controller: AnnotationController,
    origin: Option<LocalMediaOrigin>,
    pub(super) selection: Option<MeasurementSelection>,
    cache: std::cell::RefCell<Option<AnnotationCache>>,
    editor: Option<Entity<Editor>>,
    editor_subscription: Option<Subscription>,
    error: Option<SharedString>,
}

struct AnnotationCache {
    scene: u64,
    revision: u64,
    page: NodeId,
    records: Vec<AnnotationRecord>,
}

#[derive(Clone)]
pub(crate) struct AnnotationOverlay {
    pub(crate) screen: [f64; 2],
    pub(crate) label: String,
    pub(crate) selected: bool,
    pub(crate) preview: bool,
}

impl FigView {
    pub(super) fn annotation_page(&self, cx: &App) -> Option<NodeId> {
        if self.editor_mode(cx) != EditorMode::Design
            || self.editor_workspace(cx) != EditorWorkspace::Canvas
            || self.prototype_player.is_some()
            || matches!(
                self.scope,
                Some(FigScope::Component(_) | FigScope::Variables)
            )
        {
            return None;
        }
        let document = self.item.read(cx).document()?;
        let page = document.doc.active_page()?;
        (document.doc.pages().contains(&page)
            && document
                .pages
                .iter()
                .any(|candidate| candidate.root == Some(page) && !candidate.hidden))
        .then_some(page)
    }

    pub(crate) fn can_edit_annotations(&self, cx: &App) -> bool {
        !self.is_inspecting()
            && self.item.read(cx).is_editable()
            && self.annotation_page(cx).is_some()
    }

    pub(crate) fn page_annotations(&self, cx: &App) -> Vec<AnnotationRecord> {
        let Some(page) = self.annotation_page(cx) else {
            return Vec::new();
        };
        let Some(doc) = self.item.read(cx).doc() else {
            return Vec::new();
        };
        let mut cache = self.annotation_state.cache.borrow_mut();
        if let Some(cache) = cache.as_ref()
            && cache.scene == doc.scene.instance_id()
            && cache.revision == doc.scene.revision()
            && cache.page == page
        {
            return cache.records.clone();
        }
        let records = match read_annotations(doc, page) {
            Ok(records) => records,
            Err(error) => {
                log::warn!("Could not read page annotations: {error:#}");
                Vec::new()
            }
        };
        *cache = Some(AnnotationCache {
            scene: doc.scene.instance_id(),
            revision: doc.scene.revision(),
            page,
            records: records.clone(),
        });
        records
    }

    pub(crate) fn selected_annotation(&self, cx: &App) -> Option<AnnotationRecord> {
        let selection = self.annotation_state.selection.as_ref()?;
        let doc = self.item.read(cx).doc()?;
        if doc.id != selection.document
            || doc.scene.instance_id() != selection.scene
            || doc.active_page() != Some(selection.page)
        {
            return None;
        }
        self.page_annotations(cx)
            .into_iter()
            .find(|record| record.annotation().id == selection.id)
    }

    pub(super) fn annotation_origin(&self, cx: &Context<Self>) -> Result<LocalMediaOrigin> {
        anyhow::ensure!(
            self.can_edit_annotations(cx),
            "Annotations can be edited on an editable Design page."
        );
        let item = self.item.read(cx);
        anyhow::ensure!(
            item.can_preview_for_owner(cx.entity_id()) && !item.content_preview_active(),
            "Finish saving or editing this document before changing annotations."
        );
        let doc = item
            .doc()
            .context("The annotation document is unavailable.")?;
        Ok(LocalMediaOrigin {
            item: self.item.entity_id(),
            scene: doc.scene.instance_id(),
            page: self
                .annotation_page(cx)
                .context("Choose a visible Design page for the annotation.")?,
            scope: self.scope,
            mode: self.editor_mode(cx),
            path: item.abs_path().to_path_buf(),
            // Tab deactivation must not invalidate text that the author can resume.
            generation: 0,
        })
    }

    pub(super) fn validate_annotation_origin(&self, cx: &Context<Self>) -> Result<()> {
        anyhow::ensure!(
            self.annotation_state.origin.as_ref() == Some(&self.annotation_origin(cx)?),
            "The annotation's document or page changed. Return to its original page, or copy and cancel this draft."
        );
        Ok(())
    }

    pub(super) fn annotation_error(
        &mut self,
        error: impl std::fmt::Display,
        cx: &mut Context<Self>,
    ) {
        let message = format!("Annotation: {error}");
        self.annotation_state.error = Some(message.clone().into());
        self.inspector_sidebar_visible = true;
        show_canvas_notice_deferred(message, cx);
        cx.notify();
    }

    pub(super) fn refuse_pending_annotation(&self, cx: &mut Context<Self>) -> bool {
        if !self.annotation_state.controller.has_pending_authoring() {
            return false;
        }
        show_canvas_notice_deferred(
            "Add, save or cancel the annotation first. Its draft was kept.".into(),
            cx,
        );
        true
    }

    pub(super) fn cancel_annotation(
        &mut self,
        generation: Option<u64>,
        cx: &mut Context<Self>,
    ) -> bool {
        let canceled = match generation {
            Some(generation) => self.annotation_state.controller.cancel_draft(generation),
            None => self.annotation_state.controller.cancel(),
        };
        if !canceled {
            return false;
        }
        self.annotation_state.origin = None;
        self.annotation_state.editor = None;
        self.annotation_state.editor_subscription = None;
        self.annotation_state.error = None;
        self.primary_pressed = false;
        self.canvas_pointer_down = false;
        self.schedule_autosave(cx);
        self.sync_measurement_edit_barrier(cx);
        cx.notify();
        true
    }

    pub(super) fn freeze_annotation_move(&mut self, cx: &mut Context<Self>) {
        if self.annotation_state.controller.is_moving() {
            self.annotation_state.controller.freeze_move();
            self.primary_pressed = false;
            self.canvas_pointer_down = false;
            self.annotation_state.error = Some(
                "The annotation move was interrupted. Copy or cancel its retained draft.".into(),
            );
            cx.notify();
        }
    }

    pub(super) fn reconcile_annotations(&mut self, cx: &mut Context<Self>) {
        if self.annotation_state.selection.is_some()
            && (self.selected_annotation(cx).is_none()
                || self
                    .item
                    .read(cx)
                    .doc()
                    .is_some_and(|doc| !doc.selection.is_empty()))
        {
            self.annotation_state.selection = None;
            let view = cx.weak_entity();
            cx.defer(move |cx| {
                view.update(cx, |view, cx| view.sync_measurement_edit_barrier(cx))
                    .log_err();
            });
        }
        if self.annotation_state.controller.has_pending_authoring() {
            if let Err(error) = self.validate_annotation_origin(cx) {
                self.freeze_annotation_move(cx);
                self.annotation_state.error = Some(error.to_string().into());
                self.inspector_sidebar_visible = true;
            }
        }
    }

    pub(crate) fn select_annotation(&mut self, expected: AnnotationRecord, cx: &mut Context<Self>) {
        if self.has_pending_authoring_except_pointer(cx)
            || self.comment_state.draft.is_some()
            || self.has_unsent_comment_reply(cx)
        {
            show_canvas_notice_deferred(
                "Finish or cancel the current draft before selecting an annotation.".into(),
                cx,
            );
            return;
        }
        if !self
            .page_annotations(cx)
            .iter()
            .any(|record| record == &expected)
        {
            return;
        }
        if !matches!(self.tools.kind(), ToolKind::Annotation | ToolKind::Inspect) {
            let tool = if self.can_edit_annotations(cx) {
                ToolKind::Annotation
            } else {
                ToolKind::Inspect
            };
            self.activate_tool(tool, cx);
            if self.tools.kind() != tool {
                return;
            }
        }
        let Some(doc) = self.item.read(cx).doc() else {
            return;
        };
        self.annotation_state.selection = Some(MeasurementSelection {
            document: doc.id,
            scene: doc.scene.instance_id(),
            page: expected.page(),
            id: expected.annotation().id.clone(),
        });
        self.measurement_selection = None;
        self.clear_art_selection_for_annotation(cx);
        self.inspector_sidebar_visible = true;
        self.sync_measurement_edit_barrier(cx);
        cx.notify();
    }

    pub(super) fn clear_art_selection_for_annotation(&mut self, cx: &mut Context<Self>) {
        self.item.update(cx, |item, cx| {
            item.with_document(cx, |document| {
                let changed = !document.doc.selection.is_empty();
                document.doc.selection.clear();
                (
                    (),
                    if changed {
                        DocChange::Selection
                    } else {
                        DocChange::None
                    },
                )
            });
        });
        self.hovered_node = None;
        self.hover_resize_handle = None;
    }

    pub(crate) fn copy_annotation(&self, expected: &AnnotationRecord, cx: &mut Context<Self>) {
        if self.selected_annotation(cx).as_ref() == Some(expected) {
            cx.write_to_clipboard(ClipboardItem::new_string(
                expected.annotation().text.clone(),
            ));
        }
    }

    pub(super) fn copy_annotation_draft(&self, generation: u64, cx: &mut Context<Self>) {
        if let Some(draft) = self
            .annotation_state
            .controller
            .draft()
            .filter(|draft| draft.generation() == generation)
        {
            cx.write_to_clipboard(ClipboardItem::new_string(draft.annotation().text.clone()));
        }
    }

    pub(crate) fn delete_annotation(
        &mut self,
        expected: &AnnotationRecord,
        cx: &mut Context<Self>,
    ) {
        if !self.can_edit_annotations(cx)
            || self.has_pending_authoring_except_pointer(cx)
            || self.comment_state.draft.is_some()
            || self.has_unsent_comment_reply(cx)
            || self.selected_annotation(cx).as_ref() != Some(expected)
        {
            return;
        }
        let owner = cx.entity_id();
        let result = self.item.update(cx, |item, cx| -> Result<()> {
            anyhow::ensure!(
                item.is_editable()
                    && item.can_preview_for_owner(owner)
                    && !item.content_preview_active(),
                "Finish saving or editing this document before deleting the annotation."
            );
            let operation = delete_annotation_op(
                item.doc().context("The document is unavailable.")?,
                expected,
            )?;
            item.apply_for_preview_owner(owner, operation, cx)
        });
        match result {
            Ok(()) => {
                self.annotation_state.selection = None;
                self.sync_measurement_edit_barrier(cx);
                cx.notify();
            }
            Err(error) => self.annotation_error(error, cx),
        }
    }

    pub(super) fn open_annotation_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(draft) = self.annotation_state.controller.draft() else {
            return;
        };
        let generation = draft.generation();
        let text = draft.annotation().text.clone();
        let editor = cx.new(|cx| {
            let mut editor = Editor::auto_height(3, 12, window, cx);
            editor.set_placeholder_text("Write an annotation…", window, cx);
            editor.set_text(text, window, cx);
            editor
        });
        self.annotation_state.editor_subscription = Some(cx.subscribe(
            &editor,
            move |view, editor, event: &EditorEvent, cx| {
                if matches!(event, EditorEvent::BufferEdited)
                    && view
                        .annotation_state
                        .controller
                        .draft()
                        .is_some_and(|draft| draft.generation() == generation)
                {
                    let text = editor.read(cx).text(cx);
                    match view.annotation_state.controller.set_text(generation, text) {
                        Ok(()) => {
                            view.annotation_state.error = None;
                            cx.notify();
                        }
                        Err(error) => view.annotation_error(error, cx),
                    }
                }
            },
        ));
        editor.read(cx).focus_handle(cx).focus(window, cx);
        self.annotation_state.editor = Some(editor);
        self.annotation_state.error = None;
        self.inspector_sidebar_visible = true;
        self.sync_measurement_edit_barrier(cx);
        cx.notify();
    }

    pub(crate) fn edit_annotation(
        &mut self,
        expected: AnnotationRecord,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.selected_annotation(cx).as_ref() != Some(&expected) {
            return;
        }
        let result = (|| -> Result<()> {
            anyhow::ensure!(
                !self.has_pending_authoring_except_pointer(cx)
                    && self.comment_state.draft.is_none()
                    && !self.has_unsent_comment_reply(cx),
                "Finish or cancel the current edit first."
            );
            let origin = self.annotation_origin(cx)?;
            self.annotation_state.controller.begin_edit(
                self.item
                    .read(cx)
                    .doc()
                    .context("The document is unavailable.")?,
                &expected,
            )?;
            self.annotation_state.origin = Some(origin);
            Ok(())
        })();
        match result {
            Ok(()) => self.open_annotation_editor(window, cx),
            Err(error) => self.annotation_error(error, cx),
        }
    }

    pub(super) fn apply_annotation_intent(
        &mut self,
        intent: Option<AnnotationCommit>,
        generation: u64,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        self.validate_annotation_origin(cx)?;
        anyhow::ensure!(
            self.annotation_state
                .controller
                .draft()
                .is_some_and(|draft| draft.generation() == generation),
            "This action belongs to an earlier annotation draft."
        );
        let id = if let Some(intent) = intent {
            let controller = &self.annotation_state.controller;
            let owner = cx.entity_id();
            Some(self.item.update(cx, |item, cx| -> Result<String> {
                anyhow::ensure!(
                    item.is_editable()
                        && item.can_preview_for_owner(owner)
                        && !item.content_preview_active(),
                    "Finish saving or editing this document and retry the annotation."
                );
                let (id, operation) = controller.build_operation(
                    item.doc().context("The document is unavailable.")?,
                    &intent,
                )?;
                item.apply_for_preview_owner(owner, operation, cx)?;
                Ok(id)
            })?)
        } else {
            None
        };
        self.cancel_annotation(Some(generation), cx);
        if let Some(id) = id
            && let Some(record) = self
                .page_annotations(cx)
                .into_iter()
                .find(|record| record.annotation().id == id)
        {
            self.select_annotation(record, cx);
        }
        Ok(())
    }

    pub(super) fn submit_annotation(&mut self, generation: u64, cx: &mut Context<Self>) {
        let result = (|| -> Result<()> {
            self.validate_annotation_origin(cx)?;
            let draft = self
                .annotation_state
                .controller
                .draft()
                .context("There is no annotation draft.")?;
            anyhow::ensure!(
                draft.generation() == generation,
                "This action belongs to an earlier annotation draft."
            );
            let doc = self
                .item
                .read(cx)
                .doc()
                .context("The document is unavailable.")?;
            let intent = if draft.is_move() {
                self.annotation_state.controller.prepare_move(doc)?
            } else if draft.is_new() {
                Some(
                    self.annotation_state
                        .controller
                        .prepare_add(doc, generation)?,
                )
            } else {
                self.annotation_state
                    .controller
                    .prepare_save(doc, generation)?
            };
            self.apply_annotation_intent(intent, generation, cx)
        })();
        if let Err(error) = result {
            self.annotation_error(error, cx);
        }
    }

    pub(super) fn handle_annotation_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if event.button != MouseButton::Left
            || self.space_pan
            || !matches!(self.tools.kind(), ToolKind::Annotation | ToolKind::Inspect)
        {
            return false;
        }
        if self.annotation_state.controller.has_pending_authoring() {
            self.refuse_pending_annotation(cx);
            self.canvas_pointer_down = false;
            return true;
        }
        let Some(bounds) = self.container_bounds else {
            return false;
        };
        let Some(viewport) = self.viewport else {
            return false;
        };
        let (width, height) = bounds_size(bounds);
        let screen = screen_position_in_bounds(event.position, bounds).to_array();
        let hit = self.measurement_projection(cx).ok().and_then(|projection| {
            let mut records = self.page_annotations(cx);
            records.sort_by_key(|record| {
                self.annotation_state
                    .selection
                    .as_ref()
                    .is_some_and(|selected| selected.id == record.annotation().id)
            });
            records.into_iter().rev().find(|record| {
                annotation_pin_hit_test(projection, record.annotation().anchor, screen, 12.0)
                    .unwrap_or(false)
            })
        });
        if let Some(record) = hit {
            if self.has_pending_authoring_except_pointer(cx)
                || self.comment_state.draft.is_some()
                || self.has_unsent_comment_reply(cx)
            {
                self.annotation_error(
                    "Finish or cancel the current draft before moving an annotation.",
                    cx,
                );
                self.canvas_pointer_down = false;
                return true;
            }
            self.select_annotation(record.clone(), cx);
            if self.selected_annotation(cx).as_ref() != Some(&record) {
                return true;
            }
            if self.is_inspecting() {
                return true;
            }
            if event.click_count >= 2 {
                self.edit_annotation(record, window, cx);
                return true;
            }
            let result = (|| -> Result<()> {
                let origin = self.annotation_origin(cx)?;
                self.annotation_state.controller.begin_move(
                    self.item
                        .read(cx)
                        .doc()
                        .context("The document is unavailable.")?,
                    &record,
                    viewport,
                    [width, height],
                    screen,
                )?;
                self.annotation_state.origin = Some(origin);
                self.annotation_state.error = None;
                self.primary_pressed = true;
                Ok(())
            })();
            if let Err(error) = result {
                self.annotation_error(error, cx);
            }
            return true;
        }
        if self.tools.kind() != ToolKind::Annotation {
            return false;
        }
        // Existing comment pins retain their hit target rather than acquiring a note beneath them.
        if self.comment_pin_at(DVec2::from(screen), cx).is_some() {
            return false;
        }
        let result = (|| -> Result<()> {
            anyhow::ensure!(
                !self.has_pending_authoring_except_pointer(cx)
                    && self.comment_state.draft.is_none()
                    && !self.has_unsent_comment_reply(cx),
                "Finish or cancel the current edit before annotating."
            );
            let origin = self.annotation_origin(cx)?;
            let created = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .context("The system clock is before the Unix epoch.")?
                .as_secs();
            self.annotation_state.controller.begin_create(
                self.item
                    .read(cx)
                    .doc()
                    .context("The document is unavailable.")?,
                origin.page,
                viewport,
                [width, height],
                screen,
                crate::comments::author_name(),
                created,
            )?;
            self.annotation_state.origin = Some(origin);
            self.annotation_state.selection = None;
            self.measurement_selection = None;
            self.primary_pressed = false;
            self.canvas_pointer_down = false;
            self.clear_art_selection_for_annotation(cx);
            Ok(())
        })();
        match result {
            Ok(()) => self.open_annotation_editor(window, cx),
            Err(error) => self.annotation_error(error, cx),
        }
        true
    }

    pub(super) fn handle_annotation_tool_event(
        &mut self,
        event: ToolEvent,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.tools.kind() != ToolKind::Annotation {
            return false;
        }
        if !self.annotation_state.controller.is_moving() {
            return true;
        }
        let result = (|| -> Result<()> {
            self.validate_annotation_origin(cx)?;
            let bounds = self
                .container_bounds
                .context("The canvas bounds are unavailable.")?;
            let viewport = self
                .viewport
                .context("The canvas viewport is unavailable.")?;
            let (width, height) = bounds_size(bounds);
            let doc = self
                .item
                .read(cx)
                .doc()
                .context("The document is unavailable.")?;
            match event {
                ToolEvent::Pointer(fanta_tools::PointerEvent::Move { screen, .. }) => {
                    self.annotation_state.controller.update_move(
                        doc,
                        viewport,
                        [width, height],
                        screen,
                    )?;
                }
                ToolEvent::Pointer(fanta_tools::PointerEvent::Release {
                    screen,
                    button: ToolButton::Primary,
                    ..
                }) => {
                    let generation = self
                        .annotation_state
                        .controller
                        .draft()
                        .context("The move was canceled.")?
                        .generation();
                    let intent = self.annotation_state.controller.release_move(
                        doc,
                        viewport,
                        [width, height],
                        screen,
                    )?;
                    self.apply_annotation_intent(intent, generation, cx)?;
                }
                _ => {}
            }
            cx.notify();
            Ok(())
        })();
        if let Err(error) = result {
            self.freeze_annotation_move(cx);
            self.primary_pressed = false;
            self.canvas_pointer_down = false;
            self.annotation_error(error, cx);
        }
        true
    }

    pub(crate) fn annotation_overlays(
        &self,
        viewport: Viewport,
        screen_size: [f64; 2],
        cx: &App,
    ) -> Vec<AnnotationOverlay> {
        let Some(page) = self.annotation_page(cx) else {
            return Vec::new();
        };
        let Some(doc) = self.item.read(cx).doc() else {
            return Vec::new();
        };
        let Some(transform) = doc.scene.world_transform(page) else {
            return Vec::new();
        };
        let Ok(projection) = MeasurementProjection::new(transform, viewport, screen_size) else {
            return Vec::new();
        };
        let draft = self.annotation_state.controller.draft().filter(|draft| {
            draft.page() == page
                && self.annotation_state.origin.as_ref().is_some_and(|origin| {
                    origin.item == self.item.entity_id()
                        && origin.scene == doc.scene.instance_id()
                        && origin.path == self.item.read(cx).abs_path()
                })
        });
        let mut overlays: Vec<_> = self
            .page_annotations(cx)
            .into_iter()
            .enumerate()
            .filter_map(|(index, record)| {
                if draft.is_some_and(|draft| {
                    draft.is_move() && draft.annotation().id == record.annotation().id
                }) {
                    return None;
                }
                let screen = projection.page_to_screen(record.annotation().anchor).ok()?;
                let selected = self
                    .annotation_state
                    .selection
                    .as_ref()
                    .is_some_and(|selection| selection.id == record.annotation().id);
                Some(AnnotationOverlay {
                    screen,
                    label: if index < 99 {
                        format!("{}", index + 1)
                    } else {
                        "•".into()
                    },
                    selected,
                    preview: false,
                })
            })
            .collect();
        overlays.sort_by_key(|overlay| overlay.selected);
        if let Some(draft) = draft
            && let Ok(screen) = projection.page_to_screen(draft.annotation().anchor)
        {
            overlays.push(AnnotationOverlay {
                screen,
                label: if draft.is_new() {
                    "+".into()
                } else {
                    "•".into()
                },
                selected: true,
                preview: true,
            });
        }
        overlays
    }

    pub(super) fn render_annotation_list(&self, cx: &mut Context<Self>) -> AnyElement {
        let selected = self
            .annotation_state
            .selection
            .as_ref()
            .map(|selection| selection.id.as_str());
        let mut list = v_flex()
            .id("fanta-annotation-list")
            .size_full()
            .min_w_0()
            .overflow_y_scroll()
            .p_3()
            .gap_2()
            .child(
                Label::new("Annotations on this page")
                    .size(LabelSize::Small)
                    .color(Color::Muted),
            );
        let records = self.page_annotations(cx);
        if records.is_empty() {
            list = list.child(
                Label::new("Click the canvas to place a note.")
                    .size(LabelSize::Small)
                    .color(Color::Muted),
            );
        }
        for (index, record) in records.into_iter().enumerate() {
            let id = record.annotation().id.clone();
            let label = record
                .annotation()
                .text
                .chars()
                .take(64)
                .map(|character| {
                    if character.is_whitespace() {
                        ' '
                    } else {
                        character
                    }
                })
                .collect::<String>();
            let view = cx.weak_entity();
            let is_selected = selected == Some(id.as_str());
            let button = Button::new(
                SharedString::from(format!("annotation-list-{id}")),
                format!("{} · {}", index + 1, label),
            )
            .toggle_state(is_selected)
            .on_click(move |_, _, cx| {
                view.update(cx, |view, cx| view.select_annotation(record.clone(), cx))
                    .log_err();
            });
            list = list.child(button);
        }
        list.into_any_element()
    }

    pub(super) fn render_annotation_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut panel = v_flex()
            .id("fanta-annotation-properties")
            .size_full()
            .min_w_0()
            .overflow_hidden();
        let mut content = v_flex()
            .id("fanta-annotation-content")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .p_3()
            .gap_3();
        if let Some(draft) = self.annotation_state.controller.draft() {
            let generation = draft.generation();
            content = content.child(Label::new(if draft.is_new() {
                "New annotation"
            } else if draft.is_move() {
                "Move annotation"
            } else {
                "Edit annotation"
            }));
            if let Some(editor) = self.annotation_state.editor.as_ref() {
                content = content.child(
                    div()
                        .id("fanta-annotation-editor")
                        .debug_selector(|| "fanta-annotation-editor".to_owned())
                        .min_w_0()
                        .child(editor.clone()),
                );
            } else {
                content = content.child(
                    div()
                        .min_w_0()
                        .whitespace_normal()
                        .child(draft.annotation().text.clone()),
                );
            }
            let copy = cx.weak_entity();
            let cancel = copy.clone();
            let submit = copy.clone();
            content = content.child(
                h_flex()
                    .flex_wrap()
                    .gap_2()
                    .child(Button::new("annotation-copy-draft", "Copy text").on_click(
                        move |_, _, cx| {
                            copy.update(cx, |view, cx| view.copy_annotation_draft(generation, cx))
                                .log_err();
                        },
                    ))
                    .child(Button::new("annotation-cancel", "Cancel").on_click(
                        move |_, window, cx| {
                            cancel
                                .update(cx, |view, cx| {
                                    if view.cancel_annotation(Some(generation), cx) {
                                        view.focus_handle.focus(window, cx);
                                    }
                                })
                                .log_err();
                        },
                    ))
                    .child(
                        div()
                            .debug_selector(|| "annotation-submit-target".to_owned())
                            .child(
                                Button::new(
                                    "annotation-submit",
                                    if draft.is_new() {
                                        "Add"
                                    } else if draft.is_move() {
                                        "Retry move"
                                    } else {
                                        "Save"
                                    },
                                )
                                .disabled(
                                    !self.can_edit_annotations(cx)
                                        || self.annotation_state.controller.is_moving(),
                                )
                                .on_click(move |_, window, cx| {
                                    submit
                                        .update(cx, |view, cx| {
                                            let owns_draft = view
                                                .annotation_state
                                                .controller
                                                .draft()
                                                .is_some_and(|draft| {
                                                    draft.generation() == generation
                                                });
                                            view.submit_annotation(generation, cx);
                                            if owns_draft
                                                && !view
                                                    .annotation_state
                                                    .controller
                                                    .has_pending_authoring()
                                            {
                                                view.focus_handle.focus(window, cx);
                                            }
                                        })
                                        .log_err();
                                }),
                            ),
                    ),
            );
        } else if let Some(record) = self.selected_annotation(cx) {
            content = content
                .child(Label::new("Annotation"))
                .child(
                    div()
                        .min_w_0()
                        .whitespace_normal()
                        .child(record.annotation().text.clone()),
                )
                .child(
                    Label::new(format!(
                        "{} · Fixed position on this page",
                        record.annotation().author
                    ))
                    .size(LabelSize::Small)
                    .color(Color::Muted),
                );
            let copy_view = cx.weak_entity();
            let copy_record = record.clone();
            let edit_view = copy_view.clone();
            let edit_record = record.clone();
            let delete_view = copy_view.clone();
            let editable = self.can_edit_annotations(cx);
            content = content.child(
                h_flex()
                    .flex_wrap()
                    .gap_2()
                    .child(
                        Button::new("annotation-copy", "Copy text").on_click(move |_, _, cx| {
                            copy_view
                                .update(cx, |view, cx| view.copy_annotation(&copy_record, cx))
                                .log_err();
                        }),
                    )
                    .child(
                        div()
                            .debug_selector(|| "annotation-edit-target".to_owned())
                            .child(
                                Button::new("annotation-edit", "Edit")
                                    .disabled(!editable)
                                    .on_click(move |_, window, cx| {
                                        edit_view
                                            .update(cx, |view, cx| {
                                                view.edit_annotation(
                                                    edit_record.clone(),
                                                    window,
                                                    cx,
                                                )
                                            })
                                            .log_err();
                                    }),
                            ),
                    )
                    .child(
                        Button::new("annotation-delete", "Delete")
                            .disabled(!editable)
                            .on_click(move |_, _, cx| {
                                delete_view
                                    .update(cx, |view, cx| view.delete_annotation(&record, cx))
                                    .log_err();
                            }),
                    ),
            );
        }
        if let Some(error) = self.annotation_state.error.as_ref() {
            content = content.child(
                div()
                    .min_w_0()
                    .whitespace_normal()
                    .child(Label::new(error.clone()).color(Color::Error)),
            );
        }
        panel = panel.child(content).child(
            div()
                .h(px(180.0))
                .min_h(px(90.0))
                .flex_shrink_0()
                .child(self.render_annotation_list(cx)),
        );
        panel.into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::annotations::{DeveloperAnnotation, create_annotation_op, update_annotation_op};
    use fanta_doc::{Color as DocumentColor, GroupNode, VectorNode};
    use fs::FakeFs;
    use gpui::{TestAppContext, point, size};

    struct Fixture {
        item: Entity<FigItem>,
        view: Entity<FigView>,
        project: Entity<Project>,
        scratch: gpui::WindowHandle<gpui::Empty>,
        page: NodeId,
        other_page: NodeId,
        vector: NodeId,
    }

    async fn fixture(cx: &mut TestAppContext) -> Fixture {
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
        let page_node = CanvasNode::new(NodeData::Group(GroupNode::default()));
        let page = page_node.id;
        doc.apply(Operation::create_node(page_node)).expect("page");
        doc.add_page(page);
        let other = CanvasNode::new(NodeData::Group(GroupNode::default()));
        let other_page = other.id;
        doc.apply(Operation::create_node(other))
            .expect("other page");
        doc.add_page(other_page);
        doc.set_active_page(Some(page));
        let mut node = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
            0.0,
            0.0,
            20.0,
            30.0,
            DocumentColor::BLACK,
        )));
        node.parent = Some(page);
        let vector = node.id;
        doc.apply(Operation::create_node(node)).expect("rectangle");
        doc.selection.select_only(vector);
        doc.history = Default::default();
        let item = crate::document::ready_item_for_test(
            &project,
            "/tmp/Annotation-host.fig".into(),
            doc,
            cx,
        );
        let scratch = cx.add_window(|_, _| gpui::Empty);
        let view = scratch
            .update(cx, |_, window, cx| {
                cx.new(|cx| FigView::new(item.clone(), project.clone(), window, cx))
            })
            .expect("annotation view");
        cx.run_until_parked();
        view.update(cx, |view, cx| {
            view.select_page(0, cx);
            view.container_bounds = Some(Bounds::new(
                point(px(100.), px(50.)),
                size(px(800.), px(600.)),
            ));
            view.viewport = Some(Viewport::default());
        });
        Fixture {
            item,
            view,
            project,
            scratch,
            page,
            other_page,
            vector,
        }
    }

    fn start_draft(fixture: &Fixture, cx: &mut TestAppContext) -> u64 {
        fixture
            .scratch
            .update(cx, |_, window, cx| {
                fixture.view.update(cx, |view, cx| {
                    view.activate_tool(ToolKind::Annotation, cx);
                    assert_eq!(view.tools.kind(), ToolKind::Annotation);
                    view.handle_mouse_down(
                        &MouseDownEvent {
                            button: MouseButton::Left,
                            position: point(px(500.), px(350.)),
                            modifiers: gpui::Modifiers::none(),
                            click_count: 1,
                            first_mouse: false,
                        },
                        window,
                        cx,
                    );
                    view.annotation_state
                        .controller
                        .draft()
                        .expect("click opened composer")
                        .generation()
                })
            })
            .expect("place annotation")
    }

    fn type_draft(fixture: &Fixture, value: &str, cx: &mut TestAppContext) {
        fixture
            .scratch
            .update(cx, |_, window, cx| {
                fixture.view.update(cx, |view, cx| {
                    view.annotation_state
                        .editor
                        .as_ref()
                        .expect("composer editor")
                        .clone()
                        .update(cx, |editor, cx| editor.set_text(value, window, cx));
                });
            })
            .expect("type annotation body");
        cx.run_until_parked();
    }

    #[gpui::test]
    async fn annotation_click_add_edit_copy_delete_are_separate_undo_steps(
        cx: &mut TestAppContext,
    ) {
        let fixture = fixture(cx).await;
        let baseline = fixture.item.read_with(cx, |item, _| {
            serde_json::to_value(item.doc().expect("doc")).expect("baseline")
        });
        let generation = start_draft(&fixture, cx);
        fixture
            .view
            .update(cx, |view, cx| view.submit_annotation(generation, cx));
        fixture.view.read_with(cx, |view, _| {
            assert!(view.annotation_state.controller.has_pending_authoring())
        });
        fixture.item.read_with(cx, |item, _| {
            assert!(
                read_annotations(item.doc().expect("doc"), fixture.page)
                    .expect("notes")
                    .is_empty()
            );
            assert_eq!(item.doc().expect("doc").history.undo_depth(), 0);
            assert!(!item.is_dirty());
        });
        let body = "  Café\n🟠 Keep this exact text.\n";
        type_draft(&fixture, body, cx);
        fixture
            .view
            .update(cx, |view, cx| view.submit_annotation(generation, cx));
        let record = fixture.view.read_with(cx, |view, cx| {
            view.selected_annotation(cx).expect("added note selected")
        });
        assert_eq!(record.annotation().text, body);
        assert_eq!(record.annotation().anchor, [0.0, 0.0]);
        fixture.item.read_with(cx, |item, _| {
            assert_eq!(item.doc().expect("doc").history.undo_depth(), 1)
        });
        fixture
            .scratch
            .update(cx, |_, window, cx| {
                fixture.view.update(cx, |view, cx| {
                    view.edit_annotation(record.clone(), window, cx)
                })
            })
            .expect("edit selected note");
        let edit_generation = fixture.view.read_with(cx, |view, _| {
            view.annotation_state
                .controller
                .draft()
                .expect("edit draft")
                .generation()
        });
        type_draft(&fixture, "Revised\ntext", cx);
        fixture.view.update(cx, |view, cx| {
            view.submit_annotation(generation, cx);
            assert!(view.annotation_state.controller.has_pending_authoring());
            assert_eq!(
                view.selected_annotation(cx)
                    .expect("original record")
                    .annotation()
                    .text,
                body
            );
            view.submit_annotation(edit_generation, cx);
        });
        let revised = fixture.view.read_with(cx, |view, cx| {
            view.selected_annotation(cx).expect("saved edit")
        });
        assert_eq!(revised.annotation().id, record.annotation().id);
        assert_eq!(revised.annotation().text, "Revised\ntext");
        fixture.view.update(cx, |view, cx| {
            view.copy_selected_nodes(cx);
            view.delete_selected_nodes(cx);
        });
        cx.update(|cx| {
            assert_eq!(
                cx.read_from_clipboard()
                    .and_then(|clipboard| clipboard.text()),
                Some("Revised\ntext".to_owned())
            )
        });
        fixture.item.read_with(cx, |item, _| {
            assert_eq!(item.doc().expect("doc").history.undo_depth(), 3);
            assert!(
                read_annotations(item.doc().expect("doc"), fixture.page)
                    .expect("notes")
                    .is_empty()
            );
            assert!(item.doc().expect("doc").scene.get(fixture.vector).is_some());
        });
        for expected in ["Revised\ntext", body] {
            fixture.item.update(cx, |item, cx| {
                item.undo(cx).expect("undo annotation operation")
            });
            fixture.item.read_with(cx, |item, _| {
                assert_eq!(
                    read_annotations(item.doc().expect("doc"), fixture.page)
                        .expect("notes")
                        .first()
                        .expect("restored note")
                        .annotation()
                        .text,
                    expected
                )
            });
        }
        fixture
            .item
            .update(cx, |item, cx| item.undo(cx).expect("undo annotation Add"));
        fixture.item.read_with(cx, |item, _| {
            let doc = item.doc().expect("doc");
            assert!(
                read_annotations(doc, fixture.page)
                    .expect("notes")
                    .is_empty()
            );
            assert_eq!(
                serde_json::to_value(&doc.scene).expect("scene"),
                baseline["scene"]
            );
        });
    }

    #[gpui::test]
    async fn annotation_unsent_text_survives_save_navigation_and_stale_callbacks(
        cx: &mut TestAppContext,
    ) {
        let fixture = fixture(cx).await;
        let generation = start_draft(&fixture, cx);
        type_draft(&fixture, "Unsent exact\n🟠 draft", cx);
        fixture
            .scratch
            .update(cx, |_, window, cx| {
                fixture.view.update(cx, |view, cx| {
                    view.activate_tool_from_action(ToolKind::Select, window, cx);
                    view.set_editor_mode(EditorMode::Motion, cx);
                    view.set_editor_workspace(EditorWorkspace::Code, cx);
                    view.select_page(1, cx);
                    assert_eq!(view.tools.kind(), ToolKind::Annotation);
                    assert_eq!(view.editor_mode(cx), EditorMode::Design);
                    assert_eq!(view.editor_workspace(cx), EditorWorkspace::Canvas);
                    assert_eq!(
                        view.item.read(cx).doc().expect("doc").active_page(),
                        Some(fixture.page)
                    );
                    assert!(view.close_blocker(cx).is_some());
                });
            })
            .expect("refuse navigation without losing text");
        let save = fixture
            .scratch
            .update(cx, |_, window, cx| {
                fixture.view.update(cx, |view, cx| {
                    Item::save(
                        view,
                        SaveOptions::default(),
                        fixture.project.clone(),
                        window,
                        cx,
                    )
                })
            })
            .expect("Save route");
        assert!(save.await.is_err());
        let file_system = fixture
            .project
            .read_with(cx, |project, _| project.fs().clone());
        let destination_root = std::path::Path::new("/annotation-save-as-denial");
        file_system
            .create_dir(destination_root)
            .await
            .expect("fake destination worktree");
        fixture
            .project
            .update(cx, |project, cx| {
                project.find_or_create_worktree(destination_root, true, cx)
            })
            .await
            .expect("register destination worktree");
        let destination_path = destination_root.join("Annotation Copy");
        let destination = fixture
            .project
            .read_with(cx, |project, cx| {
                project.find_project_path(&destination_path, cx)
            })
            .expect("valid project-relative Save As destination");
        let save_as = fixture
            .scratch
            .update(cx, |_, window, cx| {
                fixture.view.update(cx, |view, cx| {
                    Item::save_as(view, fixture.project.clone(), destination, window, cx)
                })
            })
            .expect("Save As route");
        let error = save_as.await.expect_err("unsent annotation blocks Save As");
        assert!(error.to_string().contains("annotation"));
        assert!(
            file_system
                .metadata(&destination_path)
                .await
                .expect("destination metadata")
                .is_none()
        );
        fixture.view.update(cx, |view, cx| {
            assert_eq!(
                view.annotation_state
                    .controller
                    .draft()
                    .expect("kept draft")
                    .annotation()
                    .text,
                "Unsent exact\n🟠 draft"
            );
            assert!(view.cancel_annotation(Some(generation), cx));
        });
        let next_generation = start_draft(&fixture, cx);
        type_draft(&fixture, "New draft", cx);
        fixture.view.update(cx, |view, cx| {
            assert!(!view.cancel_annotation(Some(generation), cx));
            view.submit_annotation(generation, cx);
            assert_eq!(
                view.annotation_state
                    .controller
                    .draft()
                    .expect("new draft retained")
                    .generation(),
                next_generation
            );
            assert_eq!(
                view.annotation_state
                    .controller
                    .draft()
                    .expect("new draft retained")
                    .annotation()
                    .text,
                "New draft"
            );
        });
    }

    #[gpui::test]
    async fn annotation_inspect_reads_and_copies_but_all_host_mutations_stay_inert(
        cx: &mut TestAppContext,
    ) {
        let fixture = fixture(cx).await;
        let note = DeveloperAnnotation::new(
            [0.0, 0.0],
            "Read only note".into(),
            "Other author".into(),
            7,
        )
        .expect("note");
        fixture.item.update(cx, |item, cx| {
            let operation = create_annotation_op(item.doc().expect("doc"), fixture.page, &note)
                .expect("create");
            item.apply(operation, cx).expect("add note");
        });
        let baseline = fixture.item.read_with(cx, |item, _| {
            let doc = item.doc().expect("doc");
            (
                serde_json::to_value(&doc.scene).expect("scene"),
                doc.history.undo_depth(),
            )
        });
        fixture
            .scratch
            .update(cx, |_, window, cx| {
                fixture.view.update(cx, |view, cx| {
                    view.activate_tool(ToolKind::Inspect, cx);
                    let record = view
                        .page_annotations(cx)
                        .first()
                        .expect("visible note")
                        .clone();
                    view.select_annotation(record.clone(), cx);
                    view.copy_selected_nodes(cx);
                    view.delete_selected_nodes(cx);
                    view.cut_selected_nodes(cx);
                    view.edit_annotation(record, window, cx);
                    assert!(!view.annotation_state.controller.has_pending_authoring());
                    assert!(!view.is_editable(cx));
                    assert!(view.item.read(cx).is_editable());
                    assert!(!view.can_edit_annotations(cx));
                })
            })
            .expect("read-only note commands");
        cx.update(|cx| {
            assert_eq!(
                cx.read_from_clipboard()
                    .and_then(|clipboard| clipboard.text()),
                Some("Read only note".to_owned())
            )
        });
        fixture.item.read_with(cx, |item, _| {
            let doc = item.doc().expect("doc");
            assert_eq!(serde_json::to_value(&doc.scene).expect("scene"), baseline.0);
            assert_eq!(doc.history.undo_depth(), baseline.1);
        });
    }

    #[gpui::test]
    async fn annotation_conflicting_edit_and_replaced_scene_keep_recoverable_text(
        cx: &mut TestAppContext,
    ) {
        let fixture = fixture(cx).await;
        let generation = start_draft(&fixture, cx);
        type_draft(&fixture, "Original", cx);
        fixture
            .view
            .update(cx, |view, cx| view.submit_annotation(generation, cx));
        let original = fixture.view.read_with(cx, |view, cx| {
            view.selected_annotation(cx).expect("selected note")
        });
        fixture
            .scratch
            .update(cx, |_, window, cx| {
                fixture.view.update(cx, |view, cx| {
                    view.edit_annotation(original.clone(), window, cx)
                })
            })
            .expect("begin edit");
        type_draft(&fixture, "My unsent revision", cx);
        let generation = fixture.view.read_with(cx, |view, _| {
            view.annotation_state
                .controller
                .draft()
                .expect("draft")
                .generation()
        });
        fixture.item.update(cx, |item, cx| {
            let operation = update_annotation_op(
                item.doc().expect("doc"),
                &original,
                original.annotation().anchor,
                "Concurrent revision",
            )
            .expect("concurrent op")
            .expect("changed note");
            item.apply(operation, cx).expect("concurrent note change");
        });
        fixture.view.update(cx, |view, cx| {
            view.submit_annotation(generation, cx);
            assert_eq!(
                view.annotation_state
                    .controller
                    .draft()
                    .expect("conflict draft kept")
                    .annotation()
                    .text,
                "My unsent revision"
            );
            assert!(view.annotation_state.error.is_some());
            view.copy_annotation_draft(generation, cx);
        });
        cx.update(|cx| {
            assert_eq!(
                cx.read_from_clipboard()
                    .and_then(|clipboard| clipboard.text()),
                Some("My unsent revision".to_owned())
            )
        });
        fixture.item.update(cx, |item, cx| {
            item.with_document(cx, |document| {
                document.doc.scene = document.doc.scene.clone();
                ((), DocChange::Content)
            });
        });
        fixture.view.update(cx, |view, cx| {
            view.submit_annotation(generation, cx);
            assert!(view.annotation_state.controller.has_pending_authoring());
            assert!(view.annotation_state.error.is_some());
            assert!(view.cancel_annotation(Some(generation), cx));
        });
    }

    #[gpui::test]
    async fn annotation_move_uses_release_position_and_rejects_interrupted_drag(
        cx: &mut TestAppContext,
    ) {
        let fixture = fixture(cx).await;
        let generation = start_draft(&fixture, cx);
        type_draft(&fixture, "Move me", cx);
        fixture
            .view
            .update(cx, |view, cx| view.submit_annotation(generation, cx));
        let record = fixture
            .view
            .read_with(cx, |view, cx| view.selected_annotation(cx).expect("note"));
        fixture
            .scratch
            .update(cx, |_, window, cx| {
                fixture.view.update(cx, |view, cx| {
                    assert!(view.handle_annotation_mouse_down(
                        &MouseDownEvent {
                            button: MouseButton::Left,
                            position: point(px(503.), px(354.)),
                            modifiers: gpui::Modifiers::none(),
                            click_count: 1,
                            first_mouse: false
                        },
                        window,
                        cx
                    ));
                    view.handle_annotation_tool_event(
                        release_event(
                            DVec2::new(423., 314.),
                            ToolButton::Primary,
                            gpui::Modifiers::none(),
                        ),
                        cx,
                    );
                    let moved = view.selected_annotation(cx).expect("moved note");
                    assert_eq!(moved.annotation().anchor, [20.0, 10.0]);
                    assert_eq!(
                        view.item.read(cx).doc().expect("doc").history.undo_depth(),
                        2
                    );
                    assert!(view.handle_annotation_mouse_down(
                        &MouseDownEvent {
                            button: MouseButton::Left,
                            position: point(px(520.), px(360.)),
                            modifiers: gpui::Modifiers::none(),
                            click_count: 1,
                            first_mouse: false
                        },
                        window,
                        cx
                    ));
                    view.handle_annotation_tool_event(
                        move_event(DVec2::new(450., 330.), gpui::Modifiers::none()),
                        cx,
                    );
                    view.handle_window_mouse_move(
                        &MouseMoveEvent {
                            position: point(px(550.), px(380.)),
                            pressed_button: None,
                            modifiers: gpui::Modifiers::none(),
                        },
                        cx,
                    );
                    assert!(!view.primary_pressed);
                    let generation = view
                        .annotation_state
                        .controller
                        .draft()
                        .expect("frozen draft")
                        .generation();
                    view.submit_annotation(generation, cx);
                    assert!(view.annotation_state.controller.has_pending_authoring());
                    assert_eq!(
                        view.selected_annotation(cx)
                            .expect("persisted note")
                            .annotation()
                            .anchor,
                        moved.annotation().anchor
                    );
                    view.cancel_annotation(Some(generation), cx);
                })
            })
            .expect("drag and freeze");
        fixture
            .item
            .update(cx, |item, cx| item.undo(cx).expect("undo movement"));
        fixture.item.read_with(cx, |item, _| {
            assert_eq!(
                read_annotations(item.doc().expect("doc"), fixture.page)
                    .expect("notes")
                    .first()
                    .expect("note")
                    .annotation(),
                record.annotation()
            )
        });
    }
    #[gpui::test]
    async fn annotation_source_lock_hidden_page_and_deactivation_keep_drafts_recoverable(
        cx: &mut TestAppContext,
    ) {
        let fixture = fixture(cx).await;
        let generation = start_draft(&fixture, cx);
        type_draft(&fixture, "Keep through external changes", cx);
        fixture
            .item
            .update(cx, |item, cx| item.set_source_edit_locked(true, cx));
        cx.run_until_parked();
        fixture.view.update(cx, |view, cx| {
            view.submit_annotation(generation, cx);
            assert!(view.annotation_state.controller.has_pending_authoring());
            assert!(!view.can_edit_annotations(cx));
        });
        fixture.item.update(cx, |item, cx| {
            item.set_source_edit_locked(false, cx);
            item.with_document(cx, |document| {
                document.doc.set_active_page(Some(fixture.other_page));
                ((), DocChange::Selection)
            });
        });
        fixture.view.update(cx, |view, cx| {
            view.submit_annotation(generation, cx);
            assert!(view.annotation_state.controller.has_pending_authoring());
        });
        fixture.item.update(cx, |item, cx| {
            item.with_document(cx, |document| {
                document.doc.set_active_page(Some(fixture.page));
                document
                    .pages
                    .iter_mut()
                    .find(|page| page.root == Some(fixture.page))
                    .expect("registered page")
                    .hidden = true;
                ((), DocChange::Selection)
            });
        });
        fixture.view.update(cx, |view, cx| {
            assert!(view.page_annotations(cx).is_empty());
            view.submit_annotation(generation, cx);
            assert!(view.annotation_state.controller.has_pending_authoring());
        });
        fixture.item.update(cx, |item, cx| {
            item.with_document(cx, |document| {
                document
                    .pages
                    .iter_mut()
                    .find(|page| page.root == Some(fixture.page))
                    .expect("registered page")
                    .hidden = false;
                ((), DocChange::Selection)
            });
        });
        fixture
            .scratch
            .update(cx, |_, window, cx| {
                fixture.view.update(cx, |view, cx| {
                    Item::deactivated(view, window, cx);
                    Item::workspace_deactivated(view, window, cx);
                    assert_eq!(
                        view.annotation_state
                            .controller
                            .draft()
                            .expect("retained text")
                            .annotation()
                            .text,
                        "Keep through external changes"
                    );
                    view.submit_annotation(generation, cx);
                    assert!(!view.annotation_state.controller.has_pending_authoring());
                    assert_eq!(
                        view.selected_annotation(cx)
                            .expect("saved note")
                            .annotation()
                            .text,
                        "Keep through external changes"
                    );
                });
            })
            .expect("deactivation and explicit recovery");
        fixture.item.read_with(cx, |item, _| {
            assert_eq!(item.doc().expect("doc").history.undo_depth(), 1);
            assert!(
                read_annotations(item.doc().expect("doc"), fixture.other_page)
                    .expect("other page")
                    .is_empty()
            );
        });
    }

    #[gpui::test]
    async fn annotation_selected_pin_does_not_move_or_delete_with_an_unsent_reply(
        cx: &mut TestAppContext,
    ) {
        let fixture = fixture(cx).await;
        let generation = start_draft(&fixture, cx);
        type_draft(&fixture, "Selected annotation", cx);
        fixture
            .view
            .update(cx, |view, cx| view.submit_annotation(generation, cx));
        let before = fixture.item.read_with(cx, |item, _| {
            serde_json::to_value(item.doc().expect("doc")).expect("snapshot")
        });
        fixture
            .scratch
            .update(cx, |_, window, cx| {
                fixture.view.update(cx, |view, cx| {
                    let reply = cx.new(|cx| {
                        let mut editor = Editor::auto_height(1, 4, window, cx);
                        editor.set_text("Unsent comment reply", window, cx);
                        editor
                    });
                    view.comment_state.reply_editor = Some(reply.clone());
                    assert!(view.has_unsent_comment_reply(cx));
                    assert!(view.handle_annotation_mouse_down(
                        &MouseDownEvent {
                            button: MouseButton::Left,
                            position: point(px(500.), px(350.)),
                            modifiers: gpui::Modifiers::none(),
                            click_count: 1,
                            first_mouse: false,
                        },
                        window,
                        cx
                    ));
                    assert!(!view.annotation_state.controller.has_pending_authoring());
                    assert!(!view.primary_pressed);
                    view.delete_selected_nodes(cx);
                    view.select_all(&SelectAll, window, cx);
                    assert_eq!(reply.read(cx).text(cx), "Unsent comment reply");
                    assert!(view.item.read(cx).doc().expect("doc").selection.is_empty());
                });
            })
            .expect("same-selected-pin guards");
        fixture.item.read_with(cx, |item, _| {
            assert_eq!(
                serde_json::to_value(item.doc().expect("doc")).expect("snapshot"),
                before
            );
        });
    }

    #[gpui::test]
    async fn annotation_submit_buttons_restore_canvas_shortcuts_and_editor_typing_stays_local(
        cx: &mut TestAppContext,
    ) {
        let fixture = fixture(cx).await;
        let (view, cx) = cx.add_window_view(|window, cx| {
            FigView::new(fixture.item.clone(), fixture.project.clone(), window, cx)
        });
        cx.update(|window, cx| {
            window.activate_window();
            let bindings = settings::KeymapFile::load_asset_allow_partial_failure(
                settings::DEFAULT_KEYMAP_PATH,
                cx,
            )
            .expect("canvas bindings");
            cx.bind_keys(bindings);
        });
        cx.run_until_parked();
        view.update_in(cx, |view, window, cx| {
            view.select_page(0, cx);
            view.activate_tool(ToolKind::Annotation, cx);
            let bounds = view.container_bounds.expect("mounted canvas bounds");
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
        cx.simulate_keystrokes("shift-t");
        cx.run_until_parked();
        view.read_with(cx, |view, cx| {
            assert_eq!(
                view.annotation_state
                    .controller
                    .draft()
                    .expect("new note")
                    .annotation()
                    .text,
                "T"
            );
            assert_eq!(
                view.annotation_state
                    .editor
                    .as_ref()
                    .expect("editor")
                    .read(cx)
                    .text(cx),
                "T"
            );
        });
        let submit = cx
            .debug_bounds("annotation-submit-target")
            .expect("Add target");
        cx.simulate_click(submit.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        view.update_in(cx, |view, window, cx| {
            assert!(!view.annotation_state.controller.has_pending_authoring());
            assert!(view.focus_handle.is_focused(window));
            assert_eq!(
                view.selected_annotation(cx)
                    .expect("added note")
                    .annotation()
                    .text,
                "T"
            );
        });
        let edit = cx
            .debug_bounds("annotation-edit-target")
            .expect("Edit target");
        cx.simulate_click(edit.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        view.update_in(cx, |view, window, cx| {
            view.annotation_state
                .editor
                .as_ref()
                .expect("edit editor")
                .clone()
                .update(cx, |editor, cx| {
                    editor.set_text("Saved revision", window, cx)
                });
        });
        cx.run_until_parked();
        let submit = cx
            .debug_bounds("annotation-submit-target")
            .expect("Save target");
        cx.simulate_click(submit.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        view.update_in(cx, |view, window, _| {
            assert!(!view.annotation_state.controller.has_pending_authoring());
            assert!(view.focus_handle.is_focused(window));
        });
        cx.simulate_keystrokes("v");
        cx.run_until_parked();
        assert_eq!(
            view.read_with(cx, |view, _| view.tools.kind()),
            ToolKind::Select
        );
        #[cfg(target_os = "macos")]
        cx.simulate_keystrokes("cmd-z");
        #[cfg(not(target_os = "macos"))]
        cx.simulate_keystrokes("ctrl-z");
        cx.run_until_parked();
        fixture.item.read_with(cx, |item, _| {
            let doc = item.doc().expect("doc");
            assert_eq!(doc.history.undo_depth(), 1);
            assert_eq!(
                read_annotations(doc, fixture.page)
                    .expect("notes")
                    .first()
                    .expect("note")
                    .annotation()
                    .text,
                "T"
            );
        });
    }

    #[gpui::test]
    async fn annotation_view_local_close_blocker_keeps_owner_and_allows_sibling_close(
        cx: &mut TestAppContext,
    ) {
        let fixture = fixture(cx).await;
        let item = fixture.item.clone();
        let project = fixture.project.clone();
        let (workspace, cx) = cx.add_window_view(|window, cx| {
            workspace::Workspace::test_new(project.clone(), window, cx)
        });
        let (owner, sibling) = cx.update(|window, cx| {
            window.activate_window();
            let owner = cx.new(|cx| FigView::new(item.clone(), project.clone(), window, cx));
            let sibling = cx.new(|cx| FigView::new(item.clone(), project.clone(), window, cx));
            owner.update(cx, |view, _| {
                view.opened_entry_id = Some(ProjectEntryId::from_proto(1))
            });
            sibling.update(cx, |view, _| {
                view.opened_entry_id = Some(ProjectEntryId::from_proto(2))
            });
            (owner, sibling)
        });
        let pane = workspace.read_with(cx, |workspace, _| workspace.active_pane().clone());
        pane.update_in(cx, |pane, window, cx| {
            pane.add_item(Box::new(sibling.clone()), false, true, None, window, cx);
            pane.add_item(Box::new(owner.clone()), false, true, None, window, cx);
        });
        cx.run_until_parked();
        owner.update_in(cx, |view, window, cx| {
            view.select_page(0, cx);
            view.container_bounds = Some(Bounds::new(
                point(px(100.), px(50.)),
                size(px(800.), px(600.)),
            ));
            view.viewport = Some(Viewport::default());
            view.activate_tool(ToolKind::Annotation, cx);
            view.handle_mouse_down(
                &MouseDownEvent {
                    button: MouseButton::Left,
                    position: point(px(500.), px(350.)),
                    modifiers: gpui::Modifiers::none(),
                    click_count: 1,
                    first_mouse: false,
                },
                window,
                cx,
            );
            view.annotation_state
                .editor
                .as_ref()
                .expect("new note editor")
                .clone()
                .update(cx, |editor, cx| {
                    editor.set_text("Keep my unsent note", window, cx)
                });
        });
        cx.run_until_parked();
        let draft = owner.read_with(cx, |view, cx| {
            assert!(view.close_blocker(cx).is_some());
            view.annotation_state
                .controller
                .draft()
                .cloned()
                .expect("recoverable note")
        });
        let before = item.read_with(cx, |item, _| {
            assert!(
                !item.is_dirty(),
                "local preview does not spoof shared document dirtiness"
            );
            serde_json::to_value(item.doc().expect("document")).expect("snapshot")
        });
        for intent in [workspace::SaveIntent::Close, workspace::SaveIntent::Skip] {
            pane.update_in(cx, |pane, window, cx| {
                pane.close_item_by_id(owner.entity_id(), intent, window, cx)
            })
            .await
            .expect("blocked owner close");
            pane.read_with(cx, |pane, _| {
                assert_eq!(pane.items_len(), 2);
                assert_eq!(
                    pane.active_item()
                        .expect("owner remains reachable")
                        .item_id(),
                    owner.entity_id()
                );
            });
            owner.read_with(cx, |view, _| {
                assert_eq!(view.annotation_state.controller.draft(), Some(&draft));
            });
        }
        assert!(
            !workspace
                .update_in(cx, |workspace, window, cx| {
                    workspace.prepare_to_close(workspace::CloseIntent::Quit, window, cx)
                })
                .await
                .expect("quit preflight")
        );
        pane.update_in(cx, |pane, window, cx| {
            pane.close_item_by_id(
                sibling.entity_id(),
                workspace::SaveIntent::Close,
                window,
                cx,
            )
        })
        .await
        .expect("sibling close");
        pane.read_with(cx, |pane, _| assert_eq!(pane.items_len(), 1));
        owner.read_with(cx, |view, _| {
            assert_eq!(view.annotation_state.controller.draft(), Some(&draft));
        });
        item.read_with(cx, |item, _| {
            assert!(!item.is_dirty());
            assert_eq!(
                serde_json::to_value(item.doc().expect("document")).expect("snapshot"),
                before
            );
            assert_eq!(item.doc().expect("document").history.undo_depth(), 0);
        });
        assert!(!cx.has_pending_prompt());
        owner.update(cx, |view, cx| assert!(view.cancel_annotation(None, cx)));
        pane.update_in(cx, |pane, window, cx| {
            pane.close_item_by_id(owner.entity_id(), workspace::SaveIntent::Close, window, cx)
        })
        .await
        .expect("close after explicit draft cancellation");
        pane.read_with(cx, |pane, _| assert_eq!(pane.items_len(), 0));
    }
}
