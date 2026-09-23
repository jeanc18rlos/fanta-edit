use super::*;
use fanta_gpui::design::DesignInspector;
use fanta_gpui::molecules::ZoomControlsAction;
use fanta_gpui::properties_inspector::{
    PropertiesInspector, PropertiesInspectorAction, PropertiesInspectorChildren,
    PropertiesInspectorTab,
};
use fanta_gpui::properties_tabs::*;

pub(super) struct PropertiesAdapter {
    pub layout: Entity<PropertiesInspector>,
    pub(super) design: Entity<DesignInspector>,
    code: Entity<CodeInspector>,
    comments: Entity<CommentsInspector>,
    pub(super) draw: Entity<DrawInspector>,
    motion: Entity<MotionInspector>,
    code_snapshot: Option<(u64, Vec<NodeId>, Option<usize>)>,
    comments_snapshot: Option<(u64, bool, Option<String>)>,
    zoom: u16,
    _subscriptions: Vec<Subscription>,
}

fn inspector_tab(mode: EditorMode) -> PropertiesInspectorTab {
    match mode {
        EditorMode::Design => PropertiesInspectorTab::Design,
        EditorMode::Motion => PropertiesInspectorTab::Motion,
        EditorMode::Draw => PropertiesInspectorTab::Draw,
        EditorMode::Code => PropertiesInspectorTab::Code,
        EditorMode::Prototype => PropertiesInspectorTab::Prototype,
        EditorMode::Comments => PropertiesInspectorTab::Comments,
    }
}

impl FigView {
    pub(super) fn refresh_properties_inspector(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.gpui_properties.is_none() {
            let Some(controller) = self
                .gpui_design
                .as_ref()
                .map(|adapter| adapter.panel.clone())
            else {
                return;
            };
            let design =
                cx.new(|cx| DesignInspector::new("editor-design-inspector", controller, cx));
            let code = cx.new(|cx| {
                CodeInspector::new(
                    "editor-code-inspector",
                    CodeInspectorViewData {
                        languages: vec![InspectorChoice::new("json", "JSON")],
                        language: "json".into(),
                        ..Default::default()
                    },
                    cx,
                )
            });
            let comments = cx.new(|cx| {
                CommentsInspector::new("editor-comments-inspector", Default::default(), cx)
            });
            let draw = cx.new(|cx| {
                let mut inspector = DrawInspector::new(
                    "editor-draw-inspector",
                    DrawInspectorViewData {
                        read_only: true,
                        ..Default::default()
                    },
                    cx,
                );
                inspector.set_capabilities(
                    fanta_gpui::toolbar::DrawBrushCapabilities::VECTOR_PENCIL,
                    cx,
                );
                inspector
            });
            let motion = cx.new(|cx| {
                let mut inspector =
                    MotionInspector::new("editor-motion-inspector", Default::default(), cx);
                inspector.set_auto_keyframe_available(false, cx);
                inspector
            });
            let children = PropertiesInspectorChildren {
                design: design.clone().into(),
                motion: motion.clone().into(),
                draw: draw.clone().into(),
                code: code.clone().into(),
                prototype: self.prototype_sidebar.clone().into(),
                comments: comments.clone().into(),
            };
            let zoom = self.current_zoom_percent(cx);
            let layout = cx.new(|cx| {
                PropertiesInspector::new("editor-properties-inspector", children, zoom, cx)
            });
            let subscriptions = vec![
                cx.subscribe_in(&layout, window, Self::handle_properties_action),
                cx.subscribe(&code, |this, _, action: &CodeInspectorAction, cx| {
                    let Some(adapter) = &this.gpui_properties else {
                        return;
                    };
                    let mut data = adapter.code.read(cx).view_data().clone();
                    match action {
                        CodeInspectorAction::CopyRequested { code } => {
                            cx.write_to_clipboard(ClipboardItem::new_string(code.to_string()))
                        }
                        CodeInspectorAction::CopyPropertyRequested { property_id } => {
                            if let Some(property) = data
                                .properties
                                .iter()
                                .find(|property| property.id == *property_id)
                            {
                                cx.write_to_clipboard(ClipboardItem::new_string(
                                    property.label.to_string(),
                                ));
                            }
                        }
                        CodeInspectorAction::WrapLinesChangeRequested { enabled } => {
                            data.wrap_lines = *enabled
                        }
                        CodeInspectorAction::LanguageChangeRequested { language }
                            if data.languages.iter().any(|choice| choice.id == *language) =>
                        {
                            data.language = language.clone()
                        }
                        CodeInspectorAction::LanguageChangeRequested { .. } => return,
                    }
                    adapter
                        .code
                        .update(cx, |code, cx| code.set_view_data(data, cx));
                }),
                cx.subscribe_in(&comments, window, Self::handle_inspector_comment_action),
                cx.subscribe_in(&draw, window, Self::handle_inspector_draw_action),
                cx.subscribe_in(&motion, window, Self::handle_inspector_motion_action),
            ];
            self.gpui_properties = Some(PropertiesAdapter {
                layout,
                design,
                code,
                comments,
                draw,
                motion,
                code_snapshot: None,
                comments_snapshot: None,
                zoom,
                _subscriptions: subscriptions,
            });
        }
        let mode = inspector_tab(self.editor_mode(cx));
        let zoom = self.current_zoom_percent(cx);
        let editable = self.is_editable(cx);
        let collapsed = !self.inspector_sidebar_visible;
        let Some(adapter) = self.gpui_properties.as_mut() else {
            return;
        };
        let previous_tab = adapter.layout.read(cx).active_tab();
        if (previous_tab != mode || (collapsed && !adapter.layout.read(cx).is_collapsed()))
            && previous_tab == PropertiesInspectorTab::Design
        {
            adapter
                .design
                .update(cx, |design, cx| design.deactivate(window, cx));
        }
        if previous_tab != mode {
            adapter
                .layout
                .update(cx, |layout, cx| layout.set_active_tab(mode, cx));
        }
        if adapter.layout.read(cx).is_collapsed() != collapsed {
            adapter
                .layout
                .update(cx, |layout, cx| layout.set_collapsed(collapsed, cx));
        }
        if adapter.zoom != zoom {
            adapter.zoom = zoom;
            adapter
                .layout
                .update(cx, |layout, cx| layout.set_zoom(zoom, cx));
        }
        if collapsed {
            return;
        }
        match mode {
            PropertiesInspectorTab::Draw => {
                let mut draw = adapter.draw.read(cx).view_data().clone();
                draw.tool_name = self.tools.kind().label().into();
                draw.read_only = !editable;
                let capabilities =
                    crate::gpui_adapters::toolbar::draw_capabilities(self.tools.kind());
                adapter
                    .draw
                    .update(cx, |panel, cx| panel.set_capabilities(capabilities, cx));
                if let Some(toolbar) = &self.gpui_toolbar {
                    draw.options = toolbar.draw_options.clone();
                }
                if adapter.draw.read(cx).view_data() != &draw {
                    adapter
                        .draw
                        .update(cx, |panel, cx| panel.set_view_data(draw, cx));
                }
            }
            PropertiesInspectorTab::Motion => {
                let motion = motion_view_data(
                    self.item.read(cx),
                    self.active_motion_clip,
                    self.timeline_shell.read(cx),
                );
                if adapter.motion.read(cx).view_data() != &motion {
                    adapter
                        .motion
                        .update(cx, |panel, cx| panel.set_view_data(motion, cx));
                }
            }
            PropertiesInspectorTab::Code => {
                let item = self.item.read(cx);
                let Some(document) = item.document() else {
                    adapter.code_snapshot = None;
                    return;
                };
                let generation = document.render_generation();
                let selection = document.doc.selection.as_slice();
                if adapter.code_snapshot.as_ref().is_some_and(
                    |(cached_generation, cached_selection, cached_page)| {
                        *cached_generation == generation
                            && cached_selection.as_slice() == selection
                            && *cached_page == self.selected_page_index
                    },
                ) {
                    return;
                }
                adapter.code_snapshot =
                    Some((generation, selection.to_vec(), self.selected_page_index));
                let mut code = adapter.code.read(cx).view_data().clone();
                let nodes = document
                    .doc
                    .selection
                    .iter()
                    .filter_map(|id| document.doc.scene.get(*id))
                    .collect::<Vec<_>>();
                code.selection_name = match nodes.as_slice() {
                    [node] => node.name.clone().into(),
                    [] => "Select a layer to inspect its source".into(),
                    nodes => format!("{} selected layers", nodes.len()).into(),
                };
                code.code = if nodes.is_empty() {
                    SharedString::default()
                } else {
                    match serde_json::to_string_pretty(&nodes) {
                        Ok(source) => source.into(),
                        Err(error) => {
                            log::error!("encoding inspector source failed: {error:#}");
                            format!("Unable to encode selection: {error}").into()
                        }
                    }
                };
                adapter
                    .code
                    .update(cx, |panel, cx| panel.set_view_data(code, cx));
            }
            PropertiesInspectorTab::Comments => {
                let item = self.item.read(cx);
                let Some(document) = item.document() else {
                    adapter.comments_snapshot = None;
                    return;
                };
                let generation = document.render_generation();
                let editable = item.is_editable();
                let selected_thread = self.comment_state.open_thread.clone();
                if adapter.comments_snapshot.as_ref().is_some_and(
                    |(cached_generation, cached_editable, cached_thread)| {
                        *cached_generation == generation
                            && *cached_editable == editable
                            && *cached_thread == selected_thread
                    },
                ) {
                    return;
                }
                adapter.comments_snapshot = Some((generation, editable, selected_thread.clone()));
                let mut comments = adapter.comments.read(cx).view_data().clone();
                comments.can_comment = editable;
                comments.selected_thread = selected_thread.map(Into::into);
                comments.threads = document
                    .pages
                    .iter()
                    .filter_map(|page| page.root.map(|root| (root, page)))
                    .flat_map(|(root, page)| {
                        crate::comments::read_comments(&document.doc, root)
                            .into_iter()
                            .map(|thread| {
                                let mut messages = vec![InspectorComment {
                                    id: thread.id.clone().into(),
                                    author: thread.author.into(),
                                    time_label: crate::comments_ui::relative_time(thread.created)
                                        .into(),
                                    body: thread.text.into(),
                                }];
                                messages.extend(thread.replies.into_iter().enumerate().map(
                                    |(index, reply)| InspectorComment {
                                        id: format!("{}-{index}", thread.id).into(),
                                        author: reply.author.into(),
                                        time_label:
                                            crate::comments_ui::relative_time(reply.created).into(),
                                        body: reply.body.into(),
                                    },
                                ));
                                InspectorCommentThread {
                                    id: thread.id.into(),
                                    location: page.name.clone(),
                                    resolved: thread.resolved,
                                    unread: false,
                                    comments: messages,
                                }
                            })
                            .collect::<Vec<_>>()
                    })
                    .collect();
                adapter
                    .comments
                    .update(cx, |panel, cx| panel.set_view_data(comments, cx));
            }
            PropertiesInspectorTab::Design | PropertiesInspectorTab::Prototype => {}
        }
    }

    fn handle_properties_action(
        &mut self,
        _: &Entity<PropertiesInspector>,
        action: &PropertiesInspectorAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match action {
            PropertiesInspectorAction::TabChanged { tab } => {
                if let Some(adapter) = &self.gpui_properties {
                    adapter
                        .design
                        .update(cx, |design, cx| design.deactivate(window, cx));
                }
                let mode = match tab {
                    PropertiesInspectorTab::Design => EditorMode::Design,
                    PropertiesInspectorTab::Motion => EditorMode::Motion,
                    PropertiesInspectorTab::Draw => EditorMode::Draw,
                    PropertiesInspectorTab::Code => EditorMode::Code,
                    PropertiesInspectorTab::Prototype => EditorMode::Prototype,
                    PropertiesInspectorTab::Comments => EditorMode::Comments,
                };
                self.set_editor_mode(mode, cx);
            }
            PropertiesInspectorAction::CollapsedChanged { collapsed } => {
                if let Some(adapter) = &self.gpui_properties {
                    adapter
                        .design
                        .update(cx, |design, cx| design.deactivate(window, cx));
                }
                self.finish_panel_edits(cx);
                self.inspector_sidebar_visible = !collapsed;
                self.persist_sidebar_layout(cx);
                cx.notify();
            }
            PropertiesInspectorAction::Zoom(ZoomControlsAction::ZoomChangeRequested {
                percent,
            }) => self.zoom_to_percent(*percent, cx),
            PropertiesInspectorAction::Zoom(ZoomControlsAction::FitToViewRequested) => {
                self.fit_to_view(&FitToView, window, cx)
            }
            PropertiesInspectorAction::Zoom(ZoomControlsAction::FitToSelectionRequested) => {
                self.zoom_to_selection(cx)
            }
        }
    }

    fn handle_inspector_comment_action(
        &mut self,
        _: &Entity<CommentsInspector>,
        action: &CommentsInspectorAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use crate::comments::{
            add_comment_full_op, read_comments, reply_comment_full_op, toggle_resolved_op,
        };
        if let CommentsInspectorAction::FilterChangeRequested { filter } = action {
            if let Some(adapter) = &self.gpui_properties {
                let mut data = adapter.comments.read(cx).view_data().clone();
                data.filter = *filter;
                adapter
                    .comments
                    .update(cx, |panel, cx| panel.set_view_data(data, cx));
            }
            return;
        }
        let found = match action {
            CommentsInspectorAction::ThreadSelectRequested { id }
            | CommentsInspectorAction::ResolveChangeRequested { id, .. }
            | CommentsInspectorAction::ReplyRequested { thread_id: id, .. } => {
                self.item.read(cx).document().and_then(|document| {
                    document.pages.iter().enumerate().find_map(|(index, page)| {
                        let root = page.root?;
                        read_comments(&document.doc, root)
                            .into_iter()
                            .find(|thread| thread.id == id.as_ref())
                            .map(|thread| (index, root, thread))
                    })
                })
            }
            _ => None,
        };
        match action {
            CommentsInspectorAction::ThreadSelectRequested { id } => {
                if let Some((index, _, _)) = found {
                    self.select_page(index, cx);
                    self.toggle_comment_thread(id.to_string(), window, cx);
                }
            }
            CommentsInspectorAction::ThreadClearRequested => {
                self.comment_state.open_thread = None;
            }
            _ if !self.is_editable(cx) => return,
            CommentsInspectorAction::CommentAddRequested { body } => {
                let world = self
                    .viewport
                    .map(|viewport| viewport.center)
                    .unwrap_or_default();
                let created = self.item.read(cx).document().and_then(|document| {
                    let root = document.doc.active_page()?;
                    add_comment_full_op(
                        &document.doc,
                        root,
                        world,
                        body,
                        Vec::new(),
                        Vec::new(),
                        None,
                    )
                });
                if let Some((id, operation)) = created {
                    self.apply_motion_operation(Some(operation), "adding inspector comment", cx);
                    self.comment_state.open_thread = Some(id);
                }
            }
            CommentsInspectorAction::ResolveChangeRequested { id, resolved } => {
                if let Some((_, root, thread)) = found
                    && thread.resolved != *resolved
                {
                    let operation = self
                        .item
                        .read(cx)
                        .document()
                        .and_then(|document| toggle_resolved_op(&document.doc, root, id));
                    self.apply_motion_operation(operation, "resolving inspector comment", cx);
                }
            }
            CommentsInspectorAction::ReplyRequested { thread_id, body } => {
                if let Some((_, root, _)) = found {
                    let operation = self.item.read(cx).document().and_then(|document| {
                        reply_comment_full_op(
                            &document.doc,
                            root,
                            thread_id,
                            body,
                            Vec::new(),
                            Vec::new(),
                            None,
                        )
                    });
                    self.apply_motion_operation(operation, "replying to inspector comment", cx);
                }
            }
            CommentsInspectorAction::FilterChangeRequested { .. } => {}
        }
        if let Some(adapter) = &mut self.gpui_properties {
            adapter.comments_snapshot = None;
        }
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::motion_preset_available;
    use fanta_doc::{CanvasNode, Doc, GroupNode, NodeData, NodeFlags, Operation};

    #[test]
    fn motion_presets_require_one_editable_selected_layer() {
        let mut doc = Doc::new();
        let first = CanvasNode::new(NodeData::Group(GroupNode::default()));
        let first_id = first.id;
        doc.apply(Operation::create_node(first))
            .expect("create first layer");
        let mut locked = CanvasNode::new(NodeData::Group(GroupNode::default()));
        locked.flags.insert(NodeFlags::LOCKED);
        let locked_id = locked.id;
        doc.apply(Operation::create_node(locked))
            .expect("create locked layer");

        assert!(!motion_preset_available(&doc, true));
        doc.selection.replace_with([first_id]);
        assert!(motion_preset_available(&doc, true));
        assert!(!motion_preset_available(&doc, false));
        doc.selection.replace_with([first_id, locked_id]);
        assert!(!motion_preset_available(&doc, true));
        doc.selection.replace_with([locked_id]);
        assert!(!motion_preset_available(&doc, true));
    }
}

fn motion_view_data(
    item: &FigItem,
    active_clip: Option<AnimationClipId>,
    clock: &TimelineShell,
) -> MotionInspectorViewData {
    let mut data = MotionInspectorViewData {
        playing: clock.is_playing(),
        playback: if clock.ping_pong_playback_enabled() {
            fanta_gpui::timeline::TimelinePlayback::PingPong
        } else if clock.loop_playback_enabled() {
            fanta_gpui::timeline::TimelinePlayback::Loop
        } else {
            fanta_gpui::timeline::TimelinePlayback::Once
        },
        read_only: !item.is_editable(),
        timeline_open_available: false,
        ..Default::default()
    };
    let Some(document) = item.document() else {
        return data;
    };
    if motion_preset_available(&document.doc, item.is_editable()) {
        data.presets = crate::gpui_adapters::toolbar::motion_animation_styles()
            .into_iter()
            .map(|name| InspectorChoice::new(name.clone(), name))
            .collect();
    }
    data.selection_name = document
        .doc
        .selection
        .iter()
        .filter_map(|id| document.doc.scene.get(*id))
        .map(|node| node.name.as_str())
        .collect::<Vec<_>>()
        .join(", ")
        .into();
    if let Some(clip) = active_clip.and_then(|id| document.doc.motion.clip(id)) {
        data.can_preview = true;
        let tracks = clip
            .tracks
            .values()
            .filter(|track| document.doc.selection.contains(track.target.node))
            .collect::<Vec<_>>();
        let keys = tracks
            .iter()
            .flat_map(|track| track.keyframes.values())
            .collect::<Vec<_>>();
        data.can_edit_timing = !keys.is_empty();
        data.delay_ms = keys.iter().map(|key| key.time_ms).min().unwrap_or(0);
        data.duration_ms = keys
            .iter()
            .map(|key| key.time_ms)
            .max()
            .unwrap_or(clip.duration_ms)
            .saturating_sub(data.delay_ms)
            .max(1);
        if let Some(key) = keys.iter().min_by_key(|key| key.time_ms) {
            data.easing = super::timeline_adapter::shared_easing(key.easing, key.interpolation);
        }
        data.animated_properties = tracks
            .iter()
            .map(|track| {
                InspectorChoice::new(
                    track.id.to_string(),
                    super::timeline_adapter::property_name(track.target.property),
                )
            })
            .collect();
    }
    data
}

fn motion_preset_available(doc: &fanta_doc::Doc, item_editable: bool) -> bool {
    item_editable
        && super::single_selection(doc)
            .is_some_and(|id| crate::layer_context_ops::editable(doc, id))
}

impl FigView {
    fn handle_inspector_draw_action(
        &mut self,
        _: &Entity<DrawInspector>,
        action: &DrawInspectorAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.is_editable(cx) {
            return;
        }
        let Some(adapter) = &self.gpui_properties else {
            return;
        };
        let mut draw = adapter.draw.read(cx).view_data().clone();
        match action {
            DrawInspectorAction::OptionsChangeRequested { options } => {
                draw.options = options.clone().normalized();
                if let Some(toolbar) = &mut self.gpui_toolbar {
                    toolbar.draw_options = draw.options.clone();
                    toolbar.panel.update(cx, |panel, cx| {
                        panel.set_draw_options(draw.options.clone(), cx)
                    });
                }
            }
            DrawInspectorAction::ColorChangeRequested { hex } => {
                if fanta_doc::Color::from_hex(&format!("#{}", hex.trim_start_matches('#')))
                    .is_none()
                {
                    show_canvas_notice("Enter a six or eight digit hex color".into(), window, cx);
                    return;
                }
                draw.color_hex = hex.trim_start_matches('#').to_ascii_uppercase().into();
            }
            DrawInspectorAction::BlendModeChangeRequested { id } => {
                if !draw.blend_modes.iter().any(|choice| choice.id == *id) {
                    return;
                }
                draw.blend_mode = id.clone();
            }
            DrawInspectorAction::BrushPresetSaveRequested => {
                return;
            }
        }
        adapter
            .draw
            .update(cx, |panel, cx| panel.set_view_data(draw, cx));
    }

    fn handle_inspector_motion_action(
        &mut self,
        _: &Entity<MotionInspector>,
        action: &MotionInspectorAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use fanta_gpui::timeline::{TimelineAction, TimelinePlayback};
        match action {
            MotionInspectorAction::PlayingChangeRequested { playing } => {
                self.timeline_shell
                    .update(cx, |clock, cx| clock.set_playing(*playing, cx));
            }
            MotionInspectorAction::PlaybackChangeRequested { playback } => {
                if *playback == TimelinePlayback::PingPong {
                    self.timeline_shell
                        .update(cx, |clock, cx| clock.set_ping_pong_playback(cx));
                } else {
                    self.timeline_shell.update(cx, |clock, cx| {
                        clock.set_loop_playback(*playback == TimelinePlayback::Loop, cx)
                    });
                }
            }
            MotionInspectorAction::TimelineOpenRequested => {
                self.set_editor_mode(EditorMode::Motion, cx)
            }
            MotionInspectorAction::AutoKeyframeChangeRequested { .. } => {
                notify_unavailable("Auto keyframe recording", window, cx)
            }
            MotionInspectorAction::PresetApplyRequested { id } => {
                self.motion_sidebar
                    .update(cx, |panel, cx| panel.apply_inspector_preset(id, cx));
                self.sync_motion_timeline(cx);
            }
            MotionInspectorAction::KeyframeAddRequested { property_id } => {
                if let Some(panel) = self
                    .gpui_timeline
                    .as_ref()
                    .map(|adapter| adapter.panel.clone())
                {
                    let time_ms = (self.timeline_shell.read(cx).playhead_us().max(0) / 1000) as u32;
                    self.handle_shared_timeline_action(
                        &panel,
                        &TimelineAction::PropertyKeyframeRequested {
                            track_id: SharedString::default(),
                            property_id: property_id.clone(),
                            time_ms,
                        },
                        window,
                        cx,
                    );
                }
            }
            MotionInspectorAction::DurationChangeRequested { .. }
            | MotionInspectorAction::DelayChangeRequested { .. }
            | MotionInspectorAction::EasingChangeRequested { .. } => {
                let mut operations = vec![];
                if let Some(clip) = self
                    .active_motion_clip
                    .and_then(|id| self.item.read(cx).document()?.doc.motion.clip(id))
                {
                    let Some(document) = self.item.read(cx).document() else {
                        return;
                    };
                    let doc = &document.doc;
                    let selected = clip
                        .tracks
                        .values()
                        .filter(|track| doc.selection.contains(track.target.node))
                        .collect::<Vec<_>>();
                    let start = selected
                        .iter()
                        .flat_map(|track| track.keyframes.values().map(|key| key.time_ms))
                        .min()
                        .unwrap_or(0);
                    let end = selected
                        .iter()
                        .flat_map(|track| track.keyframes.values().map(|key| key.time_ms))
                        .max()
                        .unwrap_or(clip.duration_ms);
                    let mut duration = clip.duration_ms;
                    for track in selected {
                        let mut changed = track.clone();
                        for key in changed.keyframes.values_mut() {
                            match action {
                                MotionInspectorAction::DurationChangeRequested { duration_ms } => {
                                    key.time_ms = start.saturating_add(
                                        (f64::from(key.time_ms.saturating_sub(start))
                                            / f64::from(end.saturating_sub(start).max(1))
                                            * f64::from(*duration_ms))
                                        .round() as u32,
                                    )
                                }
                                MotionInspectorAction::DelayChangeRequested { delay_ms } => {
                                    key.time_ms =
                                        key.time_ms.saturating_sub(start).saturating_add(*delay_ms)
                                }
                                MotionInspectorAction::EasingChangeRequested { easing } => {
                                    (key.easing, key.interpolation) =
                                        super::timeline_adapter::engine_easing(easing)
                                }
                                _ => {}
                            }
                            duration = duration.max(key.time_ms);
                        }
                        if changed != *track {
                            operations.push(Operation::SetAnimationTrack {
                                clip: clip.id,
                                track: track.id,
                                old: Some(Box::new(track.clone())),
                                new: Some(Box::new(changed)),
                            });
                        }
                    }
                    if duration != clip.duration_ms {
                        operations.push(Operation::SetAnimationClipDuration {
                            id: clip.id,
                            old: clip.duration_ms,
                            new: duration,
                        });
                    }
                }
                self.apply_inspector_operations(operations, "Edit animation settings", window, cx);
            }
        }
        cx.notify();
    }
}
