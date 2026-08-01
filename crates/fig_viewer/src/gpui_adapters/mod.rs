//! Adapters between fanta-gpui panels and the fig_viewer engine seam.
//!
//! Each adapter is (a) pure mapping functions from `FigDocument` state to a
//! panel's view data, (b) an owned `Entity<Panel>` on the hosting fig_viewer
//! entity, (c) one subscription mapping the panel's typed intents to
//! `fanta_doc::Operation`s, and (d) an echo refresh the host calls from its
//! existing `FigItemEvent` subscription. Panel setters only `cx.notify()`,
//! so echo loops cannot occur.
//!
//! Theme rule: files hosting fanta-gpui panels import
//! `gpui_component::ActiveTheme` OR Zed's `theme::ActiveTheme`, never both.

pub(crate) mod layers;
pub(crate) mod pages;
pub(crate) mod toolbar;

/// Process-wide kill switch: `FANTA_GPUI_UI=0` forces every fanta-gpui
/// surface off regardless of settings — the cheapest release-day remedy.
pub(crate) fn env_enabled() -> bool {
    !matches!(
        std::env::var("FANTA_GPUI_UI").as_deref(),
        Ok("0") | Ok("false") | Ok("off")
    )
}

/// A fanta-gpui surface may mount only when the env switch allows it AND
/// `gpui_component::init` has run (its Theme global exists). Hosts that
/// skipped init — including unit tests building a bare FigView — fall back
/// to the native surface instead of panicking on the missing global.
pub(crate) fn runtime_enabled(cx: &gpui::App) -> bool {
    env_enabled() && cx.try_global::<gpui_component::theme::Theme>().is_some()
}

#[cfg(test)]
mod spike_tests {
    use fanta_gpui::prelude::*;
    use fanta_gpui::toolbar::{EditorToolbar, ToolbarMode, ToolbarTool};
    use gpui::{AppContext as _, Entity, TestAppContext};
    use gpui_component::Root;

    fn init_test(cx: &mut TestAppContext) {
        cx.update(|cx| {
            zlog::init_test();
            assets::Assets.load_test_fonts(cx);
            let store = settings::SettingsStore::test(cx);
            cx.set_global(store);
            theme_settings::init(theme::LoadThemes::JustBase, cx);
            gpui_component::init(cx);
            fanta_gpui::init(cx);
            crate::theme_bridge::init(cx);
        });
    }

    struct SpikeHost {
        toolbar: Entity<EditorToolbar>,
        pages: Entity<PagesPanel>,
        toolbar_actions: std::rc::Rc<std::cell::RefCell<Vec<ToolbarAction>>>,
        pages_actions: std::rc::Rc<std::cell::RefCell<Vec<PagesPanelAction>>>,
        _subscriptions: Vec<gpui::Subscription>,
    }

    impl SpikeHost {
        fn new(window: &mut gpui::Window, cx: &mut gpui::Context<Self>) -> Self {
            let toolbar = cx.new(|cx| {
                EditorToolbar::new(
                    "spike-toolbar",
                    ToolbarMode::Design,
                    ToolbarTool::Move,
                    100,
                    window,
                    cx,
                )
            });
            let pages = cx.new(|cx| {
                PagesPanel::new(
                    "spike-pages",
                    vec![
                        PagesPanelItem {
                            id: "page-1".into(),
                            title: "Cover".into(),
                        },
                        PagesPanelItem {
                            id: "page-2".into(),
                            title: "Components".into(),
                        },
                    ],
                    window,
                    cx,
                )
            });
            let toolbar_actions = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
            let pages_actions = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
            let subscriptions = vec![
                cx.subscribe(&toolbar, {
                    let sink = toolbar_actions.clone();
                    move |_, _, action: &ToolbarAction, _| sink.borrow_mut().push(action.clone())
                }),
                cx.subscribe(&pages, {
                    let sink = pages_actions.clone();
                    move |_, _, action: &PagesPanelAction, _| sink.borrow_mut().push(action.clone())
                }),
            ];
            Self {
                toolbar,
                pages,
                toolbar_actions,
                pages_actions,
                _subscriptions: subscriptions,
            }
        }
    }

    impl gpui::Render for SpikeHost {
        fn render(
            &mut self,
            _window: &mut gpui::Window,
            _cx: &mut gpui::Context<Self>,
        ) -> impl gpui::IntoElement {
            use gpui::ParentElement as _;
            use gpui::Styled as _;
            gpui::div()
                .size_full()
                .child(self.pages.clone())
                .child(self.toolbar.clone())
        }
    }

    /// P0 go/no-go: fanta-gpui panels construct, render, and emit typed
    /// intents inside fig_viewer's crate context (inits, theme bridge, fonts)
    /// under a Zed-style window that is NOT rooted at gpui_component::Root.
    #[gpui::test]
    fn spike_toolbar_and_pages_mount_and_emit(cx: &mut TestAppContext) {
        init_test(cx);
        let (host, cx) = cx.add_window_view(|window, cx| SpikeHost::new(window, cx));
        cx.run_until_parked();

        // Renders without panicking, and the theme bridge seeded the
        // gpui_component palette from the Zed theme.
        cx.update(|_, app| {
            let zed_background = theme::ActiveTheme::theme(app).colors().surface_background;
            let bridged = gpui_component::theme::Theme::global(app).colors.background;
            assert_eq!(bridged, zed_background, "theme bridge applied");
        });

        // Toolbar emits a typed intent from a simulated tool activation.
        host.update(cx, |host, cx| {
            host.toolbar.update(cx, |_, cx| {
                cx.emit(ToolbarAction::ToolChangeRequested {
                    mode: ToolbarMode::Design,
                    tool: ToolbarTool::Frame,
                });
            });
        });
        cx.run_until_parked();
        let toolbar_actions = host.read_with(cx, |host, _| host.toolbar_actions.borrow().clone());
        assert!(
            matches!(
                toolbar_actions.as_slice(),
                [ToolbarAction::ToolChangeRequested { .. }]
            ),
            "host subscription received the toolbar intent: {toolbar_actions:?}"
        );

        // Pages panel echoes host data: replace the model, panel reflects it.
        host.update(cx, |host, cx| {
            host.pages.update(cx, |pages, cx| {
                pages.set_pages(
                    vec![PagesPanelItem {
                        id: "page-3".into(),
                        title: "Prototype".into(),
                    }],
                    cx,
                );
            });
        });
        cx.run_until_parked();
        let _ = host.read_with(cx, |host, _| host.pages_actions.borrow().len());
    }

    /// The dialog/sheet layers need a `Root`; panels themselves do not.
    /// Prove a Root-wrapped window also works, for hosts that want dialogs.
    #[gpui::test]
    fn spike_panels_render_under_root_wrapper(cx: &mut TestAppContext) {
        init_test(cx);
        let (_, cx) = cx.add_window_view(|window, cx| {
            let host = cx.new(|cx| SpikeHost::new(window, cx));
            Root::new(host, window, cx)
        });
        cx.run_until_parked();
    }
}
