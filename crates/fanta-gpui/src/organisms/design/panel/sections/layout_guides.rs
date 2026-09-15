//! Projection/controller boundary for Layout Guides.
//!
//! The dense guide editors remain in the legacy module for this migration
//! slice, behind [`LayoutGuidesInspectorChrome`]. The extracted renderer owns
//! section visibility and bound-style presentation and cannot inspect facade
//! state directly.

use super::super::*;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in super::super) struct LayoutGuideTargetProjection {
    node_id: SharedString,
    guide_id: SharedString,
    index: usize,
}

impl LayoutGuideTargetProjection {
    pub(in super::super) fn new(
        node_id: SharedString,
        guide_id: SharedString,
        index: usize,
    ) -> Self {
        Self {
            node_id,
            guide_id,
            index,
        }
    }

    fn matches_live(&self, panel: &DesignPanel) -> bool {
        if self.node_id != panel.host.inspected_node().id {
            return false;
        }
        if self.guide_id.is_empty() {
            panel
                .host
                .inspected_node()
                .layout_grids
                .get(self.index)
                .is_some()
        } else {
            panel
                .host
                .inspected_node()
                .layout_grids
                .iter()
                .any(|guide| guide.id == self.guide_id)
        }
    }
}

#[derive(Clone)]
pub(in super::super) struct LayoutGuidesIdentityProjection {
    panel_id: SharedString,
    node_id: SharedString,
}

impl LayoutGuidesIdentityProjection {
    pub(in super::super) fn new(panel_id: SharedString, node_id: SharedString) -> Self {
        Self { panel_id, node_id }
    }
}

#[derive(Clone, Copy)]
pub(in super::super) struct LayoutGuidesAccessProjection {
    can_edit: bool,
    collection_supported: bool,
    supports_layout_guides: bool,
    supports_section: bool,
}

impl LayoutGuidesAccessProjection {
    pub(in super::super) const fn new(
        can_edit: bool,
        collection_supported: bool,
        supports_layout_guides: bool,
        supports_section: bool,
    ) -> Self {
        Self {
            can_edit,
            collection_supported,
            supports_layout_guides,
            supports_section,
        }
    }
}

#[derive(Clone)]
pub(in super::super) struct LayoutGuidesContentProjection {
    style_binding_name: Option<SharedString>,
    guides: Vec<LayoutGuideTargetProjection>,
}

impl LayoutGuidesContentProjection {
    pub(in super::super) fn new(
        style_binding_name: Option<SharedString>,
        guides: Vec<LayoutGuideTargetProjection>,
    ) -> Self {
        Self {
            style_binding_name,
            guides,
        }
    }
}

#[derive(Clone)]
pub(in super::super) struct LayoutGuidesProjection {
    identity: LayoutGuidesIdentityProjection,
    access: LayoutGuidesAccessProjection,
    content: LayoutGuidesContentProjection,
}

impl LayoutGuidesProjection {
    pub(in super::super) fn new(
        identity: LayoutGuidesIdentityProjection,
        access: LayoutGuidesAccessProjection,
        content: LayoutGuidesContentProjection,
    ) -> Self {
        debug_assert!(
            content
                .guides
                .iter()
                .all(|guide| guide.node_id == identity.node_id)
        );
        Self {
            identity,
            access,
            content,
        }
    }

    fn is_visible(&self) -> bool {
        self.access.supports_layout_guides && self.access.supports_section
    }
}

/// Narrow adapter around retained guide/style/variable editors.
pub(in super::super) trait LayoutGuidesInspectorChrome {
    fn layout_guides_bound_style_summary(
        &self,
        name: SharedString,
        cx: &mut Context<DesignPanel>,
    ) -> AnyElement;

    fn layout_guides_section(
        &self,
        content: AnyElement,
        cx: &mut Context<DesignPanel>,
    ) -> AnyElement;

    fn layout_guides_retained_editor(
        &self,
        projection: &LayoutGuidesProjection,
        cx: &mut Context<DesignPanel>,
    ) -> AnyElement;
}

impl LayoutGuidesInspectorChrome for DesignPanel {
    fn layout_guides_bound_style_summary(
        &self,
        name: SharedString,
        cx: &mut Context<DesignPanel>,
    ) -> AnyElement {
        self.render_bound_style_summary("layout-guides", name, cx)
    }

    fn layout_guides_section(
        &self,
        content: AnyElement,
        cx: &mut Context<DesignPanel>,
    ) -> AnyElement {
        self.render_section(
            DesignPanelSection::LayoutGrid,
            Some(DesignPanelCollection::LayoutGrid),
            content,
            cx,
        )
    }

    fn layout_guides_retained_editor(
        &self,
        projection: &LayoutGuidesProjection,
        cx: &mut Context<DesignPanel>,
    ) -> AnyElement {
        let current_targets_are_valid = self.id == projection.identity.panel_id
            && self.host.inspected_node().id == projection.identity.node_id
            && self.can_edit() == projection.access.can_edit
            && self.collection_is_supported(DesignPanelCollection::LayoutGrid)
                == projection.access.collection_supported
            && projection.content.guides.len() == self.host.inspected_node().layout_grids.len()
            && projection
                .content
                .guides
                .iter()
                .all(|guide| guide.matches_live(self));
        if !current_targets_are_valid {
            return self.layout_guides_section(div().into_any_element(), cx);
        }
        // Retained controls use Context listeners; their emitters re-check
        // access, capability, stable guide IDs, and applicable fields against
        // the latest host snapshot.
        self.render_layout_grids_retained(cx)
            .unwrap_or_else(|| self.layout_guides_section(div().into_any_element(), cx))
    }
}

pub(in super::super) fn render(
    projection: &LayoutGuidesProjection,
    chrome: &impl LayoutGuidesInspectorChrome,
    cx: &mut Context<DesignPanel>,
) -> Option<AnyElement> {
    projection.is_visible().then(|| {
        if let Some(name) = projection.content.style_binding_name.clone() {
            let content = v_flex()
                .px(px(PANEL_PADDING))
                .pb_4()
                .child(chrome.layout_guides_bound_style_summary(name, cx));
            return chrome.layout_guides_section(content.into_any_element(), cx);
        }
        chrome.layout_guides_retained_editor(projection, cx)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visibility_requires_both_layout_guide_capabilities() {
        let projection = LayoutGuidesProjection::new(
            LayoutGuidesIdentityProjection::new("panel".into(), "node".into()),
            LayoutGuidesAccessProjection::new(true, true, true, false),
            LayoutGuidesContentProjection::new(None, Vec::new()),
        );
        assert!(!projection.is_visible());
    }

    #[test]
    fn stable_target_keeps_host_identity_and_fallback_index() {
        let target = LayoutGuideTargetProjection::new("node".into(), "guide".into(), 2);
        assert_eq!(target.node_id.as_ref(), "node");
        assert_eq!(target.guide_id.as_ref(), "guide");
        assert_eq!(target.index, 2);
    }
}
