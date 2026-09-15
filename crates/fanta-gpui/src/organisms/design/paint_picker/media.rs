//! Media input and transform validation kept separate from retained picker UI state.

use std::path::PathBuf;

use super::super::{
    DesignMediaCropAction, DesignMediaCropToolState, DesignMediaDroppedFile,
    DesignMediaPaintCapabilities, DesignPaintTransform,
};

pub(super) const CROP_ROTATION_STEP_DEGREES: f32 = 15.;

pub(super) fn rotated_crop_preview(crop_tool: DesignMediaCropToolState) -> DesignMediaCropAction {
    DesignMediaCropAction::Preview {
        transform: crop_tool.transform.rotated(CROP_ROTATION_STEP_DEGREES),
        zoom: crop_tool.zoom,
        aspect_ratio: crop_tool.aspect_ratio,
    }
}

pub(crate) fn media_drop_from_paths(
    paths: &[PathBuf],
    capabilities: DesignMediaPaintCapabilities,
) -> Option<DesignMediaDroppedFile> {
    capabilities
        .can_upload_source
        .then(|| DesignMediaDroppedFile::from_paths(paths, capabilities.accepted_drop_file_kinds))
        .flatten()
}

pub(super) fn media_transform_is_finite(transform: DesignPaintTransform) -> bool {
    [
        transform.m11,
        transform.m12,
        transform.m21,
        transform.m22,
        transform.tx,
        transform.ty,
    ]
    .into_iter()
    .all(f32::is_finite)
}

// Retained media-editor state and rendering are kept with the pure media
// validation helpers so the parent picker remains an orchestration shell.
use super::*;

impl PaintPicker {
    pub(super) fn current_media_view(&self) -> Option<&DesignMediaPaintView> {
        let target = self.target.as_ref()?;
        self.media_view_data
            .paint(target.collection, &target.paint_id, target.index)
    }

    pub(super) fn current_media_capabilities(&self) -> DesignMediaPaintCapabilities {
        self.current_media_view()
            .map_or_else(DesignMediaPaintCapabilities::default, |view| {
                view.capabilities
            })
    }

    pub(super) fn base_editing_disabled(&self) -> bool {
        self.disabled || self.paint.as_ref().is_some_and(paint_picker_locked)
    }

    pub(super) fn editing_disabled(&self) -> bool {
        self.base_editing_disabled()
            || self.paint.as_ref().is_some_and(|paint| {
                matches!(
                    &paint.payload,
                    DesignPaintPayload::Image(_) | DesignPaintPayload::Video(_)
                ) && !self.current_media_capabilities().can_edit_properties
            })
    }

    pub(super) fn source_replacement_disabled(&self) -> bool {
        if self.base_editing_disabled() {
            return true;
        }
        match self.paint.as_ref().map(|paint| &paint.payload) {
            Some(DesignPaintPayload::Pattern(_)) => false,
            Some(DesignPaintPayload::Image(_) | DesignPaintPayload::Video(_)) => {
                self.media_source_action_disabled(DesignMediaSourceAction::Upload)
            }
            _ => true,
        }
    }

    pub(super) fn media_source_action_disabled(&self, action: DesignMediaSourceAction) -> bool {
        if self.base_editing_disabled() {
            return true;
        }
        let Some(paint_type) = self.paint.as_ref().map(DesignPaint::paint_type) else {
            return true;
        };
        !action.is_applicable_to(paint_type)
            || !self
                .current_media_capabilities()
                .allows_source_action(action)
    }

    pub(super) fn request_source_replace(&self, cx: &mut Context<Self>) {
        if self.source_replacement_disabled() {
            return;
        }
        if !self
            .paint
            .as_ref()
            .is_some_and(|paint| matches!(&paint.payload, DesignPaintPayload::Pattern(_)))
        {
            return;
        }
        let Some(target) = self.target.clone() else {
            return;
        };
        cx.emit(PaintPickerEvent::SourceReplaceRequested { target });
    }

    pub(super) fn request_media_source_action(
        &self,
        action: DesignMediaSourceAction,
        cx: &mut Context<Self>,
    ) {
        if self.media_source_action_disabled(action) {
            return;
        }
        let (Some(target), Some(paint)) = (self.target.clone(), self.paint.as_ref()) else {
            return;
        };
        let source_id = match &paint.payload {
            DesignPaintPayload::Image(image) => image.source.id.clone(),
            DesignPaintPayload::Video(video) => video.source.id.clone(),
            _ => return,
        };
        cx.emit(PaintPickerEvent::MediaSourceActionRequested {
            target,
            source_id,
            action,
        });
    }

    pub(super) fn request_media_source_drop(&self, paths: &[PathBuf], cx: &mut Context<Self>) {
        if self.base_editing_disabled() {
            return;
        }
        let (Some(target), Some(paint)) = (self.target.clone(), self.paint.as_ref()) else {
            return;
        };
        let capabilities = self.current_media_capabilities();
        let Some(file) = media_drop_from_paths(paths, capabilities) else {
            return;
        };
        let (expected_media_kind, expected_source_id) = match &paint.payload {
            DesignPaintPayload::Image(image) => (DesignMediaKind::Image, image.source.id.clone()),
            DesignPaintPayload::Video(video) => (DesignMediaKind::Video, video.source.id.clone()),
            _ => return,
        };
        cx.emit(PaintPickerEvent::MediaSourceDropRequested {
            target,
            expected_source_id,
            expected_media_kind,
            file,
        });
    }

    pub(super) fn request_media_crop_action(
        &self,
        action: DesignMediaCropAction,
        cx: &mut Context<Self>,
    ) {
        if self.editing_disabled()
            || !self.paint.as_ref().is_some_and(|paint| {
                matches!(
                    &paint.payload,
                    DesignPaintPayload::Image(DesignImagePaint {
                        placement: DesignMediaPaintPlacement::Crop { .. },
                        ..
                    }) | DesignPaintPayload::Video(DesignVideoPaint {
                        placement: DesignMediaPaintPlacement::Crop { .. },
                        ..
                    })
                )
            })
        {
            return;
        }
        let valid = match &action {
            DesignMediaCropAction::Preview {
                transform, zoom, ..
            }
            | DesignMediaCropAction::Commit {
                transform, zoom, ..
            } => media_transform_is_finite(*transform) && zoom.is_finite() && *zoom > 0.,
            DesignMediaCropAction::Begin
            | DesignMediaCropAction::Cancel
            | DesignMediaCropAction::ResizeToFit => true,
        };
        if !valid {
            return;
        }
        let Some(target) = self.target.clone() else {
            return;
        };
        cx.emit(PaintPickerEvent::MediaCropActionRequested { target, action });
    }

    pub(super) fn current_video_preview(&self) -> Option<&DesignVideoPreviewState> {
        self.current_media_view()?.video_preview.as_ref()
    }

    pub(super) fn request_video_preview_action(
        &self,
        action: DesignVideoPreviewAction,
        cx: &mut Context<Self>,
    ) {
        if !matches!(
            self.paint.as_ref().map(|paint| &paint.payload),
            Some(DesignPaintPayload::Video(_))
        ) || !matches!(
            self.current_video_preview().map(|preview| &preview.status),
            Some(DesignVideoPreviewStatus::Ready)
        ) {
            return;
        }
        let valid = match action {
            DesignVideoPreviewAction::Play | DesignVideoPreviewAction::Pause => true,
            DesignVideoPreviewAction::Seek { seconds }
            | DesignVideoPreviewAction::Scrub { seconds, .. } => {
                seconds.is_finite() && seconds >= 0.
            }
        };
        if !valid {
            return;
        }
        let Some(target) = self.target.clone() else {
            return;
        };
        cx.emit(PaintPickerEvent::VideoPreviewActionRequested { target, action });
    }

    pub(super) fn request_video_scrub(&self, seconds: f32, cx: &mut Context<Self>) {
        for phase in [
            DesignPanelEditPhase::Begin,
            DesignPanelEditPhase::Preview,
            DesignPanelEditPhase::Commit,
        ] {
            self.request_video_preview_action(
                DesignVideoPreviewAction::Scrub { seconds, phase },
                cx,
            );
        }
    }

    pub(super) fn render_source_state(
        &self,
        paint: &DesignPaint,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let disabled = self.editing_disabled();
        let (title, source_name, icon) = match &paint.payload {
            DesignPaintPayload::Pattern(pattern) => (
                "Pattern fill",
                pattern.source_node_id.clone(),
                IconName::LayoutDashboard,
            ),
            DesignPaintPayload::Image(image) => (
                "Image fill",
                image.source.name.clone(),
                IconName::GalleryVerticalEnd,
            ),
            DesignPaintPayload::Video(video) => {
                ("Video fill", video.source.name.clone(), IconName::File)
            }
            DesignPaintPayload::Shader(shader) => {
                return self.render_shader_state(shader, cx);
            }
            DesignPaintPayload::Unsupported(opaque) => {
                return v_flex()
                    .w_full()
                    .p_3()
                    .gap_2()
                    .rounded(px(7.))
                    .border_1()
                    .border_color(cx.theme().border)
                    .child(
                        div()
                            .text_sm()
                            .font_semibold()
                            .child(opaque.type_name.clone()),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child("This payload is preserved losslessly and is read-only here."),
                    )
                    .into_any_element();
            }
            DesignPaintPayload::Solid(_) | DesignPaintPayload::Gradient(_) => {
                return div().into_any_element();
            }
        };

        let pattern_source = matches!(&paint.payload, DesignPaintPayload::Pattern(_));
        let media_source = matches!(
            &paint.payload,
            DesignPaintPayload::Image(_) | DesignPaintPayload::Video(_)
        );
        let media_capabilities = self.current_media_capabilities();
        let media_drop_enabled =
            media_source && !self.base_editing_disabled() && media_capabilities.can_upload_source;
        let media_drop_background = cx.theme().selection.opacity(0.18);
        let media_drop_border = cx.theme().selection;
        let source_card = h_flex()
            .w_full()
            .p_2()
            .gap_2()
            .rounded(px(7.))
            .border_1()
            .border_color(cx.theme().border)
            .bg(if paint.paint_type() == DesignPaintType::Pattern {
                pattern_slash(cx.theme().muted_foreground.opacity(0.18), 0.5, 0.5)
            } else {
                cx.theme().secondary.into()
            })
            .child(
                Icon::new(icon)
                    .small()
                    .text_color(cx.theme().muted_foreground),
            )
            .child(
                v_flex()
                    .min_w(px(0.))
                    .flex_1()
                    .child(div().text_sm().font_semibold().child(title))
                    .child(
                        div()
                            .truncate()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(source_name),
                    ),
            )
            .when(pattern_source, |source| {
                source.child(
                    Button::new(SharedString::from(format!("{}-replace-source", self.id)))
                        .label("Replace")
                        .xsmall()
                        .compact()
                        .outline()
                        .disabled(self.source_replacement_disabled())
                        .on_activate(cx.listener(|this, _, _, cx| {
                            this.request_source_replace(cx);
                        })),
                )
            })
            .when(media_drop_enabled, |source| {
                source
                    .can_drop(move |candidate, _, _| {
                        candidate
                            .downcast_ref::<ExternalPaths>()
                            .and_then(|paths| {
                                media_drop_from_paths(paths.paths(), media_capabilities)
                            })
                            .is_some()
                    })
                    .drag_over::<ExternalPaths>(move |style, paths, _, _| {
                        if media_drop_from_paths(paths.paths(), media_capabilities).is_some() {
                            style
                                .bg(media_drop_background)
                                .border_color(media_drop_border)
                        } else {
                            style
                        }
                    })
                    .on_drop(cx.listener(|this, paths: &ExternalPaths, _, cx| {
                        this.request_media_source_drop(paths.paths(), cx);
                    }))
            });
        let mut content = v_flex().w_full().gap_2().child(source_card);
        if media_source {
            content = content.child(self.render_media_source_actions(paint, cx));
        }

        match &paint.payload {
            DesignPaintPayload::Pattern(pattern) => {
                let mut modes = h_flex().w_full().gap_1();
                for tile_type in DesignPatternTileType::ALL {
                    modes = modes.child(
                        Button::new(SharedString::from(format!(
                            "{}-pattern-mode-{}",
                            self.id,
                            tile_type.label().to_lowercase().replace(' ', "-")
                        )))
                        .label(tile_type.label())
                        .xsmall()
                        .compact()
                        .ghost()
                        .flex_1()
                        .selected(pattern.tile_type == tile_type)
                        .disabled(disabled)
                        .on_activate(cx.listener(
                            move |this, _, _, cx| {
                                let _ = this.emit_edit(
                                    DesignPaintEdit {
                                        property: DesignPaintProperty::PatternTileType,
                                        value: DesignPaintValue::PatternTileType(tile_type),
                                    },
                                    DesignPanelEditPhase::Commit,
                                    cx,
                                );
                            },
                        )),
                    );
                }
                let mut alignment = h_flex().w_full().gap_1();
                for option in DesignPatternHorizontalAlignment::ALL {
                    alignment = alignment.child(
                        Button::new(SharedString::from(format!(
                            "{}-pattern-align-{}",
                            self.id,
                            option.label().to_lowercase()
                        )))
                        .label(option.label())
                        .xsmall()
                        .compact()
                        .ghost()
                        .flex_1()
                        .selected(pattern.horizontal_alignment == option)
                        .disabled(disabled)
                        .on_activate(cx.listener(
                            move |this, _, _, cx| {
                                let _ = this.emit_edit(
                                    DesignPaintEdit {
                                        property: DesignPaintProperty::PatternHorizontalAlignment,
                                        value: DesignPaintValue::PatternHorizontalAlignment(option),
                                    },
                                    DesignPanelEditPhase::Commit,
                                    cx,
                                );
                            },
                        )),
                    );
                }
                content = content
                    .child(modes)
                    .child(
                        v_flex()
                            .w_full()
                            .gap_1()
                            .child(
                                h_flex().w_full().gap_1().child(
                                    Button::new(SharedString::from(format!(
                                        "{}-pattern-scale",
                                        self.id
                                    )))
                                    .label(format!(
                                        "Scale {}%",
                                        format_decimal(pattern.scaling_factor * 100.)
                                    ))
                                    .xsmall()
                                    .compact()
                                    .outline()
                                    .flex_1()
                                    .disabled(disabled)
                                    .on_activate({
                                        let value = pattern.scaling_factor + 0.1;
                                        cx.listener(move |this, _, _, cx| {
                                            let _ = this.emit_edit(
                                                DesignPaintEdit {
                                                    property:
                                                        DesignPaintProperty::PatternScalingFactor,
                                                    value: DesignPaintValue::Number(value),
                                                },
                                                DesignPanelEditPhase::Commit,
                                                cx,
                                            );
                                        })
                                    }),
                                ),
                            )
                            .child(
                                h_flex()
                                    .w_full()
                                    .gap_1()
                                    .child(
                                        Button::new(SharedString::from(format!(
                                            "{}-pattern-spacing-x",
                                            self.id
                                        )))
                                        .label(format!(
                                            "Space X {}",
                                            format_decimal(pattern.spacing.x)
                                        ))
                                        .xsmall()
                                        .compact()
                                        .outline()
                                        .flex_1()
                                        .disabled(disabled)
                                        .on_activate({
                                            let spacing = DesignPatternSpacing::new(
                                                pattern.spacing.x + 0.01,
                                                pattern.spacing.y,
                                            );
                                            cx.listener(move |this, _, _, cx| {
                                                let _ = this.emit_edit(
                                                    DesignPaintEdit {
                                                        property:
                                                            DesignPaintProperty::PatternSpacing,
                                                        value: DesignPaintValue::PatternSpacing(
                                                            spacing,
                                                        ),
                                                    },
                                                    DesignPanelEditPhase::Commit,
                                                    cx,
                                                );
                                            })
                                        }),
                                    )
                                    .child(
                                        Button::new(SharedString::from(format!(
                                            "{}-pattern-spacing-y",
                                            self.id
                                        )))
                                        .label(format!(
                                            "Space Y {}",
                                            format_decimal(pattern.spacing.y)
                                        ))
                                        .xsmall()
                                        .compact()
                                        .outline()
                                        .flex_1()
                                        .disabled(disabled)
                                        .on_activate({
                                            let spacing = DesignPatternSpacing::new(
                                                pattern.spacing.x,
                                                pattern.spacing.y + 0.01,
                                            );
                                            cx.listener(move |this, _, _, cx| {
                                                let _ = this.emit_edit(
                                                    DesignPaintEdit {
                                                        property:
                                                            DesignPaintProperty::PatternSpacing,
                                                        value: DesignPaintValue::PatternSpacing(
                                                            spacing,
                                                        ),
                                                    },
                                                    DesignPanelEditPhase::Commit,
                                                    cx,
                                                );
                                            })
                                        }),
                                    ),
                            ),
                    )
                    .child(alignment);
            }
            DesignPaintPayload::Image(image) => {
                content = content.child(self.render_media_settings(
                    MediaSettingsView {
                        placement: image.placement,
                        filters: image.filters,
                        crop_tool: self.current_media_view().map_or_else(
                            || DesignMediaCropToolState {
                                transform: image.placement.crop_transform().unwrap_or_default(),
                                ..DesignMediaCropToolState::default()
                            },
                            |view| view.crop_tool,
                        ),
                    },
                    cx,
                ));
            }
            DesignPaintPayload::Video(video) => {
                content = content
                    .child(self.render_media_settings(
                        MediaSettingsView {
                            placement: video.placement,
                            filters: video.filters,
                            crop_tool: self.current_media_view().map_or_else(
                                || DesignMediaCropToolState {
                                    transform: video.placement.crop_transform().unwrap_or_default(),
                                    ..DesignMediaCropToolState::default()
                                },
                                |view| view.crop_tool,
                            ),
                        },
                        cx,
                    ))
                    .child(self.render_video_preview(cx));
            }
            _ => {}
        }

        content.into_any_element()
    }

    pub(super) fn render_media_source_actions(
        &self,
        paint: &DesignPaint,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let paint_type = paint.paint_type();
        let mut actions = v_flex().w_full().gap_1();
        for action in DesignMediaSourceAction::ALL {
            if !action.is_applicable_to(paint_type) {
                continue;
            }
            let mut button = Button::new(SharedString::from(format!(
                "{}-media-source-{}",
                self.id,
                action.slug()
            )))
            .label(action.label())
            .tooltip(action.label())
            .xsmall()
            .compact()
            .w_full()
            .disabled(self.media_source_action_disabled(action))
            .on_activate(cx.listener(move |this, _, _, cx| {
                this.request_media_source_action(action, cx);
            }));
            button = if action == DesignMediaSourceAction::Upload {
                button.primary()
            } else {
                button.outline()
            };
            actions = actions.child(button);
        }
        actions.into_any_element()
    }

    pub(super) fn render_media_settings(
        &self,
        settings: MediaSettingsView,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let disabled = self.editing_disabled();
        let current_mode = settings.placement.mode();
        let rotation = settings.placement.rotation().unwrap_or_default();
        let tile_scale = settings.placement.tile_scaling_factor().unwrap_or(1.);
        let filters = settings.filters;
        let crop_tool = settings.crop_tool;
        let mut modes = h_flex().w_full().gap_1();
        for mode in DesignMediaPaintScaleMode::ALL {
            modes = modes.child(
                Button::new(SharedString::from(format!(
                    "{}-media-mode-{}",
                    self.id,
                    mode.label().to_lowercase()
                )))
                .label(mode.label())
                .xsmall()
                .compact()
                .ghost()
                .flex_1()
                .selected(current_mode == mode)
                .disabled(disabled)
                .on_activate(cx.listener(move |this, _, _, cx| {
                    let _ = this.emit_edit(
                        DesignPaintEdit {
                            property: DesignPaintProperty::MediaScaleMode,
                            value: DesignPaintValue::MediaScaleMode(mode),
                        },
                        DesignPanelEditPhase::Commit,
                        cx,
                    );
                })),
            );
        }
        v_flex()
            .w_full()
            .gap_2()
            .child(modes)
            .child(
                h_flex()
                    .w_full()
                    .gap_2()
                    .child(
                        Button::new(SharedString::from(format!("{}-media-rotation", self.id)))
                            .label(format!("Rotation {}°", rotation.degrees()))
                            .xsmall()
                            .compact()
                            .outline()
                            .disabled(disabled || current_mode == DesignMediaPaintScaleMode::Crop)
                            .on_activate(cx.listener(move |this, _, _, cx| {
                                let _ = this.emit_edit(
                                    DesignPaintEdit {
                                        property: DesignPaintProperty::MediaQuarterTurn,
                                        value: DesignPaintValue::MediaQuarterTurn(
                                            rotation.rotated_clockwise(),
                                        ),
                                    },
                                    DesignPanelEditPhase::Commit,
                                    cx,
                                );
                            })),
                    )
                    .child(
                        Button::new(SharedString::from(format!("{}-media-tile-scale", self.id)))
                            .label(format!("Tile {}%", format_decimal(tile_scale * 100.)))
                            .xsmall()
                            .compact()
                            .outline()
                            .disabled(disabled || current_mode != DesignMediaPaintScaleMode::Tile)
                            .on_activate(cx.listener(move |this, _, _, cx| {
                                let _ = this.emit_edit(
                                    DesignPaintEdit {
                                        property: DesignPaintProperty::MediaTileScalingFactor,
                                        value: DesignPaintValue::Number(tile_scale + 0.1),
                                    },
                                    DesignPanelEditPhase::Commit,
                                    cx,
                                );
                            })),
                    ),
            )
            .when(
                current_mode == DesignMediaPaintScaleMode::Crop,
                |settings| settings.child(self.render_crop_tool(crop_tool, disabled, cx)),
            )
            .child(self.render_image_filter_rows(filters, disabled, cx))
            .into_any_element()
    }

    pub(super) fn render_crop_tool(
        &self,
        crop_tool: DesignMediaCropToolState,
        disabled: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if !crop_tool.active {
            return Button::new(SharedString::from(format!("{}-media-crop", self.id)))
                .label("Crop image")
                .xsmall()
                .compact()
                .outline()
                .w_full()
                .disabled(disabled)
                .on_activate(cx.listener(|this, _, _, cx| {
                    this.request_media_crop_action(DesignMediaCropAction::Begin, cx);
                }))
                .into_any_element();
        }

        let mut nudged_transform = crop_tool.transform;
        nudged_transform.tx += 0.05;
        let next_zoom = (crop_tool.zoom + 0.1).min(64.);
        let next_aspect_ratio = crop_tool.aspect_ratio.next();
        v_flex()
            .w_full()
            .gap_1()
            .child(
                h_flex()
                    .w_full()
                    .gap_1()
                    .child(
                        Button::new(SharedString::from(format!("{}-media-crop-nudge", self.id)))
                            .label("Nudge crop")
                            .xsmall()
                            .compact()
                            .outline()
                            .flex_1()
                            .disabled(disabled)
                            .on_activate(cx.listener(move |this, _, _, cx| {
                                this.request_media_crop_action(
                                    DesignMediaCropAction::Preview {
                                        transform: nudged_transform,
                                        zoom: crop_tool.zoom,
                                        aspect_ratio: crop_tool.aspect_ratio,
                                    },
                                    cx,
                                );
                            })),
                    )
                    .child(
                        Button::new(SharedString::from(format!("{}-media-crop-zoom", self.id)))
                            .label(format!("Zoom {}×", format_decimal(crop_tool.zoom)))
                            .xsmall()
                            .compact()
                            .outline()
                            .flex_1()
                            .disabled(disabled)
                            .on_activate(cx.listener(move |this, _, _, cx| {
                                this.request_media_crop_action(
                                    DesignMediaCropAction::Preview {
                                        transform: crop_tool.transform,
                                        zoom: next_zoom,
                                        aspect_ratio: crop_tool.aspect_ratio,
                                    },
                                    cx,
                                );
                            })),
                    ),
            )
            .child(
                h_flex()
                    .w_full()
                    .gap_1()
                    .child(
                        Button::new(SharedString::from(format!("{}-media-crop-aspect", self.id)))
                            .label(format!("Ratio {}", crop_tool.aspect_ratio.label()))
                            .xsmall()
                            .compact()
                            .outline()
                            .flex_1()
                            .disabled(disabled)
                            .on_activate(cx.listener(move |this, _, _, cx| {
                                this.request_media_crop_action(
                                    DesignMediaCropAction::Preview {
                                        transform: crop_tool.transform,
                                        zoom: crop_tool.zoom,
                                        aspect_ratio: next_aspect_ratio,
                                    },
                                    cx,
                                );
                            })),
                    )
                    .child(
                        Button::new(SharedString::from(format!("{}-media-crop-rotate", self.id)))
                            .label("Rotate 15°")
                            .xsmall()
                            .compact()
                            .outline()
                            .flex_1()
                            .disabled(disabled)
                            .on_activate(cx.listener(move |this, _, _, cx| {
                                this.request_media_crop_action(rotated_crop_preview(crop_tool), cx);
                            })),
                    )
                    .child(
                        Button::new(SharedString::from(format!(
                            "{}-media-crop-resize-to-fit",
                            self.id
                        )))
                        .label("Resize to fit")
                        .xsmall()
                        .compact()
                        .outline()
                        .flex_1()
                        .disabled(disabled)
                        .on_activate(cx.listener(|this, _, _, cx| {
                            this.request_media_crop_action(DesignMediaCropAction::ResizeToFit, cx);
                        })),
                    ),
            )
            .child(
                h_flex()
                    .w_full()
                    .gap_1()
                    .child(
                        Button::new(SharedString::from(format!("{}-media-crop-cancel", self.id)))
                            .label("Cancel")
                            .xsmall()
                            .compact()
                            .ghost()
                            .flex_1()
                            .disabled(disabled)
                            .on_activate(cx.listener(|this, _, _, cx| {
                                this.request_media_crop_action(DesignMediaCropAction::Cancel, cx);
                            })),
                    )
                    .child(
                        Button::new(SharedString::from(format!("{}-media-crop-apply", self.id)))
                            .label("Apply")
                            .xsmall()
                            .compact()
                            .primary()
                            .flex_1()
                            .disabled(disabled)
                            .on_activate(cx.listener(move |this, _, _, cx| {
                                this.request_media_crop_action(
                                    DesignMediaCropAction::Commit {
                                        transform: crop_tool.transform,
                                        zoom: crop_tool.zoom,
                                        aspect_ratio: crop_tool.aspect_ratio,
                                    },
                                    cx,
                                );
                            })),
                    ),
            )
            .into_any_element()
    }

    pub(super) fn render_image_filter_rows(
        &self,
        filters: DesignImageFilters,
        disabled: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut rows = v_flex().w_full().gap_1();
        for filter in DesignImageFilter::ALL {
            let value = filters.value(filter);
            let slug = filter.label().to_ascii_lowercase();
            rows = rows.child(
                h_flex()
                    .w_full()
                    .gap_1()
                    .child(div().w(px(76.)).text_xs().child(filter.label()))
                    .child(
                        Button::new(SharedString::from(format!(
                            "{}-media-filter-{slug}-minus",
                            self.id
                        )))
                        .label("−")
                        .xsmall()
                        .compact()
                        .outline()
                        .disabled(disabled || value <= -1.)
                        .on_activate(cx.listener(
                            move |this, _, _, cx| {
                                let _ = this.emit_edit(
                                    DesignPaintEdit {
                                        property: DesignPaintProperty::MediaFilter(filter),
                                        value: DesignPaintValue::Number(value - 0.1),
                                    },
                                    DesignPanelEditPhase::Commit,
                                    cx,
                                );
                            },
                        )),
                    )
                    .child(
                        Button::new(SharedString::from(format!(
                            "{}-media-filter-{slug}-reset",
                            self.id
                        )))
                        .label(format!("{:+.0}", value * 100.))
                        .tooltip("Reset")
                        .xsmall()
                        .compact()
                        .ghost()
                        .w(px(46.))
                        .disabled(disabled)
                        .on_activate(cx.listener(
                            move |this, _, _, cx| {
                                let _ = this.emit_edit(
                                    DesignPaintEdit {
                                        property: DesignPaintProperty::MediaFilter(filter),
                                        value: DesignPaintValue::Number(0.),
                                    },
                                    DesignPanelEditPhase::Commit,
                                    cx,
                                );
                            },
                        )),
                    )
                    .child(
                        Button::new(SharedString::from(format!(
                            "{}-media-filter-{slug}-plus",
                            self.id
                        )))
                        .label("+")
                        .xsmall()
                        .compact()
                        .outline()
                        .disabled(disabled || value >= 1.)
                        .on_activate(cx.listener(
                            move |this, _, _, cx| {
                                let _ = this.emit_edit(
                                    DesignPaintEdit {
                                        property: DesignPaintProperty::MediaFilter(filter),
                                        value: DesignPaintValue::Number(value + 0.1),
                                    },
                                    DesignPanelEditPhase::Commit,
                                    cx,
                                );
                            },
                        )),
                    ),
            );
        }
        rows.into_any_element()
    }

    pub(super) fn render_video_preview(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(preview) = self.current_video_preview() else {
            return div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child("Preview unavailable")
                .into_any_element();
        };
        match &preview.status {
            DesignVideoPreviewStatus::Idle => div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child("Preview idle")
                .into_any_element(),
            DesignVideoPreviewStatus::Loading => div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child("Loading preview…")
                .into_any_element(),
            DesignVideoPreviewStatus::Error { message } => div()
                .text_xs()
                .text_color(cx.theme().red)
                .child(format!("Preview error: {message}"))
                .into_any_element(),
            DesignVideoPreviewStatus::Ready => {
                let duration = preview.duration_seconds.max(0.);
                let current = preview.current_seconds.clamp(0., duration);
                let seek_back = (current - 1.).max(0.);
                let seek_forward = (current + 1.).min(duration);
                let scrub_forward = (current + duration * 0.1).min(duration);
                v_flex()
                    .w_full()
                    .gap_1()
                    .child(
                        h_flex()
                            .w_full()
                            .justify_between()
                            .child(div().text_xs().font_semibold().child("Video preview"))
                            .child(div().text_xs().child(format!(
                                "{} / {}s",
                                format_decimal(current),
                                format_decimal(duration)
                            ))),
                    )
                    .child(
                        h_flex()
                            .w_full()
                            .gap_1()
                            .child(
                                Button::new(SharedString::from(format!(
                                    "{}-video-preview-play",
                                    self.id
                                )))
                                .label(if preview.playing { "Pause" } else { "Play" })
                                .xsmall()
                                .compact()
                                .primary()
                                .disabled(duration <= 0.)
                                .on_activate({
                                    let playing = preview.playing;
                                    cx.listener(move |this, _, _, cx| {
                                        this.request_video_preview_action(
                                            if playing {
                                                DesignVideoPreviewAction::Pause
                                            } else {
                                                DesignVideoPreviewAction::Play
                                            },
                                            cx,
                                        );
                                    })
                                }),
                            )
                            .child(
                                Button::new(SharedString::from(format!(
                                    "{}-video-preview-back",
                                    self.id
                                )))
                                .label("−1s")
                                .xsmall()
                                .compact()
                                .outline()
                                .disabled(current <= 0.)
                                .on_activate(cx.listener(
                                    move |this, _, _, cx| {
                                        this.request_video_preview_action(
                                            DesignVideoPreviewAction::Seek { seconds: seek_back },
                                            cx,
                                        );
                                    },
                                )),
                            )
                            .child(
                                Button::new(SharedString::from(format!(
                                    "{}-video-preview-forward",
                                    self.id
                                )))
                                .label("+1s")
                                .xsmall()
                                .compact()
                                .outline()
                                .disabled(current >= duration)
                                .on_activate(cx.listener(
                                    move |this, _, _, cx| {
                                        this.request_video_preview_action(
                                            DesignVideoPreviewAction::Seek {
                                                seconds: seek_forward,
                                            },
                                            cx,
                                        );
                                    },
                                )),
                            ),
                    )
                    .child(
                        Button::new(SharedString::from(format!(
                            "{}-video-preview-scrub",
                            self.id
                        )))
                        .label("Scrub +10%")
                        .tooltip("Simulate a begin/preview/commit scrub")
                        .xsmall()
                        .compact()
                        .outline()
                        .disabled(duration <= 0. || current >= duration)
                        .on_activate(cx.listener(
                            move |this, _, _, cx| {
                                this.request_video_scrub(scrub_forward, cx);
                            },
                        )),
                    )
                    .into_any_element()
            }
        }
    }
}
