use super::*;

impl LayersPanel {
    pub(super) fn toggle_panel_expanded(&mut self, cx: &mut Context<Self>) {
        self.panel_expanded = !self.panel_expanded;
        if !self.panel_expanded {
            self.menu = None;
            self.hovered_node = None;
        }
        cx.emit(LayersPanelAction::PanelExpansionChanged {
            expanded: self.panel_expanded,
        });
        cx.notify();
    }

    pub(super) fn on_close_overlay(
        &mut self,
        _: &CloseLayersOverlay,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(menu) = self.menu.take() {
            menu.return_focus
                .unwrap_or_else(|| self.focus_handle.clone())
                .focus(window, cx);
            cx.stop_propagation();
            cx.notify();
        } else if self.editing.take().is_some() {
            self.focus_panel_after_action(window, cx);
            cx.stop_propagation();
            cx.notify();
        }
    }

    pub(super) fn on_focus_previous_layer(
        &mut self,
        _: &FocusPreviousLayer,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_row_focus(-1, window, cx);
    }

    pub(super) fn on_focus_next_layer(
        &mut self,
        _: &FocusNextLayer,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_row_focus(1, window, cx);
    }

    pub(super) fn on_collapse_layer(
        &mut self,
        _: &CollapseLayer,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(row) = self.focused_visible_row(window) else {
            return;
        };
        let arena_index = self.visible_rows[row];
        let Some(node) = self.arena.get(arena_index).cloned() else {
            return;
        };
        if self.arena.has_children(arena_index) && self.expanded_node_ids.contains(&node.id) {
            cx.stop_propagation();
            self.toggle_expansion(node.id, cx);
        } else if let Some(parent_id) = node
            .parent
            .and_then(|parent| self.arena.get(parent))
            .map(|parent| parent.id.clone())
        {
            cx.stop_propagation();
            self.focus_row(&parent_id, window, cx);
        }
    }

    pub(super) fn on_expand_layer(
        &mut self,
        _: &ExpandLayer,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(row) = self.focused_visible_row(window) else {
            return;
        };
        let arena_index = self.visible_rows[row];
        if !self.arena.has_children(arena_index) {
            return;
        }
        let Some(node_id) = self.arena.get(arena_index).map(|node| node.id.clone()) else {
            return;
        };
        if self.expanded_node_ids.contains(&node_id) {
            // The first child of an expanded container is the next shown row.
            if let Some(first_child) = self
                .visible_rows
                .get(row + 1)
                .and_then(|index| self.arena.get(*index))
                .map(|child| child.id.clone())
            {
                cx.stop_propagation();
                self.focus_row(&first_child, window, cx);
            }
        } else {
            cx.stop_propagation();
            self.toggle_expansion(node_id, cx);
        }
    }

    fn move_row_focus(&mut self, offset: isize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(row) = self.focused_visible_row(window) else {
            return;
        };
        let target = row as isize + offset;
        if target < 0 || target as usize >= self.visible_rows.len() {
            return;
        }
        let Some(target_id) = self
            .arena
            .get(self.visible_rows[target as usize])
            .map(|node| node.id.clone())
        else {
            return;
        };
        cx.stop_propagation();
        self.focus_row(&target_id, window, cx);
    }

    /// Returns the position (in the shown rows) of the keyboard-focused row,
    /// or `None` while an overlay or the rename editor owns the keyboard.
    fn focused_visible_row(&self, window: &Window) -> Option<usize> {
        if self.menu.is_some() || self.editing.is_some() {
            return None;
        }
        self.visible_rows.iter().position(|index| {
            self.arena.get(*index).is_some_and(|node| {
                self.row_focus_handles
                    .get(&node.id)
                    .is_some_and(|handle| handle.is_focused(window))
            })
        })
    }

    /// Focuses a shown row, scrolling it into view. Rows outside the viewport
    /// have no element yet (the tree is virtualized), so the focus handle is
    /// created here and the row picks it up when it renders.
    fn focus_row(&mut self, node_id: &SharedString, window: &mut Window, cx: &mut Context<Self>) {
        let Some(row) = self.visible_row_of(node_id) else {
            return;
        };
        let handle = self
            .row_focus_handles
            .entry(node_id.clone())
            .or_insert_with(|| cx.focus_handle())
            .clone();
        handle.focus(window, cx);
        self.list_scroll_handle
            .scroll_to_item(row, ScrollStrategy::Nearest);
        cx.notify();
    }

    pub(super) fn collapse_all(&mut self, cx: &mut Context<Self>) {
        self.menu = None;
        self.expanded_node_ids.clear();
        self.rebuild_visible_rows();
        cx.emit(LayersPanelAction::CollapseAllRequested);
        cx.notify();
    }

    pub(super) fn toggle_expansion(&mut self, node_id: SharedString, cx: &mut Context<Self>) {
        let expanded = if self.expanded_node_ids.remove(&node_id) {
            false
        } else {
            self.expanded_node_ids.insert(node_id.clone());
            true
        };
        self.rebuild_visible_rows();
        self.menu = None;
        cx.emit(LayersPanelAction::ExpansionChanged { node_id, expanded });
        cx.notify();
    }

    pub(super) fn request_selection(
        &mut self,
        node_id: SharedString,
        mode: LayersPanelSelectionMode,
        cx: &mut Context<Self>,
    ) {
        self.menu = None;
        cx.emit(LayersPanelAction::SelectRequested { node_id, mode });
        cx.notify();
    }

    pub(super) fn request_move(
        &mut self,
        node_id: SharedString,
        target_node_id: SharedString,
        pointer: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        let Some(position) = self.drop_placement(&node_id, &target_node_id, pointer, cx) else {
            return;
        };
        self.menu = None;
        cx.emit(LayersPanelAction::MoveRequested {
            node_id,
            target_node_id,
            position,
        });
        cx.notify();
    }

    pub(super) fn begin_rename(
        &mut self,
        node_id: SharedString,
        original_title: SharedString,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.menu = None;
        self.editing = Some(LayerEditorTarget {
            node_id,
            original_title: original_title.clone(),
        });
        self.rename_input.update(cx, |input, cx| {
            input.set_value(original_title, window, cx);
            input.focus(window, cx);
        });
        self.select_editor_after_render = true;
        cx.notify();
    }

    pub(super) fn commit_rename(
        &mut self,
        restore_focus: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(target) = self.editing.take() else {
            return;
        };
        let title = self.rename_input.read(cx).value();
        let title = title.trim();
        if !title.is_empty() && title != target.original_title.as_ref() {
            cx.emit(LayersPanelAction::RenameRequested {
                node_id: target.node_id,
                title: title.to_owned().into(),
            });
        }
        if restore_focus {
            self.focus_panel_after_action(window, cx);
        }
        cx.notify();
    }

    pub(super) fn open_menu(
        &mut self,
        node: LayerNode,
        anchor: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.editing = None;
        let return_focus = window.focused(cx);
        if !self.selected_node_ids.contains(&node.id) {
            cx.emit(LayersPanelAction::SelectRequested {
                node_id: node.id.clone(),
                mode: LayersPanelSelectionMode::Replace,
            });
        }
        self.menu = Some(LayerMenuState {
            node,
            anchor,
            return_focus,
        });
        self.menu_scroll_handle.set_offset(Point::default());
        let focus_handle = self.menu_focus_handle.clone();
        window.defer(cx, move |window, cx| {
            focus_handle.focus(window, cx);
        });
        cx.notify();
    }

    pub(super) fn open_menu_from_keyboard(
        &mut self,
        node: LayerNode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(bounds) = self.node_row_bounds.get(&node.id).copied() else {
            return;
        };
        self.open_menu(
            node,
            bounds.bottom_left() + gpui::point(px(0.), px(4.)),
            window,
            cx,
        );
    }

    pub(super) fn activate_menu_action(
        &mut self,
        action: LayersPanelContextAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(menu) = self.menu.take() else {
            return;
        };
        match action {
            LayersPanelContextAction::Rename => {
                self.begin_rename(menu.node.id, menu.node.title, window, cx);
            }
            LayersPanelContextAction::ShowHide => {
                cx.emit(LayersPanelAction::VisibilityChanged {
                    node_id: menu.node.id,
                    visible: !menu.node.visible,
                });
                self.focus_panel_after_action(window, cx);
            }
            LayersPanelContextAction::LockUnlock => {
                cx.emit(LayersPanelAction::LockChanged {
                    node_id: menu.node.id,
                    locked: !menu.node.locked,
                });
                self.focus_panel_after_action(window, cx);
            }
            action => {
                cx.emit(LayersPanelAction::ContextActionRequested {
                    node_id: menu.node.id,
                    action,
                });
                self.focus_panel_after_action(window, cx);
            }
        }
        cx.notify();
    }

    pub(super) fn request_visibility(&mut self, node: &LayerNode, cx: &mut Context<Self>) {
        cx.emit(LayersPanelAction::VisibilityChanged {
            node_id: node.id.clone(),
            visible: !node.visible,
        });
        cx.notify();
    }

    pub(super) fn request_lock(&mut self, node: &LayerNode, cx: &mut Context<Self>) {
        cx.emit(LayersPanelAction::LockChanged {
            node_id: node.id.clone(),
            locked: !node.locked,
        });
        cx.notify();
    }

    fn focus_panel_after_action(&self, window: &mut Window, cx: &mut Context<Self>) {
        let focus_handle = self.focus_handle.clone();
        window.defer(cx, move |window, cx| {
            focus_handle.focus(window, cx);
        });
    }
}

pub(super) fn layer_drop_position(
    target_kind: LayersPanelNodeKind,
    bounds: Bounds<Pixels>,
    pointer: Point<Pixels>,
) -> LayersPanelDropPosition {
    let upper_edge = bounds.top() + bounds.size.height * 0.3;
    let lower_edge = bounds.bottom() - bounds.size.height * 0.3;

    if pointer.y < upper_edge {
        LayersPanelDropPosition::Before
    } else if pointer.y > lower_edge {
        LayersPanelDropPosition::After
    } else if target_kind.accepts_dropped_children() {
        LayersPanelDropPosition::Inside
    } else if pointer.y < bounds.center().y {
        LayersPanelDropPosition::Before
    } else {
        LayersPanelDropPosition::After
    }
}
