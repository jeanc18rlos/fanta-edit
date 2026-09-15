//! Presentation-only coordination for Design inspector sections.
//!
//! The controller deliberately knows only section identity, workspace
//! visibility, and disclosure state. Feature renderers keep their own narrow
//! projections and translate interactions to `DesignPanelAction` at the
//! facade boundary.

use std::collections::HashSet;

use super::{
    DesignPanelNode, DesignPanelSection, DesignPanelWorkspaceMode,
    design_panel_section_is_visible_in_workspace, resolve_design_panel_sections_with_export,
};

/// Behavioral family used by the inspector composition layer.
///
/// This is intentionally private to the Design organism: reusable molecules
/// must never learn about Design-domain section identifiers.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(super) enum DesignSectionFamily {
    Selection,
    Component,
    Position,
    Layout,
    Appearance,
    Typography,
    Paint,
    Effects,
    Guides,
    Export,
    Compatibility,
}

impl DesignSectionFamily {
    pub(super) const fn for_section(section: DesignPanelSection) -> Self {
        match section {
            DesignPanelSection::Selection => Self::Selection,
            DesignPanelSection::Component | DesignPanelSection::Instance => Self::Component,
            DesignPanelSection::Position | DesignPanelSection::Constraints => Self::Position,
            DesignPanelSection::Layout => Self::Layout,
            DesignPanelSection::Layer
            | DesignPanelSection::Section
            | DesignPanelSection::Transform
            | DesignPanelSection::Mask => Self::Appearance,
            DesignPanelSection::Typography => Self::Typography,
            DesignPanelSection::Fill | DesignPanelSection::Stroke => Self::Paint,
            DesignPanelSection::Effects => Self::Effects,
            DesignPanelSection::LayoutGrid => Self::Guides,
            DesignPanelSection::Export => Self::Export,
            DesignPanelSection::Geometry | DesignPanelSection::Media => Self::Compatibility,
        }
    }
}

/// One resolved section handed from the facade to a feature renderer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct DesignSectionProjection {
    pub(super) section: DesignPanelSection,
    pub(super) family: DesignSectionFamily,
    pub(super) expanded: bool,
}

/// Owns disclosure state and section ordering for the Design inspector.
///
/// Document data never enters this type. Resolution always starts from the
/// current host-owned node and workspace projection.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct DesignSectionController {
    expanded: HashSet<DesignPanelSection>,
    constraints_expanded: bool,
    appearance_details_expanded: bool,
}

impl Default for DesignSectionController {
    fn default() -> Self {
        Self {
            expanded: [
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
            ]
            .into_iter()
            .collect(),
            constraints_expanded: true,
            appearance_details_expanded: false,
        }
    }
}

impl DesignSectionController {
    pub(super) fn resolve(
        &self,
        node: &DesignPanelNode,
        workspace: DesignPanelWorkspaceMode,
        can_export: bool,
    ) -> Vec<DesignSectionProjection> {
        resolve_design_panel_sections_with_export(node, can_export)
            .into_iter()
            .filter(|section| design_panel_section_is_visible_in_workspace(*section, workspace))
            .map(|section| DesignSectionProjection {
                section,
                family: DesignSectionFamily::for_section(section),
                expanded: self.is_expanded(section),
            })
            .collect()
    }

    pub(super) fn is_expanded(&self, section: DesignPanelSection) -> bool {
        self.expanded.contains(&section)
    }

    pub(super) fn toggle(&mut self, section: DesignPanelSection) {
        if !self.expanded.remove(&section) {
            self.expanded.insert(section);
        }
    }

    pub(super) fn constraints_expanded(&self) -> bool {
        self.constraints_expanded
    }

    pub(super) fn toggle_constraints(&mut self) {
        self.constraints_expanded = !self.constraints_expanded;
    }

    pub(super) fn appearance_details_expanded(&self) -> bool {
        self.appearance_details_expanded
    }

    pub(super) fn toggle_appearance_details(&mut self) {
        self.appearance_details_expanded = !self.appearance_details_expanded;
    }

    pub(super) fn collapse_appearance_details(&mut self) {
        self.appearance_details_expanded = false;
    }

    /// Resets disclosures whose meaning is tied to the active selection.
    pub(super) fn reset_contextual_disclosures(&mut self) {
        self.constraints_expanded = true;
        self.appearance_details_expanded = false;
    }

    #[cfg(test)]
    pub(super) fn clear_expanded(&mut self) {
        self.expanded.clear();
    }

    #[cfg(test)]
    pub(super) fn expand(&mut self, section: DesignPanelSection) {
        self.expanded.insert(section);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::design::DesignPanelNodeKind;

    #[test]
    fn disclosure_state_is_presentation_only_and_deterministic() {
        let mut controller = DesignSectionController::default();
        assert!(controller.is_expanded(DesignPanelSection::Layout));
        controller.toggle(DesignPanelSection::Layout);
        assert!(!controller.is_expanded(DesignPanelSection::Layout));
        controller.toggle(DesignPanelSection::Layout);
        assert!(controller.is_expanded(DesignPanelSection::Layout));
    }

    #[test]
    fn resolution_preserves_host_section_order_and_adds_taxonomy() {
        let node = DesignPanelNode::new("node", "Frame", DesignPanelNodeKind::Frame);
        let controller = DesignSectionController::default();
        let projections = controller.resolve(&node, DesignPanelWorkspaceMode::Design, false);
        let sections = projections
            .iter()
            .map(|projection| projection.section)
            .collect::<Vec<_>>();
        assert_eq!(
            sections,
            resolve_design_panel_sections_with_export(&node, false)
        );
        assert!(projections.iter().all(|projection| projection.expanded));
        assert!(
            projections
                .iter()
                .any(|projection| projection.family == DesignSectionFamily::Appearance)
        );
    }

    #[test]
    fn contextual_disclosures_reset_without_touching_section_expansion() {
        let mut controller = DesignSectionController::default();
        controller.toggle(DesignPanelSection::Fill);
        controller.toggle_constraints();
        controller.toggle_appearance_details();
        controller.reset_contextual_disclosures();
        assert!(!controller.is_expanded(DesignPanelSection::Fill));
        assert!(controller.constraints_expanded());
        assert!(!controller.appearance_details_expanded());
    }
}
