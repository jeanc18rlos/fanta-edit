use gpui::{
    AppContext as _, Context, Entity, IntoElement, Modifiers, ParentElement as _, Render,
    Subscription, TestAppContext, VisualTestContext, Window, div, px, size,
};
use gpui_component::Root;

use super::*;
use crate::{
    assets::{AssetsPanel, AssetsViewData},
    design::{DesignPanel, DesignPanelNode, DesignPanelNodeKind, DesignPanelWorkspaceMode},
    layers::LayersPanel,
    pages::PagesPanel,
    prototype::{PrototypePanel, PrototypeViewData},
    timeline::{Timeline, TimelineAction, TimelineViewData},
    toolbar::{EditorToolbar, ToolbarAction, ToolbarMode, ToolbarTool},
    variables::{VariablesPage, VariablesViewData},
};

#[derive(Clone, Debug, PartialEq)]
enum WorkflowIntent {
    Toolbar(ToolbarAction),
    Timeline(TimelineAction),
}

/// A minimal application host for the composed editor.
///
/// It deliberately owns the accepted toolbar and timeline snapshots and the
/// Design panel's workspace projection, mirroring the cross-component
/// orchestration a production host performs: a toolbar mode change re-seeds
/// the tool, and the host alone decides what the inspector projects.
struct WorkflowHost {
    editor: Entity<PseudoEditor>,
    toolbar: Entity<EditorToolbar>,
    design: Entity<DesignPanel>,
    toolbar_mode: ToolbarMode,
    toolbar_tool: ToolbarTool,
    timeline_view_data: TimelineViewData,
    intents: Vec<WorkflowIntent>,
    _subscriptions: Vec<Subscription>,
}

impl WorkflowHost {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let pages = cx.new(|cx| PagesPanel::new("flow-pages", Vec::new(), window, cx));
        let layers = cx.new(|cx| LayersPanel::new("flow-layers", Vec::new(), window, cx));
        let assets =
            cx.new(|cx| AssetsPanel::new("flow-assets", AssetsViewData::default(), window, cx));
        let design = cx.new(|cx| {
            DesignPanel::new(
                "flow-design",
                DesignPanelNode::new("frame", "Frame", DesignPanelNodeKind::Frame),
                window,
                cx,
            )
        });
        let prototype =
            cx.new(|cx| PrototypePanel::new("flow-prototype", PrototypeViewData::default(), cx));
        let timeline = cx.new(|cx| Timeline::new("flow-timeline", TimelineViewData::default(), cx));
        let toolbar = cx.new(|cx| {
            EditorToolbar::new(
                "flow-toolbar",
                ToolbarMode::Design,
                ToolbarTool::Move,
                100,
                window,
                cx,
            )
        });
        let variables = cx.new(|cx| {
            VariablesPage::new(
                "flow-variables",
                VariablesViewData {
                    document_name: "Workflow test".into(),
                    collections: Vec::new(),
                    selected_collection_id: "".into(),
                    groups: Vec::new(),
                    selected_group_id: "".into(),
                    modes: Vec::new(),
                    variables: Vec::new(),
                },
                window,
                cx,
            )
        });
        let editor = cx.new(|cx| {
            PseudoEditor::new(
                "flow-editor",
                PseudoEditorChildren {
                    pages,
                    layers,
                    assets,
                    design: design.clone(),
                    prototype,
                    timeline: timeline.clone(),
                    toolbar: toolbar.clone(),
                    variables,
                },
                cx,
            )
        });
        editor.update(cx, |editor, cx| {
            editor.set_right_surface(PseudoEditorRightSurface::Design, cx);
        });

        let toolbar_subscription =
            cx.subscribe(&toolbar, |host, toolbar, action: &ToolbarAction, cx| {
                host.intents.push(WorkflowIntent::Toolbar(action.clone()));
                if let ToolbarAction::ModeChangeRequested { mode } = action {
                    host.toolbar_mode = *mode;
                    host.toolbar_tool = match mode {
                        ToolbarMode::Design => ToolbarTool::Move,
                        ToolbarMode::Motion => ToolbarTool::Move,
                        ToolbarMode::Dev => ToolbarTool::Inspect,
                    };
                    toolbar.update(cx, |toolbar, cx| {
                        toolbar.set_mode(*mode, cx);
                        toolbar.set_active_tool(host.toolbar_tool, cx);
                    });
                    // The inspector projection is host policy: this host
                    // keeps the Design workspace across every toolbar mode.
                    host.design.update(cx, |design, cx| {
                        design.set_workspace_mode(DesignPanelWorkspaceMode::Design, cx);
                    });
                }
            });
        let timeline_subscription =
            cx.subscribe(&timeline, |host, timeline, action: &TimelineAction, cx| {
                host.intents.push(WorkflowIntent::Timeline(action.clone()));
                if let TimelineAction::PlayStateChangeRequested { playing } = action {
                    host.timeline_view_data.playing = *playing;
                    timeline.update(cx, |timeline, cx| {
                        timeline.set_view_data(host.timeline_view_data.clone(), cx);
                    });
                }
            });

        Self {
            editor,
            toolbar,
            design,
            toolbar_mode: ToolbarMode::Design,
            toolbar_tool: ToolbarTool::Move,
            timeline_view_data: TimelineViewData::default(),
            intents: Vec::new(),
            _subscriptions: vec![toolbar_subscription, timeline_subscription],
        }
    }
}

impl Render for WorkflowHost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().w(px(1440.)).h(px(900.)).child(self.editor.clone())
    }
}

fn setup(cx: &mut TestAppContext) -> (Entity<WorkflowHost>, &mut VisualTestContext) {
    cx.update(|cx| {
        gpui_component::init(cx);
        crate::init(cx);
    });
    let host_slot = std::rc::Rc::new(std::cell::RefCell::new(None));
    let captured_host = host_slot.clone();
    let (_, cx) = cx.add_window_view(move |window, cx| {
        let host = cx.new(|cx| WorkflowHost::new(window, cx));
        *captured_host.borrow_mut() = Some(host.clone());
        Root::new(host, window, cx)
    });
    cx.simulate_resize(size(px(1440.), px(900.)));
    let host = host_slot
        .borrow_mut()
        .take()
        .expect("workflow host should be installed");
    (host, cx)
}

#[gpui::test]
fn composed_host_routes_intents_and_echoes_state_across_components(cx: &mut TestAppContext) {
    let (host, cx) = setup(cx);

    assert_eq!(
        cx.read(|app| host.read(app).toolbar.read(app).mode()),
        ToolbarMode::Design,
    );
    assert_eq!(
        cx.read(|app| host.read(app).design.read(app).workspace_mode()),
        DesignPanelWorkspaceMode::Design,
    );
    assert!(
        cx.debug_bounds("flow-design-draw-workspace-heading")
            .is_none(),
        "the host's initial Design projection should render",
    );

    let motion = cx
        .debug_bounds("toolbar-mode-motion")
        .expect("the composed toolbar should expose Motion")
        .center();
    cx.simulate_click(motion, Modifiers::none());
    cx.run_until_parked();

    assert_eq!(
        cx.read(|app| {
            let host = host.read(app);
            (host.toolbar_mode, host.toolbar_tool)
        }),
        (ToolbarMode::Motion, ToolbarTool::Move),
    );
    assert_eq!(
        cx.read(|app| {
            let host = host.read(app);
            (
                host.toolbar.read(app).mode(),
                host.toolbar.read(app).active_tool(),
                host.design.read(app).workspace_mode(),
            )
        }),
        (
            ToolbarMode::Motion,
            ToolbarTool::Move,
            DesignPanelWorkspaceMode::Design,
        ),
    );
    assert!(
        cx.debug_bounds("toolbar-secondary-motion-play").is_some(),
        "the host echo should mount the Motion transport row in the toolbar",
    );
    assert!(
        cx.debug_bounds("flow-design-draw-workspace-heading")
            .is_none(),
        "the host keeps its Design projection across toolbar modes",
    );

    let play = cx
        .debug_bounds("timeline-play")
        .expect("the composed timeline should expose Play")
        .center();
    cx.simulate_click(play, Modifiers::none());
    cx.run_until_parked();
    assert!(cx.read(|app| host.read(app).timeline_view_data.playing));

    let pause = cx
        .debug_bounds("timeline-play")
        .expect("the host-echoed timeline should expose Pause")
        .center();
    cx.simulate_click(pause, Modifiers::none());
    cx.run_until_parked();
    assert!(
        !cx.read(|app| host.read(app).timeline_view_data.playing),
        "the second request can be false only after the first host echo",
    );

    let design = cx
        .debug_bounds("toolbar-mode-design")
        .expect("the composed toolbar should expose Design")
        .center();
    cx.simulate_click(design, Modifiers::none());
    cx.run_until_parked();

    assert_eq!(
        cx.read(|app| {
            let host = host.read(app);
            (
                host.toolbar_mode,
                host.toolbar_tool,
                host.design.read(app).workspace_mode(),
                host.intents.clone(),
            )
        }),
        (
            ToolbarMode::Design,
            ToolbarTool::Move,
            DesignPanelWorkspaceMode::Design,
            vec![
                WorkflowIntent::Toolbar(ToolbarAction::ModeChangeRequested {
                    mode: ToolbarMode::Motion,
                }),
                WorkflowIntent::Timeline(TimelineAction::PlayStateChangeRequested {
                    playing: true,
                }),
                WorkflowIntent::Timeline(TimelineAction::PlayStateChangeRequested {
                    playing: false,
                }),
                WorkflowIntent::Toolbar(ToolbarAction::ModeChangeRequested {
                    mode: ToolbarMode::Design,
                }),
            ],
        ),
    );
    assert!(
        cx.debug_bounds("flow-design-tab-design-active").is_some(),
        "the final host echo should restore the rendered Design projection",
    );
}
