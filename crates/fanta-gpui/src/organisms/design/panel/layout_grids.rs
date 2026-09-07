use super::*;

impl DesignPanel {
    pub(super) fn layout_grid_target_for_index(&self, index: usize) -> Option<LayoutGridTarget> {
        self.node
            .layout_grids
            .get(index)
            .map(|guide| LayoutGridTarget {
                guide_id: guide.id.clone(),
                index,
            })
    }

    pub(super) fn layout_grid_target_index(&self, target: &LayoutGridTarget) -> Option<usize> {
        if target.guide_id.is_empty() {
            self.node
                .layout_grids
                .get(target.index)
                .map(|_| target.index)
        } else {
            self.node
                .layout_grids
                .iter()
                .position(|guide| guide.id == target.guide_id)
        }
    }

    pub(super) fn reconcile_layout_grid_targets(&mut self) {
        let next_variable_target = self
            .layout_grid_count_variable_target
            .clone()
            .and_then(|target| self.resolve_layout_grid_variable_target(&target))
            .map(|(target, _)| target);
        self.layout_grid_count_variable_target = next_variable_target;

        let next_editor_target = self
            .property_editor
            .as_ref()
            .and_then(|editor| editor.layout_grid_target.clone())
            .and_then(|mut target| {
                target.index = self.layout_grid_target_index(&target)?;
                Some(target)
            });
        if self
            .property_editor
            .as_ref()
            .is_some_and(|editor| editor.layout_grid_target.is_some())
        {
            if let Some(target) = next_editor_target {
                let target_index = target.index;
                if let Some(editor) = self.property_editor.as_mut() {
                    editor.property = editor.property.with_layout_grid_index(target_index);
                    editor.layout_grid_target = Some(target);
                }
                if let Some(scrub) = self
                    .numeric_property_scrub
                    .as_mut()
                    .filter(|scrub| scrub.active)
                {
                    scrub.property = scrub.property.with_layout_grid_index(target_index);
                }
            } else {
                self.property_editor = None;
                self.numeric_property_scrub = None;
                self.property_editor_invalid = false;
                self.suppress_property_input_change = false;
            }
        }
    }

    pub(super) fn layout_grid_edit_target(
        &self,
        property: DesignPanelProperty,
    ) -> Option<LayoutGridTarget> {
        let property_index = property.layout_grid_index()?;
        self.property_editor
            .as_ref()
            .filter(|editor| {
                editor.property.with_layout_grid_index(0) == property.with_layout_grid_index(0)
            })
            .and_then(|editor| editor.layout_grid_target.clone())
            .or_else(|| self.layout_grid_target_for_index(property_index))
    }

    pub(super) fn emit_layout_grid_property_edit(
        &self,
        property: DesignPanelProperty,
        value: DesignPanelValue,
        phase: DesignPanelEditPhase,
        cx: &mut Context<Self>,
    ) -> bool {
        if property.layout_grid_index().is_none() {
            return false;
        }
        if !self.collection_is_supported(DesignPanelCollection::LayoutGrid) {
            return true;
        }
        let Some(target) = self.layout_grid_edit_target(property) else {
            return true;
        };
        let Some(index) = self.layout_grid_target_index(&target) else {
            return true;
        };
        cx.emit_design_panel_action(
            self,
            DesignPanelAction::LayoutGridPropertyEditRequested {
                node_id: self.node.id.clone(),
                guide_id: self.node.layout_grids[index].id.clone(),
                index,
                property: property.with_layout_grid_index(index),
                value,
                phase,
            },
        );
        true
    }

    pub(super) fn layout_grid_variable_target(
        &self,
        index: usize,
        property: DesignPanelProperty,
    ) -> Option<(
        DesignLayoutGridVariableTarget,
        DesignLayoutGridVariableValue,
    )> {
        self.node
            .layout_grids
            .get(index)?
            .variable_target(index, property)
    }

    pub(super) fn layout_grid_variable_target_index(
        &self,
        target: &DesignLayoutGridVariableTarget,
    ) -> Option<usize> {
        self.layout_grid_target_index(&LayoutGridTarget {
            guide_id: target.guide_id.clone(),
            index: target.index,
        })
    }

    pub(super) fn resolve_layout_grid_variable_target(
        &self,
        target: &DesignLayoutGridVariableTarget,
    ) -> Option<(
        DesignLayoutGridVariableTarget,
        DesignLayoutGridVariableValue,
    )> {
        let index = self.layout_grid_variable_target_index(target)?;
        let (resolved, value) =
            self.layout_grid_variable_target(index, target.property.with_layout_grid_index(index))?;
        (resolved.field == target.field
            && resolved.property.with_layout_grid_index(0)
                == target.property.with_layout_grid_index(0))
        .then_some((resolved, value))
    }

    pub(super) fn layout_grid_variable_binding(
        &self,
        target: &DesignLayoutGridVariableTarget,
    ) -> Option<&super::super::DesignLayoutGridVariableBinding> {
        let index = self.layout_grid_variable_target_index(target)?;
        self.node
            .layout_grids
            .get(index)?
            .variable_binding(target.field)
    }

    pub(super) fn layout_grid_variable_can_change(
        &self,
        target: &DesignLayoutGridVariableTarget,
    ) -> bool {
        if !self.can_edit()
            || !self.collection_is_supported(DesignPanelCollection::LayoutGrid)
            || self.node.layout_grid_style_binding.is_some()
            || self.resolve_layout_grid_variable_target(target).is_none()
        {
            return false;
        }
        if self
            .layout_grid_variable_binding(target)
            .is_some_and(|binding| binding.read_only_reason.is_some())
        {
            return false;
        }
        self.property_value_states
            .get(&target.property)
            .is_none_or(|state| {
                !state.is_read_only()
                    && !state
                        .binding()
                        .is_some_and(|binding| binding.kind() == DesignPanelBindingKind::Style)
            })
    }

    pub(super) fn open_layout_grid_style_browser(&mut self, cx: &mut Context<Self>) {
        if !self.collection_is_supported(DesignPanelCollection::LayoutGrid)
            || self.node.layout_grids.is_empty()
        {
            return;
        }
        self.style_browser_source_filter =
            self.style_browser_source_filter.normalized_for_libraries(
                self.layout_grid_style_view_data
                    .libraries
                    .iter()
                    .map(|library| (&library.id, &library.name)),
            );
        self.layout_grid_style_browser_open = true;
        self.paint_style_browser_open = None;
        self.layout_grid_count_variable_target = None;
        self.active_effect_settings = None;
        self.effect_style_browser_open = false;
        self.cancel_menu_preview(cx);
        self.active_picker = None;
        self.typography_style_picker_open = false;
        self.type_settings_open = false;
        self.selection_header_overlay = None;
        cx.notify();
    }

    pub(super) fn open_layout_grid_variable_browser(
        &mut self,
        target: DesignLayoutGridVariableTarget,
        cx: &mut Context<Self>,
    ) {
        if !self.collection_is_supported(DesignPanelCollection::LayoutGrid) {
            return;
        }
        let Some((target, _)) = self.resolve_layout_grid_variable_target(&target) else {
            return;
        };
        self.layout_grid_count_variable_target = Some(target);
        self.layout_grid_style_browser_open = false;
        self.active_effect_settings = None;
        self.effect_style_browser_open = false;
        self.cancel_menu_preview(cx);
        self.active_picker = None;
        self.typography_style_picker_open = false;
        self.type_settings_open = false;
        self.selection_header_overlay = None;
        cx.notify();
    }

    pub(super) fn emit_layout_grid_style_apply(
        &mut self,
        style: DesignLayoutGridStyleSelection,
        cx: &mut Context<Self>,
    ) {
        if !self.can_edit()
            || !self.collection_is_supported(DesignPanelCollection::LayoutGrid)
            || !self
                .layout_grid_style_view_data
                .style(&style)
                .is_some_and(|style| {
                    style.import_state == DesignLayoutGridStyleImportState::Imported
                })
        {
            return;
        }
        self.layout_grid_style_browser_open = false;
        cx.emit_design_panel_action(
            self,
            DesignPanelAction::LayoutGridStyleApplyRequested {
                node_id: self.node.id.clone(),
                style,
            },
        );
        cx.notify();
    }

    pub(super) fn emit_layout_grid_style_import(
        &mut self,
        style: DesignLayoutGridStyleSelection,
        cx: &mut Context<Self>,
    ) {
        if !self.can_edit()
            || !self.collection_is_supported(DesignPanelCollection::LayoutGrid)
            || !matches!(&style.source, DesignLayoutGridStyleSource::Library { .. })
            || !self
                .layout_grid_style_view_data
                .style(&style)
                .is_some_and(|style| {
                    style.import_state == DesignLayoutGridStyleImportState::Available
                })
        {
            return;
        }
        cx.emit_design_panel_action(
            self,
            DesignPanelAction::LayoutGridStyleImportRequested {
                node_id: self.node.id.clone(),
                style,
            },
        );
    }

    pub(super) fn emit_layout_grid_style_create(&mut self, cx: &mut Context<Self>) {
        if !self.can_edit()
            || !self.collection_is_supported(DesignPanelCollection::LayoutGrid)
            || self.node.layout_grids.is_empty()
            || self.node.layout_grid_style_binding.is_some()
        {
            return;
        }
        self.layout_grid_style_browser_open = false;
        cx.emit_design_panel_action(
            self,
            DesignPanelAction::LayoutGridStyleCreateRequested {
                node_id: self.node.id.clone(),
                layout_grids: self.node.layout_grids.clone(),
            },
        );
        cx.notify();
    }

    pub(super) fn emit_layout_grid_style_detach(&mut self, cx: &mut Context<Self>) {
        if !self.can_edit() || !self.collection_is_supported(DesignPanelCollection::LayoutGrid) {
            return;
        }
        let Some(style) = self
            .node
            .layout_grid_style_binding
            .as_ref()
            .filter(|style| style.can_detach)
        else {
            return;
        };
        self.layout_grid_style_browser_open = false;
        cx.emit_design_panel_action(
            self,
            DesignPanelAction::LayoutGridStyleDetachRequested {
                node_id: self.node.id.clone(),
                style: style.selection(),
            },
        );
        cx.notify();
    }

    pub(super) fn emit_layout_grid_variable_apply(
        &mut self,
        target: DesignLayoutGridVariableTarget,
        variable_id: SharedString,
        cx: &mut Context<Self>,
    ) {
        let Some((target, value)) = self.resolve_layout_grid_variable_target(&target) else {
            return;
        };
        let Some(variable) = self
            .layout_grid_variable_view_data
            .variable(variable_id.as_ref())
        else {
            return;
        };
        if !self.layout_grid_variable_can_change(&target)
            || !value.is_compatible(variable)
            || variable.import_state == DesignVariableImportState::Available
        {
            return;
        }
        self.layout_grid_count_variable_target = None;
        cx.emit_design_panel_action(
            self,
            DesignPanelAction::LayoutGridVariableApplyRequested {
                node_id: self.node.id.clone(),
                target,
                variable_id,
            },
        );
        cx.notify();
    }

    pub(super) fn emit_layout_grid_variable_import(
        &mut self,
        target: DesignLayoutGridVariableTarget,
        variable_id: SharedString,
        cx: &mut Context<Self>,
    ) {
        let Some((target, value)) = self.resolve_layout_grid_variable_target(&target) else {
            return;
        };
        let Some(variable) = self
            .layout_grid_variable_view_data
            .variable(variable_id.as_ref())
        else {
            return;
        };
        if !self.layout_grid_variable_can_change(&target)
            || !value.is_compatible(variable)
            || variable.import_state != DesignVariableImportState::Available
        {
            return;
        }
        cx.emit_design_panel_action(
            self,
            DesignPanelAction::LayoutGridVariableImportRequested {
                node_id: self.node.id.clone(),
                target,
                variable_id,
            },
        );
    }

    pub(super) fn emit_layout_grid_variable_detach(
        &mut self,
        target: DesignLayoutGridVariableTarget,
        cx: &mut Context<Self>,
    ) {
        let Some((target, _)) = self.resolve_layout_grid_variable_target(&target) else {
            return;
        };
        let Some(binding) = self
            .layout_grid_variable_binding(&target)
            .filter(|binding| binding.can_detach && binding.read_only_reason.is_none())
        else {
            return;
        };
        if !self.layout_grid_variable_can_change(&target) {
            return;
        }
        let variable_id = binding.variable_id.clone();
        self.layout_grid_count_variable_target = None;
        cx.emit_design_panel_action(
            self,
            DesignPanelAction::LayoutGridVariableDetachRequested {
                node_id: self.node.id.clone(),
                target,
                variable_id,
            },
        );
        cx.notify();
    }

    pub(super) fn emit_layout_grid_variable_create(
        &mut self,
        target: DesignLayoutGridVariableTarget,
        cx: &mut Context<Self>,
    ) {
        let Some((target, value)) = self.resolve_layout_grid_variable_target(&target) else {
            return;
        };
        if !self.layout_grid_variable_can_change(&target)
            || self.layout_grid_variable_binding(&target).is_some()
            || !self
                .layout_grid_variable_view_data
                .create_state
                .is_enabled()
        {
            return;
        }
        self.layout_grid_count_variable_target = None;
        cx.emit_design_panel_action(
            self,
            DesignPanelAction::LayoutGridVariableCreateRequested {
                node_id: self.node.id.clone(),
                target,
                value,
            },
        );
        cx.notify();
    }

    pub(super) fn render_layout_grid_style_button(&self, cx: &mut Context<Self>) -> AnyElement {
        let panel = cx.entity();
        let panel_for_open = panel.clone();
        let panel_for_content = panel;
        let panel_id = self.id.clone();
        let active = self.layout_grid_style_browser_open;
        let binding = self.node.layout_grid_style_binding.clone();
        let can_edit = self.can_edit();
        let can_create = can_edit && !self.node.layout_grids.is_empty() && binding.is_none();
        let query = self.normalized_style_browser_query(cx);
        let source_filter = self.style_browser_source_filter.clone();
        let view_mode = self.style_browser_view_mode;
        let catalog_is_empty = self.layout_grid_style_view_data.page_styles.is_empty()
            && self
                .layout_grid_style_view_data
                .libraries
                .iter()
                .all(|library| library.styles.is_empty());
        let page_styles = if source_filter.includes_page() {
            self.layout_grid_style_view_data
                .page_styles
                .iter()
                .map(|style| {
                    (
                        style.name.clone(),
                        DesignLayoutGridStyleSelection::page(style.id.clone()),
                        style.import_state,
                        style
                            .layout_grids
                            .iter()
                            .map(|guide| guide.kind().label())
                            .collect::<Vec<_>>()
                            .join(", "),
                    )
                })
                .filter(|(name, _, _, summary)| {
                    Self::style_browser_row_matches(&query, name.as_ref(), summary, None)
                })
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        let libraries = self
            .layout_grid_style_view_data
            .libraries
            .iter()
            .filter(|library| source_filter.includes_library(library.id.as_ref()))
            .map(|library| {
                (
                    library.name.clone(),
                    library
                        .styles
                        .iter()
                        .map(|style| {
                            (
                                style.name.clone(),
                                DesignLayoutGridStyleSelection::library(
                                    library.id.clone(),
                                    style.id.clone(),
                                ),
                                style.import_state,
                                style
                                    .layout_grids
                                    .iter()
                                    .map(|guide| guide.kind().label())
                                    .collect::<Vec<_>>()
                                    .join(", "),
                            )
                        })
                        .filter(|(name, _, _, summary)| {
                            Self::style_browser_row_matches(
                                &query,
                                name.as_ref(),
                                summary,
                                Some(library.name.as_ref()),
                            )
                        })
                        .collect::<Vec<_>>(),
                )
            })
            .filter(|(_, styles)| !styles.is_empty())
            .collect::<Vec<_>>();
        let style_browser_search = self.style_browser_search.clone();
        let style_browser_library_sources = self
            .layout_grid_style_view_data
            .libraries
            .iter()
            .map(|library| (library.id.clone(), library.name.clone()))
            .collect::<Vec<_>>();
        let style_browser_scope =
            SharedString::from(format!("{panel_id}-layout-guide-style-browser"));
        let tooltip = binding.as_ref().map_or_else(
            || "Layout guide styles".into(),
            |binding| binding.style_name.clone(),
        );
        let trigger = Button::new(SharedString::from(format!(
            "{}-layout-guide-styles",
            self.id
        )))
        .tooltip(tooltip)
        .xsmall()
        .compact()
        .ghost()
        .w(px(24.))
        .h(px(24.))
        .selected(active || binding.is_some())
        .child(self.render_color_styles_icon(cx))
        .on_activate(cx.listener(|this, _, _, cx| {
            cx.stop_propagation();
            this.open_layout_grid_style_browser(cx);
        }));

        Popover::new(SharedString::from(format!(
            "{}-layout-guide-style-popover",
            self.id
        )))
        .anchor(Anchor::TopRight)
        .open(active)
        .overlay_closable(true)
        .on_open_change(move |open, _, cx| {
            panel_for_open.update(cx, |this, cx| {
                if *open {
                    this.open_layout_grid_style_browser(cx);
                } else if this.layout_grid_style_browser_open {
                    this.layout_grid_style_browser_open = false;
                    cx.notify();
                }
            });
        })
        .trigger(trigger)
        .content(move |_, window, cx| {
            let mut content = v_flex()
                .w(popup_width(window, 272.))
                .max_h(popup_height(window, 520.))
                .overflow_y_scrollbar()
                .gap_1()
                .child(
                    div()
                        .px_2()
                        .py_1()
                        .text_xs()
                        .font_semibold()
                        .child("Layout guide styles"),
                );
            if let Some(binding) = binding.clone() {
                let panel = panel_for_content.clone();
                content = content.child(
                    h_flex()
                        .w_full()
                        .px_2()
                        .gap_2()
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.))
                                .truncate()
                                .text_xs()
                                .child(binding.style_name),
                        )
                        .child(
                            Button::new(SharedString::from(format!(
                                "{panel_id}-detach-layout-guide-style"
                            )))
                            .label(if binding.can_detach {
                                "Detach"
                            } else {
                                "Bound"
                            })
                            .xsmall()
                            .compact()
                            .ghost()
                            .disabled(!can_edit || !binding.can_detach)
                            .on_activate(move |_, _, cx| {
                                panel.update(cx, |this, cx| {
                                    this.emit_layout_grid_style_detach(cx);
                                });
                            }),
                        ),
                );
            }
            let panel = panel_for_content.clone();
            content = content.child(
                Button::new(SharedString::from(format!(
                    "{panel_id}-create-layout-guide-style"
                )))
                .label("Create style from guides")
                .xsmall()
                .compact()
                .ghost()
                .w_full()
                .disabled(!can_create)
                .on_activate(move |_, _, cx| {
                    panel.update(cx, |this, cx| this.emit_layout_grid_style_create(cx));
                }),
            );
            content = content.child(div().px_2().child(Self::render_style_browser_toolbar(
                panel_for_content.clone(),
                style_browser_scope.clone(),
                style_browser_search.clone(),
                source_filter.clone(),
                style_browser_library_sources.clone(),
                view_mode,
            )));
            if page_styles.is_empty() && libraries.iter().all(|(_, styles)| styles.is_empty()) {
                content = content.child(
                    div()
                        .px_2()
                        .py_2()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(if catalog_is_empty {
                            "No Grid styles supplied by the host"
                        } else {
                            "No styles match this search and source filter"
                        }),
                );
            }
            if !page_styles.is_empty() {
                content = content.child(
                    div()
                        .px_2()
                        .pt_2()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child("This page"),
                );
                let mut rows = if view_mode == StyleBrowserViewMode::Grid {
                    h_flex().w_full().px_2().gap_1().flex_wrap()
                } else {
                    v_flex().w_full().px_2().gap_0p5()
                };
                for (row_index, (name, selection, import_state, summary)) in
                    page_styles.clone().into_iter().enumerate()
                {
                    let panel = panel_for_content.clone();
                    let imported = import_state == DesignLayoutGridStyleImportState::Imported;
                    rows = rows.child(
                        Button::new(SharedString::from(format!(
                            "{panel_id}-layout-guide-style-page-{row_index}"
                        )))
                        .label(name)
                        .tooltip(summary)
                        .xsmall()
                        .compact()
                        .ghost()
                        .when(view_mode == StyleBrowserViewMode::Grid, |button| {
                            button.w(px(124.)).h(px(44.))
                        })
                        .when(view_mode == StyleBrowserViewMode::List, |button| {
                            button.w_full()
                        })
                        .disabled(!can_edit || !imported)
                        .on_activate(move |_, _, cx| {
                            panel.update(cx, |this, cx| {
                                this.emit_layout_grid_style_apply(selection.clone(), cx);
                            });
                        }),
                    );
                }
                content = content.child(rows);
            }
            for (library_index, (library_name, styles)) in libraries.clone().into_iter().enumerate()
            {
                content = content.child(
                    div()
                        .px_2()
                        .pt_2()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(library_name),
                );
                let mut rows = if view_mode == StyleBrowserViewMode::Grid {
                    h_flex().w_full().px_2().gap_1().flex_wrap()
                } else {
                    v_flex().w_full().px_2().gap_0p5()
                };
                for (style_index, (name, selection, import_state, summary)) in
                    styles.into_iter().enumerate()
                {
                    let panel = panel_for_content.clone();
                    let imported = import_state == DesignLayoutGridStyleImportState::Imported;
                    let label = if imported {
                        name
                    } else {
                        SharedString::from(format!("{name} · Import"))
                    };
                    rows = rows.child(
                        Button::new(SharedString::from(format!(
                            "{panel_id}-layout-guide-style-library-{library_index}-{style_index}"
                        )))
                        .label(label)
                        .tooltip(summary)
                        .xsmall()
                        .compact()
                        .ghost()
                        .when(view_mode == StyleBrowserViewMode::Grid, |button| {
                            button.w(px(124.)).h(px(44.))
                        })
                        .when(view_mode == StyleBrowserViewMode::List, |button| {
                            button.w_full()
                        })
                        .disabled(!can_edit)
                        .on_activate(move |_, _, cx| {
                            panel.update(cx, |this, cx| {
                                if imported {
                                    this.emit_layout_grid_style_apply(selection.clone(), cx);
                                } else {
                                    this.emit_layout_grid_style_import(selection.clone(), cx);
                                }
                            });
                        }),
                    );
                }
                content = content.child(rows);
            }
            content
        })
        .into_any_element()
    }

    pub(super) fn layout_grid_variable_targets_match(
        left: &DesignLayoutGridVariableTarget,
        right: &DesignLayoutGridVariableTarget,
    ) -> bool {
        let guide_matches = if left.guide_id.is_empty() || right.guide_id.is_empty() {
            left.index == right.index
        } else {
            left.guide_id == right.guide_id
        };
        guide_matches
            && left.field == right.field
            && left.property.with_layout_grid_index(0) == right.property.with_layout_grid_index(0)
    }

    pub(super) fn layout_grid_variable_label(
        &self,
        target: &DesignLayoutGridVariableTarget,
    ) -> &'static str {
        match target.property.with_layout_grid_index(0) {
            DesignPanelProperty::LayoutGridCount(_) => "Count",
            DesignPanelProperty::LayoutGridOffset(_) => "Offset",
            DesignPanelProperty::LayoutGridMargin(_) => "Margin",
            DesignPanelProperty::LayoutGridGutter(_) => "Gutter",
            DesignPanelProperty::LayoutGridSize(_) => {
                match self
                    .node
                    .layout_grids
                    .get(target.index)
                    .map(|guide| guide.kind())
                {
                    Some(DesignGridKind::Columns) => "Width",
                    Some(DesignGridKind::Rows) => "Height",
                    Some(DesignGridKind::Uniform) | None => "Size",
                }
            }
            _ => "Number",
        }
    }

    pub(super) fn render_layout_grid_variable_cell(
        &self,
        index: usize,
        property: DesignPanelProperty,
        cell: AnyElement,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some((target, value)) = self.layout_grid_variable_target(index, property) else {
            return cell;
        };
        h_flex()
            .flex_1()
            .min_w(px(0.))
            .gap_1()
            .child(cell)
            .child(self.render_layout_grid_variable_button(target, value, cx))
            .into_any_element()
    }

    pub(super) fn render_layout_grid_variable_button(
        &self,
        target: DesignLayoutGridVariableTarget,
        value: DesignLayoutGridVariableValue,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let binding = self.layout_grid_variable_binding(&target).cloned();
        let active = self
            .layout_grid_count_variable_target
            .as_ref()
            .is_some_and(|current| Self::layout_grid_variable_targets_match(current, &target));
        let can_change = self.layout_grid_variable_can_change(&target);
        let variables = self.layout_grid_variable_view_data.variables.clone();
        let create_state = self.layout_grid_variable_view_data.create_state.clone();
        let panel = cx.entity();
        let panel_for_open = panel.clone();
        let panel_for_content = panel;
        let panel_id = self.id.clone();
        let target_for_open = target.clone();
        let target_for_content = target.clone();
        let index = target.index;
        let property_key =
            format!("{:?}", target.property.with_layout_grid_index(0)).to_ascii_lowercase();
        let label = self.layout_grid_variable_label(&target);
        let title = SharedString::from(format!("{label} variables"));
        let change_reason = if !self.can_edit() {
            Some(SharedString::from("View only"))
        } else if self.node.layout_grid_style_binding.is_some() {
            Some(SharedString::from(
                "Detach the Grid style before binding a variable",
            ))
        } else if let Some(reason) = binding
            .as_ref()
            .and_then(|binding| binding.read_only_reason.clone())
        {
            Some(reason)
        } else {
            self.property_value_states
                .get(&target.property)
                .and_then(|state| match state {
                    DesignPanelPropertyValueState::ReadOnly(value) => value
                        .reason()
                        .map(|reason| SharedString::from(reason.to_owned()))
                        .or_else(|| Some("Property is read only".into())),
                    _ if state
                        .binding()
                        .is_some_and(|binding| binding.kind() == DesignPanelBindingKind::Style) =>
                    {
                        Some("Detach the style before binding a variable".into())
                    }
                    _ => None,
                })
        };
        let tooltip = binding.as_ref().map_or_else(
            || change_reason.clone().unwrap_or_else(|| title.clone()),
            |binding| {
                SharedString::from(format!(
                    "{} · {}",
                    binding
                        .collection_name
                        .as_ref()
                        .map_or("Variables", |name| name.as_ref()),
                    binding.variable_name
                ))
            },
        );
        let trigger = Button::new(SharedString::from(format!(
            "{panel_id}-layout-guide-variable-{index}-{property_key}"
        )))
        .tooltip(tooltip)
        .xsmall()
        .compact()
        .ghost()
        .w(px(24.))
        .h(px(24.))
        .selected(active || binding.is_some())
        .child(div().text_xs().child("◇"))
        .on_activate(cx.listener(move |this, _, _, cx| {
            cx.stop_propagation();
            this.open_layout_grid_variable_browser(target_for_open.clone(), cx);
        }));

        Popover::new(SharedString::from(format!(
            "{panel_id}-layout-guide-variable-popover-{index}-{property_key}"
        )))
        .anchor(Anchor::TopRight)
        .open(active)
        .overlay_closable(true)
        .on_open_change(move |open, _, cx| {
            let target = target.clone();
            panel_for_open.update(cx, |this, cx| {
                if *open {
                    this.open_layout_grid_variable_browser(target, cx);
                } else if this
                    .layout_grid_count_variable_target
                    .as_ref()
                    .is_some_and(|current| {
                        Self::layout_grid_variable_targets_match(current, &target)
                    })
                {
                    this.layout_grid_count_variable_target = None;
                    cx.notify();
                }
            });
        })
        .trigger(trigger)
        .content(move |_, window, cx| {
            let mut content = v_flex().w(popup_width(window, 292.)).gap_1().child(
                div()
                    .px_2()
                    .py_1()
                    .text_xs()
                    .font_semibold()
                    .child(title.clone()),
            );
            if let Some(binding) = binding.clone() {
                let panel = panel_for_content.clone();
                let target = target_for_content.clone();
                let collection = binding
                    .collection_name
                    .clone()
                    .unwrap_or_else(|| "Variables".into());
                content = content.child(
                    h_flex()
                        .w_full()
                        .px_2()
                        .gap_2()
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.))
                                .truncate()
                                .text_xs()
                                .child(format!("{collection} · {}", binding.variable_name)),
                        )
                        .child(
                            Button::new(SharedString::from(format!(
                                "{panel_id}-detach-layout-guide-variable-{index}-{property_key}"
                            )))
                            .label("Detach")
                            .tooltip(
                                binding
                                    .read_only_reason
                                    .clone()
                                    .unwrap_or_else(|| "Detach variable".into()),
                            )
                            .xsmall()
                            .compact()
                            .ghost()
                            .disabled(
                                !can_change
                                    || !binding.can_detach
                                    || binding.read_only_reason.is_some(),
                            )
                            .on_activate(move |_, _, cx| {
                                panel.update(cx, |this, cx| {
                                    this.emit_layout_grid_variable_detach(target.clone(), cx);
                                });
                            }),
                        ),
                );
            }
            if create_state.is_visible() {
                let panel = panel_for_content.clone();
                let target = target_for_content.clone();
                let create_reason = create_state
                    .disabled_reason()
                    .cloned()
                    .or_else(|| change_reason.clone())
                    .or_else(|| {
                        binding
                            .as_ref()
                            .map(|_| SharedString::from("Detach the variable before creating one"))
                    });
                content = content.child(
                    Button::new(SharedString::from(format!(
                        "{panel_id}-create-layout-guide-variable-{index}-{property_key}"
                    )))
                    .label(format!("Create variable from {}", value.label()))
                    .tooltip(
                        create_reason
                            .clone()
                            .unwrap_or_else(|| "Create Number variable".into()),
                    )
                    .xsmall()
                    .compact()
                    .ghost()
                    .w_full()
                    .disabled(
                        !can_change
                            || binding.is_some()
                            || !create_state.is_enabled()
                            || create_reason.is_some(),
                    )
                    .on_activate(move |_, _, cx| {
                        panel.update(cx, |this, cx| {
                            this.emit_layout_grid_variable_create(target.clone(), cx);
                        });
                    }),
                );
            }
            if variables.is_empty() {
                content = content.child(
                    div()
                        .px_2()
                        .py_2()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child("No Number variables supplied"),
                );
            }
            for (variable_index, variable) in variables.clone().into_iter().enumerate() {
                let panel = panel_for_content.clone();
                let target = target_for_content.clone();
                let variable_id = variable.id.clone();
                let compatibility_reason = value.compatibility_reason(&variable);
                let available = variable.import_state == DesignVariableImportState::Available;
                let selected = binding
                    .as_ref()
                    .is_some_and(|binding| binding.variable_id == variable.id);
                let tooltip = if let Some(reason) = &change_reason {
                    reason.clone()
                } else if let Some(reason) = &compatibility_reason {
                    reason.clone()
                } else {
                    let resolved = match variable.resolved_value.as_ref() {
                        Some(super::super::DesignVariableResolvedValue::Float(value))
                            if value.is_infinite() =>
                        {
                            "Auto".into()
                        }
                        Some(super::super::DesignVariableResolvedValue::Float(value)) => {
                            SharedString::from(format_number(*value))
                        }
                        _ => "Unresolved".into(),
                    };
                    SharedString::from(format!("Resolved · {resolved}"))
                };
                let row_label = if available {
                    format!("{} · {} · Import", variable.collection_name, variable.name)
                } else {
                    format!("{} · {}", variable.collection_name, variable.name)
                };
                content = content.child(
                    Button::new(SharedString::from(format!(
                        "{panel_id}-layout-guide-variable-{index}-{property_key}-{variable_index}"
                    )))
                    .label(row_label)
                    .tooltip(tooltip)
                    .xsmall()
                    .compact()
                    .ghost()
                    .w_full()
                    .selected(selected)
                    .disabled(!can_change || compatibility_reason.is_some())
                    .on_activate(move |_, _, cx| {
                        panel.update(cx, |this, cx| {
                            if available {
                                this.emit_layout_grid_variable_import(
                                    target.clone(),
                                    variable_id.clone(),
                                    cx,
                                );
                            } else {
                                this.emit_layout_grid_variable_apply(
                                    target.clone(),
                                    variable_id.clone(),
                                    cx,
                                );
                            }
                        });
                    }),
                );
            }
            content
        })
        .into_any_element()
    }

    pub(super) fn render_layout_grids(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        (self.node.supports_layout_guides()
            && self.node.supports_section(DesignPanelSection::LayoutGrid))
        .then(|| {
            if let Some(binding) = self.node.layout_grid_style_binding.as_ref() {
                let content =
                    v_flex()
                        .px(px(PANEL_PADDING))
                        .pb_4()
                        .child(self.render_bound_style_summary(
                            "layout-guides",
                            binding.style_name.clone(),
                            cx,
                        ));
                return self.render_section(
                    DesignPanelSection::LayoutGrid,
                    Some(DesignPanelCollection::LayoutGrid),
                    content.into_any_element(),
                    cx,
                );
            }
            let mut content = v_flex().px(px(PANEL_PADDING)).pb_4().gap_2();
            for (index, grid) in self.node.layout_grids.iter().enumerate() {
                let next_kind = match grid.kind() {
                    DesignGridKind::Uniform => DesignGridKind::Columns,
                    DesignGridKind::Columns => DesignGridKind::Rows,
                    DesignGridKind::Rows => DesignGridKind::Uniform,
                };
                let mut editor = v_flex()
                    .w_full()
                    .p_2()
                    .gap_2()
                    .rounded(px(6.))
                    .border_1()
                    .border_color(cx.theme().border)
                    .child(
                        h_flex()
                            .w_full()
                            .gap_2()
                            .child(self.render_visibility_button(
                                format!("layout-guide-visible-{index}"),
                                grid.visible,
                                DesignPanelProperty::LayoutGridVisible(index),
                                cx,
                            ))
                            .child(
                                div()
                                    .size(px(18.))
                                    .flex_none()
                                    .rounded(px(3.))
                                    .border_1()
                                    .border_color(cx.theme().border)
                                    .bg(color_hsla(grid.color))
                                    .opacity((grid.opacity / 100.).clamp(0., 1.)),
                            )
                            .child(self.render_value_cell(
                                format!("layout-guide-kind-{index}"),
                                "▦",
                                grid.kind().label(),
                                DesignPanelProperty::LayoutGridKind(index),
                                DesignPanelValue::GridKind(next_kind),
                                cx,
                            ))
                            .child(self.render_remove_button(
                                format!("remove-layout-guide-{index}"),
                                DesignPanelCollection::LayoutGrid,
                                index,
                                cx,
                            )),
                    );

                match &grid.settings {
                    DesignLayoutGridSettings::Uniform(settings) => {
                        let property = DesignPanelProperty::LayoutGridSize(index);
                        let cell = self.render_value_cell(
                            format!("layout-guide-size-{index}"),
                            "S",
                            format!("Size · {}", format_number(settings.size)),
                            property,
                            DesignPanelValue::Number(settings.size + 1.),
                            cx,
                        );
                        editor = editor.child(
                            self.render_layout_grid_variable_cell(index, property, cell, cx),
                        );
                    }
                    DesignLayoutGridSettings::Columns(settings) => {
                        let next_count = match settings.count {
                            DesignLayoutGridCount::Auto => DesignLayoutGridCount::number(12),
                            DesignLayoutGridCount::Number(count) => {
                                DesignLayoutGridCount::number(count.saturating_add(1))
                            }
                        };
                        editor = editor.child(
                            h_flex()
                                .w_full()
                                .gap_2()
                                .child(self.render_value_cell(
                                    format!("layout-guide-alignment-{index}"),
                                    "A",
                                    settings.alignment.label(),
                                    DesignPanelProperty::LayoutGridAlignment(index),
                                    DesignPanelValue::ColumnGridAlignment(
                                        DesignColumnGridAlignment::Stretch,
                                    ),
                                    cx,
                                ))
                                .child({
                                    let property = DesignPanelProperty::LayoutGridCount(index);
                                    let cell = self.render_value_cell(
                                        format!("layout-guide-count-{index}"),
                                        "#",
                                        settings.count.label(),
                                        property,
                                        DesignPanelValue::LayoutGridCount(next_count),
                                        cx,
                                    );
                                    self.render_layout_grid_variable_cell(index, property, cell, cx)
                                }),
                        );
                        if settings.alignment.is_stretch() {
                            editor = editor.child(
                                h_flex()
                                    .w_full()
                                    .gap_2()
                                    .child(self.render_value_cell(
                                        format!("layout-guide-size-{index}"),
                                        "W",
                                        "Size · Auto",
                                        DesignPanelProperty::LayoutGridSize(index),
                                        DesignPanelValue::Number(settings.size),
                                        cx,
                                    ))
                                    .child({
                                        let property = DesignPanelProperty::LayoutGridMargin(index);
                                        let cell = self.render_value_cell(
                                            format!("layout-guide-margin-{index}"),
                                            "M",
                                            format!("Margin · {}", format_number(settings.margin)),
                                            property,
                                            DesignPanelValue::Number(settings.margin + 1.),
                                            cx,
                                        );
                                        self.render_layout_grid_variable_cell(
                                            index, property, cell, cx,
                                        )
                                    }),
                            );
                        } else {
                            let property = DesignPanelProperty::LayoutGridSize(index);
                            let cell = self.render_value_cell(
                                format!("layout-guide-size-{index}"),
                                "W",
                                format!("Width · {}", format_number(settings.size)),
                                property,
                                DesignPanelValue::Number(settings.size + 1.),
                                cx,
                            );
                            editor = editor.child(
                                self.render_layout_grid_variable_cell(index, property, cell, cx),
                            );
                            if settings.alignment.supports_offset() {
                                let property = DesignPanelProperty::LayoutGridOffset(index);
                                let cell = self.render_value_cell(
                                    format!("layout-guide-offset-{index}"),
                                    "O",
                                    format!("Offset · {}", format_number(settings.offset)),
                                    property,
                                    DesignPanelValue::Number(settings.offset + 1.),
                                    cx,
                                );
                                editor =
                                    editor.child(self.render_layout_grid_variable_cell(
                                        index, property, cell, cx,
                                    ));
                            }
                        }
                        let property = DesignPanelProperty::LayoutGridGutter(index);
                        let cell = self.render_value_cell(
                            format!("layout-guide-gutter-{index}"),
                            "G",
                            format!("Gutter · {}", format_number(settings.gutter)),
                            property,
                            DesignPanelValue::Number(settings.gutter + 1.),
                            cx,
                        );
                        editor = editor.child(
                            self.render_layout_grid_variable_cell(index, property, cell, cx),
                        );
                    }
                    DesignLayoutGridSettings::Rows(settings) => {
                        let next_count = match settings.count {
                            DesignLayoutGridCount::Auto => DesignLayoutGridCount::number(8),
                            DesignLayoutGridCount::Number(count) => {
                                DesignLayoutGridCount::number(count.saturating_add(1))
                            }
                        };
                        editor = editor.child(
                            h_flex()
                                .w_full()
                                .gap_2()
                                .child(self.render_value_cell(
                                    format!("layout-guide-alignment-{index}"),
                                    "A",
                                    settings.alignment.label(),
                                    DesignPanelProperty::LayoutGridAlignment(index),
                                    DesignPanelValue::RowGridAlignment(
                                        DesignRowGridAlignment::Stretch,
                                    ),
                                    cx,
                                ))
                                .child({
                                    let property = DesignPanelProperty::LayoutGridCount(index);
                                    let cell = self.render_value_cell(
                                        format!("layout-guide-count-{index}"),
                                        "#",
                                        settings.count.label(),
                                        property,
                                        DesignPanelValue::LayoutGridCount(next_count),
                                        cx,
                                    );
                                    self.render_layout_grid_variable_cell(index, property, cell, cx)
                                }),
                        );
                        if settings.alignment.is_stretch() {
                            editor = editor.child(
                                h_flex()
                                    .w_full()
                                    .gap_2()
                                    .child(self.render_value_cell(
                                        format!("layout-guide-size-{index}"),
                                        "H",
                                        "Size · Auto",
                                        DesignPanelProperty::LayoutGridSize(index),
                                        DesignPanelValue::Number(settings.size),
                                        cx,
                                    ))
                                    .child({
                                        let property = DesignPanelProperty::LayoutGridMargin(index);
                                        let cell = self.render_value_cell(
                                            format!("layout-guide-margin-{index}"),
                                            "M",
                                            format!("Margin · {}", format_number(settings.margin)),
                                            property,
                                            DesignPanelValue::Number(settings.margin + 1.),
                                            cx,
                                        );
                                        self.render_layout_grid_variable_cell(
                                            index, property, cell, cx,
                                        )
                                    }),
                            );
                        } else {
                            let property = DesignPanelProperty::LayoutGridSize(index);
                            let cell = self.render_value_cell(
                                format!("layout-guide-size-{index}"),
                                "H",
                                format!("Height · {}", format_number(settings.size)),
                                property,
                                DesignPanelValue::Number(settings.size + 1.),
                                cx,
                            );
                            editor = editor.child(
                                self.render_layout_grid_variable_cell(index, property, cell, cx),
                            );
                            if settings.alignment.supports_offset() {
                                let property = DesignPanelProperty::LayoutGridOffset(index);
                                let cell = self.render_value_cell(
                                    format!("layout-guide-offset-{index}"),
                                    "O",
                                    format!("Offset · {}", format_number(settings.offset)),
                                    property,
                                    DesignPanelValue::Number(settings.offset + 1.),
                                    cx,
                                );
                                editor =
                                    editor.child(self.render_layout_grid_variable_cell(
                                        index, property, cell, cx,
                                    ));
                            }
                        }
                        let property = DesignPanelProperty::LayoutGridGutter(index);
                        let cell = self.render_value_cell(
                            format!("layout-guide-gutter-{index}"),
                            "G",
                            format!("Gutter · {}", format_number(settings.gutter)),
                            property,
                            DesignPanelValue::Number(settings.gutter + 1.),
                            cx,
                        );
                        editor = editor.child(
                            self.render_layout_grid_variable_cell(index, property, cell, cx),
                        );
                    }
                }

                editor = editor.child(
                    h_flex()
                        .w_full()
                        .gap_2()
                        .child(div().flex_1().min_w(px(0.)).child(
                            self.render_auxiliary_color_picker_control(
                                cx.entity(),
                                format!("layout-guide-color-{index}"),
                                format!("#{}", grid.color.hex()),
                                grid.color,
                                AuxiliaryColorPickerTarget::LayoutGrid {
                                    node_id: self.node.id.clone(),
                                    guide_id: grid.id.clone(),
                                    index,
                                },
                                cx,
                            ),
                        ))
                        .child(self.render_value_cell(
                            format!("layout-guide-opacity-{index}"),
                            "%",
                            format_number(grid.opacity),
                            DesignPanelProperty::LayoutGridOpacity(index),
                            DesignPanelValue::Number((grid.opacity + 10.).min(100.)),
                            cx,
                        )),
                );
                content = content.child(editor);
            }
            if self.node.layout_grids.is_empty() {
                content = content.child(empty_collection("No layout guides", cx));
            }
            self.render_section(
                DesignPanelSection::LayoutGrid,
                Some(DesignPanelCollection::LayoutGrid),
                content.into_any_element(),
                cx,
            )
        })
    }
}
