use std::ops::Range;

use gpui::{ClickEvent, Hsla, InteractiveElement as _, MouseButton, MouseDownEvent, uniform_list};

use crate::toolbar::{ToolbarTool, render_tool_icon};

use super::*;

const LAYER_KIND_ICON_SIZE: f32 = 12.;

struct LayerDragPreview {
    drag: LayerDrag,
}

impl Render for LayerDragPreview {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .h(px(LAYER_ROW_HEIGHT))
            .w(px(220.))
            .px_2()
            .gap_2()
            .rounded(px(6.))
            .border_1()
            .border_color(cx.theme().selection)
            .bg(cx.theme().popover.opacity(0.96))
            .text_color(cx.theme().popover_foreground)
            .shadow_lg()
            .child(
                div()
                    .size(px(LAYER_ICON_SLOT))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(layer_kind_icon(
                        self.drag.kind,
                        layer_kind_color(self.drag.kind, cx),
                        LAYER_KIND_ICON_SIZE,
                    )),
            )
            .child(
                div()
                    .flex_1()
                    .truncate()
                    .text_xs()
                    .child(self.drag.title.clone()),
            )
    }
}

/// Group-highlight context computed once per rendered range: the arena
/// indices of the hovered group and of every selected container.
struct RowHighlights {
    hovered_group: Option<usize>,
    selected_groups: Vec<usize>,
}

impl LayersPanel {
    pub(super) fn render_tree(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let row_count = self.visible_rows.len();
        let list = uniform_list(
            SharedString::from(format!("{}-tree-list", self.id)),
            row_count,
            cx.processor(|this, range: Range<usize>, _window, cx| this.render_rows(range, cx)),
        )
        .size_full()
        .flex_shrink_1()
        .track_scroll(&self.list_scroll_handle);

        v_flex()
            .id(SharedString::from(format!("{}-tree", self.id)))
            .debug_selector(|| "layers-tree-viewport".to_owned())
            .relative()
            .flex_1()
            .min_h(px(0.))
            .w_full()
            .child(list)
            .child(
                div()
                    .absolute()
                    .top_0()
                    .right_0()
                    .bottom_0()
                    .left_0()
                    .child(
                        Scrollbar::vertical(&self.list_scroll_handle)
                            .id(SharedString::from(format!("{}-scrollbar", self.id))),
                    ),
            )
            .into_any_element()
    }

    /// Builds the rows for one visible range of the virtualized list.
    fn render_rows(&mut self, range: Range<usize>, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let highlights = RowHighlights {
            hovered_group: self.hovered_group_index(),
            selected_groups: self.selected_group_indices(),
        };
        let editing = self.editing.clone();
        let mut rows = Vec::with_capacity(range.len());
        for row in range {
            let Some(arena_index) = self.visible_rows.get(row).copied() else {
                continue;
            };
            let Some(node) = self.arena.get(arena_index).cloned() else {
                continue;
            };
            let is_editing = editing
                .as_ref()
                .is_some_and(|target| target.node_id == node.id);
            if is_editing {
                rows.push(self.render_layer_editor(row, &node));
            } else {
                let within_selected_group =
                    self.layer_is_within_groups(arena_index, &highlights.selected_groups);
                let within_hovered_group = highlights
                    .hovered_group
                    .is_some_and(|group| self.arena.is_within(arena_index, group));
                rows.push(self.render_layer_row(
                    row,
                    arena_index,
                    node,
                    within_selected_group,
                    within_hovered_group,
                    cx,
                ));
            }
        }
        rows
    }

    /// The arena index of the container a hover highlights: the hovered row
    /// itself when it has children, otherwise its parent.
    pub(super) fn hovered_group_index(&self) -> Option<usize> {
        let hovered = self.arena.index_of(self.hovered_node.as_ref()?)?;
        if self.arena.has_children(hovered) {
            Some(hovered)
        } else {
            self.arena.get(hovered)?.parent
        }
    }

    /// The id of the container a hover highlights (see `hovered_group_index`).
    #[cfg(test)]
    pub(super) fn hovered_group_id(&self) -> Option<SharedString> {
        self.hovered_group_index()
            .and_then(|index| self.arena.get(index))
            .map(|node| node.id.clone())
    }

    /// Arena indices of the selected rows that have children.
    pub(super) fn selected_group_indices(&self) -> Vec<usize> {
        self.selected_node_ids
            .iter()
            .filter_map(|id| self.arena.index_of(id))
            .filter(|index| self.arena.has_children(*index))
            .collect()
    }

    pub(super) fn layer_is_within_groups(&self, index: usize, groups: &[usize]) -> bool {
        groups
            .iter()
            .any(|group| self.arena.is_within(index, *group))
    }

    /// Ids of the shown rows that sit in (or are) a selected container — the
    /// rows drawn with the selected-group tint.
    #[cfg(test)]
    pub(super) fn rows_within_selected_groups(&self) -> Vec<SharedString> {
        let groups = self.selected_group_indices();
        self.visible_rows
            .iter()
            .copied()
            .filter(|index| self.layer_is_within_groups(*index, &groups))
            .filter_map(|index| self.arena.get(index))
            .map(|node| node.id.clone())
            .collect()
    }

    /// Whether dropping `dragged` at `pointer` over `target` is allowed, and
    /// where: never onto itself or a descendant, `Inside` only into kinds that
    /// accept children, and only where the host validator (if any) agrees.
    pub(super) fn drop_placement(
        &self,
        dragged: &SharedString,
        target: &SharedString,
        pointer: Point<Pixels>,
        cx: &App,
    ) -> Option<LayersPanelDropPosition> {
        let dragged_index = self.arena.index_of(dragged)?;
        let target_index = self.arena.index_of(target)?;
        if self.arena.is_within(target_index, dragged_index) {
            return None;
        }
        let target_kind = self.arena.get(target_index)?.kind;
        let bounds = self.node_row_bounds.get(target).copied()?;
        let position = events::layer_drop_position(target_kind, bounds, pointer);
        if let Some(validator) = &self.drop_validator
            && !validator(dragged, target, position, cx)
        {
            return None;
        }
        Some(position)
    }

    /// Left padding for a row at `depth`: a compact per-level step, capped so
    /// the row keeps room for its icon slots, title, and satellites inside the
    /// panel's current width.
    fn row_indent(&self, depth: usize) -> Pixels {
        let indent = depth as f32 * LAYER_INDENT;
        let max_indent = self
            .panel_bounds
            .map(|bounds| f32::from(bounds.size.width) - LAYER_ROW_MIN_CONTENT_WIDTH)
            .filter(|max| *max > 0.)
            .unwrap_or(f32::MAX);
        px(indent.min(max_indent))
    }

    fn render_layer_editor(&self, row: usize, node: &LayerNode) -> AnyElement {
        h_flex()
            .id(SharedString::from(format!("{}-editor-{row}", self.id)))
            .debug_selector(|| "layers-row-editor".to_owned())
            .h(px(LAYER_ROW_HEIGHT))
            .w_full()
            .flex_none()
            .pl(self.row_indent(node.depth) + px(LAYER_ICON_SLOT * 2.))
            .pr_1()
            .child(
                div()
                    .key_context(LAYERS_TEXT_ENTRY_KEY_CONTEXT)
                    .h_full()
                    .flex_1()
                    .min_w(px(0.))
                    .child(Input::new(&self.rename_input).xsmall()),
            )
            .into_any_element()
    }

    fn render_layer_row(
        &mut self,
        row: usize,
        arena_index: usize,
        node: LayerNode,
        within_selected_group: bool,
        within_hovered_group: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let has_children = self.arena.has_children(arena_index);
        let selected = self.selected_node_ids.contains(&node.id);
        let expanded = self.expanded_node_ids.contains(&node.id);
        let group_name: SharedString = format!("{}-row-hover-{row}", self.id).into();
        let row_selector = format!("layers-row-{}", node.id);
        let row_node = node.clone();
        let keyboard_node = node.clone();
        let menu_keyboard_node = node.clone();
        let menu_node = node.clone();
        let visibility_node = node.clone();
        let lock_node = node.clone();
        let expansion_id = node.id.clone();
        let bounds_id = node.id.clone();
        let drop_target_id = node.id.clone();
        let can_drop_target_id = node.id.clone();
        let panel = cx.entity();
        let drag = LayerDrag {
            node_id: node.id.clone(),
            title: node.title.clone(),
            kind: node.kind,
        };
        let drop_background = cx.theme().selection.opacity(0.22);
        let drop_border = cx.theme().selection;
        let row_focus_handle = self
            .row_focus_handles
            .entry(node.id.clone())
            .or_insert_with(|| cx.focus_handle())
            .clone()
            .tab_index(0)
            .tab_stop(true);

        let row_element = list_row(
            SharedString::from(format!("{}-row-{row}", self.id)),
            px(LAYER_ROW_HEIGHT),
            cx,
        )
        .debug_selector(move || row_selector)
        .group(group_name.clone())
        .track_focus(&row_focus_handle)
        .pl(self.row_indent(node.depth) + px(4.))
        .pr_1()
        .text_xs()
        .cursor_move()
        .hover(|style| style.bg(cx.theme().sidebar_accent.opacity(0.7)))
        .when(within_hovered_group, |row| {
            row.bg(cx.theme().sidebar_accent.opacity(0.32))
        })
        .when(within_selected_group, |row| {
            row.bg(cx.theme().selection.opacity(0.16))
                .text_color(cx.theme().sidebar_accent_foreground)
        })
        .when(selected, |row| {
            row.bg(cx.theme().selection.opacity(0.32))
                .text_color(cx.theme().sidebar_accent_foreground)
        })
        .when(!node.visible, |row| row.opacity(0.5));

        let hovered_id = node.id.clone();
        let row_element = row_element.on_hover(cx.listener(move |this, hovered, _, cx| {
            let changed = if *hovered {
                if this.hovered_node.as_ref() == Some(&hovered_id) {
                    false
                } else {
                    this.hovered_node = Some(hovered_id.clone());
                    true
                }
            } else if this.hovered_node.as_ref() == Some(&hovered_id) {
                this.hovered_node = None;
                true
            } else {
                false
            };
            if changed {
                cx.notify();
            }
        }));

        row_element
            .on_drag(drag, |drag, _, _, cx| {
                cx.new(|_| LayerDragPreview { drag: drag.clone() })
            })
            // Validity is decided from the arena and the host validator; gpui
            // applies the `drag_over` treatment only where this says yes, so
            // the highlight never promises a drop the host would refuse.
            .can_drop(move |drag, window, cx| {
                drag.downcast_ref::<LayerDrag>().is_some_and(|drag| {
                    panel
                        .read(cx)
                        .drop_placement(
                            &drag.node_id,
                            &can_drop_target_id,
                            window.mouse_position(),
                            cx,
                        )
                        .is_some()
                })
            })
            .drag_over::<LayerDrag>(move |style, _, _, _| {
                style.bg(drop_background).border_color(drop_border)
            })
            .on_drop(cx.listener(move |this, drag: &LayerDrag, window, cx| {
                cx.stop_propagation();
                this.request_move(
                    drag.node_id.clone(),
                    drop_target_id.clone(),
                    window.mouse_position(),
                    cx,
                );
            }))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    this.open_menu(menu_node.clone(), event.position, window, cx);
                }),
            )
            .on_action(cx.listener(move |this, _: &ActivateControl, _, cx| {
                this.request_selection(
                    keyboard_node.id.clone(),
                    LayersPanelSelectionMode::Replace,
                    cx,
                );
            }))
            .on_action(
                cx.listener(move |this, _: &OpenLayerContextMenu, window, cx| {
                    cx.stop_propagation();
                    this.open_menu_from_keyboard(menu_keyboard_node.clone(), window, cx);
                }),
            )
            .on_action(cx.listener(move |this, _: &ToggleLayerVisibility, _, cx| {
                cx.stop_propagation();
                this.request_visibility(&visibility_node, cx);
            }))
            .on_action(cx.listener(move |this, _: &ToggleLayerLock, _, cx| {
                cx.stop_propagation();
                this.request_lock(&lock_node, cx);
            }))
            // The pointer path carries modifier and double-click semantics that the
            // shared activation payload cannot, so the row keeps a split
            // pointer/keyboard pair converging on the same intent methods; the
            // keyboard-synthesized click is dropped to keep single activation.
            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                if event.is_keyboard() {
                    return;
                }
                if event.click_count() >= 2 {
                    this.begin_rename(row_node.id.clone(), row_node.title.clone(), window, cx);
                    return;
                }
                let modifiers = event.modifiers();
                let mode = if modifiers.shift {
                    LayersPanelSelectionMode::Range
                } else if modifiers.secondary() {
                    LayersPanelSelectionMode::Toggle
                } else {
                    LayersPanelSelectionMode::Replace
                };
                this.request_selection(row_node.id.clone(), mode, cx);
            }))
            .child(
                h_flex()
                    .w(px(LAYER_ICON_SLOT))
                    .h_full()
                    .flex_none()
                    .justify_center()
                    .when(has_children, |slot| {
                        slot.child(
                            h_flex()
                                .id(SharedString::from(format!("{}-expand-{row}", self.id)))
                                .debug_selector({
                                    let selector = format!("layers-expand-{}", expansion_id);
                                    move || selector.clone()
                                })
                                .size_full()
                                .justify_center()
                                .rounded(px(3.))
                                .hover(|style| style.bg(cx.theme().sidebar_accent))
                                .tooltip(move |window, cx| {
                                    Tooltip::new(if expanded { "Collapse" } else { "Expand" })
                                        .build(window, cx)
                                })
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(|_, _: &MouseDownEvent, _, cx| {
                                        cx.stop_propagation();
                                    }),
                                )
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    cx.stop_propagation();
                                    this.toggle_expansion(expansion_id.clone(), cx);
                                }))
                                .child(
                                    Icon::new(if expanded {
                                        IconName::ChevronDown
                                    } else {
                                        IconName::ChevronRight
                                    })
                                    .xsmall(),
                                ),
                        )
                    }),
            )
            .child(
                div()
                    .w(px(LAYER_ICON_SLOT))
                    .h_full()
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(layer_kind_icon(
                        node.kind,
                        layer_kind_color(node.kind, cx),
                        LAYER_KIND_ICON_SIZE,
                    )),
            )
            .child(truncating_label(node.title.clone()))
            .child(self.render_lock_control(row, &node, group_name.clone(), cx))
            .child(self.render_visibility_control(row, &node, group_name, cx))
            .child(track_bounds(cx.entity(), move |this, bounds| {
                this.node_row_bounds.insert(bounds_id.clone(), bounds);
            }))
            .into_any_element()
    }

    fn render_lock_control(
        &self,
        row: usize,
        node: &LayerNode,
        group_name: SharedString,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let locked = node.locked;
        let node = node.clone();
        h_flex()
            .id(SharedString::from(format!("{}-lock-{row}", self.id)))
            .debug_selector({
                let selector = format!("layers-lock-{}", node.id);
                move || selector.clone()
            })
            .size(px(22.))
            .flex_none()
            .justify_center()
            .rounded(px(3.))
            .cursor_pointer()
            .when(!locked, |control| {
                control
                    .invisible()
                    .group_hover(group_name, |control| control.visible())
            })
            .hover(|style| style.bg(cx.theme().sidebar_accent))
            .tooltip(move |window, cx| {
                Tooltip::new(if locked { "Unlock" } else { "Lock" })
                    .action(&ToggleLayerLock, Some(LAYERS_PANEL_KEY_CONTEXT))
                    .build(window, cx)
            })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|_, _: &MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                }),
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                cx.stop_propagation();
                this.request_lock(&node, cx);
            }))
            .child(render_control_icon(
                if locked {
                    ControlIcon::Lock
                } else {
                    ControlIcon::Unlock
                },
                if locked {
                    cx.theme().sidebar_foreground
                } else {
                    cx.theme().muted_foreground
                },
                LAYER_KIND_ICON_SIZE,
            ))
            .into_any_element()
    }

    fn render_visibility_control(
        &self,
        row: usize,
        node: &LayerNode,
        group_name: SharedString,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let visible = node.visible;
        let node = node.clone();
        h_flex()
            .id(SharedString::from(format!("{}-visibility-{row}", self.id)))
            .debug_selector({
                let selector = format!("layers-visibility-{}", node.id);
                move || selector.clone()
            })
            .size(px(22.))
            .flex_none()
            .justify_center()
            .rounded(px(3.))
            .cursor_pointer()
            .when(visible, |control| {
                control
                    .invisible()
                    .group_hover(group_name, |control| control.visible())
            })
            .hover(|style| style.bg(cx.theme().sidebar_accent))
            .tooltip(move |window, cx| {
                Tooltip::new(if visible { "Hide" } else { "Show" })
                    .action(&ToggleLayerVisibility, Some(LAYERS_PANEL_KEY_CONTEXT))
                    .build(window, cx)
            })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|_, _: &MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                }),
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                cx.stop_propagation();
                this.request_visibility(&node, cx);
            }))
            .child(render_control_icon(
                if visible {
                    ControlIcon::Eye
                } else {
                    ControlIcon::EyeClosed
                },
                cx.theme().sidebar_foreground,
                LAYER_KIND_ICON_SIZE,
            ))
            .into_any_element()
    }
}

fn layer_kind_color(kind: LayersPanelNodeKind, cx: &App) -> Hsla {
    if matches!(
        kind,
        LayersPanelNodeKind::Component
            | LayersPanelNodeKind::ComponentSet
            | LayersPanelNodeKind::Instance
    ) {
        cx.theme().selection
    } else {
        cx.theme().muted_foreground
    }
}

fn layer_kind_icon(kind: LayersPanelNodeKind, color: Hsla, size: f32) -> AnyElement {
    use LayersPanelNodeKind as Kind;

    let control_icon = match kind {
        Kind::Frame => Some(ControlIcon::Frame),
        Kind::Group => Some(ControlIcon::Group),
        Kind::Section => Some(ControlIcon::Section),
        // A component set is drawn with the single-component diamond until a
        // dedicated icon exists.
        Kind::Component | Kind::ComponentSet => Some(ControlIcon::Component),
        Kind::Instance => Some(ControlIcon::Instance),
        Kind::Text => Some(ControlIcon::Text),
        Kind::Image => Some(ControlIcon::Image),
        Kind::Video => Some(ControlIcon::Video),
        Kind::Mask => Some(ControlIcon::Mask),
        Kind::BooleanOperation => Some(ControlIcon::BooleanOperation),
        _ => None,
    };
    if let Some(icon) = control_icon {
        return render_control_icon(icon, color, size);
    }

    let tool = match kind {
        Kind::Rectangle => Some(ToolbarTool::Rectangle),
        Kind::Ellipse => Some(ToolbarTool::Ellipse),
        Kind::Polygon => Some(ToolbarTool::Polygon),
        Kind::Star => Some(ToolbarTool::Star),
        Kind::Line => Some(ToolbarTool::Line),
        Kind::Arrow => Some(ToolbarTool::Arrow),
        Kind::Vector => Some(ToolbarTool::NodeEdit),
        Kind::Slice => Some(ToolbarTool::Slice),
        Kind::Pen => Some(ToolbarTool::Pen),
        Kind::Pencil => Some(ToolbarTool::Pencil),
        _ => None,
    };
    if let Some(tool) = tool {
        return render_tool_icon(tool, color, size);
    }

    div()
        .text_color(color)
        .child(Icon::new(IconName::Ellipsis).xsmall())
        .into_any_element()
}
