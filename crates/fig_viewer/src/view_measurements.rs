use super::*;

impl FigView {
    pub(super) fn mark_page(&self, cx: &App) -> Option<NodeId> {
        if !self.is_design_canvas_mode(cx)
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
        let node = document.doc.scene.get(page)?;
        (document.doc.pages().contains(&page)
            && !document.doc.is_component_root(page)
            && matches!(node.data, NodeData::Group(_))
            && !node.flags.contains(fanta_doc::NodeFlags::HIDDEN)
            && document
                .pages
                .iter()
                .any(|candidate| candidate.root == Some(page) && !candidate.hidden))
        .then_some(page)
    }

    pub(crate) fn can_edit_measurements(&self, cx: &App) -> bool {
        !self.is_inspecting() && self.item.read(cx).is_editable() && self.mark_page(cx).is_some()
    }

    pub(crate) fn page_measurements(&self, cx: &App) -> Vec<MeasurementRecord> {
        let Some(page) = self.mark_page(cx) else {
            return Vec::new();
        };
        let Some(doc) = self.item.read(cx).doc() else {
            return Vec::new();
        };
        let mut cache = self.measurement_cache.borrow_mut();
        if let Some(cache) = cache.as_ref()
            && cache.scene == doc.scene.instance_id()
            && cache.revision == doc.scene.revision()
            && cache.page == page
        {
            return cache.records.clone();
        }
        let records = match read_measurements(doc, page) {
            Ok(records) => records,
            Err(error) => {
                log::warn!("Could not read page measurements: {error:#}");
                Vec::new()
            }
        };
        *cache = Some(MeasurementCache {
            scene: doc.scene.instance_id(),
            revision: doc.scene.revision(),
            page,
            records: records.clone(),
        });
        records
    }

    pub(crate) fn selected_measurement(&self, cx: &App) -> Option<MeasurementRecord> {
        let selection = self.measurement_selection.as_ref()?;
        let doc = self.item.read(cx).doc()?;
        if doc.id != selection.document
            || doc.scene.instance_id() != selection.scene
            || doc.active_page() != Some(selection.page)
        {
            return None;
        }
        self.page_measurements(cx)
            .into_iter()
            .find(|record| record.measurement().id == selection.id)
    }

    pub(super) fn sync_measurement_edit_barrier(&self, cx: &mut Context<Self>) {
        let read_only = self.is_art_read_only(cx);
        self.inspector_sidebar
            .update(cx, |panel, cx| panel.set_inspecting(read_only, cx));
        // Page navigation can originate inside this panel's own update.
        // Echo after its lease is released, using the final tool/page state.
        let view = cx.weak_entity();
        cx.defer(move |cx| {
            let Some(view) = view.upgrade() else {
                return;
            };
            let (panel, read_only) = {
                let view = view.read(cx);
                (view.layers_sidebar.clone(), view.is_art_read_only(cx))
            };
            panel.update(cx, |panel, cx| panel.set_inspecting(read_only, cx));
        });
    }

    pub(crate) fn select_measurement(
        &mut self,
        expected: MeasurementRecord,
        cx: &mut Context<Self>,
    ) {
        if self.has_pending_authoring_except_pointer(cx)
            || self.comment_state.draft.is_some()
            || self.has_unsent_comment_reply(cx)
        {
            show_canvas_notice_deferred(
                "Finish or cancel the measurement drag before selecting another measurement."
                    .into(),
                cx,
            );
            return;
        }
        let Some(current) = self
            .page_measurements(cx)
            .into_iter()
            .find(|record| record == &expected)
        else {
            return;
        };
        if !matches!(self.tools.kind(), ToolKind::Measure | ToolKind::Inspect) {
            let kind = if self.can_edit_measurements(cx) {
                ToolKind::Measure
            } else {
                ToolKind::Inspect
            };
            self.activate_tool(kind, cx);
            if self.tools.kind() != kind {
                return;
            }
        }
        let Some(doc) = self.item.read(cx).doc() else {
            return;
        };
        self.annotation_state.selection = None;
        self.measurement_selection = Some(MeasurementSelection {
            document: doc.id,
            scene: doc.scene.instance_id(),
            page: current.page(),
            id: current.measurement().id.clone(),
        });
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
        self.inspector_sidebar_visible = true;
        self.sync_measurement_edit_barrier(cx);
        cx.notify();
    }

    pub(crate) fn copy_measurement(
        &mut self,
        expected: &MeasurementRecord,
        cx: &mut Context<Self>,
    ) {
        if self.selected_measurement(cx).as_ref() != Some(expected) {
            return;
        }
        match expected.measurement().label() {
            Ok(label) => cx.write_to_clipboard(ClipboardItem::new_string(label)),
            Err(error) => show_canvas_notice_deferred(error.to_string(), cx),
        }
    }

    pub(crate) fn delete_measurement(
        &mut self,
        expected: &MeasurementRecord,
        cx: &mut Context<Self>,
    ) {
        if !self.can_edit_measurements(cx)
            || self.measurement_controller.has_pending_authoring()
            || self.selected_measurement(cx).as_ref() != Some(expected)
        {
            return;
        }
        let owner = cx.entity_id();
        let result = self.item.update(cx, |item, cx| -> Result<()> {
            anyhow::ensure!(
                item.can_preview_for_owner(owner) && !item.content_preview_active(),
                "Finish saving or editing this document before deleting its measurement."
            );
            let doc = item.doc().context("The document is no longer available.")?;
            let operation = delete_measurement_op(doc, expected)?;
            item.apply_for_preview_owner(owner, operation, cx)
        });
        match result {
            Ok(()) => {
                self.measurement_selection = None;
                self.sync_measurement_edit_barrier(cx);
                cx.notify();
            }
            Err(error) => {
                show_canvas_notice_deferred(format!("Could not delete measurement: {error:#}"), cx)
            }
        }
    }

    pub(crate) fn measurement_overlays(
        &self,
        viewport: Viewport,
        screen_size: [f64; 2],
        cx: &App,
    ) -> Vec<MeasurementOverlay> {
        let Some(doc) = self.item.read(cx).doc() else {
            return Vec::new();
        };
        let Some(page) = doc.active_page() else {
            return Vec::new();
        };
        let Some(transform) = doc.scene.world_transform(page) else {
            return Vec::new();
        };
        let Ok(projection) = MeasurementProjection::new(transform, viewport, screen_size) else {
            return Vec::new();
        };
        let selected = self
            .selected_measurement(cx)
            .map(|record| record.measurement().id.clone());
        let draft = self
            .measurement_controller
            .draft()
            .filter(|draft| draft.belongs_to(doc));
        let edited_id = draft.and_then(|draft| draft.preview().and_then(|_| draft.edited_id()));
        let handles = self.tools.kind() == ToolKind::Measure && self.can_edit_measurements(cx);
        let mut overlays: Vec<_> = self
            .page_measurements(cx)
            .into_iter()
            .filter_map(|record| {
                let measurement = record.measurement();
                if edited_id == Some(measurement.id.as_str()) {
                    return None;
                }
                let selected = selected.as_deref() == Some(measurement.id.as_str());
                Some(MeasurementOverlay {
                    id: Some(measurement.id.clone()),
                    screen: projection
                        .project(MeasurementGeometry::from(measurement))
                        .ok()?,
                    label: measurement.label().ok()?,
                    selected,
                    show_handles: selected && handles,
                    preview: false,
                })
            })
            .collect();
        overlays.sort_by_key(|overlay| overlay.selected);
        if let Some(draft) = draft
            && let Some(geometry) = draft.preview()
            && let Ok(screen) = projection.project(geometry)
        {
            let label = match geometry.label() {
                Ok(label) => label,
                Err(error) => {
                    log::warn!("Invalid measurement preview: {error:#}");
                    return overlays;
                }
            };
            overlays.push(MeasurementOverlay {
                id: draft.edited_id().map(str::to_owned),
                screen,
                label,
                selected: true,
                show_handles: handles,
                preview: true,
            });
        }
        overlays
    }

    pub(super) fn measurement_host_origin(&self, cx: &Context<Self>) -> Result<LocalMediaOrigin> {
        anyhow::ensure!(
            self.can_edit_measurements(cx),
            "Measurements can be edited on an editable Design page."
        );
        let item = self.item.read(cx);
        anyhow::ensure!(
            item.can_preview_for_owner(cx.entity_id()) && !item.content_preview_active(),
            "Finish saving or editing this document before editing measurements."
        );
        let doc = item.doc().context("The document is no longer available.")?;
        let page = self
            .mark_page(cx)
            .context("Choose a visible ordinary page before measuring.")?;
        Ok(LocalMediaOrigin {
            item: self.item.entity_id(),
            scene: doc.scene.instance_id(),
            page,
            scope: self.scope,
            mode: self.editor_mode(cx),
            path: item.abs_path().to_path_buf(),
            generation: self.media_import_generation.get(),
        })
    }

    pub(super) fn measurement_projection(&self, cx: &App) -> Result<MeasurementProjection> {
        let doc = self
            .item
            .read(cx)
            .doc()
            .context("The document is no longer available.")?;
        let page = doc
            .active_page()
            .context("Choose a page before measuring.")?;
        let transform = doc
            .scene
            .world_transform(page)
            .context("The page transform is unavailable.")?;
        let viewport = self
            .viewport
            .context("The canvas viewport is unavailable.")?;
        let bounds = self
            .container_bounds
            .context("The canvas bounds are unavailable.")?;
        let (width, height) = bounds_size(bounds);
        MeasurementProjection::new(transform, viewport, [width, height])
    }

    pub(super) fn cancel_measurement_drag(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.measurement_controller.cancel() {
            return false;
        }
        self.measurement_origin = None;
        self.primary_pressed = false;
        self.canvas_pointer_down = false;
        self.schedule_autosave(cx);
        cx.notify();
        true
    }

    pub(super) fn freeze_measurement_drag(&mut self, cx: &mut Context<Self>) {
        if !self.measurement_controller.has_pending_authoring() {
            return;
        }
        self.measurement_controller.freeze_after_release_error();
        self.primary_pressed = false;
        self.canvas_pointer_down = false;
        cx.notify();
    }

    pub(super) fn refuse_pending_measurement(&self, cx: &mut Context<Self>) -> bool {
        if !self.measurement_controller.has_pending_authoring() {
            return false;
        }
        show_canvas_notice_deferred(
            "Finish or cancel the measurement drag first. Its preview was kept.".into(),
            cx,
        );
        true
    }

    pub(super) fn handle_measurement_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if event.button != MouseButton::Left
            || self.space_pan
            || !matches!(self.tools.kind(), ToolKind::Measure | ToolKind::Inspect)
            || self.comment_state.draft.is_some()
            || self.has_unsent_comment_reply(cx)
        {
            return false;
        }
        let Some(bounds) = self.container_bounds else {
            return false;
        };
        let Some(viewport) = self.viewport else {
            return false;
        };
        let (width, height) = bounds_size(bounds);
        let screen = screen_position_in_bounds(event.position, bounds).to_array();
        let hit = self
            .measurement_overlays(viewport, [width, height], cx)
            .into_iter()
            .rev()
            .filter(|overlay| !overlay.preview)
            .find_map(|overlay| {
                let label = crate::canvas::measurement_label_bounds(
                    &overlay.screen,
                    &overlay.label,
                    window,
                );
                let hit = overlay
                    .screen
                    .hit_test(screen, 6.0, overlay.show_handles, Some(label))
                    .ok()??;
                Some((overlay.id?, hit))
            });
        let Some((id, hit)) = hit else { return false };
        let Some(record) = self
            .page_measurements(cx)
            .into_iter()
            .find(|record| record.measurement().id == id)
        else {
            return false;
        };
        self.select_measurement(record.clone(), cx);
        if self.tools.kind() == ToolKind::Measure {
            let result = (|| -> Result<()> {
                let origin = self.measurement_host_origin(cx)?;
                let projection = self.measurement_projection(cx)?;
                let kind = match hit {
                    MeasurementHit::StartEndpoint => MeasurementDragKind::StartEndpoint,
                    MeasurementHit::EndEndpoint => MeasurementDragKind::EndEndpoint,
                    MeasurementHit::Label | MeasurementHit::Segment => MeasurementDragKind::Move,
                };
                let doc = self
                    .item
                    .read(cx)
                    .doc()
                    .context("The document is no longer available.")?;
                self.measurement_controller
                    .begin_edit(doc, &record, kind, projection, screen)?;
                self.measurement_origin = Some(origin);
                self.primary_pressed = true;
                Ok(())
            })();
            if let Err(error) = result {
                show_canvas_notice_deferred(error.to_string(), cx);
            }
        }
        cx.notify();
        true
    }

    pub(super) fn apply_measurement_intent(
        &mut self,
        intent: Option<MeasurementCommit>,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        anyhow::ensure!(
            self.measurement_origin.as_ref() == Some(&self.measurement_host_origin(cx)?),
            "The measurement's document or page changed. Cancel its preview and start again."
        );
        if let Some(intent) = intent {
            let owner = cx.entity_id();
            let controller = &self.measurement_controller;
            let id = self.item.update(cx, |item, cx| -> Result<Option<String>> {
                anyhow::ensure!(
                    item.can_preview_for_owner(owner) && !item.content_preview_active(),
                    "Finish saving or editing this document and retry the measurement."
                );
                let doc = item.doc().context("The document is no longer available.")?;
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .context("The system clock is before the Unix epoch.")?
                    .as_secs();
                let Some((id, operation)) = controller.build_operation(
                    doc,
                    &intent,
                    crate::comments::author_name(),
                    now,
                )?
                else {
                    return Ok(None);
                };
                item.apply_for_preview_owner(owner, operation, cx)?;
                Ok(Some(id))
            })?;
            self.cancel_measurement_drag(cx);
            if let Some(id) = id
                && let Some(record) = self
                    .page_measurements(cx)
                    .into_iter()
                    .find(|record| record.measurement().id == id)
            {
                self.select_measurement(record, cx);
            }
        } else {
            self.cancel_measurement_drag(cx);
        }
        Ok(())
    }

    pub(super) fn handle_measurement_tool_event(
        &mut self,
        event: ToolEvent,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.tools.kind() != ToolKind::Measure {
            return false;
        }
        let result = (|| -> Result<()> {
            use fanta_tools::{ModifierKeys, PointerEvent};
            match event {
                ToolEvent::Pointer(PointerEvent::Press {
                    screen,
                    button: ToolButton::Primary,
                    ..
                }) => {
                    anyhow::ensure!(
                        !self.has_pending_authoring_except_pointer(cx)
                            && self.comment_state.draft.is_none()
                            && !self.has_unsent_comment_reply(cx),
                        "Finish or cancel the current edit before placing a measurement."
                    );
                    let origin = self.measurement_host_origin(cx)?;
                    let projection = self.measurement_projection(cx)?;
                    let doc = self
                        .item
                        .read(cx)
                        .doc()
                        .context("The document is no longer available.")?;
                    self.measurement_controller.begin_create(
                        doc,
                        origin.page,
                        projection,
                        screen,
                    )?;
                    self.measurement_origin = Some(origin);
                    self.measurement_selection = None;
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
                }
                ToolEvent::Pointer(PointerEvent::Move { screen, modifiers })
                    if self.measurement_controller.is_dragging() =>
                {
                    anyhow::ensure!(
                        self.measurement_origin.as_ref()
                            == Some(&self.measurement_host_origin(cx)?),
                        "The measurement's document or page changed. Cancel its preview and start again."
                    );
                    let projection = self.measurement_projection(cx)?;
                    let doc = self
                        .item
                        .read(cx)
                        .doc()
                        .context("The document is no longer available.")?;
                    self.measurement_controller.update_pointer(
                        doc,
                        projection,
                        screen,
                        modifiers.contains(ModifierKeys::SHIFT),
                    )?;
                }
                ToolEvent::Pointer(PointerEvent::Release {
                    screen,
                    button: ToolButton::Primary,
                    modifiers,
                }) if self.measurement_controller.is_dragging() => {
                    let projection = self.measurement_projection(cx)?;
                    let doc = self
                        .item
                        .read(cx)
                        .doc()
                        .context("The document is no longer available.")?;
                    let intent = self.measurement_controller.release_pointer(
                        doc,
                        projection,
                        screen,
                        modifiers.contains(ModifierKeys::SHIFT),
                    )?;
                    self.apply_measurement_intent(intent, cx)?;
                }
                _ => return Ok(()),
            }
            cx.notify();
            Ok(())
        })();
        if let Err(error) = result {
            if self.measurement_controller.is_dragging()
                && matches!(
                    event,
                    ToolEvent::Pointer(
                        fanta_tools::PointerEvent::Move { .. }
                            | fanta_tools::PointerEvent::Release { .. }
                    )
                )
            {
                self.measurement_controller.freeze_after_release_error();
                self.primary_pressed = false;
            }
            show_canvas_notice_deferred(
                format!("Measurement: {error:#} Press Escape to cancel the preview."),
                cx,
            );
        }
        true
    }
}
