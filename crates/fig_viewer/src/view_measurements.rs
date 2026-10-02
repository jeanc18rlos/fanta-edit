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

#[cfg(test)]
mod tests {
    use super::*;
    use fanta_doc::{CanvasNode, Color, GroupNode, NodeData, Operation, VectorNode};
    use gpui::{TestAppContext, point, size};
    use project::FakeFs;
    use settings::SettingsStore;

    fn init_visual_test(cx: &mut TestAppContext) {
        cx.update(|cx| {
            zlog::init_test();
            assets::Assets.load_test_fonts(cx);
            let settings_store = SettingsStore::test(cx);
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
        });
    }

    fn doc_with_one_page() -> fanta_doc::Doc {
        let mut doc = fanta_doc::Doc::new();
        let mut page = CanvasNode::new(NodeData::Group(GroupNode::default()));
        page.name = "Page 1".to_owned();
        let root = page.id;
        doc.apply(Operation::create_node(page))
            .expect("create page root node");
        doc.add_page(root);
        doc.set_active_page(Some(root));
        doc
    }

    async fn autosave_fixture(
        cx: &mut TestAppContext,
    ) -> (
        tempfile::TempDir,
        std::path::PathBuf,
        Entity<FigItem>,
        Entity<FigView>,
    ) {
        init_visual_test(cx);
        let project = Project::test(FakeFs::new(cx.executor()), [], cx).await;
        let dir = tempfile::tempdir().expect("temp dir");
        let root = dir.path().join("Design");
        let initial = doc_with_one_page();
        crate::document::write_project(&root, &initial, &BTreeMap::new())
            .expect("materialize project fixture");
        let item = crate::document::ready_item_with_root_for_test(
            &project,
            dir.path().join("Design.fig"),
            Some(root.clone()),
            initial,
            cx,
        );
        let scratch = cx.add_window(|_, _| gpui::Empty);
        let view = scratch
            .update(cx, |_, window, cx| {
                cx.new(|cx| FigView::new(item.clone(), project.clone(), window, cx))
            })
            .expect("create fig view");
        (dir, root, item, view)
    }

    async fn measurement_view_fixture(
        cx: &mut TestAppContext,
    ) -> (
        Entity<FigItem>,
        Entity<FigView>,
        gpui::WindowHandle<gpui::Empty>,
        NodeId,
    ) {
        init_visual_test(cx);
        let project = Project::test(FakeFs::new(cx.executor()), [], cx).await;
        let mut doc = doc_with_one_page();
        let page = doc.active_page().expect("page");
        let mut rectangle = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
            0.0,
            0.0,
            100.0,
            100.0,
            Color::BLACK,
        )));
        rectangle.parent = Some(page);
        let node = rectangle.id;
        doc.apply(Operation::create_node(rectangle))
            .expect("art layer");
        doc.selection.select_only(node);
        doc.history = Default::default();
        let item =
            crate::document::ready_item_for_test(&project, "/tmp/Measurement.fig".into(), doc, cx);
        let scratch = cx.add_window(|_, _| gpui::Empty);
        let view = scratch
            .update(cx, |_, window, cx| {
                cx.new(|cx| FigView::new(item.clone(), project.clone(), window, cx))
            })
            .expect("measurement view");
        cx.run_until_parked();
        view.update(cx, |view, cx| {
            view.select_page(0, cx);
            view.container_bounds = Some(Bounds::new(
                point(px(100.), px(50.)),
                size(px(800.), px(600.)),
            ));
            view.viewport = Some(Viewport::default());
        });
        (item, view, scratch, node)
    }

    fn measurement_pointer_down(
        view: &mut FigView,
        screen: [f64; 2],
        window: &mut Window,
        cx: &mut Context<FigView>,
    ) {
        view.handle_mouse_down(
            &MouseDownEvent {
                button: MouseButton::Left,
                position: point(px(screen[0] as f32 + 100.), px(screen[1] as f32 + 50.)),
                modifiers: gpui::Modifiers::none(),
                click_count: 1,
                first_mouse: false,
            },
            window,
            cx,
        );
    }

    fn measurement_pointer_up(
        view: &mut FigView,
        screen: [f64; 2],
        window: &mut Window,
        cx: &mut Context<FigView>,
    ) {
        view.handle_mouse_up(
            &MouseUpEvent {
                button: MouseButton::Left,
                position: point(px(screen[0] as f32 + 100.), px(screen[1] as f32 + 50.)),
                modifiers: gpui::Modifiers::none(),
                click_count: 1,
            },
            window,
            cx,
        );
    }

    #[gpui::test]
    async fn measurement_native_pointer_routes_create_edit_move_copy_delete_and_undo(
        cx: &mut TestAppContext,
    ) {
        let (item, view, scratch, node) = measurement_view_fixture(cx).await;
        let original_node = item.read_with(cx, |item, _| {
            item.doc()
                .expect("doc")
                .scene
                .get(node)
                .expect("art")
                .clone()
        });
        scratch
            .update(cx, |_, window, cx| {
                view.update(cx, |view, cx| {
                    view.activate_tool(ToolKind::Measure, cx);
                    assert_eq!(view.tools.kind(), ToolKind::Measure);
                    assert!(!view.is_editable(cx));
                    assert!(view.can_edit_measurements(cx));
                    measurement_pointer_down(view, [400., 300.], window, cx);
                    view.handle_window_mouse_move(
                        &MouseMoveEvent {
                            position: point(px(540.), px(380.)),
                            pressed_button: Some(MouseButton::Left),
                            modifiers: gpui::Modifiers::none(),
                        },
                        cx,
                    );
                    assert!(
                        view.page_measurements(cx).is_empty(),
                        "preview is not persisted"
                    );
                    assert_eq!(
                        view.item.read(cx).doc().expect("doc").history.undo_depth(),
                        0
                    );
                    let preview = view.measurement_overlays(Viewport::default(), [800., 600.], cx);
                    assert_eq!(preview.len(), 1);
                    assert_eq!(preview.first().expect("preview").label, "50 px");
                    measurement_pointer_up(view, [440., 330.], window, cx);
                    measurement_pointer_up(view, [440., 330.], window, cx);
                    let created = view
                        .selected_measurement(cx)
                        .expect("new selected measurement");
                    assert_eq!(created.measurement().start, [0., 0.]);
                    assert_eq!(created.measurement().end, [40., 30.]);
                    assert!(view.item.read(cx).doc().expect("doc").selection.is_empty());
                    assert_eq!(
                        view.item.read(cx).doc().expect("doc").history.undo_depth(),
                        1
                    );
                    measurement_pointer_down(view, [440., 330.], window, cx);
                    measurement_pointer_up(view, [450., 350.], window, cx);
                    let edited = view.selected_measurement(cx).expect("edited measurement");
                    assert_eq!(edited.measurement().id, created.measurement().id);
                    assert_eq!(edited.measurement().start, [0., 0.]);
                    assert_eq!(edited.measurement().end, [50., 50.]);
                    assert_eq!(
                        view.item.read(cx).doc().expect("doc").history.undo_depth(),
                        2
                    );
                    view.undo(&Undo, window, cx);
                    assert_eq!(
                        view.selected_measurement(cx)
                            .expect("undo endpoint")
                            .measurement()
                            .end,
                        [40., 30.]
                    );
                    view.redo(&Redo, window, cx);
                    measurement_pointer_down(view, [425., 325.], window, cx);
                    measurement_pointer_up(view, [445., 335.], window, cx);
                    let moved = view.selected_measurement(cx).expect("moved measurement");
                    assert_eq!(moved.measurement().start, [20., 10.]);
                    assert_eq!(moved.measurement().end, [70., 60.]);
                    assert_eq!(
                        view.item.read(cx).doc().expect("doc").history.undo_depth(),
                        3
                    );
                    view.copy_selected_nodes(cx);
                    assert_eq!(
                        cx.read_from_clipboard()
                            .and_then(|clipboard| clipboard.text()),
                        Some(moved.measurement().label().expect("label"))
                    );
                    view.cut_selected_nodes(cx);
                    view.duplicate_selected_nodes(cx);
                    view.nudge(LogicalKey::ArrowRight, window, cx);
                    view.group_selection(&GroupSelection, window, cx);
                    assert_eq!(
                        view.item.read(cx).doc().expect("doc").scene.get(node),
                        Some(&original_node)
                    );
                    assert_eq!(
                        view.item.read(cx).doc().expect("doc").history.undo_depth(),
                        3
                    );
                    view.delete_selection(&DeleteSelection, window, cx);
                    assert!(view.page_measurements(cx).is_empty());
                    assert_eq!(
                        view.item.read(cx).doc().expect("doc").history.undo_depth(),
                        4
                    );
                    view.undo(&Undo, window, cx);
                    let restored = view.page_measurements(cx).pop().expect("undo deletion");
                    assert_eq!(restored.measurement(), moved.measurement());
                })
            })
            .expect("full measurement lifecycle");
    }

    #[gpui::test]
    async fn measurement_drafts_block_boundaries_and_escape_consumes_a_late_release(
        cx: &mut TestAppContext,
    ) {
        let (item, view, scratch, _) = measurement_view_fixture(cx).await;
        let before = item.read_with(cx, |item, _| {
            (
                item.doc()
                    .expect("doc")
                    .scene
                    .get(item.doc().expect("doc").active_page().expect("page"))
                    .expect("page")
                    .meta
                    .clone(),
                item.doc().expect("doc").history.undo_depth(),
                item.is_dirty(),
            )
        });
        let save = scratch
            .update(cx, |_, window, cx| {
                view.update(cx, |view, cx| {
                    view.activate_tool(ToolKind::Measure, cx);
                    measurement_pointer_down(view, [400., 300.], window, cx);
                    view.handle_window_mouse_move(
                        &MouseMoveEvent {
                            position: point(px(550.), px(380.)),
                            pressed_button: Some(MouseButton::Left),
                            modifiers: gpui::Modifiers::none(),
                        },
                        cx,
                    );
                    let draft = view.measurement_controller.clone();
                    view.activate_tool(ToolKind::Inspect, cx);
                    view.set_editor_mode(EditorMode::Motion, cx);
                    view.set_editor_workspace(EditorWorkspace::Code, cx);
                    view.select_page(99, cx);
                    assert_eq!(view.measurement_controller, draft);
                    assert_eq!(view.tools.kind(), ToolKind::Measure);
                    assert_eq!(view.editor_mode(cx), EditorMode::Design);
                    assert_eq!(view.editor_workspace(cx), EditorWorkspace::Canvas);
                    let save = view.save_document(cx);
                    view.cancel(&Cancel, window, cx);
                    assert!(!view.measurement_controller.has_pending_authoring());
                    measurement_pointer_up(view, [450., 330.], window, cx);
                    assert!(view.page_measurements(cx).is_empty());
                    view.activate_tool(ToolKind::Inspect, cx);
                    assert!(view.is_inspecting());
                    save
                })
            })
            .expect("draft boundaries");
        assert!(save.await.is_err());
        item.read_with(cx, |item, _| {
            let doc = item.doc().expect("doc");
            assert_eq!(
                doc.scene
                    .get(doc.active_page().expect("page"))
                    .expect("page")
                    .meta,
                before.0
            );
            assert_eq!(doc.history.undo_depth(), before.1);
            assert_eq!(item.is_dirty(), before.2);
        });
    }

    #[gpui::test]
    async fn measurement_changed_projection_rejects_release_and_enter_keeps_preview(
        cx: &mut TestAppContext,
    ) {
        let (item, view, scratch, _) = measurement_view_fixture(cx).await;
        scratch
            .update(cx, |_, window, cx| {
                view.update(cx, |view, cx| {
                    view.activate_tool(ToolKind::Measure, cx);
                    measurement_pointer_down(view, [400., 300.], window, cx);
                    view.handle_window_mouse_move(
                        &MouseMoveEvent {
                            position: point(px(540.), px(380.)),
                            pressed_button: Some(MouseButton::Left),
                            modifiers: gpui::Modifiers::none(),
                        },
                        cx,
                    );
                    let accepted = view
                        .measurement_controller
                        .draft()
                        .expect("draft")
                        .preview();
                    view.viewport = Some(Viewport {
                        center: [0., 0.],
                        zoom: 2.,
                    });
                    measurement_pointer_up(view, [440., 330.], window, cx);
                    assert_eq!(
                        view.measurement_controller
                            .draft()
                            .expect("kept draft")
                            .preview(),
                        accepted
                    );
                    assert!(!view.measurement_controller.is_dragging());
                    view.viewport = Some(Viewport::default());
                    view.confirm(&Confirm, window, cx);
                    assert!(view.page_measurements(cx).is_empty());
                    assert!(view.measurement_controller.has_pending_authoring());
                    view.cancel(&Cancel, window, cx);
                })
            })
            .expect("rejected release stays rejected");
        assert_eq!(
            item.read_with(cx, |item, _| item.doc().expect("doc").history.undo_depth()),
            0
        );
    }

    #[gpui::test]
    async fn measurement_focus_loss_freezes_preview_and_ignores_hover_and_late_release(
        cx: &mut TestAppContext,
    ) {
        let (item, view, scratch, _) = measurement_view_fixture(cx).await;
        scratch
            .update(cx, |_, window, cx| {
                view.update(cx, |view, cx| {
                    for boundary in ["item", "workspace", "missed-release"] {
                        view.activate_tool(ToolKind::Measure, cx);
                        measurement_pointer_down(view, [400., 300.], window, cx);
                        view.handle_window_mouse_move(
                            &MouseMoveEvent {
                                position: point(px(540.), px(380.)),
                                pressed_button: Some(MouseButton::Left),
                                modifiers: gpui::Modifiers::none(),
                            },
                            cx,
                        );
                        let before = view
                            .measurement_controller
                            .draft()
                            .expect("draft")
                            .preview();
                        match boundary {
                            "item" => Item::deactivated(view, window, cx),
                            "workspace" => Item::workspace_deactivated(view, window, cx),
                            _ => {}
                        }
                        view.handle_window_mouse_move(
                            &MouseMoveEvent {
                                position: point(px(590.), px(420.)),
                                pressed_button: None,
                                modifiers: gpui::Modifiers::none(),
                            },
                            cx,
                        );
                        assert!(!view.primary_pressed, "{boundary}");
                        assert!(!view.canvas_pointer_down, "{boundary}");
                        assert!(!view.measurement_controller.is_dragging(), "{boundary}");
                        measurement_pointer_up(view, [490., 370.], window, cx);
                        view.confirm(&Confirm, window, cx);
                        assert_eq!(
                            view.measurement_controller
                                .draft()
                                .expect("kept draft")
                                .preview(),
                            before,
                            "{boundary}"
                        );
                        assert!(view.page_measurements(cx).is_empty(), "{boundary}");
                        view.cancel(&Cancel, window, cx);
                    }
                })
            })
            .expect("focus and lost-release recovery");
        assert_eq!(
            item.read_with(cx, |item, _| item.doc().expect("doc").history.undo_depth()),
            0
        );
    }

    #[gpui::test]
    async fn measurement_inspect_selection_copies_but_never_mutates_and_exit_restores_art(
        cx: &mut TestAppContext,
    ) {
        let (item, view, scratch, node) = measurement_view_fixture(cx).await;
        scratch
            .update(cx, |_, window, cx| {
                view.update(cx, |view, cx| {
                    view.activate_tool(ToolKind::Measure, cx);
                    measurement_pointer_down(view, [400., 300.], window, cx);
                    measurement_pointer_up(view, [440., 330.], window, cx);
                    let expected = view.selected_measurement(cx).expect("measurement");
                    let before = view
                        .item
                        .read(cx)
                        .doc()
                        .expect("doc")
                        .to_json_string()
                        .expect("snapshot");
                    view.activate_tool(ToolKind::Inspect, cx);
                    measurement_pointer_down(view, [420., 315.], window, cx);
                    measurement_pointer_up(view, [470., 365.], window, cx);
                    assert_eq!(view.selected_measurement(cx).as_ref(), Some(&expected));
                    view.copy_measurement(&expected, cx);
                    assert_eq!(
                        cx.read_from_clipboard()
                            .and_then(|clipboard| clipboard.text()),
                        Some("50 px".into())
                    );
                    view.delete_measurement(&expected, cx);
                    view.delete_selected_nodes(cx);
                    view.undo(&Undo, window, cx);
                    view.redo(&Redo, window, cx);
                    assert_eq!(
                        view.item
                            .read(cx)
                            .doc()
                            .expect("doc")
                            .to_json_string()
                            .expect("snapshot"),
                        before
                    );
                    view.activate_tool(ToolKind::Select, cx);
                    assert!(view.selected_measurement(cx).is_none());
                    assert!(view.is_editable(cx));
                    view.item.update(cx, |item, cx| {
                        item.with_document(cx, |document| {
                            document.doc.selection.select_only(node);
                            ((), DocChange::Selection)
                        });
                    });
                    view.nudge(LogicalKey::ArrowRight, window, cx);
                    assert_eq!(
                        view.page_measurements(cx)
                            .first()
                            .expect("measurement unchanged")
                            .measurement(),
                        expected.measurement()
                    );
                })
            })
            .expect("readonly inspection and exit");
        assert!(item.read_with(cx, |item, _| item.is_dirty()));
    }

    #[gpui::test]
    async fn measurement_cancel_rearms_autosave_for_preexisting_committed_edits(
        cx: &mut TestAppContext,
    ) {
        let (_directory, root, item, view) = autosave_fixture(cx).await;
        item.update(cx, |item, cx| item.save(SaveKind::Explicit, cx))
            .await
            .expect("write baseline");
        let page = item.read_with(cx, |item, _| {
            item.doc().expect("doc").active_page().expect("page")
        });
        item.update(cx, |item, cx| {
            item.apply(
                Operation::SetName {
                    id: page,
                    old: "Page 1".into(),
                    new: "Committed before measuring".into(),
                },
                cx,
            )
            .expect("committed edit")
        });
        cx.run_until_parked();
        let scratch = cx.add_window(|_, _| gpui::Empty);
        scratch
            .update(cx, |_, window, cx| {
                view.update(cx, |view, cx| {
                    view.container_bounds = Some(Bounds::new(
                        point(px(100.), px(50.)),
                        size(px(800.), px(600.)),
                    ));
                    view.viewport = Some(Viewport::default());
                    view.activate_tool(ToolKind::Measure, cx);
                    measurement_pointer_down(view, [400., 300.], window, cx);
                    view.handle_window_mouse_move(
                        &MouseMoveEvent {
                            position: point(px(540.), px(380.)),
                            pressed_button: Some(MouseButton::Left),
                            modifiers: gpui::Modifiers::none(),
                        },
                        cx,
                    );
                })
            })
            .expect("start uncommitted measurement");
        cx.executor().advance_clock(AUTOSAVE_DEBOUNCE * 2);
        cx.run_until_parked();
        view.read_with(cx, |view, _| {
            assert!(
                view.autosave_task.is_none(),
                "debounce expired while measurement blocked save"
            )
        });
        let (during, _) = fanta_format::read_project_tree(&root).expect("baseline still on disk");
        assert_eq!(during.scene.get(page).expect("page").name, "Page 1");
        scratch
            .update(cx, |_, window, cx| {
                view.update(cx, |view, cx| {
                    view.cancel(&Cancel, window, cx);
                    assert!(
                        view.autosave_task.is_some(),
                        "Escape rearms prior committed work"
                    );
                })
            })
            .expect("cancel draft");
        cx.executor().advance_clock(AUTOSAVE_DEBOUNCE * 2);
        cx.run_until_parked();
        let (saved, _) = fanta_format::read_project_tree(&root).expect("previous edit autosaved");
        assert_eq!(
            saved.scene.get(page).expect("page").name,
            "Committed before measuring"
        );
        assert!(
            read_measurements(&saved, page)
                .expect("read saved marks")
                .is_empty()
        );
        assert!(!item.read_with(cx, |item, _| item.is_dirty()));
    }

    #[gpui::test]
    async fn measurement_view_local_close_blocker_keeps_owner_and_allows_sibling_close(
        cx: &mut TestAppContext,
    ) {
        init_visual_test(cx);
        let project = Project::test(FakeFs::new(cx.executor()), [], cx).await;
        let mut document = doc_with_one_page();
        document.history = Default::default();
        let item = crate::document::ready_item_for_test(
            &project,
            "/tmp/MeasurementClose.fig".into(),
            document,
            cx,
        );
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
            view.activate_tool(ToolKind::Measure, cx);
            measurement_pointer_down(view, [400., 300.], window, cx);
            view.handle_window_mouse_move(
                &MouseMoveEvent {
                    position: point(px(540.), px(380.)),
                    pressed_button: Some(MouseButton::Left),
                    modifiers: gpui::Modifiers::none(),
                },
                cx,
            );
            view.freeze_measurement_drag(cx);
        });
        let draft = owner.read_with(cx, |view, cx| {
            assert!(view.close_blocker(cx).is_some());
            view.measurement_controller
                .draft()
                .cloned()
                .expect("recoverable preview")
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
                assert_eq!(view.measurement_controller.draft(), Some(&draft));
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
            assert_eq!(view.measurement_controller.draft(), Some(&draft));
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
        owner.update(cx, |view, cx| assert!(view.cancel_measurement_drag(cx)));
        pane.update_in(cx, |pane, window, cx| {
            pane.close_item_by_id(owner.entity_id(), workspace::SaveIntent::Close, window, cx)
        })
        .await
        .expect("close after explicit draft cancellation");
        pane.read_with(cx, |pane, _| assert_eq!(pane.items_len(), 0));
    }
}
