//! Gradient-kind and stop-list operations for the retained paint picker.

use super::super::{
    DesignColor, DesignGradientStop, DesignPaint, DesignPaintEdit, DesignPaintKind,
    DesignPaintProperty, DesignPaintValue,
};
use super::{normalize_paint, solid_color::mix_color};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum GradientTransformOperation {
    RotateClockwise90,
    FlipHorizontal,
}

pub(super) const GRADIENT_KINDS: [DesignPaintKind; 4] = [
    DesignPaintKind::LinearGradient,
    DesignPaintKind::RadialGradient,
    DesignPaintKind::AngularGradient,
    DesignPaintKind::DiamondGradient,
];

pub(super) fn clamp_stop_index(paint: &DesignPaint, selected_stop: usize) -> usize {
    selected_stop.min(paint.gradient_stops.len().saturating_sub(1))
}

pub(super) fn paint_with_added_stop(
    paint: &DesignPaint,
    selected_stop: usize,
) -> (DesignPaint, usize) {
    let candidate = normalize_paint(paint.clone());
    if !candidate.kind.is_gradient() {
        return (candidate, 0);
    }

    let index = clamp_stop_index(&candidate, selected_stop);
    let next_index = (index + 1).min(candidate.gradient_stops.len() - 1);
    let left = &candidate.gradient_stops[index];
    let right = &candidate.gradient_stops[next_index];
    let position = if index == next_index {
        (left.position + 0.1).min(1.)
    } else {
        (left.position + right.position) * 0.5
    };
    paint_with_added_stop_at(&candidate, position)
}

pub(super) fn paint_with_added_stop_at(paint: &DesignPaint, position: f32) -> (DesignPaint, usize) {
    let mut candidate = normalize_paint(paint.clone());
    if !candidate.kind.is_gradient() {
        return (candidate, 0);
    }
    let position = position.clamp(0., 1.);
    let right_index = candidate
        .gradient_stops
        .iter()
        .position(|stop| stop.position >= position)
        .unwrap_or(candidate.gradient_stops.len() - 1);
    let left_index = right_index.saturating_sub(1);
    let left = &candidate.gradient_stops[left_index];
    let right = &candidate.gradient_stops[right_index];
    let span = (right.position - left.position).max(f32::EPSILON);
    let amount = ((position - left.position) / span).clamp(0., 1.);
    let stop = DesignGradientStop::new(position, mix_color(left.color, right.color, amount));
    let _ = candidate.apply_edit(&DesignPaintEdit {
        property: DesignPaintProperty::GradientStopAdd,
        value: DesignPaintValue::GradientStop(stop.clone()),
    });
    let selected = candidate
        .gradient_stops
        .iter()
        .rposition(|candidate| candidate.position == stop.position && candidate.color == stop.color)
        .unwrap_or(left_index);
    (candidate, selected)
}

pub(super) fn paint_with_removed_stop(
    paint: &DesignPaint,
    selected_stop: usize,
) -> Option<(DesignPaint, usize)> {
    let mut candidate = normalize_paint(paint.clone());
    if !candidate.kind.is_gradient() || candidate.gradient_stops.len() <= 2 {
        return None;
    }
    let index = clamp_stop_index(&candidate, selected_stop);
    let stop = candidate.gradient_stops[index].clone();
    let _ = candidate.apply_edit(&DesignPaintEdit {
        property: DesignPaintProperty::GradientStopRemove {
            stop_id: stop.id,
            index,
        },
        value: DesignPaintValue::None,
    });
    Some((
        candidate,
        index.min(paint.gradient_stops.len().saturating_sub(2)),
    ))
}

pub(super) fn gradient_segments(paint: &DesignPaint) -> Vec<(f32, f32, DesignColor, DesignColor)> {
    let mut segments = Vec::new();
    if let Some(first) = paint.gradient_stops.first() {
        let position = first.position.clamp(0., 1.);
        if position > 0. {
            segments.push((0., position, first.color, first.color));
        }
    }
    segments.extend(paint.gradient_stops.windows(2).map(|pair| {
        let left = pair[0].position.clamp(0., 1.);
        let right = pair[1].position.clamp(left, 1.);
        (left, right, pair[0].color, pair[1].color)
    }));
    if let Some(last) = paint.gradient_stops.last() {
        let position = last.position.clamp(0., 1.);
        if position < 1. {
            segments.push((position, 1., last.color, last.color));
        }
    }
    segments
}

#[cfg(test)]
pub(super) fn paint_with_color(
    paint: &DesignPaint,
    selected_stop: usize,
    color: DesignColor,
) -> DesignPaint {
    let mut candidate = paint.clone();
    let _ = candidate.apply_edit(&color_edit(paint, selected_stop, color));
    candidate
}

// Gradient interaction and rendering live beside the pure stop operations.
use super::*;

impl PaintPicker {
    pub(super) fn select_gradient_stop(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.cancel_text_input_edit_sessions(cx);
        let Some(paint) = self.paint.as_ref() else {
            return;
        };
        if !paint.kind.is_gradient() || index >= paint.gradient_stops.len() {
            return;
        }
        self.selected_stop = index;
        self.selected_stop_id = paint.gradient_stops[index].id.clone();
        self.remember_current_hue();
        self.hex_invalid = false;
        self.sync_inputs(window, cx, false, false, false);
        cx.notify();
    }

    pub(super) fn add_gradient_stop(&mut self, cx: &mut Context<Self>) {
        if self.editing_disabled() {
            return;
        }
        self.cancel_text_input_edit_sessions(cx);
        let Some(paint) = self.paint.as_ref() else {
            return;
        };
        let (candidate, selected_stop) = paint_with_added_stop(paint, self.selected_stop);
        self.selected_stop = selected_stop;
        self.selected_stop_id = candidate.gradient_stops[selected_stop].id.clone();
        let stop = candidate.gradient_stops[selected_stop].clone();
        let _ = self.emit_edit(
            DesignPaintEdit {
                property: DesignPaintProperty::GradientStopAdd,
                value: DesignPaintValue::GradientStop(stop),
            },
            DesignPanelEditPhase::Commit,
            cx,
        );
    }

    pub(super) fn remove_gradient_stop(&mut self, cx: &mut Context<Self>) {
        if self.editing_disabled() {
            return;
        }
        self.cancel_text_input_edit_sessions(cx);
        let Some(paint) = self.paint.as_ref() else {
            return;
        };
        let removed_index = clamp_stop_index(paint, self.selected_stop);
        let stop = paint.gradient_stops[removed_index].clone();
        let Some((candidate, selected_stop)) = paint_with_removed_stop(paint, removed_index) else {
            return;
        };
        self.selected_stop = selected_stop;
        self.selected_stop_id = candidate
            .gradient_stops
            .get(selected_stop)
            .map_or_else(|| "".into(), |stop| stop.id.clone());
        let _ = self.emit_edit(
            DesignPaintEdit {
                property: DesignPaintProperty::GradientStopRemove {
                    stop_id: stop.id,
                    index: removed_index,
                },
                value: DesignPaintValue::None,
            },
            DesignPanelEditPhase::Commit,
            cx,
        );
    }

    pub(super) fn handle_stop_position_input(
        &mut self,
        event: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if matches!(event, InputEvent::Focus) {
            self.dismissal_event_guard = false;
        }
        if matches!(event, InputEvent::Change) && self.ignore_next_stop_position_change {
            self.ignore_next_stop_position_change = false;
            return;
        }
        if self.dismissal_event_guard || self.suppress_input_events || self.editing_disabled() {
            return;
        }
        match event {
            InputEvent::Change => {
                if !self.stop_position_input.focus_handle(cx).is_focused(window) {
                    return;
                }
                if self.stop_position_edit_session.is_none()
                    && let Some(edit) = self.gradient_stop_position_edit(
                        self.selected_gradient_stop()
                            .map_or(0., |stop| stop.position),
                    )
                {
                    self.stop_position_edit_session = self.begin_text_input_edit(edit, cx);
                }
                let value = self.stop_position_input.read(cx).value();
                if let Some(position) = parse_stop_position(value.as_ref()) {
                    self.stop_position_invalid = false;
                    if let Some(edit) = self.gradient_stop_position_edit(position) {
                        let _ = self.emit_edit(edit, DesignPanelEditPhase::Preview, cx);
                    }
                } else {
                    self.stop_position_invalid = true;
                    cx.notify();
                }
            }
            InputEvent::PressEnter { .. } => {
                window.prevent_default();
                let session = self.stop_position_edit_session.take();
                let candidate = if self.stop_position_invalid {
                    self.sync_inputs(window, cx, true, true, true);
                    self.ignore_next_stop_position_change = true;
                    self.stop_position_invalid = false;
                    cx.notify();
                    None
                } else {
                    let value = self.stop_position_input.read(cx).value();
                    parse_stop_position(value.as_ref())
                        .and_then(|position| self.gradient_stop_position_edit(position))
                };
                if let Some(session) = session {
                    self.finish_text_input_edit(session, candidate, cx);
                }
            }
            InputEvent::Blur => {
                let session = self.stop_position_edit_session.take();
                let candidate = if self.stop_position_invalid {
                    self.sync_inputs(window, cx, true, true, true);
                    self.ignore_next_stop_position_change = true;
                    self.stop_position_invalid = false;
                    cx.notify();
                    None
                } else {
                    let value = self.stop_position_input.read(cx).value();
                    parse_stop_position(value.as_ref())
                        .and_then(|position| self.gradient_stop_position_edit(position))
                };
                if let Some(session) = session {
                    self.finish_text_input_edit(session, candidate, cx);
                }
            }
            InputEvent::Focus => {
                if self.stop_position_edit_session.is_none()
                    && let Some(edit) = self.gradient_stop_position_edit(
                        self.selected_gradient_stop()
                            .map_or(0., |stop| stop.position),
                    )
                {
                    self.stop_position_edit_session = self.begin_text_input_edit(edit, cx);
                }
            }
        }
    }

    pub(super) fn selected_gradient_stop(&self) -> Option<&DesignGradientStop> {
        self.paint
            .as_ref()?
            .gradient_stops
            .get(clamp_stop_index(self.paint.as_ref()?, self.selected_stop))
    }

    pub(super) fn gradient_stop_position_edit(&self, position: f32) -> Option<DesignPaintEdit> {
        let stop = self.selected_gradient_stop()?;
        Some(DesignPaintEdit {
            property: DesignPaintProperty::GradientStopPosition {
                stop_id: stop.id.clone(),
                index: self.selected_stop,
            },
            value: DesignPaintValue::Number(position.clamp(0., 1.)),
        })
    }

    pub(super) fn add_gradient_stop_at(&mut self, position: f32, cx: &mut Context<Self>) {
        if self.editing_disabled() {
            return;
        }
        let Some(paint) = self.paint.as_ref() else {
            return;
        };
        let (candidate, selected_stop) = paint_with_added_stop_at(paint, position);
        self.selected_stop = selected_stop;
        self.selected_stop_id = candidate.gradient_stops[selected_stop].id.clone();
        let stop = candidate.gradient_stops[selected_stop].clone();
        let _ = self.emit_edit(
            DesignPaintEdit {
                property: DesignPaintProperty::GradientStopAdd,
                value: DesignPaintValue::GradientStop(stop),
            },
            DesignPanelEditPhase::Commit,
            cx,
        );
    }

    pub(super) fn begin_gradient_stop_drag(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.editing_disabled() {
            return;
        }
        self.select_gradient_stop(index, window, cx);
        self.dragging_stop = Some(index);
        if let Some(edit) = self.gradient_stop_position_edit(
            self.selected_gradient_stop()
                .map_or(0., |stop| stop.position),
        ) {
            self.begin_continuous_edit(edit, cx);
        }
    }

    pub(super) fn update_gradient_stop_from_pointer(
        &mut self,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.dragging_stop else {
            return;
        };
        self.selected_stop = index;
        let position = fraction_x(self.gradient_bounds, position);
        if let Some(edit) = self.gradient_stop_position_edit(position) {
            self.preview_continuous_edit(edit, cx);
        }
    }

    pub(super) fn finish_gradient_stop_drag(&mut self, cx: &mut Context<Self>) {
        self.dragging_stop = None;
        self.commit_continuous_edit(cx);
    }

    pub(super) fn transform_gradient(
        &self,
        operation: GradientTransformOperation,
        cx: &mut Context<Self>,
    ) {
        let Some(DesignPaintPayload::Gradient(gradient)) =
            self.paint.as_ref().map(|paint| &paint.payload)
        else {
            return;
        };
        let transform = match operation {
            GradientTransformOperation::RotateClockwise90 => gradient.transform.rotated(90.),
            GradientTransformOperation::FlipHorizontal => gradient.transform.flipped_horizontal(),
        };
        let _ = self.emit_edit(
            DesignPaintEdit {
                property: DesignPaintProperty::GradientTransform,
                value: DesignPaintValue::Transform(transform),
            },
            DesignPanelEditPhase::Commit,
            cx,
        );
    }

    pub(super) fn current_gradient_kind_index(&self) -> usize {
        self.paint
            .as_ref()
            .and_then(|paint| GRADIENT_KINDS.iter().position(|kind| *kind == paint.kind))
            .unwrap_or(0)
    }

    pub(super) fn open_gradient_kind_menu_from_keyboard(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.editing_disabled()
            || !self
                .paint
                .as_ref()
                .is_some_and(|paint| paint.kind.is_gradient())
        {
            return;
        }
        self.gradient_kind_menu_index = self.current_gradient_kind_index();
        self.open_nested_overlay(PaintPickerOverlay::GradientKind, window, cx);
        let focus_handle = self.gradient_kind_menu_focus_handle.clone();
        window.defer(cx, move |window, cx| {
            focus_handle.focus(window, cx);
        });
        cx.notify();
    }

    pub(super) fn close_gradient_kind_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close_nested_overlay(PaintPickerOverlay::GradientKind, window, cx);
    }

    pub(super) fn choose_gradient_kind(
        &mut self,
        kind: DesignPaintKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if GRADIENT_KINDS.contains(&kind) {
            self.select_paint_kind(kind, cx);
        }
        self.close_gradient_kind_menu(window, cx);
    }

    pub(super) fn handle_gradient_kind_menu_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let count = GRADIENT_KINDS.len();
        match event.keystroke.key.as_str() {
            "up" => {
                self.gradient_kind_menu_index = (self.gradient_kind_menu_index + count - 1) % count;
                cx.notify();
            }
            "down" => {
                self.gradient_kind_menu_index = (self.gradient_kind_menu_index + 1) % count;
                cx.notify();
            }
            "home" => {
                self.gradient_kind_menu_index = 0;
                cx.notify();
            }
            "end" => {
                self.gradient_kind_menu_index = count - 1;
                cx.notify();
            }
            "enter" | "space" => self.commit_gradient_kind_menu(window, cx),
            "escape" => {
                if !self.dismiss_nested_overlay(
                    PaintPickerOverlay::GradientKind,
                    InspectorOverlayDismissCause::Escape,
                    window,
                    cx,
                ) {
                    return;
                }
            }
            _ => return,
        }
        window.prevent_default();
        cx.stop_propagation();
    }

    /// Commits the highlighted gradient kind; bound to the shared
    /// `ActivateControl` command on the roving menu surface.
    pub(super) fn commit_gradient_kind_menu(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let count = GRADIENT_KINDS.len();
        self.choose_gradient_kind(
            GRADIENT_KINDS[self.gradient_kind_menu_index.min(count - 1)],
            window,
            cx,
        );
    }

    pub(super) fn render_gradient_preview(
        &self,
        paint: &DesignPaint,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut preview = div()
            .id(SharedString::from(format!("{}-gradient-preview", self.id)))
            .relative()
            .tab_index(0)
            .w_full()
            .h(px(36.))
            .overflow_hidden()
            .rounded(px(6.))
            .border_1()
            .border_color(cx.theme().border)
            .bg(pattern_slash(
                cx.theme().muted_foreground.opacity(0.18),
                0.35,
                0.35,
            ))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, _, cx| {
                    if !this.editing_disabled() && this.dragging_stop.is_none() {
                        this.add_gradient_stop_at(
                            fraction_x(this.gradient_bounds, event.position),
                            cx,
                        );
                    }
                }),
            )
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                if event.dragging() {
                    this.update_gradient_stop_from_pointer(event.position, cx);
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, cx| {
                    this.finish_gradient_stop_drag(cx);
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, cx| {
                    this.finish_gradient_stop_drag(cx);
                }),
            )
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if matches!(event.keystroke.key.as_str(), "delete" | "backspace") {
                    this.remove_gradient_stop(cx);
                    window.prevent_default();
                    cx.stop_propagation();
                }
            }));

        for (left, right, left_color, right_color) in gradient_segments(paint) {
            preview = preview.child(
                div()
                    .absolute()
                    .left(relative(left))
                    .w(relative((right - left).max(0.0001)))
                    .h_full()
                    .bg(linear_gradient(
                        90.,
                        linear_color_stop(color_to_hsla(left_color), 0.),
                        linear_color_stop(color_to_hsla(right_color), 1.),
                    )),
            );
        }

        for (index, stop) in paint.gradient_stops.iter().enumerate() {
            let selected = index == self.selected_stop;
            preview = preview.child(
                div()
                    .id(SharedString::from(format!(
                        "{}-gradient-handle-{index}",
                        self.id
                    )))
                    .absolute()
                    .left(relative(stop.position.clamp(0., 1.)))
                    .ml(px(-5.))
                    .bottom(px(2.))
                    .size(px(10.))
                    .rounded(px(2.))
                    .border_2()
                    .border_color(if selected {
                        cx.theme().selection
                    } else {
                        cx.theme().background
                    })
                    .bg(color_to_hsla(stop.color))
                    .cursor_col_resize()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _: &MouseDownEvent, window, cx| {
                            cx.stop_propagation();
                            this.begin_gradient_stop_drag(index, window, cx);
                        }),
                    ),
            );
        }

        preview
            .child(track_bounds(cx.entity(), |this, bounds| {
                this.gradient_bounds = bounds;
            }))
            .into_any_element()
    }

    pub(super) fn render_gradient_kind_selector(
        &self,
        paint: &DesignPaint,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let picker_for_open = cx.entity();
        let picker_for_trigger = picker_for_open.clone();
        let picker_for_content = picker_for_open.clone();
        let picker_id = self.id.clone();
        let highlighted_index = self
            .gradient_kind_menu_index
            .min(GRADIENT_KINDS.len().saturating_sub(1));
        let menu_focus_handle = self.gradient_kind_menu_focus_handle.clone();
        let menu_focus_for_content = menu_focus_handle.clone();
        let disabled = self.editing_disabled();
        let trigger = Button::new(SharedString::from(format!("{}-gradient-kind", self.id)))
            .label(paint.kind.label())
            .dropdown_caret(true)
            .tooltip("Gradient type")
            .xsmall()
            .compact()
            .outline()
            .flex_1()
            .min_w(px(0.))
            .disabled(disabled)
            .on_keyboard_activate(move |window, cx| {
                picker_for_trigger.update(cx, |this, cx| {
                    if this.nested_overlay_is_open(PaintPickerOverlay::GradientKind) {
                        this.close_gradient_kind_menu(window, cx);
                    } else {
                        this.open_gradient_kind_menu_from_keyboard(window, cx);
                    }
                });
            });

        Popover::new(SharedString::from(format!(
            "{}-gradient-kind-menu",
            self.id
        )))
        .anchor(Anchor::BottomLeft)
        .open(self.nested_overlay_is_open(PaintPickerOverlay::GradientKind))
        .track_focus(&menu_focus_handle)
        .overlay_closable(true)
        .on_open_change(move |open, window, cx| {
            picker_for_open.update(cx, |this, cx| {
                if *open
                    && !this.editing_disabled()
                    && this
                        .paint
                        .as_ref()
                        .is_some_and(|paint| paint.kind.is_gradient())
                {
                    this.gradient_kind_menu_index = this.current_gradient_kind_index();
                    this.open_nested_overlay(PaintPickerOverlay::GradientKind, window, cx);
                    cx.notify();
                } else {
                    this.dismiss_nested_overlay(
                        PaintPickerOverlay::GradientKind,
                        InspectorOverlayDismissCause::OutsideClick,
                        window,
                        cx,
                    );
                }
            });
        })
        .trigger(trigger)
        .content(move |_, window, _| {
            v_flex()
                .id(SharedString::from(format!(
                    "{picker_id}-gradient-kind-options"
                )))
                .key_context(CONTROL_KEY_CONTEXT)
                .track_focus(&menu_focus_for_content.clone().tab_index(0).tab_stop(true))
                .on_action({
                    let picker = picker_for_content.clone();
                    move |_: &ActivateControl, window, cx| {
                        picker.update(cx, |this, cx| {
                            this.commit_gradient_kind_menu(window, cx);
                        });
                    }
                })
                .on_key_down({
                    let picker = picker_for_content.clone();
                    move |event: &KeyDownEvent, window, cx| {
                        picker.update(cx, |this, cx| {
                            this.handle_gradient_kind_menu_key(event, window, cx);
                        });
                    }
                })
                .w(popup_width(window, 152.))
                .gap_1()
                .children(GRADIENT_KINDS.into_iter().enumerate().map(|(index, kind)| {
                    let picker = picker_for_content.clone();
                    Button::new(SharedString::from(format!(
                        "{picker_id}-gradient-kind-{}",
                        kind.label().to_lowercase()
                    )))
                    .label(kind.label())
                    .tooltip(kind.label())
                    .xsmall()
                    .compact()
                    .ghost()
                    .w_full()
                    .tab_stop(false)
                    .selected(highlighted_index == index)
                    .on_activate(move |_, window, cx| {
                        picker.update(cx, |this, cx| {
                            this.choose_gradient_kind(kind, window, cx);
                        });
                    })
                }))
        })
        .into_any_element()
    }

    pub(super) fn render_gradient_controls(
        &self,
        paint: &DesignPaint,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if !paint.kind.is_gradient() {
            return None;
        }
        let disabled = self.editing_disabled();

        let mut stops = h_flex().w_full().gap_1().flex_wrap();
        for (index, stop) in paint.gradient_stops.iter().enumerate() {
            let selected = index == self.selected_stop;
            stops = stops.child(
                Button::new(SharedString::from(format!("{}-stop-{index}", self.id)))
                    .xsmall()
                    .compact()
                    .outline()
                    .disabled(disabled)
                    .on_activate(cx.listener(move |this, _, window, cx| {
                        this.select_gradient_stop(index, window, cx);
                    }))
                    .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                        if matches!(event.keystroke.key.as_str(), "delete" | "backspace") {
                            this.remove_gradient_stop(cx);
                            window.prevent_default();
                            cx.stop_propagation();
                        }
                    }))
                    .child(
                        h_flex()
                            .gap_1()
                            .child(
                                div()
                                    .size(px(12.))
                                    .rounded(px(3.))
                                    .border_1()
                                    .border_color(cx.theme().border)
                                    .bg(color_to_hsla(stop.color)),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .child(format!("{}%", (stop.position * 100.).round() as i32)),
                            ),
                    )
                    .when(selected, |button| button.border_color(cx.theme().selection)),
            );
        }

        Some(
            v_flex()
                .w_full()
                .gap_2()
                .child(
                    h_flex()
                        .w_full()
                        .gap_1()
                        .child(self.render_gradient_kind_selector(paint, cx))
                        .child(
                            Button::new(SharedString::from(format!("{}-gradient-flip", self.id)))
                                .icon(IconName::Replace)
                                .tooltip("Flip gradient")
                                .xsmall()
                                .compact()
                                .outline()
                                .disabled(disabled)
                                .on_activate(cx.listener(|this, _, _, cx| {
                                    this.transform_gradient(
                                        GradientTransformOperation::FlipHorizontal,
                                        cx,
                                    );
                                })),
                        )
                        .child(
                            Button::new(SharedString::from(format!("{}-gradient-rotate", self.id)))
                                .icon(IconName::Redo2)
                                .tooltip("Rotate gradient 90°")
                                .xsmall()
                                .compact()
                                .outline()
                                .disabled(disabled)
                                .on_activate(cx.listener(|this, _, _, cx| {
                                    this.transform_gradient(
                                        GradientTransformOperation::RotateClockwise90,
                                        cx,
                                    );
                                })),
                        ),
                )
                .child(self.render_gradient_preview(paint, cx))
                .child(stops)
                .child(
                    h_flex().w_full().gap_2().child(
                        v_flex()
                            .flex_1()
                            .gap_1()
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child("Stop position"),
                            )
                            .child(
                                Input::new(&self.stop_position_input)
                                    .suffix(div().text_xs().child("%"))
                                    .xsmall()
                                    .h(px(26.))
                                    .disabled(disabled)
                                    .when(self.stop_position_invalid, |input| {
                                        input.border_color(cx.theme().red)
                                    }),
                            ),
                    ),
                )
                .child(
                    h_flex()
                        .w_full()
                        .gap_2()
                        .child(
                            Button::new(SharedString::from(format!("{}-add-stop", self.id)))
                                .label("Add stop")
                                .icon(IconName::Plus)
                                .xsmall()
                                .compact()
                                .outline()
                                .disabled(disabled)
                                .on_activate(cx.listener(|this, _, _, cx| {
                                    this.add_gradient_stop(cx);
                                })),
                        )
                        .child(
                            Button::new(SharedString::from(format!("{}-remove-stop", self.id)))
                                .label("Remove")
                                .icon(IconName::Minus)
                                .xsmall()
                                .compact()
                                .outline()
                                .disabled(disabled || paint.gradient_stops.len() <= 2)
                                .on_activate(cx.listener(|this, _, _, cx| {
                                    this.remove_gradient_stop(cx);
                                })),
                        ),
                )
                .into_any_element(),
        )
    }
}
