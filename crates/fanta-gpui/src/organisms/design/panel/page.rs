use super::*;

impl DesignPanel {
    pub(super) fn page_view_data_for_context(&self) -> Option<&DesignPageViewData> {
        (self.inspection_context.selection().kind() == DesignPanelSelectionKind::None)
            .then_some(self.page_view_data.as_ref())
            .flatten()
    }

    pub(super) fn page_local_styles_view_data_for_context(
        &self,
    ) -> Option<&DesignPageLocalStylesViewData> {
        let page = self.page_view_data_for_context()?;
        let target = DesignPanelTarget::Page {
            page_id: page.page_id.clone(),
        };
        self.page_local_styles_view_data
            .as_ref()
            .filter(|view_data| view_data.target == target && view_data.is_valid())
    }

    pub(super) fn can_edit_page(&self) -> bool {
        self.inspection_context.permissions().can_edit()
            && self.page_view_data_for_context().is_some()
    }

    pub(super) fn current_variable_mode_target(&self) -> Option<DesignPanelTarget> {
        match self.inspection_context.selection().kind() {
            DesignPanelSelectionKind::None => {
                self.page_view_data_for_context()
                    .map(|page| DesignPanelTarget::Page {
                        page_id: page.page_id.clone(),
                    })
            }
            DesignPanelSelectionKind::Single => Some(DesignPanelTarget::Nodes {
                node_ids: vec![self.node.id.clone()],
            }),
            DesignPanelSelectionKind::Multiple => None,
        }
    }

    pub(super) fn variable_mode_view_data_for_context(
        &self,
    ) -> Option<&DesignVariableModeViewData> {
        let target = self.current_variable_mode_target()?;
        self.variable_mode_view_data
            .as_ref()
            .filter(|view_data| view_data.target == target)
    }

    pub(super) fn can_edit_variable_modes(&self) -> bool {
        self.inspection_context.permissions().can_edit()
            && self.variable_mode_view_data_for_context().is_some()
    }

    pub(super) fn emit_local_resource_browse(
        &mut self,
        category: DesignLocalResourceCategory,
        cx: &mut Context<Self>,
    ) {
        let Some(page_id) = self
            .page_view_data_for_context()
            .map(|page| page.page_id.clone())
        else {
            return;
        };
        self.page_resource_browser = Some(category);
        self.page_background_picker_open = false;
        self.variable_mode_browser_open = false;
        cx.emit_design_panel_action(
            self,
            DesignPanelAction::LocalResourceBrowseRequested { page_id, category },
        );
        cx.notify();
    }

    pub(super) fn emit_local_resource_open(
        &mut self,
        resource: DesignLocalResourceSelection,
        cx: &mut Context<Self>,
    ) {
        let Some(page) = self.page_view_data_for_context() else {
            return;
        };
        if !page
            .local_resources
            .resource(&resource)
            .is_some_and(|resource| resource.availability.can_open())
        {
            return;
        }
        let page_id = page.page_id.clone();
        self.page_resource_browser = None;
        cx.emit_design_panel_action(
            self,
            DesignPanelAction::LocalResourceOpenRequested { page_id, resource },
        );
        cx.notify();
    }

    pub(super) fn emit_local_resource_create(
        &mut self,
        kind: DesignLocalResourceKind,
        cx: &mut Context<Self>,
    ) {
        let Some(page_id) = self
            .page_view_data_for_context()
            .map(|page| page.page_id.clone())
        else {
            return;
        };
        if !self.can_edit_page() {
            return;
        }
        self.page_resource_browser = None;
        cx.emit_design_panel_action(
            self,
            DesignPanelAction::LocalResourceCreateRequested { page_id, kind },
        );
        cx.notify();
    }

    pub(super) fn emit_local_resource_import(
        &mut self,
        resource: DesignLocalResourceSelection,
        cx: &mut Context<Self>,
    ) {
        let Some(page) = self.page_view_data_for_context() else {
            return;
        };
        if !self.can_edit_page()
            || !matches!(&resource.source, DesignLocalResourceSource::Library { .. })
            || !page
                .local_resources
                .resource(&resource)
                .is_some_and(|resource| resource.availability.can_import())
        {
            return;
        }
        let page_id = page.page_id.clone();
        cx.emit_design_panel_action(
            self,
            DesignPanelAction::LocalResourceImportRequested { page_id, resource },
        );
    }

    pub(super) fn local_style_target_is_current(&self, target: &DesignLocalStyleTarget) -> bool {
        self.page_local_styles_view_data_for_context()
            .and_then(|view_data| view_data.resolve_style(target))
            .is_some()
    }

    pub(super) fn local_style_target_is_enabled(&self, target: &DesignLocalStyleTarget) -> bool {
        self.local_style_target_is_current(target)
            && self
                .page_local_styles_view_data_for_context()
                .is_some_and(|view_data| {
                    view_data.entry_path_is_enabled(target.kind, target.style_id.as_ref())
                })
    }

    pub(super) fn local_style_folder_is_enabled(
        view_data: &DesignPageLocalStylesViewData,
        kind: DesignLocalStyleKind,
        folder_id: Option<&str>,
    ) -> bool {
        folder_id.is_none_or(|folder_id| {
            view_data.folder(kind, folder_id).is_some()
                && view_data.entry_path_is_enabled(kind, folder_id)
        })
    }

    pub(super) fn local_style_insertion_is_current(
        &self,
        insertion: &DesignLocalStyleInsertion,
    ) -> bool {
        let Some(view_data) = self.page_local_styles_view_data_for_context() else {
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

    pub(super) fn local_style_targets_are_current(
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

    pub(super) fn toggle_local_style_folder(
        &mut self,
        folder_id: SharedString,
        cx: &mut Context<Self>,
    ) {
        let folder_exists =
            self.page_local_styles_view_data_for_context()
                .is_some_and(|view_data| {
                    DesignLocalStyleKind::ALL
                        .into_iter()
                        .any(|kind| view_data.folder(kind, folder_id.as_ref()).is_some())
                });
        if !folder_exists {
            return;
        }
        if !self.collapsed_local_style_folders.remove(&folder_id) {
            self.collapsed_local_style_folders.insert(folder_id);
        }
        cx.notify();
    }

    pub(super) fn emit_local_style_command(
        &self,
        target: DesignLocalStyleTarget,
        command: DesignLocalStyleCommand,
        cx: &mut Context<Self>,
    ) {
        let permission_allows = match command {
            DesignLocalStyleCommand::Edit | DesignLocalStyleCommand::Duplicate => {
                self.can_edit_page()
            }
            DesignLocalStyleCommand::Copy => self.inspection_context.permissions().can_copy(),
            DesignLocalStyleCommand::GoToDefinition => true,
        };
        if !permission_allows || !self.local_style_target_is_enabled(&target) {
            return;
        }
        cx.emit_design_panel_action(
            self,
            DesignPanelAction::LocalStyleCommandRequested { target, command },
        );
    }

    pub(super) fn emit_local_style_create(
        &self,
        kind: DesignLocalStyleKind,
        parent_folder_id: Option<SharedString>,
        cx: &mut Context<Self>,
    ) {
        let Some(view_data) = self.page_local_styles_view_data_for_context() else {
            return;
        };
        let Some(page_id) = view_data.page_id().cloned() else {
            return;
        };
        if !self.can_edit_page()
            || view_data.create_disabled_reason.is_some()
            || parent_folder_id.as_ref().is_some_and(|folder_id| {
                !Self::local_style_folder_is_enabled(view_data, kind, Some(folder_id.as_ref()))
            })
        {
            return;
        }
        cx.emit_design_panel_action(
            self,
            DesignPanelAction::LocalStyleCreateRequested {
                page_id,
                kind,
                parent_folder_id,
            },
        );
    }

    pub(super) fn emit_local_style_folder_create(
        &self,
        kind: DesignLocalStyleKind,
        selected: Vec<DesignLocalStyleTarget>,
        cx: &mut Context<Self>,
    ) {
        let Some(view_data) = self.page_local_styles_view_data_for_context() else {
            return;
        };
        let Some(page_id) = view_data.page_id().cloned() else {
            return;
        };
        if !self.can_edit_page()
            || view_data.create_disabled_reason.is_some()
            || !self.local_style_targets_are_current(&selected, true)
            || selected
                .iter()
                .any(|target| target.page_id != page_id || target.kind != kind)
        {
            return;
        }
        cx.emit_design_panel_action(
            self,
            DesignPanelAction::LocalStyleFolderCreateRequested {
                page_id,
                kind,
                selected,
            },
        );
    }

    pub(super) fn emit_local_styles_delete(
        &self,
        targets: Vec<DesignLocalStyleTarget>,
        cx: &mut Context<Self>,
    ) {
        if !self.can_edit_page() || !self.local_style_targets_are_current(&targets, false) {
            return;
        }
        cx.emit_design_panel_action(
            self,
            DesignPanelAction::LocalStylesDeleteRequested { targets },
        );
    }

    pub(super) fn emit_local_styles_move(
        &self,
        targets: Vec<DesignLocalStyleTarget>,
        destination: DesignLocalStyleInsertion,
        cx: &mut Context<Self>,
    ) {
        if !self.can_edit_page()
            || !self.local_style_targets_are_current(&targets, false)
            || targets.iter().any(|target| target.kind != destination.kind)
            || !self.local_style_insertion_is_current(&destination)
        {
            return;
        }
        cx.emit_design_panel_action(
            self,
            DesignPanelAction::LocalStylesMoveRequested {
                targets,
                destination,
            },
        );
    }

    pub(super) fn emit_variables_view_open(&self, cx: &mut Context<Self>) {
        let DesignVariablesEntryPoint::LegacyRightSidebar { disabled_reason } =
            &self.variables_entry_point
        else {
            return;
        };
        let Some(page) = self.page_view_data_for_context() else {
            return;
        };
        if disabled_reason.is_some() {
            return;
        }
        cx.emit_design_panel_action(
            self,
            DesignPanelAction::VariablesViewOpenRequested {
                page_id: page.page_id.clone(),
            },
        );
    }

    pub(super) fn emit_variable_mode_apply(
        &mut self,
        collection_id: SharedString,
        mode_id: SharedString,
        cx: &mut Context<Self>,
    ) {
        let Some(view_data) = self.variable_mode_view_data_for_context() else {
            return;
        };
        let Some(collection) = view_data.collection(collection_id.as_ref()) else {
            return;
        };
        if !self.can_edit_variable_modes()
            || collection.mode(mode_id.as_ref()).is_none()
            || collection.mode_disabled_reason(mode_id.as_ref()).is_some()
            || collection.explicit_mode_id.as_ref() == Some(&mode_id)
        {
            return;
        }
        let target = view_data.target.clone();
        self.variable_mode_browser_open = false;
        cx.emit_design_panel_action(
            self,
            DesignPanelAction::VariableModeApplyRequested {
                target,
                collection_id,
                mode_id,
            },
        );
        cx.notify();
    }

    pub(super) fn emit_variable_mode_clear(
        &mut self,
        collection_id: SharedString,
        explicit_mode_id: SharedString,
        cx: &mut Context<Self>,
    ) {
        let Some(view_data) = self.variable_mode_view_data_for_context() else {
            return;
        };
        let Some(collection) = view_data.collection(collection_id.as_ref()) else {
            return;
        };
        if !self.can_edit_variable_modes()
            || collection.disabled_reason.is_some()
            || collection.explicit_mode_id.as_ref() != Some(&explicit_mode_id)
        {
            return;
        }
        let target = view_data.target.clone();
        self.variable_mode_browser_open = false;
        cx.emit_design_panel_action(
            self,
            DesignPanelAction::VariableModeClearRequested {
                target,
                collection_id,
                explicit_mode_id,
            },
        );
        cx.notify();
    }

    #[allow(dead_code)]
    pub(super) fn render_page_resource_browser(
        &self,
        category: DesignLocalResourceCategory,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let panel = cx.entity();
        let panel_for_open = panel.clone();
        let panel_for_content = panel;
        let panel_id = self.id.clone();
        let groups = self
            .page_view_data_for_context()
            .map(|page| page.local_resources.groups.clone())
            .unwrap_or_default();
        let has_page = self.page_view_data_for_context().is_some();
        let can_edit = self.can_edit_page();
        let trigger = Button::new(SharedString::from(format!(
            "{}-page-resource-{:?}",
            self.id, category
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
            let panel = panel_for_open.clone();
            let open = self.page_resource_browser == Some(category);
            move |_, cx| {
                panel.update(cx, |this, cx| {
                    if !open {
                        this.emit_local_resource_browse(category, cx);
                    } else if this.page_resource_browser == Some(category) {
                        this.page_resource_browser = None;
                        cx.notify();
                    }
                });
            }
        });

        Popover::new(SharedString::from(format!(
            "{}-page-resource-popover-{:?}",
            self.id, category
        )))
        .anchor(Anchor::TopRight)
        .open(self.page_resource_browser == Some(category))
        .overlay_closable(true)
        .on_open_change(move |open, _, cx| {
            panel_for_open.update(cx, |this, cx| {
                if *open {
                    this.emit_local_resource_browse(category, cx);
                } else if this.page_resource_browser == Some(category) {
                    this.page_resource_browser = None;
                    cx.notify();
                }
            });
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
                            let panel = panel_for_content.clone();
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
                                    panel.update(cx, |this, cx| {
                                        this.emit_local_resource_open(selection.clone(), cx);
                                    });
                                })
                                .into_any_element()
                        }
                        DesignLocalResourceAvailability::Available => {
                            let panel = panel_for_content.clone();
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
                                    panel.update(cx, |this, cx| {
                                        this.emit_local_resource_import(selection.clone(), cx);
                                    });
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
                let panel = panel_for_content.clone();
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
                        panel.update(cx, |this, cx| {
                            this.emit_local_resource_create(kind, cx);
                        });
                    }),
                );
            }
            content.child(create_row)
        })
        .into_any_element()
    }

    pub(super) fn render_variable_mode_popover(
        &self,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let view_data = self.variable_mode_view_data_for_context()?.clone();
        if view_data.collections.is_empty() {
            return None;
        }
        let panel = cx.entity();
        let panel_for_open = panel.clone();
        let panel_for_content = panel;
        let panel_id = self.id.clone();
        let can_edit = self.can_edit_variable_modes();
        let explicit_count = view_data
            .collections
            .iter()
            .filter(|collection| collection.explicit_mode_id.is_some())
            .count();
        let trigger = Button::new(SharedString::from(format!(
            "{}-variable-modes-trigger",
            self.id
        )))
        .label(if explicit_count == 0 {
            "◇".into()
        } else {
            SharedString::from(format!("◇ {explicit_count}"))
        })
        .tooltip("Variable modes")
        .xsmall()
        .compact()
        .ghost()
        .selected(self.variable_mode_browser_open || explicit_count > 0)
        .on_keyboard_activate({
            let panel = panel_for_open.clone();
            let open = self.variable_mode_browser_open;
            move |_, cx| {
                panel.update(cx, |this, cx| {
                    this.variable_mode_browser_open = !open;
                    if !open {
                        this.cancel_menu_preview(cx);
                        this.page_background_picker_open = false;
                        this.page_resource_browser = None;
                        this.appearance_blend_mode_open = false;
                    }
                    cx.notify();
                });
            }
        });

        Some(
            Popover::new(SharedString::from(format!(
                "{}-variable-modes-popover",
                self.id
            )))
            .anchor(Anchor::TopRight)
            .open(self.variable_mode_browser_open)
            .overlay_closable(true)
            .on_open_change(move |open, _, cx| {
                panel_for_open.update(cx, |this, cx| {
                    this.variable_mode_browser_open = *open;
                    if *open {
                        this.cancel_menu_preview(cx);
                        this.page_background_picker_open = false;
                        this.page_resource_browser = None;
                        this.appearance_blend_mode_open = false;
                    }
                    cx.notify();
                });
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
                                let panel = panel_for_content.clone();
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
                                    panel.update(cx, |this, cx| {
                                        this.emit_variable_mode_apply(
                                            collection_id.clone(),
                                            mode_id.clone(),
                                            cx,
                                        );
                                    });
                                })
                            }))
                            .when_some(collection.explicit_mode_id.clone(), |card, mode_id| {
                                let panel = panel_for_content.clone();
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
                                    .on_activate(
                                        move |_, _, cx| {
                                            panel.update(cx, |this, cx| {
                                                this.emit_variable_mode_clear(
                                                    collection_id.clone(),
                                                    mode_id.clone(),
                                                    cx,
                                                );
                                            });
                                        },
                                    ),
                                )
                            }),
                    );
                }
                content
            })
            .into_any_element(),
        )
    }

    pub(super) fn render_local_style_preview(
        &self,
        preview: &DesignLocalStylePreview,
        cx: &mut Context<Self>,
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
                        swatches = swatches.child(self.render_paint_swatch(paint, cx));
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

    pub(super) fn local_style_insertion(
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

    pub(super) fn render_local_style_entries(
        &self,
        page_id: &SharedString,
        kind: DesignLocalStyleKind,
        parent_folder_id: Option<SharedString>,
        entries: &[DesignLocalStyleEntry],
        depth: usize,
        cx: &mut Context<Self>,
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
                    let panel_for_create = cx.entity();
                    let panel_for_toggle = panel_for_create.clone();
                    let collapsed = self.collapsed_local_style_folders.contains(id);
                    let folder_enabled = self
                        .page_local_styles_view_data_for_context()
                        .is_some_and(|view_data| {
                            view_data.entry_path_is_enabled(kind, id.as_ref())
                        });
                    let can_create = self.can_edit_page()
                        && folder_enabled
                        && self
                            .page_local_styles_view_data_for_context()
                            .is_some_and(|view_data| view_data.create_disabled_reason.is_none());
                    let tooltip = disabled_reason
                        .clone()
                        .or_else(|| {
                            (!self.can_edit_page()).then(|| SharedString::from("View only"))
                        })
                        .unwrap_or_else(|| "Create style in folder".into());
                    let create_selector =
                        format!("{}-local-style-folder-{}-create", self.id, folder_id);
                    let toggle_selector =
                        format!("{}-local-style-folder-{}-toggle", self.id, folder_id);
                    let folder_row_selector =
                        format!("{}-local-style-folder-{}", self.id, folder_id);
                    let toggle_folder_id = folder_id.clone();
                    let folder_destination = Self::local_style_insertion(
                        kind,
                        Some(folder_id.clone()),
                        children,
                        children.len(),
                    )
                    .expect("folder-end insertion is in bounds");
                    let can_drop_to_folder = self.can_edit_page() && folder_enabled;
                    let drop_background = cx.theme().selection.opacity(0.18);
                    let drop_border = cx.theme().selection;
                    let folder_drop_listener =
                        cx.listener(move |this, drag: &LocalStyleDrag, _, cx| {
                            cx.stop_propagation();
                            this.emit_local_styles_move(
                                vec![drag.target.clone()],
                                folder_destination.clone(),
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
                                    panel_for_toggle.update(cx, |this, cx| {
                                        this.toggle_local_style_folder(
                                            toggle_folder_id.clone(),
                                            cx,
                                        );
                                    });
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
                                    panel_for_create.update(cx, |this, cx| {
                                        this.emit_local_style_create(
                                            kind,
                                            Some(folder_id.clone()),
                                            cx,
                                        );
                                    });
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
                        list = list.child(self.render_local_style_entries(
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
                    let enabled =
                        self.page_local_styles_view_data_for_context()
                            .is_some_and(|view_data| {
                                view_data.entry_path_is_enabled(kind, style.id.as_ref())
                            });
                    let can_edit = self.can_edit_page() && enabled;
                    let can_copy = self.inspection_context.permissions().can_copy() && enabled;
                    let can_go_to = enabled;
                    let disabled_tooltip = style
                        .disabled_reason
                        .clone()
                        .or_else(|| {
                            (!enabled)
                                .then(|| SharedString::from("Containing folder is unavailable"))
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
                        let panel = cx.entity();
                        let command_target = target.clone();
                        let command_selector =
                            format!("{}-local-style-{}-{:?}", self.id, style.id, command);
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
                                    panel.update(cx, |this, cx| {
                                        this.emit_local_style_command(
                                            command_target.clone(),
                                            command,
                                            cx,
                                        );
                                    });
                                }),
                        );
                    }

                    let panel = cx.entity();
                    let delete_target = target.clone();
                    let delete_selector = format!("{}-local-style-{}-delete", self.id, style.id);
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
                                panel.update(cx, |this, cx| {
                                    this.emit_local_styles_delete(vec![delete_target.clone()], cx);
                                });
                            }),
                    );

                    if index > 0
                        && let Some(destination) = Self::local_style_insertion(
                            kind,
                            parent_folder_id.clone(),
                            entries,
                            index - 1,
                        )
                    {
                        let panel = cx.entity();
                        let move_target = target.clone();
                        let move_selector = format!("{}-local-style-{}-move-up", self.id, style.id);
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
                                    panel.update(cx, |this, cx| {
                                        this.emit_local_styles_move(
                                            vec![move_target.clone()],
                                            destination.clone(),
                                            cx,
                                        );
                                    });
                                }),
                        );
                    }
                    if index + 1 < entries.len()
                        && let Some(destination) = Self::local_style_insertion(
                            kind,
                            parent_folder_id.clone(),
                            entries,
                            index + 2,
                        )
                    {
                        let panel = cx.entity();
                        let move_target = target.clone();
                        let move_selector =
                            format!("{}-local-style-{}-move-down", self.id, style.id);
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
                                    panel.update(cx, |this, cx| {
                                        this.emit_local_styles_move(
                                            vec![move_target.clone()],
                                            destination.clone(),
                                            cx,
                                        );
                                    });
                                }),
                        );
                    }

                    let style_row_selector = format!("{}-local-style-{}", self.id, style.id);
                    let drag = LocalStyleDrag {
                        target: target.clone(),
                        label: style.name.clone(),
                    };
                    let drop_style_id = style.id.clone();
                    let style_destination =
                        Self::local_style_insertion(kind, parent_folder_id.clone(), entries, index)
                            .expect("style-before insertion is in bounds");
                    let drop_background = cx.theme().selection.opacity(0.18);
                    let drop_border = cx.theme().selection;
                    let style_drop_listener =
                        cx.listener(move |this, drag: &LocalStyleDrag, _, cx| {
                            cx.stop_propagation();
                            this.emit_local_styles_move(
                                vec![drag.target.clone()],
                                style_destination.clone(),
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
                                .child(self.render_local_style_preview(&style.preview, cx))
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
                        .when(self.can_edit_page() && enabled, move |row| {
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

    pub(super) fn render_page_local_styles(
        &self,
        view_data: &DesignPageLocalStylesViewData,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let page_id = view_data
            .page_id()
            .expect("context-filtered Page local styles");
        let can_create = self.can_edit_page() && view_data.create_disabled_reason.is_none();
        let create_tooltip = view_data
            .create_disabled_reason
            .clone()
            .or_else(|| (!self.can_edit_page()).then(|| SharedString::from("View only")));
        let mut content = v_flex()
            .id(SharedString::from(format!("{}-local-styles", self.id)))
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
            let panel_for_style = cx.entity();
            let panel_for_folder = panel_for_style.clone();
            let style_selector = format!("{}-local-style-{:?}-create", self.id, kind);
            let folder_selector = format!("{}-local-style-{:?}-folder-create", self.id, kind);
            let tooltip = create_tooltip
                .clone()
                .unwrap_or_else(|| SharedString::from(format!("Create {}", kind.label())));
            let root_selector = format!("{}-local-style-{:?}-root-drop", self.id, kind);
            let root_destination = Self::local_style_insertion(kind, None, entries, entries.len())
                .expect("root-end insertion is in bounds");
            let root_drop_background = cx.theme().selection.opacity(0.18);
            let root_drop_border = cx.theme().selection;
            let root_drop_listener = cx.listener(move |this, drag: &LocalStyleDrag, _, cx| {
                cx.stop_propagation();
                this.emit_local_styles_move(
                    vec![drag.target.clone()],
                    root_destination.clone(),
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
                            panel_for_folder.update(cx, |this, cx| {
                                this.emit_local_style_folder_create(kind, Vec::new(), cx);
                            });
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
                            panel_for_style.update(cx, |this, cx| {
                                this.emit_local_style_create(kind, None, cx);
                            });
                        }),
                )
                .when(self.can_edit_page(), move |row| {
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
                .child(self.render_local_style_entries(page_id, kind, None, entries, 0, cx));
        }
        content.into_any_element()
    }

    pub(super) fn render_page_context(&self, cx: &mut Context<Self>) -> AnyElement {
        let page = self.page_view_data_for_context();
        let mut header = h_flex()
            .h(px(40.))
            .px(px(PANEL_PADDING))
            .gap_2()
            .child(div().flex_1().text_sm().font_semibold().child("Page"));
        if let Some(mode_browser) = self.render_variable_mode_popover(cx) {
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
                    .child(self.render_page_background_picker(page, cx)),
            );
            if let Some(local_styles) = self.page_local_styles_view_data_for_context() {
                page_body = page_body.child(self.render_page_local_styles(local_styles, cx));
            }
            if let DesignVariablesEntryPoint::LegacyRightSidebar { disabled_reason } =
                &self.variables_entry_point
            {
                let panel = cx.entity();
                let enabled = disabled_reason.is_none();
                let selector = format!("{}-open-variables", self.id);
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
                            panel.update(cx, |this, cx| {
                                this.emit_variables_view_open(cx);
                            });
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
}
