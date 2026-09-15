//! Retained entity and subscription assembly for the Design inspector façade.
//!
//! Keeping construction here makes `DesignPanel` a stable host-facing entry
//! point without mixing lifecycle wiring with snapshot and render policy.

use super::*;

pub(super) struct DesignPanelFactory;

#[allow(deprecated)]
impl DesignPanelFactory {
    pub(super) fn assemble(
        id: impl Into<SharedString>,
        node: DesignPanelNode,
        window: &mut Window,
        cx: &mut Context<DesignPanel>,
    ) -> DesignPanel {
        let id = id.into();
        let property_input = cx.new(|cx| InputState::new(window, cx));
        let subscription = cx.subscribe_in(
            &property_input,
            window,
            |this, _, event: &InputEvent, window, cx| match event {
                InputEvent::Change => {
                    if this.edit.variable_font_axis.is_some() {
                        this.validate_variable_font_axis_draft(cx);
                    } else {
                        this.validate_property_draft(cx);
                    }
                }
                InputEvent::PressEnter { .. } => {
                    this.edit.suppress_next_control_activation = true;
                    window.prevent_default();
                    if this.edit.variable_font_axis.is_some() {
                        this.finish_variable_font_axis_edit(true, window, cx);
                    } else {
                        this.finish_property_edit(true, window, cx);
                    }
                }
                InputEvent::Blur => {
                    if this.edit.variable_font_axis.is_some() {
                        this.finish_variable_font_axis_edit(true, window, cx);
                    } else {
                        this.finish_property_edit_after_input_blur(true, window, cx);
                    }
                }
                InputEvent::Focus => {}
            },
        );
        let property_variable_search = cx.new(|cx| InputState::new(window, cx));
        let property_variable_search_subscription =
            cx.subscribe(&property_variable_search, |_, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            });
        let component_property_variable_search = cx.new(|cx| InputState::new(window, cx));
        let component_property_variable_search_subscription = cx.subscribe(
            &component_property_variable_search,
            |_, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            },
        );
        let component_swap_search = cx.new(|cx| InputState::new(window, cx));
        let component_swap_search_subscription =
            cx.subscribe(&component_swap_search, |_, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            });
        let component_authoring_name_input = cx.new(|cx| InputState::new(window, cx));
        let component_authoring_name_input_subscription = cx.subscribe_in(
            &component_authoring_name_input,
            window,
            |this, _, event: &InputEvent, window, cx| match event {
                InputEvent::Change => this.preview_component_authoring_name(cx),
                InputEvent::PressEnter { .. } => {
                    window.prevent_default();
                    if this.edit.component_authoring.has_name_editor() {
                        this.finish_component_authoring_name_edit(true, window, cx);
                    } else if this.component_authoring.create_draft.is_some() {
                        this.submit_component_property_create(window, cx);
                    }
                }
                InputEvent::Blur => {
                    this.finish_component_authoring_name_edit(true, window, cx);
                }
                InputEvent::Focus => {}
            },
        );
        let component_authoring_default_input = cx.new(|cx| InputState::new(window, cx));
        let component_authoring_default_input_subscription = cx.subscribe(
            &component_authoring_default_input,
            |_, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            },
        );
        let component_authoring_slot_minimum_input = cx.new(|cx| InputState::new(window, cx));
        let component_authoring_slot_minimum_input_subscription = cx.subscribe(
            &component_authoring_slot_minimum_input,
            |_, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            },
        );
        let component_authoring_slot_maximum_input = cx.new(|cx| InputState::new(window, cx));
        let component_authoring_slot_maximum_input_subscription = cx.subscribe(
            &component_authoring_slot_maximum_input,
            |_, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            },
        );
        let font_search = cx.new(|cx| InputState::new(window, cx));
        let font_search_subscription =
            cx.subscribe(&font_search, |_, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            });
        let style_browser_search =
            cx.new(|cx| InputState::new(window, cx).placeholder("Search styles"));
        let style_browser_search_subscription =
            cx.subscribe(&style_browser_search, |_, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            });
        let component_multiline_input = cx.new(|cx| InputState::new(window, cx).auto_grow(3, 8));
        let component_multiline_input_subscription = cx.subscribe(
            &component_multiline_input,
            |this, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.preview_component_multiline(cx);
                }
            },
        );
        let paint_picker = cx.new(|cx| {
            PaintPicker::new(SharedString::from(format!("{id}-paint-picker")), window, cx)
        });
        let paint_picker_subscription = cx.subscribe_in(
            &paint_picker,
            window,
            |this, _, event: &PaintPickerEvent, _, cx| match event {
                PaintPickerEvent::Edit {
                    target,
                    edit,
                    phase,
                } if this
                    .overlays
                    .auxiliary_color_picker()
                    .as_ref()
                    .is_some_and(|active| active.matches_picker_target(target)) =>
                {
                    let leaf_is_editable = this
                        .overlays
                        .auxiliary_color_picker()
                        .as_ref()
                        .is_some_and(|active| this.auxiliary_paint_editable(active, edit));
                    if (matches!(phase, DesignPanelEditPhase::Cancel) || leaf_is_editable)
                        && this.track_paint_edit(target, edit, *phase)
                    {
                        this.emit_auxiliary_color_edit(edit, *phase, cx);
                    }
                }
                PaintPickerEvent::Edit {
                    target,
                    edit,
                    phase,
                } if target.node_id == this.host.inspected_node().id
                    && this.overlays.active_picker()
                        == Some(PaintPickerTarget {
                            collection: target.collection,
                            index: target.index,
                            paint_id: target.paint_id.clone(),
                        }) =>
                {
                    if this.track_paint_edit(target, edit, *phase) {
                        this.emit_paint_edit(
                            PaintPickerTarget {
                                collection: target.collection,
                                index: target.index,
                                paint_id: target.paint_id.clone(),
                            },
                            edit.as_ref().clone(),
                            *phase,
                            cx,
                        );
                    }
                }
                PaintPickerEvent::Edit { .. } => {}
                PaintPickerEvent::BlendModePreview {
                    target,
                    original,
                    candidate,
                    phase,
                } => match phase {
                    DesignMenuPreviewPhase::Begin => {
                        this.begin_paint_blend_mode_preview(target, *original, *candidate, cx)
                    }
                    DesignMenuPreviewPhase::End => {
                        this.end_paint_blend_mode_preview(target, *original, *candidate, cx)
                    }
                },
                PaintPickerEvent::SourceReplaceRequested { target }
                    if target.node_id == this.host.inspected_node().id
                        && this.overlays.active_picker()
                            == Some(PaintPickerTarget {
                                collection: target.collection,
                                index: target.index,
                                paint_id: target.paint_id.clone(),
                            }) =>
                {
                    let panel_target = PaintPickerTarget {
                        collection: target.collection,
                        index: target.index,
                        paint_id: target.paint_id.clone(),
                    };
                    if let Some(index) = this.paint_target_index(&panel_target)
                        && this.can_edit()
                        && this.paint_style_binding(target.collection).is_none()
                        && this
                            .paint_collection(target.collection)
                            .and_then(|paints| paints.get(index))
                            .is_some_and(|paint| {
                                !paint.read_only
                                    && matches!(&paint.payload, DesignPaintPayload::Pattern(_))
                            })
                    {
                        cx.emit_design_panel_action(
                            this,
                            DesignPanelAction::PaintSourceReplaceRequested {
                                node_id: this.host.inspected_node().id.clone(),
                                collection: target.collection,
                                target: this.paint_target(target.collection),
                                paint_id: target.paint_id.clone(),
                                index,
                            },
                        );
                    }
                }
                PaintPickerEvent::SourceReplaceRequested { .. } => {}
                PaintPickerEvent::MediaSourceActionRequested {
                    target,
                    source_id,
                    action,
                } if target.node_id == this.host.inspected_node().id
                    && this.overlays.active_picker()
                        == Some(PaintPickerTarget {
                            collection: target.collection,
                            index: target.index,
                            paint_id: target.paint_id.clone(),
                        }) =>
                {
                    this.emit_media_source_action(
                        PaintPickerTarget {
                            collection: target.collection,
                            index: target.index,
                            paint_id: target.paint_id.clone(),
                        },
                        source_id,
                        *action,
                        cx,
                    );
                }
                PaintPickerEvent::MediaSourceActionRequested { .. } => {}
                PaintPickerEvent::MediaSourceDropRequested {
                    target,
                    expected_source_id,
                    expected_media_kind,
                    file,
                } if target.node_id == this.host.inspected_node().id
                    && this.overlays.active_picker()
                        == Some(PaintPickerTarget {
                            collection: target.collection,
                            index: target.index,
                            paint_id: target.paint_id.clone(),
                        }) =>
                {
                    this.emit_media_source_drop(
                        PaintPickerTarget {
                            collection: target.collection,
                            index: target.index,
                            paint_id: target.paint_id.clone(),
                        },
                        expected_source_id,
                        *expected_media_kind,
                        file.clone(),
                        cx,
                    );
                }
                PaintPickerEvent::MediaSourceDropRequested { .. } => {}
                PaintPickerEvent::MediaCropActionRequested { target, action }
                    if target.node_id == this.host.inspected_node().id
                        && this.overlays.active_picker()
                            == Some(PaintPickerTarget {
                                collection: target.collection,
                                index: target.index,
                                paint_id: target.paint_id.clone(),
                            }) =>
                {
                    let panel_target = PaintPickerTarget {
                        collection: target.collection,
                        index: target.index,
                        paint_id: target.paint_id.clone(),
                    };
                    this.emit_media_crop_action(&panel_target, action.clone(), cx);
                }
                PaintPickerEvent::MediaCropActionRequested { .. } => {}
                PaintPickerEvent::VideoPreviewActionRequested { target, action }
                    if target.node_id == this.host.inspected_node().id
                        && this.overlays.active_picker()
                            == Some(PaintPickerTarget {
                                collection: target.collection,
                                index: target.index,
                                paint_id: target.paint_id.clone(),
                            }) =>
                {
                    let panel_target = PaintPickerTarget {
                        collection: target.collection,
                        index: target.index,
                        paint_id: target.paint_id.clone(),
                    };
                    if let Some(index) = this.paint_target_index(&panel_target) {
                        cx.emit_design_panel_action(
                            this,
                            DesignPanelAction::PaintVideoPreviewActionRequested {
                                node_id: this.host.inspected_node().id.clone(),
                                collection: target.collection,
                                target: this.paint_target(target.collection),
                                paint_id: target.paint_id.clone(),
                                index,
                                action: action.clone(),
                            },
                        );
                    }
                }
                PaintPickerEvent::VideoPreviewActionRequested { .. } => {}
                PaintPickerEvent::ShaderImportRequested { target, shader }
                    if target.node_id == this.host.inspected_node().id
                        && this.overlays.active_picker()
                            == Some(PaintPickerTarget {
                                collection: target.collection,
                                index: target.index,
                                paint_id: target.paint_id.clone(),
                            }) =>
                {
                    let panel_target = PaintPickerTarget {
                        collection: target.collection,
                        index: target.index,
                        paint_id: target.paint_id.clone(),
                    };
                    if let Some(index) = this.paint_target_index(&panel_target)
                        && this.can_edit()
                        && this
                            .resources
                            .shaders
                            .shader(shader)
                            .is_some_and(|definition| !definition.imported)
                    {
                        cx.emit_design_panel_action(
                            this,
                            DesignPanelAction::PaintShaderImportRequested {
                                node_id: this.host.inspected_node().id.clone(),
                                collection: target.collection,
                                target: this.paint_target(target.collection),
                                paint_id: target.paint_id.clone(),
                                index,
                                shader: shader.clone(),
                            },
                        );
                    }
                }
                PaintPickerEvent::ShaderImportRequested { .. } => {}
                PaintPickerEvent::ShaderApplyRequested { target, shader }
                    if target.node_id == this.host.inspected_node().id
                        && this.overlays.active_picker()
                            == Some(PaintPickerTarget {
                                collection: target.collection,
                                index: target.index,
                                paint_id: target.paint_id.clone(),
                            }) =>
                {
                    let panel_target = PaintPickerTarget {
                        collection: target.collection,
                        index: target.index,
                        paint_id: target.paint_id.clone(),
                    };
                    if let Some(index) = this.paint_target_index(&panel_target)
                        && this.can_edit()
                        && this
                            .resources
                            .shaders
                            .shader(shader)
                            .is_some_and(|definition| definition.imported)
                    {
                        cx.emit_design_panel_action(
                            this,
                            DesignPanelAction::PaintShaderApplyRequested {
                                node_id: this.host.inspected_node().id.clone(),
                                collection: target.collection,
                                target: this.paint_target(target.collection),
                                paint_id: target.paint_id.clone(),
                                index,
                                shader: shader.clone(),
                            },
                        );
                    }
                }
                PaintPickerEvent::ShaderApplyRequested { .. } => {}
                PaintPickerEvent::ShaderPropertyBindRequested {
                    target,
                    definition_id,
                } if target.node_id == this.host.inspected_node().id
                    && this.overlays.active_picker()
                        == Some(PaintPickerTarget {
                            collection: target.collection,
                            index: target.index,
                            paint_id: target.paint_id.clone(),
                        }) =>
                {
                    let panel_target = PaintPickerTarget {
                        collection: target.collection,
                        index: target.index,
                        paint_id: target.paint_id.clone(),
                    };
                    if let Some(index) = this.paint_target_index(&panel_target)
                        && this.can_edit()
                    {
                        cx.emit_design_panel_action(
                            this,
                            DesignPanelAction::PaintShaderPropertyBindRequested {
                                node_id: this.host.inspected_node().id.clone(),
                                collection: target.collection,
                                target: this.paint_target(target.collection),
                                paint_id: target.paint_id.clone(),
                                index,
                                definition_id: definition_id.clone(),
                            },
                        );
                    }
                }
                PaintPickerEvent::ShaderPropertyBindRequested { .. } => {}
                PaintPickerEvent::ShaderPropertyEditorRequested {
                    target,
                    definition_id,
                } if target.node_id == this.host.inspected_node().id
                    && this.overlays.active_picker()
                        == Some(PaintPickerTarget {
                            collection: target.collection,
                            index: target.index,
                            paint_id: target.paint_id.clone(),
                        }) =>
                {
                    let panel_target = PaintPickerTarget {
                        collection: target.collection,
                        index: target.index,
                        paint_id: target.paint_id.clone(),
                    };
                    if let Some(index) = this.paint_target_index(&panel_target)
                        && this.can_edit()
                    {
                        cx.emit_design_panel_action(
                            this,
                            DesignPanelAction::PaintShaderPropertyEditorRequested {
                                node_id: this.host.inspected_node().id.clone(),
                                collection: target.collection,
                                target: this.paint_target(target.collection),
                                paint_id: target.paint_id.clone(),
                                index,
                                definition_id: definition_id.clone(),
                            },
                        );
                    }
                }
                PaintPickerEvent::ShaderPropertyEditorRequested { .. } => {}
                PaintPickerEvent::ShaderPropertyDetachRequested {
                    target,
                    definition_id,
                    variable_id,
                } if target.node_id == this.host.inspected_node().id
                    && this.overlays.active_picker()
                        == Some(PaintPickerTarget {
                            collection: target.collection,
                            index: target.index,
                            paint_id: target.paint_id.clone(),
                        }) =>
                {
                    let panel_target = PaintPickerTarget {
                        collection: target.collection,
                        index: target.index,
                        paint_id: target.paint_id.clone(),
                    };
                    if let Some(index) = this.paint_target_index(&panel_target)
                        && this.can_edit()
                    {
                        cx.emit_design_panel_action(
                            this,
                            DesignPanelAction::PaintShaderPropertyDetachRequested {
                                node_id: this.host.inspected_node().id.clone(),
                                collection: target.collection,
                                target: this.paint_target(target.collection),
                                paint_id: target.paint_id.clone(),
                                index,
                                definition_id: definition_id.clone(),
                                variable_id: variable_id.clone(),
                            },
                        );
                    }
                }
                PaintPickerEvent::ShaderPropertyDetachRequested { .. } => {}
                PaintPickerEvent::ColorVariableApplyRequested {
                    target,
                    color_target,
                    variable_id,
                } if target.node_id == this.host.inspected_node().id
                    && this.overlays.active_picker()
                        == Some(PaintPickerTarget {
                            collection: target.collection,
                            index: target.index,
                            paint_id: target.paint_id.clone(),
                        }) =>
                {
                    let panel_target = PaintPickerTarget {
                        collection: target.collection,
                        index: target.index,
                        paint_id: target.paint_id.clone(),
                    };
                    let variable = this
                        .resources
                        .paint_variables
                        .variable(variable_id.as_ref());
                    if let Some(index) = this.paint_target_index(&panel_target)
                        && this.can_edit()
                        && variable.is_some_and(|variable| {
                            variable.disabled_reason.is_none()
                                && matches!(
                                    variable.import_state,
                                    DesignVariableImportState::Local
                                        | DesignVariableImportState::Imported
                                )
                        })
                        && let Some(color_target) =
                            this.resolve_paint_color_target(&panel_target, color_target, true)
                    {
                        cx.emit_design_panel_action(
                            this,
                            DesignPanelAction::PaintColorVariableApplyRequested {
                                node_id: this.host.inspected_node().id.clone(),
                                collection: target.collection,
                                target: this.paint_target(target.collection),
                                paint_id: target.paint_id.clone(),
                                index,
                                color_target,
                                variable_id: variable_id.clone(),
                            },
                        );
                    }
                }
                PaintPickerEvent::ColorVariableApplyRequested { .. } => {}
                PaintPickerEvent::ColorVariableImportRequested {
                    target,
                    color_target,
                    variable_id,
                } if target.node_id == this.host.inspected_node().id
                    && this.overlays.active_picker()
                        == Some(PaintPickerTarget {
                            collection: target.collection,
                            index: target.index,
                            paint_id: target.paint_id.clone(),
                        }) =>
                {
                    let panel_target = PaintPickerTarget {
                        collection: target.collection,
                        index: target.index,
                        paint_id: target.paint_id.clone(),
                    };
                    let variable = this
                        .resources
                        .paint_variables
                        .variable(variable_id.as_ref());
                    if let Some(index) = this.paint_target_index(&panel_target)
                        && this.can_edit()
                        && variable.is_some_and(|variable| {
                            variable.disabled_reason.is_none()
                                && variable.import_state == DesignVariableImportState::Available
                        })
                        && let Some(color_target) =
                            this.resolve_paint_color_target(&panel_target, color_target, true)
                    {
                        cx.emit_design_panel_action(
                            this,
                            DesignPanelAction::PaintColorVariableImportRequested {
                                node_id: this.host.inspected_node().id.clone(),
                                collection: target.collection,
                                target: this.paint_target(target.collection),
                                paint_id: target.paint_id.clone(),
                                index,
                                color_target,
                                variable_id: variable_id.clone(),
                            },
                        );
                    }
                }
                PaintPickerEvent::ColorVariableImportRequested { .. } => {}
                PaintPickerEvent::ColorVariableDetachRequested {
                    target,
                    color_target,
                    variable_id,
                } if target.node_id == this.host.inspected_node().id
                    && this.overlays.active_picker()
                        == Some(PaintPickerTarget {
                            collection: target.collection,
                            index: target.index,
                            paint_id: target.paint_id.clone(),
                        }) =>
                {
                    let panel_target = PaintPickerTarget {
                        collection: target.collection,
                        index: target.index,
                        paint_id: target.paint_id.clone(),
                    };
                    if let Some(index) = this.paint_target_index(&panel_target)
                        && this.can_edit()
                        && let Some(color_target) =
                            this.resolve_paint_color_target(&panel_target, color_target, true)
                        && this
                            .paint_color_binding(&panel_target, &color_target)
                            .is_some_and(|binding| {
                                binding.variable_id.as_ref() == variable_id.as_ref()
                            })
                    {
                        cx.emit_design_panel_action(
                            this,
                            DesignPanelAction::PaintColorVariableDetachRequested {
                                node_id: this.host.inspected_node().id.clone(),
                                collection: target.collection,
                                target: this.paint_target(target.collection),
                                paint_id: target.paint_id.clone(),
                                index,
                                color_target,
                                variable_id: variable_id.clone(),
                            },
                        );
                    }
                }
                PaintPickerEvent::ColorVariableDetachRequested { .. } => {}
                PaintPickerEvent::ColorVariableCreateRequested {
                    target,
                    color_target,
                    color,
                } if target.node_id == this.host.inspected_node().id
                    && this.overlays.active_picker()
                        == Some(PaintPickerTarget {
                            collection: target.collection,
                            index: target.index,
                            paint_id: target.paint_id.clone(),
                        }) =>
                {
                    let panel_target = PaintPickerTarget {
                        collection: target.collection,
                        index: target.index,
                        paint_id: target.paint_id.clone(),
                    };
                    if let Some(index) = this.paint_target_index(&panel_target)
                        && this.can_edit()
                        && let Some(color_target) =
                            this.resolve_paint_color_target(&panel_target, color_target, true)
                    {
                        cx.emit_design_panel_action(
                            this,
                            DesignPanelAction::PaintColorVariableCreateRequested {
                                node_id: this.host.inspected_node().id.clone(),
                                collection: target.collection,
                                target: this.paint_target(target.collection),
                                paint_id: target.paint_id.clone(),
                                index,
                                color_target,
                                color: *color,
                            },
                        );
                    }
                }
                PaintPickerEvent::ColorVariableCreateRequested { .. } => {}
                PaintPickerEvent::PaintStyleCreateRequested {
                    target,
                    color_target,
                } if target.node_id == this.host.inspected_node().id
                    && this.overlays.active_picker()
                        == Some(PaintPickerTarget {
                            collection: target.collection,
                            index: target.index,
                            paint_id: target.paint_id.clone(),
                        }) =>
                {
                    let panel_target = PaintPickerTarget {
                        collection: target.collection,
                        index: target.index,
                        paint_id: target.paint_id.clone(),
                    };
                    if this.paint_target_index(&panel_target).is_some()
                        && this.can_edit()
                        && this
                            .resolve_paint_color_target(&panel_target, color_target, true)
                            .is_some()
                    {
                        this.emit_paint_style_create(target.collection, cx);
                    }
                }
                PaintPickerEvent::PaintStyleCreateRequested { .. } => {}
                PaintPickerEvent::ColorStyleSampleRequested {
                    target,
                    color_target,
                    sample,
                } if target.node_id == this.host.inspected_node().id
                    && this.overlays.active_picker()
                        == Some(PaintPickerTarget {
                            collection: target.collection,
                            index: target.index,
                            paint_id: target.paint_id.clone(),
                        }) =>
                {
                    let panel_target = PaintPickerTarget {
                        collection: target.collection,
                        index: target.index,
                        paint_id: target.paint_id.clone(),
                    };
                    if let Some(index) = this.paint_target_index(&panel_target)
                        && this.can_edit()
                        && this
                            .resources
                            .color_style_samples
                            .sample(sample)
                            .is_some_and(|sample| sample.disabled_reason.is_none())
                        && let Some(color_target) =
                            this.resolve_paint_color_target(&panel_target, color_target, true)
                    {
                        cx.emit_design_panel_action(
                            this,
                            DesignPanelAction::PaintColorStyleSampleRequested {
                                node_id: this.host.inspected_node().id.clone(),
                                collection: target.collection,
                                target: this.paint_target(target.collection),
                                paint_id: target.paint_id.clone(),
                                index,
                                color_target,
                                sample: sample.clone(),
                            },
                        );
                    }
                }
                PaintPickerEvent::ColorStyleSampleRequested { .. } => {}
                PaintPickerEvent::EyedropperRequested {
                    target,
                    color_target,
                } if target.node_id == this.host.inspected_node().id
                    && this.overlays.active_picker()
                        == Some(PaintPickerTarget {
                            collection: target.collection,
                            index: target.index,
                            paint_id: target.paint_id.clone(),
                        }) =>
                {
                    let panel_target = PaintPickerTarget {
                        collection: target.collection,
                        index: target.index,
                        paint_id: target.paint_id.clone(),
                    };
                    if let Some(index) = this.paint_target_index(&panel_target)
                        && this.can_edit()
                        && let Some(color_target) =
                            this.resolve_paint_color_target(&panel_target, color_target, false)
                    {
                        cx.emit_design_panel_action(
                            this,
                            DesignPanelAction::PaintEyedropperRequested {
                                node_id: this.host.inspected_node().id.clone(),
                                collection: target.collection,
                                target: this.paint_target(target.collection),
                                paint_id: target.paint_id.clone(),
                                index,
                                color_target,
                            },
                        );
                    }
                }
                PaintPickerEvent::EyedropperRequested { .. } => {}
            },
        );
        let typography_style_picker = cx.new(|cx| {
            TypographyStylePicker::new(
                SharedString::from(format!("{id}-typography-style-picker")),
                window,
                cx,
            )
        });
        let typography_style_picker_subscription = cx.subscribe(
            &typography_style_picker,
            |this, _, event: &TypographyStylePickerEvent, cx| match event {
                TypographyStylePickerEvent::ApplyRequested { style }
                    if this.property_is_editable(DesignPanelProperty::TypographyStyle)
                        && this.resources.typography_styles.style(style).is_some() =>
                {
                    cx.emit_design_panel_action(
                        this,
                        DesignPanelAction::TypographyStyleApplyRequested {
                            node_id: this.host.inspected_node().id.clone(),
                            target: this.typography_target(DesignPanelProperty::TypographyStyle),
                            style: style.clone(),
                        },
                    );
                }
                TypographyStylePickerEvent::DetachRequested { style }
                    if this.property_is_editable(DesignPanelProperty::TypographyStyle)
                        && this
                            .host
                            .inspected_node()
                            .typography
                            .as_ref()
                            .and_then(|typography| typography.style_binding.as_ref())
                            .is_some_and(|binding| {
                                binding.can_detach && binding.selection == *style
                            }) =>
                {
                    cx.emit_design_panel_action(
                        this,
                        DesignPanelAction::TypographyStyleDetachRequested {
                            node_id: this.host.inspected_node().id.clone(),
                            target: this.typography_target(DesignPanelProperty::TypographyStyle),
                            style: style.clone(),
                        },
                    );
                }
                TypographyStylePickerEvent::ApplyRequested { .. }
                | TypographyStylePickerEvent::DetachRequested { .. } => {}
            },
        );
        let draw_opacity_slider = cx.new(|_| {
            SliderState::new()
                .min(0.)
                .max(100.)
                .step(1.)
                .default_value(node.opacity)
        });
        let draw_opacity_slider_subscription =
            cx.subscribe(&draw_opacity_slider, |this, _, event: &SliderEvent, cx| {
                let SliderEvent::Change(SliderValue::Single(value)) = event else {
                    return;
                };
                this.preview_draw_appearance_slider(DesignPanelProperty::Opacity, *value, cx);
            });
        let draw_corner_radius_slider = cx.new(|_| {
            SliderState::new()
                .min(0.)
                .max(100.)
                .step(1.)
                .default_value(node.corner_radii[0].clamp(0., 100.))
        });
        let draw_corner_radius_slider_subscription = cx.subscribe(
            &draw_corner_radius_slider,
            |this, _, event: &SliderEvent, cx| {
                let SliderEvent::Change(SliderValue::Single(value)) = event else {
                    return;
                };
                this.preview_draw_appearance_slider(DesignPanelProperty::CornerRadius, *value, cx);
            },
        );
        let inspection_context = DesignPanelInspectionContext::single(
            node.clone(),
            DesignPanelParentLayout::Freeform,
            DesignPanelPermissions::editor(),
        );
        let features = DesignPanelFeatureState::new(&node);
        DesignPanel {
            id,
            focus_handle: cx.focus_handle(),
            host: DesignPanelHostState::new(node, inspection_context),
            resources: DesignPanelResourceCatalogs::default(),
            preferences: DesignInspectorPreferences::default(),
            features,
            paint_picker,
            typography_style_picker,
            overlays: DesignOverlayCoordinator::with_focus_handles(cx),
            sections: DesignSectionController::default(),
            component_authoring: component_props::ComponentAuthoringState::new(
                component_authoring_name_input,
                component_authoring_default_input,
                component_authoring_slot_minimum_input,
                component_authoring_slot_maximum_input,
            ),
            edit: DesignPropertyEditController::default(),
            retained: DesignPanelRetainedChildren::new(
                DesignPanelInputStates {
                    property: property_input,
                    property_variable_search,
                    component_property_variable_search,
                    component_swap_search,
                    font_search,
                    style_browser_search,
                    component_multiline: component_multiline_input,
                },
                DesignDrawSliderStates::new(
                    draw_opacity_slider,
                    draw_opacity_slider_subscription,
                    draw_corner_radius_slider,
                    draw_corner_radius_slider_subscription,
                ),
            ),
            shell: DesignPanelShellState::default(),
            _subscriptions: vec![
                subscription,
                property_variable_search_subscription,
                component_property_variable_search_subscription,
                component_swap_search_subscription,
                component_authoring_name_input_subscription,
                component_authoring_default_input_subscription,
                component_authoring_slot_minimum_input_subscription,
                component_authoring_slot_maximum_input_subscription,
                font_search_subscription,
                style_browser_search_subscription,
                component_multiline_input_subscription,
                paint_picker_subscription,
                typography_style_picker_subscription,
            ],
        }
    }

    pub(super) fn assemble_with_context(
        id: impl Into<SharedString>,
        inspection_context: DesignPanelInspectionContext,
        window: &mut Window,
        cx: &mut Context<DesignPanel>,
    ) -> DesignPanel {
        let node = inspection_context
            .selection()
            .items()
            .first()
            .cloned()
            .unwrap_or_else(|| {
                DesignPanelNode::new("design-panel-page", "Page", DesignPanelNodeKind::Frame)
            });
        let mut panel = Self::assemble(id, node, window, cx);
        panel.host.inspection_context = inspection_context;
        panel
    }
}
