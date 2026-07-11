use gpui::{Bounds, Modifiers, Pixels, TestAppContext, VisualTestContext, WindowHandle, px, size};

pub struct ComponentHarness {
    cx: VisualTestContext,
}

impl ComponentHarness {
    pub fn from_window<T: 'static>(window: WindowHandle<T>, cx: &mut TestAppContext) -> Self {
        Self {
            cx: VisualTestContext::from_window(window.into(), cx),
        }
    }

    pub fn draw(&mut self) {
        self.cx.update(|window, cx| window.draw(cx).clear());
    }

    pub fn bounds(&mut self, selector: &'static str) -> Option<Bounds<Pixels>> {
        self.cx.debug_bounds(selector)
    }

    pub fn click(&mut self, selector: &'static str) -> bool {
        self.draw();
        let Some(bounds) = self.bounds(selector) else {
            return false;
        };
        self.cx
            .simulate_click(bounds.center(), Modifiers::default());
        true
    }

    pub fn run_until_parked(&self) {
        self.cx.run_until_parked();
    }

    pub fn default_size() -> gpui::Size<Pixels> {
        size(px(960.), px(640.))
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::Cell, rc::Rc};

    use gpui::{Context, Render, TestAppContext, Window, px, size};
    use settings::SettingsStore;
    use ui::prelude::*;

    use super::ComponentHarness;
    use crate::{
        animation_panel::{AnimationDetailHeader, AnimationProperty, AnimationPropertyRow},
        timeline::TimelineToolbarButton,
    };

    #[derive(Default)]
    struct InteractionCounts {
        animation_select: Cell<usize>,
        animation_remove: Cell<usize>,
        disabled_animation: Cell<usize>,
        detail_close: Cell<usize>,
        timeline_play: Cell<usize>,
        disabled_timeline: Cell<usize>,
    }

    struct ControlGallery {
        counts: Rc<InteractionCounts>,
    }

    impl Render for ControlGallery {
        fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let counts = self.counts.clone();
            let select_counts = counts.clone();
            let remove_counts = counts.clone();
            let disabled_animation_counts = counts.clone();
            let disabled_animation_remove_counts = counts.clone();
            let close_counts = counts.clone();
            let play_counts = counts.clone();
            let disabled_timeline_counts = counts;

            v_flex()
                .size_full()
                .p_4()
                .bg(cx.theme().colors().editor_background)
                .child(
                    AnimationPropertyRow::new("interaction-position", AnimationProperty::Position)
                        .on_select(move |_, _| {
                            select_counts
                                .animation_select
                                .set(select_counts.animation_select.get() + 1);
                        })
                        .on_remove(move |_, _| {
                            remove_counts
                                .animation_remove
                                .set(remove_counts.animation_remove.get() + 1);
                        }),
                )
                .child(
                    AnimationPropertyRow::new("interaction-path-disabled", AnimationProperty::Path)
                        .enabled(false)
                        .on_select(move |_, _| {
                            disabled_animation_counts
                                .disabled_animation
                                .set(disabled_animation_counts.disabled_animation.get() + 1);
                        })
                        .on_remove(move |_, _| {
                            disabled_animation_remove_counts
                                .disabled_animation
                                .set(disabled_animation_remove_counts.disabled_animation.get() + 1);
                        }),
                )
                .child(
                    AnimationDetailHeader::new(AnimationProperty::Position).on_close(
                        move |_, _| {
                            close_counts
                                .detail_close
                                .set(close_counts.detail_close.get() + 1);
                        },
                    ),
                )
                .child(TimelineToolbarButton::new(
                    "interaction-play",
                    IconName::PlayFilled,
                    "Play",
                    move |_, _| {
                        play_counts
                            .timeline_play
                            .set(play_counts.timeline_play.get() + 1);
                    },
                ))
                .child(
                    TimelineToolbarButton::new(
                        "interaction-disabled-play",
                        IconName::Stop,
                        "Stop",
                        move |_, _| {
                            disabled_timeline_counts
                                .disabled_timeline
                                .set(disabled_timeline_counts.disabled_timeline.get() + 1);
                        },
                    )
                    .disabled(true),
                )
        }
    }

    fn init_test(cx: &mut TestAppContext) {
        cx.update(|cx| {
            let settings_store = SettingsStore::test(cx);
            cx.set_global(settings_store);
            theme_settings::init(theme::LoadThemes::JustBase, cx);
        });
    }

    fn open_gallery(cx: &mut TestAppContext, counts: Rc<InteractionCounts>) -> ComponentHarness {
        let window = cx.open_window(size(px(520.), px(360.)), {
            move |_, _| ControlGallery { counts }
        });
        ComponentHarness::from_window(window, cx)
    }

    #[gpui::test]
    fn animation_controls_dispatch_once_and_disabled_controls_are_inert(cx: &mut TestAppContext) {
        init_test(cx);
        let counts = Rc::new(InteractionCounts::default());
        let mut harness = open_gallery(cx, counts.clone());

        assert!(harness.click("FANTA-ANIMATION-PROPERTY-Position"));
        assert_eq!(counts.animation_select.get(), 1);

        assert!(harness.click("FANTA-ANIMATION-REMOVE-Position"));
        assert_eq!(counts.animation_remove.get(), 1);
        assert_eq!(counts.animation_select.get(), 1);

        assert!(harness.click("FANTA-ANIMATION-PROPERTY-Path"));
        assert!(harness.click("FANTA-ANIMATION-REMOVE-Path"));
        assert_eq!(counts.disabled_animation.get(), 0);

        assert!(harness.click("FANTA-ANIMATION-DETAIL-CLOSE"));
        assert_eq!(counts.detail_close.get(), 1);
        assert!(!harness.click("FANTA-CONTROL-THAT-DOES-NOT-EXIST"));
        harness.run_until_parked();
    }

    #[gpui::test]
    fn timeline_controls_dispatch_once_and_disabled_controls_are_inert(cx: &mut TestAppContext) {
        init_test(cx);
        let counts = Rc::new(InteractionCounts::default());
        let mut harness = open_gallery(cx, counts.clone());
        assert!(harness.click("FANTA-TIMELINE-CONTROL-interaction-play"));
        assert_eq!(counts.timeline_play.get(), 1);

        assert!(harness.click("FANTA-TIMELINE-CONTROL-interaction-disabled-play"));
        assert_eq!(counts.disabled_timeline.get(), 0);
        harness.run_until_parked();
    }
}
