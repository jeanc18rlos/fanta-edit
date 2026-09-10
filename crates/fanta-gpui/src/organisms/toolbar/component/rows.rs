//! Persistent dock assembly: one primary row combining the tool strip with a
//! utility segment, plus a mode-specific contextual strip when needed.

use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement, MouseButton, MouseDownEvent,
    ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _, Window, canvas,
    div, prelude::FluentBuilder as _, px,
};
use gpui_component::{ActiveTheme as _, Icon, IconName, Sizable as _, h_flex, tooltip::Tooltip};

use super::{
    AGENT_LAUNCHER_WIDTH, CHROME_CONTROL_SIZE, EditorToolbar, TOOL_SIZE,
    TOOLBAR_ZOOM_STEPPERS_MIN_WIDTH, ToolbarOverlay, ZoomClusterTier,
};
use crate::atoms::{ActivateControl, CONTROL_KEY_CONTEXT, ControlExt as _, icon_button};
use crate::molecules::{horizontal_fade_overlays, track_horizontal_edge_fades};
use crate::toolbar::icons::{render_icon_asset, render_mode_icon, render_tool_icon};
use crate::toolbar::{
    ToolbarChromeControl, ToolbarControlValue, ToolbarItem, ToolbarMode, ToolbarSecondaryControl,
    ToolbarTool, ToolbarToolGroup,
};

/// Width of the pointer-transparent edge fades that mark clipped row content.
const ROW_FADE_WIDTH: f32 = 24.;

impl EditorToolbar {
    fn render_tool_glyph(
        &self,
        tool: ToolbarTool,
        selected: bool,
        compact: bool,
        cx: &gpui::App,
    ) -> AnyElement {
        let color = if selected {
            Self::mode_accent_foreground(Self::mode_accent(self.mode, cx), cx)
        } else {
            cx.theme().foreground
        };
        render_tool_icon(tool, color, if compact { 15. } else { 17. })
    }

    fn render_tool_button(
        &self,
        tool: ToolbarTool,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let actions_open =
            tool == ToolbarTool::Actions && self.overlay == Some(ToolbarOverlay::Actions);
        let selected = self.active_tool == tool || actions_open;
        let accent = Self::mode_accent(self.mode, cx);
        let tooltip = Self::control_tooltip(tool.label(), tool.shortcut());
        let button = h_flex()
            .id(SharedString::from(format!(
                "{}-tool-{}",
                self.id,
                tool.label().to_lowercase().replace([' ', '/'], "-")
            )))
            .debug_selector({
                let label = tool.label();
                move || format!("toolbar-tool-{}", label.to_lowercase().replace(' ', "-"))
            })
            .key_context(CONTROL_KEY_CONTEXT)
            .tab_index(0)
            .size(px(TOOL_SIZE))
            .justify_center()
            .rounded(px(8.))
            .cursor_pointer()
            .border_1()
            .border_color(cx.theme().transparent)
            .bg(if selected {
                accent
            } else {
                cx.theme().transparent
            })
            .hover(|style| style.bg(if selected { accent } else { cx.theme().accent }))
            .focus(|style| style.border_color(cx.theme().selection))
            .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
            .when(tool == ToolbarTool::Actions, |button| {
                // Contract (§16): the Actions trigger activates on mouse-down
                // so it wins the race against the palette's capture-phase
                // outside-dismiss; `on_activate`'s click path would reopen the
                // palette that dismissal just closed. Keyboard activation
                // converges on the same method through `ActivateControl`.
                button
                    .on_action(cx.listener(move |this, _: &ActivateControl, window, cx| {
                        this.activate_tool_or_actions(tool, actions_open, window, cx);
                    }))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _: &MouseDownEvent, window, cx| {
                            // The trigger's default focus would otherwise replace
                            // the query focus set while opening the palette.
                            window.prevent_default();
                            this.activate_tool_or_actions(tool, actions_open, window, cx);
                        }),
                    )
            })
            .when(tool != ToolbarTool::Actions, |button| {
                button.on_activate(cx.listener(move |this, _, window, cx| {
                    this.activate_tool_or_actions(tool, actions_open, window, cx);
                }))
            })
            .child(self.render_tool_glyph(tool, selected, matches!(tool, ToolbarTool::Code), cx));

        h_flex()
            .relative()
            .flex_none()
            .items_start()
            .child(button)
            .when(
                tool == ToolbarTool::Actions && self.overlay == Some(ToolbarOverlay::Actions),
                |wrapper| wrapper.child(self.render_actions_palette(window, cx)),
            )
            .into_any_element()
    }

    fn render_tool_group(
        &self,
        group: ToolbarToolGroup,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let tool = group.display_tool(self.active_tool);
        let selected = group.tools().contains(&self.active_tool);
        let menu_open = self.overlay == Some(ToolbarOverlay::ToolGroup(group));
        let accent = Self::mode_accent(self.mode, cx);
        let tooltip = Self::control_tooltip(tool.label(), tool.shortcut());

        h_flex()
            .relative()
            .h(px(TOOL_SIZE))
            .flex_none()
            .items_start()
            .child(
                h_flex()
                    .id(SharedString::from(format!(
                        "{}-group-{}-main",
                        self.id,
                        group.label().to_lowercase().replace(' ', "-")
                    )))
                    .key_context(CONTROL_KEY_CONTEXT)
                    .tab_index(0)
                    .size(px(TOOL_SIZE))
                    .justify_center()
                    .rounded(px(8.))
                    .cursor_pointer()
                    .border_1()
                    .border_color(cx.theme().transparent)
                    .bg(if selected {
                        accent
                    } else {
                        cx.theme().transparent
                    })
                    .hover(|style| style.bg(if selected { accent } else { cx.theme().accent }))
                    .focus(|style| style.border_color(cx.theme().selection))
                    .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
                    .on_activate(cx.listener(move |this, _, _, cx| {
                        this.request_tool(tool, cx);
                    }))
                    .child(self.render_tool_glyph(tool, selected, false, cx)),
            )
            .child(
                h_flex()
                    .id(SharedString::from(format!(
                        "{}-group-{}-menu",
                        self.id,
                        group.label().to_lowercase().replace(' ', "-")
                    )))
                    .debug_selector({
                        let label = group.label();
                        move || {
                            format!(
                                "toolbar-group-{}-trigger",
                                label.to_lowercase().replace(' ', "-")
                            )
                        }
                    })
                    .key_context(CONTROL_KEY_CONTEXT)
                    .tab_index(0)
                    .w(px(12.))
                    .h_full()
                    .justify_center()
                    .rounded(px(5.))
                    .cursor_pointer()
                    .border_1()
                    .border_color(cx.theme().transparent)
                    .text_color(cx.theme().muted_foreground)
                    .hover(|style| style.bg(cx.theme().accent))
                    .focus(|style| style.border_color(cx.theme().selection))
                    // Contract (§16): press-activation so the caret wins the
                    // race against its flyout's capture-phase outside-dismiss;
                    // a click handler would reopen the flyout it just closed.
                    // Enter/Space on an open flyout commit its highlighted row.
                    .on_action(cx.listener(move |this, _: &ActivateControl, window, cx| {
                        if !(menu_open && this.commit_open_menu_entry(window, cx)) {
                            this.toggle_tool_group(group, menu_open, cx);
                        }
                    }))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _: &MouseDownEvent, _, cx| {
                            this.toggle_tool_group(group, menu_open, cx);
                        }),
                    )
                    .child(
                        Icon::new(if menu_open {
                            IconName::ChevronUp
                        } else {
                            IconName::ChevronDown
                        })
                        .xsmall(),
                    ),
            )
            .when(menu_open, |control| {
                control.child(self.render_tool_group_menu(group, window, cx))
            })
            .into_any_element()
    }

    fn render_mode_button(&self, mode: ToolbarMode, cx: &mut Context<Self>) -> AnyElement {
        let selected = self.mode == mode;
        let accent = Self::mode_accent(mode, cx);
        let tooltip = Self::control_tooltip(mode.label(), mode.shortcut());
        h_flex()
            .id(SharedString::from(format!(
                "{}-mode-{}",
                self.id,
                mode.label().to_lowercase()
            )))
            .debug_selector({
                let label = mode.label();
                move || format!("toolbar-mode-{}", label.to_lowercase())
            })
            .key_context(CONTROL_KEY_CONTEXT)
            .tab_index(0)
            .size(px(28.))
            .justify_center()
            .rounded(px(8.))
            .cursor_pointer()
            .border_1()
            .border_color(cx.theme().transparent)
            .bg(if selected {
                cx.theme().background
            } else {
                cx.theme().transparent
            })
            .when(selected && cx.theme().shadow, |button| button.shadow_sm())
            .hover(|style| style.bg(cx.theme().background))
            .focus(|style| style.border_color(cx.theme().selection))
            .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
            .on_activate(cx.listener(move |this, _, _, cx| {
                this.request_mode(mode, cx);
            }))
            .child(render_mode_icon(
                mode,
                if selected {
                    accent
                } else {
                    cx.theme().muted_foreground
                },
                17.,
            ))
            .into_any_element()
    }

    pub(super) fn render_main_toolbar(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut tools = h_flex().h_full().gap(px(2.));
        for item in self.mode.layout() {
            tools = match item {
                ToolbarItem::Group(group) => {
                    tools.child(self.render_tool_group(*group, window, cx))
                }
                ToolbarItem::Tool(ToolbarTool::Actions) => tools,
                ToolbarItem::Tool(tool) => tools.child(self.render_tool_button(*tool, window, cx)),
                ToolbarItem::Separator => {
                    tools.child(div().w(px(1.)).h(px(24.)).mx_1().bg(cx.theme().border))
                }
            };
        }

        h_flex()
            .relative()
            .flex_shrink_1()
            .max_w_full()
            .min_w(px(0.))
            .child(
                // A flex wrapper so the scroll viewport keeps its flex-item
                // sizing; a block wrapper collapses a scroll child to zero
                // width and culls the row's hitboxes.
                h_flex()
                    .relative()
                    .min_w(px(0.))
                    .flex_shrink_1()
                    .child(
                        // The viewport itself is a flex row: a block scroll
                        // container clamps its row to the viewport width, so
                        // the content never overflows and cannot scroll.
                        div()
                            .id(SharedString::from(format!("{}-primary-viewport", self.id)))
                            .debug_selector(|| "toolbar-primary-viewport".to_owned())
                            .flex()
                            .min_w(px(0.))
                            .flex_shrink_1()
                            .overflow_x_scroll()
                            .track_scroll(&self.primary_scroll_handle)
                            .child(
                                h_flex()
                                    .id(SharedString::from(format!("{}-primary", self.id)))
                                    .debug_selector(|| "toolbar-primary-row".to_owned())
                                    .h(px(40.))
                                    .px_1()
                                    .flex_none()
                                    .child(tools),
                            ),
                    )
                    .children(horizontal_fade_overlays(
                        self.primary_fades,
                        px(ROW_FADE_WIDTH),
                        cx.theme().popover,
                        "toolbar-primary",
                    ))
                    .child(track_horizontal_edge_fades(
                        cx.entity(),
                        self.primary_scroll_handle.clone(),
                        |this| this.primary_fades,
                        |this, fades| this.primary_fades = fades,
                    )),
            )
            .child(div().w(px(1.)).h(px(24.)).flex_none().bg(cx.theme().border))
            // Keep Actions visible while mode-specific tools scroll, so a
            // keyboard invocation always has a real, nearby anchor.
            .child(self.render_tool_button(ToolbarTool::Actions, window, cx))
            .into_any_element()
    }

    fn render_mode_tray(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut modes = h_flex()
            .h(px(36.))
            .flex_none()
            .px_1()
            .gap(px(2.))
            .rounded(px(10.))
            .bg(cx.theme().secondary);
        for mode in ToolbarMode::ALL {
            modes = modes.child(self.render_mode_button(*mode, cx));
        }
        modes.into_any_element()
    }

    pub(super) fn render_utility_toolbar(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let tier = self.zoom_cluster_tier();
        let toolbar = cx.entity();
        // A flex wrapper so the scroll viewport keeps its flex-item sizing; a
        // block wrapper collapses a scroll child to zero width and culls the
        // row's hitboxes. The full-tier width is its shrinkable basis; host
        // chrome extends that basis rather than stealing room from zoom.
        h_flex()
            .relative()
            .w(px(
                TOOLBAR_ZOOM_STEPPERS_MIN_WIDTH + self.chrome_cluster_width()
            ))
            .flex_shrink_1()
            .max_w_full()
            .min_w(px(0.))
            .child(
                // Measures the dock's inner width so the zoom cluster can
                // collapse below the named breakpoints. The write is
                // deferred past the draw so a changed width schedules a
                // re-render (notifying mid-draw is a no-op).
                canvas(
                    move |bounds, _, app| {
                        let width = f32::from(bounds.size.width);
                        let known = toolbar.read(app).utility_width;
                        if known.is_none_or(|known| (known - width).abs() > 0.5) {
                            let toolbar = toolbar.clone();
                            app.defer(move |app| {
                                toolbar.update(app, |this, cx| {
                                    this.utility_width = Some(width);
                                    cx.notify();
                                });
                            });
                        }
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .left_0()
                .top_0()
                .size_full(),
            )
            .child(
                // The viewport itself is a flex row: a block scroll container
                // clamps its row to the viewport width, so the content never
                // overflows and cannot scroll.
                div()
                    .id(SharedString::from(format!("{}-utility-viewport", self.id)))
                    .debug_selector(|| "toolbar-utility-viewport".to_owned())
                    .flex()
                    .min_w(px(0.))
                    .flex_shrink_1()
                    .overflow_x_scroll()
                    .track_scroll(&self.utility_scroll_handle)
                    .child(
                        h_flex()
                            .debug_selector(|| "toolbar-utility-row".to_owned())
                            .min_w_full()
                            .flex_none()
                            .h(px(40.))
                            .px_1()
                            .gap_1()
                            .child(self.render_mode_tray(cx))
                            .when(tier != ZoomClusterTier::Hidden, |row| {
                                row.child(self.render_zoom_control(tier, window, cx)).child(
                                    div().w(px(1.)).h(px(22.)).flex_none().bg(cx.theme().border),
                                )
                            })
                            .child(self.render_agent_launcher(window, cx))
                            .when(!self.chrome_controls.is_empty(), |row| {
                                row.child(
                                    div().w(px(1.)).h(px(22.)).flex_none().bg(cx.theme().border),
                                )
                                .child(self.render_chrome_cluster(cx))
                            }),
                    ),
            )
            .children(horizontal_fade_overlays(
                self.utility_fades,
                px(ROW_FADE_WIDTH),
                cx.theme().popover,
                "toolbar-utility",
            ))
            .child(track_horizontal_edge_fades(
                cx.entity(),
                self.utility_scroll_handle.clone(),
                |this| this.utility_fades,
                |this, fades| this.utility_fades = fades,
            ))
            .into_any_element()
    }

    fn render_secondary_button(
        &self,
        suffix: &'static str,
        label: impl Into<SharedString>,
        control: ToolbarSecondaryControl,
        selected: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let label = label.into();
        let accent = Self::mode_accent(self.mode, cx);
        let pale_accent = Self::mode_accent_pale(self.mode, cx);
        h_flex()
            .id(SharedString::from(format!(
                "{}-secondary-{suffix}",
                self.id
            )))
            .debug_selector(move || format!("toolbar-secondary-{suffix}"))
            .key_context(CONTROL_KEY_CONTEXT)
            .tab_index(0)
            .h(px(28.))
            .px_2()
            .gap_1()
            .rounded(px(7.))
            .cursor_pointer()
            .text_xs()
            .border_1()
            .border_color(cx.theme().transparent)
            .bg(if selected {
                pale_accent
            } else {
                cx.theme().transparent
            })
            .text_color(if selected {
                accent
            } else {
                cx.theme().popover_foreground
            })
            .hover(|style| style.bg(cx.theme().accent))
            .focus(|style| style.border_color(cx.theme().selection))
            .on_activate(cx.listener(move |this, _, _, cx| {
                this.request_secondary(control, cx);
            }))
            .child(label)
            .into_any_element()
    }

    fn render_value_button(
        &self,
        suffix: &'static str,
        label: impl Into<SharedString>,
        control: ToolbarSecondaryControl,
        value: ToolbarControlValue,
        selected: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let label = label.into();
        let accent = Self::mode_accent(self.mode, cx);
        let pale_accent = Self::mode_accent_pale(self.mode, cx);
        h_flex()
            .id(SharedString::from(format!(
                "{}-secondary-{suffix}",
                self.id
            )))
            .debug_selector(move || format!("toolbar-secondary-{suffix}"))
            .key_context(CONTROL_KEY_CONTEXT)
            .tab_index(0)
            .h(px(28.))
            .px_2()
            .gap_1()
            .rounded(px(7.))
            .cursor_pointer()
            .text_xs()
            .border_1()
            .border_color(cx.theme().transparent)
            .bg(if selected {
                pale_accent
            } else {
                cx.theme().transparent
            })
            .text_color(if selected {
                accent
            } else {
                cx.theme().popover_foreground
            })
            .hover(|style| style.bg(cx.theme().accent))
            .focus(|style| style.border_color(cx.theme().selection))
            .on_activate(cx.listener(move |this, _, _, cx| {
                this.request_control_value(control, value.clone(), cx);
            }))
            .child(label)
            .into_any_element()
    }

    /// A secondary chip that opens an anchored editor of host-supplied
    /// candidates instead of cycling a hardcoded next value.
    fn render_editor_chip(
        &self,
        suffix: &'static str,
        label: impl Into<SharedString>,
        control: ToolbarSecondaryControl,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let label = label.into();
        let open = self.overlay == Some(ToolbarOverlay::OptionEditor(control));
        h_flex()
            .relative()
            .h(px(28.))
            .flex_none()
            .items_start()
            .child(
                h_flex()
                    .id(SharedString::from(format!(
                        "{}-secondary-{suffix}",
                        self.id
                    )))
                    .debug_selector(move || format!("toolbar-secondary-{suffix}"))
                    .key_context(CONTROL_KEY_CONTEXT)
                    .tab_index(0)
                    .h(px(28.))
                    .px_2()
                    .gap_1()
                    .rounded(px(7.))
                    .cursor_pointer()
                    .text_xs()
                    .border_1()
                    .border_color(cx.theme().transparent)
                    .text_color(cx.theme().popover_foreground)
                    .when(open, |chip| chip.bg(cx.theme().accent))
                    .hover(|style| style.bg(cx.theme().accent))
                    .focus(|style| style.border_color(cx.theme().selection))
                    // Contract (§16): press-activation so the chip wins the
                    // race against its editor's capture-phase outside-dismiss;
                    // a click handler would reopen the editor it just closed.
                    // Enter/Space on an open choice editor commit its
                    // highlighted candidate.
                    .on_action(cx.listener(move |this, _: &ActivateControl, window, cx| {
                        if !(open && this.commit_open_menu_entry(window, cx)) {
                            this.toggle_option_editor(control, open, cx);
                        }
                    }))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _: &MouseDownEvent, _, cx| {
                            this.toggle_option_editor(control, open, cx);
                        }),
                    )
                    .child(label)
                    .child(
                        Icon::new(if open {
                            IconName::ChevronUp
                        } else {
                            IconName::ChevronDown
                        })
                        .xsmall()
                        .text_color(cx.theme().muted_foreground),
                    ),
            )
            .when(open, |chip| {
                chip.child(self.render_option_editor(control, window, cx))
            })
            .into_any_element()
    }

    fn render_dev_secondary(&self, cx: &mut Context<Self>) -> AnyElement {
        h_flex()
            .flex_none()
            .h(px(40.))
            .px_1()
            .gap_1()
            .rounded(px(10.))
            .bg(cx.theme().secondary)
            .text_color(cx.theme().popover_foreground)
            .child(self.render_secondary_button(
                "dev-inspect",
                "Inspect",
                ToolbarSecondaryControl::DevInspect,
                self.active_tool == ToolbarTool::Inspect,
                cx,
            ))
            .child(self.render_secondary_button(
                "dev-annotate",
                "Annotate",
                ToolbarSecondaryControl::DevAnnotate,
                self.active_tool == ToolbarTool::Annotation,
                cx,
            ))
            .child(self.render_secondary_button(
                "dev-measure",
                "Measure",
                ToolbarSecondaryControl::DevMeasure,
                self.active_tool == ToolbarTool::Measure,
                cx,
            ))
            .child(div().w(px(1.)).h(px(22.)).bg(cx.theme().border))
            .child(self.render_value_button(
                "dev-ready",
                if self.dev_options.ready_for_development {
                    "Ready for dev"
                } else {
                    "Mark ready for dev"
                },
                ToolbarSecondaryControl::DevReadyForDevelopment,
                ToolbarControlValue::Toggle(!self.dev_options.ready_for_development),
                self.dev_options.ready_for_development,
                cx,
            ))
            .into_any_element()
    }

    fn render_motion_secondary(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let current_seconds = self.motion_options.current_time_ms as f32 / 1_000.;
        let duration_seconds = self.motion_options.duration_ms as f32 / 1_000.;
        h_flex()
            .flex_none()
            .h(px(40.))
            .px_1()
            .gap_1()
            .rounded(px(10.))
            .bg(cx.theme().secondary)
            .text_color(cx.theme().popover_foreground)
            .child(self.render_value_button(
                "motion-play",
                if self.motion_options.playing {
                    "Pause"
                } else {
                    "Play"
                },
                ToolbarSecondaryControl::MotionPlayPause,
                ToolbarControlValue::Toggle(!self.motion_options.playing),
                self.motion_options.playing,
                cx,
            ))
            .child(self.render_value_button(
                "motion-loop",
                "Loop",
                ToolbarSecondaryControl::MotionLoop,
                ToolbarControlValue::Toggle(!self.motion_options.looping),
                self.motion_options.looping,
                cx,
            ))
            .child(
                div()
                    .h(px(28.))
                    .px_2()
                    .flex()
                    .items_center()
                    .rounded(px(7.))
                    .bg(cx.theme().secondary)
                    .text_xs()
                    .child(format!("{current_seconds:.1}s / {duration_seconds:.1}s")),
            )
            .child(self.render_value_button(
                "motion-autokey",
                "Auto key",
                ToolbarSecondaryControl::MotionAutoKeyframe,
                ToolbarControlValue::Toggle(!self.motion_options.auto_keyframe),
                self.motion_options.auto_keyframe,
                cx,
            ))
            .child(self.render_editor_chip(
                "motion-keyframe",
                "Keyframe",
                ToolbarSecondaryControl::MotionAddKeyframe,
                window,
                cx,
            ))
            .child(self.render_editor_chip(
                "motion-style",
                "Animate",
                ToolbarSecondaryControl::MotionAnimationStyle,
                window,
                cx,
            ))
            .child(self.render_secondary_button(
                "motion-timeline",
                "Timeline",
                ToolbarSecondaryControl::MotionTimeline,
                true,
                cx,
            ))
            .child(self.render_secondary_button(
                "motion-time-comment",
                "Comment",
                ToolbarSecondaryControl::MotionTimeComment,
                false,
                cx,
            ))
            .into_any_element()
    }

    pub(super) fn render_secondary_toolbar(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let content = match self.mode {
            ToolbarMode::Design => return div().into_any_element(),
            ToolbarMode::Dev => self.render_dev_secondary(cx),
            ToolbarMode::Motion => self.render_motion_secondary(window, cx),
        };
        // A flex wrapper so the scroll viewport keeps its flex-item sizing; a
        // block wrapper collapses a scroll child to zero width and culls the
        // row's hitboxes.
        h_flex()
            .relative()
            .max_w_full()
            .min_w(px(0.))
            .child(
                // The viewport itself is a flex row: a block scroll container
                // clamps its row to the viewport width, so the content never
                // overflows and cannot scroll.
                div()
                    .id(SharedString::from(format!(
                        "{}-secondary-viewport",
                        self.id
                    )))
                    .debug_selector(|| "toolbar-secondary-viewport".to_owned())
                    .flex()
                    .w_full()
                    .min_w(px(0.))
                    .overflow_x_scroll()
                    .track_scroll(&self.secondary_scroll_handle)
                    .child(content),
            )
            .children(horizontal_fade_overlays(
                self.secondary_fades,
                px(ROW_FADE_WIDTH),
                cx.theme().popover,
                "toolbar-secondary",
            ))
            .child(track_horizontal_edge_fades(
                cx.entity(),
                self.secondary_scroll_handle.clone(),
                |this| this.secondary_fades,
                |this, fades| this.secondary_fades = fades,
            ))
            .into_any_element()
    }

    /// One − / + button of the zoom cluster, stepping the shared ladder.
    fn render_zoom_stepper(&self, zoom_in: bool, cx: &mut Context<Self>) -> AnyElement {
        let (suffix, icon) = if zoom_in {
            ("in", IconName::Plus)
        } else {
            ("out", IconName::Minus)
        };
        icon_button(
            SharedString::from(format!("{}-zoom-{suffix}", self.id)),
            px(24.),
            px(7.),
            cx,
        )
        .debug_selector(move || format!("toolbar-zoom-{suffix}"))
        .on_activate(cx.listener(move |this, _, _, cx| {
            this.request_zoom(Self::zoom_ladder_step(this.zoom_percent, zoom_in), cx);
        }))
        .child(Icon::new(icon).xsmall())
        .into_any_element()
    }

    fn render_zoom_control(
        &self,
        tier: ZoomClusterTier,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let open = self.overlay == Some(ToolbarOverlay::Zoom);
        h_flex()
            .relative()
            .h(px(36.))
            .flex_none()
            .px_1()
            .gap(px(1.))
            .rounded(px(10.))
            .bg(cx.theme().secondary)
            .text_color(cx.theme().popover_foreground)
            .debug_selector(|| "toolbar-zoom-control".to_owned())
            .when(tier == ZoomClusterTier::Full, |cluster| {
                cluster.child(self.render_zoom_stepper(false, cx))
            })
            .child(
                h_flex()
                    .relative()
                    .items_start()
                    .w(px(58.))
                    .h(px(28.))
                    .flex_none()
                    .child(
                        h_flex()
                            .id(SharedString::from(format!("{}-zoom-menu", self.id)))
                            .debug_selector(|| "toolbar-zoom-menu-trigger".to_owned())
                            .key_context(CONTROL_KEY_CONTEXT)
                            .tab_index(0)
                            .size_full()
                            .px_1()
                            .gap_1()
                            .justify_center()
                            .rounded(px(7.))
                            .cursor_pointer()
                            .border_1()
                            .border_color(cx.theme().transparent)
                            .hover(|style| style.bg(cx.theme().accent))
                            .focus(|style| style.border_color(cx.theme().selection))
                            // Contract (§16): press-activation so the trigger
                            // wins the race against the zoom menu's
                            // capture-phase outside-dismiss; a click handler
                            // would reopen the menu it just closed. Enter and
                            // Space on an open menu commit its highlighted row.
                            .on_action(cx.listener(move |this, _: &ActivateControl, window, cx| {
                                if !(open && this.commit_open_menu_entry(window, cx)) {
                                    this.toggle_zoom(open, cx);
                                }
                            }))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, _: &MouseDownEvent, _, cx| {
                                    this.toggle_zoom(open, cx);
                                }),
                            )
                            .child(div().text_xs().child(format!("{}%", self.zoom_percent)))
                            .child(
                                Icon::new(if open {
                                    IconName::ChevronUp
                                } else {
                                    IconName::ChevronDown
                                })
                                .xsmall(),
                            ),
                    )
                    .when(open, |trigger| {
                        trigger.child(self.render_zoom_menu(window, cx))
                    }),
            )
            .when(tier == ZoomClusterTier::Full, |cluster| {
                cluster.child(self.render_zoom_stepper(true, cx))
            })
            .into_any_element()
    }

    /// One host chrome control: a compact icon tile whose active state uses
    /// the same raised-tile treatment as the selected mode.
    fn render_chrome_control(
        &self,
        control: &ToolbarChromeControl,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = control.id.clone();
        let selector_id = control.id.to_string();
        let state_selector = format!(
            "toolbar-chrome-{}-{}",
            control.id,
            if control.active { "active" } else { "inactive" }
        );
        let tooltip: SharedString = match &control.shortcut {
            Some(shortcut) => format!("{}  {shortcut}", control.label).into(),
            None => control.label.clone(),
        };
        let active = control.active;
        icon_button(
            SharedString::from(format!("{}-chrome-{}", self.id, control.id)),
            px(CHROME_CONTROL_SIZE),
            px(8.),
            cx,
        )
        .debug_selector(move || format!("toolbar-chrome-{selector_id}"))
        .flex_none()
        .when(active, |button| {
            button
                .bg(cx.theme().background)
                .when(cx.theme().shadow, |button| button.shadow_sm())
        })
        .text_color(if active {
            cx.theme().foreground
        } else {
            cx.theme().muted_foreground
        })
        .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
        .on_activate(cx.listener(move |this, _, _, cx| {
            this.request_chrome_control(id.clone(), cx);
        }))
        .child(
            div()
                .debug_selector(move || state_selector.clone())
                .flex()
                .child(render_icon_asset(
                    control.icon.clone(),
                    if active {
                        cx.theme().foreground
                    } else {
                        cx.theme().muted_foreground
                    },
                    15.,
                )),
        )
        .into_any_element()
    }

    /// The host chrome capsule at the end of the utility row (§12): a
    /// secondary-filled pill matching the mode tray, one tile per control.
    fn render_chrome_cluster(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut cluster = h_flex()
            .debug_selector(|| "toolbar-chrome-cluster".to_owned())
            .h(px(36.))
            .flex_none()
            .px_1()
            .gap(px(2.))
            .rounded(px(10.))
            .bg(cx.theme().secondary);
        for control in &self.chrome_controls {
            cluster = cluster.child(self.render_chrome_control(control, cx));
        }
        cluster.into_any_element()
    }

    fn render_agent_launcher(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let open = self.overlay == Some(ToolbarOverlay::Agent);
        icon_button(
            SharedString::from(format!("{}-agent-launcher", self.id)),
            px(32.),
            px(8.),
            cx,
        )
        .w(px(AGENT_LAUNCHER_WIDTH))
        .debug_selector(|| "toolbar-agent-launcher".to_owned())
        // Top alignment keeps the anchored composer's origin at the
        // launcher's top edge, exactly like the other popup-hosting triggers.
        .relative()
        .flex_none()
        .items_start()
        .bg(if open {
            cx.theme().magenta_light
        } else {
            cx.theme().secondary
        })
        .text_color(cx.theme().magenta)
        .text_xs()
        .tooltip(|window, cx| Tooltip::new("Ask AI · Command/Ctrl+Enter").build(window, cx))
        // Contract (§16): press-activation so the launcher wins the race
        // against the composer's capture-phase outside-dismiss; a click
        // handler would reopen the composer it just closed.
        .on_action(cx.listener(move |this, _: &ActivateControl, window, cx| {
            this.toggle_agent(open, window, cx);
        }))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _: &MouseDownEvent, window, cx| {
                window.prevent_default();
                this.toggle_agent(open, window, cx);
            }),
        )
        .child(
            h_flex()
                .size_full()
                .gap_1()
                .justify_center()
                .child(Icon::new(IconName::Bot).xsmall())
                .child(
                    div()
                        .debug_selector(|| "toolbar-agent-label".to_owned())
                        .child("Ask AI"),
                ),
        )
        .when(open, |launcher| {
            launcher.child(self.render_agent_composer(window, cx))
        })
        .into_any_element()
    }
}
