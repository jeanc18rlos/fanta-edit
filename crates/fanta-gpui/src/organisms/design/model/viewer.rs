use std::collections::HashSet;

use gpui::SharedString;

use super::*;

/// Color/code representation selected for a view-only Properties section.
///
/// Figma exposes these five representations for Borders. The selected value
/// remains host owned because switching representations changes the exact
/// displayed and copied section summary.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DesignViewerColorRepresentation {
    Css,
    Hex,
    Rgb,
    Hsl,
    Hsb,
}

impl DesignViewerColorRepresentation {
    pub const ALL: [Self; 5] = [Self::Css, Self::Hex, Self::Rgb, Self::Hsl, Self::Hsb];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Css => "CSS",
            Self::Hex => "Hex",
            Self::Rgb => "RGB",
            Self::Hsl => "HSL",
            Self::Hsb => "HSB",
        }
    }
}

/// One exact host-formatted readout in the view-only Properties projection.
///
/// When `property` is present, activating the row reuses
/// [`DesignPanelAction::PropertyCopyRequested`]. Rows without a canonical
/// [`DesignPanelProperty`] remain inspectable and are copied through their
/// containing section's explicit copy action.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DesignViewerPropertyRow {
    pub id: SharedString,
    pub label: SharedString,
    pub displayed_value: SharedString,
    pub property: Option<DesignPanelProperty>,
}

impl DesignViewerPropertyRow {
    pub fn new(
        id: impl Into<SharedString>,
        label: impl Into<SharedString>,
        displayed_value: impl Into<SharedString>,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            displayed_value: displayed_value.into(),
            property: None,
        }
    }

    pub const fn with_property(mut self, property: DesignPanelProperty) -> Self {
        self.property = Some(property);
        self
    }
}

/// One stable, host-owned section in Figma's view-only Properties projection.
///
/// `summary` supports multiline content such as a Text layer's characters.
/// `copy_value` is the exact payload a host should place on the clipboard; it
/// intentionally need not equal any one displayed row. When
/// `color_representation` is present, the panel offers all five Figma viewer
/// representations and requests a host echo before changing the section.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DesignViewerPropertySection {
    pub id: SharedString,
    pub title: SharedString,
    pub summary: Option<SharedString>,
    pub rows: Vec<DesignViewerPropertyRow>,
    pub copy_label: SharedString,
    pub copy_value: Option<SharedString>,
    pub color_representation: Option<DesignViewerColorRepresentation>,
}

impl DesignViewerPropertySection {
    pub fn new(
        id: impl Into<SharedString>,
        title: impl Into<SharedString>,
        rows: impl IntoIterator<Item = DesignViewerPropertyRow>,
    ) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            summary: None,
            rows: rows.into_iter().collect(),
            copy_label: "Copy".into(),
            copy_value: None,
            color_representation: None,
        }
    }

    pub fn text_content(id: impl Into<SharedString>, content: impl Into<SharedString>) -> Self {
        let content = content.into();
        Self::new(id, "Content", [])
            .with_summary(content.clone())
            .with_copy_value(content)
    }

    pub fn with_summary(mut self, summary: impl Into<SharedString>) -> Self {
        self.summary = Some(summary.into());
        self
    }

    pub fn with_copy_value(mut self, copy_value: impl Into<SharedString>) -> Self {
        self.copy_value = Some(copy_value.into());
        self
    }

    pub fn with_copy_label(mut self, copy_label: impl Into<SharedString>) -> Self {
        self.copy_label = copy_label.into();
        self
    }

    pub fn with_copy_all(self) -> Self {
        self.with_copy_label("Copy all")
    }

    pub const fn with_color_representation(
        mut self,
        representation: DesignViewerColorRepresentation,
    ) -> Self {
        self.color_representation = Some(representation);
        self
    }

    pub fn row(&self, row_id: &str) -> Option<&DesignViewerPropertyRow> {
        self.rows.iter().find(|row| row.id.as_ref() == row_id)
    }
}

/// Exact viewer-only Properties projection for one ordered selection target.
///
/// The panel never derives Content, CSS, or color conversions from editor
/// fields. Hosts supply the strings they want displayed and copied, then echo
/// a fresh snapshot after a representation request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DesignViewerPropertiesViewData {
    pub target: DesignPanelTarget,
    pub sections: Vec<DesignViewerPropertySection>,
}

impl DesignViewerPropertiesViewData {
    pub fn new(
        target: DesignPanelTarget,
        sections: impl IntoIterator<Item = DesignViewerPropertySection>,
    ) -> Self {
        Self {
            target,
            sections: sections.into_iter().collect(),
        }
    }

    pub fn section(&self, section_id: &str) -> Option<&DesignViewerPropertySection> {
        self.sections
            .iter()
            .find(|section| section.id.as_ref() == section_id)
    }

    pub fn is_valid(&self) -> bool {
        let section_ids = self
            .sections
            .iter()
            .map(|section| section.id.as_ref())
            .collect::<HashSet<_>>();
        section_ids.len() == self.sections.len()
            && self.sections.iter().all(|section| {
                !section.id.is_empty()
                    && !section.title.is_empty()
                    && !section.copy_label.is_empty()
                    && section
                        .rows
                        .iter()
                        .map(|row| row.id.as_ref())
                        .collect::<HashSet<_>>()
                        .len()
                        == section.rows.len()
                    && section
                        .rows
                        .iter()
                        .all(|row| !row.id.is_empty() && !row.label.is_empty())
            })
    }
}
