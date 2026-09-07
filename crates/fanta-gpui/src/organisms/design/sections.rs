//! Deterministic Design-panel section visibility and ordering.
//!
//! This resolver is deliberately independent of GPUI rendering. It turns the
//! controlled node read model into the ordered section identifiers a renderer
//! should compose.

use super::{DesignPanelNode, DesignPanelNodeKind, DesignPanelSection, DesignPanelWorkspaceMode};

/// The current high-level order of sections in Figma's Design panel.
///
/// Some existing section identifiers are retained for compatibility even when
/// their controls are composed inline. In particular, constraints now render
/// inside Position, `Layer` is the existing identifier for Appearance, and
/// `LayoutGrid` represents Layout guides.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum DesignPanelSectionBand {
    Position,
    Layout,
    Appearance,
    TypeSpecific,
    Fill,
    Stroke,
    Effects,
    LayoutGuides,
    Export,
}

impl DesignPanelSectionBand {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Position => "Position",
            Self::Layout => "Layout",
            Self::Appearance => "Appearance",
            Self::TypeSpecific => "Properties",
            Self::Fill => "Fill",
            Self::Stroke => "Stroke",
            Self::Effects => "Effects",
            Self::LayoutGuides => "Layout guides",
            Self::Export => "Export",
        }
    }
}

/// Returns the Figma ordering band for an existing section identifier.
pub const fn design_panel_section_band(section: DesignPanelSection) -> DesignPanelSectionBand {
    match section {
        DesignPanelSection::Position | DesignPanelSection::Constraints => {
            DesignPanelSectionBand::Position
        }
        DesignPanelSection::Layout => DesignPanelSectionBand::Layout,
        DesignPanelSection::Layer => DesignPanelSectionBand::Appearance,
        DesignPanelSection::Selection
        | DesignPanelSection::Component
        | DesignPanelSection::Instance
        | DesignPanelSection::Section
        | DesignPanelSection::Transform
        | DesignPanelSection::Geometry
        | DesignPanelSection::Mask
        | DesignPanelSection::Typography
        | DesignPanelSection::Media => DesignPanelSectionBand::TypeSpecific,
        DesignPanelSection::Fill => DesignPanelSectionBand::Fill,
        DesignPanelSection::Stroke => DesignPanelSectionBand::Stroke,
        DesignPanelSection::Effects => DesignPanelSectionBand::Effects,
        DesignPanelSection::LayoutGrid => DesignPanelSectionBand::LayoutGuides,
        DesignPanelSection::Export => DesignPanelSectionBand::Export,
    }
}

/// Returns whether an already-resolved section belongs in one workspace.
///
/// Design preserves every host-resolved capability. Draw intentionally uses a
/// conservative allow-list: Position is composed directly below the node
/// header, Layout and Appearance use Draw-specific renderers, and only
/// drawing-relevant type/paint/export sections remain. Constraints stay
/// available inside Position rather than through their compatibility section.
pub const fn design_panel_section_is_visible_in_workspace(
    section: DesignPanelSection,
    workspace_mode: DesignPanelWorkspaceMode,
) -> bool {
    match workspace_mode {
        DesignPanelWorkspaceMode::Design => true,
        DesignPanelWorkspaceMode::Draw => match section {
            DesignPanelSection::Position
            | DesignPanelSection::Layout
            | DesignPanelSection::Layer
            | DesignPanelSection::Transform
            | DesignPanelSection::Geometry
            | DesignPanelSection::Mask
            | DesignPanelSection::Typography
            | DesignPanelSection::Fill
            | DesignPanelSection::Stroke
            | DesignPanelSection::Effects
            | DesignPanelSection::Export => true,
            DesignPanelSection::Selection
            | DesignPanelSection::Component
            | DesignPanelSection::Instance
            | DesignPanelSection::Constraints
            | DesignPanelSection::Section
            | DesignPanelSection::Media
            | DesignPanelSection::LayoutGrid => false,
        },
    }
}

/// Resolves the visible section identifiers in deterministic Design-panel
/// order.
///
/// The node kind controls structural capabilities, while optional type data
/// controls data-dependent blocks such as Typography. Image and video settings
/// live in Fill paint payloads rather than a duplicate Media section. Empty
/// collections do not hide addable sections: a fill-capable node still needs
/// its Fill section when it has no paints.
pub fn resolve_design_panel_sections(node: &DesignPanelNode) -> Vec<DesignPanelSection> {
    resolve_design_panel_sections_with_export(node, true)
}

/// Resolves sections while honoring host export permission.
///
/// The additive flag keeps the original resolver source-compatible for hosts
/// that do not yet model inspection permissions.
pub fn resolve_design_panel_sections_with_export(
    node: &DesignPanelNode,
    include_export: bool,
) -> Vec<DesignPanelSection> {
    if let Some(capabilities) = &node.capabilities {
        let mut sections = Vec::with_capacity(capabilities.sections.len());
        for section in capabilities.sections.iter().copied() {
            if (!include_export && section == DesignPanelSection::Export)
                || !node.supports_section(section)
                || sections.contains(&section)
            {
                continue;
            }
            sections.push(section);
        }
        return sections;
    }

    let mut sections = Vec::with_capacity(12);

    sections.push(DesignPanelSection::Position);

    if node.supports_dimensions() {
        sections.push(DesignPanelSection::Layout);
    }

    if node.kind != DesignPanelNodeKind::Slice
        && (node.supports_visibility()
            || node.supports_layer_appearance()
            || node.corner_capabilities.has_any()
            || node.shape_geometry.has_appearance_controls())
    {
        sections.push(DesignPanelSection::Layer);
    }

    if !node.selection_color_aggregate.colors.is_empty() || !node.selection_colors.is_empty() {
        sections.push(DesignPanelSection::Selection);
    }

    if let Some(role) = node.component_role() {
        sections.push(if role.uses_instance_section() {
            DesignPanelSection::Instance
        } else {
            DesignPanelSection::Component
        });
    }

    if node.section.is_some() {
        sections.push(DesignPanelSection::Section);
    }
    if node.kind == DesignPanelNodeKind::TransformGroup {
        sections.push(DesignPanelSection::Transform);
    }
    if has_legacy_geometry_properties(node) {
        sections.push(DesignPanelSection::Geometry);
    }
    if node.effective_is_mask() {
        sections.push(DesignPanelSection::Mask);
    }
    if node.typography.is_some() {
        sections.push(DesignPanelSection::Typography);
    }
    if node.supports_fill() {
        sections.push(DesignPanelSection::Fill);
    }
    if node.supports_stroke() {
        sections.push(DesignPanelSection::Stroke);
    }
    if node.supports_effects() {
        sections.push(DesignPanelSection::Effects);
    }
    if node.supports_layout_guides() {
        sections.push(DesignPanelSection::LayoutGrid);
    }
    if include_export {
        sections.push(DesignPanelSection::Export);
    }

    debug_assert!(
        sections
            .windows(2)
            .all(|pair| design_panel_section_band(pair[0]) <= design_panel_section_band(pair[1]))
    );
    sections
}

fn has_legacy_geometry_properties(node: &DesignPanelNode) -> bool {
    matches!(node.shape_geometry, super::DesignShapeGeometry::Table(_))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::design::{
        DesignArcData, DesignComponentContext, DesignComponentProperty, DesignComponentRole,
        DesignPolygonGeometry, DesignShapeGeometry, DesignTypography,
    };
    use std::collections::HashSet;

    fn sections(kind: DesignPanelNodeKind) -> Vec<DesignPanelSection> {
        resolve_design_panel_sections(&DesignPanelNode::new("node", kind.label(), kind))
    }

    #[test]
    fn export_permission_can_omit_the_export_band() {
        let node = DesignPanelNode::new("node", "Rectangle", DesignPanelNodeKind::Rectangle);
        assert!(
            !resolve_design_panel_sections_with_export(&node, false)
                .contains(&DesignPanelSection::Export)
        );
    }

    #[test]
    fn authoritative_other_capabilities_control_visibility_and_order() {
        let node = DesignPanelNode::new("plugin", "Plugin node", DesignPanelNodeKind::Other)
            .with_capabilities(
                super::super::DesignPanelNodeCapabilities::for_node_kind(
                    DesignPanelNodeKind::Other,
                )
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

        assert_eq!(
            resolve_design_panel_sections(&node),
            vec![
                DesignPanelSection::Position,
                DesignPanelSection::Layout,
                DesignPanelSection::Layer,
                DesignPanelSection::Stroke,
                DesignPanelSection::LayoutGrid,
                DesignPanelSection::Export,
            ]
        );
        assert_eq!(
            resolve_design_panel_sections_with_export(&node, false),
            vec![
                DesignPanelSection::Position,
                DesignPanelSection::Layout,
                DesignPanelSection::Layer,
                DesignPanelSection::Stroke,
                DesignPanelSection::LayoutGrid,
            ]
        );
    }

    #[test]
    fn rectangle_uses_the_canonical_base_order() {
        assert_eq!(
            sections(DesignPanelNodeKind::Rectangle),
            vec![
                DesignPanelSection::Position,
                DesignPanelSection::Layout,
                DesignPanelSection::Layer,
                DesignPanelSection::Fill,
                DesignPanelSection::Stroke,
                DesignPanelSection::Effects,
                DesignPanelSection::Export,
            ]
        );
    }

    #[test]
    fn frame_adds_layout_guides_before_export() {
        assert_eq!(
            sections(DesignPanelNodeKind::Frame),
            vec![
                DesignPanelSection::Position,
                DesignPanelSection::Layout,
                DesignPanelSection::Layer,
                DesignPanelSection::Fill,
                DesignPanelSection::Stroke,
                DesignPanelSection::Effects,
                DesignPanelSection::LayoutGrid,
                DesignPanelSection::Export,
            ]
        );
    }

    #[test]
    fn text_nodes_insert_typography_before_paints() {
        for kind in [DesignPanelNodeKind::Text, DesignPanelNodeKind::TextPath] {
            let resolved = sections(kind);
            assert!(
                resolved
                    .iter()
                    .position(|section| *section == DesignPanelSection::Typography)
                    < resolved
                        .iter()
                        .position(|section| *section == DesignPanelSection::Fill),
                "{kind:?} should expose typography before fills"
            );
            assert!(!resolved.contains(&DesignPanelSection::Media));
        }
    }

    #[test]
    fn media_settings_live_in_fill_paints_without_a_duplicate_section() {
        for kind in [DesignPanelNodeKind::Image, DesignPanelNodeKind::Video] {
            let resolved = sections(kind);
            assert!(!resolved.contains(&DesignPanelSection::Media));
            assert!(resolved.contains(&DesignPanelSection::Fill));
            assert!(!resolved.contains(&DesignPanelSection::Typography));
        }
    }

    #[test]
    fn instances_and_components_use_distinct_type_specific_sections() {
        let instance = sections(DesignPanelNodeKind::Instance);
        assert!(instance.contains(&DesignPanelSection::Instance));
        assert!(!instance.contains(&DesignPanelSection::Component));

        for kind in [
            DesignPanelNodeKind::Component,
            DesignPanelNodeKind::ComponentSet,
        ] {
            let resolved = sections(kind);
            assert!(resolved.contains(&DesignPanelSection::Component));
            assert!(!resolved.contains(&DesignPanelSection::Instance));
        }
    }

    #[test]
    fn canonical_shape_controls_compose_in_appearance_and_masks_have_their_own_section() {
        for kind in [
            DesignPanelNodeKind::Polygon,
            DesignPanelNodeKind::Star,
            DesignPanelNodeKind::BooleanOperation,
        ] {
            let resolved = sections(kind);
            assert!(
                resolved.contains(&DesignPanelSection::Layer),
                "{kind:?} should retain Appearance"
            );
            assert!(
                !resolved.contains(&DesignPanelSection::Geometry),
                "{kind:?} must not emit the compatibility Geometry section"
            );
        }

        let mut partial_ellipse =
            DesignPanelNode::new("ellipse", "Arc", DesignPanelNodeKind::Ellipse);
        assert!(
            !resolve_design_panel_sections(&partial_ellipse)
                .contains(&DesignPanelSection::Geometry),
            "a full ellipse has Appearance without a duplicate Arc section"
        );
        partial_ellipse.shape_geometry =
            DesignShapeGeometry::Ellipse(DesignArcData::new(0., std::f32::consts::PI, 0.));
        let partial_sections = resolve_design_panel_sections(&partial_ellipse);
        assert!(partial_sections.contains(&DesignPanelSection::Layer));
        assert!(!partial_sections.contains(&DesignPanelSection::Geometry));

        let mask_sections = sections(DesignPanelNodeKind::Mask);
        assert!(mask_sections.contains(&DesignPanelSection::Mask));
        assert!(!mask_sections.contains(&DesignPanelSection::Geometry));

        assert!(!sections(DesignPanelNodeKind::Rectangle).contains(&DesignPanelSection::Geometry));
        assert!(sections(DesignPanelNodeKind::Table).contains(&DesignPanelSection::Geometry));
    }

    #[test]
    fn explicit_geometry_capabilities_preserve_legacy_shape_projection() {
        let node = DesignPanelNode::new(
            "legacy-boolean",
            "Legacy Boolean",
            DesignPanelNodeKind::BooleanOperation,
        )
        .with_capabilities(
            super::super::DesignPanelNodeCapabilities::for_node_kind(
                DesignPanelNodeKind::BooleanOperation,
            )
            .with_sections([
                DesignPanelSection::Position,
                DesignPanelSection::Geometry,
                DesignPanelSection::Export,
            ]),
        );

        assert_eq!(
            resolve_design_panel_sections(&node),
            vec![
                DesignPanelSection::Position,
                DesignPanelSection::Geometry,
                DesignPanelSection::Export,
            ]
        );
    }

    #[test]
    fn structural_kinds_omit_unsupported_sections() {
        for kind in [
            DesignPanelNodeKind::Group,
            DesignPanelNodeKind::TransformGroup,
        ] {
            let group = sections(kind);
            assert!(!group.contains(&DesignPanelSection::Fill));
            assert!(!group.contains(&DesignPanelSection::Stroke));
        }
        assert!(
            sections(DesignPanelNodeKind::TransformGroup).contains(&DesignPanelSection::Transform)
        );

        let line = sections(DesignPanelNodeKind::Line);
        assert!(line.contains(&DesignPanelSection::Layout));
        assert!(!line.contains(&DesignPanelSection::Fill));
        assert!(line.contains(&DesignPanelSection::Stroke));

        assert_eq!(
            sections(DesignPanelNodeKind::Slice),
            vec![
                DesignPanelSection::Position,
                DesignPanelSection::Layout,
                DesignPanelSection::Export,
            ]
        );
    }

    #[test]
    fn slot_uses_frame_like_sections_and_layout_guides() {
        assert_eq!(
            sections(DesignPanelNodeKind::Slot),
            vec![
                DesignPanelSection::Position,
                DesignPanelSection::Layout,
                DesignPanelSection::Layer,
                DesignPanelSection::Component,
                DesignPanelSection::Fill,
                DesignPanelSection::Stroke,
                DesignPanelSection::Effects,
                DesignPanelSection::LayoutGrid,
                DesignPanelSection::Export,
            ]
        );
    }

    #[test]
    fn multiple_selection_uses_aggregate_sections() {
        assert_eq!(
            sections(DesignPanelNodeKind::MultipleSelection),
            vec![
                DesignPanelSection::Position,
                DesignPanelSection::Layout,
                DesignPanelSection::Layer,
                DesignPanelSection::Selection,
                DesignPanelSection::Effects,
                DesignPanelSection::Export,
            ]
        );
    }

    #[test]
    fn optional_host_data_adds_type_specific_sections_deterministically() {
        let mut node = DesignPanelNode::new("custom", "Custom", DesignPanelNodeKind::Other);
        node.component_properties
            .push(DesignComponentProperty::variant(
                "state",
                "State",
                "Default",
                "Default",
                vec!["Default".into()],
            ));
        node.shape_geometry = DesignShapeGeometry::Polygon(DesignPolygonGeometry::new(6));
        node.typography = Some(DesignTypography::default());

        let resolved = resolve_design_panel_sections(&node);
        let type_specific: Vec<_> = resolved
            .into_iter()
            .filter(|section| {
                design_panel_section_band(*section) == DesignPanelSectionBand::TypeSpecific
            })
            .collect();
        assert_eq!(
            type_specific,
            vec![
                DesignPanelSection::Component,
                DesignPanelSection::Typography,
            ]
        );
    }

    #[test]
    fn section_uses_its_native_contract_without_blend_or_effects() {
        let resolved = sections(DesignPanelNodeKind::Section);
        assert_eq!(
            resolved,
            vec![
                DesignPanelSection::Position,
                DesignPanelSection::Layout,
                DesignPanelSection::Layer,
                DesignPanelSection::Section,
                DesignPanelSection::Fill,
                DesignPanelSection::Stroke,
                DesignPanelSection::Export,
            ]
        );
        assert!(!resolved.contains(&DesignPanelSection::Effects));
    }

    #[test]
    fn explicit_roles_route_variant_slot_and_instance_contexts() {
        let variant = DesignPanelNode::new("variant", "Default", DesignPanelNodeKind::Component)
            .with_component_context(DesignComponentContext::new(
                DesignComponentRole::VariantChild,
            ));
        assert!(resolve_design_panel_sections(&variant).contains(&DesignPanelSection::Component));

        let slot_instance =
            DesignPanelNode::new("slot-instance", "Content", DesignPanelNodeKind::Slot)
                .with_component_context(DesignComponentContext::new(
                    DesignComponentRole::SlotInstance,
                ));
        let sections = resolve_design_panel_sections(&slot_instance);
        assert!(sections.contains(&DesignPanelSection::Instance));
        assert!(!sections.contains(&DesignPanelSection::Component));
    }

    #[test]
    fn every_node_kind_is_unique_band_sorted_and_export_terminated() {
        for kind in DesignPanelNodeKind::ALL {
            let resolved = sections(kind);
            assert_eq!(resolved.last(), Some(&DesignPanelSection::Export));
            assert!(
                resolved.windows(2).all(|pair| {
                    design_panel_section_band(pair[0]) <= design_panel_section_band(pair[1])
                }),
                "{kind:?} is not in canonical order: {resolved:?}"
            );

            let unique: HashSet<_> = resolved.iter().copied().collect();
            assert_eq!(
                unique.len(),
                resolved.len(),
                "{kind:?} contains duplicate sections"
            );
        }
    }

    #[test]
    fn legacy_sections_map_to_current_figma_bands() {
        assert_eq!(
            design_panel_section_band(DesignPanelSection::Constraints),
            DesignPanelSectionBand::Position
        );
        assert_eq!(
            design_panel_section_band(DesignPanelSection::Layer),
            DesignPanelSectionBand::Appearance
        );
        assert_eq!(
            design_panel_section_band(DesignPanelSection::LayoutGrid),
            DesignPanelSectionBand::LayoutGuides
        );
    }

    #[test]
    fn draw_workspace_filter_is_exhaustive_and_conservative() {
        let sections = [
            DesignPanelSection::Selection,
            DesignPanelSection::Component,
            DesignPanelSection::Instance,
            DesignPanelSection::Position,
            DesignPanelSection::Layout,
            DesignPanelSection::Constraints,
            DesignPanelSection::Layer,
            DesignPanelSection::Section,
            DesignPanelSection::Transform,
            DesignPanelSection::Geometry,
            DesignPanelSection::Mask,
            DesignPanelSection::Typography,
            DesignPanelSection::Media,
            DesignPanelSection::Fill,
            DesignPanelSection::Stroke,
            DesignPanelSection::Effects,
            DesignPanelSection::LayoutGrid,
            DesignPanelSection::Export,
        ];
        let expected_draw = HashSet::from([
            DesignPanelSection::Position,
            DesignPanelSection::Layout,
            DesignPanelSection::Layer,
            DesignPanelSection::Transform,
            DesignPanelSection::Geometry,
            DesignPanelSection::Mask,
            DesignPanelSection::Typography,
            DesignPanelSection::Fill,
            DesignPanelSection::Stroke,
            DesignPanelSection::Effects,
            DesignPanelSection::Export,
        ]);

        for section in sections {
            assert!(
                design_panel_section_is_visible_in_workspace(
                    section,
                    DesignPanelWorkspaceMode::Design,
                ),
                "Design must preserve the resolver result for {section:?}"
            );
            assert_eq!(
                design_panel_section_is_visible_in_workspace(
                    section,
                    DesignPanelWorkspaceMode::Draw,
                ),
                expected_draw.contains(&section),
                "unexpected Draw visibility for {section:?}"
            );
        }
    }
}
