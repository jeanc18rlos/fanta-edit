use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;

use crate::document::{FigItem, FigItemEvent};
use crate::mode_overrides::mode_override_operation;
use crate::variable_binding::{
    bindable_properties, variable_binding_model, variable_binding_operation,
    variable_binding_options,
};
use fanta_doc::{
    BoundProp, Color as FantaColor, Doc, Mode, ModeId, ModeScope, NodeData, NodeId, Operation,
    VarValue, Variable, VariableCollection, VariableCollectionId, VariableId, VariableType,
};
use fanta_gpui::variables::{
    VariableKind, VariableModeValue, VariableRow, VariablesAction, VariablesBindingProperty,
    VariablesChoice, VariablesCollection, VariablesContextAction, VariablesContextData,
    VariablesGroup, VariablesLayerBindings, VariablesMode, VariablesModeScope, VariablesScreen,
    VariablesViewData,
};
use gpui::{
    App, Context, Entity, EventEmitter, FocusHandle, Focusable, IntoElement, Render, SharedString,
    Subscription, Window,
};
use ui::prelude::*;

const ALL_GROUPS: &str = "all";

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
    modes: Vec<Mode>,
    variables: Vec<VariableRowSnapshot>,
}

/// One bindable property of the selected layer. Holds no candidate list: the
/// menu's entries are built when it opens, because a UI kit has hundreds of
/// type-compatible variables and this snapshot is rebuilt on every selection
/// change.
#[derive(Debug, Clone)]
struct BindingRowSnapshot {
    prop: BoundProp,
    label: SharedString,
    current: Option<VariableId>,
    current_label: Option<SharedString>,
    has_choices: bool,
}

#[derive(Debug, Clone)]
struct BindingSnapshot {
    node: NodeId,
    node_name: SharedString,
    rows: Vec<BindingRowSnapshot>,
}

#[derive(Debug, Clone, Default)]
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
    selected_group: SharedString,
    new_variable_type: VariableType,
    error_message: Option<SharedString>,
    cached_snapshot: Option<Rc<VariablesSnapshot>>,
    projected_snapshot: Option<Rc<VariablesSnapshot>>,
    screen: Option<Entity<VariablesScreen>>,
    screen_subscription: Option<Subscription>,
    context_subscription: Option<Subscription>,
    #[cfg(test)]
    snapshot_builds: usize,
    #[cfg(test)]
    renders: usize,
    _subscriptions: Vec<Subscription>,
}

impl FantaVariablesWorkspace {
    pub fn new(item: Entity<FigItem>, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        let subscription = cx.subscribe(&item, |this: &mut Self, _, event: &FigItemEvent, cx| {
            if matches!(
                event,
                FigItemEvent::EditedTransient | FigItemEvent::TextSelectionChanged
            ) {
                return;
            }
            if matches!(
                event,
                FigItemEvent::StateChanged | FigItemEvent::SourceEditLockChanged
            ) {
                // A replaced or locked document must not receive an old editor's draft.
                this.screen = None;
                this.screen_subscription = None;
                this.context_subscription = None;
            }
            this.reconcile_selection(cx);
            this.invalidate_snapshot();
            cx.notify();
        });
        let selected_collection = item
            .read(cx)
            .doc()
            .and_then(|doc| doc.variables.collections.keys().next().copied());
        Self {
            item,
            focus_handle: cx.focus_handle(),
            selected_collection,
            selected_group: ALL_GROUPS.into(),
            new_variable_type: VariableType::Color,
            error_message: None,
            cached_snapshot: None,
            projected_snapshot: None,
            screen: None,
            screen_subscription: None,
            context_subscription: None,
            #[cfg(test)]
            snapshot_builds: 0,
            #[cfg(test)]
            renders: 0,
            _subscriptions: vec![subscription],
        }
    }

    fn reconcile_selection(&mut self, cx: &App) {
        let doc = self.item.read(cx).doc();
        if !self
            .selected_collection
            .is_some_and(|id| doc.is_some_and(|doc| doc.variables.collections.contains_key(&id)))
        {
            self.selected_collection =
                doc.and_then(|doc| doc.variables.collections.keys().next().copied());
            self.selected_group = ALL_GROUPS.into();
        }
    }

    fn handle_screen_action(&mut self, action: &VariablesAction, cx: &mut Context<Self>) {
        if self.item.read(cx).source_edit_locked() {
            self.error_message =
                Some("Finish editing the document source before changing variables.".into());
            cx.notify();
            return;
        }
        match action {
            VariablesAction::CollectionSelected { collection_id } => {
                if let Ok(id) = collection_id.parse() {
                    self.selected_collection = Some(id);
                    self.selected_group = ALL_GROUPS.into();
                    self.reconcile_selection(cx);
                    self.invalidate_snapshot();
                }
            }
            VariablesAction::GroupSelected { group_id } => {
                self.selected_group = group_id.clone();
                self.projected_snapshot = None;
            }
            VariablesAction::CreateCollectionRequested => self.create_collection(cx),
            VariablesAction::CreateVariableRequested => self.create_variable(cx),
            VariablesAction::CreateTypedVariableRequested { kind } => {
                self.new_variable_type = host_variable_type(*kind);
                self.create_variable(cx);
            }
            VariablesAction::AddModeRequested => self.add_mode(cx),
            VariablesAction::SearchQueryChanged { .. }
            | VariablesAction::SearchOptionsRequested
            | VariablesAction::ValueEditRequested { .. }
            | VariablesAction::VariableSettingsRequested { .. } => {}
            VariablesAction::HelpRequested => {
                self.error_message = Some("Create a collection, then add variables and modes. Use slash-separated names to create groups; edit a value or choose an alias from its menu.".into());
            }
            VariablesAction::ImportVariablesRequested => {
                self.error_message = Some(
                    "Open a Figma .fig file to import its variables with the document.".into(),
                );
            }
            VariablesAction::ColorEyedropperRequested { .. } => {
                self.error_message = Some("Screen color sampling is not available here yet; enter a hex color or use the picker.".into());
            }
            _ => {
                let result = self
                    .item
                    .read(cx)
                    .doc()
                    .ok_or("The document is not ready")
                    .and_then(|doc| screen_operation(doc, self.selected_collection, action));
                self.apply_built_operation(result, cx);
                // Echo the accepted value even when a rejected edit changed only presentation.
                self.projected_snapshot = None;
            }
        }
        cx.notify();
    }

    fn update_screen(
        &mut self,
        snapshot: &Rc<VariablesSnapshot>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<VariablesScreen> {
        if let Some(screen) = &self.screen
            && self
                .projected_snapshot
                .as_ref()
                .is_some_and(|old| Rc::ptr_eq(old, snapshot))
        {
            return screen.clone();
        }
        let name = self
            .item
            .read(cx)
            .abs_path()
            .file_stem()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Untitled".into());
        let data = screen_view_data(
            self.item.read(cx).doc(),
            snapshot,
            &self.selected_group,
            name.into(),
        );
        let screen = if let Some(screen) = &self.screen {
            screen.update(cx, |screen, cx| screen.set_view_data(data, cx));
            screen.clone()
        } else {
            let screen = cx.new(|cx| VariablesScreen::new("fanta-variables", data, window, cx));
            self.screen_subscription = Some(cx.subscribe(&screen, |this, _, action, cx| {
                this.handle_screen_action(action, cx)
            }));
            self.context_subscription = Some(cx.subscribe(
                &screen,
                |this, _, action: &VariablesContextAction, cx| {
                    this.handle_context_action(action, cx);
                },
            ));
            self.screen = Some(screen.clone());
            screen
        };
        let context = variables_context_data(self.item.read(cx).doc(), snapshot);
        screen.update(cx, |screen, cx| screen.set_context_data(context, cx));
        self.projected_snapshot = Some(snapshot.clone());
        screen
    }

    fn apply_operation(&mut self, operation: Operation, cx: &mut Context<Self>) -> bool {
        let result = self.item.update(cx, |item, cx| item.apply(operation, cx));
        // A failed apply can still have moved the document part of the way, so
        // both outcomes drop the cached table.
        self.invalidate_snapshot();
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
            self.selected_group = ALL_GROUPS.into();
            self.invalidate_snapshot();
            cx.notify();
        }
    }

    fn create_variable(&mut self, cx: &mut Context<Self>) {
        self.reconcile_selection(cx);
        if self.selected_collection.is_none() {
            self.create_collection(cx);
        }
        let Some(collection) = self.selected_collection else {
            return;
        };
        let operation = {
            let item = self.item.read(cx);
            let Some(doc) = item.doc() else {
                return;
            };
            create_variable_operation(doc, collection, self.new_variable_type).map(
                |mut operation| {
                    if let Operation::CreateVariable { variable } = &mut operation
                        && let Some(group) = self.selected_group.strip_prefix("group:")
                        && !group.is_empty()
                    {
                        variable.name = format!("{group}/{}", variable.name);
                    }
                    operation
                },
            )
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

    fn invalidate_snapshot(&mut self) {
        self.cached_snapshot = None;
    }

    /// The cached table model, rebuilt only after an invalidation. Callers get
    /// a handle rather than a clone: the selected collection can hold a
    /// thousand rows, and this is read from `render`.
    fn snapshot(&mut self, cx: &App) -> Rc<VariablesSnapshot> {
        if let Some(snapshot) = self.cached_snapshot.clone() {
            return snapshot;
        }
        let snapshot = Rc::new(match self.item.read(cx).doc() {
            Some(doc) => variables_snapshot(doc, self.selected_collection),
            None => VariablesSnapshot::default(),
        });
        #[cfg(test)]
        {
            self.snapshot_builds += 1;
        }
        self.cached_snapshot = Some(snapshot.clone());
        snapshot
    }

    fn handle_context_action(&mut self, action: &VariablesContextAction, cx: &mut Context<Self>) {
        if self.item.read(cx).source_edit_locked() {
            return;
        }
        match action {
            VariablesContextAction::ModeSelected {
                collection_id,
                scope_id,
                mode_id,
            } => {
                let Ok(collection) = collection_id.parse() else {
                    return;
                };
                if Some(collection) != self.selected_collection {
                    return;
                }
                let scope = self.item.read(cx).doc().and_then(|doc| {
                    mode_scope_snapshots(doc, collection)
                        .into_iter()
                        .find(|scope| mode_scope_id(&scope.scope) == *scope_id)
                });
                if let Some(scope) = scope {
                    let mode = match mode_id {
                        Some(id) => match id.parse() {
                            Ok(id) => Some(id),
                            Err(_) => return,
                        },
                        None => None,
                    };
                    self.set_mode_scope(scope.scope, collection, mode, cx);
                }
            }
            VariablesContextAction::BindingSelected {
                node_id,
                property_id,
                variable_id,
            } => {
                let Ok(node) = node_id.parse() else {
                    return;
                };
                let property = self
                    .item
                    .read(cx)
                    .doc()
                    .filter(|doc| doc.selection.anchor() == Some(node))
                    .and_then(|doc| binding_snapshot(doc))
                    .and_then(|binding| {
                        binding
                            .rows
                            .into_iter()
                            .find(|row| format!("{:?}", row.prop) == property_id.as_ref())
                    });
                if let Some(property) = property {
                    if let Some(id) = variable_id {
                        if let Ok(variable) = id.parse() {
                            self.bind_property(node, property.prop, variable, cx);
                        }
                    } else {
                        self.unbind_property(node, property.prop, cx);
                    }
                }
            }
        }
        cx.notify();
    }
}

impl Render for FantaVariablesWorkspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        #[cfg(test)]
        {
            self.renders += 1;
        }
        let snapshot = self.snapshot(cx);
        let screen = self.update_screen(&snapshot, window, cx);
        let mut root = v_flex()
            .key_context("FantaVariablesWorkspace")
            .track_focus(&self.focus_handle)
            .size_full()
            .overflow_hidden()
            .bg(cx.theme().colors().panel_background);
        if let Some(error) = &self.error_message {
            root = root.child(
                h_flex()
                    .flex_none()
                    .px_3()
                    .py_1()
                    .bg(cx.theme().status().error_background)
                    .child(Label::new(error.clone()).size(LabelSize::Small)),
            );
        }
        root.child(v_flex().flex_1().min_w_0().min_h_0().child(screen))
    }
}
impl EventEmitter<()> for FantaVariablesWorkspace {}
impl Focusable for FantaVariablesWorkspace {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.screen
            .as_ref()
            .map(|screen| screen.focus_handle(cx))
            .unwrap_or_else(|| self.focus_handle.clone())
    }
}

fn variables_snapshot(doc: &Doc, selected: Option<VariableCollectionId>) -> VariablesSnapshot {
    let mut variable_counts: BTreeMap<VariableCollectionId, usize> = BTreeMap::new();
    for variable in doc.variables.variables.values() {
        *variable_counts.entry(variable.collection).or_default() += 1;
    }
    let collections = doc
        .variables
        .collections
        .values()
        .map(|collection| CollectionSummary {
            id: collection.id,
            name: collection.name.clone().into(),
            variable_count: variable_counts
                .get(&collection.id)
                .copied()
                .unwrap_or_default(),
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
                current_label: model.current_label,
                has_choices: model.has_options,
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
    if let VarValue::Alias { variable: target } = &new_value {
        validate_alias(doc, variable_id, *target)?;
    } else if new_value.variable_type() != Some(variable.ty) {
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

fn editor_variable_value(value: Option<&VarValue>, variable_type: VariableType) -> String {
    match value {
        Some(VarValue::Color { value }) => value.to_hex(),
        Some(VarValue::Float { value }) => value.to_string(),
        Some(VarValue::String { value }) => value.clone(),
        Some(VarValue::Boolean { value }) => value.to_string(),
        _ => match default_variable_value(variable_type) {
            VarValue::Color { value } => value.to_hex(),
            VarValue::Float { value } => value.to_string(),
            VarValue::String { value } => value,
            VarValue::Boolean { value } => value.to_string(),
            VarValue::TextStyle { .. } | VarValue::Alias { .. } => String::new(),
        },
    }
}

fn mode_scope_id(scope: &ModeScope) -> SharedString {
    match scope {
        ModeScope::Doc => "project".into(),
        ModeScope::Frame { node } => node.to_string().into(),
    }
}

fn variables_context_data(doc: Option<&Doc>, snapshot: &VariablesSnapshot) -> VariablesContextData {
    let Some(doc) = doc else {
        return VariablesContextData::default();
    };
    let mode_scopes = snapshot
        .selected
        .as_ref()
        .map(|collection| {
            mode_scope_snapshots(doc, collection.id)
                .into_iter()
                .map(|scope| {
                    let mut choices = vec![VariablesChoice {
                        id: None,
                        label: match scope.scope {
                            ModeScope::Doc => "Collection default".into(),
                            ModeScope::Frame { .. } => "Inherit parent".into(),
                        },
                    }];
                    choices.extend(collection.modes.iter().map(|mode| VariablesChoice {
                        id: Some(mode.id.to_string().into()),
                        label: mode.name.clone().into(),
                    }));
                    VariablesModeScope {
                        id: mode_scope_id(&scope.scope),
                        label: scope.label,
                        selected: scope.current.map(|id| id.to_string().into()),
                        choices,
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    let bindings = snapshot
        .binding
        .as_ref()
        .map(|binding| VariablesLayerBindings {
            node_id: binding.node.to_string().into(),
            name: binding.node_name.clone(),
            properties: binding
                .rows
                .iter()
                .map(|row| {
                    let mut choices = Vec::new();
                    if row.current.is_some() {
                        choices.push(VariablesChoice {
                            id: None,
                            label: "Unbind".into(),
                        });
                    }
                    if row.has_choices {
                        choices.extend(variable_binding_options(doc, row.prop).into_iter().map(
                            |choice| VariablesChoice {
                                id: Some(choice.id.to_string().into()),
                                label: choice.label,
                            },
                        ));
                    }
                    let selected = row.current.map(|id| SharedString::from(id.to_string()));
                    if selected.is_some() && !choices.iter().any(|choice| choice.id == selected) {
                        choices.push(VariablesChoice {
                            id: selected.clone(),
                            label: row
                                .current_label
                                .clone()
                                .unwrap_or_else(|| "Missing variable".into()),
                        });
                    }
                    VariablesBindingProperty {
                        id: format!("{:?}", row.prop).into(),
                        label: row.label.clone(),
                        selected,
                        choices,
                    }
                })
                .collect(),
        });
    VariablesContextData {
        mode_scopes,
        bindings,
    }
}

fn host_variable_type(kind: VariableKind) -> VariableType {
    match kind {
        VariableKind::Color => VariableType::Color,
        VariableKind::Number => VariableType::Float,
        VariableKind::String => VariableType::String,
        VariableKind::Boolean => VariableType::Boolean,
    }
}

fn screen_variable_kind(kind: VariableType) -> Option<VariableKind> {
    match kind {
        VariableType::Color => Some(VariableKind::Color),
        VariableType::Float => Some(VariableKind::Number),
        VariableType::String => Some(VariableKind::String),
        VariableType::Boolean => Some(VariableKind::Boolean),
        VariableType::Typography => None,
    }
}

fn resolved_cell_value(doc: &Doc, variable: &Variable, mode: ModeId) -> Option<VarValue> {
    let mut modes = doc.active_modes.clone();
    modes.insert(variable.collection, mode);
    fanta_doc::resolve_bound_value(
        &doc.variables,
        &doc.scene,
        NodeId::from_u128(0),
        &modes,
        variable.id,
    )
    .map(|value| value.to_var_value())
}

fn screen_view_data(
    doc: Option<&Doc>,
    snapshot: &VariablesSnapshot,
    selected_group: &SharedString,
    document_name: SharedString,
) -> VariablesViewData {
    let selected = snapshot.selected.as_ref();
    let mut group_counts = BTreeMap::<String, usize>::new();
    let mut variables = Vec::new();
    if let (Some(doc), Some(collection)) = (doc, selected) {
        for row in &collection.variables {
            let Some(kind) = screen_variable_kind(row.variable_type) else {
                continue;
            };
            let group = row
                .name
                .rsplit_once('/')
                .map(|(group, _)| group)
                .unwrap_or("");
            let group_id = format!("group:{group}");
            *group_counts.entry(group.to_owned()).or_default() += 1;
            let Some(variable) = doc.variables.variables.get(&row.id) else {
                continue;
            };
            let values = collection.modes.iter().map(|mode| {
                let resolved = resolved_cell_value(doc, variable, mode.id);
                let mut value = VariableModeValue::new(
                    mode.id.to_string(),
                    editor_variable_value(resolved.as_ref(), row.variable_type),
                );
                if let Some(VarValue::Color { value: color }) = resolved {
                    value.color_hex = Some(color.to_hex().into());
                }
                if let Some(VarValue::Alias { variable }) = row.values.get(&mode.id) {
                    value.alias_id = Some(variable.to_string().into());
                }
                value
            });
            variables.push(VariableRow::new(
                row.id.to_string(),
                row.name.clone(),
                group_id,
                kind,
                values,
            ));
        }
    }
    let mut groups =
        vec![VariablesGroup::new(ALL_GROUPS, "All variables", variables.len()).aggregate()];
    groups.extend(group_counts.into_iter().map(|(name, count)| {
        VariablesGroup::new(
            format!("group:{name}"),
            if name.is_empty() {
                "Ungrouped".to_owned()
            } else {
                name
            },
            count,
        )
    }));
    let selected_group_id = groups
        .iter()
        .find(|group| group.id == *selected_group)
        .map(|group| group.id.clone())
        .unwrap_or_else(|| ALL_GROUPS.into());
    VariablesViewData {
        document_name,
        collections: snapshot
            .collections
            .iter()
            .map(|collection| {
                VariablesCollection::new(
                    collection.id.to_string(),
                    collection.name.clone(),
                    collection.variable_count,
                )
            })
            .collect(),
        selected_collection_id: selected
            .map(|collection| collection.id.to_string().into())
            .unwrap_or_default(),
        groups,
        selected_group_id,
        modes: selected
            .map(|collection| {
                let default = doc
                    .and_then(|doc| doc.variables.collections.get(&collection.id))
                    .map(|collection| collection.default_mode);
                let mut modes = collection.modes.clone();
                modes.sort_by_key(|mode| Some(mode.id) != default);
                modes
                    .into_iter()
                    .map(|mode| VariablesMode::new(mode.id.to_string(), mode.name))
                    .collect()
            })
            .unwrap_or_default(),
        variables,
    }
}

fn screen_operation(
    doc: &Doc,
    selected: Option<VariableCollectionId>,
    action: &VariablesAction,
) -> Result<Option<Operation>, &'static str> {
    let collection = || selected.ok_or("Select a collection first");
    let variable_id = |id: &SharedString| -> Result<VariableId, &'static str> {
        let id = id.parse().map_err(|_| "Invalid variable identifier")?;
        let variable = doc
            .variables
            .variables
            .get(&id)
            .ok_or("The variable no longer exists")?;
        if Some(variable.collection) != selected {
            return Err("The selected collection has changed");
        }
        Ok(id)
    };
    let mode_id = |id: &SharedString| id.parse::<ModeId>().map_err(|_| "Invalid mode identifier");
    let nonempty_name = |name: &SharedString| {
        let name = name.trim();
        if name.is_empty() {
            Err("Names cannot be empty")
        } else {
            Ok(name.to_owned())
        }
    };
    match action {
        VariablesAction::CollectionRenameRequested {
            collection_id,
            name,
        } => {
            let id = collection_id
                .parse()
                .map_err(|_| "Invalid collection identifier")?;
            rename_operation(
                doc,
                VariableRenameTarget::Collection(id),
                nonempty_name(name)?,
            )
        }
        VariablesAction::VariableRenameRequested {
            variable_id: id,
            name,
        } => rename_operation(
            doc,
            VariableRenameTarget::Variable(variable_id(id)?),
            nonempty_name(name)?,
        ),
        VariablesAction::ModeRenameRequested { mode_id: id, name } => rename_operation(
            doc,
            VariableRenameTarget::Mode {
                collection: collection()?,
                mode: mode_id(id)?,
            },
            nonempty_name(name)?,
        ),
        VariablesAction::ValueChanged {
            variable_id: id,
            mode_id: mode,
            value,
        } => {
            let id = variable_id(id)?;
            let variable = doc
                .variables
                .variables
                .get(&id)
                .ok_or("The variable no longer exists")?;
            set_variable_value_operation(
                doc,
                id,
                mode_id(mode)?,
                parse_primitive_value(variable.ty, value)?,
            )
        }
        VariablesAction::AliasChanged {
            variable_id: id,
            mode_id: mode,
            alias_id,
        } => {
            let id = variable_id(id)?;
            let mode = mode_id(mode)?;
            let source = doc
                .variables
                .variables
                .get(&id)
                .ok_or("The variable no longer exists")?;
            let value = if let Some(alias) = alias_id {
                let target = alias.parse().map_err(|_| "Invalid alias identifier")?;
                validate_alias(doc, id, target)?;
                VarValue::Alias { variable: target }
            } else {
                resolved_cell_value(doc, source, mode)
                    .ok_or("The alias cannot be resolved; choose a literal value instead")?
            };
            set_variable_value_operation(doc, id, mode, value)
        }
        VariablesAction::DescriptionChanged { .. } => {
            Err("Variable descriptions are not supported by the document format yet")
        }
        _ => Ok(None),
    }
}

fn validate_alias(doc: &Doc, source: VariableId, target: VariableId) -> Result<(), &'static str> {
    let source_type = doc
        .variables
        .variables
        .get(&source)
        .ok_or("The variable no longer exists")?
        .ty;
    let mut pending = vec![target];
    let mut visited = BTreeSet::new();
    while let Some(id) = pending.pop() {
        if id == source {
            return Err("Variable aliases cannot form a cycle");
        }
        if !visited.insert(id) {
            continue;
        }
        let variable = doc
            .variables
            .variables
            .get(&id)
            .ok_or("The alias target no longer exists")?;
        if variable.ty != source_type {
            return Err("The alias must have the same variable type");
        }
        pending.extend(
            variable
                .values_by_mode
                .values()
                .filter_map(|value| match value {
                    VarValue::Alias { variable } => Some(*variable),
                    _ => None,
                }),
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::ready_item_for_test;
    use fanta_doc::{CanvasNode, GroupNode, NodeData, VectorNode};
    use gpui::{TestAppContext, VisualTestContext};
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
            gpui_component::init(cx);
            fanta_gpui::init(cx);
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

    fn doc_with_many_variables(count: usize) -> (Doc, VariableCollectionId, ModeId) {
        let (mut doc, collection, mode) = doc_with_collection();
        let mut order = Vec::with_capacity(count);
        for index in 0..count {
            let id = VariableId::from_u128(index as u128 + 1);
            doc.variables.variables.insert(
                id,
                Variable {
                    id,
                    collection,
                    name: format!("color/{index}"),
                    ty: VariableType::Color,
                    values_by_mode: BTreeMap::from([(
                        mode,
                        VarValue::Color {
                            value: FantaColor::BLACK,
                        },
                    )]),
                    scopes: Vec::new(),
                },
            );
            order.push(id);
        }
        doc.variables
            .collections
            .get_mut(&collection)
            .expect("the collection exists")
            .variable_order = order;
        (doc, collection, mode)
    }

    async fn workspace_for_doc(
        doc: Doc,
        cx: &mut TestAppContext,
    ) -> (
        Entity<FantaVariablesWorkspace>,
        Entity<FigItem>,
        VisualTestContext,
    ) {
        init_test(cx);
        let file_system = FakeFs::new(cx.executor());
        let roots: [&std::path::Path; 0] = [];
        let project = Project::test(file_system, roots, cx).await;
        let item = ready_item_for_test(&project, PathBuf::from("/tmp/Variables.fanta"), doc, cx);
        let workspace_item = item.clone();
        let (workspace, cx) = cx.add_window_view(move |window, cx| {
            FantaVariablesWorkspace::new(workspace_item, window, cx)
        });
        cx.run_until_parked();
        (workspace, item, cx.clone())
    }

    #[test]
    fn shared_screen_projection_preserves_groups_modes_and_aliases() {
        let (mut doc, collection, mode) = doc_with_many_variables(3);
        let first = VariableId::from_u128(1);
        let second = VariableId::from_u128(2);
        doc.variables
            .variables
            .get_mut(&second)
            .unwrap()
            .values_by_mode
            .insert(mode, VarValue::Alias { variable: first });
        let snapshot = variables_snapshot(&doc, Some(collection));
        let data = screen_view_data(Some(&doc), &snapshot, &ALL_GROUPS.into(), "Design".into());
        assert_eq!(data.variables.len(), 3);
        assert_eq!(data.groups.len(), 2);
        assert_eq!(data.modes[0].id.as_ref(), mode.to_string());
        assert_eq!(
            data.variables[1].values[0].alias_id.as_deref(),
            Some(first.to_string().as_str())
        );
        assert!(data.variables[1].values[0].color_hex.is_some());
        assert_eq!(data.variables[0].group_id.as_ref(), "group:color");
    }

    #[test]
    fn shared_screen_intents_validate_aliases_and_preserve_undo() {
        let (mut doc, collection, mode) = doc_with_many_variables(3);
        let first = VariableId::from_u128(1);
        let second = VariableId::from_u128(2);
        let alias = VariablesAction::AliasChanged {
            variable_id: second.to_string().into(),
            mode_id: mode.to_string().into(),
            alias_id: Some(first.to_string().into()),
        };
        let operation = screen_operation(&doc, Some(collection), &alias)
            .unwrap()
            .unwrap();
        doc.apply(operation).unwrap();
        let cycle = VariablesAction::AliasChanged {
            variable_id: first.to_string().into(),
            mode_id: mode.to_string().into(),
            alias_id: Some(second.to_string().into()),
        };
        assert!(screen_operation(&doc, Some(collection), &cycle).is_err());
        let unlink = VariablesAction::AliasChanged {
            variable_id: second.to_string().into(),
            mode_id: mode.to_string().into(),
            alias_id: None,
        };
        doc.apply(
            screen_operation(&doc, Some(collection), &unlink)
                .unwrap()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(
            doc.variables.variables[&second].values_by_mode[&mode],
            VarValue::Color {
                value: FantaColor::BLACK
            }
        );
        doc.undo().unwrap();
        assert_eq!(
            doc.variables.variables[&second].values_by_mode[&mode],
            VarValue::Alias { variable: first }
        );
        let stale = VariablesAction::ValueChanged {
            variable_id: first.to_string().into(),
            mode_id: ModeId::new().to_string().into(),
            value: "#ffffff".into(),
        };
        assert!(screen_operation(&doc, Some(collection), &stale).is_err());
    }

    #[gpui::test]
    async fn shared_screen_events_update_document_and_reuse_snapshot(cx: &mut TestAppContext) {
        let (doc, collection, mode) = doc_with_many_variables(3);
        let variable = VariableId::from_u128(1);
        let (workspace, item, mut cx) = workspace_for_doc(doc, cx).await;
        let cx = &mut cx;
        let screen = workspace.read_with(cx, |workspace, _| {
            workspace.screen.clone().expect("shared screen mounted")
        });
        let builds = workspace.read_with(cx, |workspace, _| workspace.snapshot_builds);
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
        assert_eq!(
            workspace.read_with(cx, |workspace, _| workspace.snapshot_builds),
            builds
        );
        screen.update(cx, |_, cx| {
            cx.emit(VariablesAction::ValueChanged {
                variable_id: variable.to_string().into(),
                mode_id: mode.to_string().into(),
                value: "#ffffff".into(),
            })
        });
        cx.run_until_parked();
        item.read_with(cx, |item, _| {
            assert_eq!(
                item.doc().unwrap().variables.variables[&variable].values_by_mode[&mode],
                VarValue::Color {
                    value: FantaColor::WHITE
                }
            )
        });
        screen.update(cx, |_, cx| {
            cx.emit(VariablesAction::CreateTypedVariableRequested {
                kind: VariableKind::Boolean,
            })
        });
        cx.run_until_parked();
        item.read_with(cx, |item, _| {
            let doc = item.doc().unwrap();
            assert!(
                doc.variables
                    .variables
                    .values()
                    .any(|variable| variable.collection == collection
                        && variable.ty == VariableType::Boolean)
            );
        });
        assert!(workspace.read_with(cx, |workspace, _| workspace.error_message.is_none()));
    }
    #[gpui::test]
    async fn first_variable_creates_a_collection_and_mode(cx: &mut TestAppContext) {
        let (workspace, item, mut cx) = workspace_for_doc(Doc::new(), cx).await;
        let screen = workspace.read_with(&cx, |workspace, _| workspace.screen.clone().unwrap());
        screen.update(&mut cx, |_, cx| {
            cx.emit(VariablesAction::CreateTypedVariableRequested {
                kind: VariableKind::Color,
            })
        });
        cx.run_until_parked();
        item.read_with(&cx, |item, _| {
            let doc = item.doc().unwrap();
            assert_eq!(doc.variables.collections.len(), 1);
            assert_eq!(doc.variables.variables.len(), 1);
            let variable = doc.variables.variables.values().next().unwrap();
            let collection = &doc.variables.collections[&variable.collection];
            assert_eq!(collection.modes.len(), 1);
            assert!(
                variable
                    .values_by_mode
                    .contains_key(&collection.default_mode)
            );
        });
    }
    #[gpui::test]
    async fn shared_context_events_apply_modes_and_layer_bindings(cx: &mut TestAppContext) {
        let (mut doc, collection, mode) = doc_with_many_variables(1);
        let variable = VariableId::from_u128(1);
        let node = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
            0.,
            0.,
            10.,
            10.,
            FantaColor::BLACK,
        )));
        let node_id = node.id;
        doc.apply(Operation::create_node(node)).unwrap();
        doc.selection.replace_with(vec![node_id]);
        let (workspace, item, mut cx) = workspace_for_doc(doc, cx).await;
        let screen = workspace.read_with(&cx, |workspace, _| workspace.screen.clone().unwrap());
        screen.update(&mut cx, |_, cx| {
            cx.emit(VariablesContextAction::ModeSelected {
                collection_id: collection.to_string().into(),
                scope_id: "project".into(),
                mode_id: Some(mode.to_string().into()),
            })
        });
        cx.run_until_parked();
        screen.update(&mut cx, |_, cx| {
            cx.emit(VariablesContextAction::BindingSelected {
                node_id: node_id.to_string().into(),
                property_id: format!("{:?}", BoundProp::FillColor { index: 0 }).into(),
                variable_id: Some(variable.to_string().into()),
            })
        });
        cx.run_until_parked();
        item.read_with(&cx, |item, _| {
            let doc = item.doc().unwrap();
            assert_eq!(doc.active_modes.get(&collection), Some(&mode));
            assert_eq!(
                doc.scene
                    .get(node_id)
                    .unwrap()
                    .bindings
                    .get(&BoundProp::FillColor { index: 0 }),
                Some(&variable)
            );
        });
        screen.update(&mut cx, |_, cx| {
            cx.emit(VariablesContextAction::BindingSelected {
                node_id: node_id.to_string().into(),
                property_id: format!("{:?}", BoundProp::FillColor { index: 0 }).into(),
                variable_id: None,
            })
        });
        cx.run_until_parked();
        item.read_with(&cx, |item, _| {
            assert!(
                !item
                    .doc()
                    .unwrap()
                    .scene
                    .get(node_id)
                    .unwrap()
                    .bindings
                    .contains_key(&BoundProp::FillColor { index: 0 })
            )
        });
    }
}
