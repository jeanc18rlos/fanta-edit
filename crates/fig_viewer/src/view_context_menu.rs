//! The canvas context menu (right-click, or a two-finger click on a
//! trackpad): the layer under the pointer is selected the way a click would
//! select it, and the menu offers what applies to it: clipboard and
//! arrangement, and for components, creating an instance, going to the main
//! component, detaching or resetting an instance.

use anyhow::Context as _;
use fanta_canvas::HitPrecision;
use fanta_doc::{NodeData, NodeId};
use fanta_gpui::layers::LayersPanelContextAction as LayerAction;
use glam::DVec2;
use gpui::{
    AnyElement, Context, Entity, Focusable as _, MouseDownEvent, Pixels, Point, Subscription,
    Window, anchored, deferred,
};
use ui::{ContextMenu, ContextMenuEntry, prelude::*};
use util::ResultExt as _;

use crate::canvas::{bounds_size, precise_hit_test_screen, screen_position_in_bounds};
use crate::document::DocChange;
use crate::editor_session::EditorMode;
use crate::view::{
    CopySelection, CutSelection, DeleteSelection, DuplicateSelection, FigView, FrameSelection,
    GroupSelection, PasteSelection, TextEditSeed, UngroupSelection, show_canvas_notice,
};

/// The open canvas context menu and where it opened.
pub(crate) struct CanvasContextMenu {
    menu: Entity<ContextMenu>,
    position: Point<Pixels>,
    _dismiss: Subscription,
}

/// What the layer a menu opened on is, for the entries it gets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MenuTarget {
    /// The canvas background: only paste applies.
    Canvas,
    /// A main component's master, or a variant set's frame.
    MainComponent(NodeId),
    /// A component instance.
    Instance(NodeId),
    /// A frame or group that is not a component yet.
    Container(NodeId),
    Text(NodeId),
    Vector(NodeId),
    Bitmap(NodeId),
    Video(NodeId),
    Audio(NodeId),
    Boolean(NodeId),
    NodeGraph(NodeId),
    Model3d(NodeId),
    AiArtifact(NodeId),
    Embed(NodeId),
}

impl MenuTarget {
    pub(crate) fn of(doc: &fanta_doc::Doc, node: Option<NodeId>) -> Self {
        let Some(node) = node else {
            return Self::Canvas;
        };
        if crate::component_actions::instantiable(doc, node).is_some() {
            return Self::MainComponent(node);
        }
        match doc.scene.get(node).map(|node| &node.data) {
            Some(NodeData::Instance(_)) => Self::Instance(node),
            Some(NodeData::Group(_)) => Self::Container(node),
            Some(NodeData::Text(_) | NodeData::TextPath(_)) => Self::Text(node),
            Some(NodeData::Vector(_)) => Self::Vector(node),
            Some(NodeData::Bitmap(_)) => Self::Bitmap(node),
            Some(NodeData::Video(_)) => Self::Video(node),
            Some(NodeData::Audio(_)) => Self::Audio(node),
            Some(NodeData::Boolean(_)) => Self::Boolean(node),
            Some(NodeData::NodeGraph(_)) => Self::NodeGraph(node),
            Some(NodeData::Model3d(_)) => Self::Model3d(node),
            Some(NodeData::AiArtifact(_)) => Self::AiArtifact(node),
            Some(NodeData::Embed(_)) => Self::Embed(node),
            None => Self::Canvas,
        }
    }

    fn node(self) -> Option<NodeId> {
        match self {
            Self::Canvas => None,
            Self::MainComponent(node)
            | Self::Instance(node)
            | Self::Container(node)
            | Self::Text(node)
            | Self::Vector(node)
            | Self::Bitmap(node)
            | Self::Video(node)
            | Self::Audio(node)
            | Self::Boolean(node)
            | Self::NodeGraph(node)
            | Self::Model3d(node)
            | Self::AiArtifact(node)
            | Self::Embed(node) => Some(node),
        }
    }

    fn editor_label(self) -> Option<&'static str> {
        match self {
            Self::Text(_) => Some("Edit text"),
            Self::Vector(_) => Some("Edit vector"),
            Self::Bitmap(_) => Some("Crop image"),
            Self::Video(_) => Some("Video properties"),
            Self::Audio(_) => Some("Audio properties"),
            Self::Boolean(_) => Some("Boolean properties"),
            Self::NodeGraph(_) => Some("Node graph properties"),
            Self::Model3d(_) => Some("3D properties"),
            Self::AiArtifact(_) => Some("AI artifact properties"),
            Self::Embed(_) => Some("Embed properties"),
            _ => None,
        }
    }

    fn edits_content(self) -> bool {
        matches!(self, Self::Text(_) | Self::Vector(_) | Self::Bitmap(_))
    }
}

fn canvas_layer_operations(
    doc: &fanta_doc::Doc,
    node: NodeId,
    action: LayerAction,
) -> anyhow::Result<Vec<fanta_doc::Operation>> {
    anyhow::ensure!(
        crate::layer_context_ops::editable(doc, node),
        "The layer is locked or is a page"
    );
    match action {
        LayerAction::BringToFront => crate::layer_context_ops::stack(doc, node, true),
        LayerAction::SendToBack => crate::layer_context_ops::stack(doc, node, false),
        LayerAction::DetachInstance => {
            anyhow::ensure!(
                matches!(
                    doc.scene.get(node).map(|node| &node.data),
                    Some(NodeData::Instance(_))
                ),
                "Choose a component instance"
            );
            let operations = crate::properties_ops::detach_instance_operations(doc, node);
            anyhow::ensure!(!operations.is_empty(), "The main component is unavailable");
            Ok(operations)
        }
        _ => crate::layer_context_ops::simple(doc, node, action),
    }
}

impl FigView {
    /// Right-click (two-finger click) on the canvas: select what was clicked
    /// and open the context menu at the pointer.
    pub(crate) fn deploy_canvas_context_menu(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.prototype_player.is_some() || self.editor_mode(cx) == EditorMode::Draw {
            return;
        }
        cx.stop_propagation();
        let deferred_event = event.clone();
        if self.defer_after_preserved_design_draft(window, cx, move |view, window, cx| {
            view.deploy_canvas_context_menu(&deferred_event, window, cx);
        }) {
            return;
        }
        self.finish_panel_edits(cx);
        self.commit_text_edit(cx);
        self.focus_handle.focus(window, cx);
        let target = self.select_context_menu_target(event.position, cx);
        let Some((doc_target, single, editable, actions)) =
            self.item.read(cx).document().map(|document| {
                let doc = &document.doc;
                (
                    MenuTarget::of(doc, target),
                    doc.selection.len() == 1,
                    self.is_editable(cx)
                        && doc
                            .selection
                            .iter()
                            .all(|node| crate::layer_context_ops::editable(doc, *node)),
                    target
                        .map(|node| crate::gpui_adapters::layers::context_actions(doc, node))
                        .unwrap_or_default(),
                )
            })
        else {
            return;
        };
        let view = cx.weak_entity();
        let focus = self.focus_handle.clone();
        let menu = ContextMenu::build(window, cx, move |menu, _, _| {
            let mut menu = menu.context(focus);
            if doc_target != MenuTarget::Canvas {
                menu = menu
                    .action("Copy", Box::new(CopySelection))
                    .action_disabled_when(!editable, "Cut", Box::new(CutSelection));
            }
            menu = menu.action_disabled_when(!editable, "Paste", Box::new(PasteSelection));
            let Some(node) = doc_target.node() else {
                return menu;
            };
            menu = menu
                .action_disabled_when(!editable, "Duplicate", Box::new(DuplicateSelection))
                .action_disabled_when(!editable, "Delete", Box::new(DeleteSelection));
            if single {
                if let Some(label) = doc_target.editor_label() {
                    let view = view.clone();
                    menu = menu.separator().item(
                        ContextMenuEntry::new(label)
                            .disabled(!editable && doc_target.edits_content())
                            .handler(move |window, cx| {
                                view.update(cx, |view, cx| {
                                    view.activate_context_target(doc_target, window, cx)
                                })
                                .log_err();
                            }),
                    );
                }
                if let MenuTarget::MainComponent(node) = doc_target {
                    let view = view.clone();
                    menu = menu.separator().item(
                        ContextMenuEntry::new("Create instance")
                            .disabled(!editable)
                            .handler(move |window, cx| {
                                view.update(cx, |view, cx| {
                                    view.create_instance_of(node, window, cx)
                                })
                                .log_err();
                            }),
                    );
                }
                if actions.contains(&LayerAction::GoToMainComponent) {
                    let view = view.clone();
                    menu =
                        menu.separator()
                            .entry("Go to main component", None, move |window, cx| {
                                view.update(cx, |view, cx| {
                                    view.go_to_main_component(node, window, cx)
                                })
                                .log_err();
                            });
                }
                for action in [
                    LayerAction::DetachInstance,
                    LayerAction::ResetInstance,
                    LayerAction::CreateComponent,
                    LayerAction::AddAutoLayout,
                    LayerAction::OutlineStroke,
                    LayerAction::Flatten,
                    LayerAction::UseAsMask,
                ] {
                    if actions.contains(&action) {
                        let view = view.clone();
                        menu = menu.item(
                            ContextMenuEntry::new(action.label())
                                .disabled(!editable)
                                .handler(move |window, cx| {
                                    view.update(cx, |view, cx| {
                                        view.run_layer_action(
                                            action.label(),
                                            node,
                                            action,
                                            window,
                                            cx,
                                        )
                                    })
                                    .log_err();
                                }),
                        );
                    }
                }
            }
            menu = menu
                .separator()
                .action_disabled_when(!editable, "Group selection", Box::new(GroupSelection))
                .action_disabled_when(!editable, "Frame selection", Box::new(FrameSelection));
            if actions.contains(&LayerAction::Ungroup)
                || actions.contains(&LayerAction::RemoveFrame)
            {
                menu = menu.action_disabled_when(!editable, "Ungroup", Box::new(UngroupSelection));
            }
            menu = menu.separator();
            for (label, action) in [
                ("Bring to front", LayerAction::BringToFront),
                ("Send to back", LayerAction::SendToBack),
            ] {
                let view = view.clone();
                menu = menu.item(ContextMenuEntry::new(label).disabled(!editable).handler(
                    move |window, cx| {
                        view.update(cx, |view, cx| {
                            view.run_layer_action(label, node, action, window, cx)
                        })
                        .log_err();
                    },
                ));
            }
            menu
        });
        let menu_focus = menu.focus_handle(cx);
        let dismiss = cx.subscribe_in(&menu, window, {
            let menu_focus = menu_focus.clone();
            move |view, _, _: &gpui::DismissEvent, window, cx| {
                let restore_focus = menu_focus.contains_focused(window, cx);
                view.canvas_context_menu = None;
                if restore_focus {
                    view.focus_handle.focus(window, cx);
                }
                cx.notify();
            }
        });
        self.canvas_context_menu = Some(CanvasContextMenu {
            menu,
            position: event.position,
            _dismiss: dismiss,
        });
        window.focus(&menu_focus, cx);
        cx.notify();
    }

    pub(crate) fn render_canvas_context_menu(&self) -> Option<AnyElement> {
        let open = self.canvas_context_menu.as_ref()?;
        Some(
            deferred(anchored().position(open.position).child(open.menu.clone()))
                .with_priority(3)
                .into_any_element(),
        )
    }

    /// Select what a right-click at `position` targets, the way a click
    /// would: a press inside the current selection keeps it; otherwise the
    /// top-level layer under the pointer becomes the selection, and the
    /// empty canvas clears it.
    fn select_context_menu_target(
        &mut self,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) -> Option<NodeId> {
        let bounds = self.container_bounds?;
        let viewport = self.viewport?;
        let (width, height) = bounds_size(bounds);
        let screen = screen_position_in_bounds(position, bounds);
        let item = self.item.clone();
        item.update(cx, |item, cx| {
            item.with_document(cx, |document| {
                let doc = &mut document.doc;
                let page = doc.active_page();
                let mut leaf = precise_hit_test_screen(
                    &doc.scene,
                    &viewport,
                    DVec2::new(width, height),
                    screen,
                    HitPrecision::Path,
                    page,
                );
                while let Some(container) = leaf
                    && doc
                        .scene
                        .get(container)
                        .is_some_and(|node| matches!(node.data, NodeData::Boolean(_)))
                    && !doc.selection.contains(container)
                    && doc.selection.iter().any(|selected| {
                        doc.scene
                            .ancestors_of(*selected)
                            .any(|node| node.id == container)
                    })
                {
                    let operand = precise_hit_test_screen(
                        &doc.scene,
                        &viewport,
                        DVec2::new(width, height),
                        screen,
                        HitPrecision::Path,
                        Some(container),
                    );
                    if operand == leaf {
                        break;
                    }
                    leaf = operand;
                }
                let Some(leaf) = leaf else {
                    let change = if doc.selection.iter().next().is_some() {
                        doc.selection.replace_with([]);
                        DocChange::Selection
                    } else {
                        DocChange::None
                    };
                    return (None, change);
                };
                let selected = std::iter::once(leaf)
                    .chain(doc.scene.ancestors_of(leaf).map(|ancestor| ancestor.id))
                    .find(|id| doc.selection.contains(*id));
                if let Some(selected) = selected {
                    return (Some(selected), DocChange::None);
                }
                let target = top_level_under(&doc.scene, leaf, page);
                doc.selection.select_only(target);
                (Some(target), DocChange::Selection)
            })
            .flatten()
        })
    }

    /// Place an instance of the main component (or variant set) `node` beside
    /// it and select the instance.
    pub(crate) fn create_instance_of(
        &mut self,
        node: NodeId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.apply_structure_edit(
            "Create instance",
            |doc| {
                let (operations, instance) =
                    crate::component_actions::create_instance_operations(doc, node)?;
                Ok((operations, vec![instance]))
            },
            window,
            cx,
        );
    }

    /// Select an instance's main component (for an instance of a variant
    /// set, the variant it shows), switching to its page and bringing it into
    /// view.
    pub(crate) fn go_to_main_component(
        &mut self,
        instance: NodeId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let selected_page = self.selected_page_index();
        let target = self.item.read(cx).document().and_then(|document| {
            let root = crate::component_actions::main_component_root(&document.doc, instance)?;
            Some((
                root,
                document.page_index_of_node(root),
                document.page_index(selected_page),
            ))
        });
        let Some((root, page, current_page)) = target else {
            show_canvas_notice(
                "This instance's main component is not in this document".to_owned(),
                window,
                cx,
            );
            return;
        };
        if let Some(page) = page
            && Some(page) != current_page
        {
            self.select_page(page, cx);
        }
        self.item.update(cx, |item, cx| {
            item.with_document(cx, |document| {
                document.doc.selection.select_only(root);
                ((), DocChange::Selection)
            });
        });
        self.focus_node(root, cx);
    }

    fn activate_context_target(
        &mut self,
        target: MenuTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.defer_after_preserved_design_draft(window, cx, move |view, window, cx| {
            view.activate_context_target(target, window, cx);
        }) {
            return;
        }
        if target.edits_content() && !self.is_editable(cx) {
            return;
        }
        let current = self.item.read(cx).doc().is_some_and(|doc| {
            doc.selection.len() == 1
                && target
                    .node()
                    .is_some_and(|node| doc.selection.contains(node))
                && MenuTarget::of(doc, target.node()) == target
                && (!target.edits_content()
                    || target
                        .node()
                        .is_some_and(|node| crate::layer_context_ops::editable(doc, node)))
        });
        if !current {
            return;
        }
        self.finish_panel_edits(cx);
        self.focus_handle.focus(window, cx);
        match target {
            MenuTarget::Text(node) => {
                self.open_text_edit(node, TextEditSeed::SelectAll, window, cx)
            }
            MenuTarget::Vector(_) => self.activate_tool(crate::tools::ToolKind::NodeEdit, cx),
            MenuTarget::Bitmap(_) => self.activate_tool(crate::tools::ToolKind::Crop, cx),
            _ => self.reveal_node_properties(cx),
        }
    }

    fn run_layer_action(
        &mut self,
        label: &str,
        node: NodeId,
        action: LayerAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.apply_structure_edit(
            label,
            |doc| {
                anyhow::ensure!(
                    doc.selection.contains(node),
                    "The context menu selection has changed"
                );
                anyhow::ensure!(
                    doc.active_page().is_none_or(|page| doc
                        .scene
                        .ancestors_of(node)
                        .any(|ancestor| ancestor.id == page)),
                    "The context menu page has changed"
                );
                let operations = canvas_layer_operations(doc, node, action)
                    .with_context(|| format!("{label} is not available here"))?;
                anyhow::ensure!(!operations.is_empty(), "{label} is not available here");
                let created = crate::layer_context_ops::created_roots(&operations);
                Ok((
                    operations,
                    if !created.is_empty() && action != LayerAction::DetachInstance {
                        created
                    } else if doc.selection.contains(node) {
                        doc.selection.iter().copied().collect()
                    } else {
                        vec![node]
                    },
                ))
            },
            window,
            cx,
        );
    }
}

/// The direct child of `container` (the page, by default) on the path down
/// to `leaf`: what a single click selects.
fn top_level_under(scene: &fanta_doc::Scene, leaf: NodeId, container: Option<NodeId>) -> NodeId {
    if scene.get(leaf).and_then(|node| node.parent) == container {
        return leaf;
    }
    scene
        .ancestors_of(leaf)
        .find(|ancestor| ancestor.parent == container)
        .map(|ancestor| ancestor.id)
        .unwrap_or(leaf)
}

#[cfg(test)]
mod tests {
    use super::*;
    use fanta_doc::{CanvasNode, ComponentDef, ComponentId, Doc, GroupNode, Operation};

    #[test]
    fn menu_targets_follow_what_the_layer_is() {
        let mut doc = Doc::new();
        let page = CanvasNode::new(NodeData::Group(GroupNode::default()));
        let page_id = page.id;
        doc.apply(Operation::create_node(page)).unwrap();
        doc.add_page(page_id);
        let mut frame = CanvasNode::new(NodeData::Group(GroupNode {
            clip_size: Some([10.0, 10.0]),
            ..Default::default()
        }));
        frame.parent = Some(page_id);
        let frame_id = frame.id;
        doc.apply(Operation::create_node(frame)).unwrap();
        assert_eq!(MenuTarget::of(&doc, None), MenuTarget::Canvas);
        assert_eq!(
            MenuTarget::of(&doc, Some(frame_id)),
            MenuTarget::Container(frame_id)
        );
        let component = ComponentId::new();
        doc.apply(Operation::DefineComponent {
            def: Box::new(ComponentDef::new(component, frame_id, "Card")),
        })
        .unwrap();
        assert_eq!(
            MenuTarget::of(&doc, Some(frame_id)),
            MenuTarget::MainComponent(frame_id)
        );
        let (operations, instance) =
            crate::component_actions::create_instance_operations(&doc, frame_id).unwrap();
        for operation in operations {
            doc.apply(operation).unwrap();
        }
        assert_eq!(
            MenuTarget::of(&doc, Some(instance)),
            MenuTarget::Instance(instance)
        );
    }

    #[test]
    fn a_click_selects_the_top_level_layer_under_the_pointer() {
        let mut doc = Doc::new();
        let page = CanvasNode::new(NodeData::Group(GroupNode::default()));
        let page_id = page.id;
        doc.apply(Operation::create_node(page)).unwrap();
        let mut frame = CanvasNode::new(NodeData::Group(GroupNode::default()));
        frame.parent = Some(page_id);
        let frame_id = frame.id;
        doc.apply(Operation::create_node(frame)).unwrap();
        let mut child = CanvasNode::new(NodeData::Group(GroupNode::default()));
        child.parent = Some(frame_id);
        let child_id = child.id;
        doc.apply(Operation::create_node(child)).unwrap();
        assert_eq!(
            top_level_under(&doc.scene, child_id, Some(page_id)),
            frame_id
        );
        assert_eq!(
            top_level_under(&doc.scene, frame_id, Some(page_id)),
            frame_id
        );
    }

    fn leaf_menu_cases() -> Vec<(NodeData, &'static str)> {
        use fanta_doc::{
            AiArtifactNode, AssetId, AudioNode, BitmapNode, EmbedNode, Model3dNode, NodeGraphNode,
            TextNode, VectorNode, VideoNode,
        };
        vec![
            (
                NodeData::Text(TextNode::new("Title", 100.0, 30.0)),
                "Edit text",
            ),
            (
                NodeData::Vector(VectorNode::rect_solid(
                    0.0,
                    0.0,
                    100.0,
                    100.0,
                    fanta_doc::Color::BLACK,
                )),
                "Edit vector",
            ),
            (
                NodeData::Bitmap(BitmapNode {
                    asset: AssetId::new(),
                    natural_size: [100, 100],
                    local_size: [100.0, 100.0],
                    crop: None,
                    fit: fanta_doc::ImageFitMode::Fill,
                    tint: None,
                }),
                "Crop image",
            ),
            (
                NodeData::Video(VideoNode {
                    asset: AssetId::new(),
                    natural_size: [100, 100],
                    local_size: [100.0, 100.0],
                    time_range_us: [0, 1_000_000],
                    speed: 1.0,
                    muted: false,
                    volume: 1.0,
                    poster_frame_us: None,
                    poster: None,
                    fit: fanta_doc::ImageFitMode::Fill,
                }),
                "Video properties",
            ),
            (
                NodeData::Audio(AudioNode {
                    asset: AssetId::new(),
                    local_size: [100.0, 100.0],
                    time_range_us: [0, 1_000_000],
                    volume: 1.0,
                    muted: false,
                    waveform_color: fanta_doc::Color::BLACK,
                }),
                "Audio properties",
            ),
            (
                NodeData::NodeGraph(NodeGraphNode {
                    local_size: [100.0, 100.0],
                    graph: Default::default(),
                    preview: None,
                }),
                "Node graph properties",
            ),
            (
                NodeData::Model3d(Model3dNode {
                    asset: AssetId::new(),
                    local_size: [100.0, 100.0],
                    camera: Default::default(),
                    overrides: serde_json::json!({"retained": true}),
                }),
                "3D properties",
            ),
            (
                NodeData::AiArtifact(AiArtifactNode {
                    local_size: [100.0, 100.0],
                    prompt: "Existing generation".into(),
                    model: "local.fixture".into(),
                    params: Default::default(),
                    inputs: Vec::new(),
                    lineage_parent: None,
                    output: None,
                    status: Default::default(),
                    seed: Some(42),
                }),
                "AI artifact properties",
            ),
            (
                NodeData::Embed(EmbedNode {
                    local_size: [100.0, 100.0],
                    kind: "local.fixture".into(),
                    payload: serde_json::json!({"retained": true}),
                }),
                "Embed properties",
            ),
        ]
    }

    #[test]
    fn leaf_menus_offer_the_editor_for_the_node_kind() {
        let mut doc = Doc::new();
        for (data, expected) in leaf_menu_cases() {
            let node = CanvasNode::new(data);
            let id = node.id;
            doc.apply(Operation::create_node(node))
                .expect("create leaf");
            let target = MenuTarget::of(&doc, Some(id));
            assert_eq!(target.editor_label(), Some(expected));
            assert_eq!(target.node(), Some(id));
            assert!(
                !crate::gpui_adapters::layers::context_actions(&doc, id)
                    .contains(&LayerAction::Ungroup)
            );
        }
        assert_eq!(
            MenuTarget::of(&doc, Some(NodeId::new())),
            MenuTarget::Canvas
        );
        let boolean = doc
            .scene
            .insert(CanvasNode::new(NodeData::Boolean(
                fanta_doc::BooleanNode::default(),
            )))
            .expect("boolean");
        assert_eq!(
            MenuTarget::of(&doc, Some(boolean)).editor_label(),
            Some("Boolean properties")
        );
    }

    #[gpui::test]
    async fn canvas_context_menu_escape_dismisses_without_clearing_selection(
        cx: &mut gpui::TestAppContext,
    ) {
        use fanta_doc::{BooleanNode, Color, VectorNode, Viewport};
        use gpui::{MouseButton, MouseUpEvent, px, size};
        use project::Project;

        cx.update(|cx| {
            zlog::init_test();
            assets::Assets.load_test_fonts(cx);
            let settings = settings::SettingsStore::test(cx);
            cx.set_global(settings);
            theme_settings::init(theme::LoadThemes::JustBase, cx);
            release_channel::init(semver::Version::new(0, 0, 0), cx);
            editor::init(cx);
            #[cfg(feature = "fanta-gpui-ui")]
            {
                gpui_component::init(cx);
                fanta_gpui::init(cx);
                crate::theme_bridge::init(cx);
            }
            cx.bind_keys([
                gpui::KeyBinding::new("escape", crate::view::Cancel, Some("FigViewer")),
                gpui::KeyBinding::new("escape", menu::Cancel, Some("menu")),
            ]);
        });
        let project = Project::test(fs::FakeFs::new(cx.executor()), [], cx).await;
        let mut doc = Doc::new();
        let page = CanvasNode::new(NodeData::Group(GroupNode::default()));
        let page_id = page.id;
        doc.apply(Operation::create_node(page)).expect("page");
        doc.add_page(page_id);
        doc.set_active_page(Some(page_id));
        let mut boolean = CanvasNode::new(NodeData::Boolean(BooleanNode {
            fills: [fanta_doc::Fill::solid(Color::BLACK)].into_iter().collect(),
            ..Default::default()
        }));
        boolean.parent = Some(page_id);
        let boolean_id = boolean.id;
        doc.apply(Operation::create_node(boolean)).expect("Boolean");
        let mut operand = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
            -50.,
            -50.,
            100.,
            100.,
            Color::BLACK,
        )));
        operand.parent = Some(boolean_id);
        doc.apply(Operation::create_node(operand)).expect("operand");
        doc.history = Default::default();
        let original_scene = serde_json::to_value(&doc.scene).expect("scene");
        let item = crate::document::ready_item_for_test(
            &project,
            "/tmp/CanvasMenuEscape.fig".into(),
            doc,
            cx,
        );
        let (view, visual) = cx.add_window_view({
            let item = item.clone();
            move |window, cx| FigView::new(item, project, window, cx)
        });
        visual.simulate_resize(size(px(1400.), px(900.)));
        view.update(visual, |view, cx| {
            view.set_viewport_silent(Viewport::default());
            cx.notify();
        });
        visual.run_until_parked();
        visual.update(|window, cx| window.draw(cx).clear());
        let position = view.read_with(visual, |view, _| {
            view.container_bounds.expect("canvas").center()
        });
        visual.simulate_event(MouseDownEvent {
            position,
            button: MouseButton::Right,
            modifiers: Default::default(),
            click_count: 1,
            first_mouse: false,
        });
        visual.simulate_event(MouseUpEvent {
            position,
            button: MouseButton::Right,
            modifiers: Default::default(),
            click_count: 1,
        });
        visual.run_until_parked();
        view.read_with(visual, |view, _| {
            assert!(view.canvas_context_menu.is_some())
        });
        item.read_with(visual, |item, _| {
            assert_eq!(
                item.doc().expect("document").selection.as_slice(),
                &[boolean_id]
            )
        });
        visual.simulate_keystrokes("escape");
        visual.run_until_parked();
        view.read_with(visual, |view, _| {
            assert!(
                view.canvas_context_menu.is_none(),
                "Escape dismisses the menu first"
            )
        });
        item.read_with(visual, |item, _| {
            let doc = item.doc().expect("document");
            assert_eq!(
                doc.selection.as_slice(),
                &[boolean_id],
                "menu dismissal must preserve canvas selection"
            );
            assert_eq!(
                serde_json::to_value(&doc.scene).expect("scene"),
                original_scene
            );
            assert_eq!(doc.history.undo_depth(), 0);
        });
        visual.simulate_keystrokes("escape");
        visual.run_until_parked();
        item.read_with(visual, |item, _| {
            assert!(
                item.doc().expect("document").selection.is_empty(),
                "after dismissal the canvas receives Escape again"
            )
        });
        visual.simulate_event(MouseDownEvent {
            position,
            button: MouseButton::Right,
            modifiers: Default::default(),
            click_count: 1,
            first_mouse: false,
        });
        visual.simulate_event(MouseUpEvent {
            position,
            button: MouseButton::Right,
            modifiers: Default::default(),
            click_count: 1,
        });
        visual.run_until_parked();
        view.read_with(visual, |view, _| {
            assert!(view.canvas_context_menu.is_some())
        });
        visual.update(|window, cx| window.draw(cx).clear());
        let next_focus = visual.update(|window, cx| {
            let focus = cx.focus_handle();
            window.focus(&focus, cx);
            let menu = view
                .read(cx)
                .canvas_context_menu
                .as_ref()
                .expect("menu")
                .menu
                .clone();
            menu.update(cx, |_, cx| cx.emit(gpui::DismissEvent));
            focus
        });
        visual.run_until_parked();
        view.read_with(visual, |view, _| {
            assert!(view.canvas_context_menu.is_none())
        });
        assert!(
            visual.update(|window, _| next_focus.is_focused(window)),
            "a dismissal after focus transfer must not steal focus from the next control"
        );
    }

    #[gpui::test]
    async fn canvas_menu_primary_entries_activate_the_matching_editor(
        cx: &mut gpui::TestAppContext,
    ) {
        use crate::tools::ToolKind;
        use fanta_doc::Viewport;
        use gpui::{MouseButton, point, px, size};
        use project::Project;
        cx.update(|cx| {
            let settings = settings::SettingsStore::test(cx);
            cx.set_global(settings);
        });
        let project = Project::test(fs::FakeFs::new(cx.executor()), [], cx).await;
        for (data, label) in leaf_menu_cases() {
            let expected_tool = match &data {
                NodeData::Vector(_) => ToolKind::NodeEdit,
                NodeData::Bitmap(_) => ToolKind::Crop,
                _ => ToolKind::Select,
            };
            let edits_text = matches!(data, NodeData::Text(_));
            let mut doc = Doc::new();
            let page = CanvasNode::new(NodeData::Group(GroupNode::default()));
            let page_id = page.id;
            doc.apply(Operation::create_node(page)).expect("page");
            doc.add_page(page_id);
            doc.set_active_page(Some(page_id));
            let mut node = CanvasNode::new(data);
            let node_id = node.id;
            node.parent = Some(page_id);
            doc.apply(Operation::create_node(node)).expect("leaf");
            doc.history = Default::default();
            let item =
                crate::document::ready_item_for_test(&project, "/tmp/LeafMenu.fig".into(), doc, cx);
            let original = item.read_with(cx, |item, _| {
                item.doc()
                    .expect("document")
                    .scene
                    .get(node_id)
                    .expect("leaf")
                    .clone()
            });
            let scratch = cx.add_window(|_, _| gpui::Empty);
            let view = scratch
                .update(cx, |_, window, cx| {
                    cx.new(|cx| FigView::new(item.clone(), project.clone(), window, cx))
                })
                .expect("view");
            scratch
                .update(cx, |_, window, cx| {
                    view.update(cx, |view, cx| {
                        view.set_container_bounds(gpui::Bounds {
                            origin: point(px(0.0), px(0.0)),
                            size: size(px(800.0), px(600.0)),
                        });
                        view.set_viewport_silent(Viewport::default());
                        view.deploy_canvas_context_menu(
                            &MouseDownEvent {
                                button: MouseButton::Right,
                                position: point(px(420.0), px(315.0)),
                                modifiers: Default::default(),
                                click_count: 1,
                                first_mouse: false,
                            },
                            window,
                            cx,
                        );
                    });
                    let menu = view
                        .read(cx)
                        .canvas_context_menu
                        .as_ref()
                        .expect("menu")
                        .menu
                        .clone();
                    menu.update(cx, |menu, cx| {
                        menu.select_first(&menu::SelectFirst, window, cx);
                        for _ in 0..5 {
                            menu.select_next(&menu::SelectNext, window, cx);
                        }
                        menu.confirm(&menu::Confirm, window, cx);
                    });
                })
                .expect("activate primary entry");
            view.read_with(cx, |view, _| {
                assert_eq!(view.active_tool(), expected_tool, "{label}");
                assert_eq!(view.text_edit.is_some(), edits_text, "{label}");
            });
            item.read_with(cx, |item, _| {
                let doc = item.doc().expect("document");
                assert_eq!(doc.selection.as_slice(), &[node_id], "{label}");
                assert_eq!(doc.scene.get(node_id), Some(&original), "{label}");
                assert_eq!(doc.scene.len(), 2, "{label}");
                assert_eq!(doc.history.undo_depth(), 0, "{label}");
                assert!(!item.is_dirty(), "{label}");
            });
        }
    }

    #[test]
    fn canvas_detach_preserves_instance_children_and_undo_restores_the_link() {
        let mut doc = Doc::new();
        let master = CanvasNode::new(NodeData::Group(GroupNode::default()));
        let master_id = master.id;
        doc.apply(Operation::create_node(master)).expect("master");
        let mut child = CanvasNode::new(NodeData::Vector(fanta_doc::VectorNode::rect_solid(
            0.0,
            0.0,
            30.0,
            20.0,
            fanta_doc::Color::BLACK,
        )));
        child.parent = Some(master_id);
        doc.apply(Operation::create_node(child))
            .expect("master child");
        doc.apply(Operation::DefineComponent {
            def: Box::new(ComponentDef::new(ComponentId::new(), master_id, "Card")),
        })
        .expect("component");
        let (operations, instance) =
            crate::component_actions::create_instance_operations(&doc, master_id)
                .expect("instance operations");
        for operation in operations {
            doc.apply(operation).expect("instance");
        }
        let before = doc
            .scene
            .get(instance)
            .expect("instance before")
            .data
            .clone();
        doc.selection.select_only(instance);
        doc.history = Default::default();
        let operations = canvas_layer_operations(&doc, instance, LayerAction::DetachInstance)
            .expect("detach operations");
        crate::clipboard::apply_transaction(&mut doc, "Detach instance", operations)
            .expect("detach");
        assert!(matches!(
            doc.scene.get(instance).expect("detached root").data,
            NodeData::Group(_)
        ));
        assert_eq!(doc.scene.children_of(Some(instance)).len(), 1);
        assert_eq!(doc.scene.children_of(Some(master_id)).len(), 1);
        assert_eq!(doc.history.undo_depth(), 1);
        assert!(doc.undo().expect("undo detach"));
        assert_eq!(
            doc.scene.get(instance).expect("restored instance").data,
            before
        );
        assert!(doc.scene.children_of(Some(instance)).is_empty());
        doc.scene
            .get_mut(instance)
            .expect("lock instance")
            .flags
            .insert(fanta_doc::NodeFlags::LOCKED);
        assert!(canvas_layer_operations(&doc, instance, LayerAction::DetachInstance).is_err());
    }

    #[gpui::test]
    async fn canvas_menu_keeps_selected_boolean_operand_and_opens_vector_editor(
        cx: &mut gpui::TestAppContext,
    ) {
        use fanta_doc::{Color, VectorNode, Viewport};
        use gpui::{MouseButton, point, px, size};
        use project::Project;
        cx.update(|cx| {
            let settings = settings::SettingsStore::test(cx);
            cx.set_global(settings);
        });
        let project = Project::test(fs::FakeFs::new(cx.executor()), [], cx).await;
        let mut doc = Doc::new();
        let page = CanvasNode::new(NodeData::Group(GroupNode::default()));
        let page_id = page.id;
        doc.apply(Operation::create_node(page)).expect("page");
        doc.add_page(page_id);
        doc.set_active_page(Some(page_id));
        let mut outer = CanvasNode::new(NodeData::Boolean(fanta_doc::BooleanNode::default()));
        let outer_id = outer.id;
        outer.parent = Some(page_id);
        doc.apply(Operation::create_node(outer))
            .expect("outer boolean");
        let mut inner = CanvasNode::new(NodeData::Boolean(fanta_doc::BooleanNode::default()));
        let inner_id = inner.id;
        inner.parent = Some(outer_id);
        doc.apply(Operation::create_node(inner))
            .expect("inner boolean");
        let mut operand = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
            0.0,
            0.0,
            100.0,
            100.0,
            Color::BLACK,
        )));
        let operand_id = operand.id;
        operand.parent = Some(inner_id);
        doc.apply(Operation::create_node(operand)).expect("operand");
        doc.history = Default::default();
        let item =
            crate::document::ready_item_for_test(&project, "/tmp/BooleanMenu.fig".into(), doc, cx);
        let scratch = cx.add_window(|_, _| gpui::Empty);
        let view = scratch
            .update(cx, |_, window, cx| {
                cx.new(|cx| FigView::new(item.clone(), project.clone(), window, cx))
            })
            .expect("view");
        view.update(cx, |view, cx| {
            view.set_container_bounds(gpui::Bounds {
                origin: point(px(0.0), px(0.0)),
                size: size(px(800.0), px(600.0)),
            });
            view.set_viewport_silent(Viewport::default());
            assert_eq!(
                view.select_context_menu_target(point(px(440.0), px(340.0)), cx),
                Some(outer_id)
            );
        });
        item.update(cx, |item, cx| {
            item.with_document(cx, |document| {
                document.doc.selection.select_only(operand_id);
                ((), DocChange::Selection)
            });
        });
        scratch
            .update(cx, |_, window, cx| {
                view.update(cx, |view, cx| {
                    view.deploy_canvas_context_menu(
                        &MouseDownEvent {
                            button: MouseButton::Right,
                            position: point(px(440.0), px(340.0)),
                            modifiers: Default::default(),
                            click_count: 1,
                            first_mouse: false,
                        },
                        window,
                        cx,
                    );
                });
                let menu = view
                    .read(cx)
                    .canvas_context_menu
                    .as_ref()
                    .expect("operand menu")
                    .menu
                    .clone();
                menu.update(cx, |menu, cx| {
                    menu.select_first(&menu::SelectFirst, window, cx);
                    for _ in 0..5 {
                        menu.select_next(&menu::SelectNext, window, cx);
                    }
                    menu.confirm(&menu::Confirm, window, cx);
                });
            })
            .expect("choose Edit vector from actual menu");
        view.read_with(cx, |view, _| {
            assert_eq!(view.active_tool(), crate::tools::ToolKind::NodeEdit)
        });
        item.read_with(cx, |item, _| {
            let doc = item.doc().expect("document");
            assert_eq!(doc.selection.as_slice(), &[operand_id]);
            assert_eq!(doc.scene.len(), 4);
            assert!(matches!(
                doc.scene.get(outer_id).expect("outer boolean").data,
                NodeData::Boolean(_)
            ));
            assert!(matches!(
                doc.scene.get(inner_id).expect("inner boolean").data,
                NodeData::Boolean(_)
            ));
            assert_eq!(doc.history.undo_depth(), 0);
            assert!(!item.is_dirty());
        });
    }

    #[gpui::test]
    async fn canvas_menu_reorder_entries_execute_and_undo(cx: &mut gpui::TestAppContext) {
        use fanta_doc::{Color, IndexKey, VectorNode, Viewport};
        use gpui::{MouseButton, point, px, size};
        use project::Project;
        cx.update(|cx| {
            let settings = settings::SettingsStore::test(cx);
            cx.set_global(settings);
        });
        let project = Project::test(fs::FakeFs::new(cx.executor()), [], cx).await;
        let mut doc = Doc::new();
        let page = CanvasNode::new(NodeData::Group(GroupNode::default()));
        let page_id = page.id;
        doc.apply(Operation::create_node(page)).expect("page");
        doc.add_page(page_id);
        doc.set_active_page(Some(page_id));
        let mut first = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
            0.0,
            0.0,
            100.0,
            100.0,
            Color::BLACK,
        )));
        first.parent = Some(page_id);
        first.index = IndexKey::FIRST;
        let first_id = first.id;
        doc.apply(Operation::create_node(first))
            .expect("first layer");
        let mut second = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
            40.0,
            0.0,
            100.0,
            100.0,
            Color::WHITE,
        )));
        second.parent = Some(page_id);
        second.index = IndexKey::after(IndexKey::FIRST);
        let second_id = second.id;
        doc.apply(Operation::create_node(second))
            .expect("second layer");
        doc.history = Default::default();
        let item =
            crate::document::ready_item_for_test(&project, "/tmp/CanvasMenu.fig".into(), doc, cx);
        let scratch = cx.add_window(|_, _| gpui::Empty);
        let view = scratch
            .update(cx, |_, window, cx| {
                cx.new(|cx| FigView::new(item.clone(), project.clone(), window, cx))
            })
            .expect("view");
        view.update(cx, |view, _| {
            view.set_container_bounds(gpui::Bounds {
                origin: point(px(0.0), px(0.0)),
                size: size(px(800.0), px(600.0)),
            });
            view.set_viewport_silent(Viewport::default());
        });
        for front in [false, true] {
            scratch
                .update(cx, |_, window, cx| {
                    view.update(cx, |view, cx| {
                        view.deploy_canvas_context_menu(
                            &MouseDownEvent {
                                button: MouseButton::Right,
                                position: point(px(530.0), px(320.0)),
                                modifiers: Default::default(),
                                click_count: 1,
                                first_mouse: false,
                            },
                            window,
                            cx,
                        );
                    });
                    let menu = view
                        .read(cx)
                        .canvas_context_menu
                        .as_ref()
                        .expect("open menu")
                        .menu
                        .clone();
                    menu.update(cx, |menu, cx| {
                        menu.select_last(window, cx);
                        if front {
                            menu.select_previous(&menu::SelectPrevious, window, cx);
                        }
                        menu.confirm(&menu::Confirm, window, cx);
                    });
                })
                .expect("activate reorder menu entry");
            item.read_with(cx, |item, _| {
                let doc = item.doc().expect("document");
                assert_eq!(doc.selection.as_slice(), &[second_id]);
                let expected = if front {
                    [first_id, second_id]
                } else {
                    [second_id, first_id]
                };
                assert_eq!(doc.scene.children_of(Some(page_id)), &expected);
                assert_eq!(doc.history.undo_depth(), if front { 2 } else { 1 });
            });
        }
        item.update(cx, |item, cx| {
            assert!(item.undo(cx).expect("undo bring to front"));
            assert_eq!(
                item.doc()
                    .expect("document")
                    .scene
                    .children_of(Some(page_id)),
                &[second_id, first_id]
            );
            assert!(item.undo(cx).expect("undo send to back"));
            assert_eq!(
                item.doc()
                    .expect("document")
                    .scene
                    .children_of(Some(page_id)),
                &[first_id, second_id]
            );
        });
    }
}
