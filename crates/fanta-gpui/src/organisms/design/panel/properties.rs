use super::*;

impl DesignPanel {
    pub(super) fn can_edit(&self) -> bool {
        self.inspection_context.permissions().can_edit()
            && self.inspection_context.selection().kind() != DesignPanelSelectionKind::None
    }

    pub(super) fn generic_property_is_copyable(&self, property: DesignPanelProperty) -> bool {
        let permissions = self.inspection_context.permissions();
        !permissions.can_edit()
            && permissions.can_copy()
            && self.inspection_context.selection().kind() != DesignPanelSelectionKind::None
            && !self.property_is_editable(property)
    }

    pub(super) fn active_vector_edit(&self) -> Option<&DesignVectorEditViewData> {
        (self.can_edit()
            && self.inspection_context.selection().kind() == DesignPanelSelectionKind::Single
            && self.inspection_context.edit_mode() == DesignPanelEditMode::Vector
            && matches!(
                self.node.kind,
                DesignPanelNodeKind::Vector | DesignPanelNodeKind::TextPath
            ))
        .then_some(self.node.vector_edit.as_ref())
        .flatten()
        .filter(|view_data| view_data.is_valid() && view_data.has_selection())
    }

    pub(super) fn vector_property_is_contextual(property: DesignPanelProperty) -> bool {
        matches!(
            property,
            DesignPanelProperty::VectorVertexX
                | DesignPanelProperty::VectorVertexY
                | DesignPanelProperty::VectorVertexCornerRadius
                | DesignPanelProperty::VectorHandleMirroring
        )
    }

    pub(super) fn vector_property_is_mixed(&self, property: DesignPanelProperty) -> bool {
        let Some(view_data) = self.active_vector_edit() else {
            return false;
        };
        match property {
            DesignPanelProperty::VectorVertexX => view_data.selected_x().is_mixed(),
            DesignPanelProperty::VectorVertexY => view_data.selected_y().is_mixed(),
            DesignPanelProperty::VectorVertexCornerRadius => {
                view_data.selected_corner_radius().is_mixed()
            }
            DesignPanelProperty::VectorHandleMirroring => {
                view_data.selected_handle_mirroring().is_mixed()
            }
            _ => false,
        }
    }

    pub(super) fn collection_is_supported(&self, collection: DesignPanelCollection) -> bool {
        match collection {
            DesignPanelCollection::Fill => {
                self.node.supports_fill() && self.node.supports_section(DesignPanelSection::Fill)
            }
            DesignPanelCollection::Stroke => {
                self.node.supports_stroke()
                    && self.node.supports_section(DesignPanelSection::Stroke)
            }
            DesignPanelCollection::Effect => {
                self.node.supports_effects()
                    && self.node.supports_section(DesignPanelSection::Effects)
            }
            DesignPanelCollection::LayoutGrid => {
                self.node.supports_layout_guides()
                    && self.node.supports_section(DesignPanelSection::LayoutGrid)
            }
            DesignPanelCollection::Export => self.node.supports_section(DesignPanelSection::Export),
        }
    }

    pub(super) fn node_capability_allows_property(&self, property: DesignPanelProperty) -> bool {
        if property.effect_index().is_some() {
            return self.collection_is_supported(DesignPanelCollection::Effect);
        }
        if property.layout_grid_index().is_some() {
            return self.collection_is_supported(DesignPanelCollection::LayoutGrid);
        }
        if property.is_typography() {
            return self.node.supports_section(DesignPanelSection::Typography);
        }

        match property {
            DesignPanelProperty::Visible => {
                self.node.supports_visibility()
                    && self.node.supports_section(DesignPanelSection::Layer)
            }
            DesignPanelProperty::Opacity | DesignPanelProperty::BlendMode => {
                self.node.supports_layer_appearance()
                    && self.node.supports_section(DesignPanelSection::Layer)
            }
            DesignPanelProperty::X | DesignPanelProperty::Y => {
                self.node.supports_position_coordinates()
                    && self.node.supports_section(DesignPanelSection::Position)
            }
            DesignPanelProperty::SmartSelectionHorizontalSpacing
            | DesignPanelProperty::SmartSelectionVerticalSpacing
            | DesignPanelProperty::AlignSelection
            | DesignPanelProperty::DistributeSelection => {
                self.node.supports_arrange()
                    && self.node.supports_section(DesignPanelSection::Position)
            }
            DesignPanelProperty::VectorVertexX
            | DesignPanelProperty::VectorVertexY
            | DesignPanelProperty::VectorVertexCornerRadius
            | DesignPanelProperty::VectorHandleMirroring => {
                self.node.supports_section(DesignPanelSection::Position)
            }
            DesignPanelProperty::Rotation => {
                self.node.supports_transforms()
                    && self.node.supports_section(DesignPanelSection::Position)
            }
            DesignPanelProperty::GridRowIndex
            | DesignPanelProperty::GridColumnIndex
            | DesignPanelProperty::GridRowSpan
            | DesignPanelProperty::GridColumnSpan
            | DesignPanelProperty::GridHorizontalAlignment
            | DesignPanelProperty::GridVerticalAlignment => {
                self.node.supports_auto_layout_child()
                    && self.node.supports_section(DesignPanelSection::Position)
            }
            DesignPanelProperty::Width
            | DesignPanelProperty::Height
            | DesignPanelProperty::HorizontalSizing
            | DesignPanelProperty::VerticalSizing
            | DesignPanelProperty::MinWidth
            | DesignPanelProperty::MaxWidth
            | DesignPanelProperty::MinHeight
            | DesignPanelProperty::MaxHeight => {
                self.node.supports_dimensions()
                    && self.node.supports_section(DesignPanelSection::Layout)
            }
            DesignPanelProperty::LayoutPositioning
            | DesignPanelProperty::LayoutAlignSelf
            | DesignPanelProperty::LayoutGrow => {
                self.node.supports_auto_layout_child()
                    && self.node.supports_dimensions()
                    && self.node.supports_section(DesignPanelSection::Layout)
            }
            DesignPanelProperty::LockAspectRatio => {
                !self.node.is_component_instance_child
                    && self.node.supports_aspect_ratio_lock()
                    && self.node.supports_dimensions()
                    && self.node.supports_section(DesignPanelSection::Layout)
            }
            DesignPanelProperty::HorizontalConstraint | DesignPanelProperty::VerticalConstraint => {
                self.node.supports_constraints()
                    && self.node.supports_section(DesignPanelSection::Position)
            }
            DesignPanelProperty::LayoutMode
            | DesignPanelProperty::AutoLayoutAlignment
            | DesignPanelProperty::AlignmentX
            | DesignPanelProperty::AlignmentY
            | DesignPanelProperty::Wrap
            | DesignPanelProperty::Gap
            | DesignPanelProperty::ItemSpacingMode
            | DesignPanelProperty::CounterAxisAlignContent
            | DesignPanelProperty::CounterAxisGap
            | DesignPanelProperty::PaddingVertical
            | DesignPanelProperty::PaddingHorizontal
            | DesignPanelProperty::PaddingShorthand
            | DesignPanelProperty::PaddingTop
            | DesignPanelProperty::PaddingRight
            | DesignPanelProperty::PaddingBottom
            | DesignPanelProperty::PaddingLeft
            | DesignPanelProperty::IncludeStrokes
            | DesignPanelProperty::StackingOrder
            | DesignPanelProperty::BaselineAlignment => {
                self.node.supports_auto_layout_container()
                    && self.node.supports_section(DesignPanelSection::Layout)
            }
            DesignPanelProperty::ClipContent => {
                self.node.supports_clip_content()
                    && self.node.supports_section(DesignPanelSection::Layout)
            }
            DesignPanelProperty::GridAutoTracks
            | DesignPanelProperty::GridItemsPositioning
            | DesignPanelProperty::GridColumnCount
            | DesignPanelProperty::GridRowCount
            | DesignPanelProperty::GridColumnTrack(_)
            | DesignPanelProperty::GridRowTrack(_)
            | DesignPanelProperty::GridColumnTrackValue(_)
            | DesignPanelProperty::GridRowTrackValue(_) => {
                self.node.supports_grid_auto_layout()
                    && self.node.supports_section(DesignPanelSection::Layout)
            }
            DesignPanelProperty::CornerRadius
            | DesignPanelProperty::CornerRadiusTopLeft
            | DesignPanelProperty::CornerRadiusTopRight
            | DesignPanelProperty::CornerRadiusBottomRight
            | DesignPanelProperty::CornerRadiusBottomLeft
            | DesignPanelProperty::IndependentCorners
            | DesignPanelProperty::CornerSmoothing => {
                self.node.supports_section(DesignPanelSection::Layer)
            }
            DesignPanelProperty::FillShowsInExports => {
                self.collection_is_supported(DesignPanelCollection::Fill)
            }
            DesignPanelProperty::TextPathStartSegment
            | DesignPanelProperty::TextPathStartPosition => {
                self.text_path_start_debug_controls_are_available()
            }
            DesignPanelProperty::ComponentProperty(_)
            | DesignPanelProperty::SlotStretchChildOnInsert(_)
            | DesignPanelProperty::SlotDisplayEmpty(_)
            | DesignPanelProperty::SlotMinimumInstances(_)
            | DesignPanelProperty::SlotMaximumInstances(_)
            | DesignPanelProperty::SlotPreferredValuesOnly(_) => {
                self.node.supports_section(DesignPanelSection::Component)
                    || self.node.supports_section(DesignPanelSection::Instance)
            }
            DesignPanelProperty::MediaCropMode
            | DesignPanelProperty::MediaExposure
            | DesignPanelProperty::MediaContrast
            | DesignPanelProperty::MediaSaturation
            | DesignPanelProperty::MediaTemperature
            | DesignPanelProperty::MediaTint
            | DesignPanelProperty::MediaHighlights
            | DesignPanelProperty::MediaShadows => {
                self.collection_is_supported(DesignPanelCollection::Fill)
            }
            DesignPanelProperty::PolygonCount
            | DesignPanelProperty::StarPointCount
            | DesignPanelProperty::StarInnerRadius
            | DesignPanelProperty::ArcStartingAngle
            | DesignPanelProperty::ArcSweep
            | DesignPanelProperty::ArcInnerRadius => {
                self.node.supports_section(DesignPanelSection::Layer)
                    || self.node.supports_section(DesignPanelSection::Geometry)
            }
            DesignPanelProperty::ArcEndingAngle
            | DesignPanelProperty::BooleanOperation
            | DesignPanelProperty::TableRows
            | DesignPanelProperty::TableColumns => {
                self.node.supports_section(DesignPanelSection::Geometry)
            }
            DesignPanelProperty::IsMask | DesignPanelProperty::MaskType => {
                self.node.supports_section(DesignPanelSection::Mask)
                    || self.node.supports_section(DesignPanelSection::Geometry)
            }
            DesignPanelProperty::SectionContentsHidden | DesignPanelProperty::SectionDevStatus => {
                self.node.supports_section(DesignPanelSection::Section)
            }
            DesignPanelProperty::TransformRepeatType(_)
            | DesignPanelProperty::TransformRepeatAxis(_)
            | DesignPanelProperty::TransformRepeatCount(_)
            | DesignPanelProperty::TransformRepeatUnit(_)
            | DesignPanelProperty::TransformRepeatOffset(_) => {
                self.node.supports_section(DesignPanelSection::Transform)
            }
            DesignPanelProperty::SelectionColor(_) => {
                self.node.supports_section(DesignPanelSection::Selection)
            }
            DesignPanelProperty::PaintOpacity { collection, index } => {
                self.collection_is_supported(collection)
                    && self
                        .paint_collection(collection)
                        .and_then(|paints| paints.get(index))
                        .is_some_and(|paint| !paint.read_only)
            }
            DesignPanelProperty::PaintVisible { collection, index } => {
                self.collection_is_supported(collection)
                    && self
                        .node
                        .paint_collection_edit_mode(collection)
                        .allows_full_controls()
                    && self
                        .paint_collection(collection)
                        .and_then(|paints| paints.get(index))
                        .is_some_and(|paint| !paint.read_only)
            }
            DesignPanelProperty::StrokeWeight
            | DesignPanelProperty::StrokeWeightMode
            | DesignPanelProperty::StrokeWeightTop
            | DesignPanelProperty::StrokeWeightRight
            | DesignPanelProperty::StrokeWeightBottom
            | DesignPanelProperty::StrokeWeightLeft
            | DesignPanelProperty::StrokeAlign
            | DesignPanelProperty::StrokeStartCap
            | DesignPanelProperty::StrokeEndCap
            | DesignPanelProperty::StrokeEndpointCap
            | DesignPanelProperty::StrokeDashMode
            | DesignPanelProperty::StrokeDashPattern
            | DesignPanelProperty::StrokeDashCap
            | DesignPanelProperty::StrokeJoin
            | DesignPanelProperty::StrokeMiterAngle
            | DesignPanelProperty::StrokeVariableWidth
            | DesignPanelProperty::StrokeVariableWidthPointPosition(_)
            | DesignPanelProperty::StrokeVariableWidthPointWidth(_)
            | DesignPanelProperty::StrokeType
            | DesignPanelProperty::StrokeStretchBrush
            | DesignPanelProperty::StrokeBrushDirection
            | DesignPanelProperty::StrokeScatterBrush
            | DesignPanelProperty::StrokeScatterGap
            | DesignPanelProperty::StrokeScatterWiggle
            | DesignPanelProperty::StrokeScatterSizeJitter
            | DesignPanelProperty::StrokeScatterAngularJitter
            | DesignPanelProperty::StrokeScatterRotation
            | DesignPanelProperty::StrokeDynamicFrequency
            | DesignPanelProperty::StrokeDynamicWiggle
            | DesignPanelProperty::StrokeDynamicSmoothen => {
                self.collection_is_supported(DesignPanelCollection::Stroke)
            }
            DesignPanelProperty::ExportSizing(_)
            | DesignPanelProperty::ExportScale(_)
            | DesignPanelProperty::ExportSuffix(_)
            | DesignPanelProperty::ExportFormat(_) => {
                self.collection_is_supported(DesignPanelCollection::Export)
            }
            _ => false,
        }
    }

    pub(super) fn node_capability_allows_action(&self, action: &DesignPanelAction) -> bool {
        match action {
            DesignPanelAction::PropertyChangeRequested { property, .. }
            | DesignPanelAction::PropertyEditRequested { property, .. } => {
                self.node_capability_allows_property(*property)
            }
            DesignPanelAction::ResizeToFitRequested { .. } => {
                self.node.supports_resize_to_fit()
                    && self.node.supports_section(DesignPanelSection::Layout)
            }
            DesignPanelAction::FramePresetApplyRequested { .. } => {
                self.node.kind == DesignPanelNodeKind::Frame
                    && self.node.supports_dimensions()
                    && self.node.supports_section(DesignPanelSection::Layout)
            }
            DesignPanelAction::AddAutoLayoutRequested { .. } => {
                self.node.supports_add_auto_layout()
                    && self.node.supports_section(DesignPanelSection::Layout)
            }
            DesignPanelAction::ArrangeRequested { .. }
            | DesignPanelAction::SmartSelectionSpacingEditRequested { .. }
            | DesignPanelAction::SmartSelectionArrangeRequested { .. } => {
                self.node.supports_arrange()
                    && self.node.supports_section(DesignPanelSection::Position)
            }
            DesignPanelAction::TransformRequested { .. } => {
                self.node.supports_transforms()
                    && self.node.supports_section(DesignPanelSection::Position)
            }
            DesignPanelAction::GridDimensionsEditRequested { .. }
            | DesignPanelAction::GridTrackAddRequested { .. }
            | DesignPanelAction::GridTrackDeleteRequested { .. }
            | DesignPanelAction::GridTracksReorderRequested { .. } => {
                self.node.supports_grid_auto_layout()
                    && self.node.supports_section(DesignPanelSection::Layout)
            }
            DesignPanelAction::SectionShareRequested { node_id } => {
                *node_id == self.node.id
                    && self.node.supports_section(DesignPanelSection::Section)
                    && self
                        .node
                        .section
                        .as_ref()
                        .is_some_and(|section| section.capabilities.share)
            }
            DesignPanelAction::SectionResolveChangedStatusRequested { node_id } => {
                *node_id == self.node.id
                    && self.can_edit()
                    && self.node.supports_section(DesignPanelSection::Section)
                    && self.node.section.as_ref().is_some_and(|section| {
                        section.capabilities.resolve_changed_status
                            && section
                                .dev_status
                                .as_ref()
                                .is_some_and(|status| status.changed)
                    })
            }
            DesignPanelAction::TransformModifierAddRequested { node_id, .. } => {
                *node_id == self.node.id
                    && self.can_edit()
                    && self.node.kind == DesignPanelNodeKind::TransformGroup
                    && self.node.supports_section(DesignPanelSection::Transform)
            }
            DesignPanelAction::TransformModifierRemoveRequested {
                node_id,
                modifier_id,
                index,
            } => {
                *node_id == self.node.id
                    && self.can_edit()
                    && self.node.kind == DesignPanelNodeKind::TransformGroup
                    && self.node.supports_section(DesignPanelSection::Transform)
                    && self
                        .node
                        .transform_modifiers
                        .get(*index)
                        .is_some_and(|modifier| modifier.id == *modifier_id)
            }
            DesignPanelAction::TransformModifierChangeRequested {
                node_id,
                modifier_id,
                index,
                change,
                ..
            } => {
                let change_is_valid = match change {
                    DesignTransformModifierChange::Count(count) => *count >= 1,
                    DesignTransformModifierChange::Offset(offset) => offset.is_finite(),
                    DesignTransformModifierChange::Mode(_)
                    | DesignTransformModifierChange::Unit(_) => true,
                };
                *node_id == self.node.id
                    && self.can_edit()
                    && self.node.kind == DesignPanelNodeKind::TransformGroup
                    && self.node.supports_section(DesignPanelSection::Transform)
                    && change_is_valid
                    && self
                        .node
                        .transform_modifiers
                        .get(*index)
                        .is_some_and(|modifier| modifier.id == *modifier_id)
            }
            DesignPanelAction::ApplyTransformModifiersRequested { node_id } => {
                *node_id == self.node.id
                    && self.can_edit()
                    && self.node.kind == DesignPanelNodeKind::TransformGroup
                    && self.node.supports_section(DesignPanelSection::Transform)
                    && !self.node.transform_modifiers.is_empty()
            }
            DesignPanelAction::CollectionItemAddRequested { collection, .. }
            | DesignPanelAction::CollectionItemRemoveRequested { collection, .. }
            | DesignPanelAction::PaintChangeRequested { collection, .. }
            | DesignPanelAction::PaintReorderRequested { collection, .. }
            | DesignPanelAction::PaintSourceReplaceRequested { collection, .. }
            | DesignPanelAction::PaintMediaSourceActionRequested { collection, .. }
            | DesignPanelAction::PaintMediaSourceDropRequested { collection, .. }
            | DesignPanelAction::PaintMediaCropActionRequested { collection, .. }
            | DesignPanelAction::PaintVideoPreviewActionRequested { collection, .. }
            | DesignPanelAction::PaintShaderImportRequested { collection, .. }
            | DesignPanelAction::PaintShaderApplyRequested { collection, .. }
            | DesignPanelAction::PaintShaderPropertyBindRequested { collection, .. }
            | DesignPanelAction::PaintShaderPropertyEditorRequested { collection, .. }
            | DesignPanelAction::PaintShaderPropertyDetachRequested { collection, .. }
            | DesignPanelAction::PaintStyleApplyRequested { collection, .. }
            | DesignPanelAction::PaintStyleImportRequested { collection, .. }
            | DesignPanelAction::PaintStyleCreateRequested { collection, .. }
            | DesignPanelAction::PaintStyleDetachRequested { collection, .. }
            | DesignPanelAction::PaintColorVariableApplyRequested { collection, .. }
            | DesignPanelAction::PaintColorVariableImportRequested { collection, .. }
            | DesignPanelAction::PaintColorVariableDetachRequested { collection, .. }
            | DesignPanelAction::PaintColorVariableCreateRequested { collection, .. }
            | DesignPanelAction::PaintColorStyleSampleRequested { collection, .. }
            | DesignPanelAction::PaintColorStyleApplyRequested { collection, .. }
            | DesignPanelAction::PaintColorStyleCreateRequested { collection, .. } => {
                self.collection_is_supported(*collection)
                    && self
                        .node
                        .paint_collection_edit_mode(*collection)
                        .allows_full_controls()
            }
            DesignPanelAction::PaintEditRequested {
                collection,
                edit,
                phase,
                ..
            } => {
                self.collection_is_supported(*collection)
                    && (*phase == DesignPanelEditPhase::Cancel
                        || self
                            .node
                            .paint_collection_edit_mode(*collection)
                            .allows_property(&edit.property))
            }
            DesignPanelAction::PaintEyedropperRequested { collection, .. } => {
                self.collection_is_supported(*collection)
                    && self
                        .node
                        .paint_collection_edit_mode(*collection)
                        .allows_property(&DesignPaintProperty::Color)
            }
            DesignPanelAction::EffectAddRequested { .. }
            | DesignPanelAction::EffectRemoveRequested { .. }
            | DesignPanelAction::EffectReorderRequested { .. }
            | DesignPanelAction::EffectEditRequested { .. }
            | DesignPanelAction::EffectShaderChooseRequested { .. }
            | DesignPanelAction::EffectShaderPropertyEditorRequested { .. }
            | DesignPanelAction::EffectShaderPropertyVariableDetachRequested { .. }
            | DesignPanelAction::EffectStyleApplyRequested { .. }
            | DesignPanelAction::EffectStyleCreateRequested { .. }
            | DesignPanelAction::EffectStyleDetachRequested { .. }
            | DesignPanelAction::EffectVariableApplyRequested { .. }
            | DesignPanelAction::EffectVariableDetachRequested { .. } => {
                self.collection_is_supported(DesignPanelCollection::Effect)
            }
            DesignPanelAction::LayoutGridStyleApplyRequested { .. }
            | DesignPanelAction::LayoutGridStyleCreateRequested { .. }
            | DesignPanelAction::LayoutGridStyleDetachRequested { .. }
            | DesignPanelAction::LayoutGridStyleImportRequested { .. }
            | DesignPanelAction::LayoutGridPropertyEditRequested { .. }
            | DesignPanelAction::LayoutGridRemoveRequested { .. }
            | DesignPanelAction::LayoutGridVariableApplyRequested { .. }
            | DesignPanelAction::LayoutGridVariableImportRequested { .. }
            | DesignPanelAction::LayoutGridVariableDetachRequested { .. }
            | DesignPanelAction::LayoutGridVariableCreateRequested { .. }
            | DesignPanelAction::LayoutGridCountVariableApplyRequested { .. }
            | DesignPanelAction::LayoutGridCountVariableDetachRequested { .. } => {
                self.collection_is_supported(DesignPanelCollection::LayoutGrid)
            }
            DesignPanelAction::ExportRequested { .. }
            | DesignPanelAction::ExportConfigurationAddRequested { .. }
            | DesignPanelAction::ExportConfigurationRemoveRequested { .. }
            | DesignPanelAction::ExportConfigurationChangeRequested { .. }
            | DesignPanelAction::ExportModeChangeRequested { .. }
            | DesignPanelAction::AnimatedExportChangeRequested { .. }
            | DesignPanelAction::AnimatedExportRequested { .. }
            | DesignPanelAction::ExportAllRequested { .. }
            | DesignPanelAction::ExportPreviewRequested { .. } => {
                self.collection_is_supported(DesignPanelCollection::Export)
            }
            DesignPanelAction::SwapStrokeEndpointsRequested { .. } => {
                self.collection_is_supported(DesignPanelCollection::Stroke)
            }
            DesignPanelAction::SelectionColorEditRequested { .. }
            | DesignPanelAction::SelectionColorPaintEditRequested { .. }
            | DesignPanelAction::SelectionColorOccurrencesSelectRequested { .. }
            | DesignPanelAction::SelectionColorPaintStyleApplyRequested { .. }
            | DesignPanelAction::SelectionColorPaintStyleImportRequested { .. }
            | DesignPanelAction::SelectionColorPaintStyleCreateRequested { .. }
            | DesignPanelAction::SelectionColorPaintStyleDetachRequested { .. }
            | DesignPanelAction::SelectionColorVariableApplyRequested { .. }
            | DesignPanelAction::SelectionColorVariableImportRequested { .. }
            | DesignPanelAction::SelectionColorVariableCreateRequested { .. }
            | DesignPanelAction::SelectionColorVariableDetachRequested { .. } => {
                self.node.supports_section(DesignPanelSection::Selection)
            }
            DesignPanelAction::TextPathFlipOrientationRequested { .. } => {
                self.text_path_flip_is_available()
            }
            _ => true,
        }
    }

    pub(super) fn property_is_editable(&self, property: DesignPanelProperty) -> bool {
        let exact_node_capability_allows_property = self.node_capability_allows_property(property);
        let typography_allows_property = match property {
            DesignPanelProperty::ParagraphIndent => {
                self.node.typography.as_ref().is_some_and(|typography| {
                    typography.horizontal_alignment == DesignTextHorizontalAlignment::Left
                })
            }
            DesignPanelProperty::ListSpacing => self
                .node
                .typography
                .as_ref()
                .is_some_and(|typography| typography.list != DesignTextList::None),
            DesignPanelProperty::TextDecorationStyle
            | DesignPanelProperty::TextDecorationOffset
            | DesignPanelProperty::TextDecorationThickness
            | DesignPanelProperty::TextDecorationColor
            | DesignPanelProperty::TextDecorationSkipInk => {
                self.node.typography.as_ref().is_some_and(|typography| {
                    typography.decoration != DesignTextDecoration::None
                        && typography.decoration_details.is_some()
                })
            }
            DesignPanelProperty::VerticalTextAlignment => self
                .node
                .typography
                .as_ref()
                .is_some_and(|typography| typography.resize == DesignTextResize::Fixed),
            DesignPanelProperty::TextMaxLines => self.text_max_lines_are_available(),
            DesignPanelProperty::TextPathStartSegment
            | DesignPanelProperty::TextPathStartPosition => {
                self.text_path_start_debug_controls_are_available()
            }
            _ => true,
        };
        let vector_allows_property = match property {
            DesignPanelProperty::VectorVertexX | DesignPanelProperty::VectorVertexY => self
                .active_vector_edit()
                .is_some_and(DesignVectorEditViewData::can_edit_coordinates),
            DesignPanelProperty::VectorVertexCornerRadius => self
                .active_vector_edit()
                .is_some_and(DesignVectorEditViewData::can_edit_corner_radius),
            DesignPanelProperty::VectorHandleMirroring => self
                .active_vector_edit()
                .is_some_and(DesignVectorEditViewData::can_edit_handle_mirroring),
            _ => true,
        };
        let smart_selection_allows_property = Self::smart_selection_axis(property)
            .is_none_or(|axis| self.smart_selection_spacing_is_editable(axis));
        let component_allows_property = match property {
            DesignPanelProperty::ComponentProperty(index) => {
                let Some(role) = self.node.component_role() else {
                    return false;
                };
                role.can_edit_property_value()
                    && self
                        .node
                        .component_properties
                        .get(index)
                        .is_some_and(|property| {
                            !matches!(
                                property.definition,
                                DesignComponentPropertyDefinition::Slot { .. }
                            ) && property.active_variable_binding(role).is_none()
                        })
            }
            DesignPanelProperty::SlotStretchChildOnInsert(index)
            | DesignPanelProperty::SlotDisplayEmpty(index)
            | DesignPanelProperty::SlotMinimumInstances(index)
            | DesignPanelProperty::SlotMaximumInstances(index)
            | DesignPanelProperty::SlotPreferredValuesOnly(index) => self
                .node
                .component_properties
                .get(index)
                .is_some_and(|property| {
                    property.slot_settings().is_some()
                        && (self
                            .node
                            .component_role()
                            .is_some_and(DesignComponentRole::can_configure_slot)
                            || self
                                .component_authoring_view_data()
                                .and_then(|authoring| authoring.definition(property.id.as_ref()))
                                .is_some_and(|definition| {
                                    definition.capabilities.edit_slot_settings
                                }))
                }),
            _ => true,
        };
        let permission_allows_property = if matches!(
            property,
            DesignPanelProperty::ExportSizing(_)
                | DesignPanelProperty::ExportScale(_)
                | DesignPanelProperty::ExportSuffix(_)
                | DesignPanelProperty::ExportFormat(_)
        ) {
            self.can_export()
        } else {
            self.can_edit()
        };
        let export_sizing_is_supported = match property {
            DesignPanelProperty::ExportSizing(index) | DesignPanelProperty::ExportScale(index) => {
                self.export_configuration(index)
                    .is_some_and(|configuration| configuration.format().supports_custom_sizing())
            }
            _ => true,
        };
        let selection_color_allows_property = match property {
            DesignPanelProperty::SelectionColor(index) => {
                self.inspection_context.selection().kind() == DesignPanelSelectionKind::Multiple
                    && self
                        .node
                        .resolved_selection_colors()
                        .get(index)
                        .is_some_and(|color| !color.read_only && color.binding.is_none())
            }
            _ => true,
        };
        let grid_auto_tracks_allows_property =
            !matches!(property, DesignPanelProperty::GridRowCount)
                || self
                    .node
                    .layout
                    .as_ref()
                    .is_some_and(|layout| layout.grid_auto_tracks == DesignGridAutoTracks::None);
        let complete_grid_dimensions_allow_property = !matches!(
            property,
            DesignPanelProperty::GridColumnCount | DesignPanelProperty::GridRowCount
        ) || self
            .node
            .layout
            .as_ref()
            .and_then(DesignLayout::grid_dimensions)
            .is_some();
        let grid_track_sizing_allows_property = match property {
            DesignPanelProperty::GridColumnTrackValue(index) => self
                .node
                .layout
                .as_ref()
                .and_then(|layout| layout.grid_columns.get(index))
                .is_some_and(|track| track.sizing != DesignGridTrackSizing::Hug),
            DesignPanelProperty::GridRowTrackValue(index) => self
                .node
                .layout
                .as_ref()
                .and_then(|layout| layout.grid_rows.get(index))
                .is_some_and(|track| track.sizing != DesignGridTrackSizing::Hug),
            _ => true,
        };
        let node_data_allows_property = match property {
            DesignPanelProperty::Visible => self.node.supports_visibility(),
            DesignPanelProperty::Opacity | DesignPanelProperty::BlendMode => {
                self.node.supports_layer_appearance()
            }
            DesignPanelProperty::CornerRadius => self.node.corner_capabilities.uniform_radius,
            DesignPanelProperty::CornerRadiusTopLeft
            | DesignPanelProperty::CornerRadiusTopRight
            | DesignPanelProperty::CornerRadiusBottomRight
            | DesignPanelProperty::CornerRadiusBottomLeft
            | DesignPanelProperty::IndependentCorners => {
                self.node.corner_capabilities.independent_radii
            }
            DesignPanelProperty::CornerSmoothing => self.node.corner_capabilities.smoothing,
            DesignPanelProperty::FillShowsInExports => {
                !self.node.fills.is_empty() && self.node.fill_shows_in_exports.is_some()
            }
            DesignPanelProperty::TableRows | DesignPanelProperty::TableColumns => false,
            DesignPanelProperty::SectionContentsHidden => self.node.section.is_some(),
            DesignPanelProperty::SectionDevStatus => self
                .node
                .section
                .as_ref()
                .is_some_and(|section| section.capabilities.set_dev_status),
            DesignPanelProperty::TransformRepeatType(index)
            | DesignPanelProperty::TransformRepeatCount(index)
            | DesignPanelProperty::TransformRepeatUnit(index)
            | DesignPanelProperty::TransformRepeatOffset(index) => {
                self.node.transform_modifiers.get(index).is_some()
            }
            DesignPanelProperty::TransformRepeatAxis(index) => self
                .node
                .transform_modifiers
                .get(index)
                .is_some_and(|modifier| matches!(modifier.mode, DesignRepeatMode::Linear(_))),
            _ => true,
        };
        let layout_grid_binding_allows_property = match property {
            DesignPanelProperty::LayoutGridKind(index)
            | DesignPanelProperty::LayoutGridVisible(index)
            | DesignPanelProperty::LayoutGridAlignment(index)
            | DesignPanelProperty::LayoutGridCount(index)
            | DesignPanelProperty::LayoutGridSize(index)
            | DesignPanelProperty::LayoutGridOffset(index)
            | DesignPanelProperty::LayoutGridGutter(index)
            | DesignPanelProperty::LayoutGridMargin(index)
            | DesignPanelProperty::LayoutGridColor(index)
            | DesignPanelProperty::LayoutGridOpacity(index) => {
                let Some(grid) = self.node.layout_grids.get(index) else {
                    return false;
                };
                if self.node.layout_grid_style_binding.is_some() {
                    false
                } else {
                    let applicable = match (&grid.settings, property) {
                        (
                            DesignLayoutGridSettings::Uniform(_),
                            DesignPanelProperty::LayoutGridKind(_)
                            | DesignPanelProperty::LayoutGridVisible(_)
                            | DesignPanelProperty::LayoutGridSize(_)
                            | DesignPanelProperty::LayoutGridColor(_)
                            | DesignPanelProperty::LayoutGridOpacity(_),
                        ) => true,
                        (
                            DesignLayoutGridSettings::Columns(_)
                            | DesignLayoutGridSettings::Rows(_),
                            DesignPanelProperty::LayoutGridKind(_)
                            | DesignPanelProperty::LayoutGridVisible(_)
                            | DesignPanelProperty::LayoutGridAlignment(_)
                            | DesignPanelProperty::LayoutGridCount(_)
                            | DesignPanelProperty::LayoutGridGutter(_)
                            | DesignPanelProperty::LayoutGridColor(_)
                            | DesignPanelProperty::LayoutGridOpacity(_),
                        ) => true,
                        (
                            DesignLayoutGridSettings::Columns(settings),
                            DesignPanelProperty::LayoutGridSize(_),
                        ) => !settings.alignment.is_stretch(),
                        (
                            DesignLayoutGridSettings::Rows(settings),
                            DesignPanelProperty::LayoutGridSize(_),
                        ) => !settings.alignment.is_stretch(),
                        (
                            DesignLayoutGridSettings::Columns(settings),
                            DesignPanelProperty::LayoutGridOffset(_),
                        ) => settings.alignment.supports_offset(),
                        (
                            DesignLayoutGridSettings::Rows(settings),
                            DesignPanelProperty::LayoutGridOffset(_),
                        ) => settings.alignment.supports_offset(),
                        (
                            DesignLayoutGridSettings::Columns(settings),
                            DesignPanelProperty::LayoutGridMargin(_),
                        ) => settings.alignment.is_stretch(),
                        (
                            DesignLayoutGridSettings::Rows(settings),
                            DesignPanelProperty::LayoutGridMargin(_),
                        ) => settings.alignment.is_stretch(),
                        _ => false,
                    };
                    applicable
                        && grid
                            .variable_target(index, property)
                            .is_none_or(|(target, _)| grid.variable_binding(target.field).is_none())
                }
            }
            _ => true,
        };
        let effect_allows_property = match property {
            DesignPanelProperty::EffectShadowSpread(index)
            | DesignPanelProperty::EffectSpread(index) => {
                self.node.effect_style_binding.is_none()
                    && self.node.effect_capabilities.shadow_spread
                    && self.node.effects.get(index).is_some_and(|effect| {
                        matches!(
                            effect.settings,
                            DesignEffectSettings::DropShadow(_)
                                | DesignEffectSettings::InnerShadow(_)
                        )
                    })
            }
            DesignPanelProperty::EffectDropShadowShowBehindNode(index) => {
                self.node.effect_style_binding.is_none()
                    && self
                        .node
                        .effect_capabilities
                        .show_shadow_behind_transparent_areas
                    && self.node.effects.get(index).is_some_and(|effect| {
                        matches!(effect.settings, DesignEffectSettings::DropShadow(_))
                    })
            }
            DesignPanelProperty::EffectShaderProperty(index, property_index) => {
                self.node.effect_style_binding.is_none()
                    && self
                        .node
                        .effects
                        .get(index)
                        .and_then(|effect| {
                            let DesignEffectSettings::Shader(shader) = &effect.settings else {
                                return None;
                            };
                            shader.properties.get(property_index)
                        })
                        .is_some_and(|property| {
                            !property.read_only
                                && !matches!(
                                    &property.value,
                                    DesignShaderPropertyValue::VariableAlias { .. }
                                        | DesignShaderPropertyValue::Opaque { .. }
                                )
                        })
            }
            property if property.effect_index().is_some() => {
                self.node.effect_style_binding.is_none()
            }
            _ => true,
        };
        let stroke_allows_property = match property {
            DesignPanelProperty::StrokeDashMode => self
                .node
                .stroke
                .as_ref()
                .is_some_and(|stroke| stroke.complex_stroke.is_basic()),
            DesignPanelProperty::StrokeDashPattern | DesignPanelProperty::StrokeDashCap => {
                self.node.stroke.as_ref().is_some_and(|stroke| {
                    stroke.complex_stroke.is_basic() && !stroke.dashes.is_solid()
                })
            }
            DesignPanelProperty::StrokeVariableWidth => self
                .node
                .stroke
                .as_ref()
                .is_some_and(|stroke| stroke.supports_variable_width()),
            DesignPanelProperty::StrokeVariableWidthPointPosition(index)
            | DesignPanelProperty::StrokeVariableWidthPointWidth(index) => self
                .node
                .stroke
                .as_ref()
                .filter(|stroke| stroke.supports_variable_width())
                .and_then(|stroke| stroke.variable_width.as_ref())
                .and_then(DesignVariableWidthStroke::points)
                .is_some_and(|points| points.get(index).is_some()),
            DesignPanelProperty::StrokeType => self.node.stroke.as_ref().is_some_and(|stroke| {
                stroke.capabilities.complex_stroke && !stroke.complex_stroke.is_opaque()
            }),
            DesignPanelProperty::StrokeStretchBrush | DesignPanelProperty::StrokeBrushDirection => {
                self.node.stroke.as_ref().is_some_and(|stroke| {
                    matches!(&stroke.complex_stroke, DesignComplexStroke::StretchBrush(_))
                })
            }
            DesignPanelProperty::StrokeScatterBrush
            | DesignPanelProperty::StrokeScatterGap
            | DesignPanelProperty::StrokeScatterWiggle
            | DesignPanelProperty::StrokeScatterSizeJitter
            | DesignPanelProperty::StrokeScatterAngularJitter
            | DesignPanelProperty::StrokeScatterRotation => {
                self.node.stroke.as_ref().is_some_and(|stroke| {
                    matches!(&stroke.complex_stroke, DesignComplexStroke::ScatterBrush(_))
                })
            }
            DesignPanelProperty::StrokeDynamicFrequency
            | DesignPanelProperty::StrokeDynamicWiggle
            | DesignPanelProperty::StrokeDynamicSmoothen => {
                self.node.stroke.as_ref().is_some_and(|stroke| {
                    matches!(&stroke.complex_stroke, DesignComplexStroke::Dynamic(_))
                })
            }
            _ => true,
        };
        let paint_style_allows_property = match property {
            DesignPanelProperty::PaintOpacity { collection, .. }
            | DesignPanelProperty::PaintVisible { collection, .. } => {
                self.paint_style_binding(collection).is_none()
            }
            _ => true,
        };
        typography_allows_property
            && vector_allows_property
            && smart_selection_allows_property
            && self.layout_property_is_applicable(property)
            && component_allows_property
            && permission_allows_property
            && export_sizing_is_supported
            && selection_color_allows_property
            && grid_auto_tracks_allows_property
            && complete_grid_dimensions_allow_property
            && grid_track_sizing_allows_property
            && exact_node_capability_allows_property
            && node_data_allows_property
            && layout_grid_binding_allows_property
            && effect_allows_property
            && stroke_allows_property
            && paint_style_allows_property
            && self
                .property_value_states
                .get(&property)
                .is_none_or(|state| !state.is_read_only() && state.binding().is_none())
    }

    pub(super) fn emit_vector_vertex_selection(
        &mut self,
        selected_vertex_ids: Vec<SharedString>,
        phase: DesignPanelEditPhase,
        cx: &mut Context<Self>,
    ) {
        let Some(view_data) = self.active_vector_edit() else {
            return;
        };
        let unique = selected_vertex_ids.iter().collect::<HashSet<_>>();
        if (!view_data.can_select_vertices() && phase != DesignPanelEditPhase::Cancel)
            || unique.len() != selected_vertex_ids.len()
            || selected_vertex_ids
                .iter()
                .any(|vertex_id| view_data.vertex(vertex_id.as_ref()).is_none())
        {
            return;
        }
        cx.emit_design_panel_action(
            self,
            DesignPanelAction::VectorVertexSelectionEditRequested {
                node_id: self.node.id.clone(),
                selected_vertex_ids,
                phase,
            },
        );
    }

    /// Returns true for every vector-only property, including rejected edits,
    /// so invalid topology or stale host IDs can never fall through to a
    /// generic document property intent.
    pub(super) fn emit_vector_property_edit(
        &mut self,
        property: DesignPanelProperty,
        value: &DesignPanelValue,
        phase: DesignPanelEditPhase,
        cx: &mut Context<Self>,
    ) -> bool {
        if !Self::vector_property_is_contextual(property) {
            return false;
        }
        let Some(view_data) = self.active_vector_edit() else {
            return true;
        };
        let current_selection = view_data.selected_vertex_ids();
        let vertex_ids = self
            .vector_edit_target_ids
            .clone()
            .unwrap_or_else(|| current_selection.clone());
        if !view_data.contains_exact_vertices(&vertex_ids)
            || current_selection.len() != vertex_ids.len()
            || !current_selection.iter().all(|id| vertex_ids.contains(id))
        {
            return true;
        }
        let cancel = phase == DesignPanelEditPhase::Cancel;
        let action = match (property, value) {
            (DesignPanelProperty::VectorVertexX, DesignPanelValue::Number(value))
                if value.is_finite() && (cancel || view_data.can_edit_coordinates()) =>
            {
                DesignPanelAction::VectorVertexPositionEditRequested {
                    node_id: self.node.id.clone(),
                    vertex_ids,
                    axis: DesignVectorCoordinateAxis::X,
                    value: *value,
                    phase,
                }
            }
            (DesignPanelProperty::VectorVertexY, DesignPanelValue::Number(value))
                if value.is_finite() && (cancel || view_data.can_edit_coordinates()) =>
            {
                DesignPanelAction::VectorVertexPositionEditRequested {
                    node_id: self.node.id.clone(),
                    vertex_ids,
                    axis: DesignVectorCoordinateAxis::Y,
                    value: *value,
                    phase,
                }
            }
            (DesignPanelProperty::VectorVertexCornerRadius, DesignPanelValue::Number(radius))
                if radius.is_finite()
                    && *radius >= 0.
                    && (cancel || view_data.can_edit_corner_radius()) =>
            {
                DesignPanelAction::VectorVertexCornerRadiusEditRequested {
                    node_id: self.node.id.clone(),
                    vertex_ids,
                    radius: *radius,
                    phase,
                }
            }
            (
                DesignPanelProperty::VectorHandleMirroring,
                DesignPanelValue::HandleMirroring(mirroring),
            ) if cancel || view_data.can_edit_handle_mirroring() => {
                DesignPanelAction::VectorHandleMirroringEditRequested {
                    node_id: self.node.id.clone(),
                    vertex_ids,
                    mirroring: *mirroring,
                    phase,
                }
            }
            _ => return true,
        };
        cx.emit_design_panel_action(self, action);
        true
    }

    pub(super) fn emit_property_copy(
        &mut self,
        property: DesignPanelProperty,
        displayed_value: SharedString,
        cx: &mut Context<Self>,
    ) {
        if !self.generic_property_is_copyable(property) {
            return;
        }
        let Some(target) = self.current_selection_header_target() else {
            return;
        };
        cx.emit_design_panel_action(
            self,
            DesignPanelAction::PropertyCopyRequested {
                target,
                property,
                displayed_value,
            },
        );
    }

    pub(super) fn emit_property(
        &mut self,
        property: DesignPanelProperty,
        value: DesignPanelValue,
        cx: &mut Context<Self>,
    ) {
        self.cancel_menu_preview(cx);
        if self.emit_smart_selection_spacing_edit(
            property,
            &value,
            DesignPanelEditPhase::Commit,
            cx,
        ) {
            return;
        }
        if !self.property_is_editable(property)
            || !self.layout_property_value_is_applicable(property, &value)
        {
            return;
        }
        if self.emit_grid_dimensions_property_edit(
            property,
            &value,
            DesignPanelEditPhase::Commit,
            cx,
        ) {
            return;
        }
        if self.emit_vector_property_edit(property, &value, DesignPanelEditPhase::Commit, cx) {
            return;
        }
        if self.emit_layout_grid_property_edit(
            property,
            value.clone(),
            DesignPanelEditPhase::Commit,
            cx,
        ) {
            return;
        }
        if let Some((property_id, value)) = self.component_change_for_property(property, &value) {
            cx.emit_design_panel_action(
                self,
                DesignPanelAction::ComponentPropertyChangeRequested {
                    node_id: self.node.id.clone(),
                    property_id,
                    value,
                },
            );
            return;
        }
        if let Some((property_id, expected_settings, change)) =
            self.slot_settings_change_for_property(property, &value)
        {
            cx.emit_design_panel_action(
                self,
                DesignPanelAction::SlotSettingsChangeRequested {
                    node_id: self.node.id.clone(),
                    property_id,
                    expected_settings,
                    change,
                    phase: DesignPanelEditPhase::Commit,
                },
            );
            return;
        }
        if let (DesignPanelProperty::SelectionColor(index), DesignPanelValue::Color(color)) =
            (property, &value)
            && let Some(selection_color) = self.node.resolved_selection_colors().get(index).cloned()
        {
            cx.emit_design_panel_action(
                self,
                DesignPanelAction::SelectionColorEditRequested {
                    target: self.command_target(),
                    selection_color_id: selection_color.id,
                    color: *color,
                    paint_references: selection_color.paint_references,
                    phase: DesignPanelEditPhase::Commit,
                },
            );
            return;
        }
        if let Some((target, edit)) = self.paint_edit_for_property(property, &value) {
            self.emit_paint_edit(target, edit, DesignPanelEditPhase::Commit, cx);
            return;
        }
        if let Some((configuration_id, change)) =
            self.export_change_for_property(property, value.clone())
        {
            self.emit_export_configuration_change(
                configuration_id,
                change,
                DesignPanelEditPhase::Commit,
                cx,
            );
            return;
        }
        if let Some((modifier_id, index, change)) =
            self.transform_modifier_change_for_property(property, &value)
        {
            cx.emit_design_panel_action(
                self,
                DesignPanelAction::TransformModifierChangeRequested {
                    node_id: self.node.id.clone(),
                    modifier_id,
                    index,
                    change,
                    phase: DesignPanelEditPhase::Commit,
                },
            );
            return;
        }
        if self.emit_effect_edit(property, value.clone(), DesignPanelEditPhase::Commit, cx) {
            return;
        }
        if matches!(
            property,
            DesignPanelProperty::TextPathStartSegment | DesignPanelProperty::TextPathStartPosition
        ) {
            let Some(data) = self.text_path_start_for_property(property, &value) else {
                return;
            };
            cx.emit_design_panel_action(
                self,
                DesignPanelAction::TextPathStartChangeRequested {
                    node_id: self.node.id.clone(),
                    data,
                    phase: DesignPanelEditPhase::Commit,
                },
            );
            return;
        }
        if property.is_typography() {
            cx.emit_design_panel_action(
                self,
                DesignPanelAction::TypographyPropertyChangeRequested {
                    node_id: self.node.id.clone(),
                    target: self.typography_target(property),
                    property,
                    value,
                },
            );
            return;
        }
        cx.emit_design_panel_action(
            self,
            DesignPanelAction::PropertyChangeRequested {
                node_id: self.node.id.clone(),
                property,
                value,
            },
        );
    }

    pub(super) fn emit_property_edit(
        &mut self,
        property: DesignPanelProperty,
        value: DesignPanelValue,
        phase: DesignPanelEditPhase,
        cx: &mut Context<Self>,
    ) {
        if self.emit_smart_selection_spacing_edit(property, &value, phase, cx) {
            return;
        }
        if self.emit_vector_property_edit(property, &value, phase, cx) {
            return;
        }
        let cancel = phase == DesignPanelEditPhase::Cancel;
        let editable = self.property_is_editable(property);
        let cancel_inapplicable_counter_spacing =
            cancel && property == DesignPanelProperty::CounterAxisGap && self.can_edit();
        if (!editable && !cancel_inapplicable_counter_spacing)
            || (!cancel && !self.layout_property_value_is_applicable(property, &value))
        {
            return;
        }
        if self.emit_grid_dimensions_property_edit(property, &value, phase, cx) {
            return;
        }
        if self.emit_layout_grid_property_edit(property, value.clone(), phase, cx) {
            return;
        }
        if let Some((property_id, value)) = self.component_change_for_property(property, &value) {
            cx.emit_design_panel_action(
                self,
                DesignPanelAction::ComponentPropertyEditRequested {
                    node_id: self.node.id.clone(),
                    property_id,
                    value,
                    phase,
                },
            );
            return;
        }
        if let Some((property_id, expected_settings, change)) =
            self.slot_settings_change_for_property(property, &value)
        {
            cx.emit_design_panel_action(
                self,
                DesignPanelAction::SlotSettingsChangeRequested {
                    node_id: self.node.id.clone(),
                    property_id,
                    expected_settings,
                    change,
                    phase,
                },
            );
            return;
        }
        if let (DesignPanelProperty::SelectionColor(index), DesignPanelValue::Color(color)) =
            (property, &value)
            && let Some(selection_color) = self.node.resolved_selection_colors().get(index).cloned()
        {
            cx.emit_design_panel_action(
                self,
                DesignPanelAction::SelectionColorEditRequested {
                    target: self.command_target(),
                    selection_color_id: selection_color.id,
                    color: *color,
                    paint_references: selection_color.paint_references,
                    phase,
                },
            );
            return;
        }
        if let Some((target, edit)) = self.paint_edit_for_property(property, &value) {
            self.emit_paint_edit(target, edit, phase, cx);
            return;
        }
        if let Some((configuration_id, change)) =
            self.export_change_for_property(property, value.clone())
        {
            self.emit_export_configuration_change(configuration_id, change, phase, cx);
            return;
        }
        if let Some((modifier_id, index, change)) =
            self.transform_modifier_change_for_property(property, &value)
        {
            cx.emit_design_panel_action(
                self,
                DesignPanelAction::TransformModifierChangeRequested {
                    node_id: self.node.id.clone(),
                    modifier_id,
                    index,
                    change,
                    phase,
                },
            );
            return;
        }
        if self.emit_effect_edit(property, value.clone(), phase, cx) {
            return;
        }
        if matches!(
            property,
            DesignPanelProperty::TextPathStartSegment | DesignPanelProperty::TextPathStartPosition
        ) {
            let Some(data) = self.text_path_start_for_property(property, &value) else {
                return;
            };
            cx.emit_design_panel_action(
                self,
                DesignPanelAction::TextPathStartChangeRequested {
                    node_id: self.node.id.clone(),
                    data,
                    phase,
                },
            );
            return;
        }
        if property.is_typography() {
            cx.emit_design_panel_action(
                self,
                DesignPanelAction::TypographyPropertyEditRequested {
                    node_id: self.node.id.clone(),
                    target: self.typography_target(property),
                    property,
                    value,
                    phase,
                },
            );
            return;
        }
        cx.emit_design_panel_action(
            self,
            DesignPanelAction::PropertyEditRequested {
                node_id: self.node.id.clone(),
                property,
                value,
                phase,
            },
        );
    }

    pub(super) fn emit_grid_dimensions_property_edit(
        &mut self,
        property: DesignPanelProperty,
        value: &DesignPanelValue,
        phase: DesignPanelEditPhase,
        cx: &mut Context<Self>,
    ) -> bool {
        let axis = match property {
            DesignPanelProperty::GridColumnCount => DesignGridTrackAxis::Column,
            DesignPanelProperty::GridRowCount => DesignGridTrackAxis::Row,
            _ => return false,
        };
        let Some(current) = self
            .node
            .layout
            .as_ref()
            .and_then(DesignLayout::grid_dimensions)
        else {
            return true;
        };
        let Some(count) = (match value {
            DesignPanelValue::Integer(count) => usize::try_from(*count).ok(),
            _ => None,
        })
        .filter(|count| *count > 0) else {
            return true;
        };

        let active = self.active_grid_dimensions_edit.clone();
        let dimensions = if phase == DesignPanelEditPhase::Cancel {
            let Some(edit) = active.as_ref().filter(|edit| edit.node_id == self.node.id) else {
                return true;
            };
            edit.original
        } else {
            match axis {
                DesignGridTrackAxis::Column => DesignGridDimensions {
                    columns: count,
                    rows: current.rows,
                },
                DesignGridTrackAxis::Row => DesignGridDimensions {
                    columns: current.columns,
                    rows: count,
                },
            }
        };
        let action = DesignPanelAction::GridDimensionsEditRequested {
            node_id: self.node.id.clone(),
            dimensions,
            phase,
        };
        if !self.grid_dimensions_action_is_applicable(&action) {
            return true;
        }

        match phase {
            DesignPanelEditPhase::Begin if active.is_none() => {
                self.active_grid_dimensions_edit = Some(GridDimensionsEdit {
                    node_id: self.node.id.clone(),
                    original: current,
                });
            }
            DesignPanelEditPhase::Begin => return true,
            DesignPanelEditPhase::Preview
                if active
                    .as_ref()
                    .is_none_or(|edit| edit.node_id != self.node.id) =>
            {
                return true;
            }
            DesignPanelEditPhase::Commit | DesignPanelEditPhase::Cancel => {
                self.active_grid_dimensions_edit = None;
            }
            DesignPanelEditPhase::Preview => {}
        }
        cx.emit_design_panel_action(self, action);
        true
    }

    pub(super) fn emit_add(&mut self, collection: DesignPanelCollection, cx: &mut Context<Self>) {
        if !self.collection_is_supported(collection) {
            return;
        }
        if collection == DesignPanelCollection::Export {
            if self.can_export() {
                cx.emit_design_panel_action(
                    self,
                    DesignPanelAction::ExportConfigurationAddRequested {
                        target: self.export_target(),
                    },
                );
            }
            return;
        }
        if !self
            .node
            .paint_collection_edit_mode(collection)
            .allows_full_controls()
        {
            return;
        }
        if !self.can_edit() {
            return;
        }
        if matches!(
            collection,
            DesignPanelCollection::Fill | DesignPanelCollection::Stroke
        ) && self.paint_style_binding(collection).is_some()
        {
            return;
        }
        if collection == DesignPanelCollection::LayoutGrid
            && self.node.layout_grid_style_binding.is_some()
        {
            return;
        }
        if collection == DesignPanelCollection::Effect {
            if self.node.effect_style_binding.is_some() {
                return;
            }
            if let Some(kind) = self.node.first_addable_effect_kind() {
                cx.emit_design_panel_action(
                    self,
                    DesignPanelAction::EffectAddRequested {
                        node_id: self.node.id.clone(),
                        kind,
                    },
                );
            }
            return;
        }
        cx.emit_design_panel_action(
            self,
            DesignPanelAction::CollectionItemAddRequested {
                node_id: self.node.id.clone(),
                collection,
                target: self.paint_target(collection),
            },
        );
    }

    pub(super) fn emit_remove(
        &mut self,
        collection: DesignPanelCollection,
        index: usize,
        cx: &mut Context<Self>,
    ) {
        if !self.collection_is_supported(collection) {
            return;
        }
        if collection == DesignPanelCollection::Export {
            if self.can_export()
                && let Some(configuration) = self.export_configuration(index)
            {
                cx.emit_design_panel_action(
                    self,
                    DesignPanelAction::ExportConfigurationRemoveRequested {
                        target: self.export_target(),
                        configuration_id: configuration.id,
                    },
                );
            }
            return;
        }
        if !self
            .node
            .paint_collection_edit_mode(collection)
            .allows_full_controls()
        {
            return;
        }
        if !self.can_edit() {
            return;
        }
        if matches!(
            collection,
            DesignPanelCollection::Fill | DesignPanelCollection::Stroke
        ) && self.paint_style_binding(collection).is_some()
        {
            return;
        }
        if collection == DesignPanelCollection::LayoutGrid
            && self.node.layout_grid_style_binding.is_some()
        {
            return;
        }
        if collection == DesignPanelCollection::LayoutGrid {
            if let Some(guide) = self.node.layout_grids.get(index) {
                cx.emit_design_panel_action(
                    self,
                    DesignPanelAction::LayoutGridRemoveRequested {
                        node_id: self.node.id.clone(),
                        guide_id: guide.id.clone(),
                        index,
                    },
                );
            }
            return;
        }
        if collection == DesignPanelCollection::Effect {
            if self.node.effect_style_binding.is_some() {
                return;
            }
            if let Some(effect) = self.node.effects.get(index) {
                cx.emit_design_panel_action(
                    self,
                    DesignPanelAction::EffectRemoveRequested {
                        node_id: self.node.id.clone(),
                        effect_id: effect.id.clone(),
                        index,
                    },
                );
            }
            return;
        }
        cx.emit_design_panel_action(
            self,
            DesignPanelAction::CollectionItemRemoveRequested {
                node_id: self.node.id.clone(),
                collection,
                target: self.paint_target(collection),
                index,
            },
        );
    }

    /// Resolves an in-progress indexed edit against a same-node host echo.
    ///
    /// Figma may reorder host-owned arrays while an editor is open. Stable
    /// identities keep the transaction attached to the same effect, paint,
    /// component property, guide, modifier, shader definition, or aggregate
    /// selection color. `None` means that exact target disappeared.
    pub(super) fn property_editor_property_for_node_echo(
        &self,
        editor: &PropertyEditor,
        next_node: &DesignPanelNode,
    ) -> Option<DesignPanelProperty> {
        let property = editor.property;
        if let Some(target) = editor.layout_grid_target.as_ref() {
            let index = if target.guide_id.is_empty() {
                next_node
                    .layout_grids
                    .get(target.index)
                    .map(|_| target.index)
            } else {
                next_node
                    .layout_grids
                    .iter()
                    .position(|guide| guide.id == target.guide_id)
            }?;
            return Some(property.with_layout_grid_index(index));
        }

        if let DesignPanelProperty::GridColumnTrackValue(index)
        | DesignPanelProperty::GridRowTrackValue(index) = property
        {
            let columns = matches!(property, DesignPanelProperty::GridColumnTrackValue(_));
            let previous_layout = self.node.layout.as_ref()?;
            let next_layout = next_node.layout.as_ref()?;
            let previous = if columns {
                previous_layout.grid_columns.as_slice()
            } else {
                previous_layout.grid_rows.as_slice()
            };
            let next = if columns {
                next_layout.grid_columns.as_slice()
            } else {
                next_layout.grid_rows.as_slice()
            };
            if previous.len() != next.len()
                || previous.get(index)?.sizing != next.get(index)?.sizing
                || previous.iter().zip(next).enumerate().any(
                    |(candidate_index, (previous, next))| {
                        candidate_index != index && previous != next
                    },
                )
            {
                return None;
            }
            return Some(property);
        }

        if let DesignPanelProperty::StrokeVariableWidthPointPosition(index)
        | DesignPanelProperty::StrokeVariableWidthPointWidth(index) = property
        {
            let previous = self
                .node
                .stroke
                .as_ref()?
                .variable_width
                .as_ref()?
                .points()?;
            let next = next_node
                .stroke
                .as_ref()?
                .variable_width
                .as_ref()?
                .points()?;
            let previous_point = previous.get(index)?;
            let next_point = next.get(index)?;
            let counterpart_changed = match property {
                DesignPanelProperty::StrokeVariableWidthPointPosition(_) => {
                    previous_point.width != next_point.width
                }
                DesignPanelProperty::StrokeVariableWidthPointWidth(_) => {
                    previous_point.position != next_point.position
                }
                _ => unreachable!(),
            };
            if previous.len() != next.len()
                || counterpart_changed
                || previous.iter().zip(next).enumerate().any(
                    |(candidate_index, (previous, next))| {
                        candidate_index != index && previous != next
                    },
                )
            {
                return None;
            }
            return Some(property);
        }

        if let Some(index) = property.effect_index() {
            let effect = self.node.effects.get(index)?;
            let next_index = if effect.id.is_empty() {
                next_node.effects.get(index).map(|_| index)
            } else {
                next_node
                    .effects
                    .iter()
                    .position(|candidate| candidate.id == effect.id)
            }?;
            if let DesignPanelProperty::EffectShaderProperty(_, property_index) = property {
                let DesignEffectSettings::Shader(shader) = &effect.settings else {
                    return None;
                };
                let definition_id = shader.properties.get(property_index)?.definition_id.clone();
                let DesignEffectSettings::Shader(next_shader) =
                    &next_node.effects.get(next_index)?.settings
                else {
                    return None;
                };
                let next_property_index = next_shader
                    .properties
                    .iter()
                    .position(|candidate| candidate.definition_id == definition_id)?;
                if next_shader.properties[next_property_index].kind
                    != shader.properties[property_index].kind
                {
                    return None;
                }
                return Some(DesignPanelProperty::EffectShaderProperty(
                    next_index,
                    next_property_index,
                ));
            }
            return Some(property.with_effect_index(next_index));
        }

        let component_index = match property {
            DesignPanelProperty::ComponentProperty(index)
            | DesignPanelProperty::SlotStretchChildOnInsert(index)
            | DesignPanelProperty::SlotDisplayEmpty(index)
            | DesignPanelProperty::SlotMinimumInstances(index)
            | DesignPanelProperty::SlotMaximumInstances(index)
            | DesignPanelProperty::SlotPreferredValuesOnly(index) => Some(index),
            _ => None,
        };
        if let Some(index) = component_index {
            let component_property = self.node.component_properties.get(index)?;
            let next_index = if component_property.id.is_empty() {
                next_node.component_properties.get(index).map(|_| index)
            } else {
                next_node
                    .component_properties
                    .iter()
                    .position(|candidate| candidate.id == component_property.id)
            }?;
            let next_property = next_node.component_properties.get(next_index)?;
            let definitions_are_compatible = match (
                &component_property.definition,
                &next_property.definition,
                property,
            ) {
                (
                    DesignComponentPropertyDefinition::Text {
                        multiline: previous,
                        ..
                    },
                    DesignComponentPropertyDefinition::Text {
                        multiline: next, ..
                    },
                    DesignPanelProperty::ComponentProperty(_),
                ) => previous == next,
                (previous, next, DesignPanelProperty::ComponentProperty(_)) => {
                    previous.kind() == next.kind()
                }
                (
                    DesignComponentPropertyDefinition::Slot { .. },
                    DesignComponentPropertyDefinition::Slot { .. },
                    DesignPanelProperty::SlotStretchChildOnInsert(_)
                    | DesignPanelProperty::SlotDisplayEmpty(_)
                    | DesignPanelProperty::SlotMinimumInstances(_)
                    | DesignPanelProperty::SlotMaximumInstances(_)
                    | DesignPanelProperty::SlotPreferredValuesOnly(_),
                ) => true,
                _ => false,
            };
            if !definitions_are_compatible {
                return None;
            }
            return Some(Self::component_property_with_index(property, next_index));
        }

        let transform_index = match property {
            DesignPanelProperty::TransformRepeatType(index)
            | DesignPanelProperty::TransformRepeatAxis(index)
            | DesignPanelProperty::TransformRepeatCount(index)
            | DesignPanelProperty::TransformRepeatUnit(index)
            | DesignPanelProperty::TransformRepeatOffset(index) => Some(index),
            _ => None,
        };
        if let Some(index) = transform_index {
            let modifier = self.node.transform_modifiers.get(index)?;
            let next_index = if modifier.id.is_empty() {
                next_node.transform_modifiers.get(index).map(|_| index)
            } else {
                next_node
                    .transform_modifiers
                    .iter()
                    .position(|candidate| candidate.id == modifier.id)
            }?;
            let next_modifier = next_node.transform_modifiers.get(next_index)?;
            if matches!(property, DesignPanelProperty::TransformRepeatOffset(_))
                && modifier.unit != next_modifier.unit
            {
                return None;
            }
            return Some(Self::transform_modifier_property_with_index(
                property, next_index,
            ));
        }

        match property {
            DesignPanelProperty::PaintOpacity { collection, index } => {
                let paint = match collection {
                    DesignPanelCollection::Fill => self.node.fills.get(index),
                    DesignPanelCollection::Stroke => self
                        .node
                        .stroke
                        .as_ref()
                        .and_then(|stroke| stroke.paints.get(index)),
                    DesignPanelCollection::Effect
                    | DesignPanelCollection::LayoutGrid
                    | DesignPanelCollection::Export => None,
                }?;
                let next_index =
                    Self::paint_index_in_node(next_node, collection, paint.id.as_ref(), index)?;
                Some(DesignPanelProperty::PaintOpacity {
                    collection,
                    index: next_index,
                })
            }
            DesignPanelProperty::PaintVisible { collection, index } => {
                let paint = match collection {
                    DesignPanelCollection::Fill => self.node.fills.get(index),
                    DesignPanelCollection::Stroke => self
                        .node
                        .stroke
                        .as_ref()
                        .and_then(|stroke| stroke.paints.get(index)),
                    DesignPanelCollection::Effect
                    | DesignPanelCollection::LayoutGrid
                    | DesignPanelCollection::Export => None,
                }?;
                let next_index =
                    Self::paint_index_in_node(next_node, collection, paint.id.as_ref(), index)?;
                Some(DesignPanelProperty::PaintVisible {
                    collection,
                    index: next_index,
                })
            }
            DesignPanelProperty::SelectionColor(index) => {
                let colors = self.node.resolved_selection_colors();
                let selection_color = colors.get(index)?;
                let next_colors = next_node.resolved_selection_colors();
                let next_index = if selection_color.id.is_empty() {
                    next_colors.get(index).map(|_| index)
                } else {
                    next_colors
                        .iter()
                        .position(|candidate| candidate.id == selection_color.id)
                }?;
                Some(DesignPanelProperty::SelectionColor(next_index))
            }
            _ => Some(property),
        }
    }

    /// Terminates only interactions whose exact target vanished or became
    /// non-editable in a same-ID host echo. Surviving indexed editors are
    /// returned with their compatibility indices rebased to stable IDs.
    pub(super) fn cancel_interactions_invalidated_by_host_echo(
        &mut self,
        next_node: &DesignPanelNode,
        next_context: &DesignPanelInspectionContext,
        cx: &mut Context<Self>,
    ) -> Option<DesignPanelProperty> {
        let active_property = self.property_editor.clone();
        let resolved_property = active_property
            .as_ref()
            .and_then(|editor| self.property_editor_property_for_node_echo(editor, next_node));
        let active_semantic_target = active_property
            .as_ref()
            .and_then(|editor| self.property_variable_target(editor.property));
        let active_crop_target = self.active_picker.clone().filter(|target| {
            let Some(index) = self.paint_target_index(target) else {
                return false;
            };
            self.media_paint_view_data
                .paint(target.collection, &target.paint_id, index)
                .is_some_and(|view| {
                    view.capabilities.can_edit_properties
                        && view.crop_tool.active
                        && self.picker_paint(target).is_some_and(|paint| {
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
                        })
                })
        });

        let previous_node = std::mem::replace(&mut self.node, next_node.clone());
        let previous_context =
            std::mem::replace(&mut self.inspection_context, next_context.clone());

        let picker_survives = self.active_picker.as_ref().is_none_or(|target| {
            self.collection_is_supported(target.collection)
                && self.paint_style_binding(target.collection).is_none()
                && self
                    .picker_paint(target)
                    .is_some_and(|paint| self.can_edit() && !paint.read_only)
        }) && self.active_paint_edit_is_editable();
        let crop_interaction_survives = active_crop_target.as_ref().is_none_or(|target| {
            let Some(index) = self.paint_target_index(target) else {
                return false;
            };
            self.media_paint_view_data
                .paint(target.collection, &target.paint_id, index)
                .is_some_and(|view| {
                    self.can_edit()
                        && view.capabilities.can_edit_properties
                        && view.crop_tool.active
                        && self.picker_paint(target).is_some_and(|paint| {
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
                        })
                })
        });
        let auxiliary_picker_survives = self
            .auxiliary_color_picker
            .as_ref()
            .is_none_or(|target| self.auxiliary_color_paint(target).is_some());
        let auxiliary_edit_survives = self.active_paint_edit.as_ref().is_none_or(|active| {
            self.auxiliary_color_picker
                .as_ref()
                .filter(|target| target.matches_picker_target(&active.target))
                .is_none_or(|target| self.auxiliary_paint_editable(target, &active.edit))
        });
        let semantic_target_survives =
            active_semantic_target
                .as_ref()
                .is_none_or(|previous_target| {
                    resolved_property
                        .and_then(|property| self.property_variable_target(property))
                        .is_some_and(|next_target| {
                            next_target.fields == previous_target.fields
                                && next_target.resolved_type == previous_target.resolved_type
                                && next_target.typography_target
                                    == previous_target.typography_target
                        })
                });
        let vector_targets_survive =
            self.vector_edit_target_ids
                .as_ref()
                .is_none_or(|target_ids| {
                    let Some(view_data) = self.active_vector_edit() else {
                        return false;
                    };
                    let current_selection = view_data.selected_vertex_ids();
                    view_data.contains_exact_vertices(target_ids)
                        && current_selection.len() == target_ids.len()
                        && current_selection.iter().all(|id| target_ids.contains(id))
                });
        let variable_font_axis_editor_survives = self
            .variable_font_axis_editor
            .as_ref()
            .is_none_or(|editor| self.variable_font_axis_editor_survives(editor));
        let variable_font_axis_scrub_survives =
            self.variable_font_axis_scrub.as_ref().is_none_or(|scrub| {
                self.variable_font_axis(scrub.tag.as_ref())
                    .is_some_and(|axis| {
                        self.variable_font_axis_is_editable(scrub.tag.as_ref())
                            && (scrub.active || axis.value == scrub.original)
                    })
            });
        let property_survives = match (&active_property, resolved_property) {
            (None, _) => true,
            (Some(_), Some(property)) => {
                self.current_property_value(property).is_some()
                    && self.property_is_editable(property)
                    && semantic_target_survives
                    && vector_targets_survive
            }
            (Some(_), None) => false,
        };
        let multiline_survives = self
            .component_multiline_editor
            .as_ref()
            .is_none_or(|editor| {
                self.component_property_index(editor.property_id.as_ref())
                    .is_some_and(|index| {
                        self.node
                            .component_properties
                            .get(index)
                            .is_some_and(|property| {
                                matches!(
                                    property.definition,
                                    DesignComponentPropertyDefinition::Text {
                                        multiline: true,
                                        ..
                                    }
                                )
                            })
                            && self
                                .property_is_editable(DesignPanelProperty::ComponentProperty(index))
                    })
            });
        let swap_preview_survives =
            self.component_swap_hovered
                .as_ref()
                .is_none_or(|(property_id, _)| {
                    self.component_property_index(property_id.as_ref())
                        .is_some_and(|index| {
                            self.node
                                .component_properties
                                .get(index)
                                .is_some_and(|property| {
                                    matches!(
                                        property.definition,
                                        DesignComponentPropertyDefinition::InstanceSwap { .. }
                                    )
                                })
                                && self.property_is_editable(
                                    DesignPanelProperty::ComponentProperty(index),
                                )
                        })
                });

        self.node = previous_node;
        self.inspection_context = previous_context;

        if !crop_interaction_survives && let Some(target) = active_crop_target.as_ref() {
            self.emit_crop_cancel_if_active(target, cx);
        }
        if !picker_survives || !auxiliary_picker_survives || !auxiliary_edit_survives {
            self.prepare_paint_picker_for_dismissal(cx);
            if !picker_survives
                && crop_interaction_survives
                && let Some(target) = self.active_picker.clone()
            {
                self.emit_crop_cancel_if_active(&target, cx);
            }
            self.cancel_active_paint_edit(cx);
            if !picker_survives {
                self.active_picker = None;
            }
            if !auxiliary_picker_survives {
                self.auxiliary_color_picker = None;
            }
        }
        if !property_survives {
            self.cancel_property_editor_transaction(cx);
        }
        if !variable_font_axis_editor_survives {
            self.cancel_variable_font_axis_transaction(cx);
        } else if !variable_font_axis_scrub_survives {
            self.variable_font_axis_scrub = None;
        }
        if !multiline_survives {
            self.cancel_component_multiline_transaction(cx);
        }
        if !swap_preview_survives {
            self.cancel_component_swap_preview(cx);
        }

        property_survives.then_some(resolved_property).flatten()
    }

    pub(super) fn cancel_property_editor_transaction(&mut self, cx: &mut Context<Self>) {
        let focus_property = self
            .property_editor
            .as_ref()
            .map(|editor| editor.property)
            .or_else(|| {
                self.numeric_property_scrub
                    .as_ref()
                    .map(|scrub| scrub.property)
            });
        if let Some(editor) = self.property_editor.clone() {
            self.emit_property_edit(
                editor.property,
                editor.original,
                DesignPanelEditPhase::Cancel,
                cx,
            );
        }
        self.property_editor = None;
        self.numeric_property_scrub = None;
        self.draw_appearance_slider_property = None;
        self.vector_edit_target_ids = None;
        self.property_editor_invalid = false;
        self.suppress_property_input_change = false;
        if let Some(property) = focus_property {
            self.clear_editor_focus_return_for_property(property);
        }
    }

    pub(super) fn close_editor_only_overlays(&mut self) {
        self.auxiliary_color_picker = None;
        self.active_picker = None;
        self.active_effect_settings = None;
        self.paint_style_browser_open = None;
        self.selection_color_resource_browser = None;
        self.effect_style_browser_open = false;
        self.property_variable_picker = None;
        self.component_property_variable_picker = None;
        self.component_swap_browser = None;
        self.component_swap_hovered = None;
        self.component_multiline_editor = None;
        self.layout_grid_style_browser_open = false;
        self.layout_grid_count_variable_target = None;
        self.frame_preset_browser_open = false;
        self.page_background_picker_open = false;
        self.variable_mode_browser_open = false;
        self.typography_style_picker_open = false;
        self.font_browser_open = false;
        self.type_settings_open = false;
        self.type_settings_tab = TypographySettingsTab::Basics;
        self.grid_dimensions_picker = None;
        self.dimension_limit_fields_disclosed.clear();
        self.dimension_menu_open = None;
        self.appearance_blend_mode_open = false;
        self.appearance_corner_details_open = false;
        self.preview_option_menu_open = None;
        self.active_menu_preview = None;
    }

    pub(super) fn cancel_host_interactions_for_context_change(
        &mut self,
        dismiss_picker: bool,
        cx: &mut Context<Self>,
    ) {
        if dismiss_picker {
            self.prepare_paint_picker_for_dismissal(cx);
        }
        if let Some(target) = self.active_picker.clone() {
            self.emit_crop_cancel_if_active(&target, cx);
        }
        self.cancel_active_paint_edit(cx);

        self.cancel_property_editor_transaction(cx);
        self.active_grid_dimensions_edit = None;
        self.cancel_variable_font_axis_transaction(cx);
        self.cancel_component_multiline_transaction(cx);
        self.cancel_component_swap_preview(cx);
        self.cancel_menu_preview(cx);
        self.editor_focus_return = None;
    }

    pub(super) fn current_property_value(
        &self,
        property: DesignPanelProperty,
    ) -> Option<DesignPanelValue> {
        let layout = self.node.layout.as_ref();
        let typography = self.node.typography.as_ref();
        let media = self.node.media.as_ref();
        match property {
            DesignPanelProperty::Visible => Some(DesignPanelValue::Bool(self.node.visible)),
            DesignPanelProperty::X => Some(DesignPanelValue::Number(self.node.x)),
            DesignPanelProperty::Y => Some(DesignPanelValue::Number(self.node.y)),
            DesignPanelProperty::SmartSelectionHorizontalSpacing
            | DesignPanelProperty::SmartSelectionVerticalSpacing => {
                let axis = Self::smart_selection_axis(property)?;
                let spacing = self
                    .smart_selection_view_data_for_context()?
                    .spacing(axis)?;
                Some(DesignPanelValue::Number(
                    spacing.value.uniform().unwrap_or_default(),
                ))
            }
            DesignPanelProperty::VectorVertexX => Some(DesignPanelValue::Number(
                self.active_vector_edit()?
                    .selected_vertices()
                    .next()?
                    .position
                    .x,
            )),
            DesignPanelProperty::VectorVertexY => Some(DesignPanelValue::Number(
                self.active_vector_edit()?
                    .selected_vertices()
                    .next()?
                    .position
                    .y,
            )),
            DesignPanelProperty::VectorVertexCornerRadius => Some(DesignPanelValue::Number(
                self.active_vector_edit()?
                    .selected_vertices()
                    .next()?
                    .corner_radius?,
            )),
            DesignPanelProperty::VectorHandleMirroring => Some(DesignPanelValue::HandleMirroring(
                self.active_vector_edit()?
                    .selected_vertices()
                    .next()?
                    .handle_mirroring?,
            )),
            DesignPanelProperty::Width => Some(DesignPanelValue::Number(self.node.width)),
            DesignPanelProperty::Height => Some(DesignPanelValue::Number(self.node.height)),
            DesignPanelProperty::Rotation => Some(DesignPanelValue::Number(self.node.rotation)),
            DesignPanelProperty::LockAspectRatio => {
                Some(DesignPanelValue::Bool(self.node.lock_aspect_ratio))
            }
            DesignPanelProperty::HorizontalConstraint => Some(DesignPanelValue::Constraint(
                self.node.horizontal_constraint,
            )),
            DesignPanelProperty::VerticalConstraint => {
                Some(DesignPanelValue::Constraint(self.node.vertical_constraint))
            }
            DesignPanelProperty::LayoutMode => Some(DesignPanelValue::LayoutMode(layout?.mode)),
            DesignPanelProperty::HorizontalSizing => {
                Some(DesignPanelValue::SizingMode(layout?.horizontal_sizing))
            }
            DesignPanelProperty::VerticalSizing => {
                Some(DesignPanelValue::SizingMode(layout?.vertical_sizing))
            }
            DesignPanelProperty::AutoLayoutAlignment => {
                Some(DesignPanelValue::AutoLayoutAlignment(
                    DesignAutoLayoutAlignment::new(layout?.alignment_x, layout?.alignment_y),
                ))
            }
            DesignPanelProperty::AlignmentX => {
                Some(DesignPanelValue::Integer(i64::from(layout?.alignment_x)))
            }
            DesignPanelProperty::AlignmentY => {
                Some(DesignPanelValue::Integer(i64::from(layout?.alignment_y)))
            }
            DesignPanelProperty::Wrap => Some(DesignPanelValue::Bool(layout?.wrap)),
            DesignPanelProperty::Gap => Some(DesignPanelValue::Number(layout?.gap)),
            DesignPanelProperty::ItemSpacingMode => {
                Some(DesignPanelValue::ItemSpacingMode(layout?.item_spacing_mode))
            }
            DesignPanelProperty::CounterAxisAlignContent => Some(
                DesignPanelValue::CounterAxisAlignContent(layout?.counter_axis_align_content),
            ),
            DesignPanelProperty::CounterAxisGap => {
                Some(DesignPanelValue::OptionalNumber(layout?.counter_axis_gap))
            }
            DesignPanelProperty::PaddingVertical => {
                Some(DesignPanelValue::Number(layout?.padding[0]))
            }
            DesignPanelProperty::PaddingHorizontal => {
                Some(DesignPanelValue::Number(layout?.padding[3]))
            }
            DesignPanelProperty::PaddingShorthand => {
                Some(DesignPanelValue::NumberList(layout?.padding.to_vec()))
            }
            DesignPanelProperty::PaddingTop => Some(DesignPanelValue::Number(layout?.padding[0])),
            DesignPanelProperty::PaddingRight => Some(DesignPanelValue::Number(layout?.padding[1])),
            DesignPanelProperty::PaddingBottom => {
                Some(DesignPanelValue::Number(layout?.padding[2]))
            }
            DesignPanelProperty::PaddingLeft => Some(DesignPanelValue::Number(layout?.padding[3])),
            DesignPanelProperty::ClipContent => Some(DesignPanelValue::Bool(layout?.clip_content)),
            DesignPanelProperty::IncludeStrokes => {
                Some(DesignPanelValue::Bool(layout?.include_strokes))
            }
            DesignPanelProperty::StackingOrder => {
                Some(DesignPanelValue::StackingOrder(layout?.stacking_order))
            }
            DesignPanelProperty::BaselineAlignment => Some(DesignPanelValue::BaselineAlignment(
                layout?.baseline_alignment,
            )),
            DesignPanelProperty::MinWidth => {
                Some(DesignPanelValue::OptionalNumber(layout?.item.min_width))
            }
            DesignPanelProperty::MaxWidth => {
                Some(DesignPanelValue::OptionalNumber(layout?.item.max_width))
            }
            DesignPanelProperty::MinHeight => {
                Some(DesignPanelValue::OptionalNumber(layout?.item.min_height))
            }
            DesignPanelProperty::MaxHeight => {
                Some(DesignPanelValue::OptionalNumber(layout?.item.max_height))
            }
            DesignPanelProperty::LayoutPositioning => Some(DesignPanelValue::LayoutPositioning(
                layout?.item.positioning,
            )),
            DesignPanelProperty::LayoutAlignSelf => {
                Some(DesignPanelValue::LayoutAlignSelf(layout?.item.align_self))
            }
            DesignPanelProperty::LayoutGrow => {
                Some(DesignPanelValue::Number(layout?.item.layout_grow))
            }
            DesignPanelProperty::GridAutoTracks => {
                Some(DesignPanelValue::GridAutoTracks(layout?.grid_auto_tracks))
            }
            DesignPanelProperty::GridItemsPositioning => Some(
                DesignPanelValue::GridItemsPositioning(layout?.grid_items_positioning),
            ),
            DesignPanelProperty::GridColumnCount => Some(DesignPanelValue::Integer(
                i64::try_from(layout?.grid_columns.len()).ok()?,
            )),
            DesignPanelProperty::GridRowCount => Some(DesignPanelValue::Integer(
                i64::try_from(layout?.grid_rows.len()).ok()?,
            )),
            DesignPanelProperty::GridColumnTrack(index) => Some(DesignPanelValue::GridTrack(
                *layout?.grid_columns.get(index)?,
            )),
            DesignPanelProperty::GridRowTrack(index) => {
                Some(DesignPanelValue::GridTrack(*layout?.grid_rows.get(index)?))
            }
            DesignPanelProperty::GridColumnTrackValue(index) => Some(DesignPanelValue::Number(
                layout?.grid_columns.get(index)?.value,
            )),
            DesignPanelProperty::GridRowTrackValue(index) => Some(DesignPanelValue::Number(
                layout?.grid_rows.get(index)?.value,
            )),
            DesignPanelProperty::GridRowIndex => Some(DesignPanelValue::Integer(
                i64::try_from(layout?.item.grid_row_index)
                    .ok()?
                    .saturating_add(1),
            )),
            DesignPanelProperty::GridColumnIndex => Some(DesignPanelValue::Integer(
                i64::try_from(layout?.item.grid_column_index)
                    .ok()?
                    .saturating_add(1),
            )),
            DesignPanelProperty::GridRowSpan => Some(DesignPanelValue::Integer(i64::from(
                layout?.item.grid_row_span,
            ))),
            DesignPanelProperty::GridColumnSpan => Some(DesignPanelValue::Integer(i64::from(
                layout?.item.grid_column_span,
            ))),
            DesignPanelProperty::GridHorizontalAlignment => Some(
                DesignPanelValue::GridItemAlignment(layout?.item.grid_horizontal_alignment),
            ),
            DesignPanelProperty::GridVerticalAlignment => Some(
                DesignPanelValue::GridItemAlignment(layout?.item.grid_vertical_alignment),
            ),
            DesignPanelProperty::Opacity => Some(DesignPanelValue::Number(self.node.opacity)),
            DesignPanelProperty::BlendMode => {
                Some(DesignPanelValue::BlendMode(self.node.blend_mode))
            }
            DesignPanelProperty::CornerRadius => self
                .node
                .corner_capabilities
                .uniform_radius
                .then_some(DesignPanelValue::Number(self.node.corner_radii[0])),
            DesignPanelProperty::CornerRadiusTopLeft => self
                .node
                .corner_capabilities
                .independent_radii
                .then_some(DesignPanelValue::Number(self.node.corner_radii[0])),
            DesignPanelProperty::CornerRadiusTopRight => self
                .node
                .corner_capabilities
                .independent_radii
                .then_some(DesignPanelValue::Number(self.node.corner_radii[1])),
            DesignPanelProperty::CornerRadiusBottomRight => self
                .node
                .corner_capabilities
                .independent_radii
                .then_some(DesignPanelValue::Number(self.node.corner_radii[2])),
            DesignPanelProperty::CornerRadiusBottomLeft => self
                .node
                .corner_capabilities
                .independent_radii
                .then_some(DesignPanelValue::Number(self.node.corner_radii[3])),
            DesignPanelProperty::IndependentCorners => self
                .node
                .corner_capabilities
                .independent_radii
                .then_some(DesignPanelValue::Bool(self.node.independent_corners)),
            DesignPanelProperty::CornerSmoothing => self
                .node
                .corner_capabilities
                .smoothing
                .then_some(DesignPanelValue::Ratio(self.node.corner_smoothing)),
            DesignPanelProperty::FillShowsInExports => self
                .node
                .fill_shows_in_exports
                .filter(|_| !self.node.fills.is_empty())
                .map(DesignPanelValue::Bool),
            DesignPanelProperty::TypographyStyle => Some(DesignPanelValue::TypographyStyle(
                typography?
                    .style_binding
                    .as_ref()
                    .map(|binding| binding.selection.clone()),
            )),
            DesignPanelProperty::FontFamily => {
                Some(DesignPanelValue::Text(typography?.family.clone()))
            }
            DesignPanelProperty::FontStyle => {
                Some(DesignPanelValue::Text(typography?.style.clone()))
            }
            DesignPanelProperty::FontWeight => Some(DesignPanelValue::Number(typography?.weight)),
            DesignPanelProperty::FontSize => Some(DesignPanelValue::Number(typography?.size)),
            DesignPanelProperty::LineHeight => {
                Some(DesignPanelValue::LineHeight(typography?.line_height))
            }
            DesignPanelProperty::LetterSpacing => {
                Some(DesignPanelValue::LetterSpacing(typography?.letter_spacing))
            }
            DesignPanelProperty::TextLeadingTrim => {
                Some(DesignPanelValue::TextLeadingTrim(typography?.leading_trim))
            }
            DesignPanelProperty::ParagraphSpacing => {
                Some(DesignPanelValue::Number(typography?.paragraph_spacing))
            }
            DesignPanelProperty::ParagraphIndent => {
                Some(DesignPanelValue::Number(typography?.paragraph_indent))
            }
            DesignPanelProperty::ListSpacing => {
                Some(DesignPanelValue::Number(typography?.list_spacing))
            }
            DesignPanelProperty::TextHangingPunctuation => {
                Some(DesignPanelValue::Bool(typography?.hanging_punctuation))
            }
            DesignPanelProperty::TextHangingLists => {
                Some(DesignPanelValue::Bool(typography?.hanging_lists))
            }
            DesignPanelProperty::HorizontalTextAlignment => Some(
                DesignPanelValue::TextHorizontalAlignment(typography?.horizontal_alignment),
            ),
            DesignPanelProperty::VerticalTextAlignment => Some(
                DesignPanelValue::TextVerticalAlignment(typography?.vertical_alignment),
            ),
            DesignPanelProperty::TextResize => {
                Some(DesignPanelValue::TextResize(typography?.resize))
            }
            DesignPanelProperty::TextTruncate => Some(DesignPanelValue::Bool(typography?.truncate)),
            DesignPanelProperty::TextMaxLines => Some(DesignPanelValue::OptionalNumber(
                typography?.max_lines.map(|lines| lines as f32),
            )),
            DesignPanelProperty::TextDecoration => {
                Some(DesignPanelValue::TextDecoration(typography?.decoration))
            }
            DesignPanelProperty::TextDecorationStyle => Some(
                DesignPanelValue::TextDecorationStyle(typography?.decoration_details?.style),
            ),
            DesignPanelProperty::TextDecorationOffset => Some(
                DesignPanelValue::TextDecorationMetric(typography?.decoration_details?.offset),
            ),
            DesignPanelProperty::TextDecorationThickness => Some(
                DesignPanelValue::TextDecorationMetric(typography?.decoration_details?.thickness),
            ),
            DesignPanelProperty::TextDecorationColor => Some(
                DesignPanelValue::TextDecorationColor(typography?.decoration_details?.color),
            ),
            DesignPanelProperty::TextDecorationSkipInk => Some(DesignPanelValue::Bool(
                typography?.decoration_details?.skip_ink,
            )),
            DesignPanelProperty::TextCase => Some(DesignPanelValue::TextCase(typography?.case)),
            DesignPanelProperty::TextList => Some(DesignPanelValue::TextList(typography?.list)),
            DesignPanelProperty::TextPathStartSegment => Some(DesignPanelValue::Integer(
                i64::from(self.node.text_path_start_data?.segment),
            )),
            DesignPanelProperty::TextPathStartPosition => Some(DesignPanelValue::Ratio(
                self.node.text_path_start_data?.position,
            )),
            DesignPanelProperty::ComponentProperty(index) => Some(DesignPanelValue::Text(
                self.node
                    .component_properties
                    .get(index)?
                    .effective_value()
                    .display_value(),
            )),
            DesignPanelProperty::SlotStretchChildOnInsert(index) => Some(DesignPanelValue::Bool(
                self.node
                    .component_properties
                    .get(index)?
                    .slot_settings()?
                    .stretch_child_on_insert,
            )),
            DesignPanelProperty::SlotDisplayEmpty(index) => Some(DesignPanelValue::Bool(
                self.node
                    .component_properties
                    .get(index)?
                    .slot_settings()?
                    .display_empty,
            )),
            DesignPanelProperty::SlotMinimumInstances(index) => {
                Some(DesignPanelValue::OptionalNumber(
                    self.node
                        .component_properties
                        .get(index)?
                        .slot_settings()?
                        .minimum_children
                        .map(|value| value as f32),
                ))
            }
            DesignPanelProperty::SlotMaximumInstances(index) => {
                Some(DesignPanelValue::OptionalNumber(
                    self.node
                        .component_properties
                        .get(index)?
                        .slot_settings()?
                        .maximum_children
                        .map(|value| value as f32),
                ))
            }
            DesignPanelProperty::SlotPreferredValuesOnly(index) => Some(DesignPanelValue::Bool(
                self.node
                    .component_properties
                    .get(index)?
                    .slot_settings()?
                    .preferred_values_only,
            )),
            DesignPanelProperty::MediaCropMode => {
                Some(DesignPanelValue::Text(media?.crop_mode.clone()))
            }
            DesignPanelProperty::MediaExposure => Some(DesignPanelValue::Number(media?.exposure)),
            DesignPanelProperty::MediaContrast => Some(DesignPanelValue::Number(media?.contrast)),
            DesignPanelProperty::MediaSaturation => {
                Some(DesignPanelValue::Number(media?.saturation))
            }
            DesignPanelProperty::MediaTemperature => {
                Some(DesignPanelValue::Number(media?.temperature))
            }
            DesignPanelProperty::MediaTint => Some(DesignPanelValue::Number(media?.tint)),
            DesignPanelProperty::MediaHighlights => {
                Some(DesignPanelValue::Number(media?.highlights))
            }
            DesignPanelProperty::MediaShadows => Some(DesignPanelValue::Number(media?.shadows)),
            DesignPanelProperty::PolygonCount => {
                let DesignShapeGeometry::Polygon(geometry) = self.node.shape_geometry else {
                    return None;
                };
                Some(DesignPanelValue::Integer(i64::from(geometry.point_count)))
            }
            DesignPanelProperty::StarPointCount => {
                let DesignShapeGeometry::Star(geometry) = self.node.shape_geometry else {
                    return None;
                };
                Some(DesignPanelValue::Integer(i64::from(geometry.point_count)))
            }
            DesignPanelProperty::StarInnerRadius => {
                let DesignShapeGeometry::Star(geometry) = self.node.shape_geometry else {
                    return None;
                };
                Some(DesignPanelValue::Ratio(geometry.inner_radius))
            }
            DesignPanelProperty::ArcStartingAngle => {
                let DesignShapeGeometry::Ellipse(arc) = self.node.shape_geometry else {
                    return None;
                };
                Some(DesignPanelValue::AngleRadians(arc.starting_angle))
            }
            DesignPanelProperty::ArcSweep => {
                let DesignShapeGeometry::Ellipse(arc) = self.node.shape_geometry else {
                    return None;
                };
                Some(DesignPanelValue::AngleRadians(arc.sweep_angle()))
            }
            DesignPanelProperty::ArcEndingAngle => {
                let DesignShapeGeometry::Ellipse(arc) = self.node.shape_geometry else {
                    return None;
                };
                Some(DesignPanelValue::AngleRadians(arc.ending_angle))
            }
            DesignPanelProperty::ArcInnerRadius => {
                let DesignShapeGeometry::Ellipse(arc) = self.node.shape_geometry else {
                    return None;
                };
                Some(DesignPanelValue::Ratio(arc.inner_radius))
            }
            DesignPanelProperty::BooleanOperation => {
                let DesignShapeGeometry::Boolean(operation) = self.node.shape_geometry else {
                    return None;
                };
                Some(DesignPanelValue::BooleanOperation(operation))
            }
            DesignPanelProperty::IsMask => {
                Some(DesignPanelValue::Bool(self.node.effective_is_mask()))
            }
            DesignPanelProperty::MaskType => self
                .node
                .effective_mask_type()
                .map(DesignPanelValue::MaskType),
            DesignPanelProperty::TableRows => {
                let DesignShapeGeometry::Table(table) = self.node.shape_geometry else {
                    return None;
                };
                Some(DesignPanelValue::Integer(i64::from(table.row_count)))
            }
            DesignPanelProperty::TableColumns => {
                let DesignShapeGeometry::Table(table) = self.node.shape_geometry else {
                    return None;
                };
                Some(DesignPanelValue::Integer(i64::from(table.column_count)))
            }
            DesignPanelProperty::SectionContentsHidden => Some(DesignPanelValue::Bool(
                self.node.section.as_ref()?.contents_hidden,
            )),
            DesignPanelProperty::SectionDevStatus => Some(DesignPanelValue::SectionDevStatus(
                self.node
                    .section
                    .as_ref()?
                    .dev_status
                    .as_ref()
                    .map(|status| status.kind),
            )),
            DesignPanelProperty::TransformRepeatType(index) => Some(DesignPanelValue::RepeatType(
                self.node.transform_modifiers.get(index)?.mode.repeat_type(),
            )),
            DesignPanelProperty::TransformRepeatAxis(index) => self
                .node
                .transform_modifiers
                .get(index)?
                .mode
                .axis()
                .map(DesignPanelValue::RepeatAxis),
            DesignPanelProperty::TransformRepeatCount(index) => Some(DesignPanelValue::Integer(
                i64::from(self.node.transform_modifiers.get(index)?.count),
            )),
            DesignPanelProperty::TransformRepeatUnit(index) => Some(
                DesignPanelValue::TransformUnit(self.node.transform_modifiers.get(index)?.unit),
            ),
            DesignPanelProperty::TransformRepeatOffset(index) => Some(DesignPanelValue::Number(
                self.node.transform_modifiers.get(index)?.offset,
            )),
            DesignPanelProperty::SelectionColor(index) => self
                .node
                .resolved_selection_colors()
                .get(index)
                .map(|selection_color| DesignPanelValue::Color(selection_color.color)),
            DesignPanelProperty::PaintOpacity { collection, index } => {
                let target = PaintPickerTarget {
                    collection,
                    index,
                    paint_id: "".into(),
                };
                Some(DesignPanelValue::Number(
                    self.picker_paint(&target)?.opacity,
                ))
            }
            DesignPanelProperty::PaintVisible { collection, index } => {
                let target = PaintPickerTarget {
                    collection,
                    index,
                    paint_id: "".into(),
                };
                Some(DesignPanelValue::Bool(self.picker_paint(&target)?.visible))
            }
            DesignPanelProperty::StrokeWeight => {
                let stroke = self.node.stroke.as_ref()?;
                (stroke.weights.mode != DesignStrokeWeightMode::Custom)
                    .then_some(DesignPanelValue::Number(stroke.weights.active()))
            }
            DesignPanelProperty::StrokeWeightMode => {
                let stroke = self.node.stroke.as_ref()?;
                stroke
                    .capabilities
                    .individual_weights
                    .then_some(DesignPanelValue::StrokeWeightMode(stroke.weights.mode))
            }
            DesignPanelProperty::StrokeWeightTop => {
                let stroke = self.node.stroke.as_ref()?;
                (stroke.capabilities.individual_weights
                    && stroke.weights.mode == DesignStrokeWeightMode::Custom)
                    .then_some(DesignPanelValue::Number(stroke.weights.top))
            }
            DesignPanelProperty::StrokeWeightRight => {
                let stroke = self.node.stroke.as_ref()?;
                (stroke.capabilities.individual_weights
                    && stroke.weights.mode == DesignStrokeWeightMode::Custom)
                    .then_some(DesignPanelValue::Number(stroke.weights.right))
            }
            DesignPanelProperty::StrokeWeightBottom => {
                let stroke = self.node.stroke.as_ref()?;
                (stroke.capabilities.individual_weights
                    && stroke.weights.mode == DesignStrokeWeightMode::Custom)
                    .then_some(DesignPanelValue::Number(stroke.weights.bottom))
            }
            DesignPanelProperty::StrokeWeightLeft => {
                let stroke = self.node.stroke.as_ref()?;
                (stroke.capabilities.individual_weights
                    && stroke.weights.mode == DesignStrokeWeightMode::Custom)
                    .then_some(DesignPanelValue::Number(stroke.weights.left))
            }
            DesignPanelProperty::StrokeAlign => Some(DesignPanelValue::StrokeAlign(
                self.node.stroke.as_ref()?.align,
            )),
            DesignPanelProperty::StrokeStartCap => {
                let stroke = self.node.stroke.as_ref()?;
                (stroke.complex_stroke.is_basic()
                    && stroke.edit_context.endpoint_control()
                        == DesignStrokeEndpointControl::StartAndEnd)
                    .then_some(DesignPanelValue::StrokeCap(stroke.start_cap))
            }
            DesignPanelProperty::StrokeEndCap => {
                let stroke = self.node.stroke.as_ref()?;
                (stroke.complex_stroke.is_basic()
                    && stroke.edit_context.endpoint_control()
                        == DesignStrokeEndpointControl::StartAndEnd)
                    .then_some(DesignPanelValue::StrokeCap(stroke.end_cap))
            }
            DesignPanelProperty::StrokeEndpointCap => {
                let stroke = self.node.stroke.as_ref()?;
                (stroke.complex_stroke.is_basic()
                    && matches!(
                        stroke.edit_context.endpoint_control(),
                        DesignStrokeEndpointControl::Aggregate
                            | DesignStrokeEndpointControl::SelectedVertices
                    ))
                .then_some(DesignPanelValue::StrokeCap(stroke.endpoint_cap))
            }
            DesignPanelProperty::StrokeDashMode => {
                let stroke = self.node.stroke.as_ref()?;
                stroke
                    .complex_stroke
                    .is_basic()
                    .then_some(DesignPanelValue::StrokeDashMode(stroke.dashes.mode))
            }
            DesignPanelProperty::StrokeDashPattern => {
                let stroke = self.node.stroke.as_ref()?;
                (stroke.complex_stroke.is_basic() && !stroke.dashes.is_solid())
                    .then(|| DesignPanelValue::NumberList(stroke.dashes.pattern.clone()))
            }
            DesignPanelProperty::StrokeDashCap => {
                let stroke = self.node.stroke.as_ref()?;
                (stroke.complex_stroke.is_basic() && !stroke.dashes.is_solid())
                    .then_some(DesignPanelValue::StrokeCap(stroke.dash_cap))
            }
            DesignPanelProperty::StrokeJoin => {
                let stroke = self.node.stroke.as_ref()?;
                (stroke.capabilities.joins && stroke.complex_stroke.is_basic())
                    .then_some(DesignPanelValue::StrokeJoin(stroke.join))
            }
            DesignPanelProperty::StrokeMiterAngle => {
                let stroke = self.node.stroke.as_ref()?;
                (stroke.capabilities.joins
                    && stroke.complex_stroke.is_basic()
                    && stroke.join == DesignStrokeJoin::Miter)
                    .then_some(DesignPanelValue::Number(stroke.miter_angle))
            }
            DesignPanelProperty::StrokeVariableWidth => {
                let stroke = self.node.stroke.as_ref()?;
                stroke
                    .supports_variable_width()
                    .then(|| DesignPanelValue::StrokeVariableWidth(stroke.variable_width.clone()))
            }
            DesignPanelProperty::StrokeVariableWidthPointPosition(index) => {
                let stroke = self.node.stroke.as_ref()?;
                if !stroke.supports_variable_width() {
                    return None;
                }
                stroke
                    .variable_width
                    .as_ref()?
                    .points()?
                    .get(index)
                    .map(|point| DesignPanelValue::Number(point.position))
            }
            DesignPanelProperty::StrokeVariableWidthPointWidth(index) => {
                let stroke = self.node.stroke.as_ref()?;
                if !stroke.supports_variable_width() {
                    return None;
                }
                stroke
                    .variable_width
                    .as_ref()?
                    .points()?
                    .get(index)
                    .map(|point| DesignPanelValue::Number(point.width))
            }
            DesignPanelProperty::StrokeType => {
                let stroke = self.node.stroke.as_ref()?;
                stroke
                    .capabilities
                    .complex_stroke
                    .then_some(DesignPanelValue::StrokeType(stroke.complex_stroke.kind()))
            }
            DesignPanelProperty::StrokeStretchBrush => {
                let stroke = self.node.stroke.as_ref()?;
                match &stroke.complex_stroke {
                    DesignComplexStroke::StretchBrush(stretch) => {
                        Some(DesignPanelValue::StrokeStretchBrush(stretch.brush))
                    }
                    _ => None,
                }
            }
            DesignPanelProperty::StrokeBrushDirection => {
                let stroke = self.node.stroke.as_ref()?;
                match &stroke.complex_stroke {
                    DesignComplexStroke::StretchBrush(stretch) => {
                        Some(DesignPanelValue::StrokeBrushDirection(stretch.direction))
                    }
                    _ => None,
                }
            }
            DesignPanelProperty::StrokeScatterBrush => {
                let stroke = self.node.stroke.as_ref()?;
                match &stroke.complex_stroke {
                    DesignComplexStroke::ScatterBrush(scatter) => {
                        Some(DesignPanelValue::StrokeScatterBrush(scatter.brush))
                    }
                    _ => None,
                }
            }
            DesignPanelProperty::StrokeScatterGap => {
                let stroke = self.node.stroke.as_ref()?;
                match &stroke.complex_stroke {
                    DesignComplexStroke::ScatterBrush(scatter) => {
                        Some(DesignPanelValue::Number(scatter.gap))
                    }
                    _ => None,
                }
            }
            DesignPanelProperty::StrokeScatterWiggle => {
                let stroke = self.node.stroke.as_ref()?;
                match &stroke.complex_stroke {
                    DesignComplexStroke::ScatterBrush(scatter) => {
                        Some(DesignPanelValue::Number(scatter.wiggle))
                    }
                    _ => None,
                }
            }
            DesignPanelProperty::StrokeScatterSizeJitter => {
                let stroke = self.node.stroke.as_ref()?;
                match &stroke.complex_stroke {
                    DesignComplexStroke::ScatterBrush(scatter) => {
                        Some(DesignPanelValue::Number(scatter.size_jitter))
                    }
                    _ => None,
                }
            }
            DesignPanelProperty::StrokeScatterAngularJitter => {
                let stroke = self.node.stroke.as_ref()?;
                match &stroke.complex_stroke {
                    DesignComplexStroke::ScatterBrush(scatter) => {
                        Some(DesignPanelValue::Number(scatter.angular_jitter))
                    }
                    _ => None,
                }
            }
            DesignPanelProperty::StrokeScatterRotation => {
                let stroke = self.node.stroke.as_ref()?;
                match &stroke.complex_stroke {
                    DesignComplexStroke::ScatterBrush(scatter) => {
                        Some(DesignPanelValue::Number(scatter.rotation))
                    }
                    _ => None,
                }
            }
            DesignPanelProperty::StrokeDynamicFrequency => {
                let stroke = self.node.stroke.as_ref()?;
                match &stroke.complex_stroke {
                    DesignComplexStroke::Dynamic(dynamic) => {
                        Some(DesignPanelValue::Number(dynamic.frequency))
                    }
                    _ => None,
                }
            }
            DesignPanelProperty::StrokeDynamicWiggle => {
                let stroke = self.node.stroke.as_ref()?;
                match &stroke.complex_stroke {
                    DesignComplexStroke::Dynamic(dynamic) => {
                        Some(DesignPanelValue::Number(dynamic.wiggle))
                    }
                    _ => None,
                }
            }
            DesignPanelProperty::StrokeDynamicSmoothen => {
                let stroke = self.node.stroke.as_ref()?;
                match &stroke.complex_stroke {
                    DesignComplexStroke::Dynamic(dynamic) => {
                        Some(DesignPanelValue::Number(dynamic.smoothen))
                    }
                    _ => None,
                }
            }
            DesignPanelProperty::EffectKind(index) => Some(DesignPanelValue::EffectKind(
                self.node.effects.get(index)?.kind,
            )),
            DesignPanelProperty::EffectVisible(index) => Some(DesignPanelValue::Bool(
                self.node.effects.get(index)?.visible,
            )),
            DesignPanelProperty::EffectSettings(index) => Some(DesignPanelValue::EffectSettings(
                self.node.effects.get(index)?.settings.clone(),
            )),
            DesignPanelProperty::EffectShadowColor(index) => {
                let effect = self.node.effects.get(index)?;
                let color = match &effect.settings {
                    DesignEffectSettings::DropShadow(settings) => settings.color,
                    DesignEffectSettings::InnerShadow(settings) => settings.color,
                    _ => return None,
                };
                Some(DesignPanelValue::Color(color))
            }
            DesignPanelProperty::EffectShadowBlendMode(index) => {
                let effect = self.node.effects.get(index)?;
                let blend_mode = match &effect.settings {
                    DesignEffectSettings::DropShadow(settings) => settings.blend_mode,
                    DesignEffectSettings::InnerShadow(settings) => settings.blend_mode,
                    _ => return None,
                };
                Some(DesignPanelValue::BlendMode(blend_mode))
            }
            DesignPanelProperty::EffectShadowBlur(index) => {
                let effect = self.node.effects.get(index)?;
                let radius = match &effect.settings {
                    DesignEffectSettings::DropShadow(settings) => settings.radius,
                    DesignEffectSettings::InnerShadow(settings) => settings.radius,
                    _ => return None,
                };
                Some(DesignPanelValue::Number(radius))
            }
            DesignPanelProperty::EffectShadowSpread(index) => {
                let effect = self.node.effects.get(index)?;
                let spread = match &effect.settings {
                    DesignEffectSettings::DropShadow(settings) => settings.spread,
                    DesignEffectSettings::InnerShadow(settings) => settings.spread,
                    _ => return None,
                };
                Some(DesignPanelValue::Number(spread))
            }
            DesignPanelProperty::EffectShadowOffsetX(index) => {
                let effect = self.node.effects.get(index)?;
                let offset_x = match &effect.settings {
                    DesignEffectSettings::DropShadow(settings) => settings.offset.x,
                    DesignEffectSettings::InnerShadow(settings) => settings.offset.x,
                    _ => return None,
                };
                Some(DesignPanelValue::Number(offset_x))
            }
            DesignPanelProperty::EffectShadowOffsetY(index) => {
                let effect = self.node.effects.get(index)?;
                let offset_y = match &effect.settings {
                    DesignEffectSettings::DropShadow(settings) => settings.offset.y,
                    DesignEffectSettings::InnerShadow(settings) => settings.offset.y,
                    _ => return None,
                };
                Some(DesignPanelValue::Number(offset_y))
            }
            DesignPanelProperty::EffectDropShadowShowBehindNode(index) => {
                let effect = self.node.effects.get(index)?;
                let DesignEffectSettings::DropShadow(settings) = &effect.settings else {
                    return None;
                };
                Some(DesignPanelValue::Bool(settings.show_behind_node))
            }
            DesignPanelProperty::EffectBlurType(index) => {
                let effect = self.node.effects.get(index)?;
                let blur_type = match &effect.settings {
                    DesignEffectSettings::LayerBlur(settings)
                    | DesignEffectSettings::BackgroundBlur(settings) => settings.blur_type(),
                    _ => return None,
                };
                Some(DesignPanelValue::EffectBlurType(blur_type))
            }
            DesignPanelProperty::EffectBlurRadius(index) => {
                let effect = self.node.effects.get(index)?;
                let settings = match &effect.settings {
                    DesignEffectSettings::LayerBlur(settings)
                    | DesignEffectSettings::BackgroundBlur(settings) => settings,
                    _ => return None,
                };
                let DesignBlurEffect::Normal { radius } = settings else {
                    return None;
                };
                Some(DesignPanelValue::Number(*radius))
            }
            DesignPanelProperty::EffectProgressiveBlurStartRadius(index) => {
                let effect = self.node.effects.get(index)?;
                let settings = match &effect.settings {
                    DesignEffectSettings::LayerBlur(settings)
                    | DesignEffectSettings::BackgroundBlur(settings) => settings,
                    _ => return None,
                };
                let DesignBlurEffect::Progressive { start_radius, .. } = settings else {
                    return None;
                };
                Some(DesignPanelValue::Number(*start_radius))
            }
            DesignPanelProperty::EffectProgressiveBlurEndRadius(index) => {
                let effect = self.node.effects.get(index)?;
                let settings = match &effect.settings {
                    DesignEffectSettings::LayerBlur(settings)
                    | DesignEffectSettings::BackgroundBlur(settings) => settings,
                    _ => return None,
                };
                let DesignBlurEffect::Progressive { end_radius, .. } = settings else {
                    return None;
                };
                Some(DesignPanelValue::Number(*end_radius))
            }
            DesignPanelProperty::EffectProgressiveBlurStartOffsetX(index) => {
                let effect = self.node.effects.get(index)?;
                let settings = match &effect.settings {
                    DesignEffectSettings::LayerBlur(settings)
                    | DesignEffectSettings::BackgroundBlur(settings) => settings,
                    _ => return None,
                };
                let DesignBlurEffect::Progressive { start_offset, .. } = settings else {
                    return None;
                };
                Some(DesignPanelValue::Number(start_offset.x))
            }
            DesignPanelProperty::EffectProgressiveBlurStartOffsetY(index) => {
                let effect = self.node.effects.get(index)?;
                let settings = match &effect.settings {
                    DesignEffectSettings::LayerBlur(settings)
                    | DesignEffectSettings::BackgroundBlur(settings) => settings,
                    _ => return None,
                };
                let DesignBlurEffect::Progressive { start_offset, .. } = settings else {
                    return None;
                };
                Some(DesignPanelValue::Number(start_offset.y))
            }
            DesignPanelProperty::EffectProgressiveBlurEndOffsetX(index) => {
                let effect = self.node.effects.get(index)?;
                let settings = match &effect.settings {
                    DesignEffectSettings::LayerBlur(settings)
                    | DesignEffectSettings::BackgroundBlur(settings) => settings,
                    _ => return None,
                };
                let DesignBlurEffect::Progressive { end_offset, .. } = settings else {
                    return None;
                };
                Some(DesignPanelValue::Number(end_offset.x))
            }
            DesignPanelProperty::EffectProgressiveBlurEndOffsetY(index) => {
                let effect = self.node.effects.get(index)?;
                let settings = match &effect.settings {
                    DesignEffectSettings::LayerBlur(settings)
                    | DesignEffectSettings::BackgroundBlur(settings) => settings,
                    _ => return None,
                };
                let DesignBlurEffect::Progressive { end_offset, .. } = settings else {
                    return None;
                };
                Some(DesignPanelValue::Number(end_offset.y))
            }
            DesignPanelProperty::EffectNoiseType(index) => {
                let effect = self.node.effects.get(index)?;
                let DesignEffectSettings::Noise(settings) = &effect.settings else {
                    return None;
                };
                Some(DesignPanelValue::EffectNoiseType(
                    settings.colors.noise_type(),
                ))
            }
            DesignPanelProperty::EffectNoisePrimaryColor(index) => {
                let effect = self.node.effects.get(index)?;
                let DesignEffectSettings::Noise(settings) = &effect.settings else {
                    return None;
                };
                settings.colors.primary_color().map(DesignPanelValue::Color)
            }
            DesignPanelProperty::EffectNoiseSecondaryColor(index) => {
                let effect = self.node.effects.get(index)?;
                let DesignEffectSettings::Noise(settings) = &effect.settings else {
                    return None;
                };
                let DesignNoiseColors::Duotone {
                    secondary_color, ..
                } = &settings.colors
                else {
                    return None;
                };
                Some(DesignPanelValue::Color(*secondary_color))
            }
            DesignPanelProperty::EffectNoiseOpacity(index) => {
                let effect = self.node.effects.get(index)?;
                let DesignEffectSettings::Noise(settings) = &effect.settings else {
                    return None;
                };
                let DesignNoiseColors::Multitone { opacity } = &settings.colors else {
                    return None;
                };
                Some(DesignPanelValue::Number(*opacity))
            }
            DesignPanelProperty::EffectNoiseBlendMode(index) => {
                let effect = self.node.effects.get(index)?;
                let DesignEffectSettings::Noise(settings) = &effect.settings else {
                    return None;
                };
                Some(DesignPanelValue::BlendMode(settings.blend_mode))
            }
            DesignPanelProperty::EffectNoiseSizeX(index) => {
                let effect = self.node.effects.get(index)?;
                let DesignEffectSettings::Noise(settings) = &effect.settings else {
                    return None;
                };
                Some(DesignPanelValue::Number(settings.size.x))
            }
            DesignPanelProperty::EffectNoiseSizeY(index) => {
                let effect = self.node.effects.get(index)?;
                let DesignEffectSettings::Noise(settings) = &effect.settings else {
                    return None;
                };
                Some(DesignPanelValue::Number(settings.size.y))
            }
            DesignPanelProperty::EffectNoiseDensity(index) => {
                let effect = self.node.effects.get(index)?;
                let DesignEffectSettings::Noise(settings) = &effect.settings else {
                    return None;
                };
                Some(DesignPanelValue::Number(settings.density))
            }
            DesignPanelProperty::EffectTextureSizeX(index) => {
                let effect = self.node.effects.get(index)?;
                let DesignEffectSettings::Texture(settings) = &effect.settings else {
                    return None;
                };
                Some(DesignPanelValue::Number(settings.size.x))
            }
            DesignPanelProperty::EffectTextureSizeY(index) => {
                let effect = self.node.effects.get(index)?;
                let DesignEffectSettings::Texture(settings) = &effect.settings else {
                    return None;
                };
                Some(DesignPanelValue::Number(settings.size.y))
            }
            DesignPanelProperty::EffectTextureRadius(index) => {
                let effect = self.node.effects.get(index)?;
                let DesignEffectSettings::Texture(settings) = &effect.settings else {
                    return None;
                };
                Some(DesignPanelValue::Number(settings.radius))
            }
            DesignPanelProperty::EffectTextureClipToShape(index) => {
                let effect = self.node.effects.get(index)?;
                let DesignEffectSettings::Texture(settings) = &effect.settings else {
                    return None;
                };
                Some(DesignPanelValue::Bool(settings.clip_to_shape))
            }
            DesignPanelProperty::EffectGlassLightIntensity(index) => {
                let effect = self.node.effects.get(index)?;
                let DesignEffectSettings::Glass(settings) = &effect.settings else {
                    return None;
                };
                Some(DesignPanelValue::Number(settings.light_intensity))
            }
            DesignPanelProperty::EffectGlassLightAngle(index) => {
                let effect = self.node.effects.get(index)?;
                let DesignEffectSettings::Glass(settings) = &effect.settings else {
                    return None;
                };
                Some(DesignPanelValue::Number(settings.light_angle))
            }
            DesignPanelProperty::EffectGlassRefraction(index) => {
                let effect = self.node.effects.get(index)?;
                let DesignEffectSettings::Glass(settings) = &effect.settings else {
                    return None;
                };
                Some(DesignPanelValue::Number(settings.refraction))
            }
            DesignPanelProperty::EffectGlassDepth(index) => {
                let effect = self.node.effects.get(index)?;
                let DesignEffectSettings::Glass(settings) = &effect.settings else {
                    return None;
                };
                Some(DesignPanelValue::Number(settings.depth))
            }
            DesignPanelProperty::EffectGlassDispersion(index) => {
                let effect = self.node.effects.get(index)?;
                let DesignEffectSettings::Glass(settings) = &effect.settings else {
                    return None;
                };
                Some(DesignPanelValue::Number(settings.dispersion))
            }
            DesignPanelProperty::EffectGlassFrost(index) => {
                let effect = self.node.effects.get(index)?;
                let DesignEffectSettings::Glass(settings) = &effect.settings else {
                    return None;
                };
                Some(DesignPanelValue::Number(settings.frost))
            }
            DesignPanelProperty::EffectGlassSplay(index) => {
                let effect = self.node.effects.get(index)?;
                let DesignEffectSettings::Glass(settings) = &effect.settings else {
                    return None;
                };
                Some(DesignPanelValue::Number(settings.splay))
            }
            DesignPanelProperty::EffectShaderProperty(index, property_index) => {
                let effect = self.node.effects.get(index)?;
                let DesignEffectSettings::Shader(shader) = &effect.settings else {
                    return None;
                };
                Some(DesignPanelValue::ShaderProperty(
                    shader.properties.get(property_index)?.value.clone(),
                ))
            }
            DesignPanelProperty::EffectBlur(index) => {
                Some(DesignPanelValue::Number(self.node.effects.get(index)?.blur))
            }
            DesignPanelProperty::EffectSpread(index) => Some(DesignPanelValue::Number(
                self.node.effects.get(index)?.spread,
            )),
            DesignPanelProperty::EffectOffsetX(index) => Some(DesignPanelValue::Number(
                self.node.effects.get(index)?.offset_x,
            )),
            DesignPanelProperty::EffectOffsetY(index) => Some(DesignPanelValue::Number(
                self.node.effects.get(index)?.offset_y,
            )),
            DesignPanelProperty::LayoutGridAlignment(index) => {
                match &self.node.layout_grids.get(index)?.settings {
                    DesignLayoutGridSettings::Uniform(_) => None,
                    DesignLayoutGridSettings::Columns(settings) => {
                        Some(DesignPanelValue::ColumnGridAlignment(settings.alignment))
                    }
                    DesignLayoutGridSettings::Rows(settings) => {
                        Some(DesignPanelValue::RowGridAlignment(settings.alignment))
                    }
                }
            }
            DesignPanelProperty::LayoutGridCount(index) => {
                match &self.node.layout_grids.get(index)?.settings {
                    DesignLayoutGridSettings::Uniform(_) => None,
                    DesignLayoutGridSettings::Columns(settings) => {
                        Some(DesignPanelValue::LayoutGridCount(settings.count))
                    }
                    DesignLayoutGridSettings::Rows(settings) => {
                        Some(DesignPanelValue::LayoutGridCount(settings.count))
                    }
                }
            }
            DesignPanelProperty::LayoutGridSize(index) => {
                match &self.node.layout_grids.get(index)?.settings {
                    DesignLayoutGridSettings::Uniform(settings) => {
                        Some(DesignPanelValue::Number(settings.size))
                    }
                    DesignLayoutGridSettings::Columns(settings)
                        if !settings.alignment.is_stretch() =>
                    {
                        Some(DesignPanelValue::Number(settings.size))
                    }
                    DesignLayoutGridSettings::Rows(settings)
                        if !settings.alignment.is_stretch() =>
                    {
                        Some(DesignPanelValue::Number(settings.size))
                    }
                    DesignLayoutGridSettings::Columns(_) | DesignLayoutGridSettings::Rows(_) => {
                        None
                    }
                }
            }
            DesignPanelProperty::LayoutGridOffset(index) => {
                match &self.node.layout_grids.get(index)?.settings {
                    DesignLayoutGridSettings::Columns(settings)
                        if settings.alignment.supports_offset() =>
                    {
                        Some(DesignPanelValue::Number(settings.offset))
                    }
                    DesignLayoutGridSettings::Rows(settings)
                        if settings.alignment.supports_offset() =>
                    {
                        Some(DesignPanelValue::Number(settings.offset))
                    }
                    DesignLayoutGridSettings::Uniform(_)
                    | DesignLayoutGridSettings::Columns(_)
                    | DesignLayoutGridSettings::Rows(_) => None,
                }
            }
            DesignPanelProperty::LayoutGridGutter(index) => {
                match &self.node.layout_grids.get(index)?.settings {
                    DesignLayoutGridSettings::Uniform(_) => None,
                    DesignLayoutGridSettings::Columns(settings) => {
                        Some(DesignPanelValue::Number(settings.gutter))
                    }
                    DesignLayoutGridSettings::Rows(settings) => {
                        Some(DesignPanelValue::Number(settings.gutter))
                    }
                }
            }
            DesignPanelProperty::LayoutGridMargin(index) => {
                match &self.node.layout_grids.get(index)?.settings {
                    DesignLayoutGridSettings::Columns(settings)
                        if settings.alignment.is_stretch() =>
                    {
                        Some(DesignPanelValue::Number(settings.margin))
                    }
                    DesignLayoutGridSettings::Rows(settings) if settings.alignment.is_stretch() => {
                        Some(DesignPanelValue::Number(settings.margin))
                    }
                    DesignLayoutGridSettings::Uniform(_)
                    | DesignLayoutGridSettings::Columns(_)
                    | DesignLayoutGridSettings::Rows(_) => None,
                }
            }
            DesignPanelProperty::LayoutGridKind(index) => Some(DesignPanelValue::GridKind(
                self.node.layout_grids.get(index)?.kind(),
            )),
            DesignPanelProperty::LayoutGridVisible(index) => Some(DesignPanelValue::Bool(
                self.node.layout_grids.get(index)?.visible,
            )),
            DesignPanelProperty::LayoutGridColor(index) => Some(DesignPanelValue::Color(
                self.node.layout_grids.get(index)?.color,
            )),
            DesignPanelProperty::LayoutGridOpacity(index) => Some(DesignPanelValue::Number(
                self.node.layout_grids.get(index)?.opacity,
            )),
            DesignPanelProperty::ExportSizing(index) => Some(DesignPanelValue::ExportSizing(
                self.export_configuration(index)?.sizing,
            )),
            DesignPanelProperty::ExportScale(index) => {
                let DesignExportSizing::Scale(scale) = self.export_configuration(index)?.sizing
                else {
                    return None;
                };
                Some(DesignPanelValue::Number(scale))
            }
            DesignPanelProperty::ExportSuffix(index) => Some(DesignPanelValue::Text(
                self.export_configuration(index)?.common.suffix,
            )),
            DesignPanelProperty::ExportFormat(index) => Some(DesignPanelValue::ExportFormat(
                self.export_configuration(index)?.format(),
            )),
            _ => None,
        }
    }

    pub(super) fn resolved_property_value(
        &self,
        property: DesignPanelProperty,
    ) -> Option<DesignPanelValue> {
        match self.property_value_states.get(&property) {
            Some(state) => state.resolved().cloned(),
            None => self.current_property_value(property),
        }
    }

    pub(super) fn display_property_value(
        &self,
        property: DesignPanelProperty,
        fallback: SharedString,
    ) -> SharedString {
        if self.smart_selection_property_is_mixed(property) {
            return "Mixed".into();
        }
        let Some(state) = self.property_value_states.get(&property) else {
            return fallback;
        };
        if state.is_mixed() {
            return "Mixed".into();
        }
        if state.is_unset() {
            return "—".into();
        }
        if state.binding().is_some() {
            return format!("{fallback}  ◇").into();
        }
        fallback
    }

    pub(super) fn property_options(
        &self,
        property: DesignPanelProperty,
    ) -> Option<Vec<PropertyOption>> {
        let option = |label: &'static str, value| PropertyOption {
            label: label.into(),
            value,
        };
        let text_options = |labels: &[&'static str]| {
            labels
                .iter()
                .map(|label| PropertyOption {
                    label: (*label).into(),
                    value: DesignPanelValue::Text((*label).into()),
                })
                .collect::<Vec<_>>()
        };

        match property {
            DesignPanelProperty::VectorHandleMirroring => Some(
                DesignHandleMirroring::ALL
                    .into_iter()
                    .map(|mirroring| PropertyOption {
                        label: mirroring.label().into(),
                        value: DesignPanelValue::HandleMirroring(mirroring),
                    })
                    .collect(),
            ),
            DesignPanelProperty::HorizontalConstraint => Some(vec![
                option(
                    DesignConstraint::Left.label(),
                    DesignPanelValue::Constraint(DesignConstraint::Left),
                ),
                option(
                    DesignConstraint::Right.label(),
                    DesignPanelValue::Constraint(DesignConstraint::Right),
                ),
                option(
                    DesignConstraint::LeftAndRight.label(),
                    DesignPanelValue::Constraint(DesignConstraint::LeftAndRight),
                ),
                option(
                    DesignConstraint::Center.label(),
                    DesignPanelValue::Constraint(DesignConstraint::Center),
                ),
                option(
                    DesignConstraint::Scale.label(),
                    DesignPanelValue::Constraint(DesignConstraint::Scale),
                ),
            ]),
            DesignPanelProperty::VerticalConstraint => Some(vec![
                option(
                    DesignConstraint::Top.label(),
                    DesignPanelValue::Constraint(DesignConstraint::Top),
                ),
                option(
                    DesignConstraint::Bottom.label(),
                    DesignPanelValue::Constraint(DesignConstraint::Bottom),
                ),
                option(
                    DesignConstraint::TopAndBottom.label(),
                    DesignPanelValue::Constraint(DesignConstraint::TopAndBottom),
                ),
                option(
                    DesignConstraint::Center.label(),
                    DesignPanelValue::Constraint(DesignConstraint::Center),
                ),
                option(
                    DesignConstraint::Scale.label(),
                    DesignPanelValue::Constraint(DesignConstraint::Scale),
                ),
            ]),
            DesignPanelProperty::HorizontalSizing | DesignPanelProperty::VerticalSizing => {
                let layout = self.node.layout.as_ref();
                let owns_active_flow = self.node.supports_auto_layout_container()
                    && layout.is_some_and(|layout| layout.mode != DesignLayoutMode::None);
                let is_text = matches!(
                    self.node.kind,
                    DesignPanelNodeKind::Text | DesignPanelNodeKind::TextPath
                );
                let participates = self.node.supports_auto_layout_child()
                    && self
                        .inspection_context
                        .parent_layout()
                        .participates_in_auto_layout();
                let requires_fill = layout.is_some_and(|layout| match property {
                    DesignPanelProperty::HorizontalSizing => layout.item.grid_column_span > 1,
                    DesignPanelProperty::VerticalSizing => layout.item.grid_row_span > 1,
                    _ => false,
                }) && self
                    .inspection_context
                    .parent_layout()
                    .auto_layout_direction()
                    == Some(DesignPanelAutoLayoutDirection::Grid);
                Some(
                    [
                        DesignSizingMode::Fixed,
                        DesignSizingMode::Hug,
                        DesignSizingMode::Fill,
                    ]
                    .into_iter()
                    .filter(|mode| match mode {
                        DesignSizingMode::Fixed => !requires_fill,
                        DesignSizingMode::Hug => !requires_fill && (owns_active_flow || is_text),
                        DesignSizingMode::Fill => participates,
                    })
                    .map(|mode| PropertyOption {
                        label: mode.label().into(),
                        value: DesignPanelValue::SizingMode(mode),
                    })
                    .collect(),
                )
            }
            DesignPanelProperty::ItemSpacingMode => Some(
                DesignItemSpacingMode::ALL
                    .into_iter()
                    .map(|mode| PropertyOption {
                        label: mode.label().into(),
                        value: DesignPanelValue::ItemSpacingMode(mode),
                    })
                    .collect(),
            ),
            DesignPanelProperty::CounterAxisAlignContent => Some(
                DesignCounterAxisAlignContent::ALL
                    .into_iter()
                    .map(|alignment| PropertyOption {
                        label: alignment.label().into(),
                        value: DesignPanelValue::CounterAxisAlignContent(alignment),
                    })
                    .collect(),
            ),
            DesignPanelProperty::LayoutPositioning => Some(
                DesignLayoutPositioning::ALL
                    .into_iter()
                    .map(|positioning| PropertyOption {
                        label: positioning.label().into(),
                        value: DesignPanelValue::LayoutPositioning(positioning),
                    })
                    .collect(),
            ),
            DesignPanelProperty::LayoutAlignSelf => Some(
                DesignLayoutAlignSelf::ALL
                    .into_iter()
                    .map(|alignment| PropertyOption {
                        label: alignment.label().into(),
                        value: DesignPanelValue::LayoutAlignSelf(alignment),
                    })
                    .collect(),
            ),
            DesignPanelProperty::StackingOrder => Some(
                DesignStackingOrder::ALL
                    .into_iter()
                    .map(|order| PropertyOption {
                        label: order.label().into(),
                        value: DesignPanelValue::StackingOrder(order),
                    })
                    .collect(),
            ),
            DesignPanelProperty::BaselineAlignment => Some(
                DesignBaselineAlignment::ALL
                    .into_iter()
                    .map(|alignment| PropertyOption {
                        label: alignment.label().into(),
                        value: DesignPanelValue::BaselineAlignment(alignment),
                    })
                    .collect(),
            ),
            DesignPanelProperty::GridItemsPositioning => Some(
                DesignGridItemsPositioning::ALL
                    .into_iter()
                    .map(|positioning| PropertyOption {
                        label: positioning.label().into(),
                        value: DesignPanelValue::GridItemsPositioning(positioning),
                    })
                    .collect(),
            ),
            property @ (DesignPanelProperty::GridColumnTrack(_)
            | DesignPanelProperty::GridRowTrack(_)) => {
                let fraction_allowed =
                    self.node
                        .layout
                        .as_ref()
                        .is_some_and(|layout| match property {
                            DesignPanelProperty::GridColumnTrack(_) => {
                                layout.horizontal_sizing != DesignSizingMode::Hug
                            }
                            DesignPanelProperty::GridRowTrack(_) => {
                                layout.vertical_sizing != DesignSizingMode::Hug
                            }
                            _ => false,
                        });
                Some(
                    vec![
                        PropertyOption {
                            label: "Fixed · 100 px".into(),
                            value: DesignPanelValue::GridTrack(DesignGridTrack::fixed(100.)),
                        },
                        PropertyOption {
                            label: "Fraction · 1 fr".into(),
                            value: DesignPanelValue::GridTrack(DesignGridTrack::fraction(1.)),
                        },
                        PropertyOption {
                            label: "Hug".into(),
                            value: DesignPanelValue::GridTrack(DesignGridTrack::hug()),
                        },
                    ]
                    .into_iter()
                    .filter(|option| {
                        fraction_allowed
                            || !matches!(
                                &option.value,
                                DesignPanelValue::GridTrack(track)
                                    if track.sizing == DesignGridTrackSizing::Fraction
                            )
                    })
                    .collect(),
                )
            }
            DesignPanelProperty::GridHorizontalAlignment
            | DesignPanelProperty::GridVerticalAlignment => Some(
                DesignGridItemAlignment::ALL
                    .into_iter()
                    .map(|alignment| PropertyOption {
                        label: alignment.label().into(),
                        value: DesignPanelValue::GridItemAlignment(alignment),
                    })
                    .collect(),
            ),
            DesignPanelProperty::BlendMode => Some(
                DesignBlendMode::ALL
                    .into_iter()
                    .filter(|mode| {
                        self.node.supports_pass_through_blend()
                            || *mode != DesignBlendMode::PassThrough
                    })
                    .map(|mode| PropertyOption {
                        label: mode.label().into(),
                        value: DesignPanelValue::BlendMode(mode),
                    })
                    .collect(),
            ),
            DesignPanelProperty::EffectShadowBlendMode(_)
            | DesignPanelProperty::EffectNoiseBlendMode(_) => Some(
                DesignBlendMode::NON_PASS_THROUGH
                    .into_iter()
                    .map(|mode| PropertyOption {
                        label: mode.label().into(),
                        value: DesignPanelValue::BlendMode(mode),
                    })
                    .collect(),
            ),
            DesignPanelProperty::EffectBlurType(_) => Some(
                DesignBlurType::ALL
                    .into_iter()
                    .map(|blur_type| PropertyOption {
                        label: blur_type.label().into(),
                        value: DesignPanelValue::EffectBlurType(blur_type),
                    })
                    .collect(),
            ),
            DesignPanelProperty::EffectNoiseType(_) => Some(
                DesignNoiseType::ALL
                    .into_iter()
                    .map(|noise_type| PropertyOption {
                        label: noise_type.label().into(),
                        value: DesignPanelValue::EffectNoiseType(noise_type),
                    })
                    .collect(),
            ),
            DesignPanelProperty::FontStyle => Some(text_options(&[
                "Regular",
                "Medium",
                "Semi Bold",
                "Bold",
                "Italic",
            ])),
            DesignPanelProperty::LineHeight => {
                let current = self.node.typography.as_ref()?.line_height;
                let pixels = match current {
                    DesignLineHeight::Pixels(value) => value,
                    _ => 24.,
                };
                let percent = match current {
                    DesignLineHeight::Percent(value) => value,
                    _ => 150.,
                };
                Some(vec![
                    PropertyOption {
                        label: "Auto".into(),
                        value: DesignPanelValue::LineHeight(DesignLineHeight::Auto),
                    },
                    PropertyOption {
                        label: format!("{} px", format_number(pixels)).into(),
                        value: DesignPanelValue::LineHeight(DesignLineHeight::Pixels(pixels)),
                    },
                    PropertyOption {
                        label: format!("{}%", format_number(percent)).into(),
                        value: DesignPanelValue::LineHeight(DesignLineHeight::Percent(percent)),
                    },
                ])
            }
            DesignPanelProperty::LetterSpacing => {
                let current = self.node.typography.as_ref()?.letter_spacing;
                let pixels = match current {
                    DesignLetterSpacing::Pixels(value) => value,
                    DesignLetterSpacing::Percent(_) => 0.,
                };
                let percent = match current {
                    DesignLetterSpacing::Percent(value) => value,
                    DesignLetterSpacing::Pixels(_) => 0.,
                };
                Some(vec![
                    PropertyOption {
                        label: format!("{} px", format_number(pixels)).into(),
                        value: DesignPanelValue::LetterSpacing(DesignLetterSpacing::Pixels(pixels)),
                    },
                    PropertyOption {
                        label: format!("{}%", format_number(percent)).into(),
                        value: DesignPanelValue::LetterSpacing(DesignLetterSpacing::Percent(
                            percent,
                        )),
                    },
                ])
            }
            DesignPanelProperty::HorizontalTextAlignment => Some(
                DesignTextHorizontalAlignment::ALL
                    .into_iter()
                    .map(|alignment| PropertyOption {
                        label: alignment.label().into(),
                        value: DesignPanelValue::TextHorizontalAlignment(alignment),
                    })
                    .collect(),
            ),
            DesignPanelProperty::VerticalTextAlignment => Some(
                DesignTextVerticalAlignment::ALL
                    .into_iter()
                    .map(|alignment| PropertyOption {
                        label: alignment.label().into(),
                        value: DesignPanelValue::TextVerticalAlignment(alignment),
                    })
                    .collect(),
            ),
            DesignPanelProperty::TextResize => Some(
                DesignTextResize::ALL
                    .into_iter()
                    .map(|resize| PropertyOption {
                        label: resize.label().into(),
                        value: DesignPanelValue::TextResize(resize),
                    })
                    .collect(),
            ),
            DesignPanelProperty::TextDecoration => Some(
                DesignTextDecoration::ALL
                    .into_iter()
                    .map(|decoration| PropertyOption {
                        label: decoration.label().into(),
                        value: DesignPanelValue::TextDecoration(decoration),
                    })
                    .collect(),
            ),
            DesignPanelProperty::TextCase => Some(
                DesignTextCase::ALL
                    .into_iter()
                    .map(|case| PropertyOption {
                        label: case.label().into(),
                        value: DesignPanelValue::TextCase(case),
                    })
                    .collect(),
            ),
            DesignPanelProperty::TextList => Some(
                DesignTextList::ALL
                    .into_iter()
                    .map(|list| PropertyOption {
                        label: list.label().into(),
                        value: DesignPanelValue::TextList(list),
                    })
                    .collect(),
            ),
            DesignPanelProperty::ComponentProperty(index) => {
                let component_property = self.node.component_properties.get(index)?;
                if matches!(
                    component_property.definition,
                    DesignComponentPropertyDefinition::InstanceSwap { .. }
                ) {
                    return None;
                }
                let options = component_property.definition.option_labels();
                if options.is_empty() {
                    None
                } else {
                    Some(
                        options
                            .into_iter()
                            .map(|label| PropertyOption {
                                value: DesignPanelValue::Text(label.clone()),
                                label,
                            })
                            .collect(),
                    )
                }
            }
            DesignPanelProperty::MediaCropMode => {
                Some(text_options(&["Fill", "Fit", "Crop", "Tile"]))
            }
            DesignPanelProperty::BooleanOperation => Some(
                DesignBooleanOperation::ALL
                    .into_iter()
                    .map(|operation| PropertyOption {
                        label: operation.label().into(),
                        value: DesignPanelValue::BooleanOperation(operation),
                    })
                    .collect(),
            ),
            DesignPanelProperty::MaskType => Some(
                DesignMaskType::ALL
                    .into_iter()
                    .map(|mask_type| PropertyOption {
                        label: mask_type.label().into(),
                        value: DesignPanelValue::MaskType(mask_type),
                    })
                    .collect(),
            ),
            DesignPanelProperty::SectionDevStatus => {
                let section = self.node.section.as_ref()?;
                let mut options = vec![PropertyOption {
                    label: "No status".into(),
                    value: DesignPanelValue::SectionDevStatus(None),
                }];
                options.extend(
                    DesignSectionDevStatusKind::ALL
                        .into_iter()
                        .filter(|status| {
                            *status != DesignSectionDevStatusKind::Completed
                                || section.capabilities.completed_status
                        })
                        .map(|status| PropertyOption {
                            label: status.label().into(),
                            value: DesignPanelValue::SectionDevStatus(Some(status)),
                        }),
                );
                Some(options)
            }
            DesignPanelProperty::TransformRepeatType(_) => Some(
                DesignRepeatType::ALL
                    .into_iter()
                    .map(|repeat_type| PropertyOption {
                        label: repeat_type.label().into(),
                        value: DesignPanelValue::RepeatType(repeat_type),
                    })
                    .collect(),
            ),
            DesignPanelProperty::TransformRepeatAxis(_) => Some(
                DesignRepeatAxis::ALL
                    .into_iter()
                    .map(|axis| PropertyOption {
                        label: axis.label().into(),
                        value: DesignPanelValue::RepeatAxis(axis),
                    })
                    .collect(),
            ),
            DesignPanelProperty::TransformRepeatUnit(_) => Some(
                DesignTransformUnit::ALL
                    .into_iter()
                    .map(|unit| PropertyOption {
                        label: unit.label().into(),
                        value: DesignPanelValue::TransformUnit(unit),
                    })
                    .collect(),
            ),
            DesignPanelProperty::StrokeAlign => Some(
                self.node
                    .stroke
                    .as_ref()?
                    .alignment_options()
                    .iter()
                    .copied()
                    .map(|align| PropertyOption {
                        label: align.label().into(),
                        value: DesignPanelValue::StrokeAlign(align),
                    })
                    .collect(),
            ),
            property @ (DesignPanelProperty::StrokeStartCap
            | DesignPanelProperty::StrokeEndCap
            | DesignPanelProperty::StrokeEndpointCap) => {
                let stroke = self.node.stroke.as_ref()?;
                if !stroke.complex_stroke.is_basic() {
                    return None;
                }
                let control = stroke.edit_context.endpoint_control();
                let supported = match property {
                    DesignPanelProperty::StrokeStartCap | DesignPanelProperty::StrokeEndCap => {
                        control == DesignStrokeEndpointControl::StartAndEnd
                    }
                    DesignPanelProperty::StrokeEndpointCap => matches!(
                        control,
                        DesignStrokeEndpointControl::Aggregate
                            | DesignStrokeEndpointControl::SelectedVertices
                    ),
                    _ => false,
                };
                supported.then(|| {
                    DesignStrokeCap::ALL
                        .into_iter()
                        .map(|cap| PropertyOption {
                            label: cap.label().into(),
                            value: DesignPanelValue::StrokeCap(cap),
                        })
                        .collect()
                })
            }
            DesignPanelProperty::StrokeDashMode => self
                .node
                .stroke
                .as_ref()?
                .complex_stroke
                .is_basic()
                .then(|| {
                    DesignStrokeDashMode::ALL
                        .into_iter()
                        .map(|mode| PropertyOption {
                            label: mode.label().into(),
                            value: DesignPanelValue::StrokeDashMode(mode),
                        })
                        .collect()
                }),
            DesignPanelProperty::StrokeDashCap => {
                let stroke = self.node.stroke.as_ref()?;
                (stroke.complex_stroke.is_basic() && !stroke.dashes.is_solid()).then(|| {
                    [
                        DesignStrokeCap::None,
                        DesignStrokeCap::Round,
                        DesignStrokeCap::Square,
                    ]
                    .into_iter()
                    .map(|cap| PropertyOption {
                        label: cap.label().into(),
                        value: DesignPanelValue::StrokeCap(cap),
                    })
                    .collect()
                })
            }
            DesignPanelProperty::StrokeWeightMode => self
                .node
                .stroke
                .as_ref()?
                .capabilities
                .individual_weights
                .then(|| {
                    DesignStrokeWeightMode::ALL
                        .into_iter()
                        .map(|mode| PropertyOption {
                            label: mode.label().into(),
                            value: DesignPanelValue::StrokeWeightMode(mode),
                        })
                        .collect()
                }),
            DesignPanelProperty::StrokeJoin => {
                self.node.stroke.as_ref()?.capabilities.joins.then(|| {
                    DesignStrokeJoin::ALL
                        .into_iter()
                        .map(|join| PropertyOption {
                            label: join.label().into(),
                            value: DesignPanelValue::StrokeJoin(join),
                        })
                        .collect()
                })
            }
            DesignPanelProperty::StrokeVariableWidth => {
                let stroke = self.node.stroke.as_ref()?;
                stroke.supports_variable_width().then(|| {
                    let mut options = vec![PropertyOption {
                        label: "None".into(),
                        value: DesignPanelValue::StrokeVariableWidth(None),
                    }];
                    options.extend(DesignVariableWidthPreset::ALL.into_iter().map(|preset| {
                        PropertyOption {
                            label: preset.label().into(),
                            value: DesignPanelValue::StrokeVariableWidth(Some(
                                DesignVariableWidthStroke::Preset(preset),
                            )),
                        }
                    }));
                    let custom = match stroke.variable_width.as_ref() {
                        Some(custom @ DesignVariableWidthStroke::Custom { .. }) => custom.clone(),
                        _ => DesignVariableWidthStroke::custom([
                            DesignVariableWidthPoint::new(0., 1.),
                            DesignVariableWidthPoint::new(1., 1.),
                        ]),
                    };
                    options.push(PropertyOption {
                        label: "Custom".into(),
                        value: DesignPanelValue::StrokeVariableWidth(Some(custom)),
                    });
                    options
                })
            }
            DesignPanelProperty::StrokeType => self
                .node
                .stroke
                .as_ref()?
                .capabilities
                .complex_stroke
                .then(|| {
                    DesignStrokeType::EDITABLE
                        .into_iter()
                        .map(|kind| PropertyOption {
                            label: kind.label().into(),
                            value: DesignPanelValue::StrokeType(kind),
                        })
                        .collect()
                }),
            DesignPanelProperty::StrokeStretchBrush => matches!(
                &self.node.stroke.as_ref()?.complex_stroke,
                DesignComplexStroke::StretchBrush(_)
            )
            .then(|| {
                DesignStretchBrushName::ALL
                    .into_iter()
                    .map(|brush| PropertyOption {
                        label: brush.label().into(),
                        value: DesignPanelValue::StrokeStretchBrush(brush),
                    })
                    .collect()
            }),
            DesignPanelProperty::StrokeBrushDirection => matches!(
                &self.node.stroke.as_ref()?.complex_stroke,
                DesignComplexStroke::StretchBrush(_)
            )
            .then(|| {
                DesignStrokeBrushDirection::ALL
                    .into_iter()
                    .map(|direction| PropertyOption {
                        label: direction.label().into(),
                        value: DesignPanelValue::StrokeBrushDirection(direction),
                    })
                    .collect()
            }),
            DesignPanelProperty::StrokeScatterBrush => matches!(
                &self.node.stroke.as_ref()?.complex_stroke,
                DesignComplexStroke::ScatterBrush(_)
            )
            .then(|| {
                DesignScatterBrushName::ALL
                    .into_iter()
                    .map(|brush| PropertyOption {
                        label: brush.label().into(),
                        value: DesignPanelValue::StrokeScatterBrush(brush),
                    })
                    .collect()
            }),
            DesignPanelProperty::EffectKind(index) => {
                let current = self.node.effects.get(index)?.settings.kind();
                let mut kinds = DesignEffectKind::ALL
                    .into_iter()
                    .filter(|kind| {
                        *kind == current || self.node.can_use_effect_kind(*kind, Some(index))
                    })
                    .collect::<Vec<_>>();
                if current == DesignEffectKind::Unsupported {
                    kinds.push(current);
                }
                Some(
                    kinds
                        .into_iter()
                        .map(|kind| PropertyOption {
                            label: kind.label().into(),
                            value: DesignPanelValue::EffectKind(kind),
                        })
                        .collect(),
                )
            }
            DesignPanelProperty::LayoutGridKind(_) => Some(
                DesignGridKind::ALL
                    .into_iter()
                    .map(|kind| PropertyOption {
                        label: kind.label().into(),
                        value: DesignPanelValue::GridKind(kind),
                    })
                    .collect(),
            ),
            DesignPanelProperty::LayoutGridAlignment(index) => {
                match &self.node.layout_grids.get(index)?.settings {
                    DesignLayoutGridSettings::Uniform(_) => None,
                    DesignLayoutGridSettings::Columns(_) => Some(
                        DesignColumnGridAlignment::ALL
                            .into_iter()
                            .map(|alignment| PropertyOption {
                                label: alignment.label().into(),
                                value: DesignPanelValue::ColumnGridAlignment(alignment),
                            })
                            .collect(),
                    ),
                    DesignLayoutGridSettings::Rows(_) => Some(
                        DesignRowGridAlignment::ALL
                            .into_iter()
                            .map(|alignment| PropertyOption {
                                label: alignment.label().into(),
                                value: DesignPanelValue::RowGridAlignment(alignment),
                            })
                            .collect(),
                    ),
                }
            }
            DesignPanelProperty::ExportFormat(_) => Some(
                DesignExportFormat::ALL
                    .into_iter()
                    .map(|format| PropertyOption {
                        label: format.label().into(),
                        value: DesignPanelValue::ExportFormat(format),
                    })
                    .collect(),
            ),
            _ => None,
        }
    }

    pub(super) fn property_clamp(property: DesignPanelProperty) -> Option<NumericClamp> {
        let bounds = match property {
            DesignPanelProperty::Opacity
            | DesignPanelProperty::LayoutGridOpacity(_)
            | DesignPanelProperty::PaintOpacity { .. } => (Some(0.), Some(100.)),
            DesignPanelProperty::EffectProgressiveBlurStartOffsetX(_)
            | DesignPanelProperty::EffectProgressiveBlurStartOffsetY(_)
            | DesignPanelProperty::EffectProgressiveBlurEndOffsetX(_)
            | DesignPanelProperty::EffectProgressiveBlurEndOffsetY(_)
            | DesignPanelProperty::EffectNoiseOpacity(_)
            | DesignPanelProperty::EffectNoiseDensity(_)
            | DesignPanelProperty::EffectGlassLightIntensity(_)
            | DesignPanelProperty::EffectGlassRefraction(_)
            | DesignPanelProperty::EffectGlassDispersion(_)
            | DesignPanelProperty::EffectGlassSplay(_) => (Some(0.), Some(1.)),
            DesignPanelProperty::MediaExposure
            | DesignPanelProperty::MediaContrast
            | DesignPanelProperty::MediaSaturation
            | DesignPanelProperty::MediaTemperature
            | DesignPanelProperty::MediaTint
            | DesignPanelProperty::MediaHighlights
            | DesignPanelProperty::MediaShadows => (Some(-100.), Some(100.)),
            DesignPanelProperty::PolygonCount | DesignPanelProperty::StarPointCount => {
                (Some(3.), Some(60.))
            }
            DesignPanelProperty::TransformRepeatCount(_) => (Some(1.), Some(f64::from(u32::MAX))),
            DesignPanelProperty::TableRows
            | DesignPanelProperty::TableColumns
            | DesignPanelProperty::TextMaxLines
            | DesignPanelProperty::GridColumnCount
            | DesignPanelProperty::GridRowCount
            | DesignPanelProperty::GridRowSpan
            | DesignPanelProperty::GridColumnSpan => (Some(1.), None),
            DesignPanelProperty::GridRowIndex | DesignPanelProperty::GridColumnIndex => {
                (Some(1.), None)
            }
            DesignPanelProperty::TextPathStartSegment => (Some(0.), Some(f64::from(u32::MAX))),
            DesignPanelProperty::TextPathStartPosition => (Some(0.), Some(1.)),
            DesignPanelProperty::SlotMinimumInstances(_)
            | DesignPanelProperty::SlotMaximumInstances(_) => (Some(0.), None),
            DesignPanelProperty::Width
            | DesignPanelProperty::Height
            | DesignPanelProperty::VectorVertexCornerRadius
            | DesignPanelProperty::CounterAxisGap
            | DesignPanelProperty::PaddingVertical
            | DesignPanelProperty::PaddingHorizontal
            | DesignPanelProperty::PaddingTop
            | DesignPanelProperty::PaddingRight
            | DesignPanelProperty::PaddingBottom
            | DesignPanelProperty::PaddingLeft
            | DesignPanelProperty::CornerRadius
            | DesignPanelProperty::CornerRadiusTopLeft
            | DesignPanelProperty::CornerRadiusTopRight
            | DesignPanelProperty::CornerRadiusBottomRight
            | DesignPanelProperty::CornerRadiusBottomLeft
            | DesignPanelProperty::StrokeWeight
            | DesignPanelProperty::StrokeWeightTop
            | DesignPanelProperty::StrokeWeightRight
            | DesignPanelProperty::StrokeWeightBottom
            | DesignPanelProperty::StrokeWeightLeft
            | DesignPanelProperty::StrokeVariableWidthPointWidth(_)
            | DesignPanelProperty::StrokeScatterWiggle
            | DesignPanelProperty::EffectBlur(_)
            | DesignPanelProperty::EffectShadowBlur(_)
            | DesignPanelProperty::EffectBlurRadius(_)
            | DesignPanelProperty::EffectProgressiveBlurStartRadius(_)
            | DesignPanelProperty::EffectProgressiveBlurEndRadius(_)
            | DesignPanelProperty::EffectNoiseSizeX(_)
            | DesignPanelProperty::EffectNoiseSizeY(_)
            | DesignPanelProperty::EffectTextureSizeX(_)
            | DesignPanelProperty::EffectTextureSizeY(_)
            | DesignPanelProperty::EffectTextureRadius(_)
            | DesignPanelProperty::EffectGlassFrost(_)
            | DesignPanelProperty::LayoutGridSize(_)
            | DesignPanelProperty::LayoutGridOffset(_)
            | DesignPanelProperty::LayoutGridGutter(_)
            | DesignPanelProperty::LayoutGridMargin(_) => (Some(0.), None),
            DesignPanelProperty::StrokeVariableWidthPointPosition(_) => (Some(0.), Some(1.)),
            DesignPanelProperty::FontWeight => (Some(1.), Some(1000.)),
            DesignPanelProperty::FontSize => (Some(1.), None),
            DesignPanelProperty::ListSpacing => (Some(0.), None),
            DesignPanelProperty::StrokeMiterAngle => (Some(0.), Some(180.)),
            DesignPanelProperty::StrokeScatterGap => (Some(0.25), None),
            DesignPanelProperty::StrokeScatterSizeJitter => (Some(0.), Some(3.)),
            DesignPanelProperty::StrokeScatterAngularJitter
            | DesignPanelProperty::StrokeScatterRotation => (Some(-180.), Some(180.)),
            DesignPanelProperty::StrokeDynamicFrequency => (Some(0.01), Some(20.)),
            DesignPanelProperty::StrokeDynamicWiggle => (Some(0.), None),
            DesignPanelProperty::StrokeDynamicSmoothen => (Some(0.), Some(1.)),
            DesignPanelProperty::MinWidth
            | DesignPanelProperty::MaxWidth
            | DesignPanelProperty::MinHeight
            | DesignPanelProperty::MaxHeight => (Some(1.), None),
            DesignPanelProperty::GridColumnTrackValue(_)
            | DesignPanelProperty::GridRowTrackValue(_) => (Some(0.01), None),
            DesignPanelProperty::LayoutGrow => (Some(0.), Some(1.)),
            DesignPanelProperty::EffectGlassDepth(_) => (Some(1.), None),
            DesignPanelProperty::ExportScale(_) => (Some(0.01), None),
            _ => return None,
        };
        NumericClamp::new(bounds.0, bounds.1).ok()
    }

    pub(super) fn editor_focus_origin_matches_property(
        origin: &EditorFocusOrigin,
        property: DesignPanelProperty,
    ) -> bool {
        matches!(
            origin,
            EditorFocusOrigin::ValueCell(origin_property)
                | EditorFocusOrigin::ShaderField {
                    property: origin_property,
                    ..
                }
                | EditorFocusOrigin::TypeSetting(origin_property)
                if *origin_property == property
        )
    }

    pub(super) fn clear_editor_focus_return_for_property(&mut self, property: DesignPanelProperty) {
        if self
            .editor_focus_return
            .as_ref()
            .is_some_and(|return_focus| {
                Self::editor_focus_origin_matches_property(&return_focus.origin, property)
            })
        {
            self.editor_focus_return = None;
        }
    }

    pub(super) fn rebase_editor_focus_origin(
        &mut self,
        previous: DesignPanelProperty,
        next: DesignPanelProperty,
    ) {
        let Some(return_focus) = self.editor_focus_return.as_mut() else {
            return;
        };
        match &mut return_focus.origin {
            EditorFocusOrigin::ValueCell(property)
            | EditorFocusOrigin::ShaderField { property, .. }
            | EditorFocusOrigin::TypeSetting(property)
                if *property == previous =>
            {
                *property = next;
            }
            _ => {}
        }
    }

    pub(super) fn property_editor_return_focus(
        &self,
        editor: &PropertyEditor,
    ) -> Option<FocusHandle> {
        let return_focus = self.editor_focus_return.as_ref()?;
        let matches = match (&return_focus.origin, editor.kind) {
            (
                EditorFocusOrigin::ShaderField { property, field },
                PropertyEditorKind::Shader {
                    field: active_field,
                    ..
                },
            ) => *property == editor.property && *field == active_field,
            (
                EditorFocusOrigin::ValueCell(property) | EditorFocusOrigin::TypeSetting(property),
                kind,
            ) if !matches!(kind, PropertyEditorKind::Shader { .. }) => *property == editor.property,
            _ => false,
        };
        matches.then(|| return_focus.handle.clone())
    }

    pub(super) fn numeric_scrub_return_focus(
        &self,
        property: DesignPanelProperty,
    ) -> Option<FocusHandle> {
        let return_focus = self.editor_focus_return.as_ref()?;
        matches!(
            return_focus.origin,
            EditorFocusOrigin::ValueCell(origin_property)
                | EditorFocusOrigin::TypeSetting(origin_property)
                if origin_property == property
        )
        .then(|| return_focus.handle.clone())
    }

    pub(super) fn component_multiline_return_focus(
        &self,
        property_id: &str,
    ) -> Option<FocusHandle> {
        let return_focus = self.editor_focus_return.as_ref()?;
        matches!(
            &return_focus.origin,
            EditorFocusOrigin::ComponentMultiline(origin_property_id)
                if origin_property_id.as_ref() == property_id
        )
        .then(|| return_focus.handle.clone())
    }

    pub(super) fn defer_editor_focus(
        handle: FocusHandle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.defer(cx, move |window, cx| {
            handle.focus(window, cx);
        });
    }

    pub(super) fn capture_numeric_scrub_focus(
        &mut self,
        origin: EditorFocusOrigin,
        property: DesignPanelProperty,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !Self::editor_focus_origin_matches_property(&origin, property)
            || !self
                .numeric_property_scrub
                .as_ref()
                .is_some_and(|scrub| scrub.property == property)
        {
            return;
        }
        let handle = cx.focus_handle();
        handle.focus(window, cx);
        self.editor_focus_return = Some(EditorFocusReturn { origin, handle });
        cx.notify();
    }

    pub(super) fn text_property_is_editable(&self, property: DesignPanelProperty) -> bool {
        match property {
            DesignPanelProperty::FontFamily | DesignPanelProperty::ExportSuffix(_) => true,
            DesignPanelProperty::ComponentProperty(index) => self
                .node
                .component_properties
                .get(index)
                .is_some_and(|property| {
                    matches!(
                        property.definition,
                        DesignComponentPropertyDefinition::Text { .. }
                    )
                }),
            _ => false,
        }
    }

    pub(super) fn activate_property_from_control(
        &mut self,
        origin: EditorFocusOrigin,
        property: DesignPanelProperty,
        fallback: DesignPanelValue,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !Self::editor_focus_origin_matches_property(&origin, property)
            || self
                .property_editor
                .as_ref()
                .is_some_and(|editor| editor.property == property)
        {
            return;
        }
        let return_focus = window
            .focused(cx)
            .map(|handle| EditorFocusReturn { origin, handle });
        self.activate_property(property, fallback, window, cx);
        if self
            .property_editor
            .as_ref()
            .is_some_and(|editor| editor.property == property)
        {
            self.editor_focus_return = return_focus;
        }
    }

    pub(super) fn activate_property(
        &mut self,
        property: DesignPanelProperty,
        fallback: DesignPanelValue,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.property_is_editable(property) {
            return;
        }
        if self.suppress_next_control_activation {
            self.suppress_next_control_activation = false;
            return;
        }
        if self
            .property_editor
            .as_ref()
            .is_some_and(|editor| editor.property == property)
        {
            return;
        }
        let non_uniform = self
            .property_value_states
            .get(&property)
            .is_some_and(|state| state.is_mixed() || state.is_unset())
            || self.vector_property_is_mixed(property)
            || self.smart_selection_property_is_mixed(property);
        let Some(current) = self.resolved_property_value(property).or_else(|| {
            non_uniform
                .then(|| self.current_property_value(property))
                .flatten()
        }) else {
            self.emit_property(property, fallback, cx);
            return;
        };
        let (kind, base, mut draft) = match &current {
            DesignPanelValue::Number(value) => (
                PropertyEditorKind::Number {
                    integer: false,
                    clamp: Self::property_clamp(property),
                },
                f64::from(*value),
                format_number(*value),
            ),
            DesignPanelValue::AngleRadians(value) => {
                let degrees = value.to_degrees();
                (
                    PropertyEditorKind::AngleDegrees,
                    f64::from(degrees),
                    format_number(degrees),
                )
            }
            DesignPanelValue::Ratio(value) => {
                let percentage = value * 100.;
                (
                    PropertyEditorKind::PercentageRatio,
                    f64::from(percentage),
                    format_number(percentage),
                )
            }
            DesignPanelValue::Integer(value) => (
                PropertyEditorKind::Number {
                    integer: true,
                    clamp: Self::property_clamp(property),
                },
                *value as f64,
                value.to_string(),
            ),
            DesignPanelValue::OptionalNumber(value) => (
                PropertyEditorKind::OptionalNumber {
                    clamp: Self::property_clamp(property),
                },
                value.map_or(0., f64::from),
                value.map_or_else(String::new, format_number),
            ),
            DesignPanelValue::LayoutGridCount(count) => (
                PropertyEditorKind::LayoutGridCount,
                f64::from(count.numeric().unwrap_or(1)),
                count.label().to_string(),
            ),
            DesignPanelValue::Color(color) => {
                (PropertyEditorKind::Color, 0., color.hex().to_string())
            }
            DesignPanelValue::Text(value) if self.text_property_is_editable(property) => {
                (PropertyEditorKind::Text, 0., value.to_string())
            }
            DesignPanelValue::NumberList(values) => (
                PropertyEditorKind::NumberList,
                0.,
                values
                    .iter()
                    .map(|value| format_number(*value))
                    .collect::<Vec<_>>()
                    .join(", "),
            ),
            DesignPanelValue::ExportSizing(sizing) => (
                PropertyEditorKind::ExportSizing,
                f64::from(sizing.value()),
                sizing.to_string(),
            ),
            _ => {
                self.emit_property(property, fallback, cx);
                return;
            }
        };
        if non_uniform {
            draft.clear();
        }

        if let Some((property_id, _)) = self.component_swap_hovered.take() {
            cx.emit_design_panel_action(
                self,
                DesignPanelAction::ComponentSwapPreviewRequested {
                    node_id: self.node.id.clone(),
                    property_id,
                    selection: None,
                },
            );
        }
        self.cancel_menu_preview(cx);
        self.active_picker = None;
        self.active_effect_settings = None;
        self.grid_dimensions_picker = None;
        self.effect_style_browser_open = false;
        self.property_variable_picker = None;
        self.component_property_variable_picker = None;
        self.component_swap_browser = None;
        self.type_settings_open = false;
        self.vector_edit_target_ids = Self::vector_property_is_contextual(property)
            .then(|| {
                self.active_vector_edit()
                    .map(DesignVectorEditViewData::selected_vertex_ids)
            })
            .flatten();
        let layout_grid_target = property
            .layout_grid_index()
            .and_then(|index| self.layout_grid_target_for_index(index));
        let export_configuration_id = Self::export_property_index(property)
            .and_then(|index| self.export_configuration(index))
            .map(|configuration| configuration.id);
        self.editor_focus_return = None;
        self.property_editor = Some(PropertyEditor {
            property,
            layout_grid_target,
            export_configuration_id,
            original: current.clone(),
            last_preview: None,
            base,
            kind,
        });
        self.property_editor_invalid = false;
        self.emit_property_edit(property, current, DesignPanelEditPhase::Begin, cx);
        self.suppress_property_input_change = true;
        self.property_input.update(cx, |input, cx| {
            input.set_value(draft, window, cx);
            input.focus(window, cx);
        });
        self.suppress_property_input_change = false;
        cx.notify();
    }

    pub(super) fn parsed_property_draft(&self, cx: &App) -> Option<Result<DesignPanelValue, ()>> {
        let editor = self.property_editor.as_ref()?;
        let draft = self.property_input.read(cx).value();
        Some(match editor.kind {
            PropertyEditorKind::Shader { field, input } => {
                let DesignPanelValue::ShaderProperty(original) = &editor.original else {
                    return Some(Err(()));
                };
                shader_property_value_from_draft(
                    original,
                    field,
                    input,
                    draft.as_ref(),
                    editor.base,
                )
                .map(DesignPanelValue::ShaderProperty)
            }
            PropertyEditorKind::LayoutGridCount if draft.trim().eq_ignore_ascii_case("auto") => Ok(
                DesignPanelValue::LayoutGridCount(DesignLayoutGridCount::Auto),
            ),
            PropertyEditorKind::LayoutGridCount => evaluate_numeric_expression(
                draft.as_ref(),
                editor.base,
                NumericClamp::new(Some(1.), Some(f64::from(u16::MAX))).ok(),
            )
            .and_then(round_to_integer)
            .map_err(|_| ())
            .and_then(|value| {
                u16::try_from(value as i64)
                    .map(DesignLayoutGridCount::number)
                    .map(DesignPanelValue::LayoutGridCount)
                    .map_err(|_| ())
            }),
            PropertyEditorKind::Color => parse_design_color(draft.as_ref())
                .map(DesignPanelValue::Color)
                .ok_or(()),
            PropertyEditorKind::Text => {
                if editor.property == DesignPanelProperty::FontFamily && draft.trim().is_empty() {
                    Err(())
                } else {
                    Ok(DesignPanelValue::Text(draft))
                }
            }
            PropertyEditorKind::NumberList => draft
                .split(|character: char| character == ',' || character.is_whitespace())
                .filter(|item| !item.is_empty())
                .map(|item| {
                    parse_decorated_number(item)
                        .map_err(|_| ())
                        .and_then(|value| {
                            (value.value >= 0. && value.value <= f64::from(f32::MAX))
                                .then_some(value.value as f32)
                                .ok_or(())
                        })
                })
                .collect::<Result<Vec<_>, _>>()
                .map(DesignPanelValue::NumberList),
            PropertyEditorKind::ExportSizing => DesignExportSizing::parse(draft.as_ref())
                .map(DesignPanelValue::ExportSizing)
                .map_err(|_| ()),
            PropertyEditorKind::AngleDegrees => {
                evaluate_numeric_expression(draft.as_ref(), editor.base, None)
                    .map_err(|_| ())
                    .and_then(|degrees| {
                        (degrees >= f64::from(f32::MIN) && degrees <= f64::from(f32::MAX))
                            .then_some(DesignPanelValue::AngleRadians(
                                (degrees as f32).to_radians(),
                            ))
                            .ok_or(())
                    })
            }
            PropertyEditorKind::PercentageRatio => evaluate_numeric_expression(
                draft.as_ref(),
                editor.base,
                NumericClamp::new(Some(0.), Some(100.)).ok(),
            )
            .map(|percentage| DesignPanelValue::Ratio(percentage as f32 / 100.))
            .map_err(|_| ()),
            PropertyEditorKind::OptionalNumber { clamp: _ } if draft.trim().is_empty() => {
                Ok(DesignPanelValue::OptionalNumber(None))
            }
            PropertyEditorKind::OptionalNumber { clamp } => {
                evaluate_numeric_expression(draft.as_ref(), editor.base, clamp)
                    .and_then(|value| {
                        if editor.property == DesignPanelProperty::TextMaxLines {
                            round_to_integer(value)
                        } else {
                            Ok(value)
                        }
                    })
                    .map_err(|_| ())
                    .and_then(|value| {
                        if value < f64::from(f32::MIN) || value > f64::from(f32::MAX) {
                            Err(())
                        } else {
                            Ok(DesignPanelValue::OptionalNumber(Some(value as f32)))
                        }
                    })
            }
            PropertyEditorKind::Number { integer, clamp } => {
                evaluate_numeric_expression(draft.as_ref(), editor.base, clamp)
                    .and_then(|value| {
                        if integer {
                            round_to_integer(value)
                        } else {
                            Ok(value)
                        }
                    })
                    .map_err(|_| ())
                    .and_then(|value| {
                        if integer {
                            if value < i64::MIN as f64 || value > i64::MAX as f64 {
                                Err(())
                            } else {
                                Ok(DesignPanelValue::Integer(value as i64))
                            }
                        } else if value < f64::from(f32::MIN) || value > f64::from(f32::MAX) {
                            Err(())
                        } else {
                            Ok(DesignPanelValue::Number(value as f32))
                        }
                    })
            }
        })
    }

    pub(super) fn validate_property_draft(&mut self, cx: &mut Context<Self>) {
        let parsed = self.parsed_property_draft(cx);
        let property = self.property_editor.as_ref().map(|editor| editor.property);
        self.property_editor_invalid = parsed.as_ref().is_some_and(|result| match result {
            Err(()) => true,
            Ok(value) => property
                .is_some_and(|property| !self.layout_property_value_is_applicable(property, value)),
        });
        if !self.suppress_property_input_change
            && !self.property_editor_invalid
            && let Some(Ok(value)) = parsed
        {
            let should_preview = self.property_editor.as_ref().is_some_and(|editor| {
                !(editor.last_preview.is_none() && editor.original == value)
                    && editor.last_preview.as_ref() != Some(&value)
            });
            if should_preview {
                let property = self
                    .property_editor
                    .as_ref()
                    .expect("preview requires an active property editor")
                    .property;
                if let Some(editor) = self.property_editor.as_mut() {
                    editor.last_preview = Some(value.clone());
                }
                self.emit_property_edit(property, value, DesignPanelEditPhase::Preview, cx);
            }
        }
        cx.notify();
    }

    pub(super) fn finish_property_edit(
        &mut self,
        commit: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.finish_property_edit_with_focus_restore(commit, true, window, cx);
    }

    pub(super) fn finish_property_edit_after_input_blur(
        &mut self,
        commit: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.finish_property_edit_with_focus_restore(commit, false, window, cx);
    }

    pub(super) fn finish_property_edit_with_focus_restore(
        &mut self,
        commit: bool,
        restore_focus: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self
            .numeric_property_scrub
            .as_ref()
            .is_some_and(|scrub| scrub.active)
        {
            self.finish_numeric_property_scrub_with_focus_restore(
                commit,
                false,
                restore_focus,
                window,
                cx,
            );
            return;
        }
        let Some(editor) = self.property_editor.clone() else {
            return;
        };
        let type_settings_was_open = self.type_settings_open;
        let return_focus = self.property_editor_return_focus(&editor);
        let value = commit
            .then(|| self.parsed_property_draft(cx))
            .flatten()
            .and_then(Result::ok)
            .filter(|value| self.layout_property_value_is_applicable(editor.property, value));
        self.property_editor = None;
        self.numeric_property_scrub = None;
        self.property_editor_invalid = false;
        self.suppress_property_input_change = false;
        let (phase, value) = value
            .map_or((DesignPanelEditPhase::Cancel, editor.original), |value| {
                (DesignPanelEditPhase::Commit, value)
            });
        self.emit_property_edit(editor.property, value, phase, cx);
        self.vector_edit_target_ids = None;
        if restore_focus {
            let fallback = if type_settings_was_open {
                self.type_settings_focus.clone()
            } else {
                self.focus_handle.clone()
            };
            Self::defer_editor_focus(return_focus.unwrap_or(fallback), window, cx);
        }
        if self.suppress_next_control_activation {
            cx.defer_in(window, |this, _, _| {
                this.suppress_next_control_activation = false;
            });
        }
        cx.notify();
    }

    pub(super) fn property_editor_survives(&self, editor: &PropertyEditor) -> bool {
        let Some(active) = self.property_editor.as_ref() else {
            return false;
        };
        if active.property != editor.property
            || active.kind != editor.kind
            || active.layout_grid_target != editor.layout_grid_target
            || active.export_configuration_id != editor.export_configuration_id
            || !self.property_is_editable(editor.property)
            || self.resolved_property_value(editor.property).is_none()
        {
            return false;
        }
        if let Some(target) = editor.layout_grid_target.as_ref()
            && self.layout_grid_target_index(target) != editor.property.layout_grid_index()
        {
            return false;
        }
        if let Some(configuration_id) = editor.export_configuration_id.as_ref() {
            let Some(index) = Self::export_property_index(editor.property) else {
                return false;
            };
            if self
                .export_configuration(index)
                .is_none_or(|configuration| configuration.id != *configuration_id)
            {
                return false;
            }
        }
        if let PropertyEditorKind::Shader { field, .. } = editor.kind {
            let Some(DesignPanelValue::ShaderProperty(value)) =
                self.resolved_property_value(editor.property)
            else {
                return false;
            };
            if shader_property_field_is_bound(&value, field) {
                return false;
            }
        }
        true
    }

    /// Steps an exact, valid scalar numeric draft and reports whether the key
    /// was handled. Invalid/blank/Auto/None and nonnumeric drafts deliberately
    /// propagate so the surrounding input keeps its native keyboard behavior.
    pub(super) fn step_property_editor(
        &mut self,
        direction: ArrowStep,
        shift: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(editor) = self.property_editor.clone() else {
            return false;
        };
        if !self.property_editor_survives(&editor) {
            return false;
        }
        let nudge = f64::from(self.nudge_settings.amount(shift));
        if editor.kind == PropertyEditorKind::ExportSizing {
            let Some(Ok(DesignPanelValue::ExportSizing(sizing))) = self.parsed_property_draft(cx)
            else {
                return false;
            };
            let DesignPanelValue::ExportSizing(original) = editor.original else {
                return false;
            };
            if !matches!(
                (original, sizing),
                (DesignExportSizing::Scale(_), DesignExportSizing::Scale(_))
                    | (DesignExportSizing::Width(_), DesignExportSizing::Width(_))
                    | (DesignExportSizing::Height(_), DesignExportSizing::Height(_))
            ) {
                return false;
            }
            let Ok(value) = step_with_arrow_amount(
                f64::from(sizing.value()),
                direction,
                nudge,
                NumericClamp::new(Some(0.01), None).ok(),
            ) else {
                return false;
            };
            let stepped = match sizing {
                DesignExportSizing::Scale(_) => DesignExportSizing::Scale(value as f32),
                DesignExportSizing::Width(_) => DesignExportSizing::Width(value as f32),
                DesignExportSizing::Height(_) => DesignExportSizing::Height(value as f32),
            };
            if !self.layout_property_value_is_applicable(
                editor.property,
                &DesignPanelValue::ExportSizing(stepped),
            ) {
                return false;
            }
            let draft = format!(
                "{}{}",
                format_nudge_number(stepped.value()),
                stepped.suffix()
            );
            self.property_input.update(cx, |input, cx| {
                input.set_value(draft, window, cx);
            });
            return true;
        }
        if editor.kind == PropertyEditorKind::LayoutGridCount {
            let Some(Ok(DesignPanelValue::LayoutGridCount(DesignLayoutGridCount::Number(current)))) =
                self.parsed_property_draft(cx)
            else {
                return false;
            };
            let Ok(value) = step_with_arrow_amount(
                f64::from(current),
                direction,
                nudge,
                NumericClamp::new(Some(1.), Some(f64::from(u16::MAX))).ok(),
            ) else {
                return false;
            };
            let Ok(value) = round_to_integer(value) else {
                return false;
            };
            let Ok(value) = u16::try_from(value as i64) else {
                return false;
            };
            if !self.layout_property_value_is_applicable(
                editor.property,
                &DesignPanelValue::LayoutGridCount(DesignLayoutGridCount::number(value)),
            ) {
                return false;
            }
            self.property_input.update(cx, |input, cx| {
                input.set_value(value.to_string(), window, cx);
            });
            return true;
        }
        if let PropertyEditorKind::Shader {
            input: ShaderPropertyEditorInput::Number { clamp },
            ..
        } = editor.kind
        {
            let draft = self.property_input.read(cx).value();
            let Ok(current) = evaluate_numeric_expression(draft.as_ref(), editor.base, clamp)
            else {
                return false;
            };
            let Ok(value) = step_with_arrow_amount(current, direction, nudge, clamp) else {
                return false;
            };
            let draft = format_nudge_number(value as f32);
            let PropertyEditorKind::Shader { field, input } = editor.kind else {
                unreachable!("numeric shader branch preserves its editor kind");
            };
            let DesignPanelValue::ShaderProperty(original) = &editor.original else {
                return false;
            };
            let Ok(candidate) = shader_property_value_from_draft(
                original,
                field,
                input,
                draft.as_str(),
                editor.base,
            ) else {
                return false;
            };
            if !self.layout_property_value_is_applicable(
                editor.property,
                &DesignPanelValue::ShaderProperty(candidate),
            ) {
                return false;
            }
            self.property_input.update(cx, |input, cx| {
                input.set_value(draft, window, cx);
            });
            return true;
        }
        let (integer, clamp) = match editor.kind {
            PropertyEditorKind::Number { integer, clamp } => (integer, clamp),
            PropertyEditorKind::OptionalNumber { clamp } => {
                (editor.property == DesignPanelProperty::TextMaxLines, clamp)
            }
            PropertyEditorKind::AngleDegrees => (false, None),
            PropertyEditorKind::PercentageRatio => {
                (false, NumericClamp::new(Some(0.), Some(100.)).ok())
            }
            PropertyEditorKind::Text
            | PropertyEditorKind::LayoutGridCount
            | PropertyEditorKind::Color
            | PropertyEditorKind::NumberList
            | PropertyEditorKind::ExportSizing
            | PropertyEditorKind::Shader { .. } => return false,
        };
        let Some(Ok(parsed)) = self.parsed_property_draft(cx) else {
            return false;
        };
        let current = match (editor.kind, parsed) {
            (
                PropertyEditorKind::Number { integer: false, .. },
                DesignPanelValue::Number(value),
            ) => f64::from(value),
            (
                PropertyEditorKind::Number { integer: true, .. },
                DesignPanelValue::Integer(value),
            ) => value as f64,
            (
                PropertyEditorKind::OptionalNumber { .. },
                DesignPanelValue::OptionalNumber(Some(value)),
            ) => f64::from(value),
            (PropertyEditorKind::AngleDegrees, DesignPanelValue::AngleRadians(value)) => {
                f64::from(value.to_degrees())
            }
            (PropertyEditorKind::PercentageRatio, DesignPanelValue::Ratio(value)) => {
                f64::from(value * 100.)
            }
            _ => return false,
        };
        let Ok(mut value) = step_with_arrow_amount(current, direction, nudge, clamp) else {
            return false;
        };
        if integer {
            let Ok(rounded) = round_to_integer(value) else {
                return false;
            };
            value = rounded;
        }
        let candidate = match editor.kind {
            PropertyEditorKind::Number { integer: true, .. } => {
                DesignPanelValue::Integer(value as i64)
            }
            PropertyEditorKind::Number { integer: false, .. } => {
                DesignPanelValue::Number(value as f32)
            }
            PropertyEditorKind::OptionalNumber { .. } => {
                DesignPanelValue::OptionalNumber(Some(value as f32))
            }
            PropertyEditorKind::AngleDegrees => {
                DesignPanelValue::AngleRadians((value as f32).to_radians())
            }
            PropertyEditorKind::PercentageRatio => DesignPanelValue::Ratio(value as f32 / 100.),
            _ => unreachable!("scalar branch exhausted all other editor kinds"),
        };
        if !self.layout_property_value_is_applicable(editor.property, &candidate) {
            return false;
        }
        let draft = if integer {
            format!("{}", value as i64)
        } else {
            format_nudge_number(value as f32)
        };
        self.property_input.update(cx, |input, cx| {
            input.set_value(draft, window, cx);
        });
        true
    }

    pub(super) fn handle_property_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.component_authoring_name_editor.is_some() {
            if event.keystroke.key.as_str() == "escape" {
                window.prevent_default();
                cx.stop_propagation();
                self.finish_component_authoring_name_edit(false, window, cx);
            }
            return;
        }
        if self.component_property_reorder.is_some() {
            if event.keystroke.key.as_str() == "escape" {
                window.prevent_default();
                cx.stop_propagation();
                self.finish_component_property_reorder(false, cx);
            }
            return;
        }
        if self.component_variant_option_reorder.is_some() {
            if event.keystroke.key.as_str() == "escape" {
                window.prevent_default();
                cx.stop_propagation();
                self.finish_component_variant_option_reorder(false, cx);
            }
            return;
        }
        if self.component_property_create_draft.is_some() {
            if event.keystroke.key.as_str() == "escape" {
                window.prevent_default();
                cx.stop_propagation();
                self.cancel_component_property_create(window, cx);
            }
            return;
        }
        if matches!(event.keystroke.key.as_str(), "delete" | "backspace")
            && let Some(property_id) = self.component_property_selected.clone()
        {
            window.prevent_default();
            cx.stop_propagation();
            self.request_component_property_delete(property_id, cx);
            return;
        }
        if self.component_multiline_editor.is_some() {
            if event.keystroke.key.as_str() == "escape" {
                window.prevent_default();
                cx.stop_propagation();
                self.finish_component_multiline_editor(false, window, cx);
            }
            return;
        }
        if self.variable_font_axis_editor.is_some() {
            let modifiers = event.keystroke.modifiers;
            match event.keystroke.key.as_str() {
                "escape" => {
                    window.prevent_default();
                    cx.stop_propagation();
                    self.finish_variable_font_axis_edit(false, window, cx);
                }
                key @ ("up" | "down")
                    if !(modifiers.control || modifiers.platform || modifiers.function) =>
                {
                    let direction = if key == "up" {
                        ArrowStep::Increase
                    } else {
                        ArrowStep::Decrease
                    };
                    if self.step_variable_font_axis_input(direction, modifiers, window, cx) {
                        window.prevent_default();
                        cx.stop_propagation();
                    }
                }
                _ => {}
            }
            return;
        }
        if self.property_editor.is_none() {
            return;
        }
        let modifiers = event.keystroke.modifiers;
        match event.keystroke.key.as_str() {
            "escape" => {
                window.prevent_default();
                cx.stop_propagation();
                self.finish_property_edit(false, window, cx);
            }
            key @ ("up" | "down")
                if !(modifiers.control
                    || modifiers.alt
                    || modifiers.platform
                    || modifiers.function) =>
            {
                let direction = if key == "up" {
                    ArrowStep::Increase
                } else {
                    ArrowStep::Decrease
                };
                if self.step_property_editor(direction, modifiers.shift, window, cx) {
                    window.prevent_default();
                    cx.stop_propagation();
                }
            }
            _ => {}
        }
    }
}
