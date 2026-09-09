use std::collections::{BTreeMap, BTreeSet};

use editor::{Editor, EditorEvent};
use fanta_doc::{
    BoundProp, Color as FantaColor, Doc, Mode, ModeId, ModeScope, NodeData, NodeId, Operation,
    VarValue, Variable, VariableCollection, VariableCollectionId, VariableId, VariableType,
};
use gpui::{
    App, ClickEvent, Context, Entity, EventEmitter, FocusHandle, Focusable, IntoElement,
    KeyDownEvent, Render, SharedString, Subscription, Window, div, px,
};
use ui::{
    ContextMenu, ContextMenuEntry, Divider, DropdownMenu, DropdownStyle, IconPosition, Tooltip,
    prelude::*,
};
use util::ResultExt as _;

use crate::document::{DocChange, FigItem, FigItemEvent};
use crate::inspector_components::{InspectorMessage, InspectorSectionHeader};
use crate::mode_overrides::mode_override_operation;
use crate::variable_binding::{
    VariableBindingOption, bindable_properties, variable_binding_model, variable_binding_operation,
};

const COLLECTION_WIDTH: f32 = 220.0;
const VARIABLE_NAME_WIDTH: f32 = 200.0;
const VARIABLE_TYPE_WIDTH: f32 = 100.0;
const MODE_WIDTH: f32 = 180.0;
const BINDINGS_WIDTH: f32 = 280.0;
const TABLE_ROW_HEIGHT: f32 = 36.0;

const PRIMITIVE_VARIABLE_TYPES: [VariableType; 4] = [
    VariableType::Color,
    VariableType::Float,
    VariableType::String,
    VariableType::Boolean,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct VariableCell {
    variable: VariableId,
    mode: ModeId,
    variable_type: VariableType,
}

#[derive(Debug, Clone)]
struct CollectionSummary {
    id: VariableCollectionId,
    name: SharedString,
    variable_count: usize,
}

#[derive(Debug, Clone)]
struct VariableRowSnapshot {
    id: VariableId,
    name: SharedString,
    variable_type: VariableType,
    values: BTreeMap<ModeId, VarValue>,
}

#[derive(Debug, Clone)]
struct CollectionSnapshot {
    id: VariableCollectionId,
    name: SharedString,
    modes: Vec<Mode>,
    variables: Vec<VariableRowSnapshot>,
}

#[derive(Debug, Clone)]
struct BindingRowSnapshot {
    prop: BoundProp,
    label: SharedString,
    current: Option<VariableId>,
    choices: Vec<VariableBindingOption>,
}

#[derive(Debug, Clone)]
struct BindingSnapshot {
    node: NodeId,
    node_name: SharedString,
    rows: Vec<BindingRowSnapshot>,
}

#[derive(Debug, Clone)]
struct VariablesSnapshot {
    collections: Vec<CollectionSummary>,
    selected: Option<CollectionSnapshot>,
    binding: Option<BindingSnapshot>,
}

#[derive(Debug, Clone)]
struct ModeScopeSnapshot {
    label: SharedString,
    scope: ModeScope,
    current: Option<ModeId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VariableRenameTarget {
    Collection(VariableCollectionId),
    Variable(VariableId),
    Mode {
        collection: VariableCollectionId,
        mode: ModeId,
    },
}

pub struct FantaVariablesWorkspace {
    item: Entity<FigItem>,
    focus_handle: FocusHandle,
    selected_collection: Option<VariableCollectionId>,
    new_variable_type: VariableType,
    editing_cell: Option<VariableCell>,
    value_edit_baseline: Option<VarValue>,
    value_edit_previewed: bool,
    value_editor: Entity<Editor>,
    rename_target: Option<VariableRenameTarget>,
    rename_editor: Entity<Editor>,
    suppress_editor_events: bool,
    error_message: Option<SharedString>,
    _subscriptions: Vec<Subscription>,
}

impl FantaVariablesWorkspace {
    pub fn new(item: Entity<FigItem>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let value_editor = cx.new(|cx| Editor::single_line(window, cx));
        let rename_editor = cx.new(|cx| Editor::single_line(window, cx));
        let value_editor_subscription = cx.subscribe_in(
            &value_editor,
            window,
            |this: &mut Self, _, event: &EditorEvent, window, cx| match event {
                EditorEvent::BufferEdited if this.editing_cell.is_some() => {
                    this.preview_value_edit(cx);
                }
                EditorEvent::Blurred if this.editing_cell.is_some() => {
                    this.commit_value_edit_and_focus(window, cx);
                }
                _ => {}
            },
        );
        let item_subscription =
            cx.subscribe(&item, |this: &mut Self, item, event: &FigItemEvent, cx| {
                if matches!(event, FigItemEvent::StateChanged) {
                    this.editing_cell = None;
                    this.value_edit_baseline = None;
                    this.value_edit_previewed = false;
                    this.rename_target = None;
                }
                if matches!(event, FigItemEvent::SourceEditLockChanged)
                    && item.read(cx).source_edit_locked()
                {
                    let workspace = cx.weak_entity();
                    cx.defer(move |cx| {
                        workspace
                            .update(cx, |workspace, cx| workspace.cancel_value_edit(cx))
                            .log_err();
                    });
                }
                if matches!(
                    event,
                    FigItemEvent::Edited
                        | FigItemEvent::StateChanged
                        | FigItemEvent::SelectionChanged
                ) {
                    this.reconcile_selection(cx);
                    cx.notify();
                }
            });
        let rename_editor_subscription = cx.subscribe_in(
            &rename_editor,
            window,
            |this: &mut Self, _, event: &EditorEvent, window, cx| {
                if matches!(event, EditorEvent::Blurred) && this.rename_target.is_some() {
                    this.commit_rename(window, cx);
                }
            },
        );
        let selected_collection = item
            .read(cx)
            .doc()
            .and_then(|doc| doc.variables.collections.keys().next().copied());
        Self {
            item,
            focus_handle: cx.focus_handle(),
            selected_collection,
            new_variable_type: VariableType::Color,
            editing_cell: None,
            value_edit_baseline: None,
            value_edit_previewed: false,
            value_editor,
            rename_target: None,
            rename_editor,
            suppress_editor_events: false,
            error_message: None,
            _subscriptions: vec![
                value_editor_subscription,
                rename_editor_subscription,
                item_subscription,
            ],
        }
    }

    fn reconcile_selection(&mut self, cx: &App) {
        let selected_exists = self.selected_collection.is_some_and(|collection| {
            self.item
                .read(cx)
                .doc()
                .is_some_and(|doc| doc.variables.collections.contains_key(&collection))
        });
        if !selected_exists {
            self.selected_collection = self
                .item
                .read(cx)
                .doc()
                .and_then(|doc| doc.variables.collections.keys().next().copied());
        }
        if self.editing_cell.is_some_and(|cell| {
            self.item
                .read(cx)
                .doc()
                .and_then(|doc| doc.variables.variables.get(&cell.variable))
                .is_none()
        }) {
            self.editing_cell = None;
            self.value_edit_baseline = None;
            self.value_edit_previewed = false;
        }
    }

    fn select_collection(
        &mut self,
        collection: VariableCollectionId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.commit_value_edit_and_focus(window, cx);
        self.commit_rename(window, cx);
        self.selected_collection = Some(collection);
        self.error_message = None;
        cx.notify();
    }

    fn apply_operation(&mut self, operation: Operation, cx: &mut Context<Self>) -> bool {
        let result = self.item.update(cx, |item, cx| item.apply(operation, cx));
        match result {
            Ok(()) => {
                self.error_message = None;
                true
            }
            Err(error) => {
                log::error!("Fanta variables workspace operation failed: {error:#}");
                self.error_message = Some(error.to_string().into());
                cx.notify();
                false
            }
        }
    }

    fn start_rename(
        &mut self,
        target: VariableRenameTarget,
        initial: SharedString,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.commit_value_edit_and_focus(window, cx);
        self.commit_rename(window, cx);
        self.suppress_editor_events = true;
        self.rename_editor.update(cx, |editor, cx| {
            editor.set_text(initial, window, cx);
            editor.select_all(&editor::actions::SelectAll, window, cx);
        });
        self.suppress_editor_events = false;
        self.rename_target = Some(target);
        self.error_message = None;
        self.rename_editor.focus_handle(cx).focus(window, cx);
        cx.notify();
    }

    fn commit_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(target) = self.rename_target.take() else {
            return;
        };
        let new_name = self.rename_editor.read(cx).text(cx).trim().to_owned();
        if new_name.is_empty() {
            self.error_message = Some("Names cannot be empty".into());
            self.focus_handle.focus(window, cx);
            cx.notify();
            return;
        }
        let operation = {
            let item = self.item.read(cx);
            let Some(doc) = item.doc() else {
                return;
            };
            rename_operation(doc, target, new_name)
        };
        self.apply_built_operation(operation, cx);
        self.focus_handle.focus(window, cx);
        cx.notify();
    }

    fn cancel_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.rename_target = None;
        self.error_message = None;
        self.focus_handle.focus(window, cx);
        cx.notify();
    }

    fn finish_content_preview(&self, committed: bool, cx: &mut Context<Self>) {
        self.item.update(cx, |item, cx| {
            item.finish_content_preview(committed, cx);
        });
    }

    fn create_collection(&mut self, cx: &mut Context<Self>) {
        let operation = {
            let item = self.item.read(cx);
            let Some(doc) = item.doc() else {
                return;
            };
            create_collection_operation(doc)
        };
        let collection = match &operation {
            Operation::CreateVariableCollection { collection } => collection.id,
            _ => return,
        };
        if self.apply_operation(operation, cx) {
            self.selected_collection = Some(collection);
            cx.notify();
        }
    }

    fn create_variable(&mut self, cx: &mut Context<Self>) {
        let Some(collection) = self.selected_collection else {
            return;
        };
        let operation = {
            let item = self.item.read(cx);
            let Some(doc) = item.doc() else {
                return;
            };
            create_variable_operation(doc, collection, self.new_variable_type)
        };
        match operation {
            Ok(operation) => {
                self.apply_operation(operation, cx);
            }
            Err(error) => {
                self.error_message = Some(error.into());
                cx.notify();
            }
        }
    }

    fn add_mode(&mut self, cx: &mut Context<Self>) {
        let Some(collection) = self.selected_collection else {
            return;
        };
        let operation = {
            let item = self.item.read(cx);
            let Some(doc) = item.doc() else {
                return;
            };
            add_mode_operation(doc, collection)
        };
        match operation {
            Ok(operation) => {
                self.apply_operation(operation, cx);
            }
            Err(error) => {
                self.error_message = Some(error.into());
                cx.notify();
            }
        }
    }

    fn start_value_edit(
        &mut self,
        cell: VariableCell,
        initial: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.commit_value_edit_and_focus(window, cx);
        self.finish_content_preview(true, cx);
        if cell.variable_type == VariableType::Boolean {
            self.toggle_boolean_value(cell, cx);
            return;
        }
        if cell.variable_type == VariableType::Typography {
            self.error_message =
                Some("Typography variables are read-only in this first slice".into());
            cx.notify();
            return;
        }
        let baseline = self
            .item
            .read(cx)
            .doc()
            .and_then(|doc| doc.variables.variables.get(&cell.variable))
            .and_then(|variable| variable.values_by_mode.get(&cell.mode))
            .cloned();
        self.suppress_editor_events = true;
        self.value_editor.update(cx, |editor, cx| {
            editor.set_text(initial, window, cx);
            editor.select_all(&editor::actions::SelectAll, window, cx);
        });
        self.suppress_editor_events = false;
        self.editing_cell = Some(cell);
        self.value_edit_baseline = baseline;
        self.value_edit_previewed = false;
        self.error_message = None;
        self.value_editor.focus_handle(cx).focus(window, cx);
        cx.notify();
    }

    fn toggle_boolean_value(&mut self, cell: VariableCell, cx: &mut Context<Self>) {
        let result = {
            let item = self.item.read(cx);
            let Some(doc) = item.doc() else {
                return;
            };
            let current = doc
                .variables
                .variables
                .get(&cell.variable)
                .and_then(|variable| variable.values_by_mode.get(&cell.mode));
            let next = !matches!(current, Some(VarValue::Boolean { value: true }));
            set_variable_value_operation(
                doc,
                cell.variable,
                cell.mode,
                VarValue::Boolean { value: next },
            )
        };
        self.apply_built_operation(result, cx);
    }

    pub(crate) fn finish_value_edit(&mut self, cx: &mut Context<Self>) {
        if self.suppress_editor_events {
            return;
        }
        let Some(cell) = self.editing_cell.take() else {
            return;
        };
        let baseline = self.value_edit_baseline.take();
        let previewed = std::mem::take(&mut self.value_edit_previewed);
        let text = self.value_editor.read(cx).text(cx);
        let parsed = parse_primitive_value(cell.variable_type, text.trim());
        if previewed {
            // Rewinding a transient preview changes what the renderer must
            // resolve even when the final text cannot produce an operation.
            // A valid commit immediately emits its own content change too, but
            // invalid input has no later event to invalidate the preview.
            self.restore_value_preview(cell, baseline, true, cx);
        }
        let result = parsed.and_then(|value| {
            let item = self.item.read(cx);
            let Some(doc) = item.doc() else {
                return Err("The document is no longer available");
            };
            set_variable_value_operation(doc, cell.variable, cell.mode, value)
        });
        let committed = self.apply_built_operation(result, cx);
        if previewed {
            self.finish_content_preview(committed, cx);
        }
        cx.notify();
    }

    fn commit_value_edit_and_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.finish_value_edit(cx);
        if self
            .value_editor
            .focus_handle(cx)
            .contains_focused(window, cx)
        {
            self.focus_handle.focus(window, cx);
        }
        cx.notify();
    }

    pub(crate) fn cancel_value_edit(&mut self, cx: &mut Context<Self>) {
        if let Some(cell) = self.editing_cell.take() {
            let baseline = self.value_edit_baseline.take();
            let previewed = std::mem::take(&mut self.value_edit_previewed);
            if previewed {
                self.restore_value_preview(cell, baseline, true, cx);
                self.finish_content_preview(false, cx);
            }
        }
        self.error_message = None;
        cx.notify();
    }

    fn preview_value_edit(&mut self, cx: &mut Context<Self>) {
        if self.suppress_editor_events {
            return;
        }
        let Some(cell) = self.editing_cell else {
            return;
        };
        let text = self.value_editor.read(cx).text(cx);
        let value = match parse_primitive_value(cell.variable_type, text.trim()) {
            Ok(value) => value,
            Err(error) => {
                self.error_message = Some(error.into());
                cx.notify();
                return;
            }
        };
        let result = self.item.update(cx, |item, cx| {
            item.with_document(cx, |document| {
                let result = preview_variable_value(&mut document.doc, cell, value);
                let change = match result {
                    Ok(true) => {
                        document.mark_variables_changed();
                        DocChange::ContentPreview
                    }
                    Ok(false) | Err(_) => DocChange::None,
                };
                (result, change)
            })
        });
        match result {
            Some(Ok(true)) => {
                self.value_edit_previewed = true;
                self.error_message = None;
            }
            Some(Ok(false)) => {
                self.error_message = None;
            }
            Some(Err(error)) => {
                self.error_message = Some(error.into());
            }
            None => {
                self.error_message = Some("The document is no longer available".into());
            }
        }
        cx.notify();
    }

    fn restore_value_preview(
        &mut self,
        cell: VariableCell,
        baseline: Option<VarValue>,
        notify: bool,
        cx: &mut Context<Self>,
    ) {
        self.item.update(cx, |item, cx| {
            item.with_document(cx, |document| {
                let changed = restore_variable_value(&mut document.doc, cell, baseline);
                if changed {
                    document.mark_variables_changed();
                }
                let change = if changed && notify {
                    DocChange::ContentPreview
                } else {
                    DocChange::None
                };
                ((), change)
            });
        });
    }

    fn apply_built_operation(
        &mut self,
        result: Result<Option<Operation>, &'static str>,
        cx: &mut Context<Self>,
    ) -> bool {
        match result {
            Ok(Some(operation)) => self.apply_operation(operation, cx),
            Ok(None) => {
                self.error_message = None;
                false
            }
            Err(error) => {
                self.error_message = Some(error.into());
                cx.notify();
                false
            }
        }
    }

    fn handle_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event.keystroke.key.as_str() {
            "enter" if self.rename_target.is_some() => {
                cx.stop_propagation();
                self.commit_rename(window, cx);
            }
            "escape" if self.rename_target.is_some() => {
                cx.stop_propagation();
                self.cancel_rename(window, cx);
            }
            "enter" if self.editing_cell.is_some() => {
                cx.stop_propagation();
                self.commit_value_edit_and_focus(window, cx);
            }
            "escape" if self.editing_cell.is_some() => {
                cx.stop_propagation();
                self.cancel_value_edit(cx);
                self.focus_handle.focus(window, cx);
            }
            _ => {}
        }
    }

    fn bind_property(
        &mut self,
        node: NodeId,
        prop: BoundProp,
        variable: VariableId,
        cx: &mut Context<Self>,
    ) {
        let result = {
            let item = self.item.read(cx);
            let Some(doc) = item.doc() else {
                return;
            };
            variable_binding_operation(doc, node, prop, Some(variable))
        };
        self.apply_built_operation(result, cx);
    }

    fn unbind_property(&mut self, node: NodeId, prop: BoundProp, cx: &mut Context<Self>) {
        let result = {
            let item = self.item.read(cx);
            let Some(doc) = item.doc() else {
                return;
            };
            variable_binding_operation(doc, node, prop, None)
        };
        self.apply_built_operation(result, cx);
    }

    fn set_mode_scope(
        &mut self,
        scope: ModeScope,
        collection: VariableCollectionId,
        new: Option<ModeId>,
        cx: &mut Context<Self>,
    ) {
        let result = {
            let item = self.item.read(cx);
            let Some(doc) = item.doc() else {
                return;
            };
            set_active_mode_operation(doc, scope, collection, new)
        };
        self.apply_built_operation(result, cx);
    }

    fn snapshot(&self, cx: &App) -> VariablesSnapshot {
        let Some(doc) = self.item.read(cx).doc() else {
            return VariablesSnapshot {
                collections: Vec::new(),
                selected: None,
                binding: None,
            };
        };
        variables_snapshot(doc, self.selected_collection)
    }

    fn render_variable_type_dropdown(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let workspace = cx.weak_entity();
        let selected = self.new_variable_type;
        let menu = ContextMenu::build(window, cx, move |mut menu, _, _| {
            for variable_type in PRIMITIVE_VARIABLE_TYPES {
                let workspace = workspace.clone();
                menu.push_item(
                    ContextMenuEntry::new(variable_type_label(variable_type))
                        .toggleable(IconPosition::End, variable_type == selected)
                        .handler(move |_, cx| {
                            workspace
                                .update(cx, |workspace, cx| {
                                    workspace.new_variable_type = variable_type;
                                    cx.notify();
                                })
                                .log_err();
                        }),
                );
            }
            menu
        });
        DropdownMenu::new(
            "fanta-variable-type",
            variable_type_label(self.new_variable_type),
            menu,
        )
        .style(DropdownStyle::Outlined)
        .trigger_size(ButtonSize::Compact)
        .aria_label("New variable type")
    }

    fn render_mode_scope_dropdown(
        &self,
        index: usize,
        collection: &CollectionSnapshot,
        scope: ModeScopeSnapshot,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let inherited_label = match &scope.scope {
            ModeScope::Doc => "Collection default",
            ModeScope::Frame { .. } => "Inherit parent",
        };
        let current_name = scope
            .current
            .and_then(|current| {
                collection
                    .modes
                    .iter()
                    .find(|mode| mode.id == current)
                    .map(|mode| mode.name.clone())
            })
            .unwrap_or_else(|| inherited_label.into());
        let label: SharedString = format!("{} · {current_name}", scope.label).into();
        let workspace = cx.weak_entity();
        let collection_id = collection.id;
        let selected = scope.current;
        let mode_scope = scope.scope;
        let modes = collection.modes.clone();
        let menu = ContextMenu::build(window, cx, move |mut menu, _, _| {
            let workspace_for_default = workspace.clone();
            let default_scope = mode_scope.clone();
            menu.push_item(
                ContextMenuEntry::new(inherited_label)
                    .toggleable(IconPosition::End, selected.is_none())
                    .handler(move |_, cx| {
                        workspace_for_default
                            .update(cx, |workspace, cx| {
                                workspace.set_mode_scope(
                                    default_scope.clone(),
                                    collection_id,
                                    None,
                                    cx,
                                )
                            })
                            .log_err();
                    }),
            );
            for mode in &modes {
                let workspace = workspace.clone();
                let scope = mode_scope.clone();
                let mode_id = mode.id;
                menu.push_item(
                    ContextMenuEntry::new(mode.name.clone())
                        .toggleable(IconPosition::End, selected == Some(mode_id))
                        .handler(move |_, cx| {
                            workspace
                                .update(cx, |workspace, cx| {
                                    workspace.set_mode_scope(
                                        scope.clone(),
                                        collection_id,
                                        Some(mode_id),
                                        cx,
                                    )
                                })
                                .log_err();
                        }),
                );
            }
            menu
        });
        DropdownMenu::new(("fanta-variable-mode-scope", index), label, menu)
            .style(DropdownStyle::Outlined)
            .trigger_size(ButtonSize::Compact)
            .aria_label("Variable mode scope")
    }

    fn render_mode_scope_bar(
        &self,
        collection: &CollectionSnapshot,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let scopes = self
            .item
            .read(cx)
            .doc()
            .map(|doc| mode_scope_snapshots(doc, collection.id))
            .unwrap_or_default();
        let dropdowns: Vec<AnyElement> = scopes
            .into_iter()
            .enumerate()
            .map(|(index, scope)| {
                self.render_mode_scope_dropdown(index, collection, scope, window, cx)
                    .into_any_element()
            })
            .collect();
        h_flex()
            .h(px(38.0))
            .flex_none()
            .px_3()
            .gap_2()
            .border_b_1()
            .border_color(cx.theme().colors().border)
            .child(
                Label::new("Modes")
                    .size(LabelSize::XSmall)
                    .weight(gpui::FontWeight::BOLD),
            )
            .children(dropdowns)
    }

    fn render_collections(
        &self,
        snapshot: &VariablesSnapshot,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let mut list = v_flex()
            .id("fanta-variable-collections")
            .w(px(COLLECTION_WIDTH))
            .h_full()
            .flex_none()
            .border_r_1()
            .border_color(cx.theme().colors().border)
            .child(
                InspectorSectionHeader::new("Collections").action(
                    IconButton::new("fanta-variable-add-collection", IconName::Plus)
                        .icon_size(IconSize::Small)
                        .tooltip(Tooltip::text("Create collection"))
                        .on_click(
                            cx.listener(|workspace, _, _, cx| workspace.create_collection(cx)),
                        ),
                ),
            );
        if snapshot.collections.is_empty() {
            return list
                .child(
                    v_flex().px_3().py_2().child(
                        Label::new("Create a collection to start defining reusable values.")
                            .size(LabelSize::Small)
                            .color(Color::Muted),
                    ),
                )
                .into_any_element();
        }
        for (index, collection) in snapshot.collections.iter().enumerate() {
            let collection_id = collection.id;
            let collection_name = collection.name.clone();
            let selected = self.selected_collection == Some(collection_id);
            list = list.child(
                Button::new(
                    ("fanta-variable-collection", index),
                    format!("{} ({})", collection.name, collection.variable_count),
                )
                .style(ButtonStyle::Subtle)
                .toggle_state(selected)
                .full_width()
                .on_click(cx.listener(
                    move |workspace, event: &ClickEvent, window, cx| {
                        if event.click_count() >= 2 {
                            workspace.start_rename(
                                VariableRenameTarget::Collection(collection_id),
                                collection_name.clone(),
                                window,
                                cx,
                            );
                        } else {
                            workspace.select_collection(collection_id, window, cx);
                        }
                    },
                )),
            );
        }
        list.into_any_element()
    }

    fn render_table_header(
        &self,
        collection: &CollectionSnapshot,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let mut row = h_flex()
            .h(px(TABLE_ROW_HEIGHT))
            .flex_none()
            .border_b_1()
            .child(table_header_cell("Name", VARIABLE_NAME_WIDTH))
            .child(table_header_cell("Type", VARIABLE_TYPE_WIDTH));
        for (mode_index, mode) in collection.modes.iter().enumerate() {
            let target = VariableRenameTarget::Mode {
                collection: collection.id,
                mode: mode.id,
            };
            if self.rename_target == Some(target) {
                row = row.child(
                    div()
                        .w(px(MODE_WIDTH))
                        .h(px(TABLE_ROW_HEIGHT))
                        .flex_none()
                        .px_1()
                        .py_1()
                        .border_l_1()
                        .child(self.rename_editor.clone()),
                );
            } else {
                let initial: SharedString = mode.name.clone().into();
                row = row.child(
                    div()
                        .id(("fanta-variable-mode-name", mode_index))
                        .w(px(MODE_WIDTH))
                        .h(px(TABLE_ROW_HEIGHT))
                        .flex_none()
                        .px_2()
                        .flex()
                        .items_center()
                        .border_l_1()
                        .border_color(cx.theme().colors().border)
                        .cursor_text()
                        .tooltip(Tooltip::text("Double-click to rename mode"))
                        .on_click(
                            cx.listener(move |workspace, event: &ClickEvent, window, cx| {
                                if event.click_count() >= 2 {
                                    workspace.start_rename(target, initial.clone(), window, cx);
                                }
                            }),
                        )
                        .child(
                            Label::new(mode.name.clone())
                                .size(LabelSize::XSmall)
                                .color(Color::Muted)
                                .single_line(),
                        ),
                );
            }
        }
        row
    }

    fn render_variable_name_cell(
        &self,
        row_index: usize,
        variable: &VariableRowSnapshot,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let target = VariableRenameTarget::Variable(variable.id);
        if self.rename_target == Some(target) {
            return div()
                .w(px(VARIABLE_NAME_WIDTH))
                .h(px(TABLE_ROW_HEIGHT))
                .flex_none()
                .px_1()
                .py_1()
                .child(self.rename_editor.clone())
                .into_any_element();
        }
        let initial = variable.name.clone();
        div()
            .id(("fanta-variable-name", row_index))
            .w(px(VARIABLE_NAME_WIDTH))
            .h(px(TABLE_ROW_HEIGHT))
            .flex_none()
            .px_2()
            .flex()
            .items_center()
            .cursor_text()
            .tooltip(Tooltip::text("Double-click to rename variable"))
            .hover(|cell| cell.bg(cx.theme().colors().element_hover))
            .on_click(
                cx.listener(move |workspace, event: &ClickEvent, window, cx| {
                    if event.click_count() >= 2 {
                        workspace.start_rename(target, initial.clone(), window, cx);
                    }
                }),
            )
            .child(
                Label::new(variable.name.clone())
                    .size(LabelSize::Small)
                    .single_line(),
            )
            .into_any_element()
    }

    fn render_value_cell(
        &self,
        row_index: usize,
        mode_index: usize,
        variable: &VariableRowSnapshot,
        mode: &Mode,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let cell = VariableCell {
            variable: variable.id,
            mode: mode.id,
            variable_type: variable.variable_type,
        };
        if self.editing_cell == Some(cell) {
            return div()
                .w(px(MODE_WIDTH))
                .h(px(TABLE_ROW_HEIGHT))
                .flex_none()
                .px_1()
                .py_1()
                .border_l_1()
                .child(self.value_editor.clone())
                .into_any_element();
        }
        let value = variable.values.get(&mode.id);
        let display = display_variable_value(value, variable.variable_type);
        let initial = editor_variable_value(value, variable.variable_type);
        div()
            .id((
                "fanta-variable-value",
                row_index.saturating_mul(1_000).saturating_add(mode_index),
            ))
            .w(px(MODE_WIDTH))
            .h(px(TABLE_ROW_HEIGHT))
            .flex_none()
            .px_2()
            .flex()
            .items_center()
            .border_l_1()
            .border_color(cx.theme().colors().border)
            .hover(|cell| cell.bg(cx.theme().colors().element_hover))
            .cursor_pointer()
            .on_click(cx.listener(move |workspace, _, window, cx| {
                workspace.start_value_edit(cell, initial.clone(), window, cx)
            }))
            .child(
                Label::new(display)
                    .size(LabelSize::Small)
                    .single_line()
                    .when(value.is_none(), |label| label.color(Color::Muted)),
            )
            .into_any_element()
    }

    fn render_variable_table(
        &self,
        collection: &CollectionSnapshot,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let collection_target = VariableRenameTarget::Collection(collection.id);
        let collection_name = if self.rename_target == Some(collection_target) {
            div()
                .flex_1()
                .min_w_0()
                .child(self.rename_editor.clone())
                .into_any_element()
        } else {
            let initial = collection.name.clone();
            v_flex()
                .id("fanta-variable-collection-name")
                .flex_1()
                .min_w_0()
                .cursor_text()
                .tooltip(Tooltip::text("Double-click to rename collection"))
                .on_click(
                    cx.listener(move |workspace, event: &ClickEvent, window, cx| {
                        if event.click_count() >= 2 {
                            workspace.start_rename(collection_target, initial.clone(), window, cx);
                        }
                    }),
                )
                .child(Label::new(collection.name.clone()).single_line())
                .child(
                    Label::new(format!("{} variables", collection.variables.len()))
                        .size(LabelSize::XSmall)
                        .color(Color::Muted),
                )
                .into_any_element()
        };
        let toolbar = h_flex()
            .h(px(44.))
            .flex_none()
            .px_3()
            .gap_2()
            .border_b_1()
            .border_color(cx.theme().colors().border)
            .child(collection_name)
            .child(self.render_variable_type_dropdown(window, cx))
            .child(
                Button::new("fanta-variable-add-variable", "Create variable")
                    .size(ButtonSize::Compact)
                    .start_icon(Icon::new(IconName::Plus).size(IconSize::XSmall))
                    .on_click(cx.listener(|workspace, _, _, cx| workspace.create_variable(cx))),
            )
            .child(
                Button::new("fanta-variable-add-mode", "Add mode")
                    .size(ButtonSize::Compact)
                    .on_click(cx.listener(|workspace, _, _, cx| workspace.add_mode(cx))),
            );

        let mut table = v_flex()
            .id("fanta-variable-table")
            .min_w(px(VARIABLE_NAME_WIDTH
                + VARIABLE_TYPE_WIDTH
                + MODE_WIDTH * collection.modes.len() as f32))
            .child(self.render_table_header(collection, cx));
        if collection.variables.is_empty() {
            table = table.child(
                h_flex().h(px(80.)).px_3().child(
                    Label::new("No variables in this collection")
                        .size(LabelSize::Small)
                        .color(Color::Muted),
                ),
            );
        } else {
            for (row_index, variable) in collection.variables.iter().enumerate() {
                let mut row = h_flex()
                    .h(px(TABLE_ROW_HEIGHT))
                    .flex_none()
                    .border_b_1()
                    .border_color(cx.theme().colors().border)
                    .child(self.render_variable_name_cell(row_index, variable, cx))
                    .child(table_value_cell(
                        variable_type_label(variable.variable_type),
                        VARIABLE_TYPE_WIDTH,
                    ));
                for (mode_index, mode) in collection.modes.iter().enumerate() {
                    row = row
                        .child(self.render_value_cell(row_index, mode_index, variable, mode, cx));
                }
                table = table.child(row);
            }
        }
        v_flex()
            .flex_1()
            .min_w_0()
            .h_full()
            .overflow_hidden()
            .child(toolbar)
            .child(self.render_mode_scope_bar(collection, window, cx))
            .child(
                div()
                    .id("fanta-variable-table-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .overflow_x_scroll()
                    .child(table),
            )
    }

    fn render_binding_row(
        &self,
        index: usize,
        binding: &BindingSnapshot,
        row: &BindingRowSnapshot,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let current_label = row
            .current
            .and_then(|current| {
                row.choices
                    .iter()
                    .find(|choice| choice.id == current)
                    .map(|choice| choice.label.clone())
            })
            .unwrap_or_else(|| {
                if row.choices.is_empty() {
                    "No compatible variables".into()
                } else {
                    "Unbound".into()
                }
            });
        let workspace = cx.weak_entity();
        let node = binding.node;
        let prop = row.prop;
        let current = row.current;
        let choices = row.choices.clone();
        let menu = ContextMenu::build(window, cx, move |mut menu, _, _| {
            if current.is_some() {
                let workspace = workspace.clone();
                menu.push_item(ContextMenuEntry::new("Unbind").handler(move |_, cx| {
                    workspace
                        .update(cx, |workspace, cx| {
                            workspace.unbind_property(node, prop, cx)
                        })
                        .log_err();
                }));
            }
            for choice in &choices {
                let workspace = workspace.clone();
                let variable = choice.id;
                menu.push_item(
                    ContextMenuEntry::new(choice.label.clone())
                        .toggleable(IconPosition::End, current == Some(variable))
                        .handler(move |_, cx| {
                            workspace
                                .update(cx, |workspace, cx| {
                                    workspace.bind_property(node, prop, variable, cx)
                                })
                                .log_err();
                        }),
                );
            }
            menu
        });
        v_flex()
            .px_3()
            .py_1()
            .gap_1()
            .child(
                Label::new(row.label.clone())
                    .size(LabelSize::XSmall)
                    .color(Color::Muted),
            )
            .child(
                DropdownMenu::new(("fanta-variable-binding", index), current_label, menu)
                    .style(DropdownStyle::Outlined)
                    .trigger_size(ButtonSize::Compact)
                    .full_width(true)
                    .disabled(row.choices.is_empty() && row.current.is_none())
                    .aria_label(format!("Bind {}", row.label)),
            )
    }

    fn render_bindings(
        &self,
        binding: Option<&BindingSnapshot>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let root = v_flex()
            .w(px(BINDINGS_WIDTH))
            .h_full()
            .flex_none()
            .border_l_1()
            .border_color(cx.theme().colors().border)
            .child(InspectorSectionHeader::new("Bind selected layer"));
        let Some(binding) = binding else {
            return root
                .child(
                    v_flex().px_3().child(
                        Label::new("Select one layer on the Canvas to bind its properties.")
                            .size(LabelSize::Small)
                            .color(Color::Muted),
                    ),
                )
                .into_any_element();
        };
        let mut root = root
            .child(
                v_flex()
                    .px_3()
                    .pb_2()
                    .child(Label::new(binding.node_name.clone()).single_line()),
            )
            .child(Divider::horizontal());
        if binding.rows.is_empty() {
            return root
                .child(
                    v_flex().px_3().py_2().child(
                        Label::new("This layer has no supported bindable fields.")
                            .size(LabelSize::Small)
                            .color(Color::Muted),
                    ),
                )
                .into_any_element();
        }
        for (index, row) in binding.rows.iter().enumerate() {
            root = root.child(self.render_binding_row(index, binding, row, window, cx));
        }
        root.into_any_element()
    }
}

impl Render for FantaVariablesWorkspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let snapshot = self.snapshot(cx);
        let mut root = v_flex()
            .key_context("FantaVariablesWorkspace")
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(Self::handle_key_down))
            .size_full()
            .overflow_hidden()
            .bg(cx.theme().colors().panel_background)
            .child(
                h_flex()
                    .h(px(40.))
                    .flex_none()
                    .px_3()
                    .gap_2()
                    .border_b_1()
                    .border_color(cx.theme().colors().border)
                    .child(Label::new("Variables").size(LabelSize::Large))
                    .child(
                        Label::new("Collections, modes, values, and bindings")
                            .size(LabelSize::Small)
                            .color(Color::Muted),
                    ),
            );
        if let Some(error) = self.error_message.clone() {
            root = root.child(
                h_flex()
                    .flex_none()
                    .px_3()
                    .py_1()
                    .bg(cx.theme().status().error_background)
                    .child(Label::new(error).size(LabelSize::Small)),
            );
        }
        root.child(
            h_flex()
                .flex_1()
                .min_h_0()
                .overflow_hidden()
                .child(self.render_collections(&snapshot, cx))
                .child(match snapshot.selected.as_ref() {
                    Some(collection) => self
                        .render_variable_table(collection, window, cx)
                        .into_any_element(),
                    None => InspectorMessage::new(
                        "Create or select a collection to edit its variables.",
                    )
                    .into_any_element(),
                })
                .child(self.render_bindings(snapshot.binding.as_ref(), window, cx)),
        )
    }
}

impl EventEmitter<()> for FantaVariablesWorkspace {}

impl Focusable for FantaVariablesWorkspace {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

fn variables_snapshot(doc: &Doc, selected: Option<VariableCollectionId>) -> VariablesSnapshot {
    let collections = doc
        .variables
        .collections
        .values()
        .map(|collection| CollectionSummary {
            id: collection.id,
            name: collection.name.clone().into(),
            variable_count: doc
                .variables
                .variables
                .values()
                .filter(|variable| variable.collection == collection.id)
                .count(),
        })
        .collect();
    let selected = selected
        .and_then(|id| doc.variables.collections.get(&id))
        .map(|collection| collection_snapshot(doc, collection));
    VariablesSnapshot {
        collections,
        selected,
        binding: binding_snapshot(doc),
    }
}

fn mode_scope_snapshots(doc: &Doc, collection: VariableCollectionId) -> Vec<ModeScopeSnapshot> {
    let mut scopes = vec![ModeScopeSnapshot {
        label: "Project".into(),
        scope: ModeScope::Doc,
        current: doc.active_modes.get(&collection).copied(),
    }];
    let page = doc.active_page();
    if let Some(page) = page
        && let Some(node) = doc.scene.get(page)
        && let NodeData::Group(group) = &node.data
    {
        scopes.push(ModeScopeSnapshot {
            label: format!(
                "Page: {}",
                if node.name.trim().is_empty() {
                    "Untitled"
                } else {
                    node.name.as_str()
                }
            )
            .into(),
            scope: ModeScope::Frame { node: page },
            current: group.explicit_modes.get(&collection).copied(),
        });
    }
    if let Some(selected) = doc.selection.anchor()
        && Some(selected) != page
        && let Some(node) = doc.scene.get(selected)
        && let NodeData::Group(group) = &node.data
    {
        scopes.push(ModeScopeSnapshot {
            label: format!(
                "Container: {}",
                if node.name.trim().is_empty() {
                    "Untitled"
                } else {
                    node.name.as_str()
                }
            )
            .into(),
            scope: ModeScope::Frame { node: selected },
            current: group.explicit_modes.get(&collection).copied(),
        });
    }
    scopes
}

fn set_active_mode_operation(
    doc: &Doc,
    scope: ModeScope,
    collection: VariableCollectionId,
    new: Option<ModeId>,
) -> Result<Option<Operation>, &'static str> {
    mode_override_operation(doc, scope, collection, new)
}

fn collection_snapshot(doc: &Doc, collection: &VariableCollection) -> CollectionSnapshot {
    let mut variable_ids = Vec::new();
    let mut included = BTreeSet::new();
    for variable in &collection.variable_order {
        if doc
            .variables
            .variables
            .get(variable)
            .is_some_and(|candidate| candidate.collection == collection.id)
            && included.insert(*variable)
        {
            variable_ids.push(*variable);
        }
    }
    let mut unlisted: Vec<_> = doc
        .variables
        .variables
        .values()
        .filter(|variable| variable.collection == collection.id && !included.contains(&variable.id))
        .collect();
    unlisted.sort_by(|left, right| {
        left.name
            .to_lowercase()
            .cmp(&right.name.to_lowercase())
            .then(left.id.cmp(&right.id))
    });
    variable_ids.extend(unlisted.into_iter().map(|variable| variable.id));
    let variables = variable_ids
        .into_iter()
        .filter_map(|id| doc.variables.variables.get(&id))
        .map(|variable| VariableRowSnapshot {
            id: variable.id,
            name: variable.name.clone().into(),
            variable_type: variable.ty,
            values: variable.values_by_mode.clone(),
        })
        .collect();
    CollectionSnapshot {
        id: collection.id,
        name: collection.name.clone().into(),
        modes: collection.modes.clone(),
        variables,
    }
}

fn binding_snapshot(doc: &Doc) -> Option<BindingSnapshot> {
    let node_id = doc.selection.anchor()?;
    let node = doc.scene.get(node_id)?;
    let rows = bindable_properties(node)
        .into_iter()
        .filter_map(|candidate| {
            let model = variable_binding_model(doc, node_id, candidate.prop)?;
            Some(BindingRowSnapshot {
                prop: candidate.prop,
                label: candidate.label,
                current: model.current,
                choices: model.options,
            })
        })
        .collect();
    Some(BindingSnapshot {
        node: node_id,
        node_name: node.name.clone().into(),
        rows,
    })
}

fn create_collection_operation(doc: &Doc) -> Operation {
    let id = VariableCollectionId::new();
    let mode = ModeId::new();
    Operation::CreateVariableCollection {
        collection: Box::new(VariableCollection {
            id,
            name: unique_collection_name(doc),
            modes: vec![Mode {
                id: mode,
                name: "Mode 1".into(),
            }],
            default_mode: mode,
            variable_order: Vec::new(),
        }),
    }
}

fn rename_operation(
    doc: &Doc,
    target: VariableRenameTarget,
    new_name: String,
) -> Result<Option<Operation>, &'static str> {
    let operation = match target {
        VariableRenameTarget::Collection(id) => {
            let collection = doc
                .variables
                .collections
                .get(&id)
                .ok_or("The collection no longer exists")?;
            if collection.name == new_name {
                return Ok(None);
            }
            Operation::RenameVariableCollection {
                id,
                old: collection.name.clone(),
                new: new_name,
            }
        }
        VariableRenameTarget::Variable(id) => {
            let variable = doc
                .variables
                .variables
                .get(&id)
                .ok_or("The variable no longer exists")?;
            if variable.name == new_name {
                return Ok(None);
            }
            Operation::RenameVariable {
                id,
                old: variable.name.clone(),
                new: new_name,
            }
        }
        VariableRenameTarget::Mode { collection, mode } => {
            let collection_data = doc
                .variables
                .collections
                .get(&collection)
                .ok_or("The collection no longer exists")?;
            let current = collection_data
                .modes
                .iter()
                .find(|candidate| candidate.id == mode)
                .ok_or("The mode no longer exists")?;
            if current.name == new_name {
                return Ok(None);
            }
            Operation::RenameMode {
                collection,
                mode,
                old: current.name.clone(),
                new: new_name,
            }
        }
    };
    Ok(Some(operation))
}

fn create_variable_operation(
    doc: &Doc,
    collection_id: VariableCollectionId,
    variable_type: VariableType,
) -> Result<Operation, &'static str> {
    let collection = doc
        .variables
        .collections
        .get(&collection_id)
        .ok_or("The selected collection no longer exists")?;
    if collection.modes.is_empty() || !collection.has_mode(collection.default_mode) {
        return Err("The collection does not have a valid default mode");
    }
    let value = default_variable_value(variable_type);
    let values_by_mode = collection
        .modes
        .iter()
        .map(|mode| (mode.id, value.clone()))
        .collect();
    Ok(Operation::CreateVariable {
        variable: Box::new(Variable {
            id: VariableId::new(),
            collection: collection_id,
            name: unique_variable_name(doc, collection_id),
            ty: variable_type,
            values_by_mode,
            scopes: Vec::new(),
        }),
    })
}

fn add_mode_operation(
    doc: &Doc,
    collection_id: VariableCollectionId,
) -> Result<Operation, &'static str> {
    let collection = doc
        .variables
        .collections
        .get(&collection_id)
        .ok_or("The selected collection no longer exists")?;
    Ok(Operation::AddMode {
        collection: collection_id,
        mode: Mode {
            id: ModeId::new(),
            name: unique_mode_name(collection),
        },
    })
}

fn set_variable_value_operation(
    doc: &Doc,
    variable_id: VariableId,
    mode_id: ModeId,
    new_value: VarValue,
) -> Result<Option<Operation>, &'static str> {
    let variable = doc
        .variables
        .variables
        .get(&variable_id)
        .ok_or("The variable no longer exists")?;
    let collection = doc
        .variables
        .collections
        .get(&variable.collection)
        .ok_or("The variable's collection no longer exists")?;
    if !collection.has_mode(mode_id) {
        return Err("The mode no longer exists");
    }
    if new_value.variable_type() != Some(variable.ty) {
        return Err("The value does not match the variable type");
    }
    let old = variable.values_by_mode.get(&mode_id).cloned();
    if old.as_ref() == Some(&new_value) {
        return Ok(None);
    }
    Ok(Some(Operation::SetVariableValue {
        variable: variable_id,
        mode: mode_id,
        old,
        new: Some(new_value),
    }))
}

fn preview_variable_value(
    doc: &mut Doc,
    cell: VariableCell,
    value: VarValue,
) -> Result<bool, &'static str> {
    let variable = doc
        .variables
        .variables
        .get(&cell.variable)
        .ok_or("The variable no longer exists")?;
    let collection = doc
        .variables
        .collections
        .get(&variable.collection)
        .ok_or("The variable's collection no longer exists")?;
    if !collection.has_mode(cell.mode) {
        return Err("The mode no longer exists");
    }
    if variable.ty != cell.variable_type || value.variable_type() != Some(variable.ty) {
        return Err("The value does not match the variable type");
    }
    if variable.values_by_mode.get(&cell.mode) == Some(&value) {
        return Ok(false);
    }
    let variable = doc
        .variables
        .variables
        .get_mut(&cell.variable)
        .ok_or("The variable no longer exists")?;
    variable.values_by_mode.insert(cell.mode, value);
    Ok(true)
}

fn restore_variable_value(doc: &mut Doc, cell: VariableCell, baseline: Option<VarValue>) -> bool {
    let Some(variable) = doc.variables.variables.get_mut(&cell.variable) else {
        return false;
    };
    if variable.values_by_mode.get(&cell.mode) == baseline.as_ref() {
        return false;
    }
    match baseline {
        Some(value) => {
            variable.values_by_mode.insert(cell.mode, value);
        }
        None => {
            variable.values_by_mode.remove(&cell.mode);
        }
    }
    true
}

fn unique_collection_name(doc: &Doc) -> String {
    unique_numbered_name("Collection", |candidate| {
        doc.variables
            .collections
            .values()
            .any(|collection| collection.name == candidate)
    })
}

fn unique_variable_name(doc: &Doc, collection: VariableCollectionId) -> String {
    unique_numbered_name("Variable", |candidate| {
        doc.variables
            .variables
            .values()
            .any(|variable| variable.collection == collection && variable.name == candidate)
    })
}

fn unique_mode_name(collection: &VariableCollection) -> String {
    unique_numbered_name("Mode", |candidate| {
        collection.modes.iter().any(|mode| mode.name == candidate)
    })
}

fn unique_numbered_name(prefix: &str, exists: impl Fn(&str) -> bool) -> String {
    for number in 1..=usize::MAX {
        let candidate = format!("{prefix} {number}");
        if !exists(&candidate) {
            return candidate;
        }
    }
    prefix.to_owned()
}

fn default_variable_value(variable_type: VariableType) -> VarValue {
    match variable_type {
        VariableType::Color => VarValue::Color {
            value: FantaColor::BLACK,
        },
        VariableType::Float => VarValue::Float { value: 0.0 },
        VariableType::String => VarValue::String {
            value: String::new(),
        },
        VariableType::Boolean => VarValue::Boolean { value: false },
        VariableType::Typography => VarValue::TextStyle {
            value: fanta_doc::TextStyle::default(),
        },
    }
}

fn parse_primitive_value(
    variable_type: VariableType,
    text: &str,
) -> Result<VarValue, &'static str> {
    match variable_type {
        VariableType::Color => {
            let color = FantaColor::from_hex(text)
                .or_else(|| FantaColor::from_hex(&format!("#{text}")))
                .ok_or("Enter a color as #RRGGBB or #RRGGBBAA")?;
            Ok(VarValue::Color { value: color })
        }
        VariableType::Float => {
            let value = text
                .parse::<f64>()
                .ok()
                .filter(|value| value.is_finite())
                .ok_or("Enter a finite number")?;
            Ok(VarValue::Float { value })
        }
        VariableType::String => Ok(VarValue::String {
            value: text.to_owned(),
        }),
        VariableType::Boolean => match text.to_ascii_lowercase().as_str() {
            "true" => Ok(VarValue::Boolean { value: true }),
            "false" => Ok(VarValue::Boolean { value: false }),
            _ => Err("Enter true or false"),
        },
        VariableType::Typography => Err("Typography values are not editable here yet"),
    }
}

fn variable_type_label(variable_type: VariableType) -> &'static str {
    match variable_type {
        VariableType::Color => "Color",
        VariableType::Float => "Number",
        VariableType::String => "String",
        VariableType::Boolean => "Boolean",
        VariableType::Typography => "Typography",
    }
}

fn display_variable_value(value: Option<&VarValue>, variable_type: VariableType) -> SharedString {
    match value {
        Some(VarValue::Color { value }) => value.to_hex().into(),
        Some(VarValue::Float { value }) => format_float(*value).into(),
        Some(VarValue::String { value }) if value.is_empty() => "Empty string".into(),
        Some(VarValue::String { value }) => value.clone().into(),
        Some(VarValue::Boolean { value }) => value.to_string().into(),
        Some(VarValue::TextStyle { .. }) => "Typography style".into(),
        Some(VarValue::Alias { .. }) => "Alias".into(),
        None => match variable_type {
            VariableType::Boolean => "false".into(),
            _ => "Unset".into(),
        },
    }
}

fn editor_variable_value(value: Option<&VarValue>, variable_type: VariableType) -> String {
    match value {
        Some(VarValue::Color { value }) => value.to_hex(),
        Some(VarValue::Float { value }) => format_float(*value),
        Some(VarValue::String { value }) => value.clone(),
        Some(VarValue::Boolean { value }) => value.to_string(),
        _ => match default_variable_value(variable_type) {
            VarValue::Color { value } => value.to_hex(),
            VarValue::Float { value } => format_float(value),
            VarValue::String { value } => value,
            VarValue::Boolean { value } => value.to_string(),
            VarValue::TextStyle { .. } | VarValue::Alias { .. } => String::new(),
        },
    }
}

fn format_float(value: f64) -> String {
    if value.fract().abs() <= f64::EPSILON {
        format!("{value:.0}")
    } else {
        format!("{value:.3}")
            .trim_end_matches('0')
            .trim_end_matches('.')
            .to_owned()
    }
}

fn table_header_cell(label: impl Into<SharedString>, width: f32) -> impl IntoElement {
    h_flex()
        .w(px(width))
        .h_full()
        .flex_none()
        .px_2()
        .border_l_1()
        .child(
            Label::new(label.into())
                .size(LabelSize::XSmall)
                .color(Color::Muted)
                .single_line(),
        )
}

fn table_value_cell(label: impl Into<SharedString>, width: f32) -> impl IntoElement {
    h_flex().w(px(width)).h_full().flex_none().px_2().child(
        Label::new(label.into())
            .size(LabelSize::Small)
            .single_line(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::ready_item_for_test;
    use fanta_doc::{CanvasNode, GroupNode, NodeData, VectorNode};
    use gpui::TestAppContext;
    use project::{FakeFs, Project};
    use settings::SettingsStore;
    use std::path::PathBuf;

    fn init_test(cx: &mut TestAppContext) {
        cx.update(|cx| {
            zlog::init_test();
            assets::Assets.load_test_fonts(cx);
            let settings_store = SettingsStore::test(cx);
            cx.set_global(settings_store);
            theme_settings::init(theme::LoadThemes::JustBase, cx);
            release_channel::init(semver::Version::new(0, 0, 0), cx);
            editor::init(cx);
        });
    }

    fn doc_with_collection() -> (Doc, VariableCollectionId, ModeId) {
        let mut doc = Doc::new();
        let operation = create_collection_operation(&doc);
        let (collection, mode) = match &operation {
            Operation::CreateVariableCollection { collection } => {
                (collection.id, collection.default_mode)
            }
            _ => panic!("expected create collection operation"),
        };
        doc.apply(operation).expect("create collection");
        (doc, collection, mode)
    }

    #[test]
    fn create_and_edit_variable_operations_preserve_type_and_mode_invariants() {
        let (mut doc, collection, mode) = doc_with_collection();
        let create = create_variable_operation(&doc, collection, VariableType::Color)
            .expect("create color variable");
        let variable = match &create {
            Operation::CreateVariable { variable } => variable.id,
            _ => panic!("expected create variable operation"),
        };
        doc.apply(create).expect("apply create variable");

        let set = set_variable_value_operation(
            &doc,
            variable,
            mode,
            VarValue::Color {
                value: FantaColor::WHITE,
            },
        )
        .expect("valid color value")
        .expect("changed value");
        doc.apply(set).expect("apply set value");
        assert_eq!(
            doc.variables.variables[&variable].values_by_mode[&mode],
            VarValue::Color {
                value: FantaColor::WHITE
            }
        );
        assert!(
            set_variable_value_operation(&doc, variable, mode, VarValue::Float { value: 1.0 })
                .is_err()
        );
        assert_eq!(
            doc.variables.collections[&collection].variable_order,
            vec![variable]
        );
    }

    #[test]
    fn rename_builders_cover_collection_mode_and_variable_names() {
        let (mut doc, collection, mode) = doc_with_collection();
        let create = create_variable_operation(&doc, collection, VariableType::String)
            .expect("create variable");
        let variable = match &create {
            Operation::CreateVariable { variable } => variable.id,
            _ => panic!("expected variable"),
        };
        doc.apply(create).expect("apply variable");

        for (target, expected) in [
            (
                VariableRenameTarget::Collection(collection),
                "Theme collection",
            ),
            (VariableRenameTarget::Mode { collection, mode }, "Dark mode"),
            (
                VariableRenameTarget::Variable(variable),
                "surface/background",
            ),
        ] {
            let operation = rename_operation(&doc, target, expected.into())
                .expect("valid rename")
                .expect("changed name");
            doc.apply(operation).expect("apply rename");
        }

        assert_eq!(
            doc.variables.collections[&collection].name,
            "Theme collection"
        );
        assert_eq!(
            doc.variables.collections[&collection].modes[0].name,
            "Dark mode"
        );
        assert_eq!(
            doc.variables.variables[&variable].name,
            "surface/background"
        );
    }

    #[test]
    fn project_page_and_parent_mode_scopes_build_in_inheritance_order() {
        let (mut doc, collection, default_mode) = doc_with_collection();
        let alternate_mode = ModeId::new();
        doc.apply(Operation::AddMode {
            collection,
            mode: Mode {
                id: alternate_mode,
                name: "Dark".into(),
            },
        })
        .expect("add alternate mode");
        let mut page = CanvasNode::new(NodeData::Group(GroupNode::default()));
        page.name = "Page 1".into();
        let page_id = page.id;
        doc.apply(Operation::create_node(page))
            .expect("create page");
        doc.add_page(page_id);
        let mut container = CanvasNode::new(NodeData::Group(GroupNode::default()));
        container.name = "Card".into();
        container.parent = Some(page_id);
        let container_id = container.id;
        doc.apply(Operation::create_node(container))
            .expect("create container");
        doc.selection.select_only(container_id);

        let project =
            set_active_mode_operation(&doc, ModeScope::Doc, collection, Some(default_mode))
                .expect("project mode")
                .expect("project change");
        doc.apply(project).expect("apply project mode");
        let page = set_active_mode_operation(
            &doc,
            ModeScope::Frame { node: page_id },
            collection,
            Some(alternate_mode),
        )
        .expect("page mode")
        .expect("page change");
        doc.apply(page).expect("apply page mode");

        let scopes = mode_scope_snapshots(&doc, collection);
        assert_eq!(scopes.len(), 3);
        assert_eq!(scopes[0].current, Some(default_mode));
        assert_eq!(scopes[1].current, Some(alternate_mode));
        assert_eq!(scopes[2].current, None);
        assert!(scopes[1].label.contains("Page 1"));
        assert!(scopes[2].label.contains("Card"));
    }

    #[test]
    fn live_value_previews_commit_as_one_undoable_operation() {
        let (mut doc, collection, mode) = doc_with_collection();
        let create = create_variable_operation(&doc, collection, VariableType::Float)
            .expect("create float variable");
        let variable = match &create {
            Operation::CreateVariable { variable } => variable.id,
            _ => panic!("expected create variable operation"),
        };
        doc.apply(create).expect("apply create variable");
        let cell = VariableCell {
            variable,
            mode,
            variable_type: VariableType::Float,
        };
        let baseline = doc.variables.variables[&variable]
            .values_by_mode
            .get(&mode)
            .cloned();

        assert!(
            preview_variable_value(&mut doc, cell, VarValue::Float { value: 12.0 })
                .expect("first preview")
        );
        assert!(
            preview_variable_value(&mut doc, cell, VarValue::Float { value: 24.0 })
                .expect("second preview")
        );
        assert_eq!(
            doc.variables.variables[&variable].values_by_mode[&mode],
            VarValue::Float { value: 24.0 }
        );

        assert!(restore_variable_value(&mut doc, cell, baseline));
        let operation =
            set_variable_value_operation(&doc, variable, mode, VarValue::Float { value: 24.0 })
                .expect("valid commit")
                .expect("changed commit");
        doc.apply(operation).expect("commit previewed value");
        assert_eq!(
            doc.variables.variables[&variable].values_by_mode[&mode],
            VarValue::Float { value: 24.0 }
        );

        assert!(doc.undo().expect("undo value edit"));
        assert_eq!(
            doc.variables.variables[&variable].values_by_mode[&mode],
            VarValue::Float { value: 0.0 }
        );
    }

    #[gpui::test]
    async fn invalid_final_text_rewinds_the_preview_and_invalidates_rendering(
        cx: &mut TestAppContext,
    ) {
        init_test(cx);
        let (mut doc, collection, mode) = doc_with_collection();
        let create = create_variable_operation(&doc, collection, VariableType::Float)
            .expect("create float variable");
        let variable = match &create {
            Operation::CreateVariable { variable } => variable.id,
            _ => panic!("expected create variable operation"),
        };
        doc.apply(create).expect("apply create variable");
        let baseline = doc.variables.variables[&variable]
            .values_by_mode
            .get(&mode)
            .cloned();
        let cell = VariableCell {
            variable,
            mode,
            variable_type: VariableType::Float,
        };

        let file_system = FakeFs::new(cx.executor());
        let roots: [&std::path::Path; 0] = [];
        let project = Project::test(file_system, roots, cx).await;
        let item = ready_item_for_test(&project, PathBuf::from("/tmp/Variables.fanta"), doc, cx);
        let workspace_item = item.clone();
        let workspace = cx
            .add_window(move |window, cx| FantaVariablesWorkspace::new(workspace_item, window, cx));

        workspace
            .update(cx, |workspace, window, cx| {
                workspace.editing_cell = Some(cell);
                workspace.value_edit_baseline = baseline.clone();
                workspace.suppress_editor_events = true;
                workspace.value_editor.update(cx, |editor, cx| {
                    editor.set_text("12", window, cx);
                });
                workspace.suppress_editor_events = false;
                workspace.preview_value_edit(cx);
            })
            .expect("preview a valid variable value");
        cx.run_until_parked();

        item.read_with(cx, |item, _| {
            let document = item.document().expect("ready document");
            assert_eq!(
                document.doc.variables.variables[&variable].values_by_mode[&mode],
                VarValue::Float { value: 12.0 }
            );
            assert!(item.is_dirty(), "a live preview marks the item dirty");
        });
        let preview_generation = item.read_with(cx, |item, _| {
            item.document().expect("ready document").render_generation()
        });

        workspace
            .update(cx, |workspace, window, cx| {
                workspace.suppress_editor_events = true;
                workspace.value_editor.update(cx, |editor, cx| {
                    editor.set_text("not-a-number", window, cx);
                });
                workspace.suppress_editor_events = false;
                workspace.finish_value_edit(cx);
            })
            .expect("finish the invalid variable value");
        cx.run_until_parked();

        item.read_with(cx, |item, _| {
            let document = item.document().expect("ready document");
            assert_eq!(
                document.doc.variables.variables[&variable].values_by_mode[&mode],
                VarValue::Float { value: 0.0 }
            );
            assert!(
                document.render_generation() > preview_generation,
                "rewinding an invalid final value must invalidate the rendered preview"
            );
            assert!(
                !item.is_dirty(),
                "an invalid final value restores the clean pre-preview state"
            );
        });
    }

    #[test]
    fn binding_builder_filters_types_and_unbind_bakes_the_resolved_value() {
        let (mut doc, collection, mode) = doc_with_collection();
        let create = create_variable_operation(&doc, collection, VariableType::Float)
            .expect("create float variable");
        let variable = match &create {
            Operation::CreateVariable { variable } => variable.id,
            _ => panic!("expected create variable operation"),
        };
        doc.apply(create).expect("apply create variable");
        let set =
            set_variable_value_operation(&doc, variable, mode, VarValue::Float { value: 0.4 })
                .expect("valid float")
                .expect("changed value");
        doc.apply(set).expect("apply value");

        let node = CanvasNode::new(NodeData::Vector(VectorNode::default()));
        let node_id = node.id;
        doc.apply(Operation::create_node(node))
            .expect("create node");
        let bind = variable_binding_operation(&doc, node_id, BoundProp::Opacity, Some(variable))
            .expect("valid binding")
            .expect("new binding");
        doc.apply(bind).expect("apply binding");
        let unbind = variable_binding_operation(&doc, node_id, BoundProp::Opacity, None)
            .expect("valid unbind")
            .expect("bound property");
        doc.apply(unbind).expect("apply unbind");
        let node = doc.scene.get(node_id).expect("node remains");
        assert!(node.bindings.is_empty());
        assert!((node.opacity.get() - 0.4).abs() < f32::EPSILON);
    }

    #[test]
    fn collection_snapshot_uses_explicit_order_then_stable_fallback() {
        let (mut doc, collection, _) = doc_with_collection();
        let first = VariableId::from_u128(1);
        let second = VariableId::from_u128(2);
        for (id, name) in [(first, "zeta"), (second, "alpha")] {
            doc.variables.variables.insert(
                id,
                Variable {
                    id,
                    collection,
                    name: name.into(),
                    ty: VariableType::String,
                    values_by_mode: BTreeMap::new(),
                    scopes: Vec::new(),
                },
            );
        }
        doc.variables
            .collections
            .get_mut(&collection)
            .expect("collection")
            .variable_order = vec![first];
        let snapshot = collection_snapshot(&doc, &doc.variables.collections[&collection]);
        assert_eq!(
            snapshot
                .variables
                .iter()
                .map(|row| row.id)
                .collect::<Vec<_>>(),
            vec![first, second]
        );
    }
}
