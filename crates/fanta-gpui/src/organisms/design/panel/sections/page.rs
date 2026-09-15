//! Standalone projection, controller, and rendering for the Page inspector.

use std::collections::HashSet;

use super::super::*;

mod controller;

pub(in super::super) use controller::PagePanelController;

#[derive(Clone)]
pub(in super::super) struct PageProjection {
    id: SharedString,
    permissions: DesignPanelPermissions,
    page: Option<DesignPageViewData>,
    page_local_styles: Option<DesignPageLocalStylesViewData>,
    variable_modes: Option<DesignVariableModeViewData>,
    preferences: PagePreferencesProjection,
    overlays: PageOverlayProjection,
    collapsed_local_style_folders: HashSet<SharedString>,
}

#[derive(Clone)]
struct PagePreferencesProjection {
    variables_entry_point: DesignVariablesEntryPoint,
}

#[derive(Clone, Copy)]
struct PageOverlayProjection {
    page_resource_browser: Option<DesignLocalResourceCategory>,
    variable_mode_browser_open: bool,
}

#[derive(Clone)]
pub(in super::super) struct PageContextProjection {
    selection_kind: DesignPanelSelectionKind,
    permissions: DesignPanelPermissions,
    selected_node_id: SharedString,
}

impl PageContextProjection {
    pub(in super::super) fn new(
        selection_kind: DesignPanelSelectionKind,
        permissions: DesignPanelPermissions,
        selected_node_id: SharedString,
    ) -> Self {
        Self {
            selection_kind,
            permissions,
            selected_node_id,
        }
    }
}

#[derive(Clone)]
pub(in super::super) struct PageHostProjection {
    page: Option<DesignPageViewData>,
    page_local_styles: Option<DesignPageLocalStylesViewData>,
    variable_modes: Option<DesignVariableModeViewData>,
}

impl PageHostProjection {
    pub(in super::super) fn new(
        page: Option<DesignPageViewData>,
        page_local_styles: Option<DesignPageLocalStylesViewData>,
        variable_modes: Option<DesignVariableModeViewData>,
    ) -> Self {
        Self {
            page,
            page_local_styles,
            variable_modes,
        }
    }
}

#[derive(Clone)]
pub(in super::super) struct PagePresentationProjection {
    variables_entry_point: DesignVariablesEntryPoint,
    page_resource_browser: Option<DesignLocalResourceCategory>,
    variable_mode_browser_open: bool,
    collapsed_local_style_folders: HashSet<SharedString>,
}

impl PagePresentationProjection {
    pub(in super::super) fn new(
        variables_entry_point: DesignVariablesEntryPoint,
        page_resource_browser: Option<DesignLocalResourceCategory>,
        variable_mode_browser_open: bool,
        collapsed_local_style_folders: HashSet<SharedString>,
    ) -> Self {
        Self {
            variables_entry_point,
            page_resource_browser,
            variable_mode_browser_open,
            collapsed_local_style_folders,
        }
    }
}

impl PageProjection {
    pub(in super::super) fn new(
        id: SharedString,
        context: PageContextProjection,
        host: PageHostProjection,
        presentation: PagePresentationProjection,
    ) -> Self {
        let page = (context.selection_kind == DesignPanelSelectionKind::None)
            .then_some(host.page)
            .flatten();
        let page_target = page.as_ref().map(|page| DesignPanelTarget::Page {
            page_id: page.page_id.clone(),
        });
        let page_local_styles = host.page_local_styles.filter(|view_data| {
            page_target.as_ref() == Some(&view_data.target) && view_data.is_valid()
        });
        let variable_target = match context.selection_kind {
            DesignPanelSelectionKind::None => page_target,
            DesignPanelSelectionKind::Single => Some(DesignPanelTarget::Nodes {
                node_ids: vec![context.selected_node_id],
            }),
            DesignPanelSelectionKind::Multiple => None,
        };
        let variable_modes = host
            .variable_modes
            .filter(|view_data| Some(&view_data.target) == variable_target.as_ref());

        Self {
            id,
            permissions: context.permissions,
            page,
            page_local_styles,
            variable_modes,
            preferences: PagePreferencesProjection {
                variables_entry_point: presentation.variables_entry_point,
            },
            overlays: PageOverlayProjection {
                page_resource_browser: presentation.page_resource_browser,
                variable_mode_browser_open: presentation.variable_mode_browser_open,
            },
            collapsed_local_style_folders: presentation.collapsed_local_style_folders,
        }
    }

    pub(in super::super) fn page_view_data_for_context(&self) -> Option<&DesignPageViewData> {
        self.page.as_ref()
    }

    pub(in super::super) fn page_local_styles_view_data_for_context(
        &self,
    ) -> Option<&DesignPageLocalStylesViewData> {
        self.page_local_styles.as_ref()
    }

    pub(in super::super) fn can_edit_page(&self) -> bool {
        self.permissions.can_edit() && self.page.is_some()
    }

    pub(in super::super) fn variable_mode_view_data_for_context(
        &self,
    ) -> Option<&DesignVariableModeViewData> {
        self.variable_modes.as_ref()
    }

    pub(in super::super) fn can_edit_variable_modes(&self) -> bool {
        self.permissions.can_edit() && self.variable_modes.is_some()
    }

    pub(in super::super) fn local_style_target_is_current(
        &self,
        target: &DesignLocalStyleTarget,
    ) -> bool {
        self.page_local_styles
            .as_ref()
            .and_then(|view_data| view_data.resolve_style(target))
            .is_some()
    }

    pub(in super::super) fn local_style_target_is_enabled(
        &self,
        target: &DesignLocalStyleTarget,
    ) -> bool {
        self.local_style_target_is_current(target)
            && self.page_local_styles.as_ref().is_some_and(|view_data| {
                view_data.entry_path_is_enabled(target.kind, target.style_id.as_ref())
            })
    }

    pub(in super::super) fn local_style_folder_is_enabled(
        view_data: &DesignPageLocalStylesViewData,
        kind: DesignLocalStyleKind,
        folder_id: Option<&str>,
    ) -> bool {
        folder_id.is_none_or(|folder_id| {
            view_data.folder(kind, folder_id).is_some()
                && view_data.entry_path_is_enabled(kind, folder_id)
        })
    }

    pub(in super::super) fn local_style_insertion_is_current(
        &self,
        insertion: &DesignLocalStyleInsertion,
    ) -> bool {
        let Some(view_data) = self.page_local_styles.as_ref() else {
            return false;
        };
        if !Self::local_style_folder_is_enabled(
            view_data,
            insertion.kind,
            insertion.parent_folder_id.as_ref().map(|id| id.as_ref()),
        ) {
            return false;
        }
        let Some(entries) = view_data.entries(
            insertion.kind,
            insertion.parent_folder_id.as_ref().map(|id| id.as_ref()),
        ) else {
            return false;
        };
        if insertion.expected_index > entries.len() {
            return false;
        }
        let before = insertion
            .expected_index
            .checked_sub(1)
            .and_then(|index| entries.get(index))
            .map(DesignLocalStyleEntry::id);
        let after = entries
            .get(insertion.expected_index)
            .map(DesignLocalStyleEntry::id);
        before == insertion.expected_before_id.as_ref()
            && after == insertion.expected_after_id.as_ref()
    }

    pub(in super::super) fn local_style_targets_are_current(
        &self,
        targets: &[DesignLocalStyleTarget],
        allow_empty: bool,
    ) -> bool {
        if targets.is_empty() {
            return allow_empty;
        }
        let unique = targets.iter().collect::<HashSet<_>>();
        unique.len() == targets.len()
            && targets
                .iter()
                .all(|target| self.local_style_target_is_enabled(target))
    }
}

pub(in super::super) fn local_resource_browse_action(
    projection: &PageProjection,
    category: DesignLocalResourceCategory,
) -> Option<DesignPanelAction> {
    let page_id = projection.page.as_ref()?.page_id.clone();
    Some(DesignPanelAction::LocalResourceBrowseRequested { page_id, category })
}

pub(in super::super) fn local_resource_open_action(
    projection: &PageProjection,
    resource: DesignLocalResourceSelection,
) -> Option<DesignPanelAction> {
    let page = projection.page.as_ref()?;
    page.local_resources
        .resource(&resource)
        .is_some_and(|resource| resource.availability.can_open())
        .then(|| DesignPanelAction::LocalResourceOpenRequested {
            page_id: page.page_id.clone(),
            resource,
        })
}

pub(in super::super) fn local_resource_create_action(
    projection: &PageProjection,
    kind: DesignLocalResourceKind,
) -> Option<DesignPanelAction> {
    projection
        .can_edit_page()
        .then(|| DesignPanelAction::LocalResourceCreateRequested {
            page_id: projection
                .page
                .as_ref()
                .expect("editable Page projection has Page data")
                .page_id
                .clone(),
            kind,
        })
}

pub(in super::super) fn local_resource_import_action(
    projection: &PageProjection,
    resource: DesignLocalResourceSelection,
) -> Option<DesignPanelAction> {
    let page = projection.page.as_ref()?;
    (projection.can_edit_page()
        && matches!(&resource.source, DesignLocalResourceSource::Library { .. })
        && page
            .local_resources
            .resource(&resource)
            .is_some_and(|resource| resource.availability.can_import()))
    .then(|| DesignPanelAction::LocalResourceImportRequested {
        page_id: page.page_id.clone(),
        resource,
    })
}

pub(in super::super) fn local_style_command_action(
    projection: &PageProjection,
    target: DesignLocalStyleTarget,
    command: DesignLocalStyleCommand,
) -> Option<DesignPanelAction> {
    let permission_allows = match command {
        DesignLocalStyleCommand::Edit | DesignLocalStyleCommand::Duplicate => {
            projection.can_edit_page()
        }
        DesignLocalStyleCommand::Copy => projection.permissions.can_copy(),
        DesignLocalStyleCommand::GoToDefinition => true,
    };
    (permission_allows && projection.local_style_target_is_enabled(&target))
        .then_some(DesignPanelAction::LocalStyleCommandRequested { target, command })
}

pub(in super::super) fn local_style_create_action(
    projection: &PageProjection,
    kind: DesignLocalStyleKind,
    parent_folder_id: Option<SharedString>,
) -> Option<DesignPanelAction> {
    let view_data = projection.page_local_styles.as_ref()?;
    let page_id = view_data.page_id()?.clone();
    (projection.can_edit_page()
        && view_data.create_disabled_reason.is_none()
        && parent_folder_id.as_ref().is_none_or(|folder_id| {
            PageProjection::local_style_folder_is_enabled(view_data, kind, Some(folder_id.as_ref()))
        }))
    .then_some(DesignPanelAction::LocalStyleCreateRequested {
        page_id,
        kind,
        parent_folder_id,
    })
}

pub(in super::super) fn local_style_folder_create_action(
    projection: &PageProjection,
    kind: DesignLocalStyleKind,
    selected: Vec<DesignLocalStyleTarget>,
) -> Option<DesignPanelAction> {
    let view_data = projection.page_local_styles.as_ref()?;
    let page_id = view_data.page_id()?.clone();
    (projection.can_edit_page()
        && view_data.create_disabled_reason.is_none()
        && projection.local_style_targets_are_current(&selected, true)
        && selected
            .iter()
            .all(|target| target.page_id == page_id && target.kind == kind))
    .then_some(DesignPanelAction::LocalStyleFolderCreateRequested {
        page_id,
        kind,
        selected,
    })
}

pub(in super::super) fn local_styles_delete_action(
    projection: &PageProjection,
    targets: Vec<DesignLocalStyleTarget>,
) -> Option<DesignPanelAction> {
    (projection.can_edit_page() && projection.local_style_targets_are_current(&targets, false))
        .then_some(DesignPanelAction::LocalStylesDeleteRequested { targets })
}

pub(in super::super) fn local_styles_move_action(
    projection: &PageProjection,
    targets: Vec<DesignLocalStyleTarget>,
    destination: DesignLocalStyleInsertion,
) -> Option<DesignPanelAction> {
    (projection.can_edit_page()
        && projection.local_style_targets_are_current(&targets, false)
        && targets.iter().all(|target| target.kind == destination.kind)
        && projection.local_style_insertion_is_current(&destination))
    .then_some(DesignPanelAction::LocalStylesMoveRequested {
        targets,
        destination,
    })
}

pub(in super::super) fn variables_view_open_action(
    projection: &PageProjection,
) -> Option<DesignPanelAction> {
    let DesignVariablesEntryPoint::LegacyRightSidebar { disabled_reason } =
        &projection.preferences.variables_entry_point
    else {
        return None;
    };
    let page = projection.page.as_ref()?;
    disabled_reason
        .is_none()
        .then(|| DesignPanelAction::VariablesViewOpenRequested {
            page_id: page.page_id.clone(),
        })
}

pub(in super::super) fn variable_mode_apply_action(
    projection: &PageProjection,
    collection_id: SharedString,
    mode_id: SharedString,
) -> Option<DesignPanelAction> {
    let view_data = projection.variable_modes.as_ref()?;
    let collection = view_data.collection(collection_id.as_ref())?;
    (projection.can_edit_variable_modes()
        && collection.mode(mode_id.as_ref()).is_some()
        && collection.mode_disabled_reason(mode_id.as_ref()).is_none()
        && collection.explicit_mode_id.as_ref() != Some(&mode_id))
    .then(|| DesignPanelAction::VariableModeApplyRequested {
        target: view_data.target.clone(),
        collection_id,
        mode_id,
    })
}

pub(in super::super) fn variable_mode_clear_action(
    projection: &PageProjection,
    collection_id: SharedString,
    explicit_mode_id: SharedString,
) -> Option<DesignPanelAction> {
    let view_data = projection.variable_modes.as_ref()?;
    let collection = view_data.collection(collection_id.as_ref())?;
    (projection.can_edit_variable_modes()
        && collection.disabled_reason.is_none()
        && collection.explicit_mode_id.as_ref() == Some(&explicit_mode_id))
    .then(|| DesignPanelAction::VariableModeClearRequested {
        target: view_data.target.clone(),
        collection_id,
        explicit_mode_id,
    })
}

pub(in super::super) struct PageChrome<'a> {
    background_picker: Option<AnyElement>,
    paint_swatch: &'a dyn Fn(&DesignPaint, &mut Context<DesignPanel>) -> AnyElement,
}

impl<'a> PageChrome<'a> {
    pub(in super::super) fn new(
        background_picker: Option<AnyElement>,
        paint_swatch: &'a dyn Fn(&DesignPaint, &mut Context<DesignPanel>) -> AnyElement,
    ) -> Self {
        Self {
            background_picker,
            paint_swatch,
        }
    }

    fn paint_swatch(&self, paint: &DesignPaint, cx: &mut Context<DesignPanel>) -> AnyElement {
        (self.paint_swatch)(paint, cx)
    }

    fn take_background_picker(&mut self) -> AnyElement {
        self.background_picker
            .take()
            .expect("a projected Page owns its background picker chrome")
    }
}

#[derive(Clone)]
pub(in super::super) struct PageEventSink {
    panel: Entity<DesignPanel>,
}

impl PageEventSink {
    pub(in super::super) fn new(panel: Entity<DesignPanel>) -> Self {
        Self { panel }
    }

    fn dispatch(&self, event: PageEvent, cx: &mut App) {
        self.panel
            .update(cx, |panel, cx| dispatch(panel, event, cx));
    }

    /// Dispatches from a `cx.listener` that already holds the panel update.
    /// Re-entering the entity through `self.panel.update` from that callback
    /// would attempt a nested mutable update and panic.
    fn dispatch_in_context(
        panel: &mut DesignPanel,
        event: PageEvent,
        cx: &mut Context<DesignPanel>,
    ) {
        dispatch(panel, event, cx);
    }

    fn dispatch_overlay(&self, event: PageOverlayEvent, window: &mut Window, cx: &mut App) {
        self.panel
            .update(cx, |panel, cx| dispatch_overlay(panel, event, window, cx));
    }
}

#[derive(Clone)]
enum PageEvent {
    ToggleResourceBrowser(DesignLocalResourceCategory),
    OpenResource(DesignLocalResourceSelection),
    CreateResource(DesignLocalResourceKind),
    ImportResource(DesignLocalResourceSelection),
    SetVariableModeBrowserOpen(bool),
    ApplyVariableMode {
        collection_id: SharedString,
        mode_id: SharedString,
    },
    ClearVariableMode {
        collection_id: SharedString,
        explicit_mode_id: SharedString,
    },
    ToggleLocalStyleFolder(SharedString),
    LocalStyleCommand {
        target: DesignLocalStyleTarget,
        command: DesignLocalStyleCommand,
    },
    CreateLocalStyle {
        kind: DesignLocalStyleKind,
        parent_folder_id: Option<SharedString>,
    },
    CreateLocalStyleFolder {
        kind: DesignLocalStyleKind,
        selected: Vec<DesignLocalStyleTarget>,
    },
    DeleteLocalStyles(Vec<DesignLocalStyleTarget>),
    MoveLocalStyles {
        targets: Vec<DesignLocalStyleTarget>,
        destination: DesignLocalStyleInsertion,
    },
    OpenVariables,
}

#[derive(Clone, Copy)]
enum PageOverlayEvent {
    ResourceBrowserChanged {
        category: DesignLocalResourceCategory,
        open: bool,
    },
    VariableModeBrowserChanged(bool),
}

fn dispatch(panel: &mut DesignPanel, event: PageEvent, cx: &mut Context<DesignPanel>) {
    match event {
        PageEvent::ToggleResourceBrowser(category) => {
            if panel.overlays.page_resource_browser() == Some(category) {
                panel.close_page_resource_browser(category, cx);
            } else {
                panel.emit_local_resource_browse(category, cx);
            }
        }
        PageEvent::OpenResource(resource) => panel.emit_local_resource_open(resource, cx),
        PageEvent::CreateResource(kind) => panel.emit_local_resource_create(kind, cx),
        PageEvent::ImportResource(resource) => panel.emit_local_resource_import(resource, cx),
        PageEvent::SetVariableModeBrowserOpen(open) => {
            panel.set_variable_mode_browser_open(open, cx);
        }
        PageEvent::ApplyVariableMode {
            collection_id,
            mode_id,
        } => panel.emit_variable_mode_apply(collection_id, mode_id, cx),
        PageEvent::ClearVariableMode {
            collection_id,
            explicit_mode_id,
        } => panel.emit_variable_mode_clear(collection_id, explicit_mode_id, cx),
        PageEvent::ToggleLocalStyleFolder(folder_id) => {
            panel.toggle_local_style_folder(folder_id, cx);
        }
        PageEvent::LocalStyleCommand { target, command } => {
            panel.emit_local_style_command(target, command, cx);
        }
        PageEvent::CreateLocalStyle {
            kind,
            parent_folder_id,
        } => panel.emit_local_style_create(kind, parent_folder_id, cx),
        PageEvent::CreateLocalStyleFolder { kind, selected } => {
            panel.emit_local_style_folder_create(kind, selected, cx);
        }
        PageEvent::DeleteLocalStyles(targets) => panel.emit_local_styles_delete(targets, cx),
        PageEvent::MoveLocalStyles {
            targets,
            destination,
        } => panel.emit_local_styles_move(targets, destination, cx),
        PageEvent::OpenVariables => panel.emit_variables_view_open(cx),
    }
}

fn dispatch_overlay(
    panel: &mut DesignPanel,
    event: PageOverlayEvent,
    window: &mut Window,
    cx: &mut Context<DesignPanel>,
) {
    match event {
        PageOverlayEvent::ResourceBrowserChanged { category, open } => {
            if open {
                panel.remember_overlay_focus_return(DesignOpenOverlay::PageResource, window, cx);
                panel.emit_local_resource_browse(category, cx);
            } else if panel.overlays.page_resource_browser() == Some(category) {
                let _ = panel.dismiss_overlay_from_outside_click(
                    DesignOpenOverlay::PageResource,
                    window,
                    cx,
                );
            }
        }
        PageOverlayEvent::VariableModeBrowserChanged(open) => {
            if open {
                panel.remember_overlay_focus_return(DesignOpenOverlay::VariableMode, window, cx);
                panel.set_variable_mode_browser_open(true, cx);
            } else if panel.overlays.variable_mode_browser_open() {
                let _ = panel.dismiss_overlay_from_outside_click(
                    DesignOpenOverlay::VariableMode,
                    window,
                    cx,
                );
            }
        }
    }
}

#[allow(dead_code)]
pub(in super::super) fn render_page_resource_browser(
    projection: &PageProjection,
    events: PageEventSink,
    category: DesignLocalResourceCategory,
    _cx: &mut Context<DesignPanel>,
) -> AnyElement {
    let events_for_open = events.clone();
    let events_for_content = events;
    let panel_id = projection.id.clone();
    let groups = projection
        .page_view_data_for_context()
        .map(|page| page.local_resources.groups.clone())
        .unwrap_or_default();
    let has_page = projection.page_view_data_for_context().is_some();
    let can_edit = projection.can_edit_page();
    let trigger = Button::new(SharedString::from(format!(
        "{}-page-resource-{:?}",
        projection.id, category
    )))
    .label("Browse")
    .tooltip(if has_page {
        SharedString::from(format!("Browse {}", category.label()))
    } else {
        "Page data unavailable".into()
    })
    .xsmall()
    .compact()
    .ghost()
    .disabled(!has_page)
    .on_keyboard_activate({
        let events = events_for_open.clone();
        move |_, cx| {
            events.dispatch(PageEvent::ToggleResourceBrowser(category), cx);
        }
    });

    Popover::new(SharedString::from(format!(
        "{}-page-resource-popover-{:?}",
        projection.id, category
    )))
    .anchor(Anchor::TopRight)
    .open(projection.overlays.page_resource_browser == Some(category))
    .overlay_closable(true)
    .on_open_change(move |open, window, cx| {
        events_for_open.dispatch_overlay(
            PageOverlayEvent::ResourceBrowserChanged {
                category,
                open: *open,
            },
            window,
            cx,
        );
    })
    .trigger(trigger)
    .content(move |_, window, cx| {
        let mut content = v_flex()
            .w(popup_width(window, 320.))
            .max_h(popup_height(window, 440.))
            .gap_1()
            .p_2()
            .child(
                h_flex()
                    .w_full()
                    .justify_between()
                    .gap_2()
                    .child(div().text_sm().font_semibold().child(category.label()))
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child("This file + libraries"),
                    ),
            );
        let mut resource_count = 0_usize;
        for group in &groups {
            let resources = group
                .resources
                .iter()
                .filter(|resource| resource.kind.category() == category)
                .cloned()
                .collect::<Vec<_>>();
            if resources.is_empty() {
                continue;
            }
            resource_count += resources.len();
            content = content.child(
                h_flex()
                    .h(px(24.))
                    .mt_1()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .truncate()
                            .text_xs()
                            .font_semibold()
                            .child(group.name.clone()),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(SharedString::from(group.source.label().to_owned())),
                    ),
            );
            for resource in resources {
                let selection = group.selection(resource.id.clone());
                let row_id = format!("{panel_id}-page-resource-{}-{}", group.id, resource.id);
                let action: AnyElement = match &resource.availability {
                    DesignLocalResourceAvailability::Imported => {
                        let events = events_for_content.clone();
                        let selection = selection.clone();
                        Button::new(SharedString::from(format!("{row_id}-open")))
                            .label("Open")
                            .tooltip(SharedString::from(format!(
                                "Open {}",
                                resource.kind.label()
                            )))
                            .xsmall()
                            .compact()
                            .ghost()
                            .on_activate(move |_, _, cx| {
                                events.dispatch(PageEvent::OpenResource(selection.clone()), cx);
                            })
                            .into_any_element()
                    }
                    DesignLocalResourceAvailability::Available => {
                        let events = events_for_content.clone();
                        let selection = selection.clone();
                        Button::new(SharedString::from(format!("{row_id}-import")))
                            .label("Import")
                            .tooltip(if can_edit {
                                SharedString::from(format!("Import {}", resource.kind.label()))
                            } else {
                                "View only".into()
                            })
                            .xsmall()
                            .compact()
                            .ghost()
                            .disabled(!can_edit)
                            .on_activate(move |_, _, cx| {
                                events.dispatch(PageEvent::ImportResource(selection.clone()), cx);
                            })
                            .into_any_element()
                    }
                    DesignLocalResourceAvailability::Unavailable { reason } => {
                        Button::new(SharedString::from(format!("{row_id}-unavailable")))
                            .label("Unavailable")
                            .tooltip(reason.clone())
                            .xsmall()
                            .compact()
                            .ghost()
                            .disabled(true)
                            .into_any_element()
                    }
                };
                content = content.child(
                    h_flex()
                        .h(px(ROW_HEIGHT))
                        .w_full()
                        .gap_2()
                        .px_1()
                        .child(
                            v_flex()
                                .flex_1()
                                .min_w(px(0.))
                                .child(div().truncate().text_xs().child(resource.name.clone()))
                                .child(
                                    div()
                                        .truncate()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(resource.kind.label()),
                                ),
                        )
                        .child(action),
                );
            }
        }
        if resource_count == 0 {
            content = content.child(
                div()
                    .py_3()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child("No resources supplied by the host"),
            );
        }
        let create_kinds: &[DesignLocalResourceKind] = match category {
            DesignLocalResourceCategory::Styles => &[
                DesignLocalResourceKind::PaintStyle,
                DesignLocalResourceKind::TextStyle,
                DesignLocalResourceKind::EffectStyle,
                DesignLocalResourceKind::GridStyle,
            ],
            DesignLocalResourceCategory::VariableCollections => {
                &[DesignLocalResourceKind::VariableCollection]
            }
        };
        let mut create_row = h_flex()
            .w_full()
            .flex_wrap()
            .gap_1()
            .pt_2()
            .border_t_1()
            .border_color(cx.theme().border);
        for kind in create_kinds {
            let events = events_for_content.clone();
            let kind = *kind;
            create_row = create_row.child(
                Button::new(SharedString::from(format!(
                    "{panel_id}-create-page-resource-{kind:?}"
                )))
                .label(SharedString::from(format!("+ {}", kind.label())))
                .tooltip(if can_edit {
                    SharedString::from(format!("Create {}", kind.label()))
                } else {
                    "View only".into()
                })
                .xsmall()
                .compact()
                .ghost()
                .disabled(!can_edit)
                .on_activate(move |_, _, cx| {
                    events.dispatch(PageEvent::CreateResource(kind), cx);
                }),
            );
        }
        content.child(create_row)
    })
    .into_any_element()
}

pub(in super::super) fn render_variable_mode_popover(
    projection: &PageProjection,
    events: PageEventSink,
    _cx: &mut Context<DesignPanel>,
) -> Option<AnyElement> {
    let view_data = projection.variable_mode_view_data_for_context()?.clone();
    if view_data.collections.is_empty() {
        return None;
    }
    let events_for_open = events.clone();
    let events_for_content = events;
    let panel_id = projection.id.clone();
    let can_edit = projection.can_edit_variable_modes();
    let explicit_count = view_data
        .collections
        .iter()
        .filter(|collection| collection.explicit_mode_id.is_some())
        .count();
    let trigger = Button::new(SharedString::from(format!(
        "{}-variable-modes-trigger",
        projection.id
    )))
    .label(if explicit_count == 0 {
        "Modes".into()
    } else {
        SharedString::from(format!("Modes {explicit_count}"))
    })
    .tooltip("Variable modes")
    .xsmall()
    .compact()
    .ghost()
    .selected(projection.overlays.variable_mode_browser_open || explicit_count > 0)
    .on_keyboard_activate({
        let events = events_for_open.clone();
        let open = projection.overlays.variable_mode_browser_open;
        move |_, cx| {
            events.dispatch(PageEvent::SetVariableModeBrowserOpen(!open), cx);
        }
    });

    Some(
        Popover::new(SharedString::from(format!(
            "{}-variable-modes-popover",
            projection.id
        )))
        .anchor(Anchor::TopRight)
        .open(projection.overlays.variable_mode_browser_open)
        .overlay_closable(true)
        .on_open_change(move |open, window, cx| {
            events_for_open.dispatch_overlay(
                PageOverlayEvent::VariableModeBrowserChanged(*open),
                window,
                cx,
            );
        })
        .trigger(trigger)
        .content(move |_, window, cx| {
            let mut content = v_flex()
                .w(popup_width(window, 300.))
                .max_h(popup_height(window, 440.))
                .gap_2()
                .p_2()
                .child(
                    h_flex()
                        .w_full()
                        .justify_between()
                        .child(div().text_sm().font_semibold().child("Variable modes"))
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child("Explicit overrides"),
                        ),
                );
            for collection in &view_data.collections {
                let resolved_name = collection
                    .resolved_mode()
                    .map_or_else(|| "Unknown".into(), |mode| mode.name.clone());
                let status = if let Some(mode) = collection.explicit_mode() {
                    SharedString::from(format!("Explicit · {}", mode.name))
                } else {
                    SharedString::from(format!("Resolved · {resolved_name}"))
                };
                content = content.child(
                    v_flex()
                        .w_full()
                        .gap_1()
                        .p_2()
                        .rounded(px(6.))
                        .border_1()
                        .border_color(cx.theme().border)
                        .child(
                            h_flex()
                                .w_full()
                                .justify_between()
                                .gap_2()
                                .child(
                                    v_flex()
                                        .flex_1()
                                        .min_w(px(0.))
                                        .child(
                                            div()
                                                .truncate()
                                                .text_xs()
                                                .font_semibold()
                                                .child(collection.name.clone()),
                                        )
                                        .child(
                                            div()
                                                .truncate()
                                                .text_xs()
                                                .text_color(cx.theme().muted_foreground)
                                                .child(SharedString::from(
                                                    collection.source.label().to_owned(),
                                                )),
                                        ),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(status),
                                ),
                        )
                        .children(collection.modes.iter().map(|mode| {
                            let events = events_for_content.clone();
                            let collection_id = collection.id.clone();
                            let mode_id = mode.id.clone();
                            let disabled_reason = if !can_edit {
                                Some(SharedString::from("View only"))
                            } else {
                                collection.mode_disabled_reason(mode.id.as_ref())
                            };
                            let label = if mode.id == collection.default_mode_id {
                                SharedString::from(format!("{} · default", mode.name))
                            } else {
                                mode.name.clone()
                            };
                            Button::new(SharedString::from(format!(
                                "{panel_id}-variable-mode-{}-{}",
                                collection.id, mode.id
                            )))
                            .label(label)
                            .tooltip(
                                disabled_reason
                                    .clone()
                                    .unwrap_or_else(|| "Set explicit mode".into()),
                            )
                            .xsmall()
                            .compact()
                            .ghost()
                            .w_full()
                            .selected(mode.id == collection.resolved_mode_id)
                            .disabled(disabled_reason.is_some())
                            .on_activate(move |_, _, cx| {
                                events.dispatch(
                                    PageEvent::ApplyVariableMode {
                                        collection_id: collection_id.clone(),
                                        mode_id: mode_id.clone(),
                                    },
                                    cx,
                                );
                            })
                        }))
                        .when_some(collection.explicit_mode_id.clone(), |card, mode_id| {
                            let events = events_for_content.clone();
                            let collection_id = collection.id.clone();
                            let disabled_reason = if !can_edit {
                                Some(SharedString::from("View only"))
                            } else {
                                collection.disabled_reason.clone()
                            };
                            card.child(
                                Button::new(SharedString::from(format!(
                                    "{panel_id}-clear-variable-mode-{}",
                                    collection.id
                                )))
                                .label("Use inherited mode")
                                .tooltip(
                                    disabled_reason
                                        .clone()
                                        .unwrap_or_else(|| "Clear explicit mode".into()),
                                )
                                .xsmall()
                                .compact()
                                .ghost()
                                .w_full()
                                .disabled(disabled_reason.is_some())
                                .on_activate(move |_, _, cx| {
                                    events.dispatch(
                                        PageEvent::ClearVariableMode {
                                            collection_id: collection_id.clone(),
                                            explicit_mode_id: mode_id.clone(),
                                        },
                                        cx,
                                    );
                                }),
                            )
                        }),
                );
            }
            content
        })
        .into_any_element(),
    )
}

fn render_local_style_preview(
    chrome: &PageChrome<'_>,
    preview: &DesignLocalStylePreview,
    cx: &mut Context<DesignPanel>,
) -> AnyElement {
    match preview {
        DesignLocalStylePreview::Text(style) => v_flex()
            .w(px(40.))
            .flex_none()
            .items_center()
            .justify_center()
            .rounded(px(3.))
            .bg(cx.theme().secondary)
            .child(
                div()
                    .text_xs()
                    .font_semibold()
                    .child(SharedString::from(format!(
                        "{} {}",
                        style.family, style.size
                    ))),
            )
            .into_any_element(),
        DesignLocalStylePreview::Color(paints) => {
            let mut swatches = h_flex().w(px(40.)).flex_none().gap(px(2.));
            if paints.is_empty() {
                swatches = swatches.child(
                    div()
                        .size(px(14.))
                        .rounded(px(2.))
                        .border_1()
                        .border_color(cx.theme().border),
                );
            } else {
                for paint in paints.iter().take(3) {
                    swatches = swatches.child(chrome.paint_swatch(paint, cx));
                }
            }
            swatches.into_any_element()
        }
        DesignLocalStylePreview::Effect(effects) => {
            let summary = effects
                .first()
                .map_or("No effects", |effect| effect.kind.label());
            h_flex()
                .w(px(40.))
                .flex_none()
                .gap_1()
                .child(
                    div()
                        .size(px(18.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(3.))
                        .bg(cx.theme().secondary)
                        .text_xs()
                        .child("fx"),
                )
                .child(
                    div()
                        .min_w(px(0.))
                        .truncate()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(summary),
                )
                .into_any_element()
        }
        DesignLocalStylePreview::LayoutGuide(grids) => {
            let first = grids.first();
            h_flex()
                .w(px(40.))
                .flex_none()
                .gap_1()
                .child(
                    div()
                        .size(px(18.))
                        .rounded(px(3.))
                        .border_1()
                        .border_color(cx.theme().border)
                        .bg(first.map_or(cx.theme().secondary, |grid| color_hsla(grid.color))),
                )
                .child(
                    div()
                        .min_w(px(0.))
                        .truncate()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(first.map_or("No guides", |grid| grid.kind().label())),
                )
                .into_any_element()
        }
    }
}

pub(in super::super) fn local_style_insertion(
    kind: DesignLocalStyleKind,
    parent_folder_id: Option<SharedString>,
    entries: &[DesignLocalStyleEntry],
    expected_index: usize,
) -> Option<DesignLocalStyleInsertion> {
    if expected_index > entries.len() {
        return None;
    }
    Some(DesignLocalStyleInsertion::new(
        kind,
        parent_folder_id,
        expected_index,
        expected_index
            .checked_sub(1)
            .and_then(|index| entries.get(index))
            .map(|entry| entry.id().clone()),
        entries.get(expected_index).map(|entry| entry.id().clone()),
    ))
}

#[allow(clippy::too_many_arguments)]
fn render_local_style_entries(
    projection: &PageProjection,
    chrome: &PageChrome<'_>,
    events: &PageEventSink,
    page_id: &SharedString,
    kind: DesignLocalStyleKind,
    parent_folder_id: Option<SharedString>,
    entries: &[DesignLocalStyleEntry],
    depth: usize,
    cx: &mut Context<DesignPanel>,
) -> AnyElement {
    let mut list = v_flex().w_full();
    for (index, entry) in entries.iter().enumerate() {
        match entry {
            DesignLocalStyleEntry::Folder {
                id,
                name,
                disabled_reason,
                entries: children,
            } => {
                let folder_id = id.clone();
                let events_for_create = events.clone();
                let events_for_toggle = events.clone();
                let collapsed = projection.collapsed_local_style_folders.contains(id);
                let folder_enabled = projection
                    .page_local_styles_view_data_for_context()
                    .is_some_and(|view_data| view_data.entry_path_is_enabled(kind, id.as_ref()));
                let can_create = projection.can_edit_page()
                    && folder_enabled
                    && projection
                        .page_local_styles_view_data_for_context()
                        .is_some_and(|view_data| view_data.create_disabled_reason.is_none());
                let tooltip = disabled_reason
                    .clone()
                    .or_else(|| {
                        (!projection.can_edit_page()).then(|| SharedString::from("View only"))
                    })
                    .unwrap_or_else(|| "Create style in folder".into());
                let create_selector =
                    format!("{}-local-style-folder-{}-create", projection.id, folder_id);
                let toggle_selector =
                    format!("{}-local-style-folder-{}-toggle", projection.id, folder_id);
                let folder_row_selector =
                    format!("{}-local-style-folder-{}", projection.id, folder_id);
                let toggle_folder_id = folder_id.clone();
                let folder_destination =
                    local_style_insertion(kind, Some(folder_id.clone()), children, children.len())
                        .expect("folder-end insertion is in bounds");
                let can_drop_to_folder = projection.can_edit_page() && folder_enabled;
                let drop_background = cx.theme().selection.opacity(0.18);
                let drop_border = cx.theme().selection;
                let folder_drop_listener =
                    cx.listener(move |panel, drag: &LocalStyleDrag, _, cx| {
                        cx.stop_propagation();
                        PageEventSink::dispatch_in_context(
                            panel,
                            PageEvent::MoveLocalStyles {
                                targets: vec![drag.target.clone()],
                                destination: folder_destination.clone(),
                            },
                            cx,
                        );
                    });
                let folder_row = h_flex()
                    .id(SharedString::from(folder_row_selector.clone()))
                    .debug_selector(move || folder_row_selector.clone())
                    .h(px(28.))
                    .w_full()
                    .pl(px((depth as f32) * 12.))
                    .gap_1()
                    .child(
                        Button::new(SharedString::from(toggle_selector.clone()))
                            .debug_selector(move || toggle_selector.clone())
                            .label(name.clone())
                            .icon(if collapsed {
                                IconName::ChevronRight
                            } else {
                                IconName::ChevronDown
                            })
                            .tooltip(if collapsed {
                                "Expand style folder"
                            } else {
                                "Collapse style folder"
                            })
                            .xsmall()
                            .compact()
                            .ghost()
                            .flex_1()
                            .min_w(px(0.))
                            .on_activate(move |_, _, cx| {
                                events_for_toggle.dispatch(
                                    PageEvent::ToggleLocalStyleFolder(toggle_folder_id.clone()),
                                    cx,
                                );
                            }),
                    )
                    .child(
                        Button::new(SharedString::from(create_selector.clone()))
                            .debug_selector(move || create_selector.clone())
                            .icon(IconName::Plus)
                            .tooltip(tooltip)
                            .xsmall()
                            .compact()
                            .ghost()
                            .w(px(22.))
                            .h(px(22.))
                            .disabled(!can_create)
                            .on_activate(move |_, _, cx| {
                                events_for_create.dispatch(
                                    PageEvent::CreateLocalStyle {
                                        kind,
                                        parent_folder_id: Some(folder_id.clone()),
                                    },
                                    cx,
                                );
                            }),
                    )
                    .when(can_drop_to_folder, move |row| {
                        row.can_drop(move |candidate, _, _| {
                            candidate
                                .downcast_ref::<LocalStyleDrag>()
                                .is_some_and(|drag| drag.target.kind == kind)
                        })
                        .drag_over::<LocalStyleDrag>(move |style, drag, _, _| {
                            if drag.target.kind == kind {
                                style.bg(drop_background).border_color(drop_border)
                            } else {
                                style
                            }
                        })
                        .on_drop(folder_drop_listener)
                    });
                list = list.child(folder_row);
                if !collapsed {
                    list = list.child(render_local_style_entries(
                        projection,
                        chrome,
                        events,
                        page_id,
                        kind,
                        Some(id.clone()),
                        children,
                        depth + 1,
                        cx,
                    ));
                }
            }
            DesignLocalStyleEntry::Style(style) => {
                let target = DesignLocalStyleTarget::new(
                    page_id.clone(),
                    kind,
                    style.id.clone(),
                    parent_folder_id.clone(),
                    index,
                );
                let enabled = projection
                    .page_local_styles_view_data_for_context()
                    .is_some_and(|view_data| {
                        view_data.entry_path_is_enabled(kind, style.id.as_ref())
                    });
                let can_edit = projection.can_edit_page() && enabled;
                let can_copy = projection.permissions.can_copy() && enabled;
                let can_go_to = enabled;
                let disabled_tooltip = style
                    .disabled_reason
                    .clone()
                    .or_else(|| {
                        (!enabled).then(|| SharedString::from("Containing folder is unavailable"))
                    })
                    .unwrap_or_else(|| "View only".into());
                let mut commands = h_flex().w_full().justify_end().gap_1();
                for (command, icon, allowed) in [
                    (
                        DesignLocalStyleCommand::GoToDefinition,
                        IconName::ArrowRight,
                        can_go_to,
                    ),
                    (DesignLocalStyleCommand::Copy, IconName::File, can_copy),
                    (DesignLocalStyleCommand::Edit, IconName::Settings2, can_edit),
                    (DesignLocalStyleCommand::Duplicate, IconName::Plus, can_edit),
                ] {
                    let events = events.clone();
                    let command_target = target.clone();
                    let command_selector =
                        format!("{}-local-style-{}-{:?}", projection.id, style.id, command);
                    commands = commands.child(
                        Button::new(SharedString::from(command_selector.clone()))
                            .debug_selector(move || command_selector.clone())
                            .icon(icon)
                            .tooltip(if allowed {
                                command.label().into()
                            } else {
                                disabled_tooltip.clone()
                            })
                            .xsmall()
                            .compact()
                            .ghost()
                            .w(px(22.))
                            .h(px(22.))
                            .disabled(!allowed)
                            .on_activate(move |_, _, cx| {
                                events.dispatch(
                                    PageEvent::LocalStyleCommand {
                                        target: command_target.clone(),
                                        command,
                                    },
                                    cx,
                                );
                            }),
                    );
                }

                let events_for_delete = events.clone();
                let delete_target = target.clone();
                let delete_selector = format!("{}-local-style-{}-delete", projection.id, style.id);
                commands = commands.child(
                    Button::new(SharedString::from(delete_selector.clone()))
                        .debug_selector(move || delete_selector.clone())
                        .icon(IconName::Minus)
                        .tooltip(if can_edit {
                            "Delete style".into()
                        } else {
                            disabled_tooltip.clone()
                        })
                        .xsmall()
                        .compact()
                        .ghost()
                        .w(px(22.))
                        .h(px(22.))
                        .disabled(!can_edit)
                        .on_activate(move |_, _, cx| {
                            events_for_delete.dispatch(
                                PageEvent::DeleteLocalStyles(vec![delete_target.clone()]),
                                cx,
                            );
                        }),
                );

                if index > 0
                    && let Some(destination) =
                        local_style_insertion(kind, parent_folder_id.clone(), entries, index - 1)
                {
                    let events_for_move = events.clone();
                    let move_target = target.clone();
                    let move_selector =
                        format!("{}-local-style-{}-move-up", projection.id, style.id);
                    commands = commands.child(
                        Button::new(SharedString::from(move_selector.clone()))
                            .debug_selector(move || move_selector.clone())
                            .icon(IconName::ChevronUp)
                            .tooltip(if can_edit {
                                "Move style up".into()
                            } else {
                                disabled_tooltip.clone()
                            })
                            .xsmall()
                            .compact()
                            .ghost()
                            .w(px(22.))
                            .h(px(22.))
                            .disabled(!can_edit)
                            .on_activate(move |_, _, cx| {
                                events_for_move.dispatch(
                                    PageEvent::MoveLocalStyles {
                                        targets: vec![move_target.clone()],
                                        destination: destination.clone(),
                                    },
                                    cx,
                                );
                            }),
                    );
                }
                if index + 1 < entries.len()
                    && let Some(destination) =
                        local_style_insertion(kind, parent_folder_id.clone(), entries, index + 2)
                {
                    let events_for_move = events.clone();
                    let move_target = target.clone();
                    let move_selector =
                        format!("{}-local-style-{}-move-down", projection.id, style.id);
                    commands = commands.child(
                        Button::new(SharedString::from(move_selector.clone()))
                            .debug_selector(move || move_selector.clone())
                            .icon(IconName::ChevronDown)
                            .tooltip(if can_edit {
                                "Move style down".into()
                            } else {
                                disabled_tooltip.clone()
                            })
                            .xsmall()
                            .compact()
                            .ghost()
                            .w(px(22.))
                            .h(px(22.))
                            .disabled(!can_edit)
                            .on_activate(move |_, _, cx| {
                                events_for_move.dispatch(
                                    PageEvent::MoveLocalStyles {
                                        targets: vec![move_target.clone()],
                                        destination: destination.clone(),
                                    },
                                    cx,
                                );
                            }),
                    );
                }

                let style_row_selector = format!("{}-local-style-{}", projection.id, style.id);
                let drag = LocalStyleDrag {
                    target: target.clone(),
                    label: style.name.clone(),
                };
                let drop_style_id = style.id.clone();
                let style_destination =
                    local_style_insertion(kind, parent_folder_id.clone(), entries, index)
                        .expect("style-before insertion is in bounds");
                let drop_background = cx.theme().selection.opacity(0.18);
                let drop_border = cx.theme().selection;
                let style_drop_listener =
                    cx.listener(move |panel, drag: &LocalStyleDrag, _, cx| {
                        cx.stop_propagation();
                        PageEventSink::dispatch_in_context(
                            panel,
                            PageEvent::MoveLocalStyles {
                                targets: vec![drag.target.clone()],
                                destination: style_destination.clone(),
                            },
                            cx,
                        );
                    });
                let style_row = v_flex()
                    .id(SharedString::from(style_row_selector.clone()))
                    .debug_selector(move || style_row_selector.clone())
                    .w_full()
                    .pl(px((depth as f32) * 12.))
                    .py_1()
                    .gap_1()
                    .child(
                        h_flex()
                            .w_full()
                            .gap_2()
                            .child(render_local_style_preview(chrome, &style.preview, cx))
                            .child(
                                v_flex()
                                    .flex_1()
                                    .min_w(px(0.))
                                    .child(
                                        div()
                                            .truncate()
                                            .text_xs()
                                            .text_color(if enabled {
                                                cx.theme().foreground
                                            } else {
                                                cx.theme().muted_foreground
                                            })
                                            .child(style.name.clone()),
                                    )
                                    .when(!style.description.is_empty(), |details| {
                                        details.child(
                                            div()
                                                .truncate()
                                                .text_xs()
                                                .text_color(cx.theme().muted_foreground)
                                                .child(style.description.clone()),
                                        )
                                    }),
                            ),
                    )
                    .child(commands)
                    .when(can_edit, move |row| {
                        row.cursor_move().on_drag(drag, |drag, _, _, cx| {
                            cx.new(|_| LocalStyleDragPreview { drag: drag.clone() })
                        })
                    })
                    .when(projection.can_edit_page() && enabled, move |row| {
                        let can_drop_style_id = drop_style_id.clone();
                        let style_id_for_highlight = drop_style_id.clone();
                        row.can_drop(move |candidate, _, _| {
                            candidate
                                .downcast_ref::<LocalStyleDrag>()
                                .is_some_and(|drag| {
                                    drag.target.kind == kind
                                        && drag.target.style_id != can_drop_style_id
                                })
                        })
                        .drag_over::<LocalStyleDrag>(move |style, drag, _, _| {
                            if drag.target.kind == kind
                                && drag.target.style_id != style_id_for_highlight
                            {
                                style.bg(drop_background).border_color(drop_border)
                            } else {
                                style
                            }
                        })
                        .on_drop(style_drop_listener)
                    });
                list = list.child(style_row);
            }
        }
    }
    list.into_any_element()
}

fn render_page_local_styles(
    projection: &PageProjection,
    chrome: &PageChrome<'_>,
    events: &PageEventSink,
    view_data: &DesignPageLocalStylesViewData,
    cx: &mut Context<DesignPanel>,
) -> AnyElement {
    let page_id = view_data
        .page_id()
        .expect("context-filtered Page local styles");
    let can_create = projection.can_edit_page() && view_data.create_disabled_reason.is_none();
    let create_tooltip = view_data
        .create_disabled_reason
        .clone()
        .or_else(|| (!projection.can_edit_page()).then(|| SharedString::from("View only")));
    let mut content = v_flex()
        .id(SharedString::from(format!(
            "{}-local-styles",
            projection.id
        )))
        .w_full()
        .gap_1()
        .pt_1()
        .border_t_1()
        .border_color(cx.theme().sidebar_border)
        .child(
            h_flex().h(px(32.)).w_full().child(
                div()
                    .flex_1()
                    .text_xs()
                    .font_semibold()
                    .child("Local styles"),
            ),
        );
    for kind in DesignLocalStyleKind::ALL {
        let entries = view_data
            .section(kind)
            .map_or(&[][..], |section| section.entries.as_slice());
        let events_for_style = events.clone();
        let events_for_folder = events.clone();
        let style_selector = format!("{}-local-style-{:?}-create", projection.id, kind);
        let folder_selector = format!("{}-local-style-{:?}-folder-create", projection.id, kind);
        let tooltip = create_tooltip
            .clone()
            .unwrap_or_else(|| SharedString::from(format!("Create {}", kind.label())));
        let root_selector = format!("{}-local-style-{:?}-root-drop", projection.id, kind);
        let root_destination = local_style_insertion(kind, None, entries, entries.len())
            .expect("root-end insertion is in bounds");
        let root_drop_background = cx.theme().selection.opacity(0.18);
        let root_drop_border = cx.theme().selection;
        let root_drop_listener = cx.listener(move |panel, drag: &LocalStyleDrag, _, cx| {
            cx.stop_propagation();
            PageEventSink::dispatch_in_context(
                panel,
                PageEvent::MoveLocalStyles {
                    targets: vec![drag.target.clone()],
                    destination: root_destination.clone(),
                },
                cx,
            );
        });
        let family_header = h_flex()
            .id(SharedString::from(root_selector.clone()))
            .debug_selector(move || root_selector.clone())
            .h(px(28.))
            .w_full()
            .gap_1()
            .child(div().flex_1().text_xs().font_semibold().child(kind.label()))
            .child(
                Button::new(SharedString::from(folder_selector.clone()))
                    .debug_selector(move || folder_selector.clone())
                    .label("Folder")
                    .tooltip(
                        create_tooltip
                            .clone()
                            .unwrap_or_else(|| "Create style folder".into()),
                    )
                    .xsmall()
                    .compact()
                    .ghost()
                    .disabled(!can_create)
                    .on_activate(move |_, _, cx| {
                        events_for_folder.dispatch(
                            PageEvent::CreateLocalStyleFolder {
                                kind,
                                selected: Vec::new(),
                            },
                            cx,
                        );
                    }),
            )
            .child(
                Button::new(SharedString::from(style_selector.clone()))
                    .debug_selector(move || style_selector.clone())
                    .icon(IconName::Plus)
                    .tooltip(tooltip)
                    .xsmall()
                    .compact()
                    .ghost()
                    .w(px(22.))
                    .h(px(22.))
                    .disabled(!can_create)
                    .on_activate(move |_, _, cx| {
                        events_for_style.dispatch(
                            PageEvent::CreateLocalStyle {
                                kind,
                                parent_folder_id: None,
                            },
                            cx,
                        );
                    }),
            )
            .when(projection.can_edit_page(), move |row| {
                row.can_drop(move |candidate, _, _| {
                    candidate
                        .downcast_ref::<LocalStyleDrag>()
                        .is_some_and(|drag| drag.target.kind == kind)
                })
                .drag_over::<LocalStyleDrag>(move |style, drag, _, _| {
                    if drag.target.kind == kind {
                        style
                            .bg(root_drop_background)
                            .border_color(root_drop_border)
                    } else {
                        style
                    }
                })
                .on_drop(root_drop_listener)
            });
        content = content
            .child(family_header)
            .child(render_local_style_entries(
                projection, chrome, events, page_id, kind, None, entries, 0, cx,
            ));
    }
    content.into_any_element()
}

pub(in super::super) fn render(
    projection: &PageProjection,
    mut chrome: PageChrome<'_>,
    events: PageEventSink,
    cx: &mut Context<DesignPanel>,
) -> AnyElement {
    let page = projection.page_view_data_for_context();
    let mut header = h_flex()
        .h(px(40.))
        .px(px(PANEL_PADDING))
        .gap_2()
        .child(div().flex_1().text_sm().font_semibold().child("Page"));
    if let Some(mode_browser) = render_variable_mode_popover(projection, events.clone(), cx) {
        header = header.child(mode_browser);
    }
    let mut page_body = v_flex().px(px(PANEL_PADDING)).pb_4().gap_2();
    if let Some(page) = page {
        page_body = page_body.child(
            h_flex()
                .h(px(ROW_HEIGHT))
                .gap_2()
                .child(
                    div()
                        .size(px(20.))
                        .rounded(px(4.))
                        .border_1()
                        .border_color(cx.theme().border)
                        .bg(color_hsla(page.background.color)),
                )
                .child(div().flex_1().text_xs().child("Page background"))
                .child(chrome.take_background_picker()),
        );
        if let Some(local_styles) = projection.page_local_styles_view_data_for_context() {
            page_body = page_body.child(render_page_local_styles(
                projection,
                &chrome,
                &events,
                local_styles,
                cx,
            ));
        }
        if let DesignVariablesEntryPoint::LegacyRightSidebar { disabled_reason } =
            &projection.preferences.variables_entry_point
        {
            let events = events.clone();
            let enabled = disabled_reason.is_none();
            let selector = format!("{}-open-variables", projection.id);
            page_body = page_body.child(
                Button::new(SharedString::from(selector.clone()))
                    .debug_selector(move || selector.clone())
                    .label("Open variables")
                    .tooltip(
                        disabled_reason
                            .clone()
                            .unwrap_or_else(|| "Open Variables".into()),
                    )
                    .xsmall()
                    .compact()
                    .ghost()
                    .w_full()
                    .disabled(!enabled)
                    .on_activate(move |_, _, cx| {
                        events.dispatch(PageEvent::OpenVariables, cx);
                    }),
            );
        }
    } else {
        page_body = page_body.child(
            div()
                .py_2()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child("Supply DesignPageViewData to inspect this Page"),
        );
    }
    v_flex()
        .w_full()
        .flex_none()
        .border_b_1()
        .border_color(cx.theme().sidebar_border)
        .child(header)
        .child(page_body)
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::organisms::design::{
        DesignLocalStyleItem, DesignLocalStyleSection, DesignPageBackground, DesignVariableMode,
        DesignVariableModeCollection,
    };

    const PAGE_ID: &str = "page-1";
    const NODE_ID: &str = "node-1";

    fn page_target() -> DesignPanelTarget {
        DesignPanelTarget::Page {
            page_id: PAGE_ID.into(),
        }
    }

    fn node_target() -> DesignPanelTarget {
        DesignPanelTarget::Nodes {
            node_ids: vec![NODE_ID.into()],
        }
    }

    fn page() -> DesignPageViewData {
        DesignPageViewData::canonical(PAGE_ID, DesignPageBackground::new(DesignColor::WHITE))
    }

    fn color_style() -> DesignLocalStyleEntry {
        DesignLocalStyleEntry::style(DesignLocalStyleItem::new(
            "color-a",
            "A",
            DesignLocalStylePreview::Color(vec![DesignPaint::solid(DesignColor::BLUE)]),
        ))
    }

    fn local_styles(target: DesignPanelTarget) -> DesignPageLocalStylesViewData {
        DesignPageLocalStylesViewData::new(
            target,
            [DesignLocalStyleSection::new(
                DesignLocalStyleKind::Color,
                [color_style()],
            )],
        )
    }

    fn color_style_target() -> DesignLocalStyleTarget {
        DesignLocalStyleTarget::new(PAGE_ID, DesignLocalStyleKind::Color, "color-a", None, 0)
    }

    fn variable_modes(target: DesignPanelTarget) -> DesignVariableModeViewData {
        DesignVariableModeViewData::new(
            target,
            [DesignVariableModeCollection::new(
                "theme",
                "Theme",
                DesignLocalResourceSource::Local,
                [
                    DesignVariableMode::new("light", "Light"),
                    DesignVariableMode::new("dark", "Dark"),
                ],
                "light",
                "light",
            )
            .explicit("dark")],
        )
    }

    fn projection(
        selection_kind: DesignPanelSelectionKind,
        permissions: DesignPanelPermissions,
        host: PageHostProjection,
    ) -> PageProjection {
        PageProjection::new(
            "design".into(),
            PageContextProjection::new(selection_kind, permissions, NODE_ID.into()),
            host,
            PagePresentationProjection::new(
                DesignVariablesEntryPoint::NavigationBarOnly,
                None,
                false,
                HashSet::new(),
            ),
        )
    }

    #[test]
    fn page_projection_hides_page_data_for_node_selection() {
        let page_selection = projection(
            DesignPanelSelectionKind::None,
            DesignPanelPermissions::editor(),
            PageHostProjection::new(
                Some(page()),
                Some(local_styles(page_target())),
                Some(variable_modes(page_target())),
            ),
        );
        assert_eq!(
            page_selection.page_view_data_for_context(),
            Some(&page()),
            "an empty canvas selection is the only context that inspects the Page"
        );
        assert!(
            page_selection
                .page_local_styles_view_data_for_context()
                .is_some(),
            "Page local styles ride along with the projected Page"
        );
        assert!(page_selection.can_edit_page());

        let node_selection = projection(
            DesignPanelSelectionKind::Single,
            DesignPanelPermissions::editor(),
            PageHostProjection::new(
                Some(page()),
                Some(local_styles(page_target())),
                Some(variable_modes(node_target())),
            ),
        );
        assert!(
            node_selection.page_view_data_for_context().is_none(),
            "a selected node must never project Page data the host scoped to the Page"
        );
        assert!(
            node_selection
                .page_local_styles_view_data_for_context()
                .is_none(),
            "without a projected Page there is no anchor for Page local styles"
        );
        assert!(!node_selection.can_edit_page());
        assert!(
            local_resource_create_action(&node_selection, DesignLocalResourceKind::PaintStyle)
                .is_none(),
            "Page-scoped intents cannot be emitted while a node is selected"
        );
        assert!(
            node_selection
                .variable_mode_view_data_for_context()
                .is_some(),
            "node-targeted variable modes still project, so the missing Page data is the \
             selection gate and not an empty host projection"
        );

        let multiple_selection = projection(
            DesignPanelSelectionKind::Multiple,
            DesignPanelPermissions::editor(),
            PageHostProjection::new(
                Some(page()),
                Some(local_styles(page_target())),
                Some(variable_modes(page_target())),
            ),
        );
        assert!(
            multiple_selection.page_view_data_for_context().is_none(),
            "a multi-selection is not a Page context either"
        );
        assert!(
            multiple_selection
                .variable_mode_view_data_for_context()
                .is_none(),
            "a multi-selection has no single variable-mode target to match"
        );
    }

    #[test]
    fn stale_local_styles_target_is_filtered() {
        let current = projection(
            DesignPanelSelectionKind::None,
            DesignPanelPermissions::editor(),
            PageHostProjection::new(Some(page()), Some(local_styles(page_target())), None),
        );
        assert!(
            current.page_local_styles_view_data_for_context().is_some(),
            "a local-styles projection whose target is the projected Page survives"
        );
        assert!(current.local_style_target_is_enabled(&color_style_target()));
        assert!(
            local_style_create_action(&current, DesignLocalStyleKind::Color, None).is_some(),
            "the control case must reach the intent, or the stale cases prove nothing"
        );

        let stale_page = projection(
            DesignPanelSelectionKind::None,
            DesignPanelPermissions::editor(),
            PageHostProjection::new(
                Some(page()),
                Some(local_styles(DesignPanelTarget::Page {
                    page_id: "page-removed".into(),
                })),
                None,
            ),
        );
        assert!(
            stale_page
                .page_local_styles_view_data_for_context()
                .is_none(),
            "local styles targeting a Page that is no longer projected must be dropped"
        );
        assert!(!stale_page.local_style_target_is_enabled(&color_style_target()));
        assert!(
            local_style_create_action(&stale_page, DesignLocalStyleKind::Color, None).is_none(),
            "a dropped local-styles projection cannot emit create intents"
        );
        assert!(
            local_styles_delete_action(&stale_page, vec![color_style_target()]).is_none(),
            "a dropped local-styles projection cannot emit delete intents"
        );

        let node_scoped = projection(
            DesignPanelSelectionKind::None,
            DesignPanelPermissions::editor(),
            PageHostProjection::new(Some(page()), Some(local_styles(node_target())), None),
        );
        assert!(
            node_scoped
                .page_local_styles_view_data_for_context()
                .is_none(),
            "a node-shaped local-styles target is never a Page target"
        );

        let page_absent = projection(
            DesignPanelSelectionKind::None,
            DesignPanelPermissions::editor(),
            PageHostProjection::new(None, Some(local_styles(page_target())), None),
        );
        assert!(
            page_absent
                .page_local_styles_view_data_for_context()
                .is_none(),
            "with no Page there is no target for local styles to match"
        );

        let invalid = DesignPageLocalStylesViewData::new(
            page_target(),
            [
                DesignLocalStyleSection::new(DesignLocalStyleKind::Color, [color_style()]),
                DesignLocalStyleSection::new(DesignLocalStyleKind::Color, [color_style()]),
            ],
        );
        assert!(
            !invalid.is_valid(),
            "duplicate family sections are an invalid host tree"
        );
        let invalid_tree = projection(
            DesignPanelSelectionKind::None,
            DesignPanelPermissions::editor(),
            PageHostProjection::new(Some(page()), Some(invalid), None),
        );
        assert!(
            invalid_tree
                .page_local_styles_view_data_for_context()
                .is_none(),
            "an on-target but invalid local-styles tree is filtered out as well"
        );
    }

    #[test]
    fn viewer_permissions_disable_page_and_variable_mode_edits() {
        let host = || {
            PageHostProjection::new(
                Some(page()),
                Some(local_styles(page_target())),
                Some(variable_modes(page_target())),
            )
        };

        let viewer = projection(
            DesignPanelSelectionKind::None,
            DesignPanelPermissions::viewer(),
            host(),
        );
        assert!(
            viewer.page_view_data_for_context().is_some(),
            "a viewer still inspects the Page"
        );
        assert!(
            viewer.variable_mode_view_data_for_context().is_some(),
            "a viewer still reads the projected variable modes"
        );
        assert!(
            !viewer.can_edit_page(),
            "view-only access cannot edit the Page"
        );
        assert!(
            !viewer.can_edit_variable_modes(),
            "view-only access cannot edit variable modes"
        );
        assert!(
            local_resource_create_action(&viewer, DesignLocalResourceKind::PaintStyle).is_none()
        );
        assert!(local_style_create_action(&viewer, DesignLocalStyleKind::Color, None).is_none());
        assert!(local_styles_delete_action(&viewer, vec![color_style_target()]).is_none());
        assert!(
            local_style_command_action(
                &viewer,
                color_style_target(),
                DesignLocalStyleCommand::Edit,
            )
            .is_none()
        );
        assert!(
            variable_mode_apply_action(&viewer, "theme".into(), "light".into()).is_none(),
            "a viewer cannot apply an explicit variable mode"
        );
        assert!(
            variable_mode_clear_action(&viewer, "theme".into(), "dark".into()).is_none(),
            "a viewer cannot clear an explicit variable mode"
        );
        assert!(
            local_style_command_action(
                &viewer,
                color_style_target(),
                DesignLocalStyleCommand::Copy,
            )
            .is_some(),
            "copy stays available, so the gate is the edit permission and not a blanket deny"
        );

        let editor = projection(
            DesignPanelSelectionKind::None,
            DesignPanelPermissions::editor(),
            host(),
        );
        assert!(editor.can_edit_page());
        assert!(editor.can_edit_variable_modes());
        assert!(
            local_resource_create_action(&editor, DesignLocalResourceKind::PaintStyle).is_some()
        );
        assert!(local_style_create_action(&editor, DesignLocalStyleKind::Color, None).is_some());
        assert!(local_styles_delete_action(&editor, vec![color_style_target()]).is_some());
        assert!(
            local_style_command_action(
                &editor,
                color_style_target(),
                DesignLocalStyleCommand::Edit,
            )
            .is_some()
        );
        assert!(variable_mode_apply_action(&editor, "theme".into(), "light".into()).is_some());
        assert!(variable_mode_clear_action(&editor, "theme".into(), "dark".into()).is_some());
    }
}
