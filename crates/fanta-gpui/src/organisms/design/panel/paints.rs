use super::*;

impl DesignPanel {
    pub(super) fn emit_page_background_edit(
        &self,
        color: DesignColor,
        phase: DesignPanelEditPhase,
        cx: &mut Context<Self>,
    ) {
        let Some(page) = self.page_view_data_for_context() else {
            return;
        };
        if !self.can_edit_page() || page.background.read_only {
            return;
        }
        cx.emit_design_panel_action(
            self,
            DesignPanelAction::PageBackgroundEditRequested {
                page_id: page.page_id.clone(),
                color,
                phase,
            },
        );
    }

    pub(super) fn paint_target(&self, collection: DesignPanelCollection) -> DesignPaintTarget {
        if collection == DesignPanelCollection::Fill
            && self.inspection_context.edit_mode() == DesignPanelEditMode::Text
            && matches!(
                self.node.kind,
                DesignPanelNodeKind::Text | DesignPanelNodeKind::TextPath
            )
        {
            self.inspection_context.text_range_revision().map_or(
                DesignPaintTarget::SelectedTextRange,
                DesignPaintTarget::SelectedTextRangeRevision,
            )
        } else {
            DesignPaintTarget::WholeLayer
        }
    }

    pub(super) fn paint_edit_for_property(
        &self,
        property: DesignPanelProperty,
        value: &DesignPanelValue,
    ) -> Option<(PaintPickerTarget, DesignPaintEdit)> {
        let (collection, index, edit) = match (property, value) {
            (
                DesignPanelProperty::PaintOpacity { collection, index },
                DesignPanelValue::Number(value),
            ) if value.is_finite() => (
                collection,
                index,
                DesignPaintEdit {
                    property: DesignPaintProperty::Opacity,
                    value: DesignPaintValue::Number(*value),
                },
            ),
            (
                DesignPanelProperty::PaintVisible { collection, index },
                DesignPanelValue::Bool(value),
            ) => (
                collection,
                index,
                DesignPaintEdit {
                    property: DesignPaintProperty::Visible,
                    value: DesignPaintValue::Bool(*value),
                },
            ),
            _ => return None,
        };
        let paint = self.paint_collection(collection)?.get(index)?;
        Some((
            PaintPickerTarget {
                collection,
                index,
                paint_id: paint.id.clone(),
            },
            edit,
        ))
    }

    pub(super) fn emit_paint_edit(
        &mut self,
        target: PaintPickerTarget,
        edit: DesignPaintEdit,
        phase: DesignPanelEditPhase,
        cx: &mut Context<Self>,
    ) {
        if phase == DesignPanelEditPhase::Commit {
            self.cancel_menu_preview(cx);
        }
        if !self.can_edit()
            || !self.collection_is_supported(target.collection)
            || self.paint_style_binding(target.collection).is_some()
        {
            return;
        }
        let Some(index) = self.paint_target_index(&target) else {
            return;
        };
        if phase != DesignPanelEditPhase::Cancel && !self.paint_edit_is_applicable(&target, &edit) {
            return;
        }
        cx.emit_design_panel_action(
            self,
            DesignPanelAction::PaintEditRequested {
                node_id: self.node.id.clone(),
                collection: target.collection,
                target: self.paint_target(target.collection),
                paint_id: target.paint_id,
                index,
                edit,
                phase,
            },
        );
    }

    pub(super) fn paint_edit_is_applicable(
        &self,
        target: &PaintPickerTarget,
        edit: &DesignPaintEdit,
    ) -> bool {
        let Some(mut paint) = self.picker_paint(target) else {
            return false;
        };
        if paint.read_only {
            return false;
        }
        if let (
            DesignPaintProperty::ShaderProperty { definition_id },
            DesignPaintValue::ShaderProperty(value),
            DesignPaintPayload::Shader(shader),
        ) = (&edit.property, &edit.value, &paint.payload)
        {
            let Some(definition) = self
                .shader_view_data
                .definition(shader.shader_id.as_ref())
                .and_then(|shader| shader.property(definition_id.as_ref()))
            else {
                return false;
            };
            if !value.is_compatible_with(definition.kind) {
                return false;
            }
        }
        paint.apply_edit(edit)
    }

    pub(super) fn track_paint_edit(
        &mut self,
        target: &PickerEventTarget,
        edit: &DesignPaintEdit,
        phase: DesignPanelEditPhase,
    ) -> bool {
        // A picker transaction snapshots the complete paint. Its previews may
        // legitimately cross leaf properties (for example color, then alpha)
        // as long as the exact stable paint target remains unchanged.
        let matches_active = self
            .active_paint_edit
            .as_ref()
            .is_some_and(|active| active.target == *target);
        match phase {
            DesignPanelEditPhase::Begin if self.active_paint_edit.is_none() => {
                self.active_paint_edit = Some(ActivePaintEdit {
                    target: target.clone(),
                    edit: edit.clone(),
                });
                true
            }
            DesignPanelEditPhase::Begin => false,
            DesignPanelEditPhase::Preview if matches_active => {
                if let Some(active) = self.active_paint_edit.as_mut() {
                    active.edit = edit.clone();
                }
                true
            }
            DesignPanelEditPhase::Preview => false,
            DesignPanelEditPhase::Commit if self.active_paint_edit.is_none() => {
                // Picker buttons and toggles are atomic Commit-only edits.
                true
            }
            DesignPanelEditPhase::Commit | DesignPanelEditPhase::Cancel if matches_active => {
                self.active_paint_edit = None;
                true
            }
            DesignPanelEditPhase::Commit | DesignPanelEditPhase::Cancel => false,
        }
    }

    pub(super) fn auxiliary_color(
        &self,
        target: &AuxiliaryColorPickerTarget,
    ) -> Option<DesignColor> {
        match target {
            AuxiliaryColorPickerTarget::PageBackground { page_id } => self
                .page_view_data_for_context()
                .filter(|page| page.page_id == *page_id)
                .map(|page| page.background.color),
            AuxiliaryColorPickerTarget::TextDecoration { node_id, .. } => {
                if self.node.id != *node_id {
                    return None;
                }
                self.node
                    .typography
                    .as_ref()?
                    .decoration_details
                    .as_ref()
                    .map(|details| match details.color {
                        DesignTextDecorationColor::Auto => DesignColor::BLACK,
                        DesignTextDecorationColor::Solid(color) => color,
                    })
            }
            AuxiliaryColorPickerTarget::Effect {
                node_id,
                effect_id,
                index,
                property,
            } => {
                if self.node.id != *node_id {
                    return None;
                }
                let resolved_index = if effect_id.is_empty() {
                    (*index < self.node.effects.len()).then_some(*index)
                } else {
                    self.node
                        .effects
                        .iter()
                        .position(|effect| effect.id == *effect_id)
                }?;
                match self.current_property_value(property.with_effect_index(resolved_index))? {
                    DesignPanelValue::Color(color) => Some(color),
                    _ => None,
                }
            }
            AuxiliaryColorPickerTarget::LayoutGrid {
                node_id,
                guide_id,
                index,
            } => {
                if self.node.id != *node_id {
                    return None;
                }
                let resolved_index = if guide_id.is_empty() {
                    (*index < self.node.layout_grids.len()).then_some(*index)
                } else {
                    self.node
                        .layout_grids
                        .iter()
                        .position(|guide| guide.id == *guide_id)
                }?;
                let guide = self.node.layout_grids.get(resolved_index)?;
                Some(DesignColor::rgba(
                    guide.color.red,
                    guide.color.green,
                    guide.color.blue,
                    (guide.opacity.clamp(0., 100.) / 100. * f32::from(u8::MAX)).round() as u8,
                ))
            }
            AuxiliaryColorPickerTarget::SelectionColor {
                selection_color_id, ..
            } => self
                .node
                .resolved_selection_colors()
                .iter()
                .find(|color| color.id == *selection_color_id)
                .map(|color| color.color),
        }
    }

    pub(super) fn auxiliary_color_paint(
        &self,
        target: &AuxiliaryColorPickerTarget,
    ) -> Option<DesignPaint> {
        if let AuxiliaryColorPickerTarget::SelectionColor {
            selection_color_id, ..
        } = target
        {
            let color = self
                .node
                .resolved_selection_colors()
                .into_iter()
                .find(|color| color.id == *selection_color_id)?;
            let mut paint = color.paint;
            paint.id = target.picker_paint_id();
            if let (Some(binding), DesignPaintPayload::Solid(solid)) =
                (color.binding, &mut paint.payload)
                && solid.binding.is_none()
            {
                solid.binding = Some(binding);
                paint.sync_legacy_projection();
            }
            return Some(paint);
        }
        let color = self.auxiliary_color(target)?;
        let mut paint = DesignPaint::solid(DesignColor::rgb(color.red, color.green, color.blue))
            .with_id(target.picker_paint_id());
        paint.opacity = f32::from(color.alpha) / f32::from(u8::MAX) * 100.;
        Some(paint)
    }

    pub(super) fn auxiliary_color_candidate(
        &self,
        target: &AuxiliaryColorPickerTarget,
        edit: &DesignPaintEdit,
    ) -> Option<DesignColor> {
        let current = self.auxiliary_color(target)?;
        match (&edit.property, &edit.value) {
            (DesignPaintProperty::Color, DesignPaintValue::Color(color)) => Some(
                DesignColor::rgba(color.red, color.green, color.blue, current.alpha),
            ),
            (DesignPaintProperty::Opacity, DesignPaintValue::Number(opacity)) => {
                let alpha = (opacity.clamp(0., 100.) / 100. * f32::from(u8::MAX)).round() as u8;
                Some(DesignColor::rgba(
                    current.red,
                    current.green,
                    current.blue,
                    alpha,
                ))
            }
            _ => None,
        }
    }

    pub(super) fn emit_auxiliary_color_edit(
        &mut self,
        edit: &DesignPaintEdit,
        phase: DesignPanelEditPhase,
        cx: &mut Context<Self>,
    ) {
        let Some(target) = self.auxiliary_color_picker.clone() else {
            return;
        };
        if let AuxiliaryColorPickerTarget::SelectionColor {
            target: selection_target,
            selection_color_id,
        } = target
        {
            if selection_target != self.command_target() {
                return;
            }
            let Some(color) = self.selection_color(selection_color_id.as_ref()) else {
                return;
            };
            if phase != DesignPanelEditPhase::Cancel
                && !self.selection_color_paint_editable(&color, edit)
            {
                return;
            }
            if phase == DesignPanelEditPhase::Cancel && color.paint_references.is_empty() {
                return;
            }
            cx.emit_design_panel_action(
                self,
                DesignPanelAction::SelectionColorPaintEditRequested {
                    target: selection_target,
                    selection_color_id,
                    paint_references: color.paint_references,
                    edit: edit.clone(),
                    phase,
                },
            );
            return;
        }
        if phase != DesignPanelEditPhase::Cancel
            && !self.auxiliary_paint_property_editable(&target, &edit.property)
        {
            return;
        }
        let Some(color) = self.auxiliary_color_candidate(&target, edit) else {
            return;
        };
        match target {
            AuxiliaryColorPickerTarget::PageBackground { .. } => {
                self.emit_page_background_edit(color, phase, cx);
            }
            AuxiliaryColorPickerTarget::TextDecoration {
                node_id,
                typography_target,
            } if node_id == self.node.id && self.can_edit() => {
                cx.emit_design_panel_action(
                    self,
                    DesignPanelAction::TypographyPropertyEditRequested {
                        node_id,
                        target: typography_target,
                        property: DesignPanelProperty::TextDecorationColor,
                        value: DesignPanelValue::TextDecorationColor(
                            DesignTextDecorationColor::Solid(color),
                        ),
                        phase,
                    },
                );
            }
            AuxiliaryColorPickerTarget::Effect {
                node_id,
                effect_id,
                index,
                property,
            } if node_id == self.node.id && self.can_edit() => {
                let resolved_index = if effect_id.is_empty() {
                    (index < self.node.effects.len()).then_some(index)
                } else {
                    self.node
                        .effects
                        .iter()
                        .position(|effect| effect.id == effect_id)
                };
                let Some(resolved_index) = resolved_index else {
                    return;
                };
                cx.emit_design_panel_action(
                    self,
                    DesignPanelAction::EffectEditRequested {
                        node_id,
                        effect_id,
                        index: resolved_index,
                        property: property.with_effect_index(resolved_index),
                        shader_property_id: None,
                        value: DesignPanelValue::Color(color),
                        phase,
                    },
                );
            }
            AuxiliaryColorPickerTarget::LayoutGrid {
                node_id,
                guide_id,
                index,
            } if node_id == self.node.id && self.can_edit() => {
                let resolved_index = if guide_id.is_empty() {
                    (index < self.node.layout_grids.len()).then_some(index)
                } else {
                    self.node
                        .layout_grids
                        .iter()
                        .position(|guide| guide.id == guide_id)
                };
                let Some(resolved_index) = resolved_index else {
                    return;
                };
                let Some(guide) = self.node.layout_grids.get(resolved_index) else {
                    return;
                };
                let (property, value) = match (&edit.property, &edit.value) {
                    (DesignPaintProperty::Color, DesignPaintValue::Color(color)) => (
                        DesignPanelProperty::LayoutGridColor(resolved_index),
                        DesignPanelValue::Color(DesignColor::rgba(
                            color.red,
                            color.green,
                            color.blue,
                            guide.color.alpha,
                        )),
                    ),
                    (DesignPaintProperty::Opacity, DesignPaintValue::Number(opacity)) => (
                        DesignPanelProperty::LayoutGridOpacity(resolved_index),
                        DesignPanelValue::Number(opacity.clamp(0., 100.)),
                    ),
                    _ => return,
                };
                cx.emit_design_panel_action(
                    self,
                    DesignPanelAction::LayoutGridPropertyEditRequested {
                        node_id,
                        guide_id: guide.id.clone(),
                        index: resolved_index,
                        property,
                        value,
                        phase,
                    },
                );
            }
            AuxiliaryColorPickerTarget::TextDecoration { .. }
            | AuxiliaryColorPickerTarget::Effect { .. }
            | AuxiliaryColorPickerTarget::LayoutGrid { .. }
            | AuxiliaryColorPickerTarget::SelectionColor { .. } => {}
        }
    }

    pub(super) fn cancel_active_paint_edit(&mut self, cx: &mut Context<Self>) {
        let Some(active) = self.active_paint_edit.take() else {
            return;
        };
        if self
            .auxiliary_color_picker
            .as_ref()
            .is_some_and(|target| target.matches_picker_target(&active.target))
        {
            self.emit_auxiliary_color_edit(&active.edit, DesignPanelEditPhase::Cancel, cx);
            return;
        }
        if active.target.node_id == self.node.id {
            self.emit_paint_edit(
                PaintPickerTarget {
                    collection: active.target.collection,
                    index: active.target.index,
                    paint_id: active.target.paint_id,
                },
                active.edit,
                DesignPanelEditPhase::Cancel,
                cx,
            );
        }
    }

    pub(super) fn prepare_paint_picker_for_dismissal(&self, cx: &mut Context<Self>) {
        self.paint_picker
            .update(cx, |picker, cx| picker.prepare_for_dismissal(cx));
    }

    pub(super) fn paint_index_in_node(
        node: &DesignPanelNode,
        collection: DesignPanelCollection,
        paint_id: &str,
        fallback_index: usize,
    ) -> Option<usize> {
        let paints = match collection {
            DesignPanelCollection::Fill => node.fills.as_slice(),
            DesignPanelCollection::Stroke => node.stroke.as_ref()?.paints.as_slice(),
            DesignPanelCollection::Effect
            | DesignPanelCollection::LayoutGrid
            | DesignPanelCollection::Export => return None,
        };
        if paint_id.is_empty() {
            paints.get(fallback_index).map(|_| fallback_index)
        } else {
            paints
                .iter()
                .position(|paint| paint.id.as_ref() == paint_id)
        }
    }

    pub(super) fn active_paint_edit_is_editable(&self) -> bool {
        let Some(active) = self.active_paint_edit.as_ref() else {
            return true;
        };
        if self
            .auxiliary_color_picker
            .as_ref()
            .is_some_and(|target| target.matches_picker_target(&active.target))
        {
            return true;
        }
        if active.target.node_id != self.node.id {
            return false;
        }
        let target = PaintPickerTarget {
            collection: active.target.collection,
            index: active.target.index,
            paint_id: active.target.paint_id.clone(),
        };
        let Some(paint) = self.picker_paint(&target) else {
            return false;
        };
        if paint.read_only {
            return false;
        }
        match &active.edit.property {
            DesignPaintProperty::Color => matches!(
                &paint.payload,
                DesignPaintPayload::Solid(solid) if solid.binding.is_none()
            ),
            DesignPaintProperty::GradientStopColor { stop_id, index } => {
                let DesignPaintPayload::Gradient(gradient) = &paint.payload else {
                    return false;
                };
                let stop = if stop_id.is_empty() {
                    gradient.stops.get(*index)
                } else {
                    gradient.stops.iter().find(|stop| stop.id == *stop_id)
                };
                stop.is_some_and(|stop| stop.binding.is_none())
            }
            DesignPaintProperty::GradientStopPosition { stop_id, index }
            | DesignPaintProperty::GradientStopRemove { stop_id, index } => {
                let DesignPaintPayload::Gradient(gradient) = &paint.payload else {
                    return false;
                };
                if stop_id.is_empty() {
                    gradient.stops.get(*index).is_some()
                } else {
                    gradient.stops.iter().any(|stop| stop.id == *stop_id)
                }
            }
            _ => {
                let mut candidate = paint;
                candidate.apply_edit(&active.edit)
            }
        }
    }

    pub(super) fn emit_paint_reorder(
        &self,
        collection: DesignPanelCollection,
        paint: &DesignPaint,
        from_index: usize,
        to_index: usize,
        cx: &mut Context<Self>,
    ) {
        if !self.can_edit()
            || !self.collection_is_supported(collection)
            || self.paint_style_binding(collection).is_some()
            || from_index == to_index
        {
            return;
        }
        let current_paint = self.paint_collection(collection).and_then(|paints| {
            if paint.id.is_empty() {
                paints.get(from_index)
            } else {
                paints.iter().find(|candidate| candidate.id == paint.id)
            }
        });
        if current_paint.is_none_or(|paint| paint.read_only) {
            return;
        }
        cx.emit_design_panel_action(
            self,
            DesignPanelAction::PaintReorderRequested {
                node_id: self.node.id.clone(),
                collection,
                target: self.paint_target(collection),
                paint_id: paint.id.clone(),
                from_index,
                to_index,
            },
        );
    }

    pub(super) fn open_paint_style_browser(
        &mut self,
        collection: DesignPanelCollection,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !matches!(
            collection,
            DesignPanelCollection::Fill | DesignPanelCollection::Stroke
        ) || !self.collection_is_supported(collection)
        {
            return;
        }
        self.style_browser_source_filter =
            self.style_browser_source_filter.normalized_for_libraries(
                self.paint_style_view_data
                    .libraries
                    .iter()
                    .map(|library| (&library.id, &library.name)),
            );
        self.prepare_paint_picker_for_dismissal(cx);
        self.cancel_active_paint_edit(cx);
        self.auxiliary_color_picker = None;
        self.active_picker = None;
        self.active_effect_settings = None;
        self.paint_style_browser_open = Some(collection);
        self.effect_style_browser_open = false;
        self.layout_grid_style_browser_open = false;
        self.typography_style_picker_open = false;
        self.type_settings_open = false;
        self.selection_header_overlay = None;
        cx.notify();
    }

    pub(super) fn paint_style_binding(
        &self,
        collection: DesignPanelCollection,
    ) -> Option<super::super::DesignPaintStyleBinding> {
        match collection {
            DesignPanelCollection::Fill => self.node.fill_style_binding.clone(),
            DesignPanelCollection::Stroke => self.node.stroke_style_binding.clone(),
            DesignPanelCollection::Effect
            | DesignPanelCollection::LayoutGrid
            | DesignPanelCollection::Export => None,
        }
    }

    pub(super) fn emit_paint_style_apply(
        &mut self,
        collection: DesignPanelCollection,
        style: DesignPaintStyleSelection,
        cx: &mut Context<Self>,
    ) {
        if !self.can_edit()
            || !self.collection_is_supported(collection)
            || self
                .paint_style_view_data
                .style(&style)
                .is_none_or(|style| style.import_state != DesignPaintStyleImportState::Imported)
        {
            return;
        }
        self.paint_style_browser_open = None;
        cx.emit_design_panel_action(
            self,
            DesignPanelAction::PaintStyleApplyRequested {
                node_id: self.node.id.clone(),
                collection,
                target: self.paint_target(collection),
                style,
            },
        );
        cx.notify();
    }

    pub(super) fn emit_paint_style_import(
        &mut self,
        collection: DesignPanelCollection,
        style: DesignPaintStyleSelection,
        cx: &mut Context<Self>,
    ) {
        if !self.can_edit()
            || !self.collection_is_supported(collection)
            || self
                .paint_style_view_data
                .style(&style)
                .is_none_or(|style| style.import_state != DesignPaintStyleImportState::Available)
        {
            return;
        }
        self.paint_style_browser_open = None;
        cx.emit_design_panel_action(
            self,
            DesignPanelAction::PaintStyleImportRequested {
                node_id: self.node.id.clone(),
                collection,
                target: self.paint_target(collection),
                style,
            },
        );
        cx.notify();
    }

    pub(super) fn emit_paint_style_create(
        &mut self,
        collection: DesignPanelCollection,
        cx: &mut Context<Self>,
    ) {
        let Some(paints) = self.paint_collection(collection).map(<[_]>::to_vec) else {
            return;
        };
        if !self.can_edit() || paints.is_empty() || self.paint_style_binding(collection).is_some() {
            return;
        }
        self.paint_style_browser_open = None;
        cx.emit_design_panel_action(
            self,
            DesignPanelAction::PaintStyleCreateRequested {
                node_id: self.node.id.clone(),
                collection,
                target: self.paint_target(collection),
                paints,
            },
        );
        cx.notify();
    }

    pub(super) fn emit_paint_style_detach(
        &mut self,
        collection: DesignPanelCollection,
        cx: &mut Context<Self>,
    ) {
        let Some(binding) = self
            .paint_style_binding(collection)
            .filter(|binding| binding.can_detach)
        else {
            return;
        };
        if !self.can_edit() || !self.collection_is_supported(collection) {
            return;
        }
        self.paint_style_browser_open = None;
        cx.emit_design_panel_action(
            self,
            DesignPanelAction::PaintStyleDetachRequested {
                node_id: self.node.id.clone(),
                collection,
                target: self.paint_target(collection),
                style: binding.selection,
            },
        );
        cx.notify();
    }

    pub(super) fn render_paint_style_button(
        &self,
        collection: DesignPanelCollection,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let panel = cx.entity();
        let panel_for_open = panel.clone();
        let panel_for_content = panel;
        let panel_id = self.id.clone();
        let active = self.paint_style_browser_open == Some(collection);
        let binding = self.paint_style_binding(collection);
        let can_edit = self.can_edit();
        let paints = self
            .paint_collection(collection)
            .map(<[_]>::to_vec)
            .unwrap_or_default();
        let can_create = can_edit && !paints.is_empty() && binding.is_none();
        let query = self.normalized_style_browser_query(cx);
        let source_filter = self.style_browser_source_filter.clone();
        let view_mode = self.style_browser_view_mode;
        let catalog_is_empty = self.paint_style_view_data.page_styles.is_empty()
            && self
                .paint_style_view_data
                .libraries
                .iter()
                .all(|library| library.styles.is_empty());
        let page_styles = if source_filter.includes_page() {
            self.paint_style_view_data
                .page_styles
                .iter()
                .map(|style| {
                    (
                        style.name.clone(),
                        DesignPaintStyleSelection::page(style.id.clone()),
                        style.import_state,
                        paint_style_summary(&style.paints),
                    )
                })
                .filter(|(name, _, _, summary)| {
                    Self::style_browser_row_matches(&query, name.as_ref(), summary.as_ref(), None)
                })
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        let libraries = self
            .paint_style_view_data
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
                                DesignPaintStyleSelection::library(
                                    library.id.clone(),
                                    style.id.clone(),
                                ),
                                style.import_state,
                                paint_style_summary(&style.paints),
                            )
                        })
                        .filter(|(name, _, _, summary)| {
                            Self::style_browser_row_matches(
                                &query,
                                name.as_ref(),
                                summary.as_ref(),
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
            .paint_style_view_data
            .libraries
            .iter()
            .map(|library| (library.id.clone(), library.name.clone()))
            .collect::<Vec<_>>();
        let style_browser_scope = SharedString::from(format!(
            "{panel_id}-{}-paint-style-browser",
            collection.label().to_ascii_lowercase().replace(' ', "-")
        ));
        let tooltip = binding
            .as_ref()
            .map_or_else(|| "Paint styles".into(), |binding| binding.name.clone());
        let trigger = Button::new(SharedString::from(format!(
            "{}-{}-styles",
            self.id,
            collection.label().to_lowercase().replace(' ', "-")
        )))
        .tooltip(tooltip)
        .xsmall()
        .compact()
        .ghost()
        .w(px(24.))
        .h(px(24.))
        .selected(active || binding.is_some())
        .child(self.render_color_styles_icon(cx))
        .on_activate(cx.listener(move |this, _, window, cx| {
            cx.stop_propagation();
            this.open_paint_style_browser(collection, window, cx);
        }));

        Popover::new(SharedString::from(format!(
            "{}-{}-paint-style-popover",
            self.id,
            collection.label().to_lowercase().replace(' ', "-")
        )))
        .anchor(Anchor::TopRight)
        .open(active)
        .overlay_closable(true)
        .on_open_change(move |open, window, cx| {
            panel_for_open.update(cx, |this, cx| {
                if *open {
                    this.open_paint_style_browser(collection, window, cx);
                } else if this.paint_style_browser_open == Some(collection) {
                    this.paint_style_browser_open = None;
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
                        .child(format!("{} styles", collection.label())),
                );
            if let Some(binding) = binding.clone() {
                let panel = panel_for_content.clone();
                content = content.child(
                    h_flex()
                        .w_full()
                        .px_2()
                        .gap_2()
                        .child(div().flex_1().truncate().text_xs().child(binding.name))
                        .child(
                            Button::new(SharedString::from(format!(
                                "{panel_id}-detach-{}-paint-style",
                                collection.label().to_lowercase()
                            )))
                            .label("Detach")
                            .xsmall()
                            .compact()
                            .ghost()
                            .disabled(!can_edit || !binding.can_detach)
                            .on_activate(move |_, _, cx| {
                                panel.update(cx, |this, cx| {
                                    this.emit_paint_style_detach(collection, cx);
                                });
                            }),
                        ),
                );
            }
            let panel = panel_for_content.clone();
            content = content.child(
                Button::new(SharedString::from(format!(
                    "{panel_id}-create-{}-paint-style",
                    collection.label().to_lowercase()
                )))
                .label("Create style from selection")
                .xsmall()
                .compact()
                .ghost()
                .w_full()
                .disabled(!can_create)
                .on_activate(move |_, _, cx| {
                    panel.update(cx, |this, cx| {
                        this.emit_paint_style_create(collection, cx);
                    });
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
                        .py_3()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(if catalog_is_empty {
                            "No Paint styles supplied by the host"
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
                for (name, selection, import_state, summary) in page_styles.clone() {
                    let panel = panel_for_content.clone();
                    let selection_for_click = selection.clone();
                    let selected = binding
                        .as_ref()
                        .is_some_and(|binding| binding.selection == selection);
                    rows = rows.child(
                        Button::new(SharedString::from(format!(
                            "{panel_id}-paint-style-page-{}",
                            selection.style_id
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
                        .selected(selected)
                        .disabled(!can_edit)
                        .on_activate(move |_, _, cx| {
                            panel.update(cx, |this, cx| match import_state {
                                DesignPaintStyleImportState::Imported => this
                                    .emit_paint_style_apply(
                                        collection,
                                        selection_for_click.clone(),
                                        cx,
                                    ),
                                DesignPaintStyleImportState::Available => this
                                    .emit_paint_style_import(
                                        collection,
                                        selection_for_click.clone(),
                                        cx,
                                    ),
                            });
                        }),
                    );
                }
                content = content.child(rows);
            }
            for (library_name, styles) in libraries.clone() {
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
                for (name, selection, import_state, summary) in styles {
                    let panel = panel_for_content.clone();
                    let selection_for_click = selection.clone();
                    let selected = binding
                        .as_ref()
                        .is_some_and(|binding| binding.selection == selection);
                    let label = if import_state == DesignPaintStyleImportState::Available {
                        format!("Import {name}")
                    } else {
                        name.to_string()
                    };
                    rows = rows.child(
                        Button::new(SharedString::from(format!(
                            "{panel_id}-paint-style-library-{}",
                            selection.style_id
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
                        .selected(selected)
                        .disabled(!can_edit)
                        .on_activate(move |_, _, cx| {
                            panel.update(cx, |this, cx| match import_state {
                                DesignPaintStyleImportState::Imported => this
                                    .emit_paint_style_apply(
                                        collection,
                                        selection_for_click.clone(),
                                        cx,
                                    ),
                                DesignPaintStyleImportState::Available => this
                                    .emit_paint_style_import(
                                        collection,
                                        selection_for_click.clone(),
                                        cx,
                                    ),
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

    pub(super) fn render_paint_swatch(
        &self,
        paint: &DesignPaint,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let swatch = div()
            .size(px(14.))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .overflow_hidden()
            .rounded(px(2.))
            .border_1()
            .border_color(cx.theme().border)
            .text_xs();
        match &paint.payload {
            DesignPaintPayload::Solid(solid) => {
                swatch.bg(color_hsla(solid.color)).into_any_element()
            }
            DesignPaintPayload::Gradient(gradient) => {
                let first = gradient
                    .stops
                    .first()
                    .map_or(DesignColor::rgba(0, 0, 0, 0), |stop| stop.color);
                let last = gradient
                    .stops
                    .last()
                    .map_or(DesignColor::rgba(0, 0, 0, 0), |stop| stop.color);
                swatch
                    .bg(linear_gradient(
                        90.,
                        linear_color_stop(color_hsla(first), 0.),
                        linear_color_stop(color_hsla(last), 1.),
                    ))
                    .into_any_element()
            }
            DesignPaintPayload::Pattern(_) => swatch
                .bg(pattern_slash(
                    cx.theme().muted_foreground.opacity(0.35),
                    0.35,
                    0.35,
                ))
                .child("P")
                .into_any_element(),
            DesignPaintPayload::Image(_) => swatch
                .bg(cx.theme().secondary)
                .child(Icon::new(IconName::GalleryVerticalEnd).xsmall())
                .into_any_element(),
            DesignPaintPayload::Video(_) => swatch
                .bg(cx.theme().secondary)
                .child(Icon::new(IconName::File).xsmall())
                .into_any_element(),
            DesignPaintPayload::Shader(_) => swatch
                .bg(color_hsla(paint.color))
                .child(Icon::new(IconName::Asterisk).xsmall())
                .into_any_element(),
            DesignPaintPayload::Unsupported(_) => swatch
                .bg(cx.theme().secondary)
                .text_color(cx.theme().muted_foreground)
                .child("?")
                .into_any_element(),
        }
    }

    pub(super) fn render_paint_row(
        &self,
        paint: DesignPaint,
        collection: DesignPanelCollection,
        index: usize,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let label: SharedString = match &paint.payload {
            DesignPaintPayload::Solid(_) => paint.color.hex().to_string(),
            DesignPaintPayload::Gradient(_) => format!(
                "{} · {} stops",
                paint.kind.label(),
                paint.gradient_stops.len()
            ),
            DesignPaintPayload::Pattern(pattern) => {
                format!("Pattern · {}", pattern.source_node_id)
            }
            DesignPaintPayload::Image(image) => {
                format!("Image · {}", image.source.name)
            }
            DesignPaintPayload::Video(video) => {
                format!("Video · {}", video.source.name)
            }
            DesignPaintPayload::Shader(shader) => format!("Shader · {}", shader.name),
            DesignPaintPayload::Unsupported(opaque) => opaque.type_name.to_string(),
        }
        .into();
        let target = PaintPickerTarget {
            collection,
            index,
            paint_id: paint.id.clone(),
        };
        let swatch = self.render_paint_swatch(&paint, cx);
        let swatch = if let Some((_, _, _, capabilities)) =
            self.media_drop_capabilities_for_target(&target)
        {
            let drop_target = target.clone();
            let drop_highlight = cx.theme().selection;
            let drop_id = SharedString::from(format!(
                "{}-{}-media-drop-{index}",
                self.id,
                collection.label().to_lowercase().replace(' ', "-")
            ));
            let drop_debug_selector = drop_id.to_string();
            div()
                .id(drop_id)
                .debug_selector(move || drop_debug_selector)
                .size(px(14.))
                .flex_none()
                .rounded(px(2.))
                .child(swatch)
                .can_drop(move |candidate, _, _| {
                    candidate
                        .downcast_ref::<ExternalPaths>()
                        .and_then(|paths| media_drop_from_paths(paths.paths(), capabilities))
                        .is_some()
                })
                .drag_over::<ExternalPaths>(move |style, paths, _, _| {
                    if media_drop_from_paths(paths.paths(), capabilities).is_some() {
                        style.border_2().border_color(drop_highlight)
                    } else {
                        style
                    }
                })
                .on_drop(cx.listener(move |this, paths: &ExternalPaths, _, cx| {
                    this.emit_media_source_drop_from_paths(drop_target.clone(), paths.paths(), cx);
                }))
                .into_any_element()
        } else {
            swatch
        };
        let collection_style_bound = self.paint_style_binding(collection).is_some();
        let can_reorder = self.can_edit()
            && self.collection_is_supported(collection)
            && !collection_style_bound
            && !paint.read_only
            && self
                .paint_collection(collection)
                .is_some_and(|paints| paints.len() > 1);
        let active = self.active_picker.as_ref() == Some(&target);
        let panel = cx.entity();
        let picker = self.paint_picker.clone();
        let picker_content = picker.clone();
        let picker_focus = picker.focus_handle(cx);
        let trigger_id = SharedString::from(format!(
            "{}-{}-paint-{index}",
            self.id,
            collection.label().to_lowercase()
        ));
        let popover_id = SharedString::from(format!(
            "{}-{}-paint-popover-{index}",
            self.id,
            collection.label().to_lowercase()
        ));
        let trigger = Button::new(trigger_id)
            .xsmall()
            .w_full()
            .h_full()
            .justify_start()
            .disabled(
                self.inspection_context.selection().kind() == DesignPanelSelectionKind::None
                    || collection_style_bound,
            )
            .on_keyboard_activate({
                let panel = panel.clone();
                let target = target.clone();
                move |_, cx| {
                    panel.update(cx, |this, cx| {
                        if !active
                            && this.inspection_context.selection().kind()
                                != DesignPanelSelectionKind::None
                            && this.paint_style_binding(collection).is_none()
                        {
                            this.cancel_active_paint_edit(cx);
                            this.auxiliary_color_picker = None;
                            this.paint_style_browser_open = None;
                            this.active_picker = Some(target.clone());
                            this.typography_style_picker_open = false;
                            this.type_settings_open = false;
                        } else if this.active_picker.as_ref() == Some(&target) {
                            this.prepare_paint_picker_for_dismissal(cx);
                            this.cancel_active_paint_edit(cx);
                            this.emit_crop_cancel_if_active(&target, cx);
                            this.active_picker = None;
                        }
                        cx.notify();
                    });
                }
            })
            .child(swatch)
            .child(
                div()
                    .flex_1()
                    .truncate()
                    .text_left()
                    .text_xs()
                    .child(label.clone()),
            );
        let paint_values = h_flex()
            .h_full()
            .flex_1()
            .min_w(px(0.))
            .overflow_hidden()
            .rounded(px(4.))
            .bg(cx.theme().secondary)
            .child(
                div().flex_1().min_w(px(0.)).h_full().child(
                    Popover::new(popover_id)
                        .anchor(Anchor::TopRight)
                        .open(active)
                        .overlay_closable(true)
                        .track_focus(&picker_focus)
                        .on_open_change({
                            let target = target.clone();
                            move |open, _, cx| {
                                panel.update(cx, |this, cx| {
                                    if *open
                                        && this.inspection_context.selection().kind()
                                            != DesignPanelSelectionKind::None
                                        && this.paint_style_binding(collection).is_none()
                                    {
                                        this.cancel_active_paint_edit(cx);
                                        this.auxiliary_color_picker = None;
                                        this.paint_style_browser_open = None;
                                        this.active_picker = Some(target.clone());
                                        this.typography_style_picker_open = false;
                                        this.type_settings_open = false;
                                    } else if this.active_picker.as_ref() == Some(&target) {
                                        this.prepare_paint_picker_for_dismissal(cx);
                                        this.cancel_active_paint_edit(cx);
                                        this.emit_crop_cancel_if_active(&target, cx);
                                        this.active_picker = None;
                                    }
                                    cx.notify();
                                });
                            }
                        })
                        .trigger(trigger)
                        .content(move |_, _, _| picker_content.clone())
                        .w_full()
                        .h_full(),
                ),
            )
            .child(
                div()
                    .w(px(58.))
                    .h_full()
                    .flex_none()
                    .border_l_1()
                    .border_color(cx.theme().border)
                    .child(self.render_value_cell_with_left_padding(
                        format!(
                            "{}-paint-opacity-{index}",
                            collection.label().to_lowercase().replace(' ', "-")
                        ),
                        "",
                        format!("{}  %", format_number(paint.opacity)),
                        DesignPanelProperty::PaintOpacity { collection, index },
                        DesignPanelValue::Number((paint.opacity - 10.).max(0.)),
                        8.,
                        cx,
                    )),
            );
        let drop_paint_id = paint.id.clone();
        let can_drop_paint_id = drop_paint_id.clone();
        let drop_background = cx.theme().selection.opacity(0.18);
        let drop_border = cx.theme().selection;
        let drag = PaintDrag {
            collection,
            from_index: index,
            label,
            paint: paint.clone(),
        };
        let drop_listener = cx.listener(move |this, drag: &PaintDrag, _, cx| {
            this.emit_paint_reorder(drag.collection, &drag.paint, drag.from_index, index, cx);
        });
        h_flex()
            .id(SharedString::from(format!(
                "{}-{}-paint-row-{index}",
                self.id,
                collection.label().to_lowercase().replace(' ', "-")
            )))
            .h(px(ROW_HEIGHT))
            .gap_1()
            .rounded(px(4.))
            .border_1()
            .border_color(cx.theme().transparent)
            .when(can_reorder, |row| row.cursor_move())
            .child(
                div()
                    .w(px(12.))
                    .flex_none()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .when(can_reorder, |handle| handle.child("⠿")),
            )
            .child(paint_values)
            .child(self.render_visibility_button(
                format!(
                    "{}-paint-visible-{index}",
                    collection.label().to_lowercase().replace(' ', "-")
                ),
                paint.visible,
                DesignPanelProperty::PaintVisible { collection, index },
                cx,
            ))
            .child(self.render_remove_button(
                format!(
                    "remove-{}-{index}",
                    collection.label().to_lowercase().replace(' ', "-")
                ),
                collection,
                index,
                cx,
            ))
            .when(can_reorder, move |row| {
                row.on_drag(drag, |drag, _, _, cx| {
                    cx.new(|_| PaintDragPreview { drag: drag.clone() })
                })
                .can_drop(move |candidate, _, _| {
                    candidate
                        .downcast_ref::<PaintDrag>()
                        .is_some_and(|candidate| {
                            candidate.collection == collection
                                && if candidate.paint.id.is_empty() || can_drop_paint_id.is_empty()
                                {
                                    candidate.from_index != index
                                } else {
                                    candidate.paint.id != can_drop_paint_id
                                }
                        })
                })
                .drag_over::<PaintDrag>(move |style, candidate, _, _| {
                    let same = candidate.collection != collection
                        || if candidate.paint.id.is_empty() || drop_paint_id.is_empty() {
                            candidate.from_index == index
                        } else {
                            candidate.paint.id == drop_paint_id
                        };
                    if same {
                        style
                    } else {
                        style.bg(drop_background).border_color(drop_border)
                    }
                })
                .on_drop(drop_listener)
            })
            .into_any_element()
    }

    pub(super) fn render_fill(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        (self.node.supports_fill() && self.node.supports_section(DesignPanelSection::Fill)).then(
            || {
                let mut content = v_flex().pl(px(PANEL_PADDING)).pr_2().pt_1().pb_4().gap_2();
                if let Some(binding) = self.node.fill_style_binding.as_ref() {
                    content = content.child(self.render_bound_style_summary(
                        "fill",
                        binding.name.clone(),
                        cx,
                    ));
                } else {
                    for (index, paint) in self.node.fills.iter().cloned().enumerate() {
                        content = content.child(self.render_paint_row(
                            paint,
                            DesignPanelCollection::Fill,
                            index,
                            cx,
                        ));
                    }
                    if self.node.fills.is_empty() {
                        content = content.child(empty_collection("No fills", cx));
                    }
                    if let Some(show_in_exports) = self
                        .node
                        .fill_shows_in_exports
                        .filter(|_| !self.node.fills.is_empty())
                    {
                        content = content.child(self.render_checkbox_row(
                            "fill-shows-in-exports",
                            "Show in exports",
                            show_in_exports,
                            DesignPanelProperty::FillShowsInExports,
                            cx,
                        ));
                    }
                }
                self.render_section(
                    DesignPanelSection::Fill,
                    Some(DesignPanelCollection::Fill),
                    content.into_any_element(),
                    cx,
                )
            },
        )
    }

    pub(super) fn render_stroke(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        (self.node.supports_stroke() && self.node.supports_section(DesignPanelSection::Stroke))
            .then(|| {
                let mut content = v_flex().pl(px(PANEL_PADDING)).pr_2().pb_4().gap_2();
                let Some(stroke) = self.node.stroke.as_ref() else {
                    return self.render_section(
                        DesignPanelSection::Stroke,
                        Some(DesignPanelCollection::Stroke),
                        div().into_any_element(),
                        cx,
                    );
                };

                if let Some(binding) = self.node.stroke_style_binding.as_ref() {
                    content = content.child(self.render_bound_style_summary(
                        "stroke",
                        binding.name.clone(),
                        cx,
                    ));
                } else {
                    for (index, paint) in stroke.paints.iter().cloned().enumerate() {
                        content = content.child(self.render_paint_row(
                            paint,
                            DesignPanelCollection::Stroke,
                            index,
                            cx,
                        ));
                    }
                }
                if stroke.paints.is_empty() && self.node.stroke_style_binding.is_none() {
                    return self.render_section(
                        DesignPanelSection::Stroke,
                        Some(DesignPanelCollection::Stroke),
                        div().into_any_element(),
                        cx,
                    );
                } else {
                    let basic = stroke.complex_stroke.is_basic();
                    let mut geometry = v_flex().w_full().gap_2();
                    let weight_control = h_flex()
                        .w_full()
                        .gap_1()
                        .child(div().flex_1().min_w(px(0.)).child(self.render_value_cell(
                            "stroke-weight",
                            "—",
                            format_number(stroke.weights.active()),
                            DesignPanelProperty::StrokeWeight,
                            DesignPanelValue::Number(stroke.weights.active() + 1.),
                            cx,
                        )))
                        .when(stroke.capabilities.individual_weights, |row| {
                            row.child(div().w(px(24.)).flex_none().child(
                                self.render_icon_action_button(
                                    "stroke-weight-mode",
                                    IconName::Settings2,
                                    DesignPanelAction::PropertyChangeRequested {
                                        node_id: self.node.id.clone(),
                                        property: DesignPanelProperty::StrokeWeightMode,
                                        value: DesignPanelValue::StrokeWeightMode(
                                            if stroke.weights.mode == DesignStrokeWeightMode::Custom
                                            {
                                                DesignStrokeWeightMode::All
                                            } else {
                                                DesignStrokeWeightMode::Custom
                                            },
                                        ),
                                    },
                                    cx,
                                ),
                            ))
                        })
                        .into_any_element();

                    if stroke.capabilities.position && basic {
                        geometry = geometry.child(
                            h_flex()
                                .w_full()
                                .items_end()
                                .gap_2()
                                .child(
                                    v_flex()
                                        .flex_1()
                                        .min_w(px(0.))
                                        .gap_1()
                                        .child(self.render_group_label("Position", cx))
                                        .child(self.render_value_cell(
                                            "stroke-align",
                                            "▣",
                                            stroke.align.label(),
                                            DesignPanelProperty::StrokeAlign,
                                            DesignPanelValue::StrokeAlign(
                                                DesignStrokeAlign::Center,
                                            ),
                                            cx,
                                        )),
                                )
                                .child(
                                    v_flex()
                                        .flex_1()
                                        .min_w(px(0.))
                                        .gap_1()
                                        .child(self.render_group_label("Weight", cx))
                                        .child(weight_control),
                                ),
                        );
                    } else {
                        geometry = geometry.child(
                            v_flex()
                                .w_full()
                                .gap_1()
                                .child(self.render_group_label("Weight", cx))
                                .child(weight_control),
                        );
                    }

                    if stroke.capabilities.individual_weights
                        && stroke.weights.mode == DesignStrokeWeightMode::Custom
                    {
                        geometry = geometry
                            .child(
                                h_flex()
                                    .gap_2()
                                    .child(self.render_value_cell(
                                        "stroke-weight-top",
                                        "T",
                                        format_number(stroke.weights.top),
                                        DesignPanelProperty::StrokeWeightTop,
                                        DesignPanelValue::Number(stroke.weights.top + 1.),
                                        cx,
                                    ))
                                    .child(self.render_value_cell(
                                        "stroke-weight-right",
                                        "R",
                                        format_number(stroke.weights.right),
                                        DesignPanelProperty::StrokeWeightRight,
                                        DesignPanelValue::Number(stroke.weights.right + 1.),
                                        cx,
                                    )),
                            )
                            .child(
                                h_flex()
                                    .gap_2()
                                    .child(self.render_value_cell(
                                        "stroke-weight-bottom",
                                        "B",
                                        format_number(stroke.weights.bottom),
                                        DesignPanelProperty::StrokeWeightBottom,
                                        DesignPanelValue::Number(stroke.weights.bottom + 1.),
                                        cx,
                                    ))
                                    .child(self.render_value_cell(
                                        "stroke-weight-left",
                                        "L",
                                        format_number(stroke.weights.left),
                                        DesignPanelProperty::StrokeWeightLeft,
                                        DesignPanelValue::Number(stroke.weights.left + 1.),
                                        cx,
                                    )),
                            );
                    }

                    if basic {
                        geometry = geometry
                            .child(self.render_group_label("Dash style", cx))
                            .child(self.render_value_cell(
                                "stroke-dash-mode",
                                "⋯",
                                stroke.dashes.mode.label(),
                                DesignPanelProperty::StrokeDashMode,
                                DesignPanelValue::StrokeDashMode(stroke.dashes.mode.next()),
                                cx,
                            ));
                        if !stroke.dashes.is_solid() {
                            geometry = geometry
                                .child(self.render_group_label("Dash pattern", cx))
                                .child(
                                    h_flex()
                                        .gap_2()
                                        .child(
                                            self.render_value_cell(
                                                "stroke-dash-pattern",
                                                "⋯",
                                                stroke
                                                    .dashes
                                                    .pattern
                                                    .iter()
                                                    .map(|value| format_number(*value))
                                                    .collect::<Vec<_>>()
                                                    .join(", "),
                                                DesignPanelProperty::StrokeDashPattern,
                                                DesignPanelValue::NumberList(
                                                    stroke.dashes.pattern.clone(),
                                                ),
                                                cx,
                                            ),
                                        )
                                        .child(self.render_value_cell(
                                            "stroke-dash-cap",
                                            "•",
                                            stroke.dash_cap.label(),
                                            DesignPanelProperty::StrokeDashCap,
                                            DesignPanelValue::StrokeCap(DesignStrokeCap::Round),
                                            cx,
                                        )),
                                );
                        }

                        geometry = match stroke.edit_context.endpoint_control() {
                            DesignStrokeEndpointControl::None => geometry,
                            DesignStrokeEndpointControl::StartAndEnd => geometry.child(
                                h_flex()
                                    .w_full()
                                    .items_end()
                                    .gap_2()
                                    .child(
                                        v_flex()
                                            .flex_1()
                                            .min_w(px(0.))
                                            .gap_1()
                                            .child(self.render_group_label("Start point", cx))
                                            .child(self.render_value_cell(
                                                "stroke-start-cap",
                                                "←",
                                                stroke.start_cap.label(),
                                                DesignPanelProperty::StrokeStartCap,
                                                DesignPanelValue::StrokeCap(DesignStrokeCap::Round),
                                                cx,
                                            )),
                                    )
                                    .child(
                                        v_flex()
                                            .flex_1()
                                            .min_w(px(0.))
                                            .gap_1()
                                            .child(self.render_group_label("End point", cx))
                                            .child(self.render_value_cell(
                                                "stroke-end-cap",
                                                "→",
                                                stroke.end_cap.label(),
                                                DesignPanelProperty::StrokeEndCap,
                                                DesignPanelValue::StrokeCap(
                                                    DesignStrokeCap::LineArrow,
                                                ),
                                                cx,
                                            )),
                                    )
                                    .child(self.render_symbol_action_button(
                                        "swap-stroke-endpoints",
                                        "⇄",
                                        DesignPanelAction::SwapStrokeEndpointsRequested {
                                            node_id: self.node.id.clone(),
                                        },
                                        cx,
                                    )),
                            ),
                            DesignStrokeEndpointControl::Aggregate => {
                                geometry.child(self.render_value_cell(
                                    "stroke-endpoint-cap",
                                    "↔",
                                    format!("End points · {}", stroke.endpoint_cap.label()),
                                    DesignPanelProperty::StrokeEndpointCap,
                                    DesignPanelValue::StrokeCap(DesignStrokeCap::Round),
                                    cx,
                                ))
                            }
                            DesignStrokeEndpointControl::SelectedVertices => {
                                geometry.child(self.render_value_cell(
                                    "stroke-selected-endpoint-cap",
                                    "◆",
                                    format!("Selection · {}", stroke.endpoint_cap.label()),
                                    DesignPanelProperty::StrokeEndpointCap,
                                    DesignPanelValue::StrokeCap(DesignStrokeCap::Round),
                                    cx,
                                ))
                            }
                        };

                        if stroke.capabilities.joins {
                            geometry = geometry.child(
                                h_flex()
                                    .gap_2()
                                    .child(self.render_value_cell(
                                        "stroke-join",
                                        "⌁",
                                        stroke.join.label(),
                                        DesignPanelProperty::StrokeJoin,
                                        DesignPanelValue::StrokeJoin(DesignStrokeJoin::Round),
                                        cx,
                                    ))
                                    .when(stroke.join == DesignStrokeJoin::Miter, |row| {
                                        row.child(self.render_value_cell(
                                            "stroke-miter-angle",
                                            "°",
                                            format_number(stroke.miter_angle),
                                            DesignPanelProperty::StrokeMiterAngle,
                                            DesignPanelValue::Number(
                                                (stroke.miter_angle + 1.).min(180.),
                                            ),
                                            cx,
                                        ))
                                    }),
                            );
                        }
                    }

                    if stroke.supports_variable_width() {
                        let variable_width_label = stroke
                            .variable_width
                            .as_ref()
                            .map_or("None", DesignVariableWidthStroke::label);
                        geometry = geometry
                            .child(self.render_group_label("Variable width", cx))
                            .child(self.render_value_cell(
                                "stroke-variable-width",
                                "⌇",
                                variable_width_label,
                                DesignPanelProperty::StrokeVariableWidth,
                                DesignPanelValue::StrokeVariableWidth(Some(
                                    DesignVariableWidthStroke::Preset(
                                        DesignVariableWidthPreset::Taper,
                                    ),
                                )),
                                cx,
                            ));
                        if let Some(DesignVariableWidthStroke::Custom { points }) =
                            stroke.variable_width.as_ref()
                        {
                            for (index, point) in points.iter().enumerate() {
                                geometry = geometry
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(format!("Point {}", index + 1)),
                                )
                                .child(
                                    h_flex()
                                        .gap_2()
                                        .child(self.render_value_cell(
                                            format!("stroke-width-point-{index}-position"),
                                            "P",
                                            format_number(point.position),
                                            DesignPanelProperty::StrokeVariableWidthPointPosition(
                                                index,
                                            ),
                                            DesignPanelValue::Number(
                                                (point.position + 0.05).min(1.),
                                            ),
                                            cx,
                                        ))
                                        .child(self.render_value_cell(
                                            format!("stroke-width-point-{index}-width"),
                                            "W",
                                            format_number(point.width),
                                            DesignPanelProperty::StrokeVariableWidthPointWidth(
                                                index,
                                            ),
                                            DesignPanelValue::Number(point.width + 0.1),
                                            cx,
                                        )),
                                );
                            }
                        }
                    }

                    if stroke.capabilities.complex_stroke {
                        let next_type = match stroke.complex_stroke.kind() {
                            DesignStrokeType::Basic => DesignStrokeType::StretchBrush,
                            DesignStrokeType::StretchBrush => DesignStrokeType::ScatterBrush,
                            DesignStrokeType::ScatterBrush => DesignStrokeType::Dynamic,
                            DesignStrokeType::Dynamic => DesignStrokeType::Basic,
                            DesignStrokeType::Opaque => DesignStrokeType::Opaque,
                        };
                        geometry = geometry.child(self.render_value_cell(
                            "stroke-type",
                            "⌁",
                            stroke.complex_stroke.label(),
                            DesignPanelProperty::StrokeType,
                            DesignPanelValue::StrokeType(next_type),
                            cx,
                        ));
                        match &stroke.complex_stroke {
                            DesignComplexStroke::Basic => {}
                            DesignComplexStroke::StretchBrush(stretch) => {
                                geometry = geometry.child(
                                    h_flex()
                                        .gap_2()
                                        .child(self.render_value_cell(
                                            "stroke-stretch-brush",
                                            "✎",
                                            stretch.brush.label(),
                                            DesignPanelProperty::StrokeStretchBrush,
                                            DesignPanelValue::StrokeStretchBrush(
                                                stretch.brush.next(),
                                            ),
                                            cx,
                                        ))
                                        .child(self.render_value_cell(
                                            "stroke-brush-direction",
                                            "→",
                                            stretch.direction.label(),
                                            DesignPanelProperty::StrokeBrushDirection,
                                            DesignPanelValue::StrokeBrushDirection(
                                                match stretch.direction {
                                                    DesignStrokeBrushDirection::Forward => {
                                                        DesignStrokeBrushDirection::Backward
                                                    }
                                                    DesignStrokeBrushDirection::Backward => {
                                                        DesignStrokeBrushDirection::Forward
                                                    }
                                                },
                                            ),
                                            cx,
                                        )),
                                );
                            }
                            DesignComplexStroke::ScatterBrush(scatter) => {
                                geometry = geometry
                                    .child(self.render_value_cell(
                                        "stroke-scatter-brush",
                                        "✣",
                                        scatter.brush.label(),
                                        DesignPanelProperty::StrokeScatterBrush,
                                        DesignPanelValue::StrokeScatterBrush(scatter.brush.next()),
                                        cx,
                                    ))
                                    .child(
                                        h_flex()
                                            .gap_2()
                                            .child(self.render_value_cell(
                                                "stroke-scatter-gap",
                                                "G",
                                                format_number(scatter.gap),
                                                DesignPanelProperty::StrokeScatterGap,
                                                DesignPanelValue::Number(scatter.gap + 0.25),
                                                cx,
                                            ))
                                            .child(self.render_value_cell(
                                                "stroke-scatter-wiggle",
                                                "W",
                                                format_number(scatter.wiggle),
                                                DesignPanelProperty::StrokeScatterWiggle,
                                                DesignPanelValue::Number(scatter.wiggle + 0.1),
                                                cx,
                                            )),
                                    )
                                    .child(
                                        h_flex()
                                            .gap_2()
                                            .child(self.render_value_cell(
                                                "stroke-scatter-size-jitter",
                                                "S",
                                                format_number(scatter.size_jitter),
                                                DesignPanelProperty::StrokeScatterSizeJitter,
                                                DesignPanelValue::Number(
                                                    (scatter.size_jitter + 0.1).min(3.),
                                                ),
                                                cx,
                                            ))
                                            .child(self.render_value_cell(
                                                "stroke-scatter-angular-jitter",
                                                "A",
                                                format_number(scatter.angular_jitter),
                                                DesignPanelProperty::StrokeScatterAngularJitter,
                                                DesignPanelValue::Number(
                                                    (scatter.angular_jitter + 15.).min(180.),
                                                ),
                                                cx,
                                            )),
                                    )
                                    .child(self.render_value_cell(
                                        "stroke-scatter-rotation",
                                        "R",
                                        format_number(scatter.rotation),
                                        DesignPanelProperty::StrokeScatterRotation,
                                        DesignPanelValue::Number(
                                            (scatter.rotation + 15.).min(180.),
                                        ),
                                        cx,
                                    ));
                            }
                            DesignComplexStroke::Dynamic(dynamic) => {
                                geometry = geometry
                                    .child(self.render_value_cell(
                                        "stroke-dynamic-frequency",
                                        "F",
                                        format_number(dynamic.frequency),
                                        DesignPanelProperty::StrokeDynamicFrequency,
                                        DesignPanelValue::Number(
                                            (dynamic.frequency + 0.25).min(20.),
                                        ),
                                        cx,
                                    ))
                                    .child(
                                        h_flex()
                                            .gap_2()
                                            .child(self.render_value_cell(
                                                "stroke-dynamic-wiggle",
                                                "W",
                                                format_number(dynamic.wiggle),
                                                DesignPanelProperty::StrokeDynamicWiggle,
                                                DesignPanelValue::Number(dynamic.wiggle + 0.1),
                                                cx,
                                            ))
                                            .child(self.render_value_cell(
                                                "stroke-dynamic-smoothen",
                                                "S",
                                                format_number(dynamic.smoothen),
                                                DesignPanelProperty::StrokeDynamicSmoothen,
                                                DesignPanelValue::Number(
                                                    (dynamic.smoothen + 0.1).min(1.),
                                                ),
                                                cx,
                                            )),
                                    );
                            }
                            DesignComplexStroke::Opaque(opaque) => {
                                geometry = geometry.child(
                                    v_flex()
                                        .w_full()
                                        .gap_1()
                                        .child(self.render_group_label("Custom stroke", cx))
                                        .child(
                                            div()
                                                .w_full()
                                                .min_h(px(ROW_HEIGHT))
                                                .px_2()
                                                .py_1()
                                                .rounded(px(4.))
                                                .bg(cx.theme().secondary)
                                                .text_xs()
                                                .text_color(cx.theme().muted_foreground)
                                                .child(format!(
                                                    "{} · {}",
                                                    opaque.type_name, opaque.raw
                                                )),
                                        ),
                                );
                            }
                        }
                    }
                    content = content.child(geometry);
                }
                self.render_section(
                    DesignPanelSection::Stroke,
                    Some(DesignPanelCollection::Stroke),
                    content.into_any_element(),
                    cx,
                )
            })
    }

    pub(super) fn render_auxiliary_color_picker_control(
        &self,
        panel: Entity<Self>,
        id_suffix: impl Into<SharedString>,
        label: impl Into<SharedString>,
        color: DesignColor,
        target: AuxiliaryColorPickerTarget,
        cx: &mut App,
    ) -> AnyElement {
        let panel_for_open = panel.clone();
        let picker = self.paint_picker.clone();
        let picker_content = picker.clone();
        let picker_focus = picker.focus_handle(cx);
        let id_suffix = id_suffix.into();
        let label = label.into();
        let open = self.auxiliary_color_picker.as_ref() == Some(&target);
        let selected = matches!(target, AuxiliaryColorPickerTarget::TextDecoration { .. })
            && self
                .node
                .typography
                .as_ref()
                .and_then(|typography| typography.decoration_details.as_ref())
                .is_some_and(|details| {
                    matches!(details.color, DesignTextDecorationColor::Solid(_))
                });
        let inspectable = self.auxiliary_color_paint(&target).is_some();
        let trigger = Button::new(SharedString::from(format!(
            "{}-{id_suffix}-color-trigger",
            self.id
        )))
        .xsmall()
        .compact()
        .w_full()
        .h(px(28.))
        .justify_start()
        .selected(selected || open)
        .disabled(!inspectable)
        .on_keyboard_activate({
            let panel = panel.clone();
            let target = target.clone();
            move |window, cx| {
                let target = target.clone();
                panel.update(cx, |this, cx| {
                    if !open {
                        this.open_auxiliary_color_picker(target, window, cx);
                    } else if this.auxiliary_color_picker.as_ref() == Some(&target) {
                        this.prepare_paint_picker_for_dismissal(cx);
                        this.cancel_active_paint_edit(cx);
                        this.auxiliary_color_picker = None;
                    }
                    cx.notify();
                });
            }
        })
        .child(
            div()
                .size(px(14.))
                .flex_none()
                .rounded(px(3.))
                .border_1()
                .border_color(cx.theme().border)
                .bg(color_hsla(color)),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .truncate()
                .text_left()
                .child(label),
        );

        Popover::new(SharedString::from(format!(
            "{}-{id_suffix}-color-popover",
            self.id
        )))
        .anchor(Anchor::TopRight)
        .open(open)
        .overlay_closable(true)
        .track_focus(&picker_focus)
        .on_open_change(move |open, window, cx| {
            let target = target.clone();
            panel_for_open.update(cx, |this, cx| {
                if *open {
                    this.open_auxiliary_color_picker(target, window, cx);
                } else if this.auxiliary_color_picker.as_ref() == Some(&target) {
                    this.prepare_paint_picker_for_dismissal(cx);
                    this.cancel_active_paint_edit(cx);
                    this.auxiliary_color_picker = None;
                }
                cx.notify();
            });
        })
        .trigger(trigger)
        .content(move |_, _, _| picker_content.clone())
        .into_any_element()
    }

    pub(super) fn selection_color(
        &self,
        selection_color_id: &str,
    ) -> Option<super::super::DesignSelectionColor> {
        self.node
            .resolved_selection_colors()
            .into_iter()
            .find(|color| color.id.as_ref() == selection_color_id)
    }

    pub(super) fn selection_color_references_target_current(
        &self,
        color: &super::super::DesignSelectionColor,
    ) -> bool {
        self.inspection_context.selection().kind() == DesignPanelSelectionKind::Multiple
            && !color.paint_references.is_empty()
            && matches!(
                self.command_target(),
                DesignPanelTarget::Nodes { ref node_ids }
                    if !node_ids.is_empty()
                        && color
                            .paint_references
                            .iter()
                            .all(|reference| node_ids.contains(&reference.node_id))
            )
    }

    pub(super) fn selection_color_can_mutate(
        &self,
        color: &super::super::DesignSelectionColor,
    ) -> bool {
        self.can_edit() && !color.read_only && self.selection_color_references_target_current(color)
    }

    pub(super) fn selection_color_can_select_occurrences(
        &self,
        color: &super::super::DesignSelectionColor,
    ) -> bool {
        self.selection_color_references_target_current(color)
            && u32::try_from(color.paint_references.len()) == Ok(color.occurrence_count)
    }

    pub(super) fn selection_color_paint_editable(
        &self,
        color: &super::super::DesignSelectionColor,
        edit: &DesignPaintEdit,
    ) -> bool {
        if !self.selection_color_can_mutate(color)
            || color.style_binding.is_some()
            || !matches!(
                &color.paint.payload,
                DesignPaintPayload::Solid(_) | DesignPaintPayload::Gradient(_)
            )
        {
            return false;
        }
        let mut candidate = color.paint.clone();
        if let (Some(binding), DesignPaintPayload::Solid(solid)) =
            (color.binding.clone(), &mut candidate.payload)
            && solid.binding.is_none()
        {
            solid.binding = Some(binding);
        }
        candidate.apply_edit(edit)
    }

    pub(super) fn emit_selection_color_occurrences_select(
        &mut self,
        selection_color_id: SharedString,
        cx: &mut Context<Self>,
    ) {
        let Some(color) = self.selection_color(selection_color_id.as_ref()) else {
            return;
        };
        if !self.selection_color_can_select_occurrences(&color) {
            return;
        }
        cx.emit_design_panel_action(
            self,
            DesignPanelAction::SelectionColorOccurrencesSelectRequested {
                target: self.command_target(),
                selection_color_id,
                paint_references: color.paint_references,
            },
        );
    }

    pub(super) fn open_selection_color_resource_browser(
        &mut self,
        selection_color_id: SharedString,
        kind: SelectionColorResourceKind,
        cx: &mut Context<Self>,
    ) {
        if self.selection_color(selection_color_id.as_ref()).is_none() {
            return;
        }
        if kind == SelectionColorResourceKind::PaintStyle {
            self.style_browser_source_filter =
                self.style_browser_source_filter.normalized_for_libraries(
                    self.paint_style_view_data
                        .libraries
                        .iter()
                        .map(|library| (&library.id, &library.name)),
                );
        }
        self.prepare_paint_picker_for_dismissal(cx);
        self.cancel_active_paint_edit(cx);
        self.auxiliary_color_picker = None;
        self.active_picker = None;
        self.paint_style_browser_open = None;
        self.effect_style_browser_open = false;
        self.layout_grid_style_browser_open = false;
        self.typography_style_picker_open = false;
        self.selection_header_overlay = None;
        self.selection_color_resource_browser = Some(SelectionColorResourceTarget {
            selection_color_id,
            kind,
        });
        cx.notify();
    }

    pub(super) fn emit_selection_color_paint_style_apply(
        &mut self,
        selection_color_id: SharedString,
        style: DesignPaintStyleSelection,
        cx: &mut Context<Self>,
    ) {
        let Some(color) = self.selection_color(selection_color_id.as_ref()) else {
            return;
        };
        if !self.selection_color_can_mutate(&color)
            || self
                .paint_style_view_data
                .style(&style)
                .is_none_or(|style| style.import_state != DesignPaintStyleImportState::Imported)
        {
            return;
        }
        self.selection_color_resource_browser = None;
        cx.emit_design_panel_action(
            self,
            DesignPanelAction::SelectionColorPaintStyleApplyRequested {
                target: self.command_target(),
                selection_color_id,
                paint_references: color.paint_references,
                style,
            },
        );
        cx.notify();
    }

    pub(super) fn emit_selection_color_paint_style_import(
        &mut self,
        selection_color_id: SharedString,
        style: DesignPaintStyleSelection,
        cx: &mut Context<Self>,
    ) {
        let Some(color) = self.selection_color(selection_color_id.as_ref()) else {
            return;
        };
        if !self.selection_color_can_mutate(&color)
            || self
                .paint_style_view_data
                .style(&style)
                .is_none_or(|style| style.import_state != DesignPaintStyleImportState::Available)
        {
            return;
        }
        self.selection_color_resource_browser = None;
        cx.emit_design_panel_action(
            self,
            DesignPanelAction::SelectionColorPaintStyleImportRequested {
                target: self.command_target(),
                selection_color_id,
                paint_references: color.paint_references,
                style,
            },
        );
        cx.notify();
    }

    pub(super) fn emit_selection_color_paint_style_create(
        &mut self,
        selection_color_id: SharedString,
        cx: &mut Context<Self>,
    ) {
        let Some(color) = self.selection_color(selection_color_id.as_ref()) else {
            return;
        };
        if !self.selection_color_can_mutate(&color)
            || color.style_binding.is_some()
            || color.style_paints.is_empty()
        {
            return;
        }
        self.selection_color_resource_browser = None;
        cx.emit_design_panel_action(
            self,
            DesignPanelAction::SelectionColorPaintStyleCreateRequested {
                target: self.command_target(),
                selection_color_id,
                paint_references: color.paint_references,
                paints: color.style_paints,
            },
        );
        cx.notify();
    }

    pub(super) fn emit_selection_color_paint_style_detach(
        &mut self,
        selection_color_id: SharedString,
        cx: &mut Context<Self>,
    ) {
        let Some(color) = self.selection_color(selection_color_id.as_ref()) else {
            return;
        };
        let Some(binding) = color
            .style_binding
            .as_ref()
            .filter(|binding| binding.can_detach)
            .cloned()
        else {
            return;
        };
        if !self.selection_color_can_mutate(&color) {
            return;
        }
        self.selection_color_resource_browser = None;
        cx.emit_design_panel_action(
            self,
            DesignPanelAction::SelectionColorPaintStyleDetachRequested {
                target: self.command_target(),
                selection_color_id,
                paint_references: color.paint_references,
                style: binding.selection,
            },
        );
        cx.notify();
    }

    pub(super) fn emit_selection_color_variable_apply(
        &mut self,
        selection_color_id: SharedString,
        variable_id: SharedString,
        cx: &mut Context<Self>,
    ) {
        let Some(color) = self.selection_color(selection_color_id.as_ref()) else {
            return;
        };
        let variable = self.paint_variable_view_data.variable(variable_id.as_ref());
        if !self.selection_color_can_mutate(&color)
            || color.style_binding.is_some()
            || variable.is_none_or(|variable| {
                variable.disabled_reason.is_some()
                    || !matches!(
                        variable.import_state,
                        DesignVariableImportState::Local | DesignVariableImportState::Imported
                    )
            })
        {
            return;
        }
        self.selection_color_resource_browser = None;
        cx.emit_design_panel_action(
            self,
            DesignPanelAction::SelectionColorVariableApplyRequested {
                target: self.command_target(),
                selection_color_id,
                paint_references: color.paint_references,
                variable_id,
            },
        );
        cx.notify();
    }

    pub(super) fn emit_selection_color_variable_import(
        &mut self,
        selection_color_id: SharedString,
        variable_id: SharedString,
        cx: &mut Context<Self>,
    ) {
        let Some(color) = self.selection_color(selection_color_id.as_ref()) else {
            return;
        };
        let variable = self.paint_variable_view_data.variable(variable_id.as_ref());
        if !self.selection_color_can_mutate(&color)
            || color.style_binding.is_some()
            || variable.is_none_or(|variable| {
                variable.disabled_reason.is_some()
                    || variable.import_state != DesignVariableImportState::Available
            })
        {
            return;
        }
        self.selection_color_resource_browser = None;
        cx.emit_design_panel_action(
            self,
            DesignPanelAction::SelectionColorVariableImportRequested {
                target: self.command_target(),
                selection_color_id,
                paint_references: color.paint_references,
                variable_id,
            },
        );
        cx.notify();
    }

    pub(super) fn emit_selection_color_variable_create(
        &mut self,
        selection_color_id: SharedString,
        cx: &mut Context<Self>,
    ) {
        let Some(color) = self.selection_color(selection_color_id.as_ref()) else {
            return;
        };
        if !self.selection_color_can_mutate(&color)
            || color.style_binding.is_some()
            || color.binding.is_some()
        {
            return;
        }
        self.selection_color_resource_browser = None;
        cx.emit_design_panel_action(
            self,
            DesignPanelAction::SelectionColorVariableCreateRequested {
                target: self.command_target(),
                selection_color_id,
                paint_references: color.paint_references,
                color: color.color,
            },
        );
        cx.notify();
    }

    pub(super) fn emit_selection_color_variable_detach(
        &mut self,
        selection_color_id: SharedString,
        cx: &mut Context<Self>,
    ) {
        let Some(color) = self.selection_color(selection_color_id.as_ref()) else {
            return;
        };
        let Some(binding) = color.binding.as_ref().cloned() else {
            return;
        };
        if !self.selection_color_can_mutate(&color) || color.style_binding.is_some() {
            return;
        }
        self.selection_color_resource_browser = None;
        cx.emit_design_panel_action(
            self,
            DesignPanelAction::SelectionColorVariableDetachRequested {
                target: self.command_target(),
                selection_color_id,
                paint_references: color.paint_references,
                variable_id: binding.variable_id,
            },
        );
        cx.notify();
    }

    pub(super) fn render_selection_color_paint_style_button(
        &self,
        color: &super::super::DesignSelectionColor,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let color = color.clone();
        let selection_color_id = color.id.clone();
        let active = self
            .selection_color_resource_browser
            .as_ref()
            .is_some_and(|target| {
                target.selection_color_id == selection_color_id
                    && target.kind == SelectionColorResourceKind::PaintStyle
            });
        let can_mutate = self.selection_color_can_mutate(&color);
        let binding = color.style_binding.clone();
        let can_create = can_mutate && binding.is_none() && !color.style_paints.is_empty();
        let query = self.normalized_style_browser_query(cx);
        let source_filter = self.style_browser_source_filter.clone();
        let view_mode = self.style_browser_view_mode;
        let catalog_is_empty = self.paint_style_view_data.page_styles.is_empty()
            && self
                .paint_style_view_data
                .libraries
                .iter()
                .all(|library| library.styles.is_empty());
        let page_styles = if source_filter.includes_page() {
            self.paint_style_view_data
                .page_styles
                .iter()
                .map(|style| {
                    (
                        style.name.clone(),
                        DesignPaintStyleSelection::page(style.id.clone()),
                        style.import_state,
                        paint_style_summary(&style.paints),
                    )
                })
                .filter(|(name, _, _, summary)| {
                    Self::style_browser_row_matches(&query, name.as_ref(), summary.as_ref(), None)
                })
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        let libraries = self
            .paint_style_view_data
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
                                DesignPaintStyleSelection::library(
                                    library.id.clone(),
                                    style.id.clone(),
                                ),
                                style.import_state,
                                paint_style_summary(&style.paints),
                            )
                        })
                        .filter(|(name, _, _, summary)| {
                            Self::style_browser_row_matches(
                                &query,
                                name.as_ref(),
                                summary.as_ref(),
                                Some(library.name.as_ref()),
                            )
                        })
                        .collect::<Vec<_>>(),
                )
            })
            .filter(|(_, styles)| !styles.is_empty())
            .collect::<Vec<_>>();
        let panel = cx.entity();
        let panel_for_open = panel.clone();
        let panel_for_content = panel;
        let panel_id = self.id.clone();
        let style_browser_search = self.style_browser_search.clone();
        let style_browser_library_sources = self
            .paint_style_view_data
            .libraries
            .iter()
            .map(|library| (library.id.clone(), library.name.clone()))
            .collect::<Vec<_>>();
        let style_browser_scope = SharedString::from(format!(
            "{panel_id}-selection-color-{selection_color_id}-paint-style-browser"
        ));
        let id_for_open = selection_color_id.clone();
        let tooltip = binding
            .as_ref()
            .map_or_else(|| "Paint styles".into(), |binding| binding.name.clone());
        let trigger = Button::new(SharedString::from(format!(
            "{panel_id}-selection-color-{selection_color_id}-paint-style"
        )))
        .tooltip(tooltip)
        .xsmall()
        .compact()
        .ghost()
        .w(px(24.))
        .h(px(24.))
        .selected(active || binding.is_some())
        .child(self.render_color_styles_icon(cx))
        .on_activate(cx.listener(move |this, _, _, cx| {
            cx.stop_propagation();
            this.open_selection_color_resource_browser(
                id_for_open.clone(),
                SelectionColorResourceKind::PaintStyle,
                cx,
            );
        }));

        Popover::new(SharedString::from(format!(
            "{panel_id}-selection-color-{selection_color_id}-paint-style-popover"
        )))
        .anchor(Anchor::TopRight)
        .open(active)
        .overlay_closable(true)
        .on_open_change(move |open, _, cx| {
            let selection_color_id = selection_color_id.clone();
            panel_for_open.update(cx, |this, cx| {
                if *open {
                    this.open_selection_color_resource_browser(
                        selection_color_id,
                        SelectionColorResourceKind::PaintStyle,
                        cx,
                    );
                } else if this.selection_color_resource_browser.as_ref()
                    == Some(&SelectionColorResourceTarget {
                        selection_color_id,
                        kind: SelectionColorResourceKind::PaintStyle,
                    })
                {
                    this.selection_color_resource_browser = None;
                    cx.notify();
                }
            });
        })
        .trigger(trigger)
        .content(move |_, window, cx| {
            let mut content = v_flex()
                .w(popup_width(window, 280.))
                .max_h(popup_height(window, 520.))
                .overflow_y_scrollbar()
                .gap_1()
                .p_2()
                .child(
                    div()
                        .text_sm()
                        .font_semibold()
                        .child("Selection Paint styles"),
                );
            if let Some(binding) = binding.clone() {
                let panel = panel_for_content.clone();
                let color_id = color.id.clone();
                content = content.child(
                    h_flex()
                        .w_full()
                        .gap_2()
                        .child(div().flex_1().truncate().text_xs().child(binding.name))
                        .child(
                            Button::new(SharedString::from(format!(
                                "{panel_id}-selection-color-{}-style-detach",
                                color.id
                            )))
                            .label("Detach")
                            .xsmall()
                            .compact()
                            .ghost()
                            .disabled(!can_mutate || !binding.can_detach)
                            .on_activate(move |_, _, cx| {
                                panel.update(cx, |this, cx| {
                                    this.emit_selection_color_paint_style_detach(
                                        color_id.clone(),
                                        cx,
                                    );
                                });
                            }),
                        ),
                );
            }
            let panel = panel_for_content.clone();
            let color_id = color.id.clone();
            content = content.child(
                Button::new(SharedString::from(format!(
                    "{panel_id}-selection-color-{}-style-create",
                    color.id
                )))
                .label("Create style from collection")
                .xsmall()
                .compact()
                .ghost()
                .w_full()
                .disabled(!can_create)
                .on_activate(move |_, _, cx| {
                    panel.update(cx, |this, cx| {
                        this.emit_selection_color_paint_style_create(color_id.clone(), cx);
                    });
                }),
            );
            content = content.child(Self::render_style_browser_toolbar(
                panel_for_content.clone(),
                style_browser_scope.clone(),
                style_browser_search.clone(),
                source_filter.clone(),
                style_browser_library_sources.clone(),
                view_mode,
            ));
            if page_styles.is_empty() && libraries.iter().all(|(_, styles)| styles.is_empty()) {
                content = content.child(
                    div()
                        .py_2()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(if catalog_is_empty {
                            "No Paint styles supplied by the host"
                        } else {
                            "No styles match this search and source filter"
                        }),
                );
            }
            if !page_styles.is_empty() {
                content = content.child(
                    div()
                        .pt_2()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child("This page"),
                );
                let mut rows = if view_mode == StyleBrowserViewMode::Grid {
                    h_flex().w_full().gap_1().flex_wrap()
                } else {
                    v_flex().w_full().gap_0p5()
                };
                for (name, selection, import_state, summary) in page_styles.clone() {
                    let panel = panel_for_content.clone();
                    let color_id = color.id.clone();
                    let selection_for_click = selection.clone();
                    let selected = binding
                        .as_ref()
                        .is_some_and(|binding| binding.selection == selection);
                    rows = rows.child(
                        Button::new(SharedString::from(format!(
                            "{panel_id}-selection-color-{}-style-page-{}",
                            color.id, selection.style_id
                        )))
                        .label(name)
                        .tooltip(summary)
                        .xsmall()
                        .compact()
                        .ghost()
                        .when(view_mode == StyleBrowserViewMode::Grid, |button| {
                            button.w(px(127.)).h(px(44.))
                        })
                        .when(view_mode == StyleBrowserViewMode::List, |button| {
                            button.w_full()
                        })
                        .selected(selected)
                        .disabled(!can_mutate)
                        .on_activate(move |_, _, cx| {
                            let selection = selection_for_click.clone();
                            panel.update(cx, |this, cx| match import_state {
                                DesignPaintStyleImportState::Imported => this
                                    .emit_selection_color_paint_style_apply(
                                        color_id.clone(),
                                        selection,
                                        cx,
                                    ),
                                DesignPaintStyleImportState::Available => this
                                    .emit_selection_color_paint_style_import(
                                        color_id.clone(),
                                        selection,
                                        cx,
                                    ),
                            });
                        }),
                    );
                }
                content = content.child(rows);
            }
            for (library_name, styles) in libraries.clone() {
                content = content.child(
                    div()
                        .pt_2()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(library_name),
                );
                let mut rows = if view_mode == StyleBrowserViewMode::Grid {
                    h_flex().w_full().gap_1().flex_wrap()
                } else {
                    v_flex().w_full().gap_0p5()
                };
                for (name, selection, import_state, summary) in styles {
                    let panel = panel_for_content.clone();
                    let color_id = color.id.clone();
                    let selection_for_click = selection.clone();
                    let selected = binding
                        .as_ref()
                        .is_some_and(|binding| binding.selection == selection);
                    let label = if import_state == DesignPaintStyleImportState::Available {
                        format!("Import {name}")
                    } else {
                        name.to_string()
                    };
                    rows = rows.child(
                        Button::new(SharedString::from(format!(
                            "{panel_id}-selection-color-{}-style-library-{}",
                            color.id, selection.style_id
                        )))
                        .label(label)
                        .tooltip(summary)
                        .xsmall()
                        .compact()
                        .ghost()
                        .when(view_mode == StyleBrowserViewMode::Grid, |button| {
                            button.w(px(127.)).h(px(44.))
                        })
                        .when(view_mode == StyleBrowserViewMode::List, |button| {
                            button.w_full()
                        })
                        .selected(selected)
                        .disabled(!can_mutate)
                        .on_activate(move |_, _, cx| {
                            let selection = selection_for_click.clone();
                            panel.update(cx, |this, cx| match import_state {
                                DesignPaintStyleImportState::Imported => this
                                    .emit_selection_color_paint_style_apply(
                                        color_id.clone(),
                                        selection,
                                        cx,
                                    ),
                                DesignPaintStyleImportState::Available => this
                                    .emit_selection_color_paint_style_import(
                                        color_id.clone(),
                                        selection,
                                        cx,
                                    ),
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

    pub(super) fn render_selection_color_variable_button(
        &self,
        color: &super::super::DesignSelectionColor,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let color = color.clone();
        let selection_color_id = color.id.clone();
        let active = self
            .selection_color_resource_browser
            .as_ref()
            .is_some_and(|target| {
                target.selection_color_id == selection_color_id
                    && target.kind == SelectionColorResourceKind::ColorVariable
            });
        let can_mutate = self.selection_color_can_mutate(&color);
        let style_bound = color.style_binding.is_some();
        let solid = matches!(&color.paint.payload, DesignPaintPayload::Solid(_));
        let can_change_leaf = solid && can_mutate && !style_bound;
        let binding = color.binding.clone();
        let can_create = can_change_leaf && binding.is_none();
        let mut groups: Vec<(DesignVariableSource, Vec<DesignVariable>)> = Vec::new();
        for variable in self.paint_variable_view_data.variables.iter().cloned() {
            if let Some((_, variables)) = groups
                .iter_mut()
                .find(|(source, _)| *source == variable.source)
            {
                variables.push(variable);
            } else {
                groups.push((variable.source.clone(), vec![variable]));
            }
        }
        let panel = cx.entity();
        let panel_for_open = panel.clone();
        let panel_for_content = panel;
        let panel_id = self.id.clone();
        let id_for_open = selection_color_id.clone();
        let tooltip = binding.as_ref().map_or_else(
            || {
                if !solid {
                    "Color variables apply to Solid selection paints".into()
                } else if style_bound {
                    "Detach the Paint style before changing a leaf variable".into()
                } else {
                    "Color variables".into()
                }
            },
            |binding| binding.variable_name.clone(),
        );
        let trigger = Button::new(SharedString::from(format!(
            "{panel_id}-selection-color-{selection_color_id}-variable"
        )))
        .label("◇")
        .tooltip(tooltip)
        .xsmall()
        .compact()
        .ghost()
        .w(px(24.))
        .h(px(24.))
        .selected(active || binding.is_some())
        .disabled(!solid)
        .on_activate(cx.listener(move |this, _, _, cx| {
            cx.stop_propagation();
            this.open_selection_color_resource_browser(
                id_for_open.clone(),
                SelectionColorResourceKind::ColorVariable,
                cx,
            );
        }));

        Popover::new(SharedString::from(format!(
            "{panel_id}-selection-color-{selection_color_id}-variable-popover"
        )))
        .anchor(Anchor::TopRight)
        .open(active)
        .overlay_closable(true)
        .on_open_change(move |open, _, cx| {
            let selection_color_id = selection_color_id.clone();
            panel_for_open.update(cx, |this, cx| {
                if *open {
                    this.open_selection_color_resource_browser(
                        selection_color_id,
                        SelectionColorResourceKind::ColorVariable,
                        cx,
                    );
                } else if this.selection_color_resource_browser.as_ref()
                    == Some(&SelectionColorResourceTarget {
                        selection_color_id,
                        kind: SelectionColorResourceKind::ColorVariable,
                    })
                {
                    this.selection_color_resource_browser = None;
                    cx.notify();
                }
            });
        })
        .trigger(trigger)
        .content(move |_, window, cx| {
            let mut content = v_flex()
                .w(popup_width(window, 280.))
                .max_h(popup_height(window, 420.))
                .gap_1()
                .p_2()
                .child(
                    div()
                        .text_sm()
                        .font_semibold()
                        .child("Selection Color variables"),
                );
            if style_bound {
                content = content.child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child("Detach the Paint style before changing this color leaf."),
                );
            }
            if let Some(binding) = binding.clone() {
                let panel = panel_for_content.clone();
                let color_id = color.id.clone();
                content = content.child(
                    h_flex()
                        .w_full()
                        .gap_2()
                        .child(
                            div()
                                .flex_1()
                                .truncate()
                                .text_xs()
                                .child(binding.variable_name),
                        )
                        .child(
                            Button::new(SharedString::from(format!(
                                "{panel_id}-selection-color-{}-variable-detach",
                                color.id
                            )))
                            .label("Detach")
                            .xsmall()
                            .compact()
                            .ghost()
                            .disabled(!can_change_leaf)
                            .on_activate(move |_, _, cx| {
                                panel.update(cx, |this, cx| {
                                    this.emit_selection_color_variable_detach(color_id.clone(), cx);
                                });
                            }),
                        ),
                );
            }
            let panel = panel_for_content.clone();
            let color_id = color.id.clone();
            content = content.child(
                Button::new(SharedString::from(format!(
                    "{panel_id}-selection-color-{}-variable-create",
                    color.id
                )))
                .label("Create variable from color")
                .xsmall()
                .compact()
                .ghost()
                .w_full()
                .disabled(!can_create)
                .on_activate(move |_, _, cx| {
                    panel.update(cx, |this, cx| {
                        this.emit_selection_color_variable_create(color_id.clone(), cx);
                    });
                }),
            );
            if groups.is_empty() {
                content = content.child(
                    div()
                        .py_2()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child("No Color variables supplied by the host"),
                );
            }
            for (source, variables) in groups.clone() {
                content = content.child(
                    div()
                        .pt_2()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(source.label().clone()),
                );
                for variable in variables {
                    let panel = panel_for_content.clone();
                    let color_id = color.id.clone();
                    let variable_id = variable.id.clone();
                    let selected = binding
                        .as_ref()
                        .is_some_and(|binding| binding.variable_id == variable.id);
                    let label = if variable.import_state == DesignVariableImportState::Available {
                        format!("Import {}", variable.name)
                    } else {
                        variable.name.to_string()
                    };
                    let disabled = !can_change_leaf || variable.disabled_reason.is_some();
                    let tooltip = variable.disabled_reason.clone().unwrap_or_else(|| {
                        format!("{} · {}", variable.collection_name, source.label()).into()
                    });
                    content = content.child(
                        Button::new(SharedString::from(format!(
                            "{panel_id}-selection-color-{}-variable-{}",
                            color.id, variable.id
                        )))
                        .label(label)
                        .tooltip(tooltip)
                        .xsmall()
                        .compact()
                        .ghost()
                        .w_full()
                        .selected(selected)
                        .disabled(disabled)
                        .on_activate(move |_, _, cx| {
                            let variable_id = variable_id.clone();
                            panel.update(cx, |this, cx| match variable.import_state {
                                DesignVariableImportState::Local
                                | DesignVariableImportState::Imported => this
                                    .emit_selection_color_variable_apply(
                                        color_id.clone(),
                                        variable_id,
                                        cx,
                                    ),
                                DesignVariableImportState::Available => this
                                    .emit_selection_color_variable_import(
                                        color_id.clone(),
                                        variable_id,
                                        cx,
                                    ),
                            });
                        }),
                    );
                }
            }
            content
        })
        .into_any_element()
    }

    pub(super) fn render_selection_paint_picker_control(
        &self,
        index: usize,
        selection_color: &super::super::DesignSelectionColor,
        target: AuxiliaryColorPickerTarget,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let paint = &selection_color.paint;
        let paint_label = match &paint.payload {
            DesignPaintPayload::Solid(_) => format!("#{}", paint.color.hex()),
            DesignPaintPayload::Gradient(gradient) => {
                format!("{} · {} stops", gradient.kind.label(), gradient.stops.len())
            }
            DesignPaintPayload::Pattern(_) => "Pattern".to_owned(),
            DesignPaintPayload::Image(_) => "Image".to_owned(),
            DesignPaintPayload::Video(_) => "Video".to_owned(),
            DesignPaintPayload::Shader(shader) => format!("Shader · {}", shader.name),
            DesignPaintPayload::Unsupported(opaque) => opaque.type_name.to_string(),
        };
        let label = if selection_color.occurrence_count > 1 {
            format!("{paint_label} · {}", selection_color.occurrence_count)
        } else {
            paint_label
        };
        let open = self.auxiliary_color_picker.as_ref() == Some(&target);
        let panel = cx.entity();
        let picker = self.paint_picker.clone();
        let picker_content = picker.clone();
        let picker_focus = picker.focus_handle(cx);
        let trigger = Button::new(SharedString::from(format!(
            "{}-selection-paint-{index}-trigger",
            self.id
        )))
        .xsmall()
        .compact()
        .w_full()
        .h(px(28.))
        .justify_start()
        .selected(open)
        .on_keyboard_activate({
            let panel = panel.clone();
            let target = target.clone();
            move |window, cx| {
                let target = target.clone();
                panel.update(cx, |this, cx| {
                    if !open {
                        this.open_auxiliary_color_picker(target, window, cx);
                    } else if this.auxiliary_color_picker.as_ref() == Some(&target) {
                        this.prepare_paint_picker_for_dismissal(cx);
                        this.cancel_active_paint_edit(cx);
                        this.auxiliary_color_picker = None;
                    }
                    cx.notify();
                });
            }
        })
        .child(self.render_paint_swatch(paint, cx))
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .truncate()
                .text_left()
                .child(label),
        );

        Popover::new(SharedString::from(format!(
            "{}-selection-paint-{index}-popover",
            self.id
        )))
        .anchor(Anchor::TopRight)
        .open(open)
        .overlay_closable(true)
        .track_focus(&picker_focus)
        .on_open_change(move |open, window, cx| {
            let target = target.clone();
            panel.update(cx, |this, cx| {
                if *open {
                    this.open_auxiliary_color_picker(target, window, cx);
                } else if this.auxiliary_color_picker.as_ref() == Some(&target) {
                    this.prepare_paint_picker_for_dismissal(cx);
                    this.cancel_active_paint_edit(cx);
                    this.auxiliary_color_picker = None;
                }
                cx.notify();
            });
        })
        .trigger(trigger)
        .content(move |_, _, _| picker_content.clone())
        .into_any_element()
    }

    pub(super) fn render_selection_color_occurrences_button(
        &self,
        color: &super::super::DesignSelectionColor,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let enabled = self.selection_color_can_select_occurrences(color);
        let color_id = color.id.clone();
        Button::new(SharedString::from(format!(
            "{}-selection-color-{}-select-occurrences",
            self.id, color.id
        )))
        .icon(IconName::Maximize)
        .tooltip(if enabled {
            SharedString::from(format!(
                "Select all {} paint occurrences",
                color.occurrence_count
            ))
        } else {
            "Exact occurrence references unavailable".into()
        })
        .xsmall()
        .compact()
        .ghost()
        .w(px(24.))
        .h(px(24.))
        .disabled(!enabled)
        .on_activate(cx.listener(move |this, _, _, cx| {
            this.emit_selection_color_occurrences_select(color_id.clone(), cx);
        }))
        .into_any_element()
    }

    pub(super) fn render_selection_colors(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.inspection_context.selection().kind() != DesignPanelSelectionKind::Multiple {
            return None;
        }
        let colors = self.node.resolved_selection_colors();
        if colors.is_empty() {
            return None;
        }
        let mut content = v_flex().px(px(PANEL_PADDING)).pb_4().gap_1();
        for (index, selection_color) in colors.into_iter().enumerate() {
            let target = AuxiliaryColorPickerTarget::SelectionColor {
                target: self.command_target(),
                selection_color_id: selection_color.id.clone(),
            };
            content = content.child(
                h_flex()
                    .h(px(28.))
                    .w_full()
                    .gap_1()
                    .child(div().flex_1().min_w(px(0.)).child(
                        self.render_selection_paint_picker_control(
                            index,
                            &selection_color,
                            target,
                            cx,
                        ),
                    ))
                    .child(self.render_selection_color_paint_style_button(&selection_color, cx))
                    .child(self.render_selection_color_variable_button(&selection_color, cx))
                    .child(self.render_selection_color_occurrences_button(&selection_color, cx)),
            );
        }
        Some(self.render_section(
            DesignPanelSection::Selection,
            None,
            content.into_any_element(),
            cx,
        ))
    }

    pub(super) fn paint_collection(
        &self,
        collection: DesignPanelCollection,
    ) -> Option<&[DesignPaint]> {
        if !self.collection_is_supported(collection) {
            return None;
        }
        match collection {
            DesignPanelCollection::Fill => Some(&self.node.fills),
            DesignPanelCollection::Stroke => Some(self.node.stroke.as_ref()?.paints.as_slice()),
            DesignPanelCollection::Effect
            | DesignPanelCollection::LayoutGrid
            | DesignPanelCollection::Export => None,
        }
    }

    pub(super) fn paint_target_index(&self, target: &PaintPickerTarget) -> Option<usize> {
        let paints = self.paint_collection(target.collection)?;
        if !target.paint_id.is_empty() {
            return paints.iter().position(|paint| paint.id == target.paint_id);
        }
        (target.index < paints.len()).then_some(target.index)
    }

    pub(super) fn emit_media_source_action(
        &self,
        target: PaintPickerTarget,
        expected_source_id: &SharedString,
        action: DesignMediaSourceAction,
        cx: &mut Context<Self>,
    ) {
        if !self.can_edit()
            || self.paint_style_binding(target.collection).is_some()
            || !self.collection_is_supported(target.collection)
        {
            return;
        }
        let Some(index) = self.paint_target_index(&target) else {
            return;
        };
        let Some(paint) = self
            .paint_collection(target.collection)
            .and_then(|paints| paints.get(index))
        else {
            return;
        };
        if paint.read_only || !action.is_applicable_to(paint.paint_type()) {
            return;
        }
        let source_id = match &paint.payload {
            DesignPaintPayload::Image(image) => &image.source.id,
            DesignPaintPayload::Video(video) => &video.source.id,
            _ => return,
        };
        if source_id != expected_source_id {
            return;
        }
        let capabilities = self
            .media_paint_view_data
            .paint(target.collection, &target.paint_id, index)
            .map_or_else(DesignMediaPaintCapabilities::default, |view| {
                view.capabilities
            });
        if !capabilities.allows_source_action(action) {
            return;
        }
        cx.emit_design_panel_action(
            self,
            DesignPanelAction::PaintMediaSourceActionRequested {
                node_id: self.node.id.clone(),
                collection: target.collection,
                target: self.paint_target(target.collection),
                paint_id: target.paint_id,
                index,
                source_id: source_id.clone(),
                action,
            },
        );
    }

    pub(super) fn media_drop_capabilities_for_target(
        &self,
        target: &PaintPickerTarget,
    ) -> Option<(
        usize,
        DesignMediaKind,
        SharedString,
        DesignMediaPaintCapabilities,
    )> {
        if !self.can_edit()
            || self.paint_style_binding(target.collection).is_some()
            || !self.collection_is_supported(target.collection)
        {
            return None;
        }
        let index = self.paint_target_index(target)?;
        let paint = self.paint_collection(target.collection)?.get(index)?;
        if paint.read_only {
            return None;
        }
        let (media_kind, source_id) = match &paint.payload {
            DesignPaintPayload::Image(image) => (DesignMediaKind::Image, image.source.id.clone()),
            DesignPaintPayload::Video(video) => (DesignMediaKind::Video, video.source.id.clone()),
            _ => return None,
        };
        let capabilities = self
            .media_paint_view_data
            .paint(target.collection, &target.paint_id, index)
            .map_or_else(DesignMediaPaintCapabilities::default, |view| {
                view.capabilities
            });
        (capabilities.can_upload_source && !capabilities.accepted_drop_file_kinds.is_empty())
            .then_some((index, media_kind, source_id, capabilities))
    }

    pub(super) fn emit_media_source_drop_from_paths(
        &self,
        target: PaintPickerTarget,
        paths: &[PathBuf],
        cx: &mut Context<Self>,
    ) {
        let Some((_, expected_media_kind, expected_source_id, capabilities)) =
            self.media_drop_capabilities_for_target(&target)
        else {
            return;
        };
        let Some(file) = media_drop_from_paths(paths, capabilities) else {
            return;
        };
        self.emit_media_source_drop(target, &expected_source_id, expected_media_kind, file, cx);
    }

    pub(super) fn emit_media_source_drop(
        &self,
        target: PaintPickerTarget,
        expected_source_id: &SharedString,
        expected_media_kind: DesignMediaKind,
        file: DesignMediaDroppedFile,
        cx: &mut Context<Self>,
    ) {
        let Some((index, media_kind, source_id, capabilities)) =
            self.media_drop_capabilities_for_target(&target)
        else {
            return;
        };
        if media_kind != expected_media_kind
            || source_id != *expected_source_id
            || super::super::DesignMediaFileKind::from_path(&file.path) != Some(file.kind)
            || !capabilities.allows_file_drop(file.kind)
        {
            return;
        }
        cx.emit_design_panel_action(
            self,
            DesignPanelAction::PaintMediaSourceDropRequested {
                node_id: self.node.id.clone(),
                collection: target.collection,
                target: self.paint_target(target.collection),
                paint_id: target.paint_id,
                index,
                expected_source_id: source_id,
                expected_media_kind: media_kind,
                file,
            },
        );
    }

    pub(super) fn emit_crop_cancel_if_active(
        &self,
        target: &PaintPickerTarget,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.paint_target_index(target) else {
            return;
        };
        let Some(view) =
            self.media_paint_view_data
                .paint(target.collection, &target.paint_id, index)
        else {
            return;
        };
        let crop_paint = self.picker_paint(target).is_some_and(|paint| {
            matches!(
                paint.payload,
                DesignPaintPayload::Image(super::super::DesignImagePaint {
                    placement: super::super::DesignMediaPaintPlacement::Crop { .. },
                    ..
                }) | DesignPaintPayload::Video(super::super::DesignVideoPaint {
                    placement: super::super::DesignMediaPaintPlacement::Crop { .. },
                    ..
                })
            )
        });
        if self.can_edit()
            && view.capabilities.can_edit_properties
            && view.crop_tool.active
            && crop_paint
        {
            cx.emit_design_panel_action(
                self,
                DesignPanelAction::PaintMediaCropActionRequested {
                    node_id: self.node.id.clone(),
                    collection: target.collection,
                    target: self.paint_target(target.collection),
                    paint_id: target.paint_id.clone(),
                    index,
                    action: DesignMediaCropAction::Cancel,
                },
            );
        }
    }

    pub(super) fn resolve_paint_color_target(
        &self,
        target: &PaintPickerTarget,
        color_target: &DesignPaintColorTarget,
        allow_bound: bool,
    ) -> Option<DesignPaintColorTarget> {
        let paint = self.picker_paint(target)?;
        if paint.read_only {
            return None;
        }
        match (&paint.payload, color_target) {
            (DesignPaintPayload::Solid(solid), DesignPaintColorTarget::Solid)
                if allow_bound || solid.binding.is_none() =>
            {
                Some(DesignPaintColorTarget::Solid)
            }
            (
                DesignPaintPayload::Gradient(gradient),
                DesignPaintColorTarget::GradientStop { stop_id, index },
            ) => {
                let index = if stop_id.is_empty() {
                    (*index < gradient.stops.len()).then_some(*index)
                } else {
                    gradient.stops.iter().position(|stop| stop.id == *stop_id)
                }?;
                let stop = &gradient.stops[index];
                if !allow_bound && stop.binding.is_some() {
                    return None;
                }
                Some(DesignPaintColorTarget::GradientStop {
                    stop_id: stop.id.clone(),
                    index,
                })
            }
            _ => None,
        }
    }

    pub(super) fn paint_color_binding(
        &self,
        target: &PaintPickerTarget,
        color_target: &DesignPaintColorTarget,
    ) -> Option<super::super::DesignPaintBinding> {
        let paint = self.picker_paint(target)?;
        match (&paint.payload, color_target) {
            (DesignPaintPayload::Solid(solid), DesignPaintColorTarget::Solid) => {
                solid.binding.clone()
            }
            (
                DesignPaintPayload::Gradient(gradient),
                DesignPaintColorTarget::GradientStop { stop_id, index },
            ) => {
                let index = if stop_id.is_empty() {
                    (*index < gradient.stops.len()).then_some(*index)
                } else {
                    gradient.stops.iter().position(|stop| stop.id == *stop_id)
                }?;
                gradient.stops.get(index)?.binding.clone()
            }
            _ => None,
        }
    }

    pub(super) fn picker_paint(&self, target: &PaintPickerTarget) -> Option<DesignPaint> {
        let index = self.paint_target_index(target)?;
        self.paint_collection(target.collection)?
            .get(index)
            .cloned()
    }

    pub(super) fn auxiliary_color_state_properties(
        &self,
        target: &AuxiliaryColorPickerTarget,
    ) -> Vec<DesignPanelProperty> {
        match target {
            AuxiliaryColorPickerTarget::PageBackground { .. } => Vec::new(),
            AuxiliaryColorPickerTarget::TextDecoration { .. } => {
                vec![DesignPanelProperty::TextDecorationColor]
            }
            AuxiliaryColorPickerTarget::Effect {
                effect_id,
                index,
                property,
                ..
            } => {
                let resolved_index = if effect_id.is_empty() {
                    self.node.effects.get(*index).map(|_| *index)
                } else {
                    self.node
                        .effects
                        .iter()
                        .position(|effect| effect.id == *effect_id)
                };
                resolved_index
                    .map(|index| vec![property.with_effect_index(index)])
                    .unwrap_or_default()
            }
            AuxiliaryColorPickerTarget::LayoutGrid {
                guide_id, index, ..
            } => {
                let resolved_index = if guide_id.is_empty() {
                    self.node.layout_grids.get(*index).map(|_| *index)
                } else {
                    self.node
                        .layout_grids
                        .iter()
                        .position(|guide| guide.id == *guide_id)
                };
                resolved_index.map_or_else(Vec::new, |index| {
                    vec![
                        DesignPanelProperty::LayoutGridColor(index),
                        DesignPanelProperty::LayoutGridOpacity(index),
                    ]
                })
            }
            AuxiliaryColorPickerTarget::SelectionColor {
                selection_color_id, ..
            } => self
                .node
                .resolved_selection_colors()
                .iter()
                .position(|color| color.id == *selection_color_id)
                .map(|index| vec![DesignPanelProperty::SelectionColor(index)])
                .unwrap_or_default(),
        }
    }

    pub(super) fn active_auxiliary_state_property(&self) -> Option<DesignPanelProperty> {
        let target = self.auxiliary_color_picker.as_ref()?;
        let active = self
            .active_paint_edit
            .as_ref()
            .filter(|active| target.matches_picker_target(&active.target))?;
        let properties = self.auxiliary_color_state_properties(target);
        match (&target, &active.edit.property) {
            (AuxiliaryColorPickerTarget::LayoutGrid { .. }, DesignPaintProperty::Color) => {
                properties.first().copied()
            }
            (AuxiliaryColorPickerTarget::LayoutGrid { .. }, DesignPaintProperty::Opacity) => {
                properties.get(1).copied()
            }
            (_, DesignPaintProperty::Color | DesignPaintProperty::Opacity) => {
                properties.first().copied()
            }
            _ => None,
        }
    }

    pub(super) fn auxiliary_color_leaf_editability(
        &self,
        target: &AuxiliaryColorPickerTarget,
    ) -> (bool, bool) {
        match target {
            AuxiliaryColorPickerTarget::PageBackground { page_id } => {
                let editable = self.page_view_data_for_context().is_some_and(|page| {
                    page.page_id == *page_id && self.can_edit_page() && !page.background.read_only
                });
                (editable, editable)
            }
            AuxiliaryColorPickerTarget::TextDecoration { node_id, .. } => {
                let editable = *node_id == self.node.id
                    && self.property_is_editable(DesignPanelProperty::TextDecorationColor);
                (editable, editable)
            }
            AuxiliaryColorPickerTarget::Effect {
                node_id,
                effect_id,
                index,
                property,
            } => {
                let resolved_index = if effect_id.is_empty() {
                    (*index < self.node.effects.len()).then_some(*index)
                } else {
                    self.node
                        .effects
                        .iter()
                        .position(|effect| effect.id == *effect_id)
                };
                let editable = *node_id == self.node.id
                    && resolved_index.is_some_and(|resolved_index| {
                        self.property_is_editable(property.with_effect_index(resolved_index))
                    });
                (editable, editable)
            }
            AuxiliaryColorPickerTarget::LayoutGrid {
                node_id,
                guide_id,
                index,
            } => {
                let resolved_index = if guide_id.is_empty() {
                    (*index < self.node.layout_grids.len()).then_some(*index)
                } else {
                    self.node
                        .layout_grids
                        .iter()
                        .position(|guide| guide.id == *guide_id)
                };
                if *node_id != self.node.id {
                    return (false, false);
                }
                resolved_index.map_or((false, false), |resolved_index| {
                    (
                        self.property_is_editable(DesignPanelProperty::LayoutGridColor(
                            resolved_index,
                        )),
                        self.property_is_editable(DesignPanelProperty::LayoutGridOpacity(
                            resolved_index,
                        )),
                    )
                })
            }
            AuxiliaryColorPickerTarget::SelectionColor {
                selection_color_id, ..
            } => {
                let Some(color) = self
                    .node
                    .resolved_selection_colors()
                    .into_iter()
                    .find(|color| color.id == *selection_color_id)
                else {
                    return (false, false);
                };
                let paint_editable =
                    self.selection_color_can_mutate(&color) && color.style_binding.is_none();
                let color_editable = paint_editable
                    && color.binding.is_none()
                    && matches!(color.paint.payload, DesignPaintPayload::Solid(_));
                (color_editable, paint_editable)
            }
        }
    }

    #[cfg(test)]
    pub(super) fn auxiliary_color_editable(&self, target: &AuxiliaryColorPickerTarget) -> bool {
        let (color_editable, opacity_editable) = self.auxiliary_color_leaf_editability(target);
        color_editable || opacity_editable
    }

    pub(super) fn auxiliary_paint_property_editable(
        &self,
        target: &AuxiliaryColorPickerTarget,
        property: &DesignPaintProperty,
    ) -> bool {
        if let AuxiliaryColorPickerTarget::SelectionColor {
            selection_color_id, ..
        } = target
        {
            let Some(color) = self.selection_color(selection_color_id.as_ref()) else {
                return false;
            };
            let Some(paint) = self.auxiliary_color_paint(target) else {
                return false;
            };
            let candidate = match property {
                DesignPaintProperty::Payload => DesignPaintEdit {
                    property: DesignPaintProperty::Payload,
                    value: DesignPaintValue::Payload(paint.payload),
                },
                DesignPaintProperty::Opacity => DesignPaintEdit {
                    property: DesignPaintProperty::Opacity,
                    value: DesignPaintValue::Number(paint.opacity),
                },
                DesignPaintProperty::Visible => DesignPaintEdit {
                    property: DesignPaintProperty::Visible,
                    value: DesignPaintValue::Bool(paint.visible),
                },
                DesignPaintProperty::BlendMode => DesignPaintEdit {
                    property: DesignPaintProperty::BlendMode,
                    value: DesignPaintValue::BlendMode(paint.blend_mode),
                },
                _ => {
                    return self.selection_color_can_mutate(&color)
                        && color.style_binding.is_none();
                }
            };
            return self.selection_color_paint_editable(&color, &candidate);
        }
        let (color_editable, opacity_editable) = self.auxiliary_color_leaf_editability(target);
        match property {
            DesignPaintProperty::Color => color_editable,
            DesignPaintProperty::Opacity => opacity_editable,
            _ => false,
        }
    }

    pub(super) fn auxiliary_paint_editable(
        &self,
        target: &AuxiliaryColorPickerTarget,
        edit: &DesignPaintEdit,
    ) -> bool {
        if let AuxiliaryColorPickerTarget::SelectionColor {
            selection_color_id, ..
        } = target
        {
            return self
                .selection_color(selection_color_id.as_ref())
                .is_some_and(|color| self.selection_color_paint_editable(&color, edit));
        }
        self.auxiliary_paint_property_editable(target, &edit.property)
    }

    pub(super) fn open_auxiliary_color_picker(
        &mut self,
        target: AuxiliaryColorPickerTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(paint) = self.auxiliary_color_paint(&target) else {
            return;
        };
        let editability = self.auxiliary_color_leaf_editability(&target);
        self.cancel_active_paint_edit(cx);
        self.cancel_menu_preview(cx);
        self.active_picker = None;
        self.paint_style_browser_open = None;
        self.auxiliary_color_picker = Some(target.clone());
        self.effect_style_browser_open = false;
        self.typography_style_picker_open = false;
        self.selection_header_overlay = None;
        let title = SharedString::from(target.title());
        let full_paint = matches!(target, AuxiliaryColorPickerTarget::SelectionColor { .. });
        let disabled = full_paint
            && target
                .selection_color_id()
                .and_then(|id| self.selection_color(id.as_ref()))
                .is_none_or(|color| {
                    !self.selection_color_can_mutate(&color)
                        || color.style_binding.is_some()
                        || !matches!(
                            &color.paint.payload,
                            DesignPaintPayload::Solid(_) | DesignPaintPayload::Gradient(_)
                        )
                });
        let contrast_view_data = self.active_color_contrast_view_data();
        self.paint_picker.update(cx, |picker, cx| {
            picker.set_color_only((!full_paint).then_some(title), cx);
            picker.set_target(
                AuxiliaryColorPickerTarget::PICKER_NODE_ID,
                DesignPanelCollection::Fill,
                0,
                paint,
                window,
                cx,
            );
            picker.set_disabled(disabled, cx);
            picker.set_color_only_editability(editability.0, editability.1, cx);
            picker.set_contrast_view_data(contrast_view_data, cx);
        });
        cx.notify();
    }

    pub(super) fn active_color_contrast_view_data(
        &self,
    ) -> Option<DesignColorContrastPaintViewData> {
        let target = if let Some(AuxiliaryColorPickerTarget::SelectionColor {
            target,
            selection_color_id,
        }) = self.auxiliary_color_picker.as_ref()
        {
            if *target != self.command_target() {
                return None;
            }
            DesignColorContrastPaintTarget::selection_color(
                target.clone(),
                selection_color_id.clone(),
            )
        } else if self.auxiliary_color_picker.is_none() {
            let target = self.active_picker.as_ref()?;
            let index = self.paint_target_index(target)?;
            DesignColorContrastPaintTarget::paint(
                self.node.id.clone(),
                target.collection,
                target.paint_id.clone(),
                index,
            )
        } else {
            return None;
        };
        self.color_contrast_view_data.paint(&target).cloned()
    }

    pub(super) fn sync_paint_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let contrast_view_data = self.active_color_contrast_view_data();
        if let Some(target) = self.auxiliary_color_picker.clone() {
            let Some(paint) = self.auxiliary_color_paint(&target) else {
                self.cancel_active_paint_edit(cx);
                self.auxiliary_color_picker = None;
                self.paint_picker
                    .update(cx, |picker, cx| picker.clear(window, cx));
                return;
            };
            let editability = self.auxiliary_color_leaf_editability(&target);
            let paint_id = target.picker_paint_id();
            let title = SharedString::from(target.title());
            let full_paint = matches!(target, AuxiliaryColorPickerTarget::SelectionColor { .. });
            let disabled = full_paint
                && target
                    .selection_color_id()
                    .and_then(|id| self.selection_color(id.as_ref()))
                    .is_none_or(|color| {
                        !self.selection_color_can_mutate(&color)
                            || color.style_binding.is_some()
                            || !matches!(
                                &color.paint.payload,
                                DesignPaintPayload::Solid(_) | DesignPaintPayload::Gradient(_)
                            )
                    });
            let picker_matches = {
                let picker = self.paint_picker.read(cx);
                picker.target().is_some_and(|current| {
                    current.node_id.as_ref() == AuxiliaryColorPickerTarget::PICKER_NODE_ID
                        && current.paint_id == paint_id
                        && current.index == 0
                }) && picker.paint() == Some(&paint)
                    && picker.is_disabled() == disabled
                    && picker.color_only_editability() == editability
            };
            if !picker_matches {
                self.paint_picker.update(cx, |picker, cx| {
                    picker.set_color_only((!full_paint).then_some(title), cx);
                    picker.set_target(
                        AuxiliaryColorPickerTarget::PICKER_NODE_ID,
                        DesignPanelCollection::Fill,
                        0,
                        paint,
                        window,
                        cx,
                    );
                    picker.set_disabled(disabled, cx);
                    picker.set_color_only_editability(editability.0, editability.1, cx);
                    picker.set_contrast_view_data(contrast_view_data, cx);
                });
            } else {
                self.paint_picker.update(cx, |picker, cx| {
                    picker.set_contrast_view_data(contrast_view_data, cx);
                });
            }
            return;
        }
        let desired = self.active_picker.as_ref().and_then(|target| {
            let index = self.paint_target_index(target)?;
            self.picker_paint(target)
                .map(|paint| (target.clone(), index, paint))
        });
        let disabled = !self.can_edit()
            || desired.as_ref().is_some_and(|(target, _, paint)| {
                paint.read_only || self.paint_style_binding(target.collection).is_some()
            });
        let (target_matches, paint_matches, disabled_matches, has_target) = {
            let picker = self.paint_picker.read(cx);
            let target_matches = match (&desired, picker.target()) {
                (Some((target, index, _)), Some(current)) => {
                    current.node_id == self.node.id
                        && current.collection == target.collection
                        && if current.paint_id.is_empty() || target.paint_id.is_empty() {
                            current.index == *index
                        } else {
                            current.paint_id == target.paint_id && current.index == *index
                        }
                }
                (None, None) => true,
                _ => false,
            };
            let paint_matches = match (&desired, picker.paint()) {
                (Some((_, _, paint)), Some(current)) => current == paint,
                (None, None) => true,
                _ => false,
            };
            (
                target_matches,
                paint_matches,
                picker.is_disabled() == disabled,
                picker.target().is_some(),
            )
        };
        if target_matches && paint_matches && disabled_matches {
            self.paint_picker.update(cx, |picker, cx| {
                picker.set_contrast_view_data(contrast_view_data, cx);
            });
            return;
        }

        let node_id = self.node.id.clone();
        self.paint_picker.update(cx, |picker, cx| {
            picker.set_color_only(None, cx);
            match desired {
                Some((target, index, paint)) if !target_matches || !paint_matches => {
                    picker.set_target(node_id, target.collection, index, paint, window, cx);
                }
                None if has_target => picker.clear(window, cx),
                _ => {}
            }
            if !disabled_matches {
                picker.set_disabled(disabled, cx);
            }
            picker.set_contrast_view_data(contrast_view_data, cx);
        });
    }

    pub(super) fn render_page_background_picker(
        &self,
        page: &DesignPageViewData,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let panel = cx.entity();
        let panel_for_open = panel.clone();
        let picker = self.paint_picker.clone();
        let picker_content = picker.clone();
        let picker_focus = picker.focus_handle(cx);
        let current = page.background.color;
        let target = AuxiliaryColorPickerTarget::PageBackground {
            page_id: page.page_id.clone(),
        };
        let open = self.page_background_picker_open
            && self.auxiliary_color_picker.as_ref() == Some(&target);
        let disabled_reason = if !self.inspection_context.permissions().can_edit() {
            Some(SharedString::from("View only"))
        } else if page.background.read_only {
            page.background
                .disabled_reason
                .clone()
                .or_else(|| Some("Page background is read only".into()))
        } else {
            None
        };
        let trigger = Button::new(SharedString::from(format!(
            "{}-page-background-trigger",
            self.id
        )))
        .label(SharedString::from(format!("#{}", current.hex())))
        .tooltip(
            disabled_reason
                .clone()
                .unwrap_or_else(|| "Change Page background".into()),
        )
        .xsmall()
        .compact()
        .ghost()
        .on_keyboard_activate({
            let panel = panel.clone();
            let target = target.clone();
            move |window, cx| {
                let target = target.clone();
                panel.update(cx, |this, cx| {
                    this.page_background_picker_open = !open;
                    if !open {
                        this.page_resource_browser = None;
                        this.variable_mode_browser_open = false;
                        this.open_auxiliary_color_picker(target, window, cx);
                    } else if this.auxiliary_color_picker.as_ref() == Some(&target) {
                        this.prepare_paint_picker_for_dismissal(cx);
                        this.cancel_active_paint_edit(cx);
                        this.auxiliary_color_picker = None;
                    }
                    cx.notify();
                });
            }
        });

        Popover::new(SharedString::from(format!(
            "{}-page-background-popover",
            self.id
        )))
        .anchor(Anchor::TopRight)
        .open(open)
        .overlay_closable(true)
        .track_focus(&picker_focus)
        .on_open_change(move |open, window, cx| {
            let target = target.clone();
            panel_for_open.update(cx, |this, cx| {
                this.page_background_picker_open = *open;
                if *open {
                    this.page_resource_browser = None;
                    this.variable_mode_browser_open = false;
                    this.open_auxiliary_color_picker(target, window, cx);
                } else if this.auxiliary_color_picker.as_ref() == Some(&target) {
                    this.prepare_paint_picker_for_dismissal(cx);
                    this.cancel_active_paint_edit(cx);
                    this.auxiliary_color_picker = None;
                }
                cx.notify();
            });
        })
        .trigger(trigger)
        .content(move |_, _, _| picker_content.clone())
        .into_any_element()
    }
}

fn paint_style_summary(paints: &[DesignPaint]) -> SharedString {
    if paints.is_empty() {
        return "Empty Paint style".into();
    }
    let kinds = paints
        .iter()
        .map(|paint| paint.paint_type().label())
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "{} ordered paint{} · {kinds}",
        paints.len(),
        if paints.len() == 1 { "" } else { "s" }
    )
    .into()
}
