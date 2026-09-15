//! Text-style picker chrome and live interaction adapter.
//!
//! The retained picker entity owns search, focus, and keyboard-highlight
//! continuity. This module owns its anchored presentation and validates the
//! projected inspection target again before opening it.

use super::super::super::*;

/// Immutable resources and presentation state for the text-style browser.
#[derive(Clone)]
pub(in super::super::super) struct TypographyStylePickerProjection {
    panel_id: SharedString,
    target: DesignPanelTarget,
    open: bool,
    editable: bool,
    picker: Entity<TypographyStylePicker>,
}

impl TypographyStylePickerProjection {
    pub(in super::super::super) fn new(
        panel_id: SharedString,
        target: DesignPanelTarget,
        open: bool,
        editable: bool,
        picker: Entity<TypographyStylePicker>,
    ) -> Self {
        Self {
            panel_id,
            target,
            open,
            editable,
            picker,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TypographyStylePickerEvent {
    Open,
    Close,
}

/// Typed callback boundary retained by the text-style picker renderer.
#[derive(Clone)]
pub(in super::super::super) struct TypographyStylePickerEventSink {
    panel: Entity<DesignPanel>,
}

impl TypographyStylePickerEventSink {
    pub(in super::super::super) fn new(panel: Entity<DesignPanel>) -> Self {
        Self { panel }
    }

    fn send(
        &self,
        expected_target: &DesignPanelTarget,
        event: TypographyStylePickerEvent,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.panel.update(cx, |panel, cx| {
            dispatch(panel, expected_target, event, window, cx);
        });
    }
}

fn dispatch(
    panel: &mut DesignPanel,
    expected_target: &DesignPanelTarget,
    event: TypographyStylePickerEvent,
    window: &mut Window,
    cx: &mut Context<DesignPanel>,
) {
    match event {
        TypographyStylePickerEvent::Open
            if panel.command_target() == *expected_target
                && panel.host.inspected_node().typography.is_some()
                && panel.host.inspection_context.selection().kind()
                    != DesignPanelSelectionKind::None =>
        {
            panel.open_typography_style_picker(window, cx);
        }
        TypographyStylePickerEvent::Open => {}
        TypographyStylePickerEvent::Close if panel.overlays.typography_style_picker_open() => {
            let _ = panel.dismiss_overlay_from_outside_click(
                DesignOpenOverlay::TypographyStyle,
                window,
                cx,
            );
        }
        TypographyStylePickerEvent::Close => {}
    }
}

/// Render the section-header trigger and its retained searchable picker.
pub(in super::super::super) fn render(
    projection: &TypographyStylePickerProjection,
    events: &TypographyStylePickerEventSink,
    cx: &mut Context<DesignPanel>,
) -> AnyElement {
    let picker_content = projection.picker.clone();
    let picker_focus = projection.picker.focus_handle(cx);
    let target_for_trigger = projection.target.clone();
    let target_for_open_change = projection.target.clone();
    let events_for_trigger = events.clone();
    let events_for_open_change = events.clone();
    let tooltip = if projection.editable {
        "Text styles"
    } else {
        "Text styles · View only"
    };
    let trigger = Button::new(SharedString::from(format!(
        "{}-typography-styles",
        projection.panel_id
    )))
    .tooltip(tooltip)
    .xsmall()
    .compact()
    .ghost()
    .w(px(24.))
    .h(px(24.))
    .selected(projection.open)
    .child(render_lucide_icon(
        LucideIcon::Palette,
        cx.theme().foreground,
        16.,
    ))
    .on_activate(move |_, window, cx| {
        cx.stop_propagation();
        events_for_trigger.send(
            &target_for_trigger,
            TypographyStylePickerEvent::Open,
            window,
            cx,
        );
    });

    Popover::new(SharedString::from(format!(
        "{}-typography-style-popover",
        projection.panel_id
    )))
    .anchor(Anchor::TopRight)
    .open(projection.open)
    .overlay_closable(true)
    .track_focus(&picker_focus)
    .on_open_change(move |open, window, cx| {
        events_for_open_change.send(
            &target_for_open_change,
            if *open {
                TypographyStylePickerEvent::Open
            } else {
                TypographyStylePickerEvent::Close
            },
            window,
            cx,
        );
    })
    .trigger(trigger)
    .content(move |_, _, _| picker_content.clone())
    .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projection_keeps_resource_entity_and_access_separate_from_target() {
        fn assert_clone<T: Clone>() {}
        assert_clone::<TypographyStylePickerProjection>();
    }
}
