use std::collections::HashSet;

use gpui::SharedString;

use super::*;

/// Top-level resource browser shown in the Page/no-selection inspector.
#[deprecated(
    note = "use DesignPageLocalStylesViewData; canonical Page styles are current-file-only"
)]
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DesignLocalResourceCategory {
    Styles,
    VariableCollections,
}

impl DesignLocalResourceCategory {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Styles => "Local styles",
            Self::VariableCollections => "Local variables",
        }
    }
}

/// One concrete Figma resource kind that can be created from the Page panel.
#[deprecated(note = "use DesignLocalStyleKind for canonical current-file Page styles")]
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DesignLocalResourceKind {
    PaintStyle,
    TextStyle,
    EffectStyle,
    GridStyle,
    VariableCollection,
}

impl DesignLocalResourceKind {
    pub const fn category(self) -> DesignLocalResourceCategory {
        match self {
            Self::PaintStyle | Self::TextStyle | Self::EffectStyle | Self::GridStyle => {
                DesignLocalResourceCategory::Styles
            }
            Self::VariableCollection => DesignLocalResourceCategory::VariableCollections,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::PaintStyle => "Paint style",
            Self::TextStyle => "Text style",
            Self::EffectStyle => "Effect style",
            Self::GridStyle => "Grid style",
            Self::VariableCollection => "Variable collection",
        }
    }
}

/// Stable origin of a local/importable style or variable collection.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum DesignLocalResourceSource {
    Local,
    Library {
        library_id: SharedString,
        library_name: SharedString,
    },
}

impl DesignLocalResourceSource {
    pub fn library(
        library_id: impl Into<SharedString>,
        library_name: impl Into<SharedString>,
    ) -> Self {
        Self::Library {
            library_id: library_id.into(),
            library_name: library_name.into(),
        }
    }

    pub fn label(&self) -> &str {
        match self {
            Self::Local => "This file",
            Self::Library { library_name, .. } => library_name.as_ref(),
        }
    }
}

/// Whether a resource can be opened now, imported, or only inspected.
#[deprecated(note = "the canonical Page local-style tree does not browse or import libraries")]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DesignLocalResourceAvailability {
    Imported,
    Available,
    Unavailable { reason: SharedString },
}

impl DesignLocalResourceAvailability {
    pub const fn can_open(&self) -> bool {
        matches!(self, Self::Imported)
    }

    pub const fn can_import(&self) -> bool {
        matches!(self, Self::Available)
    }

    pub const fn disabled_reason(&self) -> Option<&SharedString> {
        match self {
            Self::Unavailable { reason } => Some(reason),
            Self::Imported | Self::Available => None,
        }
    }
}

#[deprecated(note = "use DesignLocalStyleItem")]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DesignLocalResource {
    pub id: SharedString,
    pub name: SharedString,
    pub kind: DesignLocalResourceKind,
    pub availability: DesignLocalResourceAvailability,
}

impl DesignLocalResource {
    pub fn local(
        id: impl Into<SharedString>,
        name: impl Into<SharedString>,
        kind: DesignLocalResourceKind,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            kind,
            availability: DesignLocalResourceAvailability::Imported,
        }
    }

    pub fn available(
        id: impl Into<SharedString>,
        name: impl Into<SharedString>,
        kind: DesignLocalResourceKind,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            kind,
            availability: DesignLocalResourceAvailability::Available,
        }
    }

    pub fn unavailable(
        id: impl Into<SharedString>,
        name: impl Into<SharedString>,
        kind: DesignLocalResourceKind,
        reason: impl Into<SharedString>,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            kind,
            availability: DesignLocalResourceAvailability::Unavailable {
                reason: reason.into(),
            },
        }
    }
}

#[deprecated(note = "use DesignLocalStyleTarget")]
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct DesignLocalResourceSelection {
    pub group_id: SharedString,
    pub source: DesignLocalResourceSource,
    pub resource_id: SharedString,
}

#[deprecated(note = "use nested DesignLocalStyleEntry folders")]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DesignLocalResourceGroup {
    pub id: SharedString,
    pub name: SharedString,
    pub source: DesignLocalResourceSource,
    pub resources: Vec<DesignLocalResource>,
}

impl DesignLocalResourceGroup {
    pub fn new(
        id: impl Into<SharedString>,
        name: impl Into<SharedString>,
        source: DesignLocalResourceSource,
        resources: impl IntoIterator<Item = DesignLocalResource>,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            source,
            resources: resources.into_iter().collect(),
        }
    }

    pub fn selection(&self, resource_id: impl Into<SharedString>) -> DesignLocalResourceSelection {
        DesignLocalResourceSelection {
            group_id: self.id.clone(),
            source: self.source.clone(),
            resource_id: resource_id.into(),
        }
    }
}

#[deprecated(note = "use DesignPageLocalStylesViewData")]
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DesignLocalResourceViewData {
    pub groups: Vec<DesignLocalResourceGroup>,
}

impl DesignLocalResourceViewData {
    pub fn new(groups: impl IntoIterator<Item = DesignLocalResourceGroup>) -> Self {
        Self {
            groups: groups.into_iter().collect(),
        }
    }

    pub fn resource(
        &self,
        selection: &DesignLocalResourceSelection,
    ) -> Option<&DesignLocalResource> {
        self.groups
            .iter()
            .find(|group| group.id == selection.group_id && group.source == selection.source)?
            .resources
            .iter()
            .find(|resource| resource.id == selection.resource_id)
    }
}

/// Canonical current-file style families shown by Figma's Page/no-selection
/// Design inspector.
///
/// The order is deliberate and matches the native Page surface. Library
/// discovery and importing belong to the contextual style pickers, not this
/// current-file tree.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DesignLocalStyleKind {
    Text,
    Color,
    Effect,
    LayoutGuide,
}

impl DesignLocalStyleKind {
    pub const ALL: [Self; 4] = [Self::Text, Self::Color, Self::Effect, Self::LayoutGuide];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Text => "Text styles",
            Self::Color => "Color styles",
            Self::Effect => "Effect styles",
            Self::LayoutGuide => "Layout guide styles",
        }
    }
}

/// Host-authored visual summary for one local style.
///
/// These snapshots are display-only. Activating a row emits an exact style
/// target and never applies the preview data to a document node.
#[derive(Clone, Debug, PartialEq)]
pub enum DesignLocalStylePreview {
    Text(DesignTypographyStyle),
    Color(Vec<DesignPaint>),
    Effect(Vec<DesignEffect>),
    LayoutGuide(Vec<DesignLayoutGrid>),
}

impl DesignLocalStylePreview {
    pub const fn kind(&self) -> DesignLocalStyleKind {
        match self {
            Self::Text(_) => DesignLocalStyleKind::Text,
            Self::Color(_) => DesignLocalStyleKind::Color,
            Self::Effect(_) => DesignLocalStyleKind::Effect,
            Self::LayoutGuide(_) => DesignLocalStyleKind::LayoutGuide,
        }
    }
}

/// One current-file style leaf in the Page inspector.
#[derive(Clone, Debug, PartialEq)]
pub struct DesignLocalStyleItem {
    pub id: SharedString,
    pub name: SharedString,
    pub description: SharedString,
    pub preview: DesignLocalStylePreview,
    pub disabled_reason: Option<SharedString>,
}

impl DesignLocalStyleItem {
    pub fn new(
        id: impl Into<SharedString>,
        name: impl Into<SharedString>,
        preview: DesignLocalStylePreview,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            description: SharedString::default(),
            preview,
            disabled_reason: None,
        }
    }

    pub fn with_description(mut self, description: impl Into<SharedString>) -> Self {
        self.description = description.into();
        self
    }

    pub fn disabled(mut self, reason: impl Into<SharedString>) -> Self {
        self.disabled_reason = Some(reason.into());
        self
    }
}

/// One recursively nested current-file style tree entry.
#[derive(Clone, Debug, PartialEq)]
pub enum DesignLocalStyleEntry {
    Folder {
        id: SharedString,
        name: SharedString,
        disabled_reason: Option<SharedString>,
        entries: Vec<DesignLocalStyleEntry>,
    },
    Style(DesignLocalStyleItem),
}

impl DesignLocalStyleEntry {
    pub fn folder(
        id: impl Into<SharedString>,
        name: impl Into<SharedString>,
        entries: impl IntoIterator<Item = DesignLocalStyleEntry>,
    ) -> Self {
        Self::Folder {
            id: id.into(),
            name: name.into(),
            disabled_reason: None,
            entries: entries.into_iter().collect(),
        }
    }

    pub fn style(item: DesignLocalStyleItem) -> Self {
        Self::Style(item)
    }

    pub fn disabled(mut self, reason: impl Into<SharedString>) -> Self {
        match &mut self {
            Self::Folder {
                disabled_reason, ..
            } => *disabled_reason = Some(reason.into()),
            Self::Style(item) => item.disabled_reason = Some(reason.into()),
        }
        self
    }

    pub const fn id(&self) -> &SharedString {
        match self {
            Self::Folder { id, .. } => id,
            Self::Style(item) => &item.id,
        }
    }

    pub const fn disabled_reason(&self) -> Option<&SharedString> {
        match self {
            Self::Folder {
                disabled_reason, ..
            } => disabled_reason.as_ref(),
            Self::Style(item) => item.disabled_reason.as_ref(),
        }
    }
}

/// One canonical style-family section in a Page local-style tree.
#[derive(Clone, Debug, PartialEq)]
pub struct DesignLocalStyleSection {
    pub kind: DesignLocalStyleKind,
    pub entries: Vec<DesignLocalStyleEntry>,
}

impl DesignLocalStyleSection {
    pub fn new(
        kind: DesignLocalStyleKind,
        entries: impl IntoIterator<Item = DesignLocalStyleEntry>,
    ) -> Self {
        Self {
            kind,
            entries: entries.into_iter().collect(),
        }
    }
}

/// Exact identity of one style at its current location.
///
/// `expected_index` counts both folders and styles in the direct parent. It
/// makes commands safe against a host echo that moved the same stable style
/// ID before the pointer event was delivered.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct DesignLocalStyleTarget {
    pub page_id: SharedString,
    pub kind: DesignLocalStyleKind,
    pub style_id: SharedString,
    pub parent_folder_id: Option<SharedString>,
    pub expected_index: usize,
}

impl DesignLocalStyleTarget {
    pub fn new(
        page_id: impl Into<SharedString>,
        kind: DesignLocalStyleKind,
        style_id: impl Into<SharedString>,
        parent_folder_id: Option<SharedString>,
        expected_index: usize,
    ) -> Self {
        Self {
            page_id: page_id.into(),
            kind,
            style_id: style_id.into(),
            parent_folder_id,
            expected_index,
        }
    }
}

/// One exact insertion point in a current-file style tree.
///
/// Both neighbors are carried because an index alone can silently retarget
/// after a concurrent host reorder.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct DesignLocalStyleInsertion {
    pub kind: DesignLocalStyleKind,
    pub parent_folder_id: Option<SharedString>,
    pub expected_index: usize,
    pub expected_before_id: Option<SharedString>,
    pub expected_after_id: Option<SharedString>,
}

impl DesignLocalStyleInsertion {
    pub fn new(
        kind: DesignLocalStyleKind,
        parent_folder_id: Option<SharedString>,
        expected_index: usize,
        expected_before_id: Option<SharedString>,
        expected_after_id: Option<SharedString>,
    ) -> Self {
        Self {
            kind,
            parent_folder_id,
            expected_index,
            expected_before_id,
            expected_after_id,
        }
    }
}

/// Commands available for one exact local style.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DesignLocalStyleCommand {
    Edit,
    GoToDefinition,
    Copy,
    Duplicate,
}

impl DesignLocalStyleCommand {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Edit => "Edit style",
            Self::GoToDefinition => "Go to definition",
            Self::Copy => "Copy style",
            Self::Duplicate => "Duplicate style",
        }
    }
}

/// Host-owned canonical current-file style projection for one exact Page.
#[derive(Clone, Debug, PartialEq)]
pub struct DesignPageLocalStylesViewData {
    pub target: DesignPanelTarget,
    pub sections: Vec<DesignLocalStyleSection>,
    pub create_disabled_reason: Option<SharedString>,
}

impl DesignPageLocalStylesViewData {
    pub fn new(
        target: DesignPanelTarget,
        sections: impl IntoIterator<Item = DesignLocalStyleSection>,
    ) -> Self {
        Self {
            target,
            sections: sections.into_iter().collect(),
            create_disabled_reason: None,
        }
    }

    pub fn for_page(
        page_id: impl Into<SharedString>,
        sections: impl IntoIterator<Item = DesignLocalStyleSection>,
    ) -> Self {
        Self::new(
            DesignPanelTarget::Page {
                page_id: page_id.into(),
            },
            sections,
        )
    }

    pub fn with_create_disabled_reason(mut self, reason: impl Into<SharedString>) -> Self {
        self.create_disabled_reason = Some(reason.into());
        self
    }

    pub fn page_id(&self) -> Option<&SharedString> {
        match &self.target {
            DesignPanelTarget::Page { page_id } if !page_id.is_empty() => Some(page_id),
            DesignPanelTarget::Page { .. } | DesignPanelTarget::Nodes { .. } => None,
        }
    }

    pub fn section(&self, kind: DesignLocalStyleKind) -> Option<&DesignLocalStyleSection> {
        self.sections.iter().find(|section| section.kind == kind)
    }

    /// Returns supplied sections in Figma's canonical family order.
    pub fn ordered_sections(&self) -> impl Iterator<Item = &DesignLocalStyleSection> {
        DesignLocalStyleKind::ALL
            .into_iter()
            .filter_map(|kind| self.section(kind))
    }

    pub fn entries(
        &self,
        kind: DesignLocalStyleKind,
        parent_folder_id: Option<&str>,
    ) -> Option<&[DesignLocalStyleEntry]> {
        let section = self.section(kind)?;
        match parent_folder_id {
            None => Some(section.entries.as_slice()),
            Some(folder_id) => {
                fn find<'a>(
                    entries: &'a [DesignLocalStyleEntry],
                    folder_id: &str,
                ) -> Option<&'a [DesignLocalStyleEntry]> {
                    for entry in entries {
                        if let DesignLocalStyleEntry::Folder {
                            id,
                            entries: children,
                            ..
                        } = entry
                        {
                            if id.as_ref() == folder_id {
                                return Some(children.as_slice());
                            }
                            if let Some(found) = find(children, folder_id) {
                                return Some(found);
                            }
                        }
                    }
                    None
                }
                find(&section.entries, folder_id)
            }
        }
    }

    pub fn folder(
        &self,
        kind: DesignLocalStyleKind,
        folder_id: &str,
    ) -> Option<&DesignLocalStyleEntry> {
        fn find<'a>(
            entries: &'a [DesignLocalStyleEntry],
            folder_id: &str,
        ) -> Option<&'a DesignLocalStyleEntry> {
            for entry in entries {
                if let DesignLocalStyleEntry::Folder {
                    id,
                    entries: children,
                    ..
                } = entry
                {
                    if id.as_ref() == folder_id {
                        return Some(entry);
                    }
                    if let Some(found) = find(children, folder_id) {
                        return Some(found);
                    }
                }
            }
            None
        }
        find(&self.section(kind)?.entries, folder_id)
    }

    /// Whether an entry and every containing folder are enabled.
    pub fn entry_path_is_enabled(&self, kind: DesignLocalStyleKind, entry_id: &str) -> bool {
        fn find(
            entries: &[DesignLocalStyleEntry],
            entry_id: &str,
            ancestors_enabled: bool,
        ) -> Option<bool> {
            for entry in entries {
                let enabled = ancestors_enabled && entry.disabled_reason().is_none();
                if entry.id().as_ref() == entry_id {
                    return Some(enabled);
                }
                if let DesignLocalStyleEntry::Folder {
                    entries: children, ..
                } = entry
                    && let Some(found) = find(children, entry_id, enabled)
                {
                    return Some(found);
                }
            }
            None
        }
        self.section(kind)
            .and_then(|section| find(&section.entries, entry_id, true))
            .unwrap_or(false)
    }

    pub fn resolve_style(&self, target: &DesignLocalStyleTarget) -> Option<&DesignLocalStyleItem> {
        if self.page_id()? != &target.page_id {
            return None;
        }
        let entries = self.entries(
            target.kind,
            target.parent_folder_id.as_ref().map(|id| id.as_ref()),
        )?;
        match entries.get(target.expected_index)? {
            DesignLocalStyleEntry::Style(item) if item.id == target.style_id => Some(item),
            DesignLocalStyleEntry::Folder { .. } | DesignLocalStyleEntry::Style(_) => None,
        }
    }

    /// Validates target shape, globally unique stable IDs, unique family
    /// sections, and exact preview/family agreement.
    pub fn is_valid(&self) -> bool {
        if self.page_id().is_none() {
            return false;
        }
        let section_kinds = self
            .sections
            .iter()
            .map(|section| section.kind)
            .collect::<HashSet<_>>();
        if section_kinds.len() != self.sections.len() {
            return false;
        }

        fn validate_entries(
            entries: &[DesignLocalStyleEntry],
            kind: DesignLocalStyleKind,
            ids: &mut HashSet<SharedString>,
        ) -> bool {
            entries.iter().all(|entry| {
                if entry.id().is_empty() || !ids.insert(entry.id().clone()) {
                    return false;
                }
                match entry {
                    DesignLocalStyleEntry::Folder {
                        entries: children, ..
                    } => validate_entries(children, kind, ids),
                    DesignLocalStyleEntry::Style(item) => item.preview.kind() == kind,
                }
            })
        }

        let mut ids = HashSet::new();
        self.sections
            .iter()
            .all(|section| validate_entries(&section.entries, section.kind, &mut ids))
    }
}

/// Canonical single solid paint exposed by `PageNode.backgrounds`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DesignPageBackground {
    pub color: DesignColor,
    pub read_only: bool,
    pub disabled_reason: Option<SharedString>,
}

impl DesignPageBackground {
    pub const fn new(color: DesignColor) -> Self {
        Self {
            color,
            read_only: false,
            disabled_reason: None,
        }
    }

    pub fn read_only(mut self, reason: impl Into<SharedString>) -> Self {
        self.read_only = true;
        self.disabled_reason = Some(reason.into());
        self
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DesignPageViewData {
    pub page_id: SharedString,
    pub background: DesignPageBackground,
    #[deprecated(note = "set DesignPageLocalStylesViewData on DesignPanel instead")]
    pub local_resources: DesignLocalResourceViewData,
}

impl DesignPageViewData {
    pub fn new(
        page_id: impl Into<SharedString>,
        background: DesignPageBackground,
        local_resources: DesignLocalResourceViewData,
    ) -> Self {
        Self {
            page_id: page_id.into(),
            background,
            local_resources,
        }
    }

    /// Canonical Page projection without the deprecated resource browser.
    pub fn canonical(page_id: impl Into<SharedString>, background: DesignPageBackground) -> Self {
        Self::new(page_id, background, DesignLocalResourceViewData::default())
    }
}
