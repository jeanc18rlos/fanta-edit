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
    AnyElement, Context, Entity, MouseDownEvent, Pixels, Point, Subscription, Window, anchored,
    deferred,
};
use ui::{ContextMenu, prelude::*};
use util::ResultExt as _;

use crate::canvas::{bounds_size, screen_position_in_bounds};
use crate::document::DocChange;
use crate::editor_session::EditorMode;
use crate::view::{
    CopySelection, CutSelection, DeleteSelection, DuplicateSelection, FigView, FrameSelection,
    GroupSelection, PasteSelection, UngroupSelection, show_canvas_notice,
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
    /// Any other layer.
    Layer(NodeId),
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
            _ => Self::Layer(node),
        }
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
        self.focus_handle.focus(window, cx);
        let target = self.select_context_menu_target(event.position, cx);
        let Some(doc_target) = self
            .item
            .read(cx)
            .document()
            .map(|document| MenuTarget::of(&document.doc, target))
        else {
            return;
        };
        let editable = self.is_editable(cx);
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
            if doc_target == MenuTarget::Canvas {
                return menu;
            }
            menu = menu
                .action_disabled_when(!editable, "Duplicate", Box::new(DuplicateSelection))
                .action_disabled_when(!editable, "Delete", Box::new(DeleteSelection))
                .separator();
            match doc_target {
                MenuTarget::MainComponent(node) => {
                    let view = view.clone();
                    menu = menu.entry("Create instance", None, move |window, cx| {
                        view.update(cx, |view, cx| view.create_instance_of(node, window, cx))
                            .log_err();
                    });
                }
                MenuTarget::Instance(node) => {
                    let (go, detach, reset) = (view.clone(), view.clone(), view.clone());
                    menu = menu
                        .entry("Go to main component", None, move |window, cx| {
                            go.update(cx, |view, cx| view.go_to_main_component(node, window, cx))
                                .log_err();
                        })
                        .entry("Detach instance", None, move |window, cx| {
                            detach
                                .update(cx, |view, cx| {
                                    view.run_layer_action(
                                        "Detach instance",
                                        node,
                                        LayerAction::DetachInstance,
                                        window,
                                        cx,
                                    )
                                })
                                .log_err();
                        })
                        .entry("Reset all overrides", None, move |window, cx| {
                            reset
                                .update(cx, |view, cx| {
                                    view.run_layer_action(
                                        "Reset all overrides",
                                        node,
                                        LayerAction::ResetInstance,
                                        window,
                                        cx,
                                    )
                                })
                                .log_err();
                        });
                }
                MenuTarget::Container(node) => {
                    let view = view.clone();
                    menu = menu.entry("Create component", None, move |window, cx| {
                        view.update(cx, |view, cx| {
                            view.run_layer_action(
                                "Create component",
                                node,
                                LayerAction::CreateComponent,
                                window,
                                cx,
                            )
                        })
                        .log_err();
                    });
                }
                MenuTarget::Layer(_) | MenuTarget::Canvas => {}
            }
            let (front, back) = (view.clone(), view.clone());
            let node = match doc_target {
                MenuTarget::MainComponent(node)
                | MenuTarget::Instance(node)
                | MenuTarget::Container(node)
                | MenuTarget::Layer(node) => node,
                MenuTarget::Canvas => return menu,
            };
            menu.separator()
                .action_disabled_when(!editable, "Group selection", Box::new(GroupSelection))
                .action_disabled_when(!editable, "Frame selection", Box::new(FrameSelection))
                .action_disabled_when(!editable, "Ungroup", Box::new(UngroupSelection))
                .separator()
                .entry("Bring to front", None, move |window, cx| {
                    front
                        .update(cx, |view, cx| {
                            view.run_layer_action(
                                "Bring to front",
                                node,
                                LayerAction::BringToFront,
                                window,
                                cx,
                            )
                        })
                        .log_err();
                })
                .entry("Send to back", None, move |window, cx| {
                    back.update(cx, |view, cx| {
                        view.run_layer_action(
                            "Send to back",
                            node,
                            LayerAction::SendToBack,
                            window,
                            cx,
                        )
                    })
                    .log_err();
                })
        });
        let dismiss = cx.subscribe_in(
            &menu,
            window,
            |view, _, _: &gpui::DismissEvent, window, cx| {
                view.canvas_context_menu = None;
                view.focus_handle.focus(window, cx);
                cx.notify();
            },
        );
        self.canvas_context_menu = Some(CanvasContextMenu {
            menu,
            position: event.position,
            _dismiss: dismiss,
        });
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
        let world = fanta_canvas::screen_to_world(screen, &viewport, DVec2::new(width, height));
        let item = self.item.clone();
        item.update(cx, |item, cx| {
            item.with_document(cx, |document| {
                let doc = &mut document.doc;
                let page = doc.active_page();
                let leaf = fanta_canvas::hit_test_deep(&doc.scene, world, HitPrecision::Path, page)
                    .into_iter()
                    .next();
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
                let operations = crate::layer_context_ops::simple(doc, node, action)
                    .with_context(|| format!("{label} is not available here"))?;
                anyhow::ensure!(!operations.is_empty(), "{label} is not available here");
                let created = crate::layer_context_ops::created_roots(&operations);
                Ok((
                    operations,
                    if created.is_empty() {
                        vec![node]
                    } else {
                        created
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
}
