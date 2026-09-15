//! Live Page controller boundary for the Design inspector facade.

use super::*;

#[allow(dead_code)]
pub(in super::super::super) trait PagePanelController: Sized {
    fn page_projection(&self) -> PageProjection;
    fn render_page(&self, cx: &mut Context<Self>) -> AnyElement;
    fn render_variable_mode_popover(&self, cx: &mut Context<Self>) -> Option<AnyElement>;
    fn close_page_resource_browser(
        &mut self,
        category: DesignLocalResourceCategory,
        cx: &mut Context<Self>,
    );
    fn set_variable_mode_browser_open(&mut self, open: bool, cx: &mut Context<Self>);
    fn page_view_data_for_context(&self) -> Option<&DesignPageViewData>;
    fn page_local_styles_view_data_for_context(&self) -> Option<&DesignPageLocalStylesViewData>;
    fn can_edit_page(&self) -> bool;
    fn current_variable_mode_target(&self) -> Option<DesignPanelTarget>;
    fn variable_mode_view_data_for_context(&self) -> Option<&DesignVariableModeViewData>;
    fn can_edit_variable_modes(&self) -> bool;
    fn emit_local_resource_browse(
        &mut self,
        category: DesignLocalResourceCategory,
        cx: &mut Context<Self>,
    );
    fn emit_local_resource_open(
        &mut self,
        resource: DesignLocalResourceSelection,
        cx: &mut Context<Self>,
    );
    fn emit_local_resource_create(&mut self, kind: DesignLocalResourceKind, cx: &mut Context<Self>);
    fn emit_local_resource_import(
        &mut self,
        resource: DesignLocalResourceSelection,
        cx: &mut Context<Self>,
    );
    fn local_style_target_is_current(&self, target: &DesignLocalStyleTarget) -> bool;
    fn local_style_target_is_enabled(&self, target: &DesignLocalStyleTarget) -> bool;
    fn local_style_folder_is_enabled(
        view_data: &DesignPageLocalStylesViewData,
        kind: DesignLocalStyleKind,
        folder_id: Option<&str>,
    ) -> bool;
    fn local_style_insertion_is_current(&self, insertion: &DesignLocalStyleInsertion) -> bool;
    fn local_style_targets_are_current(
        &self,
        targets: &[DesignLocalStyleTarget],
        allow_empty: bool,
    ) -> bool;
    fn toggle_local_style_folder(&mut self, folder_id: SharedString, cx: &mut Context<Self>);
    fn emit_local_style_command(
        &self,
        target: DesignLocalStyleTarget,
        command: DesignLocalStyleCommand,
        cx: &mut Context<Self>,
    );
    fn emit_local_style_create(
        &self,
        kind: DesignLocalStyleKind,
        parent_folder_id: Option<SharedString>,
        cx: &mut Context<Self>,
    );
    fn emit_local_style_folder_create(
        &self,
        kind: DesignLocalStyleKind,
        selected: Vec<DesignLocalStyleTarget>,
        cx: &mut Context<Self>,
    );
    fn emit_local_styles_delete(
        &self,
        targets: Vec<DesignLocalStyleTarget>,
        cx: &mut Context<Self>,
    );
    fn emit_local_styles_move(
        &self,
        targets: Vec<DesignLocalStyleTarget>,
        destination: DesignLocalStyleInsertion,
        cx: &mut Context<Self>,
    );
    fn emit_variables_view_open(&self, cx: &mut Context<Self>);
    fn emit_variable_mode_apply(
        &mut self,
        collection_id: SharedString,
        mode_id: SharedString,
        cx: &mut Context<Self>,
    );
    fn emit_variable_mode_clear(
        &mut self,
        collection_id: SharedString,
        explicit_mode_id: SharedString,
        cx: &mut Context<Self>,
    );
}

impl PagePanelController for DesignPanel {
    fn page_projection(&self) -> PageProjection {
        PageProjection::new(
            self.id.clone(),
            PageContextProjection::new(
                self.host.inspection_context.selection().kind(),
                self.host.inspection_context.permissions(),
                self.host.inspected_node().id.clone(),
            ),
            PageHostProjection::new(
                self.host.projections.page.clone(),
                self.host.projections.page_local_styles.clone(),
                self.host.projections.variable_modes.clone(),
            ),
            PagePresentationProjection::new(
                self.preferences.variables_entry_point.clone(),
                self.overlays.page_resource_browser(),
                self.overlays.variable_mode_browser_open(),
                self.features.page.collapsed_local_style_folders.clone(),
            ),
        )
    }

    fn render_page(&self, cx: &mut Context<Self>) -> AnyElement {
        let projection = self.page_projection();
        let background_picker = projection
            .page_view_data_for_context()
            .map(|page| self.render_page_background_picker(page, cx));
        let paint_swatch = |paint: &DesignPaint, cx: &mut Context<DesignPanel>| {
            self.render_paint_swatch(paint, cx)
        };
        render(
            &projection,
            PageChrome::new(background_picker, &paint_swatch),
            PageEventSink::new(cx.entity()),
            cx,
        )
    }

    fn render_variable_mode_popover(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let projection = self.page_projection();
        render_variable_mode_popover(&projection, PageEventSink::new(cx.entity()), cx)
    }

    fn close_page_resource_browser(
        &mut self,
        category: DesignLocalResourceCategory,
        cx: &mut Context<Self>,
    ) {
        if self.overlays.page_resource_browser() == Some(category) {
            self.overlays.discard(DesignOpenOverlay::PageResource);
            cx.notify();
        }
    }

    fn set_variable_mode_browser_open(&mut self, open: bool, cx: &mut Context<Self>) {
        self.overlays
            .set_open(DesignOverlayState::VariableMode, open);
        if open {
            self.cancel_menu_preview(cx);
        }
        cx.notify();
    }

    fn page_view_data_for_context(&self) -> Option<&DesignPageViewData> {
        (self.host.inspection_context.selection().kind() == DesignPanelSelectionKind::None)
            .then_some(self.host.projections.page.as_ref())
            .flatten()
    }

    fn page_local_styles_view_data_for_context(&self) -> Option<&DesignPageLocalStylesViewData> {
        let page = self.page_view_data_for_context()?;
        let target = DesignPanelTarget::Page {
            page_id: page.page_id.clone(),
        };
        self.host
            .projections
            .page_local_styles
            .as_ref()
            .filter(|view_data| view_data.target == target && view_data.is_valid())
    }

    fn can_edit_page(&self) -> bool {
        self.host.inspection_context.permissions().can_edit()
            && self.page_view_data_for_context().is_some()
    }

    fn current_variable_mode_target(&self) -> Option<DesignPanelTarget> {
        match self.host.inspection_context.selection().kind() {
            DesignPanelSelectionKind::None => {
                self.page_view_data_for_context()
                    .map(|page| DesignPanelTarget::Page {
                        page_id: page.page_id.clone(),
                    })
            }
            DesignPanelSelectionKind::Single => Some(DesignPanelTarget::Nodes {
                node_ids: vec![self.host.inspected_node().id.clone()],
            }),
            DesignPanelSelectionKind::Multiple => None,
        }
    }

    fn variable_mode_view_data_for_context(&self) -> Option<&DesignVariableModeViewData> {
        let target = self.current_variable_mode_target()?;
        self.host
            .projections
            .variable_modes
            .as_ref()
            .filter(|view_data| view_data.target == target)
    }

    fn can_edit_variable_modes(&self) -> bool {
        self.host.inspection_context.permissions().can_edit()
            && self.variable_mode_view_data_for_context().is_some()
    }

    fn emit_local_resource_browse(
        &mut self,
        category: DesignLocalResourceCategory,
        cx: &mut Context<Self>,
    ) {
        let Some(action) = local_resource_browse_action(&self.page_projection(), category) else {
            return;
        };
        self.overlays
            .open(DesignOverlayState::PageResource(category));
        cx.emit_design_panel_action(self, action);
        cx.notify();
    }

    fn emit_local_resource_open(
        &mut self,
        resource: DesignLocalResourceSelection,
        cx: &mut Context<Self>,
    ) {
        let Some(action) = local_resource_open_action(&self.page_projection(), resource) else {
            return;
        };
        self.overlays.discard(DesignOpenOverlay::PageResource);
        cx.emit_design_panel_action(self, action);
        cx.notify();
    }

    fn emit_local_resource_create(
        &mut self,
        kind: DesignLocalResourceKind,
        cx: &mut Context<Self>,
    ) {
        let Some(action) = local_resource_create_action(&self.page_projection(), kind) else {
            return;
        };
        self.overlays.discard(DesignOpenOverlay::PageResource);
        cx.emit_design_panel_action(self, action);
        cx.notify();
    }

    fn emit_local_resource_import(
        &mut self,
        resource: DesignLocalResourceSelection,
        cx: &mut Context<Self>,
    ) {
        let Some(action) = local_resource_import_action(&self.page_projection(), resource) else {
            return;
        };
        cx.emit_design_panel_action(self, action);
    }

    fn local_style_target_is_current(&self, target: &DesignLocalStyleTarget) -> bool {
        self.page_projection().local_style_target_is_current(target)
    }

    fn local_style_target_is_enabled(&self, target: &DesignLocalStyleTarget) -> bool {
        self.page_projection().local_style_target_is_enabled(target)
    }

    fn local_style_folder_is_enabled(
        view_data: &DesignPageLocalStylesViewData,
        kind: DesignLocalStyleKind,
        folder_id: Option<&str>,
    ) -> bool {
        PageProjection::local_style_folder_is_enabled(view_data, kind, folder_id)
    }

    fn local_style_insertion_is_current(&self, insertion: &DesignLocalStyleInsertion) -> bool {
        self.page_projection()
            .local_style_insertion_is_current(insertion)
    }

    fn local_style_targets_are_current(
        &self,
        targets: &[DesignLocalStyleTarget],
        allow_empty: bool,
    ) -> bool {
        self.page_projection()
            .local_style_targets_are_current(targets, allow_empty)
    }

    fn toggle_local_style_folder(&mut self, folder_id: SharedString, cx: &mut Context<Self>) {
        let projection = self.page_projection();
        let folder_exists = projection
            .page_local_styles_view_data_for_context()
            .is_some_and(|view_data| {
                DesignLocalStyleKind::ALL
                    .into_iter()
                    .any(|kind| view_data.folder(kind, folder_id.as_ref()).is_some())
            });
        if !folder_exists {
            return;
        }
        if !self
            .features
            .page
            .collapsed_local_style_folders
            .remove(&folder_id)
        {
            self.features
                .page
                .collapsed_local_style_folders
                .insert(folder_id);
        }
        cx.notify();
    }

    fn emit_local_style_command(
        &self,
        target: DesignLocalStyleTarget,
        command: DesignLocalStyleCommand,
        cx: &mut Context<Self>,
    ) {
        let Some(action) = local_style_command_action(&self.page_projection(), target, command)
        else {
            return;
        };
        cx.emit_design_panel_action(self, action);
    }

    fn emit_local_style_create(
        &self,
        kind: DesignLocalStyleKind,
        parent_folder_id: Option<SharedString>,
        cx: &mut Context<Self>,
    ) {
        let Some(action) =
            local_style_create_action(&self.page_projection(), kind, parent_folder_id)
        else {
            return;
        };
        cx.emit_design_panel_action(self, action);
    }

    fn emit_local_style_folder_create(
        &self,
        kind: DesignLocalStyleKind,
        selected: Vec<DesignLocalStyleTarget>,
        cx: &mut Context<Self>,
    ) {
        let Some(action) =
            local_style_folder_create_action(&self.page_projection(), kind, selected)
        else {
            return;
        };
        cx.emit_design_panel_action(self, action);
    }

    fn emit_local_styles_delete(
        &self,
        targets: Vec<DesignLocalStyleTarget>,
        cx: &mut Context<Self>,
    ) {
        let Some(action) = local_styles_delete_action(&self.page_projection(), targets) else {
            return;
        };
        cx.emit_design_panel_action(self, action);
    }

    fn emit_local_styles_move(
        &self,
        targets: Vec<DesignLocalStyleTarget>,
        destination: DesignLocalStyleInsertion,
        cx: &mut Context<Self>,
    ) {
        let Some(action) = local_styles_move_action(&self.page_projection(), targets, destination)
        else {
            return;
        };
        cx.emit_design_panel_action(self, action);
    }

    fn emit_variables_view_open(&self, cx: &mut Context<Self>) {
        let Some(action) = variables_view_open_action(&self.page_projection()) else {
            return;
        };
        cx.emit_design_panel_action(self, action);
    }

    fn emit_variable_mode_apply(
        &mut self,
        collection_id: SharedString,
        mode_id: SharedString,
        cx: &mut Context<Self>,
    ) {
        let Some(action) =
            variable_mode_apply_action(&self.page_projection(), collection_id, mode_id)
        else {
            return;
        };
        self.overlays.discard(DesignOpenOverlay::VariableMode);
        cx.emit_design_panel_action(self, action);
        cx.notify();
    }

    fn emit_variable_mode_clear(
        &mut self,
        collection_id: SharedString,
        explicit_mode_id: SharedString,
        cx: &mut Context<Self>,
    ) {
        let Some(action) =
            variable_mode_clear_action(&self.page_projection(), collection_id, explicit_mode_id)
        else {
            return;
        };
        self.overlays.discard(DesignOpenOverlay::VariableMode);
        cx.emit_design_panel_action(self, action);
        cx.notify();
    }
}
