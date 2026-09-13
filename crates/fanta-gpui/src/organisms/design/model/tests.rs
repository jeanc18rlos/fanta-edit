use super::*;

#[test]
fn draw_slider_ranges_validate_and_snap_without_inventing_a_corner_maximum() {
    assert!(DesignDrawSliderRange::new(f32::NAN, 100., 1.).is_none());
    assert!(DesignDrawSliderRange::new(0., f32::INFINITY, 1.).is_none());
    assert!(DesignDrawSliderRange::new(100., 100., 1.).is_none());
    assert!(DesignDrawSliderRange::new(100., 0., 1.).is_none());
    assert!(DesignDrawSliderRange::new(0., 100., 0.).is_none());

    let range = DesignDrawSliderRange::new(10., 70., 2.5).expect("valid Draw range");
    assert_eq!(range.min(), 10.);
    assert_eq!(range.max(), 70.);
    assert_eq!(range.step(), 2.5);
    assert_eq!(range.clamp_and_snap(-50.), 10.);
    assert_eq!(range.clamp_and_snap(16.4), 17.5);
    assert_eq!(range.clamp_and_snap(500.), 70.);
    assert_eq!(range.clamp_and_snap(f32::NAN), 10.);
}

#[test]
fn draw_appearance_view_data_requires_one_exact_nonempty_node_target() {
    let range = DesignDrawSliderRange::new(0., 240., 1.).expect("valid Draw range");
    assert!(
        DesignDrawAppearanceViewData::new(
            DesignPanelTarget::Nodes {
                node_ids: vec!["frame".into(), "shape".into()],
            },
            range,
        )
        .is_valid()
    );
    for target in [
        DesignPanelTarget::Page {
            page_id: "page".into(),
        },
        DesignPanelTarget::Nodes {
            node_ids: Vec::new(),
        },
        DesignPanelTarget::Nodes {
            node_ids: vec!["frame".into(), "frame".into()],
        },
        DesignPanelTarget::Nodes {
            node_ids: vec![" ".into()],
        },
    ] {
        assert!(!DesignDrawAppearanceViewData::new(target, range).is_valid());
    }
}

#[allow(deprecated)]
#[test]
fn compatibility_only_paths_are_classified_with_canonical_replacements() {
    for property in [
        DesignPanelProperty::AlignmentX,
        DesignPanelProperty::AlignmentY,
        DesignPanelProperty::EffectSettings(0),
        DesignPanelProperty::AlignSelection,
        DesignPanelProperty::DistributeSelection,
        DesignPanelProperty::MediaCropMode,
        DesignPanelProperty::MediaExposure,
        DesignPanelProperty::MediaContrast,
        DesignPanelProperty::MediaSaturation,
        DesignPanelProperty::MediaTemperature,
        DesignPanelProperty::MediaTint,
        DesignPanelProperty::MediaHighlights,
        DesignPanelProperty::MediaShadows,
        DesignPanelProperty::EffectBlur(0),
        DesignPanelProperty::EffectSpread(0),
        DesignPanelProperty::EffectOffsetX(0),
        DesignPanelProperty::EffectOffsetY(0),
        DesignPanelProperty::ExportScale(0),
    ] {
        let path = property
            .compatibility_path()
            .expect("every compatibility-only property must be classified");
        assert!(!path.replacement().is_empty());
    }
    assert_eq!(
        DesignPanelProperty::Width.compatibility_path(),
        None,
        "canonical properties must remain distinguishable"
    );

    let whole_paint = DesignPanelAction::PaintChangeRequested {
        node_id: "node".into(),
        collection: DesignPanelCollection::Fill,
        target: DesignPaintTarget::WholeLayer,
        index: 0,
        paint: DesignPaint::solid(DesignColor::BLACK),
    };
    let conflated_color_style = DesignPanelAction::PaintColorStyleApplyRequested {
        node_id: "node".into(),
        collection: DesignPanelCollection::Fill,
        target: DesignPaintTarget::WholeLayer,
        paint_id: "fill".into(),
        index: 0,
        color_target: DesignPaintColorTarget::Solid,
        style: DesignColorStyleSelection::page("legacy-style"),
    };
    let replace_media = DesignPanelAction::ReplaceMediaRequested {
        node_id: "node".into(),
    };
    let export = DesignPanelAction::ExportRequested {
        node_id: "node".into(),
        index: 0,
    };
    let indexed_guide_edit = DesignPanelAction::PropertyEditRequested {
        node_id: "node".into(),
        property: DesignPanelProperty::LayoutGridSize(0),
        value: DesignPanelValue::Number(8.),
        phase: DesignPanelEditPhase::Preview,
    };
    let indexed_guide_remove = DesignPanelAction::CollectionItemRemoveRequested {
        node_id: "node".into(),
        collection: DesignPanelCollection::LayoutGrid,
        target: DesignPaintTarget::WholeLayer,
        index: 0,
    };
    assert_eq!(
        whole_paint.compatibility_path(),
        Some(DesignPanelCompatibilityPath::WholePaintAction)
    );
    assert_eq!(
        conflated_color_style.compatibility_path(),
        Some(DesignPanelCompatibilityPath::ConflatedColorStyleAction)
    );
    assert_eq!(
        replace_media.compatibility_path(),
        Some(DesignPanelCompatibilityPath::NodeMediaReplaceAction)
    );
    assert_eq!(
        export.compatibility_path(),
        Some(DesignPanelCompatibilityPath::SingleExportAction)
    );
    assert_eq!(
        indexed_guide_edit.compatibility_path(),
        Some(DesignPanelCompatibilityPath::IndexedLayoutGridPropertyAction)
    );
    assert_eq!(
        indexed_guide_remove.compatibility_path(),
        Some(DesignPanelCompatibilityPath::IndexedLayoutGridRemoveAction)
    );
    assert_eq!(
        DesignPanelAction::PropertyChangeRequested {
            node_id: "node".into(),
            property: DesignPanelProperty::Width,
            value: DesignPanelValue::Number(10.),
        }
        .compatibility_path(),
        None
    );
}

#[test]
fn color_style_view_data_preserves_stable_page_and_library_identity() {
    let local = DesignColorStyle::new("local-brand", "Brand", DesignColor::PURPLE).with_binding(
        DesignPaintBinding::new("local-variable", "Brand").with_collection("Local colors"),
    );
    let library_style =
        DesignColorStyle::new("remote-brand", "Brand / Primary", DesignColor::BLACK).with_binding(
            DesignPaintBinding::new("remote-variable", "Brand / Primary").with_collection("Acme"),
        );
    let view_data = DesignColorStyleViewData::new(
        [local.clone()],
        [DesignColorStyleLibrary::new(
            "acme-library",
            "Acme",
            [library_style.clone()],
        )],
    );

    assert_eq!(view_data.page_styles, vec![local]);
    assert_eq!(view_data.libraries[0].id.as_ref(), "acme-library");
    assert_eq!(view_data.libraries[0].styles, vec![library_style]);
    assert_eq!(
        DesignColorStyleSelection::page("local-brand"),
        DesignColorStyleSelection {
            source: DesignColorStyleSource::Page,
            style_id: "local-brand".into(),
        }
    );
    assert_eq!(
        DesignColorStyleSelection::library("acme-library", "remote-brand"),
        DesignColorStyleSelection {
            source: DesignColorStyleSource::Library {
                library_id: "acme-library".into(),
            },
            style_id: "remote-brand".into(),
        }
    );
}

#[test]
fn color_style_samples_are_leaf_only_and_preserve_stable_source_identity() {
    let local = DesignColorStyleSample::new("local-brand", "Brand", DesignColor::PURPLE);
    let remote = DesignColorStyleSample::new(
        "remote-brand",
        "Brand / Primary",
        DesignColor::rgba(0x0d, 0x99, 0xff, 0x80),
    );
    let disabled =
        DesignColorStyleSample::new("remote-old", "Old", DesignColor::BLACK).disabled("Deprecated");
    let view_data = DesignColorStyleSampleViewData::new(
        [local.clone()],
        [DesignColorStyleSampleLibrary::new(
            "acme-library",
            "Acme",
            [remote.clone(), disabled],
        )],
    );
    let local_selection = DesignColorStyleSampleSelection::page("local-brand");
    let remote_selection = DesignColorStyleSampleSelection::library("acme-library", "remote-brand");

    assert_eq!(view_data.sample(&local_selection), Some(&local));
    assert_eq!(view_data.sample(&remote_selection), Some(&remote));
    assert_eq!(
        remote_selection.source,
        DesignColorStyleSampleSource::Library {
            library_id: "acme-library".into(),
        }
    );
    let action = DesignPanelAction::PaintColorStyleSampleRequested {
        node_id: "node".into(),
        collection: DesignPanelCollection::Fill,
        target: DesignPaintTarget::WholeLayer,
        paint_id: "paint".into(),
        index: 0,
        color_target: DesignPaintColorTarget::Solid,
        sample: local_selection,
    };
    assert_eq!(
        action.compatibility_path(),
        None,
        "sample-only color styles are a canonical leaf action, not the deprecated conflated path"
    );
}

#[test]
fn layout_grid_style_view_data_preserves_atomic_guide_snapshots_and_sources() {
    let page_guide = DesignLayoutGrid::uniform(8., DesignColor::BLUE).with_id("page-grid");
    let library_guides = [
        DesignLayoutGrid::columns(DesignColumnLayoutGrid::default(), DesignColor::PURPLE)
            .with_id("library-columns"),
        DesignLayoutGrid::rows(DesignRowLayoutGrid::default(), DesignColor::BLUE)
            .with_id("library-rows"),
    ];
    let page_style = DesignLayoutGridStyle::new("page-style", "8 px grid", [page_guide.clone()]);
    let library_style =
        DesignLayoutGridStyle::new("responsive", "Responsive", library_guides.clone()).available();
    let view_data = DesignLayoutGridStyleViewData::new(
        [page_style.clone()],
        [DesignLayoutGridStyleLibrary::new(
            "acme-grid-library",
            "Acme grids",
            [library_style.clone()],
        )],
    );

    assert_eq!(
        view_data.style(&DesignLayoutGridStyleSelection::page("page-style")),
        Some(&page_style)
    );
    assert_eq!(
        view_data.style(&DesignLayoutGridStyleSelection::library(
            "acme-grid-library",
            "responsive"
        )),
        Some(&library_style)
    );
    assert_eq!(library_style.layout_grids, library_guides);
    assert_eq!(
        library_style.import_state,
        DesignLayoutGridStyleImportState::Available
    );
}

#[test]
fn layout_grid_count_variables_preserve_number_auto_compatibility_and_reasons() {
    let numeric = DesignLayoutGridCountVariable::new(
        "columns-12",
        "Columns / 12",
        "Breakpoints",
        DesignLayoutGridCount::number(12),
    );
    let automatic = DesignLayoutGridCountVariable::new(
        "columns-auto",
        "Columns / Auto",
        "Breakpoints",
        DesignLayoutGridCount::Auto,
    );
    let unavailable = numeric
        .clone()
        .disabled("Not available in the active variable mode");

    assert!(numeric.is_compatible(DesignLayoutGridCount::number(4)));
    assert!(!numeric.is_compatible(DesignLayoutGridCount::Auto));
    assert!(automatic.is_compatible(DesignLayoutGridCount::Auto));
    assert_eq!(
        automatic.compatibility_reason(DesignLayoutGridCount::number(4)),
        Some("This guide requires a numeric count variable".into())
    );
    assert_eq!(
        unavailable.compatibility_reason(DesignLayoutGridCount::number(4)),
        Some("Not available in the active variable mode".into())
    );
}

#[test]
fn layout_grid_number_variables_preserve_exact_fields_states_and_values() {
    let size_variable = DesignVariable::page(
        "spacing-8",
        "Spacing / 8",
        "spacing",
        "Spacing",
        DesignVariableResolvedType::Float,
    )
    .with_resolved_value(DesignVariableResolvedValue::Float(8.));
    let auto_variable = DesignVariable::page(
        "count-auto",
        "Count / Auto",
        "responsive",
        "Responsive",
        DesignVariableResolvedType::Float,
    )
    .with_resolved_value(DesignVariableResolvedValue::Float(f32::INFINITY));
    let importable = DesignVariable::page(
        "gutter-24",
        "Gutter / 24",
        "spacing",
        "Spacing",
        DesignVariableResolvedType::Float,
    )
    .with_source(DesignVariableSource::library(
        "layout-library",
        "Layout library",
    ))
    .with_import_state(DesignVariableImportState::Available)
    .with_resolved_value(DesignVariableResolvedValue::Float(24.));
    let wrong_type = DesignVariable::page(
        "wrong",
        "Wrong",
        "content",
        "Content",
        DesignVariableResolvedType::String,
    )
    .with_resolved_value(DesignVariableResolvedValue::String("8".into()));
    let view_data = DesignLayoutGridVariableViewData::new([
        size_variable.clone(),
        auto_variable.clone(),
        importable.clone(),
        wrong_type,
    ])
    .with_create_state(DesignLayoutGridVariableCreateState::Disabled {
        reason: "Choose a writable collection".into(),
    });

    assert_eq!(view_data.variables.len(), 3);
    assert_eq!(
        view_data
            .variable("gutter-24")
            .map(|value| value.import_state),
        Some(DesignVariableImportState::Available)
    );
    assert_eq!(
        view_data.create_state.disabled_reason(),
        Some(&SharedString::from("Choose a writable collection"))
    );
    assert!(DesignLayoutGridVariableValue::Number(8.).is_compatible(&size_variable));
    assert!(
        !DesignLayoutGridVariableValue::Count(DesignLayoutGridCount::number(12))
            .is_compatible(&auto_variable)
    );
    assert!(
        DesignLayoutGridVariableValue::Count(DesignLayoutGridCount::Auto)
            .is_compatible(&auto_variable)
    );

    let binding = DesignLayoutGridVariableBinding::new("spacing-8", "Spacing / 8")
        .with_collection("Spacing")
        .read_only("Published library binding");
    let uniform = DesignLayoutGrid::uniform(8., DesignColor::BLUE)
        .with_id("uniform")
        .with_variable_binding(DesignLayoutGridVariableField::SectionSize, binding.clone());
    let (target, value) = uniform
        .variable_target(3, DesignPanelProperty::LayoutGridSize(99))
        .expect("uniform section-size target");
    assert_eq!(
        target,
        DesignLayoutGridVariableTarget::new(
            "uniform",
            3,
            DesignPanelProperty::LayoutGridSize(3),
            DesignLayoutGridVariableField::SectionSize,
        )
    );
    assert_eq!(value, DesignLayoutGridVariableValue::Number(8.));
    assert_eq!(
        uniform.variable_binding(DesignLayoutGridVariableField::SectionSize),
        Some(&binding)
    );
    assert!(!binding.can_detach);

    let fixed = DesignLayoutGrid::columns(
        DesignColumnLayoutGrid {
            alignment: DesignColumnGridAlignment::Left,
            ..DesignColumnLayoutGrid::default()
        },
        DesignColor::PURPLE,
    );
    assert_eq!(
        fixed
            .variable_target(0, DesignPanelProperty::LayoutGridSize(0))
            .map(|(target, _)| target.field),
        Some(DesignLayoutGridVariableField::SectionSize)
    );
    assert_eq!(
        fixed
            .variable_target(0, DesignPanelProperty::LayoutGridOffset(0))
            .map(|(target, _)| target.field),
        Some(DesignLayoutGridVariableField::Offset)
    );
    assert_eq!(
        fixed
            .variable_target(0, DesignPanelProperty::LayoutGridGutter(0))
            .map(|(target, _)| target.field),
        Some(DesignLayoutGridVariableField::GutterSize)
    );

    let stretch = DesignLayoutGrid::rows(DesignRowLayoutGrid::default(), DesignColor::BLUE);
    assert!(
        stretch
            .variable_target(0, DesignPanelProperty::LayoutGridSize(0))
            .is_none(),
        "stretch section size is implicit Auto and not bindable"
    );
    let (margin_target, _) = stretch
        .variable_target(0, DesignPanelProperty::LayoutGridMargin(0))
        .expect("stretch margin target");
    assert_eq!(margin_target.field, DesignLayoutGridVariableField::Offset);
    assert_eq!(
        margin_target.property,
        DesignPanelProperty::LayoutGridMargin(0)
    );
}

#[test]
fn presets_expose_kind_specific_sections() {
    let text = DesignPanelNode::new("text", "Title", DesignPanelNodeKind::Text);
    let text_path =
        DesignPanelNode::new("text-path", "Circular title", DesignPanelNodeKind::TextPath);
    let transform_group = DesignPanelNode::new(
        "transform-group",
        "Repeated petals",
        DesignPanelNodeKind::TransformGroup,
    );
    let slot = DesignPanelNode::new("slot", "Content", DesignPanelNodeKind::Slot);
    let image = DesignPanelNode::new("image", "Hero", DesignPanelNodeKind::Image);
    let line = DesignPanelNode::new("line", "Divider", DesignPanelNodeKind::Line);

    assert!(text.typography.is_some());
    assert_eq!(text.text_path, None);
    assert_eq!(text.text_path_start_data, None);
    assert!(text_path.typography.is_some());
    assert_eq!(text_path.text_path, Some(DesignTextPathViewData::default()));
    assert_eq!(
        text_path.text_path_start_data,
        Some(DesignTextPathStartData::DEFAULT)
    );
    let text_path_view = DesignTextPathViewData::new(DesignTextPathOrientation::Flipped)
        .flippable(false)
        .with_start_data_debug_controls(true);
    assert_eq!(
        text_path_view.orientation.toggled(),
        DesignTextPathOrientation::Default
    );
    assert!(!text_path_view.can_flip_orientation);
    assert!(text_path_view.show_start_data_debug_controls);
    assert_eq!(
        DesignTextPathStartData::new(3, 0.25),
        Some(DesignTextPathStartData {
            segment: 3,
            position: 0.25,
        })
    );
    assert_eq!(DesignTextPathStartData::new(0, -0.01), None);
    assert_eq!(DesignTextPathStartData::new(0, 1.01), None);
    assert_eq!(DesignTextPathStartData::new(0, f32::NAN), None);
    assert_eq!(text_path.fills[0].color, DesignColor::BLACK);
    assert!(transform_group.layout.is_some());
    assert!(transform_group.fills.is_empty());
    assert_eq!(transform_group.blend_mode, DesignBlendMode::PassThrough);
    assert!(slot.layout.is_some());
    assert_eq!(slot.corner_radii, [8.; 4]);
    assert_eq!(slot.blend_mode, DesignBlendMode::PassThrough);
    assert!(image.media.is_none());
    assert!(image.fills[0].kind == DesignPaintKind::Image);
    assert!(line.layout.is_some());
    assert_eq!(
        line.stroke.as_ref().map(|stroke| stroke.paints.len()),
        Some(1)
    );
}

#[test]
fn native_shape_geometry_preserves_figma_units_and_capabilities() {
    let arc = DesignArcData::new(std::f32::consts::FRAC_PI_2, std::f32::consts::PI, 1.5);
    assert!((arc.starting_degrees() - 90.).abs() < f32::EPSILON);
    assert!((arc.sweep_degrees() - 90.).abs() < f32::EPSILON);
    assert!((arc.sweep_angle() - std::f32::consts::FRAC_PI_2).abs() < f32::EPSILON);
    assert!((arc.ending_degrees() - 180.).abs() < f32::EPSILON);
    assert_eq!(arc.inner_radius, 1.);
    assert!(arc.has_inspector_controls());
    assert!(!DesignArcData::FULL_CIRCLE.has_inspector_controls());
    assert!(DesignArcData::new(0., std::f32::consts::TAU, 0.25).has_inspector_controls());
    assert_eq!(
        DesignShapeGeometry::for_node_kind(DesignPanelNodeKind::Ellipse),
        DesignShapeGeometry::Ellipse(DesignArcData::FULL_CIRCLE)
    );
    assert!(!DesignShapeGeometry::Ellipse(DesignArcData::FULL_CIRCLE).has_appearance_controls());
    assert!(DesignShapeGeometry::Ellipse(arc).has_appearance_controls());

    assert_eq!(DesignPolygonGeometry::new(1).point_count, 3);
    assert_eq!(DesignPolygonGeometry::new(u16::MAX).point_count, 60);
    assert_eq!(
        DesignShapeGeometry::for_node_kind(DesignPanelNodeKind::Polygon),
        DesignShapeGeometry::Polygon(DesignPolygonGeometry::new(3))
    );
    assert_eq!(
        DesignStarGeometry::new(2, -1.),
        DesignStarGeometry::new(3, 0.)
    );
    assert_eq!(DesignStarGeometry::new(u16::MAX, 2.).point_count, 60);
    assert_eq!(
        DesignShapeGeometry::for_node_kind(DesignPanelNodeKind::BooleanOperation),
        DesignShapeGeometry::Boolean(DesignBooleanOperation::Union)
    );
    assert_eq!(
        DesignShapeGeometry::for_node_kind(DesignPanelNodeKind::Table),
        DesignShapeGeometry::Table(DesignTableGeometry::new(4, 3))
    );

    let polygon_corners = DesignCornerCapabilities::for_node_kind(DesignPanelNodeKind::Polygon);
    assert!(polygon_corners.uniform_radius);
    assert!(!polygon_corners.independent_radii);
    assert!(polygon_corners.smoothing);
    assert!(
        DesignCornerCapabilities::for_node_kind(DesignPanelNodeKind::Section).independent_radii
    );
    assert!(DesignCornerCapabilities::for_node_kind(DesignPanelNodeKind::Ellipse).uniform_radius);
    for kind in [
        DesignPanelNodeKind::Line,
        DesignPanelNodeKind::Text,
        DesignPanelNodeKind::TextPath,
    ] {
        let corners = DesignCornerCapabilities::for_node_kind(kind);
        assert_eq!(corners, DesignCornerCapabilities::NONE);
    }
}

#[test]
fn section_preset_matches_its_native_paint_corner_and_status_contract() {
    let section = DesignPanelNode::new("section", "Checkout", DesignPanelNodeKind::Section);
    assert!(DesignPanelNodeKind::Section.supports_fill());
    assert!(DesignPanelNodeKind::Section.supports_stroke());
    assert!(DesignPanelNodeKind::Section.supports_corner_radius());
    assert!(!DesignPanelNodeKind::Section.supports_layer_appearance());
    assert!(!DesignPanelNodeKind::Section.supports_effects());
    assert!(section.corner_capabilities.independent_radii);
    assert!(section.corner_capabilities.smoothing);
    assert_eq!(section.section, Some(DesignSectionProperties::default()));

    let status = DesignSectionDevStatus::new(DesignSectionDevStatusKind::ReadyForDev)
        .with_description("Awaiting handoff")
        .changed(true);
    assert_eq!(status.kind, DesignSectionDevStatusKind::ReadyForDev);
    assert_eq!(
        status
            .description
            .as_ref()
            .map(|description| description.as_ref()),
        Some("Awaiting handoff")
    );
    assert!(status.changed);
}

#[test]
fn transform_repeat_modifiers_retain_api_discriminants_and_stable_ids() {
    let linear = DesignRepeatModifier::linear("linear", DesignRepeatAxis::Vertical);
    assert_eq!(linear.id.as_ref(), "linear");
    assert_eq!(
        linear.mode,
        DesignRepeatMode::Linear(DesignRepeatAxis::Vertical)
    );
    assert_eq!(linear.mode.repeat_type(), DesignRepeatType::Linear);
    assert_eq!(linear.mode.axis(), Some(DesignRepeatAxis::Vertical));
    assert_eq!(linear.count, 5);
    assert_eq!(linear.unit, DesignTransformUnit::Relative);
    assert_eq!(linear.offset, 100.);

    let radial = DesignRepeatModifier::radial("radial");
    assert_eq!(radial.mode, DesignRepeatMode::Radial);
    assert_eq!(radial.mode.repeat_type(), DesignRepeatType::Radial);
    assert_eq!(radial.mode.axis(), None);
    assert_eq!(radial.count, 8);
    assert_eq!(radial.unit, DesignTransformUnit::Pixels);
    assert_eq!(radial.offset, 96.);

    let node = DesignPanelNode::new("transform", "Pattern", DesignPanelNodeKind::TransformGroup);
    assert_eq!(node.transform_modifiers.len(), 1);
    assert_eq!(node.transform_modifiers[0].id.as_ref(), "repeat-0");
}

#[test]
fn component_presets_cover_every_discriminated_property_kind() {
    let mut kinds = [
        DesignPanelNodeKind::Component,
        DesignPanelNodeKind::ComponentSet,
        DesignPanelNodeKind::Instance,
        DesignPanelNodeKind::Slot,
    ]
    .into_iter()
    .flat_map(|kind| {
        DesignPanelNode::new("node", kind.label(), kind)
            .component_properties
            .into_iter()
            .map(|property| {
                assert_eq!(property.definition.kind(), property.resolved_value.kind());
                property.definition.kind()
            })
    })
    .collect::<Vec<_>>();
    kinds.sort_by_key(|kind| match kind {
        DesignComponentPropertyKind::Boolean => 0,
        DesignComponentPropertyKind::Text => 1,
        DesignComponentPropertyKind::InstanceSwap => 2,
        DesignComponentPropertyKind::Variant => 3,
        DesignComponentPropertyKind::Slot => 4,
    });
    kinds.dedup();
    assert_eq!(
        kinds,
        vec![
            DesignComponentPropertyKind::Boolean,
            DesignComponentPropertyKind::Text,
            DesignComponentPropertyKind::InstanceSwap,
            DesignComponentPropertyKind::Variant,
            DesignComponentPropertyKind::Slot,
        ]
    );
}

#[test]
fn component_roles_enforce_definition_instance_and_slot_boundaries() {
    assert!(DesignComponentRole::StandaloneMain.can_edit_property_value());
    assert!(DesignComponentRole::VariantChild.can_edit_property_value());
    assert!(DesignComponentRole::ComponentSet.can_edit_property_value());
    assert!(DesignComponentRole::Instance.can_edit_property_value());
    assert!(!DesignComponentRole::SlotDefinition.can_edit_property_value());
    assert!(DesignComponentRole::SlotInstance.can_edit_property_value());

    assert!(DesignComponentRole::SlotDefinition.can_configure_slot());
    assert!(!DesignComponentRole::Instance.can_configure_slot());
    assert!(DesignComponentRole::Instance.can_modify_slot_instances());
    assert!(DesignComponentRole::SlotInstance.can_modify_slot_instances());
    assert!(!DesignComponentRole::StandaloneMain.can_modify_slot_instances());

    assert!(DesignComponentRole::Instance.can_detach_instance());
    assert!(!DesignComponentRole::SlotInstance.can_detach_instance());
    assert!(DesignComponentRole::SlotInstance.can_reset_instance_overrides());
    assert!(!DesignComponentRole::ComponentSet.can_reset_instance_overrides());

    assert!(DesignComponentRole::StandaloneMain.can_author_component_properties());
    assert!(DesignComponentRole::ComponentSet.can_author_component_properties());
    assert!(!DesignComponentRole::VariantChild.can_author_component_properties());
    assert!(!DesignComponentRole::Instance.can_author_component_properties());
}

#[test]
fn component_authoring_projection_preserves_partition_and_stable_lookup_identity() {
    let variant = DesignComponentProperty::variant(
        "state",
        "State",
        "Default",
        "Default",
        vec!["Default".into(), "Hover".into()],
    );
    let text = DesignComponentProperty::text("label", "Label", "Button", "Button");
    let boolean = DesignComponentProperty::boolean("visible", "Visible", true, true);
    let option = DesignComponentVariantOptionAuthoring::editable("state-hover", "Hover");
    let definition = DesignComponentPropertyDefinitionAuthoring::editable("state")
        .with_variant_options([option.clone()]);
    let choice = DesignComponentPropertyChoice::new(
        "visible",
        "Visible",
        DesignComponentPropertyKind::Boolean,
    );
    let control = DesignAppliedComponentPropertyControl::new(
        "visible-control",
        "icon-layer",
        "Icon",
        DesignComponentPropertyApplicationSurface::Appearance,
        [choice.clone()],
    )
    .applied_to("visible");
    let exposure = DesignNestedComponentPropertyExposureCandidate::new(
        "nested-visible",
        "nested-icon",
        "Nested icon",
        choice,
    )
    .exposed_as("public-visible");
    let authoring = DesignComponentAuthoringViewData {
        create_kinds: vec![
            DesignComponentPropertyKind::Variant,
            DesignComponentPropertyKind::Text,
        ],
        definitions: vec![definition],
        applied_properties: vec![control],
        exposure_candidates: vec![exposure],
    };

    assert!(authoring.preserves_variant_partition(&[
        variant.clone(),
        text.clone(),
        boolean.clone(),
    ]));
    assert!(!authoring.preserves_variant_partition(&[text, variant, boolean,]));
    assert_eq!(
        DesignComponentPropertyPartition::for_kind(DesignComponentPropertyKind::Variant),
        DesignComponentPropertyPartition::Variant
    );
    assert_eq!(
        DesignComponentPropertyPartition::for_kind(DesignComponentPropertyKind::Slot),
        DesignComponentPropertyPartition::Regular
    );
    assert_eq!(
        authoring
            .definition("state")
            .and_then(|definition| definition.variant_options.first()),
        Some(&option)
    );
    assert_eq!(
        authoring
            .applied_control("visible-control")
            .and_then(DesignAppliedComponentPropertyControl::applied_property)
            .map(|property| property.property_id.as_ref()),
        Some("visible")
    );
    assert_eq!(
        authoring
            .exposure_candidate("nested-visible")
            .and_then(|candidate| candidate.exposed_property_id.as_ref())
            .map(SharedString::as_ref),
        Some("public-visible")
    );
}

#[test]
fn component_property_variable_targets_match_figmas_exact_fields_and_types() {
    let boolean = DesignComponentProperty::boolean("visible", "Visible", true, true);
    let text = DesignComponentProperty::text("label", "Label", "Button", "Continue");
    let swap = DesignComponentProperty::instance_swap("icon", "Icon", None, None, Vec::new());
    let variant = DesignComponentProperty::variant(
        "state",
        "State",
        "Default",
        "Hover",
        vec!["Default".into(), "Hover".into()],
    );
    let slot = DesignComponentProperty::slot(
        "content",
        "Content",
        DesignSlotValue::default(),
        DesignSlotValue::default(),
        DesignSlotSettings::default(),
        DesignSlotState::default(),
    );

    let boolean_definition = boolean
        .variable_target(DesignComponentRole::StandaloneMain)
        .expect("boolean definition target");
    assert_eq!(
        boolean_definition.field,
        DesignComponentPropertyVariableField::DefinitionDefaultValue
    );
    assert_eq!(boolean_definition.field.api_name(), "defaultValue");
    assert_eq!(
        boolean_definition.resolved_type,
        DesignVariableResolvedType::Boolean
    );

    for property in [&text, &swap] {
        let target = property
            .variable_target(DesignComponentRole::VariantChild)
            .expect("string-backed definition target");
        assert_eq!(
            target.field,
            DesignComponentPropertyVariableField::DefinitionDefaultValue
        );
        assert_eq!(target.resolved_type, DesignVariableResolvedType::String);
    }
    assert!(
        variant
            .variable_target(DesignComponentRole::ComponentSet)
            .is_none(),
        "variant definition defaults do not support variable aliases"
    );

    let variant_value = variant
        .variable_target(DesignComponentRole::Instance)
        .expect("variant instance value target");
    assert_eq!(
        variant_value.field,
        DesignComponentPropertyVariableField::InstanceValue
    );
    assert_eq!(variant_value.field.api_name(), "value");
    assert_eq!(
        variant_value.resolved_type,
        DesignVariableResolvedType::String
    );
    assert!(
        slot.variable_target(DesignComponentRole::SlotInstance)
            .is_none()
    );
    assert!(
        boolean
            .variable_target(DesignComponentRole::SlotDefinition)
            .is_none()
    );
}

#[test]
fn component_property_bindings_origins_and_slot_descriptions_are_lossless() {
    let default_binding = DesignComponentPropertyVariableBinding::new(
        "definition-variable",
        "Definition variable",
        DesignVariableResolvedValue::String("Default label".into()),
    )
    .detachable(false);
    let value_binding = DesignComponentPropertyVariableBinding::new(
        "instance-variable",
        "Instance variable",
        DesignVariableResolvedValue::String("Resolved label".into()),
    );
    let nested_main =
        DesignComponentReference::remote("badge-main", "Badge", "Product foundations");
    let property = DesignComponentProperty::text("label", "Label", "Button", "Continue")
        .with_description("Text descriptions are not a Figma component-property field")
        .with_multiline(true)
        .with_default_value_binding(default_binding.clone())
        .with_resolved_value_binding(value_binding.clone())
        .from_nested_instance("badge-instance", "Status badge", Some(nested_main.clone()));

    assert_eq!(property.description, None);
    assert!(matches!(
        property.definition,
        DesignComponentPropertyDefinition::Text {
            multiline: true,
            ..
        }
    ));
    assert_eq!(
        property.variable_binding(DesignComponentPropertyVariableField::DefinitionDefaultValue),
        Some(&default_binding)
    );
    assert_eq!(
        property.variable_binding(DesignComponentPropertyVariableField::InstanceValue),
        Some(&value_binding)
    );
    assert!(matches!(
        &property.origin,
        DesignComponentPropertyOrigin::NestedInstance {
            instance_id,
            instance_name,
            main_component: Some(main),
        } if instance_id.as_ref() == "badge-instance"
            && instance_name.as_ref() == "Status badge"
            && main == &nested_main
    ));

    let slot = DesignComponentProperty::slot(
        "content",
        "Content",
        DesignSlotValue::default(),
        DesignSlotValue::default(),
        DesignSlotSettings::default(),
        DesignSlotState::default(),
    )
    .with_description("Optional inserted content");
    assert_eq!(
        slot.description
            .as_ref()
            .map(|description| description.as_ref()),
        Some("Optional inserted content")
    );
}

#[test]
fn component_swap_catalog_preserves_stable_identity_source_and_import_state() {
    let local = DesignComponentSwapCandidate::local(
        DesignComponentReference::local("icon-plus", "Plus"),
        "local-key-plus",
        "page-5",
        "Page 5",
    )
    .with_search(
        DesignComponentSearchMetadata::new(["Icons", "Actions"], ["add", "create"])
            .described("Adds an item"),
    );
    let available = DesignComponentSwapCandidate::library(
        DesignComponentReference::remote("icon-status", "Status icons", "Product foundations"),
        "library-key-status",
        "product-foundations",
        "Product foundations",
    )
    .with_asset_kind(DesignComponentAssetKind::ComponentSet);
    let imported = available
        .clone()
        .with_import_state(DesignComponentImportState::Imported);
    let view_data =
        DesignComponentSwapViewData::new([local.clone(), available.clone(), imported.clone()]);

    assert!(local.can_apply());
    assert!(!local.can_import());
    assert!(available.can_import());
    assert!(!available.can_apply());
    assert!(imported.can_apply());
    assert_eq!(available.asset_kind.api_name(), "COMPONENT_SET");
    assert!(local.matches_search("icons create"));
    assert!(!local.matches_search("avatar"));

    let selection = imported.selection();
    let resolved = view_data
        .candidate(&selection)
        .expect("stable source/key selection");
    assert_eq!(resolved.component_key.as_ref(), "library-key-status");
    assert!(matches!(
        &resolved.source,
        DesignComponentSource::Library {
            library_id,
            library_name,
        } if library_id.as_ref() == "product-foundations"
            && library_name.as_ref() == "Product foundations"
    ));
}

#[test]
fn typed_component_values_accept_the_legacy_story_projection() {
    let mut boolean = DesignComponentProperty::boolean("visible", "Visible", true, true);
    boolean.value = "False".into();
    assert_eq!(
        boolean.effective_value(),
        DesignComponentPropertyValue::Boolean(false)
    );

    let available = DesignComponentReference::local("available", "Available");
    let missing = DesignComponentReference::remote("missing", "Missing", "Library")
        .with_availability(DesignComponentAvailability::Missing);
    let swap = DesignComponentProperty::instance_swap(
        "icon",
        "Icon",
        Some(available.clone()),
        Some(available),
        vec![missing],
    );
    assert!(
        swap.definition.option_labels().is_empty(),
        "unavailable remote references must not be offered as swap choices"
    );
}

#[test]
fn slot_schema_preserves_settings_contents_violations_and_reset_state() {
    let preferred = DesignComponentReference::remote("card", "Content card", "Product foundations");
    let value = DesignSlotValue {
        children: vec![{
            let mut child = DesignSlotChild::instance("instance", "Card");
            child.main_component = Some(preferred.clone());
            child
        }],
    };
    let mut property = DesignComponentProperty::slot(
        "content",
        "Content",
        DesignSlotValue::default(),
        value.clone(),
        DesignSlotSettings {
            stretch_child_on_insert: false,
            display_empty: false,
            minimum_children: Some(2),
            maximum_children: Some(4),
            preferred_values_only: true,
            preferred_values: vec![preferred],
        },
        DesignSlotState {
            violations: vec![DesignSlotViolation::BelowMinimum {
                minimum: 2,
                actual: 1,
            }],
            reset_state: DesignComponentResetState::Resettable,
        },
    );
    assert_eq!(
        property.effective_value(),
        DesignComponentPropertyValue::Slot(value)
    );
    let settings = property.slot_settings().expect("slot settings");
    assert!(!settings.stretch_child_on_insert);
    assert!(settings.preferred_values_only);
    assert_eq!(settings.minimum_children, Some(2));
    assert_eq!(settings.maximum_children, Some(4));
    assert!(
        property
            .slot_state
            .as_ref()
            .expect("slot state")
            .reset_state
            .can_reset()
    );
    assert_eq!(
        property.slot_state.as_ref().expect("slot state").violations[0].label(),
        "Needs at least 2 layers; contains 1"
    );
    assert!(
        property.apply_slot_settings_change(&DesignSlotSettingsChange::MaximumInstances(Some(0)))
    );
    assert!(
        property
            .slot_state
            .as_ref()
            .expect("slot state")
            .violations
            .iter()
            .any(|violation| matches!(
                violation,
                DesignSlotViolation::AboveMaximum {
                    maximum: 0,
                    actual: 1
                }
            ))
    );
    property.mark_overridden();
    property.reset_to_default();
    assert_eq!(
        property.override_state,
        DesignComponentPropertyOverrideState::Default
    );
    assert_eq!(
        property.effective_value(),
        DesignComponentPropertyValue::Slot(DesignSlotValue::default())
    );
}

#[test]
fn slot_values_preserve_arbitrary_children_and_native_null_limits() {
    let value = DesignSlotValue {
        children: vec![
            DesignSlotChild::layer("copy", "Supporting copy", DesignPanelNodeKind::Text),
            DesignSlotChild::layer("hero", "Hero image", DesignPanelNodeKind::Rectangle),
            DesignSlotChild::instance("action", "Primary action"),
        ],
    };
    assert_eq!(value.children[0].kind, DesignPanelNodeKind::Text);
    assert_eq!(value.children[1].kind, DesignPanelNodeKind::Rectangle);
    assert!(value.children[2].is_instance());
    assert_eq!(
        DesignComponentPropertyValue::Slot(value.clone()).display_value(),
        "3 layers"
    );

    let mut property = DesignComponentProperty::slot(
        "content",
        "Content",
        DesignSlotValue::default(),
        value,
        DesignSlotSettings::default(),
        DesignSlotState::default(),
    );
    let settings = property.slot_settings().expect("slot settings");
    assert_eq!(settings.minimum_children, None);
    assert_eq!(settings.maximum_children, None);
    assert!(
        property.apply_slot_settings_change(&DesignSlotSettingsChange::MinimumInstances(Some(1),))
    );
    assert_eq!(
        property
            .slot_settings()
            .expect("slot settings")
            .minimum_children,
        Some(1)
    );
    assert!(property.apply_slot_settings_change(&DesignSlotSettingsChange::MinimumInstances(None)));
    assert_eq!(
        property
            .slot_settings()
            .expect("slot settings")
            .minimum_children,
        None
    );
    assert!(
        property.apply_slot_settings_change(&DesignSlotSettingsChange::PreferredValuesOnly(true))
    );
    assert!(
        property
            .slot_state
            .as_ref()
            .expect("slot state")
            .violations
            .iter()
            .any(|violation| matches!(
                violation,
                DesignSlotViolation::NonPreferredChild { child_id, .. }
                    if child_id.as_ref() == "copy"
            ))
    );
}

#[test]
fn real_beta_node_kinds_have_fidelity_capabilities() {
    assert_eq!(DesignPanelNodeKind::TextPath.label(), "Text path");
    assert_eq!(
        DesignPanelNodeKind::TransformGroup.label(),
        "Transform group"
    );
    assert_eq!(DesignPanelNodeKind::Slot.label(), "Slot");

    assert!(DesignPanelNodeKind::TextPath.supports_layout());
    assert!(!DesignPanelNodeKind::TextPath.supports_auto_layout_container());
    assert!(DesignPanelNodeKind::TextPath.supports_fill());
    assert!(DesignPanelNodeKind::TextPath.supports_stroke());
    assert!(!DesignPanelNodeKind::TextPath.supports_corner_radius());
    assert!(!DesignPanelNodeKind::Text.supports_corner_radius());
    assert!(DesignPanelNodeKind::Image.supports_corner_radius());
    assert!(DesignPanelNodeKind::Video.supports_corner_radius());
    assert!(!DesignPanelNodeKind::Mask.supports_corner_radius());
    assert!(!DesignPanelNodeKind::Pen.supports_corner_radius());
    assert!(!DesignPanelNodeKind::Pencil.supports_corner_radius());
    assert!(!DesignPanelNodeKind::Table.supports_corner_radius());

    assert!(DesignPanelNodeKind::TransformGroup.supports_layout());
    assert!(!DesignPanelNodeKind::TransformGroup.supports_auto_layout_container());
    assert!(!DesignPanelNodeKind::TransformGroup.supports_fill());
    assert!(!DesignPanelNodeKind::TransformGroup.supports_stroke());
    assert!(!DesignPanelNodeKind::TransformGroup.supports_corner_radius());

    assert!(DesignPanelNodeKind::Slot.supports_layout());
    assert!(DesignPanelNodeKind::Slot.supports_auto_layout_container());
    assert!(!DesignPanelNodeKind::Slot.supports_grid_auto_layout());
    assert!(DesignPanelNodeKind::Slot.supports_fill());
    assert!(DesignPanelNodeKind::Slot.supports_stroke());
    assert!(DesignPanelNodeKind::Slot.supports_corner_radius());
    assert!(DesignPanelNodeKind::Slot.supports_constraints());
    assert!(DesignPanelNodeKind::Group.supports_constraints());
    assert!(DesignPanelNodeKind::TransformGroup.supports_constraints());
    assert!(!DesignPanelNodeKind::BooleanOperation.supports_constraints());
    assert!(DesignPanelNodeKind::Frame.supports_auto_layout_container());
    assert!(DesignPanelNodeKind::Frame.supports_grid_auto_layout());
    assert!(DesignPanelNodeKind::Component.supports_auto_layout_container());
    assert!(!DesignPanelNodeKind::Group.supports_auto_layout_container());
    assert!(!DesignPanelNodeKind::Table.supports_auto_layout_container());
    assert!(!DesignPanelNodeKind::Rectangle.supports_auto_layout_container());
    assert!(!DesignPanelNodeKind::Image.supports_auto_layout_container());
    assert!(DesignPanelNodeKind::Line.supports_dimensions());
    assert!(DesignPanelNodeKind::Slice.supports_dimensions());
    assert!(DesignPanelNodeKind::Section.supports_dimensions());
    assert!(DesignPanelNodeKind::Section.supports_visibility());
    assert!(!DesignPanelNodeKind::Section.supports_transforms());
    assert!(!DesignPanelNodeKind::Section.supports_auto_layout_child());
    assert!(!DesignPanelNodeKind::Section.supports_add_auto_layout());
    assert!(DesignPanelNodeKind::MultipleSelection.supports_dimensions());
    assert!(DesignPanelNodeKind::Frame.supports_resize_to_fit());
    assert!(DesignPanelNodeKind::Group.supports_resize_to_fit());
    assert!(DesignPanelNodeKind::TransformGroup.supports_resize_to_fit());
    assert!(DesignPanelNodeKind::Frame.supports_clip_content());
    assert!(!DesignPanelNodeKind::Table.supports_clip_content());
    assert!(!DesignPanelNodeKind::Text.supports_resize_to_fit());
    assert!(!DesignPanelNodeKind::Rectangle.supports_clip_content());
}

#[test]
fn widget_is_a_canonical_opaque_design_node_with_an_exact_profile() {
    let kind = DesignPanelNodeKind::Widget;
    assert_eq!(kind.canonical_kind(), kind);
    assert!(!kind.is_compatibility_alias());
    assert!(DesignPanelNodeKind::ALL.contains(&kind));
    assert!(DesignPanelNodeKind::COMPATIBILITY_ALL.contains(&kind));
    assert!(kind.supports_dimensions());
    assert!(kind.supports_visibility());
    assert!(kind.supports_position_coordinates());
    assert!(!kind.supports_arrange());
    assert!(!kind.supports_transforms());
    assert!(!kind.supports_aspect_ratio_lock());
    assert!(!kind.supports_auto_layout_child());
    assert!(!kind.supports_add_auto_layout());
    assert!(!kind.supports_auto_layout_container());
    assert!(!kind.supports_grid_auto_layout());
    assert!(!kind.supports_resize_to_fit());
    assert!(!kind.supports_clip_content());
    assert!(!kind.supports_fill());
    assert!(!kind.supports_stroke());
    assert!(!kind.supports_layer_appearance());
    assert!(!kind.supports_effects());
    assert!(!kind.supports_constraints());

    let node = DesignPanelNode::new("widget", "Widget", kind)
        .with_capabilities(DesignPanelNodeCapabilities::for_node_kind(kind));
    assert_eq!(
        node.capabilities
            .as_ref()
            .expect("exact Widget capabilities")
            .sections,
        [
            DesignPanelSection::Position,
            DesignPanelSection::Layout,
            DesignPanelSection::Layer,
            DesignPanelSection::Export,
        ]
    );
    assert!(node.supports_section(DesignPanelSection::Layer));
    assert!(
        node.property_variable_target(DesignPanelProperty::Visible)
            .is_some()
    );
    assert!(
        node.property_variable_target(DesignPanelProperty::Opacity)
            .is_none()
    );
    for unsupported in [
        DesignPanelProperty::Gap,
        DesignPanelProperty::PaddingTop,
        DesignPanelProperty::MinWidth,
        DesignPanelProperty::MaxHeight,
    ] {
        assert!(
            node.property_variable_target(unsupported).is_none(),
            "Widget must not publish a variable target for {unsupported:?}",
        );
    }
    let header = DesignSelectionHeaderViewData::for_node_kind(kind);
    assert!(header.title_menu.is_none());
    assert!(header.primary_controls.is_empty());
    assert!(header.overflow_controls.is_empty());
}

#[test]
fn exact_node_capabilities_override_the_other_kind_fallback() {
    let mut node = DesignPanelNode::new("plugin", "Plugin node", DesignPanelNodeKind::Other);
    assert!(node.supports_fill());
    assert!(node.supports_effects());
    assert!(!node.supports_constraints());
    assert!(!node.supports_layout_guides());

    node.capabilities = Some(
        DesignPanelNodeCapabilities::for_node_kind(DesignPanelNodeKind::Other)
            .with_sections([
                DesignPanelSection::Position,
                DesignPanelSection::Layout,
                DesignPanelSection::Layer,
                DesignPanelSection::Stroke,
                DesignPanelSection::LayoutGrid,
                DesignPanelSection::Export,
            ])
            .with_fill(false)
            .with_effects(false)
            .with_constraints(true)
            .with_layout_guides(true),
    );

    assert!(!node.supports_fill());
    assert!(!node.supports_effects());
    assert!(node.supports_constraints());
    assert!(node.supports_layout_guides());
    assert!(!node.supports_section(DesignPanelSection::Fill));
    assert!(node.supports_section(DesignPanelSection::LayoutGrid));
    assert!(
        node.property_variable_target(DesignPanelProperty::Opacity)
            .is_some()
    );
    assert!(
        node.property_variable_target(DesignPanelProperty::StrokeWeight)
            .is_none(),
        "a missing stroke snapshot remains ineligible even when the section is supported"
    );
}

#[test]
fn exact_fill_edit_mode_keeps_color_and_opacity_without_paint_structure() {
    let mut node = DesignPanelNode::new("synthetic", "Synthetic", DesignPanelNodeKind::Rectangle);
    assert_eq!(
        node.paint_collection_edit_mode(DesignPanelCollection::Fill),
        DesignPaintCollectionEditMode::Full
    );

    node.capabilities = Some(
        DesignPanelNodeCapabilities::for_node_kind(DesignPanelNodeKind::Rectangle)
            .with_fill_edit_mode(DesignPaintCollectionEditMode::ColorAndOpacityOnly),
    );
    let edit_mode = node.paint_collection_edit_mode(DesignPanelCollection::Fill);
    assert!(edit_mode.allows_property(&DesignPaintProperty::Color));
    assert!(edit_mode.allows_property(&DesignPaintProperty::Opacity));
    assert!(!edit_mode.allows_property(&DesignPaintProperty::Visible));
    assert!(!edit_mode.allows_property(&DesignPaintProperty::Payload));
    assert_eq!(
        node.paint_collection_edit_mode(DesignPanelCollection::Stroke),
        DesignPaintCollectionEditMode::Full,
        "the exact Fill mode must not narrow other collections"
    );
}

#[test]
fn frame_fill_export_visibility_is_an_explicit_host_capability() {
    assert_eq!(
        DesignPanelNode::new("frame", "Frame", DesignPanelNodeKind::Frame).fill_shows_in_exports,
        Some(true)
    );
    for kind in [
        DesignPanelNodeKind::Component,
        DesignPanelNodeKind::Rectangle,
        DesignPanelNodeKind::Section,
    ] {
        assert_eq!(
            DesignPanelNode::new("node", kind.label(), kind).fill_shows_in_exports,
            None,
            "{kind:?} should not inherit a UI-only Frame capability"
        );
    }
}

#[test]
fn all_node_kinds_include_new_real_kinds_once() {
    for expected in [
        DesignPanelNodeKind::TextPath,
        DesignPanelNodeKind::TransformGroup,
        DesignPanelNodeKind::Slot,
    ] {
        assert_eq!(
            DesignPanelNodeKind::ALL
                .iter()
                .filter(|kind| **kind == expected)
                .count(),
            1,
            "{expected:?} should appear exactly once"
        );
    }
}

#[test]
fn stroke_cap_set_matches_figma_plugin_api_exactly() {
    assert_eq!(
        DesignStrokeCap::ALL.map(DesignStrokeCap::label),
        [
            "None",
            "Round",
            "Square",
            "Line arrow",
            "Triangle arrow",
            "Reverse triangle",
            "Diamond arrow",
            "Circle",
        ]
    );
}

#[test]
fn multiple_stroke_paints_share_and_preserve_geometry() {
    let mut stroke = DesignStroke::for_node(
        DesignPanelNodeKind::Rectangle,
        DesignPaint::solid(DesignColor::BLACK),
        3.,
        DesignStrokeAlign::Inside,
    );
    stroke.weights = DesignStrokeWeights::custom(1., 2., 3., 4.);
    stroke.join = DesignStrokeJoin::Bevel;
    stroke.miter_angle = 72.;
    stroke.add_paint(DesignPaint::solid(DesignColor::PURPLE));

    let geometry = (
        stroke.weights,
        stroke.align,
        stroke.join,
        stroke.miter_angle,
    );
    assert!(stroke.move_paint(0, 1));
    assert!(stroke.remove_paint(0).is_some());
    assert_eq!(
        (
            stroke.weights,
            stroke.align,
            stroke.join,
            stroke.miter_angle,
        ),
        geometry
    );
    assert_eq!(stroke.paints.len(), 1);

    assert!(stroke.remove_paint(0).is_some());
    assert!(stroke.paints.is_empty());
    stroke.add_paint(DesignPaint::solid(DesignColor::BLUE));
    assert_eq!(
        (
            stroke.weights,
            stroke.align,
            stroke.join,
            stroke.miter_angle,
        ),
        geometry,
        "removing the final paint must not discard shared geometry"
    );
}

#[test]
fn stroke_alignment_and_individual_side_capabilities_follow_node_and_style() {
    let individual_weight_kinds = DesignPanelNodeKind::ALL
        .into_iter()
        .filter(|kind| DesignStrokeCapabilities::for_node_kind(*kind).individual_weights)
        .collect::<Vec<_>>();
    assert_eq!(
        individual_weight_kinds,
        vec![
            DesignPanelNodeKind::Frame,
            DesignPanelNodeKind::Component,
            DesignPanelNodeKind::ComponentSet,
            DesignPanelNodeKind::Instance,
            DesignPanelNodeKind::Slot,
            DesignPanelNodeKind::Rectangle,
        ]
    );

    let rectangle = DesignStroke::for_node(
        DesignPanelNodeKind::Rectangle,
        DesignPaint::solid(DesignColor::BLACK),
        1.,
        DesignStrokeAlign::Inside,
    );
    assert_eq!(rectangle.alignment_options(), &DesignStrokeAlign::ALL);
    assert!(rectangle.capabilities.individual_weights);

    let mut line_arrow = DesignStroke::for_node(
        DesignPanelNodeKind::Line,
        DesignPaint::solid(DesignColor::BLACK),
        1.,
        DesignStrokeAlign::Inside,
    );
    line_arrow.end_cap = DesignStrokeCap::LineArrow;
    assert_eq!(line_arrow.align, DesignStrokeAlign::Center);
    assert_eq!(line_arrow.alignment_options(), &[DesignStrokeAlign::Center]);
    assert!(!line_arrow.capabilities.individual_weights);
    assert!(!line_arrow.capabilities.position);

    let legacy_arrow = DesignStroke::for_node(
        DesignPanelNodeKind::Arrow,
        DesignPaint::solid(DesignColor::BLACK),
        2.,
        DesignStrokeAlign::Inside,
    );
    assert_eq!(legacy_arrow.align, DesignStrokeAlign::Center);
    assert_eq!(
        legacy_arrow.alignment_options(),
        &[DesignStrokeAlign::Center]
    );
    assert!(!legacy_arrow.capabilities.position);

    line_arrow.set_type(DesignStrokeType::StretchBrush);
    assert_eq!(line_arrow.align, DesignStrokeAlign::Center);
    assert_eq!(line_arrow.alignment_options(), &[DesignStrokeAlign::Center]);
    let mut rectangle_brush = rectangle;
    rectangle_brush.set_type(DesignStrokeType::Dynamic);
    assert_eq!(rectangle_brush.align, DesignStrokeAlign::Center);
    assert_eq!(
        rectangle_brush.alignment_options(),
        &[DesignStrokeAlign::Center]
    );
}

#[test]
fn endpoint_control_uses_host_topology_and_vector_edit_context() {
    assert_eq!(
        DesignStrokeEditContext::open(2).endpoint_control(),
        DesignStrokeEndpointControl::StartAndEnd
    );
    assert_eq!(
        DesignStrokeEditContext::closed().endpoint_control(),
        DesignStrokeEndpointControl::None
    );
    assert_eq!(
        DesignStrokeEditContext::branching(4).endpoint_control(),
        DesignStrokeEndpointControl::Aggregate
    );
    assert_eq!(
        DesignStrokeEditContext::branching(4)
            .with_selected_vertices(1)
            .endpoint_control(),
        DesignStrokeEndpointControl::SelectedVertices
    );
    assert_eq!(
        DesignStrokeEditContext::open(2)
            .with_selected_vertices(1)
            .endpoint_control(),
        DesignStrokeEndpointControl::SelectedVertices
    );
    assert_eq!(
        DesignStrokeEditContext::open(2)
            .with_vertex_selection(1, 0)
            .endpoint_control(),
        DesignStrokeEndpointControl::None
    );
}

#[test]
fn uniform_and_custom_side_weight_edits_keep_all_four_values() {
    let mut weights = DesignStrokeWeights::uniform(2.);
    weights.set_active(6.);
    assert_eq!(
        (weights.top, weights.right, weights.bottom, weights.left),
        (6., 6., 6., 6.)
    );

    weights.mode = DesignStrokeWeightMode::Right;
    weights.set_active(12.);
    assert_eq!(
        (weights.top, weights.right, weights.bottom, weights.left),
        (6., 12., 6., 6.)
    );

    weights.mode = DesignStrokeWeightMode::Custom;
    weights.left = 18.;
    assert!(!weights.is_uniform());
    assert_eq!(weights.active(), 6.);

    weights.set_mode(DesignStrokeWeightMode::All);
    assert_eq!(
        (weights.top, weights.right, weights.bottom, weights.left),
        (6., 6., 6., 6.)
    );
    assert!(weights.is_uniform());
}

#[test]
fn joins_miter_and_variable_width_are_shared_lossless_stroke_geometry() {
    let mut stroke = DesignStroke::for_node(
        DesignPanelNodeKind::Vector,
        DesignPaint::solid(DesignColor::BLACK),
        2.,
        DesignStrokeAlign::Center,
    );
    stroke.join = DesignStrokeJoin::Miter;
    stroke.miter_angle = 45.;
    stroke.variable_width = Some(DesignVariableWidthStroke::Preset(
        DesignVariableWidthPreset::MirroredTaper,
    ));
    assert_eq!(stroke.join, DesignStrokeJoin::Miter);
    assert_eq!(stroke.miter_angle, 45.);
    assert!(stroke.supports_variable_width());

    stroke.set_dash_mode(DesignStrokeDashMode::Dashed);
    assert!(stroke.supports_variable_width());
    stroke.edit_context = DesignStrokeEditContext::branching(3);
    assert!(!stroke.supports_variable_width());
}

#[test]
fn dash_modes_have_explicit_valid_lossless_transitions() {
    let mut dashes = DesignStrokeDashes::solid();
    assert!(dashes.is_valid());

    dashes.transition(DesignStrokeDashMode::Dashed);
    assert_eq!(dashes.pattern, vec![4., 4.]);
    assert!(dashes.set_pattern(vec![12., 6.]));
    assert!(!dashes.set_pattern(vec![12., 6., 2., 6.]));

    dashes.transition(DesignStrokeDashMode::Custom);
    assert_eq!(dashes.pattern, vec![12., 6.]);
    assert!(dashes.set_pattern(vec![12., 6., 2., 6.]));
    dashes.transition(DesignStrokeDashMode::Solid);
    assert!(dashes.pattern.is_empty());
    assert!(dashes.is_valid());
}

#[test]
fn variable_width_keeps_preset_identity_and_host_point_order() {
    assert_eq!(
        DesignVariableWidthPreset::ALL,
        [
            DesignVariableWidthPreset::Uniform,
            DesignVariableWidthPreset::Wedge,
            DesignVariableWidthPreset::Taper,
            DesignVariableWidthPreset::QuarterTaper,
            DesignVariableWidthPreset::Eye,
            DesignVariableWidthPreset::MirroredTaper,
        ]
    );
    assert_eq!(
        DesignVariableWidthPreset::ALL.map(DesignVariableWidthPreset::api_name),
        [
            "UNIFORM",
            "WEDGE",
            "TAPER",
            "QUARTER_TAPER",
            "EYE",
            "MIRRORED_TAPER",
        ]
    );
    let mut width = DesignVariableWidthStroke::custom([
        DesignVariableWidthPoint::new(0.75, 1.5),
        DesignVariableWidthPoint::new(0.25, 0.5),
    ]);
    assert!(width.is_valid());
    assert_eq!(
        width.points().expect("custom points"),
        &[
            DesignVariableWidthPoint::new(0.75, 1.5),
            DesignVariableWidthPoint::new(0.25, 0.5),
        ]
    );
    assert!(width.set_point_position(1, 0.4));
    assert_eq!(width.points().expect("custom points")[0].position, 0.75);
    assert!(!width.set_point_position(1, 1.01));
    assert!(!width.set_point_width(1, -0.01));
}

#[test]
fn complex_strokes_keep_exact_forms_domains_and_read_only_custom_payloads() {
    assert_eq!(DesignStretchBrushName::ALL.len(), 15);
    assert_eq!(DesignScatterBrushName::ALL.len(), 10);
    assert_eq!(
        DesignStrokeBrushDirection::ALL.map(DesignStrokeBrushDirection::api_name),
        ["FORWARD", "BACKWARD"]
    );
    assert_eq!(
        DesignStretchBrushName::ALL.map(DesignStretchBrushName::api_name),
        [
            "HEIST",
            "BLOCKBUSTER",
            "GRINDHOUSE",
            "BIOPIC",
            "SPAGHETTI_WESTERN",
            "SLASHER",
            "HARDBOILED",
            "VERITE",
            "EPIC",
            "SCREWBALL",
            "ROM_COM",
            "NOIR",
            "PROPAGANDA",
            "MELODRAMA",
            "NEW_WAVE",
        ]
    );
    assert_eq!(
        DesignScatterBrushName::ALL.map(DesignScatterBrushName::api_name),
        [
            "BUBBLEGUM",
            "WITCH_HOUSE",
            "SHOEGAZE",
            "HONKY_TONK",
            "SCREAMO",
            "DRONE",
            "DOO_WOP",
            "SPOKEN_WORD",
            "VAPORWAVE",
            "OI",
        ]
    );
    assert!(
        DesignScatterBrushStroke {
            brush: DesignScatterBrushName::Bubblegum,
            gap: 0.25,
            wiggle: 0.,
            size_jitter: 3.,
            angular_jitter: -180.,
            rotation: 180.,
        }
        .is_valid()
    );
    assert!(
        !DesignScatterBrushStroke {
            gap: 0.24,
            ..DesignScatterBrushStroke::default()
        }
        .is_valid()
    );
    assert!(
        DesignDynamicStroke {
            frequency: 0.01,
            wiggle: 0.,
            smoothen: 1.,
        }
        .is_valid()
    );
    assert!(
        !DesignDynamicStroke {
            frequency: 20.01,
            ..DesignDynamicStroke::default()
        }
        .is_valid()
    );

    let mut stroke = DesignStroke::for_node(
        DesignPanelNodeKind::Vector,
        DesignPaint::solid(DesignColor::BLACK),
        2.,
        DesignStrokeAlign::Center,
    );
    assert!(
        stroke.set_variable_width(Some(DesignVariableWidthStroke::Preset(
            DesignVariableWidthPreset::Eye,
        )))
    );
    assert!(stroke.set_type(DesignStrokeType::Dynamic));
    assert!(stroke.variable_width.is_none());
    stroke.complex_stroke = DesignComplexStroke::Opaque(DesignOpaqueComplexStroke::new(
        "CUSTOM",
        "Host brush",
        "{\"brushName\":\"CUSTOM\"}",
    ));
    assert!(!stroke.set_type(DesignStrokeType::Opaque));
    assert!(!stroke.supports_variable_width());
}

#[test]
fn auto_layout_preset_keeps_layout_host_controlled() {
    let node = DesignPanelNode::new("frame", "Card", DesignPanelNodeKind::Frame)
        .with_layout_mode(DesignLayoutMode::Horizontal);
    let layout = node.layout.expect("frame layout");

    assert_eq!(layout.mode, DesignLayoutMode::Horizontal);
    assert_eq!(layout.horizontal_sizing, DesignSizingMode::Hug);
    assert_eq!(layout.vertical_sizing, DesignSizingMode::Hug);
}

#[test]
fn auto_layout_mode_and_wrap_follow_node_capabilities() {
    let mut slot = DesignLayout::default();
    assert!(!slot.set_mode_for_kind(DesignPanelNodeKind::Slot, DesignLayoutMode::Grid));
    assert_eq!(slot.mode, DesignLayoutMode::None);
    assert!(slot.set_mode_for_kind(DesignPanelNodeKind::Slot, DesignLayoutMode::Horizontal));
    slot.set_wrap(true);
    assert!(slot.wrap);
    assert!(slot.set_mode_for_kind(DesignPanelNodeKind::Slot, DesignLayoutMode::Vertical));
    assert!(!slot.wrap);

    let mut imported = DesignLayout {
        mode: DesignLayoutMode::Grid,
        wrap: true,
        ..DesignLayout::default()
    };
    imported.normalize_for_kind(DesignPanelNodeKind::Group);
    assert_eq!(imported.mode, DesignLayoutMode::None);
    assert!(!imported.wrap);
}

#[test]
fn wrapped_counter_axis_preserves_nullable_positive_spacing_semantics() {
    let mut layout = DesignLayout::default();
    assert_eq!(
        layout.counter_axis_align_content,
        DesignCounterAxisAlignContent::Auto
    );
    assert_eq!(layout.counter_axis_gap, None);
    assert!(!layout.set_counter_axis_gap(Some(8.)));

    layout.mode = DesignLayoutMode::Horizontal;
    layout.set_wrap(true);
    assert!(!layout.set_counter_axis_gap(Some(0.)));
    assert!(!layout.set_counter_axis_gap(Some(f32::NAN)));
    assert!(layout.set_counter_axis_gap(Some(6.)));
    assert_eq!(layout.counter_axis_gap, Some(6.));

    assert!(layout.set_counter_axis_align_content(DesignCounterAxisAlignContent::SpaceBetween));
    assert_eq!(layout.counter_axis_gap, None);
    assert!(!layout.set_counter_axis_gap(Some(4.)));

    assert!(layout.set_counter_axis_align_content(DesignCounterAxisAlignContent::Auto));
    assert!(layout.set_counter_axis_gap(None));
    assert_eq!(layout.counter_axis_gap, None);
}

#[test]
fn counter_axis_normalization_clears_inapplicable_or_ignored_spacing() {
    let mut space_between = DesignLayout {
        mode: DesignLayoutMode::Horizontal,
        wrap: true,
        counter_axis_gap: Some(12.),
        counter_axis_align_content: DesignCounterAxisAlignContent::SpaceBetween,
        ..DesignLayout::default()
    };
    space_between.normalize_for_kind(DesignPanelNodeKind::Frame);
    assert_eq!(space_between.counter_axis_gap, None);

    let mut no_wrap = DesignLayout {
        mode: DesignLayoutMode::Horizontal,
        wrap: false,
        counter_axis_gap: Some(12.),
        ..DesignLayout::default()
    };
    no_wrap.normalize_for_kind(DesignPanelNodeKind::Frame);
    assert_eq!(
        no_wrap.counter_axis_align_content,
        DesignCounterAxisAlignContent::Auto
    );
    assert_eq!(no_wrap.counter_axis_gap, None);
}

#[test]
fn layout_guides_preserve_discriminated_axis_specific_settings() {
    let uniform = DesignLayoutGrid::uniform(8., DesignColor::BLUE);
    assert_eq!(uniform.kind(), DesignGridKind::Uniform);
    let DesignLayoutGridSettings::Uniform(settings) = uniform.settings else {
        panic!("expected uniform guide");
    };
    assert_eq!(settings.size, 8.);

    let count_binding = DesignLayoutGridVariableBinding::new("variable-count", "Desktop columns")
        .with_collection("Breakpoints");
    let columns = DesignLayoutGrid::columns(
        DesignColumnLayoutGrid {
            alignment: DesignColumnGridAlignment::Left,
            count: DesignLayoutGridCount::Auto,
            count_binding: Some(count_binding.clone()),
            size: 64.,
            offset: 32.,
            gutter: 16.,
            margin: 0.,
        },
        DesignColor::PURPLE,
    )
    .with_opacity(18.)
    .with_id("columns-guide");
    assert_eq!(columns.kind(), DesignGridKind::Columns);
    let DesignLayoutGridSettings::Columns(settings) = &columns.settings else {
        panic!("expected column guide");
    };
    assert_eq!(settings.count, DesignLayoutGridCount::Auto);
    assert_eq!(settings.count_binding, Some(count_binding));
    assert!(settings.alignment.supports_offset());
    assert_eq!(columns.opacity, 18.);
    assert_eq!(columns.id.as_ref(), "columns-guide");

    let rows = DesignLayoutGrid::rows(
        DesignRowLayoutGrid {
            alignment: DesignRowGridAlignment::Stretch,
            count: DesignLayoutGridCount::number(0),
            ..DesignRowLayoutGrid::default()
        },
        DesignColor::BLACK,
    );
    let DesignLayoutGridSettings::Rows(settings) = rows.settings else {
        panic!("expected row guide");
    };
    assert_eq!(settings.count, DesignLayoutGridCount::Number(1));
    assert!(settings.alignment.is_stretch());
    assert!(!settings.alignment.supports_offset());
}

#[test]
fn layout_guide_alignment_labels_are_axis_specific() {
    assert_eq!(
        DesignColumnGridAlignment::ALL.map(DesignColumnGridAlignment::label),
        ["Left", "Center", "Right", "Stretch"]
    );
    assert_eq!(
        DesignRowGridAlignment::ALL.map(DesignRowGridAlignment::label),
        ["Top", "Center", "Bottom", "Stretch"]
    );
    assert_eq!(
        DesignGridKind::ALL.map(DesignGridKind::label),
        ["Grid", "Columns", "Rows"]
    );
    for kind in DesignGridKind::ALL {
        assert_eq!(DesignLayoutGridSettings::from(kind).kind(), kind);
    }
}

#[test]
fn auto_layout_extension_defaults_preserve_existing_behavior() {
    let layout = DesignLayout::default();

    assert_eq!(layout.mode, DesignLayoutMode::None);
    assert_eq!(layout.gap, 8.);
    assert_eq!(layout.item_spacing_mode, DesignItemSpacingMode::Fixed);
    assert_eq!(
        layout.counter_axis_align_content,
        DesignCounterAxisAlignContent::Auto
    );
    assert_eq!(layout.counter_axis_gap, None);
    assert_eq!(layout.stacking_order, DesignStackingOrder::LastOnTop);
    assert_eq!(layout.baseline_alignment, DesignBaselineAlignment::Bounds);
    assert!(layout.grid_columns.is_empty());
    assert!(layout.grid_rows.is_empty());
    assert_eq!(layout.grid_auto_tracks, DesignGridAutoTracks::None);
    assert_eq!(
        layout.grid_items_positioning,
        DesignGridItemsPositioning::Manual
    );

    assert_eq!(layout.item.positioning, DesignLayoutPositioning::InFlow);
    assert_eq!(layout.item.align_self, DesignLayoutAlignSelf::Inherit);
    assert_eq!(layout.item.layout_grow, 0.);
    assert_eq!(
        (
            layout.item.min_width,
            layout.item.max_width,
            layout.item.min_height,
            layout.item.max_height,
        ),
        (None, None, None, None)
    );
    assert_eq!(
        (
            layout.item.grid_row_index,
            layout.item.grid_column_index,
            layout.item.grid_row_span,
            layout.item.grid_column_span,
        ),
        (0, 0, 1, 1)
    );
}

#[test]
fn auto_layout_variation_enums_expose_stable_picker_labels() {
    assert_eq!(
        DesignItemSpacingMode::ALL.map(DesignItemSpacingMode::label),
        ["Fixed", "Auto"]
    );
    assert_eq!(
        DesignCounterAxisAlignContent::ALL.map(DesignCounterAxisAlignContent::label),
        ["Auto", "Space between"]
    );
    assert_eq!(
        DesignLayoutPositioning::ALL.map(DesignLayoutPositioning::label),
        ["In auto layout", "Ignore auto layout"]
    );
    assert_eq!(
        DesignLayoutAlignSelf::ALL.map(DesignLayoutAlignSelf::label),
        ["Auto", "Stretch"]
    );
    assert_eq!(
        DesignStackingOrder::ALL.map(DesignStackingOrder::label),
        ["Last on top", "First on top"]
    );
    assert_eq!(
        DesignBaselineAlignment::ALL.map(DesignBaselineAlignment::label),
        ["Bounds", "Baseline"]
    );
    assert_eq!(
        DesignGridTrackSizing::ALL.map(DesignGridTrackSizing::label),
        ["Fixed", "Fraction", "Hug"]
    );
    assert_eq!(
        DesignGridTrackAxis::ALL.map(DesignGridTrackAxis::label),
        ["Column", "Row"]
    );
    assert_eq!(
        DesignGridAutoTracks::ALL.map(DesignGridAutoTracks::label),
        ["Manual rows", "Auto rows"]
    );
    assert_eq!(
        DesignGridItemsPositioning::ALL.map(DesignGridItemsPositioning::label),
        ["Manual", "Auto flow"]
    );
    assert_eq!(
        DesignGridItemAlignment::ALL.map(DesignGridItemAlignment::label),
        ["Auto", "Start", "Center", "End"]
    );
}

#[test]
fn auto_layout_item_represents_limits_absolute_stretch_and_grow() {
    let item = DesignAutoLayoutItem {
        positioning: DesignLayoutPositioning::Absolute,
        align_self: DesignLayoutAlignSelf::Stretch,
        layout_grow: 1.,
        min_width: Some(120.),
        max_width: Some(640.),
        min_height: Some(44.),
        max_height: Some(480.),
        ..DesignAutoLayoutItem::default()
    };

    assert_eq!(item.positioning, DesignLayoutPositioning::Absolute);
    assert_eq!(item.align_self, DesignLayoutAlignSelf::Stretch);
    assert_eq!(item.layout_grow, 1.);
    assert_eq!((item.min_width, item.max_width), (Some(120.), Some(640.)));
    assert_eq!((item.min_height, item.max_height), (Some(44.), Some(480.)));
}

#[test]
fn grid_tracks_and_item_placement_preserve_every_axis() {
    let mut layout = DesignLayout {
        mode: DesignLayoutMode::Grid,
        grid_columns: vec![
            DesignGridTrack::fixed(240.),
            DesignGridTrack::fraction(2.),
            DesignGridTrack::hug(),
        ],
        grid_rows: vec![DesignGridTrack::fraction(1.), DesignGridTrack::fixed(96.)],
        grid_items_positioning: DesignGridItemsPositioning::RowAutoFlow,
        ..DesignLayout::default()
    };
    layout.item.grid_row_index = 2;
    layout.item.grid_column_index = 1;
    layout.item.grid_row_span = 3;
    layout.item.grid_column_span = 2;
    layout.item.grid_horizontal_alignment = DesignGridItemAlignment::Center;
    layout.item.grid_vertical_alignment = DesignGridItemAlignment::End;

    assert_eq!(
        layout.grid_columns,
        vec![
            DesignGridTrack {
                sizing: DesignGridTrackSizing::Fixed,
                value: 240.,
            },
            DesignGridTrack {
                sizing: DesignGridTrackSizing::Fraction,
                value: 2.,
            },
            DesignGridTrack {
                sizing: DesignGridTrackSizing::Hug,
                value: 0.,
            },
        ]
    );
    assert_eq!(
        layout.grid_rows,
        vec![DesignGridTrack::fraction(1.), DesignGridTrack::fixed(96.)]
    );
    assert_eq!(
        (
            layout.item.grid_row_index,
            layout.item.grid_column_index,
            layout.item.grid_row_span,
            layout.item.grid_column_span,
        ),
        (2, 1, 3, 2)
    );
    assert_eq!(
        layout.item.grid_horizontal_alignment,
        DesignGridItemAlignment::Center
    );
    assert_eq!(
        layout.item.grid_vertical_alignment,
        DesignGridItemAlignment::End
    );
}

#[test]
fn design_ui_grid_transition_seeds_hug_tracks_and_automatic_rows() {
    let mut layout = DesignLayout {
        grid_columns: vec![DesignGridTrack::fraction(2.)],
        grid_rows: vec![DesignGridTrack::fixed(96.)],
        ..DesignLayout::default()
    };

    assert!(layout.set_mode_for_kind(DesignPanelNodeKind::Frame, DesignLayoutMode::Grid));

    assert_eq!(layout.horizontal_sizing, DesignSizingMode::Hug);
    assert_eq!(layout.vertical_sizing, DesignSizingMode::Hug);
    assert_eq!(layout.grid_columns, vec![DesignGridTrack::hug()]);
    assert_eq!(layout.grid_rows, vec![DesignGridTrack::hug()]);
    assert_eq!(layout.counter_axis_gap, Some(8.));
    assert_eq!(layout.grid_auto_tracks, DesignGridAutoTracks::Rows);
    assert_eq!(
        layout.grid_items_positioning,
        DesignGridItemsPositioning::RowAutoFlow
    );
    assert!(!layout.grid_track_count_is_editable(DesignGridTrackAxis::Row));
    assert!(layout.grid_track_count_is_editable(DesignGridTrackAxis::Column));
    assert!(!layout.can_delete_grid_track(DesignGridTrackAxis::Column));
    assert_eq!(layout.grid_dimensions(), DesignGridDimensions::new(1, 1));
}

#[test]
fn grid_dimensions_require_both_positive_axes() {
    assert_eq!(
        DesignGridDimensions::new(3, 2),
        Some(DesignGridDimensions {
            columns: 3,
            rows: 2,
        })
    );
    assert!(DesignGridDimensions::new(0, 2).is_none());
    assert!(DesignGridDimensions::new(3, 0).is_none());
    assert!(DesignGridDimensions::new(3, 2).is_some_and(DesignGridDimensions::is_valid));
}

#[test]
fn imported_grid_normalizes_missing_and_invalid_tracks() {
    let mut layout = DesignLayout {
        mode: DesignLayoutMode::Grid,
        horizontal_sizing: DesignSizingMode::Hug,
        grid_columns: vec![
            DesignGridTrack::fixed(0.),
            DesignGridTrack::fraction(f32::NAN),
            DesignGridTrack::fraction(2.),
        ],
        grid_rows: Vec::new(),
        counter_axis_gap: Some(f32::NAN),
        ..DesignLayout::default()
    };

    layout.normalize_for_kind(DesignPanelNodeKind::Frame);

    assert_eq!(
        layout.grid_columns,
        vec![
            DesignGridTrack::hug(),
            DesignGridTrack::hug(),
            DesignGridTrack::hug()
        ]
    );
    assert_eq!(layout.grid_rows, vec![DesignGridTrack::hug()]);
    assert_eq!(layout.counter_axis_gap, Some(8.));
    assert!(DesignGridTrack::fixed(0.5).is_valid());
    assert!(!DesignGridTrack::fraction(0.).is_valid());
}

#[test]
fn node_constructor_populates_new_layout_slice_without_extra_arguments() {
    let node = DesignPanelNode::new("rectangle", "Card", DesignPanelNodeKind::Rectangle);
    let layout = node.layout.expect("rectangle layout");

    assert_eq!(layout.item, DesignAutoLayoutItem::default());
    assert_eq!(layout.item_spacing_mode, DesignItemSpacingMode::Fixed);
    assert_eq!(layout.stacking_order, DesignStackingOrder::LastOnTop);
}

#[test]
fn paint_types_match_figmas_six_top_level_classifications() {
    assert_eq!(
        DesignPaintType::ALL,
        [
            DesignPaintType::Solid,
            DesignPaintType::Gradient,
            DesignPaintType::Pattern,
            DesignPaintType::Image,
            DesignPaintType::Video,
            DesignPaintType::Shader,
        ]
    );
    assert_eq!(
        DesignPaintType::ALL.map(DesignPaintType::label),
        ["Solid", "Gradient", "Pattern", "Image", "Video", "Shader"]
    );
}

#[test]
fn concrete_paint_kinds_classify_without_losing_gradient_subtypes() {
    let expected = [
        (DesignPaintKind::Solid, DesignPaintType::Solid),
        (DesignPaintKind::LinearGradient, DesignPaintType::Gradient),
        (DesignPaintKind::RadialGradient, DesignPaintType::Gradient),
        (DesignPaintKind::AngularGradient, DesignPaintType::Gradient),
        (DesignPaintKind::DiamondGradient, DesignPaintType::Gradient),
        (DesignPaintKind::Pattern, DesignPaintType::Pattern),
        (DesignPaintKind::Image, DesignPaintType::Image),
        (DesignPaintKind::Video, DesignPaintType::Video),
        (DesignPaintKind::Shader, DesignPaintType::Shader),
    ];

    assert_eq!(DesignPaintKind::ALL, expected.map(|(kind, _)| kind));
    for (kind, paint_type) in expected {
        assert_eq!(kind.paint_type(), paint_type);
        assert_eq!(DesignPaintType::from(kind), paint_type);
    }
}

#[test]
fn pattern_is_a_distinct_non_gradient_kind() {
    assert_eq!(DesignPaintKind::Pattern.label(), "Pattern");
    assert!(!DesignPaintKind::Pattern.is_gradient());
    assert_eq!(
        DesignPaintType::Pattern.default_kind(),
        DesignPaintKind::Pattern
    );
}

#[test]
fn top_level_gradient_defaults_to_linear_but_existing_subtypes_remain_exact() {
    assert_eq!(
        DesignPaintType::Gradient.default_kind(),
        DesignPaintKind::LinearGradient
    );

    for kind in [
        DesignPaintKind::LinearGradient,
        DesignPaintKind::RadialGradient,
        DesignPaintKind::AngularGradient,
        DesignPaintKind::DiamondGradient,
    ] {
        let paint = DesignPaint::gradient(
            kind,
            vec![
                DesignGradientStop::new(0., DesignColor::BLACK),
                DesignGradientStop::new(1., DesignColor::WHITE),
            ],
        );
        assert_eq!(paint.kind, kind);
        assert_eq!(paint.paint_type(), DesignPaintType::Gradient);
    }
}

#[test]
fn canonical_paint_payload_retains_ids_bindings_blend_and_gradient_transform() {
    let binding =
        DesignPaintBinding::new("variable:brand", "Brand / Primary").with_collection("Brand");
    let stops = vec![
        DesignGradientStop::new(0., DesignColor::PURPLE)
            .with_id("stop-a")
            .with_binding(binding.clone()),
        DesignGradientStop::new(1., DesignColor::BLUE).with_id("stop-b"),
    ];
    let mut paint = DesignPaint::gradient(DesignPaintKind::DiamondGradient, stops)
        .with_id("paint-gradient")
        .with_blend_mode(DesignBlendMode::Multiply);
    let transform = DesignPaintTransform::IDENTITY
        .rotated(90.)
        .flipped_horizontal();
    assert!(paint.apply_edit(&DesignPaintEdit {
        property: DesignPaintProperty::GradientTransform,
        value: DesignPaintValue::Transform(transform),
    }));

    let DesignPaintPayload::Gradient(gradient) = &paint.payload else {
        panic!("gradient payload");
    };
    assert_eq!(paint.id.as_ref(), "paint-gradient");
    assert_eq!(paint.blend_mode, DesignBlendMode::Multiply);
    assert_eq!(gradient.transform, transform);
    assert_eq!(gradient.stops[0].id.as_ref(), "stop-a");
    assert_eq!(gradient.stops[0].binding, Some(binding));
    assert_eq!(paint.gradient_stops, gradient.stops);
}

#[test]
fn stable_stop_identity_survives_position_reordering() {
    let mut paint = DesignPaint::gradient(
        DesignPaintKind::LinearGradient,
        vec![
            DesignGradientStop::new(0.2, DesignColor::PURPLE).with_id("moving"),
            DesignGradientStop::new(0.8, DesignColor::BLUE).with_id("fixed"),
        ],
    );
    assert!(paint.apply_edit(&DesignPaintEdit {
        property: DesignPaintProperty::GradientStopPosition {
            stop_id: "moving".into(),
            index: 0,
        },
        value: DesignPaintValue::Number(0.95),
    }));

    let DesignPaintPayload::Gradient(gradient) = &paint.payload else {
        panic!("gradient payload");
    };
    assert_eq!(gradient.stops[1].id.as_ref(), "moving");
    assert_eq!(gradient.stops[1].position, 0.95);
    assert_eq!(paint.gradient_stops[1].id.as_ref(), "moving");

    let before = paint.clone();
    assert!(!paint.apply_edit(&DesignPaintEdit {
        property: DesignPaintProperty::GradientStopPosition {
            stop_id: "removed-stop".into(),
            index: 0,
        },
        value: DesignPaintValue::Number(0.1),
    }));
    assert_eq!(
        paint, before,
        "a stale stable ID must not fall back to index"
    );
}

#[test]
fn bound_colors_reject_color_edits_without_blocking_other_metadata() {
    let mut paint = DesignPaint::from_payload(DesignPaintPayload::Solid(DesignSolidPaint {
        color: DesignColor::PURPLE,
        binding: Some(DesignPaintBinding::new("variable:brand", "Brand")),
    }));
    assert!(paint.is_bound());
    assert!(!paint.apply_edit(&DesignPaintEdit {
        property: DesignPaintProperty::Color,
        value: DesignPaintValue::Color(DesignColor::BLUE),
    }));
    assert!(paint.apply_edit(&DesignPaintEdit {
        property: DesignPaintProperty::BlendMode,
        value: DesignPaintValue::BlendMode(DesignBlendMode::Screen),
    }));
    assert_eq!(paint.color, DesignColor::PURPLE);
    assert_eq!(paint.blend_mode, DesignBlendMode::Screen);
}

#[test]
fn media_pattern_and_shader_payloads_preserve_type_specific_state() {
    let image_source = DesignPaintSource::new("asset:image", "Cover");
    let mut image = DesignPaint::image(image_source.clone()).with_id("image-paint");
    assert!(image.apply_edit(&DesignPaintEdit {
        property: DesignPaintProperty::MediaScaleMode,
        value: DesignPaintValue::MediaScaleMode(DesignMediaPaintScaleMode::Crop),
    }));
    let crop_transform = DesignPaintTransform {
        m11: 1.2,
        m12: 0.1,
        m21: -0.1,
        m22: 1.2,
        tx: 0.15,
        ty: -0.05,
    };
    assert!(image.apply_edit(&DesignPaintEdit {
        property: DesignPaintProperty::MediaCropTransform,
        value: DesignPaintValue::Transform(crop_transform),
    }));
    let DesignPaintPayload::Image(image_payload) = &image.payload else {
        panic!("image payload");
    };
    assert_eq!(image_payload.source, image_source);
    assert_eq!(
        image_payload.placement,
        DesignMediaPaintPlacement::Crop {
            transform: crop_transform
        }
    );

    let mut pattern = DesignPaint::pattern("12:34");
    assert!(pattern.apply_edit(&DesignPaintEdit {
        property: DesignPaintProperty::PatternTileType,
        value: DesignPaintValue::PatternTileType(DesignPatternTileType::HorizontalHexagonal,),
    }));
    assert!(pattern.apply_edit(&DesignPaintEdit {
        property: DesignPaintProperty::PatternScalingFactor,
        value: DesignPaintValue::Number(0.75),
    }));
    assert!(pattern.apply_edit(&DesignPaintEdit {
        property: DesignPaintProperty::PatternSpacing,
        value: DesignPaintValue::PatternSpacing(DesignPatternSpacing::new(0.2, 0.3)),
    }));
    assert!(pattern.apply_edit(&DesignPaintEdit {
        property: DesignPaintProperty::PatternHorizontalAlignment,
        value: DesignPaintValue::PatternHorizontalAlignment(DesignPatternHorizontalAlignment::End,),
    }));
    let DesignPaintPayload::Pattern(pattern) = &pattern.payload else {
        panic!("pattern payload");
    };
    assert_eq!(pattern.source_node_id.as_ref(), "12:34");
    assert_eq!(
        pattern.tile_type,
        DesignPatternTileType::HorizontalHexagonal
    );
    assert_eq!(pattern.scaling_factor, 0.75);
    assert_eq!(pattern.spacing, DesignPatternSpacing::new(0.2, 0.3));
    assert_eq!(
        pattern.horizontal_alignment,
        DesignPatternHorizontalAlignment::End
    );

    let mut shader = DesignPaint::from_payload(DesignPaintPayload::Shader(
        DesignShaderPaint::new("runtime-shader", "Fractal noise").with_properties([
            DesignShaderPropertyAssignment::new("scale", DesignShaderPropertyValue::Number(4.)),
        ]),
    ));
    assert!(shader.apply_edit(&DesignPaintEdit {
        property: DesignPaintProperty::ShaderProperty {
            definition_id: "scale".into(),
        },
        value: DesignPaintValue::ShaderProperty(DesignShaderPropertyValue::Number(8.)),
    }));
    assert!(shader.apply_edit(&DesignPaintEdit {
        property: DesignPaintProperty::ShaderProperty {
            definition_id: "tint".into(),
        },
        value: DesignPaintValue::ShaderProperty(DesignShaderPropertyValue::VariableAlias {
            variable_id: "variable:tint".into(),
        },),
    }));
    let DesignPaintPayload::Shader(shader_payload) = &shader.payload else {
        panic!("shader payload");
    };
    assert_eq!(shader_payload.shader_id.as_ref(), "runtime-shader");
    assert_eq!(
        shader_payload.property("scale"),
        Some(&DesignShaderPropertyValue::Number(8.))
    );
    assert!(matches!(
        shader_payload.property("tint"),
        Some(DesignShaderPropertyValue::VariableAlias { variable_id })
            if variable_id.as_ref() == "variable:tint"
    ));
    assert_eq!(shader.paint_type(), DesignPaintType::Shader);
}

#[test]
fn shader_view_data_preserves_library_identity_import_state_and_definition_ids() {
    let definition =
        DesignShaderPropertyDefinition::new("def:scale", "Scale", DesignShaderPropertyKind::Number)
            .with_default(DesignShaderPropertyValue::Number(2.))
            .with_description("Frequency multiplier");
    let local =
        DesignShaderDefinition::new("shader:local", "Local waves", true, [definition.clone()]);
    let remote = DesignShaderDefinition::new("shader:remote", "Library noise", false, []);
    let view_data = DesignShaderViewData::new(
        [local.clone()],
        [DesignShaderLibrary::new(
            "shader-library",
            "Shader lab",
            [remote.clone()],
        )],
    );

    assert_eq!(
        view_data
            .shader(&DesignShaderSelection::page("shader:local"))
            .and_then(|shader| shader.property("def:scale")),
        Some(&definition)
    );
    assert_eq!(
        DesignShaderPaint::from_definition(&local)
            .and_then(|paint| paint.property("def:scale").cloned()),
        Some(DesignShaderPropertyValue::Number(2.))
    );
    assert_eq!(
        view_data
            .shader(&DesignShaderSelection::library(
                "shader-library",
                "shader:remote"
            ))
            .map(|shader| shader.imported),
        Some(false)
    );
    assert_eq!(
        view_data
            .definition("shader:remote")
            .map(|shader| &shader.name),
        Some(&remote.name)
    );
    assert!(DesignShaderPaint::from_definition(&remote).is_none());
}

#[test]
fn shader_host_actions_keep_stable_paint_library_definition_and_variable_ids() {
    let import = DesignPanelAction::PaintShaderImportRequested {
        node_id: "node".into(),
        collection: DesignPanelCollection::Fill,
        target: DesignPaintTarget::WholeLayer,
        paint_id: "fill:shader".into(),
        index: 3,
        shader: DesignShaderSelection::library("library:effects", "shader:noise"),
    };
    assert!(matches!(
        import,
        DesignPanelAction::PaintShaderImportRequested {
            paint_id,
            index: 3,
            shader: DesignShaderSelection {
                source: DesignShaderSource::Library { library_id },
                shader_id,
            },
            ..
        } if paint_id.as_ref() == "fill:shader"
            && library_id.as_ref() == "library:effects"
            && shader_id.as_ref() == "shader:noise"
    ));

    let detach = DesignPanelAction::PaintShaderPropertyDetachRequested {
        node_id: "node".into(),
        collection: DesignPanelCollection::Stroke,
        target: DesignPaintTarget::WholeLayer,
        paint_id: "stroke:shader".into(),
        index: 1,
        definition_id: "def:tint".into(),
        variable_id: "variable:brand".into(),
    };
    assert!(matches!(
        detach,
        DesignPanelAction::PaintShaderPropertyDetachRequested {
            definition_id,
            variable_id,
            ..
        } if definition_id.as_ref() == "def:tint"
            && variable_id.as_ref() == "variable:brand"
    ));
}

#[test]
fn media_edits_reject_wrong_modes_and_nonfinite_values_and_bound_filters() {
    let mut image = DesignPaint::image(DesignPaintSource::new("asset:image", "Cover"));
    assert!(!image.apply_edit(&DesignPaintEdit {
        property: DesignPaintProperty::MediaCropTransform,
        value: DesignPaintValue::Transform(DesignPaintTransform::IDENTITY),
    }));
    assert!(!image.apply_edit(&DesignPaintEdit {
        property: DesignPaintProperty::MediaTileScalingFactor,
        value: DesignPaintValue::Number(2.),
    }));
    assert!(image.apply_edit(&DesignPaintEdit {
        property: DesignPaintProperty::MediaQuarterTurn,
        value: DesignPaintValue::MediaQuarterTurn(DesignMediaQuarterTurn::Clockwise90,),
    }));

    assert!(image.apply_edit(&DesignPaintEdit {
        property: DesignPaintProperty::MediaScaleMode,
        value: DesignPaintValue::MediaScaleMode(DesignMediaPaintScaleMode::Crop),
    }));
    assert!(!image.apply_edit(&DesignPaintEdit {
        property: DesignPaintProperty::MediaQuarterTurn,
        value: DesignPaintValue::MediaQuarterTurn(DesignMediaQuarterTurn::Clockwise180,),
    }));
    let mut invalid_transform = DesignPaintTransform::IDENTITY;
    invalid_transform.tx = f32::NAN;
    assert!(!image.apply_edit(&DesignPaintEdit {
        property: DesignPaintProperty::MediaCropTransform,
        value: DesignPaintValue::Transform(invalid_transform),
    }));

    assert!(image.apply_edit(&DesignPaintEdit {
        property: DesignPaintProperty::MediaFilter(DesignImageFilter::Exposure),
        value: DesignPaintValue::Number(2.),
    }));
    assert!(!image.apply_edit(&DesignPaintEdit {
        property: DesignPaintProperty::MediaFilter(DesignImageFilter::Contrast),
        value: DesignPaintValue::Number(f32::INFINITY),
    }));
    let DesignPaintPayload::Image(image) = image.payload else {
        panic!("image payload");
    };
    assert_eq!(image.filters.exposure, 1.);
    assert_eq!(image.filters.contrast, 0.);
}

#[test]
fn media_view_state_and_preview_actions_keep_stable_paint_identity() {
    assert_eq!(
        DesignMediaSourceAction::ALL.map(DesignMediaSourceAction::label),
        ["Upload from computer", "Make an image", "Edit image"]
    );
    assert!(
        DesignMediaSourceAction::Upload.is_applicable_to(DesignPaintType::Image)
            && DesignMediaSourceAction::Upload.is_applicable_to(DesignPaintType::Video)
    );
    assert!(
        DesignMediaSourceAction::MakeImage.is_applicable_to(DesignPaintType::Image)
            && !DesignMediaSourceAction::MakeImage.is_applicable_to(DesignPaintType::Video)
            && !DesignMediaSourceAction::EditImage.is_applicable_to(DesignPaintType::Solid)
    );
    let selective_capabilities =
        DesignMediaPaintCapabilities::editor().with_source_actions(true, false, true);
    assert!(selective_capabilities.allows_source_action(DesignMediaSourceAction::Upload));
    assert!(!selective_capabilities.allows_source_action(DesignMediaSourceAction::MakeImage));
    assert!(selective_capabilities.allows_source_action(DesignMediaSourceAction::EditImage));

    let view_data = DesignMediaPaintViewData::new([DesignMediaPaintView::new(
        DesignPanelCollection::Fill,
        "video-fill",
        4,
    )
    .with_capabilities(DesignMediaPaintCapabilities::property_editor_only())
    .with_video_preview(DesignVideoPreviewState::ready(30., 8., false))]);
    let view = view_data
        .paint(DesignPanelCollection::Fill, &"video-fill".into(), 99)
        .expect("stable paint id resolves independently of the stale index");
    assert!(view.capabilities.can_edit_properties);
    assert!(!view.capabilities.can_upload_source);
    assert!(
        DesignMediaPaintCapabilities::editor()
            .allows_source_action(DesignMediaSourceAction::MakeImage)
    );

    let action = DesignPanelAction::PaintVideoPreviewActionRequested {
        node_id: "node".into(),
        collection: DesignPanelCollection::Fill,
        target: DesignPaintTarget::WholeLayer,
        paint_id: "video-fill".into(),
        index: 4,
        action: DesignVideoPreviewAction::Scrub {
            seconds: 12.,
            phase: DesignPanelEditPhase::Preview,
        },
    };
    let DesignPanelAction::PaintVideoPreviewActionRequested {
        paint_id,
        action:
            DesignVideoPreviewAction::Scrub {
                seconds,
                phase: DesignPanelEditPhase::Preview,
            },
        ..
    } = action
    else {
        panic!("video scrub action");
    };
    assert_eq!(paint_id.as_ref(), "video-fill");
    assert_eq!(seconds, 12.);
}

#[test]
fn media_file_drop_classification_is_exact_case_insensitive_and_host_gated() {
    use std::path::PathBuf;

    for (path, expected) in [
        ("cover.JPG", DesignMediaFileKind::Jpeg),
        ("cover.jpeg", DesignMediaFileKind::Jpeg),
        ("cover.PnG", DesignMediaFileKind::Png),
        ("cover.HEIC", DesignMediaFileKind::Heic),
        ("cover.webp", DesignMediaFileKind::Webp),
        ("cover.GIF", DesignMediaFileKind::Gif),
        ("clip.MP4", DesignMediaFileKind::Mp4),
        ("clip.mov", DesignMediaFileKind::Mov),
        ("clip.WeBm", DesignMediaFileKind::Webm),
    ] {
        let dropped = DesignMediaDroppedFile::from_paths(
            &[PathBuf::from(path)],
            DesignMediaFileKinds::STANDARD,
        )
        .expect("standard media path");
        assert_eq!(dropped.kind, expected);
        assert_eq!(dropped.media_kind(), expected.media_kind());
    }

    for rejected in ["art.svg", "document.PDF", "archive.zip", "extensionless"] {
        assert_eq!(
            DesignMediaDroppedFile::from_paths(
                &[PathBuf::from(rejected)],
                DesignMediaFileKinds::ALL,
            ),
            None
        );
    }
    assert_eq!(
        DesignMediaDroppedFile::from_paths(
            &[PathBuf::from("first.png"), PathBuf::from("second.png")],
            DesignMediaFileKinds::STANDARD,
        ),
        None,
        "multi-file drops are atomic rejections"
    );
    assert_eq!(
        DesignMediaDroppedFile::from_paths(&[], DesignMediaFileKinds::STANDARD),
        None
    );
    assert_eq!(
        DesignMediaDroppedFile::from_paths(
            &[PathBuf::from("scan.tiff")],
            DesignMediaFileKinds::STANDARD,
        ),
        None,
        "TIFF requires host opt-in"
    );
    assert_eq!(
        DesignMediaDroppedFile::from_paths(
            &[PathBuf::from("scan.TIF")],
            DesignMediaFileKinds::STANDARD | DesignMediaFileKinds::TIFF,
        )
        .map(|file| file.kind),
        Some(DesignMediaFileKind::Tiff)
    );
    assert!(DesignMediaPaintCapabilities::editor().allows_file_drop(DesignMediaFileKind::Png));
    assert!(
        !DesignMediaPaintCapabilities::property_editor_only()
            .allows_file_drop(DesignMediaFileKind::Png)
    );
}

#[test]
fn typed_paint_actions_carry_stable_and_legacy_targets_with_edit_phases() {
    let action = DesignPanelAction::PaintEditRequested {
        node_id: "node".into(),
        collection: DesignPanelCollection::Fill,
        target: DesignPaintTarget::WholeLayer,
        paint_id: "fill-primary".into(),
        index: 2,
        edit: DesignPaintEdit {
            property: DesignPaintProperty::MediaQuarterTurn,
            value: DesignPaintValue::MediaQuarterTurn(DesignMediaQuarterTurn::Clockwise90),
        },
        phase: DesignPanelEditPhase::Preview,
    };
    assert!(matches!(
        action,
        DesignPanelAction::PaintEditRequested {
            paint_id,
            index: 2,
            phase: DesignPanelEditPhase::Preview,
            ..
        } if paint_id.as_ref() == "fill-primary"
    ));
}

#[test]
fn paint_picker_host_requests_preserve_color_leaf_and_stable_paint_identity() {
    let color_target = DesignPaintColorTarget::GradientStop {
        stop_id: "brand-stop".into(),
        index: 4,
    };
    let create = DesignPanelAction::PaintColorVariableCreateRequested {
        node_id: "node".into(),
        collection: DesignPanelCollection::Stroke,
        target: DesignPaintTarget::WholeLayer,
        paint_id: "stroke-primary".into(),
        index: 2,
        color_target: color_target.clone(),
        color: DesignColor::rgba(12, 34, 56, 78),
    };
    assert!(matches!(
        create,
        DesignPanelAction::PaintColorVariableCreateRequested {
            paint_id,
            index: 2,
            color_target: DesignPaintColorTarget::GradientStop {
                stop_id,
                index: 4,
            },
            color: DesignColor {
                red: 12,
                green: 34,
                blue: 56,
                alpha: 78,
            },
            ..
        } if paint_id.as_ref() == "stroke-primary" && stop_id.as_ref() == "brand-stop"
    ));

    let eyedropper = DesignPanelAction::PaintEyedropperRequested {
        node_id: "node".into(),
        collection: DesignPanelCollection::Stroke,
        target: DesignPaintTarget::WholeLayer,
        paint_id: "stroke-primary".into(),
        index: 2,
        color_target,
    };
    assert!(matches!(
        eyedropper,
        DesignPanelAction::PaintEyedropperRequested {
            paint_id,
            color_target: DesignPaintColorTarget::GradientStop {
                stop_id,
                index: 4,
            },
            ..
        } if paint_id.as_ref() == "stroke-primary" && stop_id.as_ref() == "brand-stop"
    ));
}

#[test]
fn paint_styles_and_color_variables_have_disjoint_binding_levels() {
    let style = DesignPaintStyle::new(
        "style:hero",
        "Hero",
        [
            DesignPaint::solid(DesignColor::PURPLE).with_id("style-paint-a"),
            DesignPaint::gradient(
                DesignPaintKind::LinearGradient,
                vec![
                    DesignGradientStop::new(0., DesignColor::BLACK).with_id("style-stop-a"),
                    DesignGradientStop::new(1., DesignColor::WHITE).with_id("style-stop-b"),
                ],
            )
            .with_id("style-paint-b"),
        ],
    );
    let selection = DesignPaintStyleSelection::library("library:brand", "style:hero");
    let view_data = DesignPaintStyleViewData::new(
        [],
        [DesignPaintStyleLibrary::new(
            "library:brand",
            "Brand",
            [style.clone()],
        )],
    );
    assert_eq!(view_data.style(&selection), Some(&style));

    let binding = DesignPaintStyleBinding::new(selection.clone(), "Hero");
    let mut node = DesignPanelNode::new("node", "Card", DesignPanelNodeKind::Rectangle);
    node.fill_style_binding = Some(binding);
    assert_eq!(
        node.fill_style_binding
            .as_ref()
            .map(|binding| &binding.selection),
        Some(&selection)
    );
    assert!(
        node.fills.iter().all(|paint| match &paint.payload {
            DesignPaintPayload::Solid(solid) => solid.binding.is_none(),
            DesignPaintPayload::Gradient(gradient) => {
                gradient.stops.iter().all(|stop| stop.binding.is_none())
            }
            _ => true,
        }),
        "a collection Paint-style binding does not invent leaf variable bindings"
    );

    let variable = DesignVariable::page(
        "variable:brand",
        "Brand",
        "collection:colors",
        "Colors",
        DesignVariableResolvedType::Color,
    )
    .with_resolved_value(DesignVariableResolvedValue::Color(DesignColor::PURPLE));
    let variables = DesignPaintVariableViewData::new([variable.clone()]);
    assert_eq!(variables.variable("variable:brand"), Some(&variable));
    assert!(variables.variable("style:hero").is_none());
}

#[test]
fn style_browser_catalogs_disambiguate_page_and_library_style_ids() {
    let page_paint = DesignPaintStyle::new(
        "shared",
        "Page paint",
        [DesignPaint::solid(DesignColor::BLACK)],
    );
    let library_paint = DesignPaintStyle::new(
        "shared",
        "Library paint",
        [DesignPaint::solid(DesignColor::WHITE)],
    );
    let paints = DesignPaintStyleViewData::new(
        [page_paint.clone()],
        [DesignPaintStyleLibrary::new(
            "brand",
            "Brand",
            [library_paint.clone()],
        )],
    );
    assert_eq!(
        paints.style(&DesignPaintStyleSelection::page("shared")),
        Some(&page_paint)
    );
    assert_eq!(
        paints.style(&DesignPaintStyleSelection::library("brand", "shared")),
        Some(&library_paint)
    );

    let page_effect =
        DesignEffectStyle::new("shared", "Page effect", [DesignEffectKind::LayerBlur]);
    let library_effect =
        DesignEffectStyle::new("shared", "Library effect", [DesignEffectKind::DropShadow]);
    let effects = DesignEffectStyleViewData::new(
        [page_effect.clone()],
        [DesignEffectStyleLibrary::new(
            "brand",
            "Brand",
            [library_effect.clone()],
        )],
    );
    assert_eq!(
        effects.style(&DesignEffectStyleSelection::page("shared")),
        Some(&page_effect)
    );
    assert_eq!(
        effects.style(&DesignEffectStyleSelection::library("brand", "shared")),
        Some(&library_effect)
    );

    let page_grid = DesignLayoutGridStyle::new(
        "shared",
        "Page grid",
        [DesignLayoutGrid::uniform(8., DesignColor::BLUE)],
    );
    let library_grid = DesignLayoutGridStyle::new(
        "shared",
        "Library grid",
        [DesignLayoutGrid::uniform(4., DesignColor::PURPLE)],
    );
    let grids = DesignLayoutGridStyleViewData::new(
        [page_grid.clone()],
        [DesignLayoutGridStyleLibrary::new(
            "brand",
            "Brand",
            [library_grid.clone()],
        )],
    );
    assert_eq!(
        grids.style(&DesignLayoutGridStyleSelection::page("shared")),
        Some(&page_grid)
    );
    assert_eq!(
        grids.style(&DesignLayoutGridStyleSelection::library("brand", "shared")),
        Some(&library_grid)
    );
}

#[test]
fn effect_kinds_have_distinct_typed_default_settings() {
    assert_eq!(
        DesignEffectKind::ALL.map(DesignEffectKind::label),
        [
            "Drop shadow",
            "Inner shadow",
            "Layer blur",
            "Background blur",
            "Noise",
            "Texture",
            "Glass",
            "Shader",
        ]
    );

    for kind in DesignEffectKind::ALL {
        let effect = DesignEffect::new(kind);
        assert_eq!(effect.kind, kind);
        assert_eq!(effect.settings.kind(), kind);
        assert!(effect.visible);
    }

    assert_eq!(DesignEffectKind::DropShadow.maximum_per_node(), 8);
    assert_eq!(DesignEffectKind::InnerShadow.maximum_per_node(), 8);
    assert_eq!(DesignEffectKind::Noise.maximum_per_node(), 2);
    for kind in [
        DesignEffectKind::LayerBlur,
        DesignEffectKind::BackgroundBlur,
        DesignEffectKind::Texture,
        DesignEffectKind::Glass,
    ] {
        assert_eq!(kind.maximum_per_node(), 1);
    }
    assert_eq!(DesignEffectKind::Shader.maximum_per_node(), u8::MAX);
    assert_eq!(DesignEffectKind::Unsupported.maximum_per_node(), 0);
}

#[test]
fn effect_ids_capabilities_and_limits_are_host_controlled() {
    let mut node = DesignPanelNode::new("effects", "Effects", DesignPanelNodeKind::Rectangle);
    for index in 0..8 {
        node.effects.push(
            DesignEffect::new(DesignEffectKind::DropShadow).with_id(format!("shadow-{index}")),
        );
    }
    assert_eq!(node.effect_index_by_id("shadow-3"), Some(3));
    assert!(!node.can_use_effect_kind(DesignEffectKind::DropShadow, None));
    assert!(node.can_use_effect_kind(DesignEffectKind::DropShadow, Some(3)));
    assert!(node.effect_capabilities.shadow_spread);
    assert!(
        !node
            .effect_capabilities
            .kind_is_available(DesignEffectKind::Shader)
    );

    node.effect_capabilities
        .set_kind_availability(DesignEffectKindAvailability::available(
            DesignEffectKind::Shader,
        ));
    assert!(node.can_use_effect_kind(DesignEffectKind::Shader, None));
}

#[test]
fn effect_styles_variables_shader_and_future_payloads_round_trip() {
    let selection = DesignEffectStyleSelection::library("library", "raised");
    let view_data = DesignEffectStyleViewData::new(
        [],
        [DesignEffectStyleLibrary::new(
            "library",
            "Library",
            [DesignEffectStyle::new(
                "raised",
                "Raised",
                [DesignEffectKind::DropShadow],
            )],
        )],
    );
    assert_eq!(
        view_data.style(&selection).map(|style| style.name.as_ref()),
        Some("Raised")
    );

    let mut effect = DesignEffect::new(DesignEffectKind::DropShadow)
        .with_id("shadow")
        .with_variable_binding(DesignEffectVariableBinding::new(
            DesignEffectVariableField::Color,
            "shadow-color",
            "Shadow",
            "Semantic",
        ));
    assert_eq!(
        effect
            .variable_binding(DesignEffectVariableField::Color)
            .map(|binding| binding.variable_id.as_ref()),
        Some("shadow-color")
    );
    effect.set_kind(DesignEffectKind::Shader);
    assert!(effect.variable_bindings.is_empty());

    effect.set_settings(DesignEffectSettings::Shader(DesignShaderEffect::new(
        "shader",
        "Noise",
        [DesignShaderProperty::new(
            "amount",
            "Amount",
            DesignShaderPropertyKind::Number,
            DesignShaderPropertyValue::Number(0.5),
        )],
    )));
    let DesignEffectSettings::Shader(shader) = &effect.settings else {
        panic!("shader effect");
    };
    assert_eq!(shader.properties[0].definition_id.as_ref(), "amount");

    effect.set_settings(DesignEffectSettings::Opaque(DesignOpaqueEffect::new(
        "FUTURE",
        "Future",
        "{\"preserve\":true}",
    )));
    let DesignEffectSettings::Opaque(opaque) = &effect.settings else {
        panic!("opaque effect");
    };
    assert_eq!(opaque.payload.as_ref(), "{\"preserve\":true}");
}

#[test]
fn effect_shader_complex_values_and_editor_targets_remain_typed() {
    let origin = DesignEffectVector::new(0., 0.);
    let values = [
        (
            DesignShaderPropertyKind::Boolean,
            DesignShaderPropertyValue::Boolean(true),
        ),
        (
            DesignShaderPropertyKind::Text,
            DesignShaderPropertyValue::Text("Text".into()),
        ),
        (
            DesignShaderPropertyKind::Number,
            DesignShaderPropertyValue::Number(1.),
        ),
        (
            DesignShaderPropertyKind::Image,
            DesignShaderPropertyValue::AssetId("image".into()),
        ),
        (
            DesignShaderPropertyKind::InstanceSwap,
            DesignShaderPropertyValue::AssetId("component".into()),
        ),
        (
            DesignShaderPropertyKind::Slot,
            DesignShaderPropertyValue::AssetId("slot".into()),
        ),
        (
            DesignShaderPropertyKind::Color,
            DesignShaderPropertyValue::Color(DesignColor::BLUE),
        ),
        (
            DesignShaderPropertyKind::Point,
            DesignShaderPropertyValue::Point(origin),
        ),
        (
            DesignShaderPropertyKind::Line,
            DesignShaderPropertyValue::Line {
                start: origin,
                end: DesignEffectVector::new(1., 1.),
            },
        ),
        (
            DesignShaderPropertyKind::Circle,
            DesignShaderPropertyValue::Circle {
                center: origin,
                radius: 1.,
            },
        ),
        (
            DesignShaderPropertyKind::CirclePoint,
            DesignShaderPropertyValue::CirclePoint {
                center: origin,
                radius: 1.,
                angle: 45.,
            },
        ),
        (
            DesignShaderPropertyKind::ColorPoint,
            DesignShaderPropertyValue::ColorPoint {
                point: origin,
                color: DesignColor::PURPLE,
                variable_id: Some("variable:color".into()),
            },
        ),
        (
            DesignShaderPropertyKind::Gradient,
            DesignShaderPropertyValue::Gradient(vec![
                DesignShaderGradientStop::new(0., DesignColor::BLACK),
                DesignShaderGradientStop::new(1., DesignColor::WHITE),
            ]),
        ),
        (
            DesignShaderPropertyKind::Unsupported,
            DesignShaderPropertyValue::Opaque {
                type_name: "FUTURE".into(),
                payload: "{}".into(),
            },
        ),
    ];
    assert!(
        values
            .iter()
            .all(|(kind, value)| value.is_compatible_with(*kind))
    );
    assert!(
        DesignShaderPropertyValue::VariableAlias {
            variable_id: "variable:any".into()
        }
        .is_compatible_with(DesignShaderPropertyKind::Gradient)
    );

    let request = DesignPanelAction::EffectShaderPropertyEditorRequested {
        node_id: "node".into(),
        effect_id: "effect".into(),
        index: 4,
        shader_property_id: "definition".into(),
        property_index: 7,
        property_kind: DesignShaderPropertyKind::Gradient,
        target: DesignShaderPropertyEditorTarget::GradientStopColor(2),
        editor: DesignShaderPropertyEditorKind::Variable,
        current_value: DesignShaderPropertyValue::Gradient(Vec::new()),
    };
    assert!(matches!(
        request,
        DesignPanelAction::EffectShaderPropertyEditorRequested {
            effect_id,
            index: 4,
            shader_property_id,
            property_index: 7,
            target: DesignShaderPropertyEditorTarget::GradientStopColor(2),
            editor: DesignShaderPropertyEditorKind::Variable,
            ..
        } if effect_id.as_ref() == "effect"
            && shader_property_id.as_ref() == "definition"
    ));
}

#[test]
fn background_blur_and_glass_keep_host_order() {
    let mut effects = vec![
        DesignEffect::new(DesignEffectKind::Glass).with_id("glass"),
        DesignEffect::new(DesignEffectKind::BackgroundBlur).with_id("blur"),
    ];
    let effect = effects.remove(1);
    effects.insert(0, effect);
    assert_eq!(effects[0].id.as_ref(), "blur");
    assert_eq!(effects[1].id.as_ref(), "glass");
}

#[test]
fn typed_effect_settings_preserve_controls_unique_to_each_effect() {
    let progressive = DesignBlurEffect::progressive(
        2.,
        24.,
        DesignEffectVector::new(0.1, 0.2),
        DesignEffectVector::new(0.8, 0.9),
    );
    let noise = DesignNoiseEffect {
        colors: DesignNoiseColors::Duotone {
            color: DesignColor::BLUE,
            secondary_color: DesignColor::PURPLE,
        },
        size: DesignEffectVector::new(3., 5.),
        density: 0.72,
        blend_mode: DesignBlendMode::Overlay,
    };
    let texture = DesignTextureEffect {
        size: DesignEffectVector::new(2., 4.),
        radius: 12.,
        clip_to_shape: true,
    };
    let glass = DesignGlassEffect {
        light_intensity: 0.8,
        light_angle: 135.,
        refraction: 0.64,
        depth: 18.,
        dispersion: 0.3,
        frost: 14.,
        splay: 0.77,
    };

    assert_eq!(progressive.blur_type(), DesignBlurType::Progressive);
    assert_eq!(progressive.end_radius(), 24.);
    assert_eq!(noise.colors.noise_type(), DesignNoiseType::Duotone);
    assert_eq!(noise.size, DesignEffectVector::new(3., 5.));
    assert!(texture.clip_to_shape);
    assert_eq!(glass.splay, 0.77);

    assert_eq!(
        DesignEffectSettings::LayerBlur(progressive).kind(),
        DesignEffectKind::LayerBlur
    );
    assert_eq!(
        DesignEffectSettings::Noise(noise).kind(),
        DesignEffectKind::Noise
    );
    assert_eq!(
        DesignEffectSettings::Texture(texture).kind(),
        DesignEffectKind::Texture
    );
    assert_eq!(
        DesignEffectSettings::Glass(glass).kind(),
        DesignEffectKind::Glass
    );
}

#[test]
fn compact_effect_fields_and_typed_shadow_settings_stay_in_sync() {
    let mut effect = DesignEffect::drop_shadow(DesignColor::rgba(10, 20, 30, 40), 24., -2., 3., 8.);

    effect.set_blur(30.);
    effect.set_spread(4.);
    effect.set_offset_x(-6.);
    effect.set_offset_y(12.);

    assert_eq!(effect.blur, 30.);
    assert_eq!(effect.spread, 4.);
    assert_eq!((effect.offset_x, effect.offset_y), (-6., 12.));
    let DesignEffectSettings::DropShadow(settings) = &effect.settings else {
        panic!("drop shadow settings");
    };
    assert_eq!(settings.radius, 30.);
    assert_eq!(settings.spread, 4.);
    assert_eq!(settings.offset, DesignEffectVector::new(-6., 12.));

    effect.visible = false;
    effect.set_kind(DesignEffectKind::Glass);
    assert!(!effect.visible);
    assert_eq!(effect.kind, DesignEffectKind::Glass);
    assert!(matches!(effect.settings, DesignEffectSettings::Glass(_)));
}

#[test]
fn effect_specific_leaf_properties_have_typed_values() {
    let edits = [
        (
            DesignPanelProperty::EffectShadowColor(0),
            DesignPanelValue::Color(DesignColor::BLACK),
        ),
        (
            DesignPanelProperty::EffectShadowBlendMode(0),
            DesignPanelValue::BlendMode(DesignBlendMode::Multiply),
        ),
        (
            DesignPanelProperty::EffectBlurType(1),
            DesignPanelValue::EffectBlurType(DesignBlurType::Progressive),
        ),
        (
            DesignPanelProperty::EffectNoiseType(2),
            DesignPanelValue::EffectNoiseType(DesignNoiseType::Duotone),
        ),
        (
            DesignPanelProperty::EffectTextureClipToShape(3),
            DesignPanelValue::Bool(true),
        ),
        (
            DesignPanelProperty::EffectGlassDepth(4),
            DesignPanelValue::Number(16.),
        ),
    ];

    assert_eq!(edits.len(), 6);
    assert!(matches!(edits[0].1, DesignPanelValue::Color(_)));
    assert!(matches!(
        edits[2].1,
        DesignPanelValue::EffectBlurType(DesignBlurType::Progressive)
    ));
    assert!(matches!(
        edits[3].1,
        DesignPanelValue::EffectNoiseType(DesignNoiseType::Duotone)
    ));
}

#[test]
fn text_resize_and_truncation_gate_nullable_max_lines() {
    assert_eq!(
        DesignTextResize::ALL.map(DesignTextResize::label),
        ["Auto width", "Auto height", "Fixed size"]
    );

    let mut typography = DesignTypography::default();
    assert_eq!(typography.resize, DesignTextResize::AutoHeight);
    assert!(!typography.truncate);
    assert_eq!(typography.max_lines, None);
    assert!(!typography.text_max_lines_are_available());

    typography.truncate = true;
    typography.max_lines = Some(3);
    assert!(typography.text_max_lines_are_available());
    assert!(typography.truncate);
    assert_eq!(typography.max_lines, Some(3));

    typography.resize = DesignTextResize::Fixed;
    assert!(!typography.text_max_lines_are_available());
}

#[test]
fn text_height_limits_are_contextual_mutually_exclusive_and_lossless() {
    let mut node = DesignPanelNode::new("text", "Text", DesignPanelNodeKind::Text);
    let typography = node.typography.as_mut().expect("text typography");
    typography.truncate = true;
    typography.resize = DesignTextResize::AutoHeight;
    node.layout.as_mut().expect("text layout").vertical_sizing = DesignSizingMode::Hug;

    assert!(node.text_max_lines_are_available(false));
    assert!(node.text_max_lines_are_available(true));
    assert!(!node.set_text_max_lines(Some(0), true));
    assert!(!node.set_layout_max_height(Some(0.)));
    assert!(node.set_layout_max_height(Some(96.)));
    assert_eq!(
        node.layout.as_ref().expect("text layout").item.max_height,
        Some(96.)
    );
    assert_eq!(
        node.typography.as_ref().expect("text typography").max_lines,
        None
    );

    assert!(node.set_text_max_lines(Some(3), true));
    assert_eq!(
        node.typography.as_ref().expect("text typography").max_lines,
        Some(3)
    );
    assert_eq!(
        node.layout.as_ref().expect("text layout").item.max_height,
        None
    );

    assert!(node.set_text_max_lines(None, true));
    assert_eq!(
        node.typography.as_ref().expect("text typography").max_lines,
        None
    );

    node.layout.as_mut().expect("text layout").vertical_sizing = DesignSizingMode::Fixed;
    assert!(!node.text_max_lines_are_available(true));
    assert!(!node.set_text_max_lines(Some(2), true));
    assert!(node.set_text_max_lines(None, true));
    assert!(node.text_max_lines_are_available(false));

    assert!(node.set_text_max_lines(Some(2), false));
    assert!(node.set_text_resize(DesignTextResize::Fixed));
    assert_eq!(
        node.typography.as_ref().expect("text typography").max_lines,
        None
    );
    assert!(node.set_text_resize(DesignTextResize::AutoWidth));
    assert!(node.set_text_max_lines(Some(4), false));
    assert!(node.set_text_truncation(false));
    assert_eq!(
        node.typography.as_ref().expect("text typography").max_lines,
        None
    );
}

#[test]
fn typography_preserves_units_and_typed_choices() {
    let typography = DesignTypography {
        weight: 520.,
        line_height: DesignLineHeight::Percent(150.),
        letter_spacing: DesignLetterSpacing::Pixels(-0.25),
        leading_trim: DesignTextLeadingTrim::CapHeight,
        horizontal_alignment: DesignTextHorizontalAlignment::Justified,
        vertical_alignment: DesignTextVerticalAlignment::Bottom,
        decoration: DesignTextDecoration::Underline,
        case: DesignTextCase::SmallCaps,
        list: DesignTextList::Numbered,
        ..DesignTypography::default()
    };

    assert_eq!(typography.line_height, DesignLineHeight::Percent(150.));
    assert_eq!(typography.weight, 520.);
    assert_eq!(
        typography.letter_spacing,
        DesignLetterSpacing::Pixels(-0.25)
    );
    assert_eq!(typography.leading_trim.label(), "Cap height");
    assert_eq!(typography.horizontal_alignment.label(), "Justified");
    assert_eq!(typography.vertical_alignment.label(), "Bottom");
    assert_eq!(typography.case.label(), "Small Caps");
    assert_eq!(typography.decoration.label(), "Underline");
    assert_eq!(typography.list.label(), "Numbered list");
}

#[test]
fn typography_details_preserve_list_decoration_and_lossless_open_type_features() {
    let registered =
        DesignOpenTypeFeatureTag::registered("liga").expect("liga is a registered tag");
    let opaque = DesignOpenTypeFeatureTag::opaque("future:round-dots");
    let typography = DesignTypography {
        paragraph_indent: 24.,
        list_spacing: 8.,
        hanging_punctuation: true,
        hanging_lists: true,
        decoration: DesignTextDecoration::Underline,
        decoration_details: Some(DesignTextDecorationDetails {
            style: DesignTextDecorationStyle::Wavy,
            offset: DesignTextDecorationMetric::Pixels(-1.),
            thickness: DesignTextDecorationMetric::Percent(125.),
            color: DesignTextDecorationColor::Solid(DesignColor::BLUE),
            skip_ink: false,
        }),
        open_type_features: vec![
            DesignOpenTypeFeature::new(registered.clone(), "Standard ligatures", true, false)
                .with_preview("fi fl"),
            DesignOpenTypeFeature::new(opaque.clone(), "Future round dots", false, true)
                .unavailable("Not supported by this font style"),
        ],
        ..DesignTypography::default()
    };

    assert_eq!(typography.paragraph_indent, 24.);
    assert_eq!(typography.list_spacing, 8.);
    assert!(typography.hanging_punctuation);
    assert!(typography.hanging_lists);
    assert_eq!(
        typography.decoration_details,
        Some(DesignTextDecorationDetails {
            style: DesignTextDecorationStyle::Wavy,
            offset: DesignTextDecorationMetric::Pixels(-1.),
            thickness: DesignTextDecorationMetric::Percent(125.),
            color: DesignTextDecorationColor::Solid(DesignColor::BLUE),
            skip_ink: false,
        })
    );
    assert_eq!(typography.open_type_features[0].tag, registered);
    assert_eq!(
        typography.open_type_features[0]
            .preview
            .as_ref()
            .map(AsRef::<str>::as_ref),
        Some("fi fl")
    );
    assert_eq!(typography.open_type_features[1].tag, opaque);
    assert!(!typography.open_type_features[1].availability.is_available());

    for property in [
        DesignPanelProperty::ParagraphIndent,
        DesignPanelProperty::ListSpacing,
        DesignPanelProperty::TextHangingPunctuation,
        DesignPanelProperty::TextHangingLists,
        DesignPanelProperty::TextDecorationStyle,
        DesignPanelProperty::TextDecorationOffset,
        DesignPanelProperty::TextDecorationThickness,
        DesignPanelProperty::TextDecorationColor,
        DesignPanelProperty::TextDecorationSkipInk,
    ] {
        assert!(property.is_typography());
        assert!(property.supports_selected_text_range());
    }
}

#[test]
fn font_catalog_preserves_source_identity_search_and_availability() {
    let local = DesignFontFamily::local(
        "inter",
        "Inter",
        [DesignFontStyle::imported("regular", "Regular")],
    );
    let library = DesignFontFamily {
        id: "inter".into(),
        name: "Inter Library".into(),
        source: DesignFontSource::Library {
            library_id: "type-library".into(),
            library_name: "Type Library".into(),
        },
        styles: vec![DesignFontStyle {
            id: "regular".into(),
            name: "Regular".into(),
            weight: Some(400),
            italic: false,
            availability: DesignFontAvailability::Available,
            preview: Some("Aa 0123".into()),
        }],
    };
    let catalog = DesignFontViewData::ready([local, library]);
    let local_selection = DesignFontSelection::local("inter", "regular");
    let library_selection =
        DesignFontSelection::library("type-library", "Type Library", "inter", "regular");

    assert!(
        catalog
            .font(&local_selection)
            .is_some_and(|(_, style)| style.availability.can_apply())
    );
    assert!(
        catalog
            .font(&library_selection)
            .is_some_and(|(_, style)| style.availability.can_import())
    );
    assert_eq!(catalog.matching("library").count(), 1);
    assert_eq!(catalog.matching("regular").count(), 2);
    assert!(DesignFontViewData::default().families.is_empty());
    assert_eq!(
        DesignFontViewData::default().state,
        DesignFontCatalogState::Loading
    );
}

#[test]
fn typography_style_data_preserves_exact_page_and_library_identity() {
    let page_style = DesignTypographyStyle::new("body", "Body", "Inter", "Regular", 16.);
    let library_style = DesignTypographyStyle::new("body", "Body", "Source Sans", "Regular", 18.);
    let view_data = DesignTypographyStyleViewData::new(
        [page_style.clone()],
        [DesignTypographyStyleLibrary::new(
            "acme",
            "Acme",
            [library_style.clone()],
        )],
    );
    let page_selection = DesignTypographyStyleSelection::page("body");
    let library_selection = DesignTypographyStyleSelection::library("acme", "body");

    assert_eq!(view_data.style(&page_selection), Some(&page_style));
    assert_eq!(view_data.style(&library_selection), Some(&library_style));
    assert_eq!(
        view_data.style(&DesignTypographyStyleSelection::library("other", "body")),
        None
    );

    let binding =
        DesignTypographyStyleBinding::new(library_selection.clone(), "Body").detachable(false);
    let typography = DesignTypography {
        style_binding: Some(binding.clone()),
        ..DesignTypography::default()
    };
    assert_eq!(typography.style_binding, Some(binding));
    assert!(
        !typography
            .style_binding
            .as_ref()
            .expect("style binding")
            .can_detach
    );
    assert!(DesignPanelProperty::TypographyStyle.is_typography());
    assert!(DesignPanelProperty::TypographyStyle.supports_selected_text_range());
    assert!(matches!(
        DesignPanelValue::TypographyStyle(Some(library_selection)),
        DesignPanelValue::TypographyStyle(Some(_))
    ));
}

#[test]
fn typography_actions_carry_the_character_range_target() {
    let action = DesignPanelAction::TypographyPropertyChangeRequested {
        node_id: "text".into(),
        target: DesignTypographyTarget::SelectedTextRange,
        property: DesignPanelProperty::FontSize,
        value: DesignPanelValue::Number(18.),
    };

    assert!(matches!(
        action,
        DesignPanelAction::TypographyPropertyChangeRequested {
            target: DesignTypographyTarget::SelectedTextRange,
            property: DesignPanelProperty::FontSize,
            ..
        }
    ));
}

#[test]
fn variable_font_axes_validate_lossless_metadata_and_binding_provenance() {
    let editable = DesignFontAxis::new("wdth", "Width", 96.5, 75., 125., 100.).with_step(0.1);
    assert!(editable.has_valid_tag());
    assert!(editable.has_valid_range());
    assert!(editable.is_editable());
    assert_eq!(editable.default, 100.);
    assert_eq!(editable.step, 0.1);

    let read_only = editable
        .clone()
        .read_only_with_reason("The selected range mixes font faces");
    assert!(!read_only.is_editable());
    assert_eq!(
        read_only.disabled_reason().map(|reason| reason.to_string()),
        Some(String::from("The selected range mixes font faces"))
    );

    let bound = editable.with_binding(
        DesignFontAxisBinding::new("width-variable", "Body width").with_collection("Typography"),
    );
    assert!(!bound.is_editable());
    assert_eq!(
        bound
            .binding
            .as_ref()
            .and_then(|binding| binding.collection_name.as_ref())
            .map(|name| name.to_string()),
        Some(String::from("Typography"))
    );
    assert_eq!(
        bound.disabled_reason().map(|reason| reason.to_string()),
        Some(String::from("Bound to variable Body width"))
    );

    assert!(!DesignFontAxis::new("bad", "Bad tag", 0., -1., 1., 0.).has_valid_tag());
    assert!(!DesignFontAxis::new("wght", "Bad range", 500., 900., 100., 400.).has_valid_range());
}

#[test]
fn variable_font_axis_intent_keeps_tag_phase_and_exact_range_target() {
    let action = DesignPanelAction::TypographyVariableAxisEditRequested {
        node_id: "text".into(),
        target: DesignTypographyTarget::SelectedTextRangeRevision(19),
        tag: "GRAD".into(),
        value: 25.,
        phase: DesignPanelEditPhase::Preview,
    };
    assert!(matches!(
        action,
        DesignPanelAction::TypographyVariableAxisEditRequested {
            target: DesignTypographyTarget::SelectedTextRangeRevision(19),
            tag,
            value: 25.,
            phase: DesignPanelEditPhase::Preview,
            ..
        } if tag.as_ref() == "GRAD"
    ));
}

#[test]
fn export_sizing_parses_scale_width_and_height_units() {
    assert_eq!(
        DesignExportSizing::parse("2x"),
        Ok(DesignExportSizing::Scale(2.))
    );
    assert_eq!(
        DesignExportSizing::parse("1440W"),
        Ok(DesignExportSizing::Width(1440.))
    );
    assert_eq!(
        DesignExportSizing::parse("800h"),
        Ok(DesignExportSizing::Height(800.))
    );
    assert_eq!(DesignExportSizing::Width(320.5).to_string(), "320.5w");
    assert!(DesignExportSizing::parse("0x").is_err());
    assert!(DesignExportSizing::parse("2").is_err());
}

#[test]
fn static_export_formats_are_exactly_png_jpg_svg_and_pdf() {
    assert_eq!(
        DesignExportFormat::ALL.map(DesignExportFormat::label),
        ["PNG", "JPG", "SVG", "PDF"]
    );
    assert!(DesignExportFormat::Png.supports_custom_sizing());
    assert!(DesignExportFormat::Jpg.supports_custom_sizing());
    assert!(!DesignExportFormat::Svg.supports_custom_sizing());
    assert!(!DesignExportFormat::Pdf.supports_custom_sizing());
}

#[test]
fn vector_and_pdf_configurations_are_normalized_to_intrinsic_size() {
    let mut configuration = DesignExportConfiguration::new("stable-svg", DesignExportFormat::Png);
    configuration.sizing = DesignExportSizing::Width(640.);
    configuration.apply_change(DesignExportConfigurationChange::Format(
        DesignExportFormat::Svg,
    ));
    assert_eq!(configuration.id.as_ref(), "stable-svg");
    assert_eq!(configuration.sizing, DesignExportSizing::Scale(1.));
    assert!(matches!(
        configuration.format_settings,
        DesignExportFormatSettings::Svg(_)
    ));

    let legacy = DesignExportSetting {
        scale: 3.,
        suffix: "-legacy".into(),
        format: DesignExportFormat::Pdf,
    };
    let adapted = DesignExportConfiguration::from_legacy("legacy-pdf", &legacy);
    assert_eq!(adapted.sizing, DesignExportSizing::Scale(1.));
    assert_eq!(adapted.common.suffix.as_ref(), "-legacy");
}

#[test]
fn format_specific_export_defaults_and_changes_remain_discriminated() {
    let mut png = DesignExportConfiguration::new("png", DesignExportFormat::Png);
    let DesignExportFormatSettings::Png(defaults) = png.format_settings else {
        panic!("PNG defaults");
    };
    assert!(defaults.ignore_overlapping_layers);
    assert_eq!(defaults.resampling, DesignExportImageResampling::Detailed);

    png.apply_change(DesignExportConfigurationChange::IncludeTextBoundingBox(
        true,
    ));
    let DesignExportFormatSettings::Png(settings) = png.format_settings else {
        panic!("PNG settings");
    };
    assert!(settings.include_text_bounding_box);

    png.apply_change(DesignExportConfigurationChange::Format(
        DesignExportFormat::Svg,
    ));
    png.apply_change(DesignExportConfigurationChange::IncludeIdAttribute(true));
    let DesignExportFormatSettings::Svg(settings) = png.format_settings else {
        panic!("SVG settings");
    };
    assert!(settings.ignore_overlapping_layers);
    assert!(settings.include_id_attribute);
    assert!(settings.outline_text);
    assert!(settings.simplify_stroke);
}

#[test]
fn pdf_defaults_to_medium_quality() {
    let configuration = DesignExportConfiguration::new("pdf", DesignExportFormat::Pdf);
    let DesignExportFormatSettings::Pdf(settings) = configuration.format_settings else {
        panic!("PDF settings");
    };
    assert_eq!(settings.quality, DesignExportImageQuality::Medium);
}

#[test]
fn static_export_defaults_are_derived_from_host_target_capabilities() {
    let capabilities = DesignStaticExportCapabilities {
        can_include_text_bounding_box: true,
        can_include_svg_bounds: true,
        svg_outline_text_default: false,
        svg_simplify_stroke_default: false,
    };
    let configuration = DesignExportConfiguration::new_for_capabilities(
        "svg-text",
        DesignExportFormat::Svg,
        capabilities,
    );
    let DesignExportFormatSettings::Svg(settings) = configuration.format_settings else {
        panic!("SVG settings");
    };
    assert!(!settings.outline_text);
    assert!(!settings.simplify_stroke);

    let mut shape = DesignExportConfiguration::new("shape-png", DesignExportFormat::Png);
    shape.apply_change(DesignExportConfigurationChange::IncludeTextBoundingBox(
        true,
    ));
    shape.apply_target_defaults(Default::default());
    let DesignExportFormatSettings::Png(settings) = shape.format_settings else {
        panic!("PNG settings");
    };
    assert!(!settings.include_text_bounding_box);
}

#[test]
fn animated_export_formats_have_exact_fps_defaults_and_limits() {
    assert_eq!(
        DesignAnimatedExportFormat::ALL.map(DesignAnimatedExportFormat::label),
        ["MP4", "WebM", "GIF", "SVG"]
    );
    assert_eq!(
        DesignVideoExportFps::ALL.map(DesignVideoExportFps::value),
        [12, 24, 30, 60]
    );
    assert_eq!(
        DesignGifExportFps::ALL.map(DesignGifExportFps::value),
        [8, 12, 15, 24, 30]
    );
    assert!(matches!(
        DesignAnimatedExportSettings::for_format(DesignAnimatedExportFormat::Mp4),
        DesignAnimatedExportSettings::Mp4 {
            sizing: DesignExportSizing::Scale(1.),
            fps: DesignVideoExportFps::Fps30,
            quality: DesignExportImageQuality::High,
        }
    ));
    assert!(matches!(
        DesignAnimatedExportSettings::for_format(DesignAnimatedExportFormat::Gif),
        DesignAnimatedExportSettings::Gif {
            sizing: DesignExportSizing::Scale(1.),
            fps: DesignGifExportFps::Fps15,
            loop_count: 0,
        }
    ));
}

#[test]
fn animated_export_rejects_cross_format_changes_and_normalizes_values() {
    let mut settings = DesignAnimatedExportSettings::for_format(DesignAnimatedExportFormat::Gif);
    assert!(!settings.apply_change(DesignAnimatedExportChange::VideoFps(
        DesignVideoExportFps::Fps60
    )));
    assert!(settings.apply_change(DesignAnimatedExportChange::GifLoopCount(1200)));
    assert!(matches!(
        settings,
        DesignAnimatedExportSettings::Gif {
            loop_count: 1000,
            ..
        }
    ));
    assert!(settings.apply_change(DesignAnimatedExportChange::Sizing(
        DesignExportSizing::Scale(1.25)
    )));
    assert!(matches!(
        settings,
        DesignAnimatedExportSettings::Gif {
            sizing: DesignExportSizing::Scale(1.),
            ..
        }
    ));

    assert!(settings.apply_change(DesignAnimatedExportChange::Format(
        DesignAnimatedExportFormat::WebM
    )));
    assert!(!settings.apply_change(DesignAnimatedExportChange::GifFps(DesignGifExportFps::Fps8)));
}

#[test]
fn animated_export_capability_explains_top_level_and_plan_gates() {
    let disabled =
        DesignAnimatedExportCapability::disabled("Only top-level animated frames export");
    let settings = DesignAnimatedExportSettings::default();
    assert!(!disabled.allows(&settings));
    assert_eq!(
        disabled.reason_for(&settings),
        Some("Only top-level animated frames export")
    );

    let mut starter = DesignAnimatedExportCapability::eligible(1920, 1080);
    starter.high_resolution_allowed = false;
    starter.high_resolution_reason = Some("Upgrade for high-resolution export".into());
    let high_fps = DesignAnimatedExportSettings::Mp4 {
        sizing: DesignExportSizing::Scale(1.),
        fps: DesignVideoExportFps::Fps60,
        quality: DesignExportImageQuality::High,
    };
    assert!(!starter.allows(&high_fps));
    assert_eq!(
        starter.reason_for(&high_fps),
        Some("Upgrade for high-resolution export")
    );
    let standard = DesignAnimatedExportSettings::Mp4 {
        sizing: DesignExportSizing::Scale(1.),
        fps: DesignVideoExportFps::Fps30,
        quality: DesignExportImageQuality::High,
    };
    assert!(starter.allows(&standard));
}

#[test]
fn canonical_node_taxonomy_excludes_tools_paints_and_context_aliases() {
    assert_eq!(DesignPanelNodeKind::ALL.len(), 20);
    assert_eq!(DesignPanelNodeKind::COMPATIBILITY_ALL.len(), 28);
    for compatibility_only in [
        DesignPanelNodeKind::Image,
        DesignPanelNodeKind::Video,
        DesignPanelNodeKind::Arrow,
        DesignPanelNodeKind::Mask,
        DesignPanelNodeKind::Table,
        DesignPanelNodeKind::Pen,
        DesignPanelNodeKind::Pencil,
        DesignPanelNodeKind::MultipleSelection,
    ] {
        assert!(compatibility_only.is_compatibility_alias());
        assert!(!DesignPanelNodeKind::ALL.contains(&compatibility_only));
        assert!(DesignPanelNodeKind::COMPATIBILITY_ALL.contains(&compatibility_only));
    }
    assert_eq!(
        DesignPanelNodeKind::Arrow.canonical_kind(),
        DesignPanelNodeKind::Line
    );
    assert_eq!(
        DesignPanelNodeKind::Pen.canonical_kind(),
        DesignPanelNodeKind::Vector
    );
}

#[test]
fn exact_blend_modes_include_plus_and_light_variants() {
    assert_eq!(DesignBlendMode::ALL.len(), 19);
    assert_eq!(DesignBlendMode::LinearBurn.label(), "Plus darker");
    assert_eq!(DesignBlendMode::LinearDodge.label(), "Plus lighter");
    assert!(DesignBlendMode::ALL.contains(&DesignBlendMode::SoftLight));
    assert!(DesignBlendMode::ALL.contains(&DesignBlendMode::HardLight));
    assert_eq!(DesignBlendMode::NON_PASS_THROUGH.len(), 18);
    assert!(!DesignBlendMode::NON_PASS_THROUGH.contains(&DesignBlendMode::PassThrough));
    assert!(DesignPanelNodeKind::Frame.supports_pass_through_blend());
    assert!(DesignPanelNodeKind::Group.supports_pass_through_blend());
    assert!(DesignPanelNodeKind::Table.supports_pass_through_blend());
    assert!(!DesignPanelNodeKind::Rectangle.supports_pass_through_blend());
    assert!(!DesignPanelNodeKind::Text.supports_pass_through_blend());
}

#[test]
fn selection_colors_retain_counts_and_stable_occurrence_targets() {
    let first = DesignSelectionPaintReference::paint(
        "rectangle-a",
        DesignSelectionPaintCollection::Fill,
        "fill-a",
        0,
    );
    let second = DesignSelectionPaintReference::paint(
        "vector-b",
        DesignSelectionPaintCollection::Stroke,
        "stroke-b",
        1,
    )
    .gradient_stop("stop-blue", 2);
    let aggregate = DesignSelectionColors::new([DesignSelectionColor::new(
        "selection-blue",
        DesignColor::BLUE,
        [first, second.clone()],
    )]);

    assert_eq!(aggregate.total_occurrences(), 2);
    assert_eq!(aggregate.colors[0].occurrence_count, 2);
    assert_eq!(
        aggregate.colors[0].paint_references[1], second,
        "stable gradient-stop identity must survive aggregation"
    );
}

#[test]
fn selection_color_rows_keep_paint_style_and_leaf_variable_identities_separate() {
    let reference = DesignSelectionPaintReference::paint(
        "rectangle-a",
        DesignSelectionPaintCollection::Fill,
        "fill-a",
        0,
    );
    let mut translucent = DesignPaint::solid(DesignColor::BLUE).with_id("fill-a");
    translucent.opacity = 42.;
    let style_selection = DesignPaintStyleSelection::library("library", "surface");
    let style_binding =
        DesignPaintStyleBinding::new(style_selection.clone(), "Surface").detachable(false);
    let variable_binding =
        DesignPaintBinding::new("variable-blue", "Blue").with_collection("Primitives");
    let row = DesignSelectionColor::new("selection-blue", DesignColor::BLUE, [reference.clone()])
        .with_paint(translucent.clone())
        .with_style_context([translucent.clone()], Some(style_binding.clone()))
        .with_binding(variable_binding.clone());
    let aggregate = DesignSelectionColors::new([row.clone()]);

    assert_eq!(
        aggregate.color("selection-blue").map(|row| &row.paint),
        Some(&translucent),
        "equal RGBA must not erase non-color Paint semantics"
    );
    assert_eq!(row.style_paints, vec![translucent]);
    assert_eq!(row.style_binding, Some(style_binding));
    assert_eq!(row.binding, Some(variable_binding));

    let target = DesignPanelTarget::Nodes {
        node_ids: vec!["rectangle-a".into(), "vector-b".into()],
    };
    let style_action = DesignPanelAction::SelectionColorPaintStyleApplyRequested {
        target: target.clone(),
        selection_color_id: row.id.clone(),
        paint_references: row.paint_references.clone(),
        style: style_selection,
    };
    let variable_action = DesignPanelAction::SelectionColorVariableDetachRequested {
        target: target.clone(),
        selection_color_id: row.id,
        paint_references: row.paint_references,
        variable_id: "variable-blue".into(),
    };
    assert!(matches!(
        style_action,
        DesignPanelAction::SelectionColorPaintStyleApplyRequested {
            target: DesignPanelTarget::Nodes { node_ids },
            paint_references,
            ..
        } if node_ids == vec![SharedString::from("rectangle-a"), SharedString::from("vector-b")]
            && paint_references == vec![reference.clone()]
    ));
    assert!(matches!(
        variable_action,
        DesignPanelAction::SelectionColorVariableDetachRequested {
            target: DesignPanelTarget::Nodes { node_ids },
            paint_references,
            variable_id,
            ..
        } if node_ids == vec![SharedString::from("rectangle-a"), SharedString::from("vector-b")]
            && paint_references == vec![reference]
            && variable_id.as_ref() == "variable-blue"
    ));
}

#[test]
fn mask_is_orthogonal_and_legacy_string_is_only_a_fallback() {
    let mut rectangle =
        DesignPanelNode::new("rectangle", "Mask shape", DesignPanelNodeKind::Rectangle);
    assert!(!rectangle.effective_is_mask());

    rectangle.is_mask = true;
    rectangle.mask_mode = DesignMaskType::Luminance;
    assert_eq!(
        rectangle.effective_mask_type(),
        Some(DesignMaskType::Luminance)
    );

    rectangle.is_mask = false;
    rectangle.mask_type = Some("Vector".into());
    assert_eq!(
        rectangle.effective_mask_type(),
        Some(DesignMaskType::Vector)
    );
}

#[test]
fn legacy_media_snapshot_has_a_lossless_paint_filter_migration() {
    let media = DesignMedia {
        crop_mode: "Crop".into(),
        exposure: 12.,
        contrast: -8.,
        saturation: 20.,
        temperature: 4.,
        tint: -3.,
        highlights: 18.,
        shadows: -14.,
    };

    assert_eq!(media.paint_scale_mode(), DesignMediaPaintScaleMode::Crop);
    assert_eq!(
        media.image_filters(),
        DesignImageFilters {
            exposure: 12.,
            contrast: -8.,
            saturation: 20.,
            temperature: 4.,
            tint: -3.,
            highlights: 18.,
            shadows: -14.,
        }
    );
}

#[test]
fn selected_node_header_presets_preserve_the_observed_control_order() {
    use DesignSelectionHeaderControlKind as Kind;

    let ellipse = DesignSelectionHeaderViewData::for_node_kind(DesignPanelNodeKind::Ellipse);
    assert_eq!(
        ellipse
            .primary_controls
            .iter()
            .map(|control| control.kind)
            .collect::<Vec<_>>(),
        vec![
            Kind::CreateComponent,
            Kind::UseAsMask,
            Kind::BooleanFlattenMenu,
            Kind::EditObject,
        ]
    );

    let text = DesignSelectionHeaderViewData::for_node_kind(DesignPanelNodeKind::Text);
    assert_eq!(
        text.primary_controls
            .iter()
            .map(|control| control.kind)
            .collect::<Vec<_>>(),
        vec![
            Kind::SelectMatchingLayers,
            Kind::CreateLink,
            Kind::ApplyTextContentVariable,
            Kind::CreateComponent,
        ]
    );
    assert!(!text.overflow_controls.is_empty());

    let frame = DesignSelectionHeaderViewData::for_node_kind(DesignPanelNodeKind::Frame);
    assert!(frame.title_menu.is_some());
    assert_eq!(
        frame
            .primary_controls
            .iter()
            .map(|control| control.kind)
            .collect::<Vec<_>>(),
        vec![
            Kind::SelectMatchingLayers,
            Kind::CreateComponent,
            Kind::UseAsMask,
            Kind::BooleanFlattenMenu,
        ]
    );
}

#[test]
fn multiple_selection_header_preset_is_kind_neutral() {
    use DesignSelectionHeaderControlKind as Kind;

    let header = DesignSelectionHeaderViewData::for_multiple_selection(3);
    assert_eq!(header.title, "3 layers");
    assert!(header.title_menu.is_none());
    assert_eq!(
        header
            .primary_controls
            .iter()
            .map(|control| control.kind)
            .collect::<Vec<_>>(),
        vec![Kind::CreateComponent],
    );
    assert!(header.overflow_controls.is_empty());
    assert!(
        header.primary_controls.iter().all(|control| !matches!(
            control.kind,
            Kind::CreateLink | Kind::ApplyTextContentVariable
        )),
        "the aggregate preset must not borrow Text-only commands",
    );
}

#[test]
fn boolean_header_menu_contains_typed_leaf_commands_and_flatten() {
    let control = DesignSelectionHeaderControl::boolean_flatten_menu();
    assert!(control.is_menu());
    assert_eq!(control.menu_items.len(), 5);
    assert_eq!(
        control.menu_items[0].command,
        DesignSelectionHeaderCommand::Boolean(DesignBooleanOperation::Union)
    );
    assert_eq!(
        control.menu_items[4].command,
        DesignSelectionHeaderCommand::Flatten
    );
    assert!(DesignSelectionHeaderCommand::CreateComponent.requires_edit());
    assert!(!DesignSelectionHeaderCommand::SelectMatchingLayers.requires_edit());
    assert_eq!(
        DesignSelectionHeaderCommand::SelectMatchingLayers.default_access(),
        DesignSelectionHeaderCommandAccess::ViewerSafe
    );
}

#[test]
fn host_defined_header_controls_preserve_presentation_menu_and_access() {
    let control = DesignSelectionHeaderControl::new(
        "plugin-action",
        DesignSelectionHeaderControlKind::HostDefined,
    )
    .with_icon(DesignSelectionHeaderControlIcon::Glyph("AC".into()))
    .with_tooltip("Run Acme action")
    .viewer_safe();

    assert_eq!(
        control.icon,
        DesignSelectionHeaderControlIcon::Glyph("AC".into())
    );
    assert_eq!(
        control.access,
        DesignSelectionHeaderCommandAccess::ViewerSafe
    );
    assert_eq!(
        control.command(),
        Some(DesignSelectionHeaderCommand::HostDefined {
            command_id: "plugin-action".into(),
        })
    );

    let menu = DesignSelectionHeaderControl::new(
        "plugin-menu",
        DesignSelectionHeaderControlKind::HostDefined,
    )
    .with_menu_items([
        DesignSelectionHeaderMenuItem::new(
            "inspect",
            "Inspect",
            DesignSelectionHeaderCommand::HostDefined {
                command_id: "inspect".into(),
            },
        )
        .viewer_safe(),
        DesignSelectionHeaderMenuItem::new(
            "mutate",
            "Mutate",
            DesignSelectionHeaderCommand::HostDefined {
                command_id: "mutate".into(),
            },
        ),
    ]);
    assert!(menu.is_menu());
    assert_eq!(menu.command(), None);
    assert_eq!(
        menu.menu_items
            .iter()
            .map(DesignSelectionHeaderMenuItem::effective_access)
            .collect::<Vec<_>>(),
        vec![
            DesignSelectionHeaderCommandAccess::ViewerSafe,
            DesignSelectionHeaderCommandAccess::EditRequired,
        ]
    );
    assert_eq!(
        DesignSelectionHeaderMenuItem::new(
            "inspect-type",
            "Inspect type",
            DesignSelectionHeaderCommand::TitleMenuItem {
                item_id: "inspect-type".into(),
            },
        )
        .viewer_safe()
        .effective_access(),
        DesignSelectionHeaderCommandAccess::ViewerSafe,
    );
    assert_eq!(
        DesignSelectionHeaderControl::direct(DesignSelectionHeaderControlKind::CreateComponent,)
            .viewer_safe()
            .effective_access(),
        DesignSelectionHeaderCommandAccess::EditRequired,
    );
}

#[test]
fn typography_target_distinguishes_exact_selected_range_revisions() {
    let legacy = DesignTypographyTarget::SelectedTextRange;
    let first = DesignTypographyTarget::SelectedTextRangeRevision(7);
    let second = DesignTypographyTarget::SelectedTextRangeRevision(8);

    assert!(legacy.is_selected_text_range());
    assert!(first.is_selected_text_range());
    assert!(!DesignTypographyTarget::WholeLayer.is_selected_text_range());
    assert_eq!(legacy.selected_text_range_revision(), None);
    assert_eq!(first.selected_text_range_revision(), Some(7));
    assert_ne!(first, second);
}

#[test]
fn paint_target_distinguishes_ranges_and_is_exposed_by_paint_actions() {
    let legacy = DesignPaintTarget::SelectedTextRange;
    let first = DesignPaintTarget::SelectedTextRangeRevision(7);
    let second = DesignPaintTarget::SelectedTextRangeRevision(8);

    assert!(legacy.is_selected_text_range());
    assert!(first.is_selected_text_range());
    assert!(!DesignPaintTarget::WholeLayer.is_selected_text_range());
    assert_eq!(legacy.selected_text_range_revision(), None);
    assert_eq!(first.selected_text_range_revision(), Some(7));
    assert_ne!(first, second);

    let source = DesignPanelAction::PaintSourceReplaceRequested {
        node_id: "text".into(),
        collection: DesignPanelCollection::Fill,
        target: first,
        paint_id: "fill-image".into(),
        index: 0,
    };
    assert_eq!(
        source.paint_target(),
        Some((DesignPanelCollection::Fill, first))
    );
    let media_source = DesignPanelAction::PaintMediaSourceActionRequested {
        node_id: "text".into(),
        collection: DesignPanelCollection::Fill,
        target: first,
        paint_id: "fill-image".into(),
        index: 0,
        source_id: "image-source".into(),
        action: DesignMediaSourceAction::EditImage,
    };
    assert_eq!(
        media_source.paint_target(),
        Some((DesignPanelCollection::Fill, first))
    );
    let media_drop = DesignPanelAction::PaintMediaSourceDropRequested {
        node_id: "text".into(),
        collection: DesignPanelCollection::Fill,
        target: first,
        paint_id: "fill-image".into(),
        index: 0,
        expected_source_id: "image-source".into(),
        expected_media_kind: DesignMediaKind::Image,
        file: DesignMediaDroppedFile::new(
            std::path::PathBuf::from("replacement.png"),
            DesignMediaFileKind::Png,
        ),
    };
    assert_eq!(
        media_drop.paint_target(),
        Some((DesignPanelCollection::Fill, first))
    );
    assert_eq!(
        DesignPanelAction::PropertyChangeRequested {
            node_id: "text".into(),
            property: DesignPanelProperty::Opacity,
            value: DesignPanelValue::Number(50.),
        }
        .paint_target(),
        None
    );
}

#[test]
fn generic_variable_targets_preserve_exact_figma_fields() {
    use DesignVariableBindableField::{Node, Text};
    use DesignVariableBindableNodeField as NodeField;
    use DesignVariableBindableTextField as TextField;

    let mut frame = DesignPanelNode::new("frame", "Frame", DesignPanelNodeKind::Frame);
    let corner = frame
        .property_variable_target(DesignPanelProperty::CornerRadius)
        .expect("frames expose uniform corners through four retained leaves");
    assert_eq!(
        corner.fields,
        vec![
            Node(NodeField::TopLeftRadius),
            Node(NodeField::TopRightRadius),
            Node(NodeField::BottomRightRadius),
            Node(NodeField::BottomLeftRadius),
        ]
    );
    frame.layout.as_mut().expect("frame layout").mode = DesignLayoutMode::Horizontal;
    assert_eq!(
        frame
            .property_variable_target(DesignPanelProperty::PaddingVertical)
            .expect("auto-layout padding")
            .fields,
        vec![Node(NodeField::PaddingTop), Node(NodeField::PaddingBottom)]
    );

    assert_eq!(
        frame
            .property_variable_target(DesignPanelProperty::Gap)
            .expect("flow gap")
            .fields,
        vec![Node(NodeField::ItemSpacing)]
    );
    frame.layout.as_mut().expect("frame layout").mode = DesignLayoutMode::Grid;
    assert_eq!(
        frame
            .property_variable_target(DesignPanelProperty::Gap)
            .expect("grid column gap")
            .fields,
        vec![Node(NodeField::GridColumnGap)]
    );
    assert_eq!(
        frame
            .property_variable_target(DesignPanelProperty::CounterAxisGap)
            .expect("grid row gap")
            .fields,
        vec![Node(NodeField::GridRowGap)]
    );

    let ellipse = DesignPanelNode::new("ellipse", "Ellipse", DesignPanelNodeKind::Ellipse);
    assert_eq!(
        ellipse
            .property_variable_target(DesignPanelProperty::CornerRadius)
            .expect("ellipse uniform corner")
            .fields,
        vec![Node(NodeField::CornerRadius)]
    );

    let text_node = DesignPanelNode::new("text", "Text", DesignPanelNodeKind::Text);
    let font_size = text_node
        .property_variable_target(DesignPanelProperty::FontSize)
        .expect("font size variable target");
    assert_eq!(font_size.fields, vec![Text(TextField::FontSize)]);
    assert_eq!(
        font_size.typography_target,
        Some(DesignTypographyTarget::WholeLayer)
    );
    let font_weight = text_node
        .property_variable_target(DesignPanelProperty::FontWeight)
        .expect("font weight variable target");
    assert_eq!(font_weight.fields, vec![Text(TextField::FontWeight)]);
    assert_eq!(font_weight.resolved_type, DesignVariableResolvedType::Float);
    assert_eq!(
        font_weight.compatible_scope,
        Some(DesignVariableScope::FontWeight)
    );
    assert_eq!(TextField::FontWeight.api_name(), "fontWeight");
    assert_eq!(
        NodeField::CounterAxisSpacing.api_name(),
        "counterAxisSpacing"
    );
    assert_eq!(TextField::ParagraphIndent.api_name(), "paragraphIndent");
}

#[test]
fn generic_variable_catalog_filters_type_scope_and_search_metadata() {
    let target = DesignPanelNode::new("frame", "Frame", DesignPanelNodeKind::Frame)
        .property_variable_target(DesignPanelProperty::Width)
        .expect("width target");
    let width = DesignVariable::page(
        "size-card",
        "Card",
        "sizes",
        "Layout sizes",
        DesignVariableResolvedType::Float,
    )
    .with_source(DesignVariableSource::library(
        "design-system",
        "Design system",
    ))
    .with_scopes([DesignVariableScope::WidthHeight])
    .with_import_state(DesignVariableImportState::Available)
    .with_resolved_value(DesignVariableResolvedValue::Float(320.))
    .with_search(
        DesignVariableSearchMetadata::new(["foundations", "size"], ["dimension"])
            .described("Default product card width"),
    );
    let gap = DesignVariable::page(
        "space-200",
        "200",
        "spacing",
        "Spacing",
        DesignVariableResolvedType::Float,
    )
    .with_scopes([DesignVariableScope::Gap]);
    let wrong_type = DesignVariable::page(
        "size-label",
        "Card",
        "strings",
        "Strings",
        DesignVariableResolvedType::String,
    )
    .with_scopes([DesignVariableScope::AllScopes]);
    let data = DesignVariableViewData::new([width.clone(), gap, wrong_type]);

    assert_eq!(
        data.compatible(&target, "product dimension")
            .map(|variable| variable.id.as_ref())
            .collect::<Vec<_>>(),
        vec!["size-card"]
    );
    assert!(width.source.is_remote());
    assert_eq!(width.import_state, DesignVariableImportState::Available);
    assert_eq!(
        width
            .resolved_value
            .as_ref()
            .map(|value| value.panel_value()),
        Some(DesignPanelValue::Number(320.))
    );
    assert_eq!(
        DesignVariableScope::Opaque("FUTURE_SCOPE".into()).api_name(),
        "FUTURE_SCOPE"
    );
}

#[test]
fn page_resources_resolve_by_stable_group_source_and_resource_ids() {
    let source = DesignLocalResourceSource::library("library-brand", "Brand");
    let group = DesignLocalResourceGroup::new(
        "brand-styles",
        "Brand styles",
        source.clone(),
        [DesignLocalResource::available(
            "accent",
            "Accent",
            DesignLocalResourceKind::PaintStyle,
        )],
    );
    let selection = group.selection("accent");
    let data = DesignLocalResourceViewData::new([group]);

    assert_eq!(selection.group_id.as_ref(), "brand-styles");
    assert_eq!(selection.source, source);
    assert_eq!(
        data.resource(&selection)
            .map(|resource| resource.name.as_ref()),
        Some("Accent")
    );
    assert!(
        data.resource(&DesignLocalResourceSelection {
            group_id: "another-group".into(),
            source: selection.source.clone(),
            resource_id: selection.resource_id.clone(),
        })
        .is_none()
    );
}

#[test]
fn page_local_styles_validate_nested_current_file_trees_and_canonical_order() {
    let view_data = DesignPageLocalStylesViewData::for_page(
        "page",
        [
            DesignLocalStyleSection::new(
                DesignLocalStyleKind::Effect,
                [DesignLocalStyleEntry::style(DesignLocalStyleItem::new(
                    "effect-soft",
                    "Soft",
                    DesignLocalStylePreview::Effect(vec![DesignEffect::new(
                        DesignEffectKind::DropShadow,
                    )]),
                ))],
            ),
            DesignLocalStyleSection::new(
                DesignLocalStyleKind::Text,
                [DesignLocalStyleEntry::folder(
                    "folder-type",
                    "Typography",
                    [DesignLocalStyleEntry::folder(
                        "folder-display",
                        "Display",
                        [DesignLocalStyleEntry::style(DesignLocalStyleItem::new(
                            "text-hero",
                            "Hero",
                            DesignLocalStylePreview::Text(DesignTypographyStyle::new(
                                "text-hero",
                                "Hero",
                                "Inter",
                                "Bold",
                                64.,
                            )),
                        ))],
                    )
                    .disabled("Managed folder")],
                )],
            ),
            DesignLocalStyleSection::new(
                DesignLocalStyleKind::LayoutGuide,
                [DesignLocalStyleEntry::style(DesignLocalStyleItem::new(
                    "layout-desktop",
                    "Desktop",
                    DesignLocalStylePreview::LayoutGuide(vec![DesignLayoutGrid::uniform(
                        8.,
                        DesignColor::BLUE,
                    )]),
                ))],
            ),
            DesignLocalStyleSection::new(
                DesignLocalStyleKind::Color,
                [DesignLocalStyleEntry::style(DesignLocalStyleItem::new(
                    "color-brand",
                    "Brand",
                    DesignLocalStylePreview::Color(vec![DesignPaint::solid(DesignColor::PURPLE)]),
                ))],
            ),
        ],
    );

    assert!(view_data.is_valid());
    assert_eq!(
        view_data
            .ordered_sections()
            .map(|section| section.kind)
            .collect::<Vec<_>>(),
        DesignLocalStyleKind::ALL
    );
    let target = DesignLocalStyleTarget::new(
        "page",
        DesignLocalStyleKind::Text,
        "text-hero",
        Some("folder-display".into()),
        0,
    );
    assert_eq!(
        view_data
            .resolve_style(&target)
            .map(|style| style.name.as_ref()),
        Some("Hero")
    );
    assert!(
        !view_data.entry_path_is_enabled(DesignLocalStyleKind::Text, "text-hero"),
        "a disabled ancestor folder disables its complete subtree"
    );
    assert_eq!(
        DesignVariablesEntryPoint::default(),
        DesignVariablesEntryPoint::NavigationBarOnly
    );
}

#[test]
fn page_local_styles_reject_duplicate_ids_preview_mismatches_and_non_page_targets() {
    let color = || {
        DesignLocalStyleEntry::style(DesignLocalStyleItem::new(
            "duplicate",
            "Brand",
            DesignLocalStylePreview::Color(vec![DesignPaint::solid(DesignColor::BLUE)]),
        ))
    };
    let duplicate_ids = DesignPageLocalStylesViewData::for_page(
        "page",
        [
            DesignLocalStyleSection::new(DesignLocalStyleKind::Color, [color()]),
            DesignLocalStyleSection::new(
                DesignLocalStyleKind::Text,
                [DesignLocalStyleEntry::folder("duplicate", "Typography", [])],
            ),
        ],
    );
    assert!(!duplicate_ids.is_valid());

    let wrong_preview = DesignPageLocalStylesViewData::for_page(
        "page",
        [DesignLocalStyleSection::new(
            DesignLocalStyleKind::Text,
            [color()],
        )],
    );
    assert!(!wrong_preview.is_valid());

    let node_target = DesignPageLocalStylesViewData::new(
        DesignPanelTarget::Nodes {
            node_ids: vec!["node".into()],
        },
        [],
    );
    assert!(!node_target.is_valid());
}

#[test]
fn variable_modes_keep_resolved_and_explicit_modes_distinct() {
    let inherited = DesignVariableModeCollection::new(
        "theme",
        "Theme",
        DesignLocalResourceSource::Local,
        [
            DesignVariableMode::new("light", "Light"),
            DesignVariableMode::new("dark", "Dark"),
            DesignVariableMode::new("contrast", "High contrast").disabled("Not available here"),
        ],
        "light",
        "dark",
    );
    assert_eq!(
        inherited.resolved_mode().map(|mode| mode.name.as_ref()),
        Some("Dark")
    );
    assert!(inherited.explicit_mode().is_none());
    assert_eq!(
        inherited
            .mode_disabled_reason("contrast")
            .as_ref()
            .map(|reason| reason.as_ref()),
        Some("Not available here")
    );

    let explicit = inherited.explicit("light");
    assert_eq!(
        explicit.explicit_mode().map(|mode| mode.name.as_ref()),
        Some("Light")
    );
    assert_eq!(explicit.resolved_mode_id.as_ref(), "light");
}

#[test]
fn page_background_is_one_controlled_solid_color_with_read_only_reason() {
    let page = DesignPageViewData::new(
        "page",
        DesignPageBackground::new(DesignColor::rgb(0xf5, 0xf5, 0xf5))
            .read_only("Managed by the host"),
        DesignLocalResourceViewData::default(),
    );
    assert_eq!(page.background.color.hex().as_ref(), "F5F5F5");
    assert!(page.background.read_only);
    assert_eq!(
        page.background
            .disabled_reason
            .as_ref()
            .map(|reason| reason.as_ref()),
        Some("Managed by the host")
    );
}

#[test]
fn vector_edit_view_data_preserves_stable_ids_and_resolves_mixed_values() {
    let view_data = DesignVectorEditViewData::new([
        DesignVectorVertexViewData::new("vertex-b", 48., 24.)
            .with_corner_radius(Some(12.))
            .with_handle_mirroring(Some(DesignHandleMirroring::Angle))
            .selected(true),
        DesignVectorVertexViewData::new("vertex-a", 16., 24.)
            .with_topology(DesignVectorVertexTopology::Endpoint)
            .with_corner_radius(Some(4.))
            .with_handle_mirroring(Some(DesignHandleMirroring::None))
            .selected(true),
        DesignVectorVertexViewData::new("vertex-branch", 96., 72.)
            .with_topology(DesignVectorVertexTopology::Branch)
            .with_corner_radius(None)
            .with_handle_mirroring(None),
    ]);

    assert!(view_data.is_valid());
    assert_eq!(
        view_data.selected_vertex_ids(),
        vec![
            SharedString::from("vertex-b"),
            SharedString::from("vertex-a")
        ]
    );
    assert_eq!(view_data.selected_x(), DesignVectorSelectionValue::Mixed);
    assert_eq!(
        view_data.selected_y(),
        DesignVectorSelectionValue::Uniform(24.)
    );
    assert_eq!(
        view_data.selected_corner_radius(),
        DesignVectorSelectionValue::Mixed
    );
    assert_eq!(
        view_data.selected_handle_mirroring(),
        DesignVectorSelectionValue::Mixed
    );
    assert!(view_data.can_edit_coordinates());
    assert!(view_data.can_edit_corner_radius());
    assert!(view_data.can_edit_handle_mirroring());
}

#[test]
fn vector_edit_validation_and_branch_read_only_gates_are_explicit() {
    assert_eq!(
        DesignHandleMirroring::ALL.map(DesignHandleMirroring::api_name),
        ["NONE", "ANGLE", "ANGLE_AND_LENGTH"]
    );

    let branch = DesignVectorEditViewData::new([DesignVectorVertexViewData::new("branch", 4., 8.)
        .with_topology(DesignVectorVertexTopology::Branch)
        .with_corner_radius(None)
        .with_handle_mirroring(None)
        .selected(true)]);
    assert!(branch.can_edit_coordinates());
    assert!(!branch.can_edit_corner_radius());
    assert!(!branch.can_edit_handle_mirroring());
    assert!(branch.selected_corner_radius().is_unset());
    assert!(branch.selected_handle_mirroring().is_unset());

    let duplicate = DesignVectorEditViewData::new([
        DesignVectorVertexViewData::new("same", 0., 0.),
        DesignVectorVertexViewData::new("same", 1., 1.),
    ]);
    assert!(!duplicate.is_valid());

    let invalid_radius =
        DesignVectorEditViewData::new([
            DesignVectorVertexViewData::new("invalid-radius", 0., 0.).with_corner_radius(Some(-1.))
        ]);
    assert!(!invalid_radius.is_valid());

    let invalid_coordinate = DesignVectorEditViewData::new([DesignVectorVertexViewData::new(
        "invalid-position",
        f32::NAN,
        0.,
    )]);
    assert!(!invalid_coordinate.is_valid());

    let read_only =
        DesignVectorEditViewData::new([
            DesignVectorVertexViewData::new("locked", 0., 0.).selected(true)
        ])
        .read_only(true);
    assert!(!read_only.can_select_vertices());
    assert!(!read_only.can_edit_coordinates());
}

#[test]
fn frame_preset_catalog_preserves_grouped_identity_dimensions_and_availability() {
    let phone = DesignFramePresetGroup::new(
        "phone",
        "Phone",
        [
            DesignFramePreset::new("compact", "Compact phone", 360., 800.),
            DesignFramePreset::new("pro", "Pro phone", 393., 852.)
                .disabled("Requires a mobile design library"),
        ],
    );
    let desktop = DesignFramePresetGroup::new(
        "desktop",
        "Desktop",
        [DesignFramePreset::new("compact", "Desktop", 1440., 1024.)],
    )
    .disabled("Desktop presets are managed by the host");
    let phone_compact = phone.selection("compact");
    let phone_pro = phone.selection("pro");
    let desktop_compact = desktop.selection("compact");
    let catalog = DesignFramePresetViewData::new("frame", [phone, desktop]);

    assert!(catalog.is_valid());
    assert!(catalog.can_apply(&phone_compact));
    assert!(!catalog.can_apply(&phone_pro));
    assert_eq!(
        catalog
            .effective_disabled_reason(&phone_pro)
            .map(|reason| reason.as_ref()),
        Some("Requires a mobile design library")
    );
    assert_eq!(
        catalog
            .preset(&desktop_compact)
            .map(|(_, preset)| (preset.width, preset.height)),
        Some((1440., 1024.))
    );
    assert!(!catalog.can_apply(&desktop_compact));
    assert_eq!(
        catalog
            .effective_disabled_reason(&desktop_compact)
            .map(|reason| reason.as_ref()),
        Some("Desktop presets are managed by the host")
    );
    assert_eq!(catalog.matching_dimensions(393., 852.), Some(phone_pro));

    let globally_disabled = catalog
        .clone()
        .disabled("Frame presets are unavailable in this file");
    assert!(!globally_disabled.can_apply(&phone_compact));
    assert_eq!(
        globally_disabled
            .effective_disabled_reason(&phone_compact)
            .map(|reason| reason.as_ref()),
        Some("Frame presets are unavailable in this file")
    );
}

#[test]
fn frame_preset_catalog_rejects_ambiguous_or_invalid_host_records() {
    let duplicate_presets = DesignFramePresetGroup::new(
        "phone",
        "Phone",
        [
            DesignFramePreset::new("same", "One", 320., 640.),
            DesignFramePreset::new("same", "Two", 360., 800.),
        ],
    );
    assert!(!duplicate_presets.is_valid());

    let duplicate_groups = DesignFramePresetViewData::new(
        "frame",
        [
            DesignFramePresetGroup::new(
                "phone",
                "Phone",
                [DesignFramePreset::new("one", "One", 320., 640.)],
            ),
            DesignFramePresetGroup::new(
                "phone",
                "Phone archive",
                [DesignFramePreset::new("two", "Two", 375., 667.)],
            ),
        ],
    );
    assert!(!duplicate_groups.is_valid());

    let invalid_dimensions = DesignFramePreset::new("bad", "Bad", f32::NAN, 800.);
    assert!(!invalid_dimensions.is_valid());
    let empty_reason =
        DesignFramePreset::new("bad", "Bad", 360., 800.).disabled(SharedString::default());
    assert!(!empty_reason.is_valid());
}

#[test]
fn targeted_node_action_preserves_order_and_retargets_legacy_leaf_payloads() {
    let target = DesignPanelTarget::Nodes {
        node_ids: vec!["third".into(), "first".into(), "second".into()],
    };
    let leaves = [
        DesignPanelAction::PropertyChangeRequested {
            node_id: "aggregate".into(),
            property: DesignPanelProperty::Width,
            value: DesignPanelValue::Number(320.),
        },
        DesignPanelAction::CollectionItemAddRequested {
            node_id: "aggregate".into(),
            collection: DesignPanelCollection::Fill,
            target: DesignPaintTarget::WholeLayer,
        },
        DesignPanelAction::EffectAddRequested {
            node_id: "aggregate".into(),
            kind: DesignEffectKind::DropShadow,
        },
        DesignPanelAction::ComponentPropertyResetRequested {
            node_id: "aggregate".into(),
            property_id: "label".into(),
        },
        DesignPanelAction::PaintStyleCreateRequested {
            node_id: "aggregate".into(),
            collection: DesignPanelCollection::Fill,
            target: DesignPaintTarget::WholeLayer,
            paints: vec![DesignPaint::solid(DesignColor::BLUE)],
        },
        DesignPanelAction::PaintColorVariableCreateRequested {
            node_id: "aggregate".into(),
            collection: DesignPanelCollection::Fill,
            target: DesignPaintTarget::WholeLayer,
            paint_id: "fill".into(),
            index: 0,
            color_target: DesignPaintColorTarget::Solid,
            color: DesignColor::BLUE,
        },
    ];

    for leaf in leaves {
        let wrapped = DesignPanelAction::for_selection_target(target.clone(), leaf);
        let (actual_target, nested) = wrapped
            .targeted_node_action()
            .expect("the action should expose its authoritative target");
        assert_eq!(actual_target, &target);

        let mut retargeted = nested
            .retargeted_legacy_node_action("second")
            .expect("ordinary leaf actions retain a compatibility node ID");
        assert_eq!(
            retargeted
                .legacy_node_id_mut()
                .map(|node_id| node_id.clone()),
            Some(SharedString::from("second"))
        );
    }
}

#[test]
fn property_copy_intent_keeps_exact_target_property_and_display_text() {
    let action = DesignPanelAction::PropertyCopyRequested {
        target: DesignPanelTarget::Nodes {
            node_ids: vec!["second".into(), "first".into()],
        },
        property: DesignPanelProperty::Width,
        displayed_value: "1,024  ◇".into(),
    };

    assert_eq!(action.compatibility_path(), None);
    assert!(action.targeted_node_action().is_none());
    assert!(action.retargeted_legacy_node_action("ignored").is_none());
    assert!(matches!(
        action,
        DesignPanelAction::PropertyCopyRequested {
            target: DesignPanelTarget::Nodes { node_ids },
            property: DesignPanelProperty::Width,
            displayed_value,
        } if node_ids == vec![SharedString::from("second"), SharedString::from("first")]
            && displayed_value.as_ref() == "1,024  ◇"
    ));
}

#[test]
fn viewer_properties_projection_retains_stable_sections_and_exact_copy_payloads() {
    let target = DesignPanelTarget::Nodes {
        node_ids: vec!["title".into()],
    };
    let projection = DesignViewerPropertiesViewData::new(
        target.clone(),
        [
            DesignViewerPropertySection::text_content("content", "Hello, world"),
            DesignViewerPropertySection::new(
                "typography",
                "Typography",
                [DesignViewerPropertyRow::new("size", "Size", "16px")
                    .with_property(DesignPanelProperty::FontSize)],
            )
            .with_copy_value("font-size: 16px;")
            .with_copy_all(),
            DesignViewerPropertySection::new("borders", "Borders", [])
                .with_summary("border: 1px solid #000;")
                .with_copy_value("border: 1px solid #000;")
                .with_color_representation(DesignViewerColorRepresentation::Css),
        ],
    );

    assert!(projection.is_valid());
    assert_eq!(
        projection
            .section("typography")
            .and_then(|section| section.row("size"))
            .and_then(|row| row.property),
        Some(DesignPanelProperty::FontSize)
    );
    assert_eq!(
        projection
            .section("content")
            .and_then(|section| section.copy_value.as_ref())
            .map(SharedString::as_ref),
        Some("Hello, world")
    );
    assert_eq!(
        DesignViewerColorRepresentation::ALL.map(DesignViewerColorRepresentation::label),
        ["CSS", "Hex", "RGB", "HSL", "HSB"]
    );

    let copy = DesignPanelAction::ViewerSectionCopyRequested {
        target: target.clone(),
        section_id: "typography".into(),
        copy_value: "font-size: 16px;".into(),
    };
    let representation = DesignPanelAction::ViewerSectionRepresentationChangeRequested {
        target,
        section_id: "borders".into(),
        representation: DesignViewerColorRepresentation::Hsb,
    };
    for action in [copy, representation] {
        assert_eq!(action.compatibility_path(), None);
        assert!(action.targeted_node_action().is_none());
        assert!(action.retargeted_legacy_node_action("ignored").is_none());
    }
}

#[test]
fn viewer_properties_projection_rejects_duplicate_section_and_row_ids() {
    let target = DesignPanelTarget::Nodes {
        node_ids: vec!["title".into()],
    };
    let duplicate_sections = DesignViewerPropertiesViewData::new(
        target.clone(),
        [
            DesignViewerPropertySection::new("same", "One", []),
            DesignViewerPropertySection::new("same", "Two", []),
        ],
    );
    let duplicate_rows = DesignViewerPropertiesViewData::new(
        target,
        [DesignViewerPropertySection::new(
            "layout",
            "Layout",
            [
                DesignViewerPropertyRow::new("x", "X", "0"),
                DesignViewerPropertyRow::new("x", "X duplicate", "1"),
            ],
        )],
    );
    assert!(!duplicate_sections.is_valid());
    assert!(!duplicate_rows.is_valid());
}

#[test]
fn add_auto_layout_projection_requires_one_exact_unique_node_target() {
    let target = DesignPanelTarget::Nodes {
        node_ids: vec!["second".into(), "first".into()],
    };
    let projection = DesignAddAutoLayoutViewData::eligible(target.clone());
    assert!(projection.is_valid());
    assert!(projection.can_request());
    assert!(
        !DesignAddAutoLayoutViewData::eligible(DesignPanelTarget::Page {
            page_id: "page".into(),
        })
        .is_valid()
    );
    assert!(
        !DesignAddAutoLayoutViewData::eligible(DesignPanelTarget::Nodes {
            node_ids: vec!["same".into(), "same".into()],
        })
        .is_valid()
    );
    assert!(
        !DesignAddAutoLayoutViewData::eligible(DesignPanelTarget::Nodes { node_ids: vec![] })
            .is_valid()
    );

    let disabled = projection.clone().disabled("Locked selection");
    assert!(disabled.is_valid());
    assert!(!disabled.can_request());
    let action = DesignPanelAction::AddAutoLayoutRequested { target };
    assert!(action.targeted_node_action().is_none());
    assert!(action.retargeted_legacy_node_action("ignored").is_none());
}

#[test]
fn smart_selection_projection_validates_all_geometry_kinds_and_exact_ordered_targets() {
    let target = DesignPanelTarget::Nodes {
        node_ids: vec!["third".into(), "first".into(), "second".into()],
    };
    let none = DesignSmartSelectionViewData::new(target.clone(), DesignSmartSelectionKind::None);
    let horizontal = DesignSmartSelectionViewData::horizontal(
        target.clone(),
        DesignSmartSelectionSpacingViewData::uniform(24.),
    );
    let vertical = DesignSmartSelectionViewData::vertical(
        target.clone(),
        DesignSmartSelectionSpacingViewData::mixed(),
    );
    let two_dimensional = DesignSmartSelectionViewData::two_dimensional(
        target.clone(),
        DesignSmartSelectionSpacingViewData::uniform(24.),
        DesignSmartSelectionSpacingViewData::mixed(),
    );

    for projection in [&none, &horizontal, &vertical, &two_dimensional] {
        assert!(projection.is_valid());
        assert_eq!(projection.target, target);
    }
    assert_eq!(
        horizontal
            .spacing(DesignSmartSelectionAxis::Horizontal)
            .and_then(|spacing| spacing.value.uniform()),
        Some(24.)
    );
    assert!(
        vertical
            .spacing(DesignSmartSelectionAxis::Vertical)
            .is_some_and(|spacing| spacing.value.is_mixed())
    );

    let mut wrong_axis = horizontal.clone();
    wrong_axis.vertical_spacing = Some(DesignSmartSelectionSpacingViewData::uniform(8.));
    assert!(!wrong_axis.is_valid());
    let mut missing_axis = two_dimensional.clone();
    missing_axis.horizontal_spacing = None;
    assert!(!missing_axis.is_valid());

    for invalid_target in [
        DesignPanelTarget::Page {
            page_id: "page".into(),
        },
        DesignPanelTarget::Nodes {
            node_ids: vec!["only".into()],
        },
        DesignPanelTarget::Nodes {
            node_ids: vec!["same".into(), "same".into()],
        },
        DesignPanelTarget::Nodes {
            node_ids: vec!["".into(), "second".into()],
        },
    ] {
        let projection =
            DesignSmartSelectionViewData::new(invalid_target, DesignSmartSelectionKind::None);
        assert!(!projection.is_valid());
    }
}

#[test]
fn smart_selection_projection_rejects_invalid_values_and_resolves_effective_access() {
    let target = DesignPanelTarget::Nodes {
        node_ids: vec!["second".into(), "first".into()],
    };
    let editable = DesignSmartSelectionViewData::two_dimensional(
        target.clone(),
        DesignSmartSelectionSpacingViewData::uniform(12.),
        DesignSmartSelectionSpacingViewData::mixed(),
    )
    .with_operation(
        DesignSmartSelectionOperation::TidyUp,
        DesignSmartSelectionAvailability::Available,
    );
    assert!(editable.is_valid());
    assert!(editable.spacing_is_editable(DesignSmartSelectionAxis::Horizontal));
    assert!(editable.spacing_is_editable(DesignSmartSelectionAxis::Vertical));
    assert!(editable.operation_is_available(DesignSmartSelectionOperation::TidyUp));

    let read_only = editable.clone().read_only("Locked selection");
    assert!(read_only.is_valid());
    assert!(!read_only.spacing_is_editable(DesignSmartSelectionAxis::Horizontal));
    assert!(!read_only.operation_is_available(DesignSmartSelectionOperation::TidyUp));

    let disabled = DesignSmartSelectionViewData::horizontal(
        target.clone(),
        DesignSmartSelectionSpacingViewData::uniform(12.).disabled("Mixed layer locks"),
    )
    .with_operation(
        DesignSmartSelectionOperation::TidyUp,
        DesignSmartSelectionAvailability::disabled("Tidy up is unavailable"),
    );
    assert!(disabled.is_valid());
    assert!(!disabled.spacing_is_editable(DesignSmartSelectionAxis::Horizontal));
    assert!(!disabled.operation_is_available(DesignSmartSelectionOperation::TidyUp));

    let mut invalid_number = editable.clone();
    invalid_number.horizontal_spacing =
        Some(DesignSmartSelectionSpacingViewData::uniform(f32::NAN));
    assert!(!invalid_number.is_valid());
    let mut empty_reason = editable.clone();
    empty_reason.tidy_up = Some(DesignSmartSelectionAvailability::disabled(" "));
    assert!(!empty_reason.is_valid());
    assert!(!editable.clone().read_only("").is_valid());

    for action in [
        DesignPanelAction::SmartSelectionSpacingEditRequested {
            target: target.clone(),
            axis: DesignSmartSelectionAxis::Horizontal,
            value: 12.,
            phase: DesignPanelEditPhase::Commit,
        },
        DesignPanelAction::SmartSelectionArrangeRequested {
            target,
            operation: DesignSmartSelectionOperation::TidyUp,
        },
    ] {
        assert_eq!(action.compatibility_path(), None);
        assert!(action.targeted_node_action().is_none());
        assert!(action.retargeted_legacy_node_action("ignored").is_none());
    }
}

#[test]
fn color_contrast_catalog_rejects_ambiguous_or_non_native_modes() {
    let target =
        DesignColorContrastPaintTarget::paint("shape", DesignPanelCollection::Fill, "fill", 0);
    let valid_leaf = DesignColorContrastLeafViewData::new(
        DesignPaintColorTarget::Solid,
        DesignColor::WHITE,
        2.19,
        DesignColorContrastCategory::Graphics,
    )
    .with_corrections([DesignColorContrastCorrection::new(
        DesignColorContrastCategory::Graphics,
        DesignColorContrastLevel::Aa,
        DesignColor::rgb(0, 0, 0),
    )]);
    let valid = DesignColorContrastPaintViewData::new(target.clone(), [valid_leaf.clone()]);
    assert!(valid.is_valid());
    assert_eq!(
        valid.leaf(&DesignPaintColorTarget::Solid).and_then(|leaf| {
            leaf.correction(
                DesignColorContrastCategory::Graphics,
                DesignColorContrastLevel::Aa,
            )
        }),
        Some(DesignColor::rgb(0, 0, 0))
    );

    let duplicate =
        DesignColorContrastPaintViewData::new(target.clone(), [valid_leaf.clone(), valid_leaf]);
    assert!(!duplicate.is_valid());

    let invalid_aaa_graphics = DesignColorContrastLeafViewData::new(
        DesignPaintColorTarget::Solid,
        DesignColor::WHITE,
        1.,
        DesignColorContrastCategory::Graphics,
    )
    .with_corrections([DesignColorContrastCorrection::new(
        DesignColorContrastCategory::Graphics,
        DesignColorContrastLevel::Aaa,
        DesignColor::BLACK,
    )]);
    assert!(!invalid_aaa_graphics.is_valid());

    let invalid_auto_resolution = DesignColorContrastLeafViewData::new(
        DesignPaintColorTarget::Solid,
        DesignColor::WHITE,
        4.5,
        DesignColorContrastCategory::Auto,
    );
    assert!(!invalid_auto_resolution.is_valid());

    let catalog = DesignColorContrastViewData::new([
        valid,
        DesignColorContrastPaintViewData::new(target, [invalid_auto_resolution]),
    ]);
    assert_eq!(catalog.paints.len(), 1);
}

#[test]
fn nudge_settings_validate_host_preferences_and_preserve_figma_defaults() {
    let defaults = DesignNudgeSettings::default();
    assert_eq!((defaults.small(), defaults.big()), (1., 10.));
    assert_eq!(
        DesignNudgeSettings::new(0.5, 8.)
            .map(|settings| { (settings.amount(false), settings.amount(true)) }),
        Some((0.5, 8.))
    );
    for (small, big) in [
        (0., 10.),
        (-1., 10.),
        (1., 0.),
        (1., -10.),
        (f32::NAN, 10.),
        (1., f32::INFINITY),
    ] {
        assert!(DesignNudgeSettings::new(small, big).is_none());
    }
}

#[test]
fn scrub_speed_bands_are_deterministic_at_every_boundary() {
    assert_eq!(
        DesignScrubSpeed::from_vertical_displacement(DESIGN_SCRUB_DOUBLE_SPEED_Y_THRESHOLD - 1.),
        DesignScrubSpeed::Double
    );
    assert_eq!(
        DesignScrubSpeed::from_vertical_displacement(DESIGN_SCRUB_DOUBLE_SPEED_Y_THRESHOLD),
        DesignScrubSpeed::Double
    );
    assert_eq!(
        DesignScrubSpeed::from_vertical_displacement(DESIGN_SCRUB_DOUBLE_SPEED_Y_THRESHOLD + 1.),
        DesignScrubSpeed::Normal
    );
    assert_eq!(
        DesignScrubSpeed::from_vertical_displacement(DESIGN_SCRUB_HALF_SPEED_Y_THRESHOLD),
        DesignScrubSpeed::Half
    );
    assert_eq!(
        DesignScrubSpeed::from_vertical_displacement(DESIGN_SCRUB_QUARTER_SPEED_Y_THRESHOLD),
        DesignScrubSpeed::Quarter
    );
    assert_eq!(
        DesignScrubSpeed::from_vertical_displacement(f32::NAN),
        DesignScrubSpeed::Normal
    );
    assert_eq!(
        [
            DesignScrubSpeed::Double.multiplier(),
            DesignScrubSpeed::Normal.multiplier(),
            DesignScrubSpeed::Half.multiplier(),
            DesignScrubSpeed::Quarter.multiplier(),
        ],
        [2., 1., 0.5, 0.25]
    );
}
