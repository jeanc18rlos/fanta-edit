use gpui::SharedString;

use super::*;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DesignComponentPropertyKind {
    Variant,
    Text,
    Boolean,
    InstanceSwap,
    Slot,
}

impl DesignComponentPropertyKind {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Variant => "Variant",
            Self::Text => "Text",
            Self::Boolean => "Boolean",
            Self::InstanceSwap => "Instance swap",
            Self::Slot => "Slot",
        }
    }
}

/// Figma keeps Variant definitions ahead of all other component-property
/// definitions. Reorder intents carry this partition explicitly so a host can
/// reject stale cross-partition moves without relying on a display index.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DesignComponentPropertyPartition {
    Variant,
    Regular,
}

impl DesignComponentPropertyPartition {
    pub const fn for_kind(kind: DesignComponentPropertyKind) -> Self {
        if matches!(kind, DesignComponentPropertyKind::Variant) {
            Self::Variant
        } else {
            Self::Regular
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Variant => "Variant properties",
            Self::Regular => "Component properties",
        }
    }
}

/// Per-definition authoring permissions supplied by the host.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DesignComponentPropertyDefinitionCapabilities {
    pub rename: bool,
    pub edit_metadata: bool,
    pub edit_default_value: bool,
    pub edit_preferred_values: bool,
    pub edit_slot_settings: bool,
    pub delete: bool,
    pub reorder: bool,
    pub edit_variant_options: bool,
}

impl DesignComponentPropertyDefinitionCapabilities {
    pub const FULL: Self = Self {
        rename: true,
        edit_metadata: true,
        edit_default_value: true,
        edit_preferred_values: true,
        edit_slot_settings: true,
        delete: true,
        reorder: true,
        edit_variant_options: true,
    };

    pub const READ_ONLY: Self = Self {
        rename: false,
        edit_metadata: false,
        edit_default_value: false,
        edit_preferred_values: false,
        edit_slot_settings: false,
        delete: false,
        reorder: false,
        edit_variant_options: false,
    };
}

/// Stable host identity and permissions for one option in a Variant property.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DesignComponentVariantOptionAuthoring {
    pub id: SharedString,
    pub name: SharedString,
    pub can_rename: bool,
    pub can_delete: bool,
    pub can_reorder: bool,
}

impl DesignComponentVariantOptionAuthoring {
    pub fn editable(id: impl Into<SharedString>, name: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            can_rename: true,
            can_delete: true,
            can_reorder: true,
        }
    }
}

/// Authoring projection for one exact component-property definition.
///
/// The canonical definition and current/default values remain in
/// [`DesignComponentProperty`]. This record contributes only host-authorized
/// authoring controls and stable Variant-option identities.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DesignComponentPropertyDefinitionAuthoring {
    pub property_id: SharedString,
    pub capabilities: DesignComponentPropertyDefinitionCapabilities,
    pub variant_options: Vec<DesignComponentVariantOptionAuthoring>,
}

impl DesignComponentPropertyDefinitionAuthoring {
    pub fn editable(property_id: impl Into<SharedString>) -> Self {
        Self {
            property_id: property_id.into(),
            capabilities: DesignComponentPropertyDefinitionCapabilities::FULL,
            variant_options: Vec::new(),
        }
    }

    pub fn with_variant_options(
        mut self,
        options: impl IntoIterator<Item = DesignComponentVariantOptionAuthoring>,
    ) -> Self {
        self.variant_options = options.into_iter().collect();
        self
    }
}

/// Stable property choice used by selected-sublayer application controls and
/// nested-property exposure candidates.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DesignComponentPropertyChoice {
    pub property_id: SharedString,
    pub property_name: SharedString,
    pub kind: DesignComponentPropertyKind,
}

impl DesignComponentPropertyChoice {
    pub fn new(
        property_id: impl Into<SharedString>,
        property_name: impl Into<SharedString>,
        kind: DesignComponentPropertyKind,
    ) -> Self {
        Self {
            property_id: property_id.into(),
            property_name: property_name.into(),
            kind,
        }
    }
}

/// Figma surface beside which an applied component-property pill is rendered.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DesignComponentPropertyApplicationSurface {
    Appearance,
    Text,
    NestedInstance,
}

impl DesignComponentPropertyApplicationSurface {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Appearance => "Appearance",
            Self::Text => "Text",
            Self::NestedInstance => "Nested instance",
        }
    }
}

/// Host-authorized operations for one selected-sublayer property pill.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DesignAppliedComponentPropertyCapabilities {
    pub apply: bool,
    pub switch: bool,
    pub detach: bool,
}

impl DesignAppliedComponentPropertyCapabilities {
    pub const FULL: Self = Self {
        apply: true,
        switch: true,
        detach: true,
    };

    pub const READ_ONLY: Self = Self {
        apply: false,
        switch: false,
        detach: false,
    };
}

/// One host-controlled purple-pill control for the selected layer inside a
/// main component or component set.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DesignAppliedComponentPropertyControl {
    pub control_id: SharedString,
    pub layer_id: SharedString,
    pub layer_name: SharedString,
    pub surface: DesignComponentPropertyApplicationSurface,
    pub candidates: Vec<DesignComponentPropertyChoice>,
    pub applied_property_id: Option<SharedString>,
    pub capabilities: DesignAppliedComponentPropertyCapabilities,
}

impl DesignAppliedComponentPropertyControl {
    pub fn new(
        control_id: impl Into<SharedString>,
        layer_id: impl Into<SharedString>,
        layer_name: impl Into<SharedString>,
        surface: DesignComponentPropertyApplicationSurface,
        candidates: impl IntoIterator<Item = DesignComponentPropertyChoice>,
    ) -> Self {
        Self {
            control_id: control_id.into(),
            layer_id: layer_id.into(),
            layer_name: layer_name.into(),
            surface,
            candidates: candidates.into_iter().collect(),
            applied_property_id: None,
            capabilities: DesignAppliedComponentPropertyCapabilities::FULL,
        }
    }

    pub fn applied_to(mut self, property_id: impl Into<SharedString>) -> Self {
        self.applied_property_id = Some(property_id.into());
        self
    }

    pub fn candidate(&self, property_id: &str) -> Option<&DesignComponentPropertyChoice> {
        self.candidates
            .iter()
            .find(|candidate| candidate.property_id.as_ref() == property_id)
    }

    pub fn applied_property(&self) -> Option<&DesignComponentPropertyChoice> {
        self.applied_property_id
            .as_deref()
            .and_then(|property_id| self.candidate(property_id))
    }
}

/// Host-authorized operations for one nested-property exposure row.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DesignNestedComponentPropertyExposureCapabilities {
    pub expose: bool,
    pub unexpose: bool,
    pub preview: bool,
}

impl DesignNestedComponentPropertyExposureCapabilities {
    pub const FULL: Self = Self {
        expose: true,
        unexpose: true,
        preview: true,
    };

    pub const READ_ONLY: Self = Self {
        expose: false,
        unexpose: false,
        preview: false,
    };
}

/// One stable nested component-property candidate which may be exposed on the
/// selected main component/component set.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DesignNestedComponentPropertyExposureCandidate {
    pub candidate_id: SharedString,
    pub nested_instance_id: SharedString,
    pub nested_instance_name: SharedString,
    pub nested_property: DesignComponentPropertyChoice,
    pub exposed_property_id: Option<SharedString>,
    pub capabilities: DesignNestedComponentPropertyExposureCapabilities,
}

impl DesignNestedComponentPropertyExposureCandidate {
    pub fn new(
        candidate_id: impl Into<SharedString>,
        nested_instance_id: impl Into<SharedString>,
        nested_instance_name: impl Into<SharedString>,
        nested_property: DesignComponentPropertyChoice,
    ) -> Self {
        Self {
            candidate_id: candidate_id.into(),
            nested_instance_id: nested_instance_id.into(),
            nested_instance_name: nested_instance_name.into(),
            nested_property,
            exposed_property_id: None,
            capabilities: DesignNestedComponentPropertyExposureCapabilities::FULL,
        }
    }

    pub fn exposed_as(mut self, property_id: impl Into<SharedString>) -> Self {
        self.exposed_property_id = Some(property_id.into());
        self
    }
}

/// Complete host-controlled component-authoring projection.
///
/// The panel owns only focus/hover presentation. Definition application,
/// deletion, reordering, exposure, and generated identities remain host
/// operations and are reflected only after a fresh snapshot is supplied.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DesignComponentAuthoringViewData {
    pub create_kinds: Vec<DesignComponentPropertyKind>,
    pub definitions: Vec<DesignComponentPropertyDefinitionAuthoring>,
    pub applied_properties: Vec<DesignAppliedComponentPropertyControl>,
    pub exposure_candidates: Vec<DesignNestedComponentPropertyExposureCandidate>,
}

impl DesignComponentAuthoringViewData {
    pub fn definition(
        &self,
        property_id: &str,
    ) -> Option<&DesignComponentPropertyDefinitionAuthoring> {
        self.definitions
            .iter()
            .find(|definition| definition.property_id.as_ref() == property_id)
    }

    pub fn applied_control(
        &self,
        control_id: &str,
    ) -> Option<&DesignAppliedComponentPropertyControl> {
        self.applied_properties
            .iter()
            .find(|control| control.control_id.as_ref() == control_id)
    }

    pub fn exposure_candidate(
        &self,
        candidate_id: &str,
    ) -> Option<&DesignNestedComponentPropertyExposureCandidate> {
        self.exposure_candidates
            .iter()
            .find(|candidate| candidate.candidate_id.as_ref() == candidate_id)
    }

    /// Validates Figma's leading-Variant definition partition without
    /// interpreting any host identifier.
    pub fn preserves_variant_partition(&self, properties: &[DesignComponentProperty]) -> bool {
        let mut reached_regular = false;
        for property in properties {
            match DesignComponentPropertyPartition::for_kind(property.definition.kind()) {
                DesignComponentPropertyPartition::Variant if reached_regular => return false,
                DesignComponentPropertyPartition::Variant => {}
                DesignComponentPropertyPartition::Regular => reached_regular = true,
            }
        }
        true
    }
}

/// Documentation attached to a component, property, or library reference.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DesignDocumentationLink {
    pub label: SharedString,
    pub url: SharedString,
}

impl DesignDocumentationLink {
    pub fn new(label: impl Into<SharedString>, url: impl Into<SharedString>) -> Self {
        Self {
            label: label.into(),
            url: url.into(),
        }
    }
}

/// Where a component reference is defined.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DesignComponentOrigin {
    Local,
    Remote { library_name: SharedString },
}

impl DesignComponentOrigin {
    pub const fn label(&self) -> &'static str {
        match self {
            Self::Local => "Local component",
            Self::Remote { .. } => "Library component",
        }
    }
}

/// Whether a referenced main component can currently be resolved.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DesignComponentAvailability {
    Available,
    Missing,
    Unavailable { reason: SharedString },
}

impl DesignComponentAvailability {
    pub const fn is_available(&self) -> bool {
        matches!(self, Self::Available)
    }

    pub const fn label(&self) -> &'static str {
        match self {
            Self::Available => "Available",
            Self::Missing => "Missing",
            Self::Unavailable { .. } => "Unavailable",
        }
    }
}

/// Stable identity used by main-component, instance-swap, and slot controls.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DesignComponentReference {
    pub id: SharedString,
    pub name: SharedString,
    pub origin: DesignComponentOrigin,
    pub availability: DesignComponentAvailability,
    pub description: Option<SharedString>,
    pub documentation_links: Vec<DesignDocumentationLink>,
}

impl DesignComponentReference {
    pub fn local(id: impl Into<SharedString>, name: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            origin: DesignComponentOrigin::Local,
            availability: DesignComponentAvailability::Available,
            description: None,
            documentation_links: Vec::new(),
        }
    }

    pub fn remote(
        id: impl Into<SharedString>,
        name: impl Into<SharedString>,
        library_name: impl Into<SharedString>,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            origin: DesignComponentOrigin::Remote {
                library_name: library_name.into(),
            },
            availability: DesignComponentAvailability::Available,
            description: None,
            documentation_links: Vec::new(),
        }
    }

    pub fn with_availability(mut self, availability: DesignComponentAvailability) -> Self {
        self.availability = availability;
        self
    }
}

/// Exact Figma asset discriminant retained by an instance-swap browser row.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DesignComponentAssetKind {
    Component,
    ComponentSet,
}

impl DesignComponentAssetKind {
    pub const fn api_name(self) -> &'static str {
        match self {
            Self::Component => "COMPONENT",
            Self::ComponentSet => "COMPONENT_SET",
        }
    }
}

/// Stable page or library origin for a component browser result.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum DesignComponentSource {
    Page {
        page_id: SharedString,
        page_name: SharedString,
    },
    Library {
        library_id: SharedString,
        library_name: SharedString,
    },
}

impl DesignComponentSource {
    pub fn page(page_id: impl Into<SharedString>, page_name: impl Into<SharedString>) -> Self {
        Self::Page {
            page_id: page_id.into(),
            page_name: page_name.into(),
        }
    }

    pub fn library(
        library_id: impl Into<SharedString>,
        library_name: impl Into<SharedString>,
    ) -> Self {
        Self::Library {
            library_id: library_id.into(),
            library_name: library_name.into(),
        }
    }

    pub const fn label(&self) -> &SharedString {
        match self {
            Self::Page { page_name, .. } => page_name,
            Self::Library { library_name, .. } => library_name,
        }
    }
}

/// Whether a browser result is already usable or still needs importing.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DesignComponentImportState {
    Local,
    Imported,
    Available,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DesignComponentSearchMetadata {
    pub path: Vec<SharedString>,
    pub description: Option<SharedString>,
    pub keywords: Vec<SharedString>,
}

impl DesignComponentSearchMetadata {
    pub fn new(
        path: impl IntoIterator<Item = impl Into<SharedString>>,
        keywords: impl IntoIterator<Item = impl Into<SharedString>>,
    ) -> Self {
        Self {
            path: path.into_iter().map(Into::into).collect(),
            description: None,
            keywords: keywords.into_iter().map(Into::into).collect(),
        }
    }

    pub fn described(mut self, description: impl Into<SharedString>) -> Self {
        self.description = Some(description.into());
        self
    }
}

/// Stable selection returned by instance-swap browser intents.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct DesignComponentSwapSelection {
    pub component_id: SharedString,
    pub component_key: SharedString,
    pub asset_kind: DesignComponentAssetKind,
    pub source: DesignComponentSource,
}

/// One lossless local or library result in the all-components swap browser.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DesignComponentSwapCandidate {
    pub reference: DesignComponentReference,
    /// Figma library identity used by `importComponentByKeyAsync`.
    pub component_key: SharedString,
    pub asset_kind: DesignComponentAssetKind,
    pub source: DesignComponentSource,
    pub import_state: DesignComponentImportState,
    pub search: DesignComponentSearchMetadata,
    pub disabled_reason: Option<SharedString>,
}

impl DesignComponentSwapCandidate {
    pub fn local(
        reference: DesignComponentReference,
        component_key: impl Into<SharedString>,
        page_id: impl Into<SharedString>,
        page_name: impl Into<SharedString>,
    ) -> Self {
        Self {
            reference,
            component_key: component_key.into(),
            asset_kind: DesignComponentAssetKind::Component,
            source: DesignComponentSource::page(page_id, page_name),
            import_state: DesignComponentImportState::Local,
            search: DesignComponentSearchMetadata::default(),
            disabled_reason: None,
        }
    }

    pub fn library(
        reference: DesignComponentReference,
        component_key: impl Into<SharedString>,
        library_id: impl Into<SharedString>,
        library_name: impl Into<SharedString>,
    ) -> Self {
        Self {
            reference,
            component_key: component_key.into(),
            asset_kind: DesignComponentAssetKind::Component,
            source: DesignComponentSource::library(library_id, library_name),
            import_state: DesignComponentImportState::Available,
            search: DesignComponentSearchMetadata::default(),
            disabled_reason: None,
        }
    }

    pub const fn with_asset_kind(mut self, asset_kind: DesignComponentAssetKind) -> Self {
        self.asset_kind = asset_kind;
        self
    }

    pub const fn with_import_state(mut self, import_state: DesignComponentImportState) -> Self {
        self.import_state = import_state;
        self
    }

    pub fn with_search(mut self, search: DesignComponentSearchMetadata) -> Self {
        self.search = search;
        self
    }

    pub fn disabled(mut self, reason: impl Into<SharedString>) -> Self {
        self.disabled_reason = Some(reason.into());
        self
    }

    pub fn selection(&self) -> DesignComponentSwapSelection {
        DesignComponentSwapSelection {
            component_id: self.reference.id.clone(),
            component_key: self.component_key.clone(),
            asset_kind: self.asset_kind,
            source: self.source.clone(),
        }
    }

    pub fn can_apply(&self) -> bool {
        self.import_state != DesignComponentImportState::Available
            && self.reference.availability.is_available()
            && self.disabled_reason.is_none()
    }

    pub fn can_import(&self) -> bool {
        self.import_state == DesignComponentImportState::Available
            && self.reference.availability.is_available()
            && self.disabled_reason.is_none()
    }

    pub fn matches_search(&self, query: &str) -> bool {
        let tokens = query
            .split_whitespace()
            .map(str::to_ascii_lowercase)
            .collect::<Vec<_>>();
        if tokens.is_empty() {
            return true;
        }
        let mut haystack = format!(
            "{} {} {} {} {}",
            self.reference.id,
            self.reference.name,
            self.component_key,
            self.source.label(),
            self.asset_kind.api_name(),
        )
        .to_ascii_lowercase();
        for segment in &self.search.path {
            haystack.push(' ');
            haystack.push_str(&segment.to_ascii_lowercase());
        }
        if let Some(description) = &self.search.description {
            haystack.push(' ');
            haystack.push_str(&description.to_ascii_lowercase());
        }
        for keyword in &self.search.keywords {
            haystack.push(' ');
            haystack.push_str(&keyword.to_ascii_lowercase());
        }
        tokens.iter().all(|token| haystack.contains(token))
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DesignComponentSwapViewData {
    pub candidates: Vec<DesignComponentSwapCandidate>,
}

impl DesignComponentSwapViewData {
    pub fn new(candidates: impl IntoIterator<Item = DesignComponentSwapCandidate>) -> Self {
        Self {
            candidates: candidates.into_iter().collect(),
        }
    }

    pub fn candidate(
        &self,
        selection: &DesignComponentSwapSelection,
    ) -> Option<&DesignComponentSwapCandidate> {
        self.candidates.iter().find(|candidate| {
            candidate.reference.id == selection.component_id
                && candidate.component_key == selection.component_key
                && candidate.asset_kind == selection.asset_kind
                && candidate.source == selection.source
        })
    }

    pub fn matching<'a>(
        &'a self,
        query: &'a str,
    ) -> impl Iterator<Item = &'a DesignComponentSwapCandidate> + 'a {
        self.candidates
            .iter()
            .filter(move |candidate| candidate.matches_search(query))
    }
}

/// The selected object's component-system role.
///
/// This is deliberately independent of [`DesignPanelNodeKind`]: a host may
/// inspect a variant child or a slot instance while mapping both onto its own
/// frame-like node kind.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DesignComponentRole {
    StandaloneMain,
    VariantChild,
    ComponentSet,
    Instance,
    SlotDefinition,
    SlotInstance,
}

impl DesignComponentRole {
    pub const fn label(self) -> &'static str {
        match self {
            Self::StandaloneMain => "Main component",
            Self::VariantChild => "Variant",
            Self::ComponentSet => "Component set",
            Self::Instance => "Instance",
            Self::SlotDefinition => "Slot definition",
            Self::SlotInstance => "Slot instance",
        }
    }

    pub const fn uses_instance_section(self) -> bool {
        matches!(self, Self::Instance | Self::SlotInstance)
    }

    pub const fn can_edit_property_value(self) -> bool {
        !matches!(self, Self::SlotDefinition)
    }

    pub const fn can_configure_slot(self) -> bool {
        matches!(self, Self::SlotDefinition)
    }

    pub const fn can_reset_instance_overrides(self) -> bool {
        matches!(self, Self::Instance | Self::SlotInstance)
    }

    pub const fn can_detach_instance(self) -> bool {
        matches!(self, Self::Instance)
    }

    pub const fn can_modify_slot_instances(self) -> bool {
        matches!(self, Self::Instance | Self::SlotInstance)
    }

    pub const fn can_author_component_properties(self) -> bool {
        matches!(self, Self::StandaloneMain | Self::ComponentSet)
    }
}

/// Host-controlled reset availability for an override or slot value.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DesignComponentResetState {
    NotApplicable,
    Clean,
    Resettable,
    Unavailable { reason: SharedString },
}

impl DesignComponentResetState {
    pub const fn can_reset(&self) -> bool {
        matches!(self, Self::Resettable)
    }

    pub const fn is_overridden(&self) -> bool {
        matches!(self, Self::Resettable | Self::Unavailable { .. })
    }
}

/// Aggregate instance-override information shown above its properties.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DesignComponentOverrideSummary {
    pub overridden_property_count: u32,
    pub nested_override_count: u32,
    pub reset_state: DesignComponentResetState,
}

impl Default for DesignComponentOverrideSummary {
    fn default() -> Self {
        Self {
            overridden_property_count: 0,
            nested_override_count: 0,
            reset_state: DesignComponentResetState::Clean,
        }
    }
}

/// Component-level context shared by the property rows.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DesignComponentContext {
    pub role: DesignComponentRole,
    pub main_component: Option<DesignComponentReference>,
    pub description: Option<SharedString>,
    pub documentation_links: Vec<DesignDocumentationLink>,
    pub overrides: DesignComponentOverrideSummary,
    /// Present only while the host exposes component-definition authoring for
    /// this exact selection.
    pub authoring: Option<DesignComponentAuthoringViewData>,
}

impl DesignComponentContext {
    pub fn new(role: DesignComponentRole) -> Self {
        Self {
            role,
            main_component: None,
            description: None,
            documentation_links: Vec::new(),
            overrides: DesignComponentOverrideSummary::default(),
            authoring: None,
        }
    }

    pub fn with_main_component(mut self, main_component: DesignComponentReference) -> Self {
        self.main_component = Some(main_component);
        self
    }

    pub fn with_authoring(mut self, authoring: DesignComponentAuthoringViewData) -> Self {
        self.authoring = Some(authoring);
        self
    }
}

/// Per-child actions that a host allows from an instance's Slot surface.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DesignSlotChildCapabilities {
    pub select: bool,
    pub remove: bool,
    pub reorder: bool,
    pub replace_instance: bool,
}

impl DesignSlotChildCapabilities {
    pub const EDITABLE_LAYER: Self = Self {
        select: true,
        remove: true,
        reorder: true,
        replace_instance: false,
    };

    pub const EDITABLE_INSTANCE: Self = Self {
        replace_instance: true,
        ..Self::EDITABLE_LAYER
    };

    pub const READ_ONLY: Self = Self {
        select: true,
        remove: false,
        reorder: false,
        replace_instance: false,
    };
}

/// One arbitrary scene-node child contained by a Slot.
///
/// Slots accept instances, text, images, vectors, frames, and other layers.
/// `main_component` is populated only for instance children and is never used
/// as the stable identity of the inserted layer.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DesignSlotChild {
    pub node_id: SharedString,
    pub name: SharedString,
    pub kind: DesignPanelNodeKind,
    pub main_component: Option<DesignComponentReference>,
    pub capabilities: DesignSlotChildCapabilities,
}

impl DesignSlotChild {
    pub fn layer(
        node_id: impl Into<SharedString>,
        name: impl Into<SharedString>,
        kind: DesignPanelNodeKind,
    ) -> Self {
        Self {
            node_id: node_id.into(),
            name: name.into(),
            kind,
            main_component: None,
            capabilities: DesignSlotChildCapabilities::EDITABLE_LAYER,
        }
    }

    pub fn instance(node_id: impl Into<SharedString>, name: impl Into<SharedString>) -> Self {
        Self {
            capabilities: DesignSlotChildCapabilities::EDITABLE_INSTANCE,
            ..Self::layer(node_id, name, DesignPanelNodeKind::Instance)
        }
    }

    /// Compatibility constructor for the original instance-only Slot model.
    #[deprecated(note = "use DesignSlotChild::instance or DesignSlotChild::layer")]
    pub fn new(node_id: impl Into<SharedString>, name: impl Into<SharedString>) -> Self {
        Self::instance(node_id, name)
    }

    pub const fn is_instance(&self) -> bool {
        matches!(self.kind, DesignPanelNodeKind::Instance)
    }
}

/// Compatibility alias for hosts migrating from the instance-only Slot model.
#[deprecated(note = "Slots accept arbitrary layers; use DesignSlotChild")]
pub type DesignSlotInstance = DesignSlotChild;

/// Ordered, host-controlled value of a slot component property.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DesignSlotValue {
    pub children: Vec<DesignSlotChild>,
}

/// Settings authored on a slot definition.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DesignSlotSettings {
    /// Native `stretchChildOnInsert`: inserted items fill the Slot's
    /// counter-axis by default.
    pub stretch_child_on_insert: bool,
    /// Native `displayEmptyByDefault`.
    pub display_empty: bool,
    /// Native `minChildren`; `None` preserves Figma's explicit `null`.
    pub minimum_children: Option<u32>,
    /// Native `maxChildren`; `None` preserves Figma's explicit `null`.
    pub maximum_children: Option<u32>,
    pub preferred_values_only: bool,
    pub preferred_values: Vec<DesignComponentReference>,
}

impl Default for DesignSlotSettings {
    fn default() -> Self {
        Self {
            stretch_child_on_insert: true,
            display_empty: true,
            minimum_children: None,
            maximum_children: None,
            preferred_values_only: false,
            preferred_values: Vec::new(),
        }
    }
}

impl DesignSlotSettings {
    pub const fn has_valid_child_range(&self) -> bool {
        match (self.minimum_children, self.maximum_children) {
            (Some(minimum), Some(maximum)) => minimum <= maximum,
            _ => true,
        }
    }
}

/// A reason the current slot contents do not satisfy their definition.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DesignSlotViolation {
    BelowMinimum {
        minimum: u32,
        actual: u32,
    },
    AboveMaximum {
        maximum: u32,
        actual: u32,
    },
    NonPreferredValue {
        instance_id: SharedString,
        component_name: SharedString,
    },
    NonPreferredChild {
        child_id: SharedString,
        child_name: SharedString,
    },
    MissingMainComponent {
        instance_id: SharedString,
    },
}

impl DesignSlotViolation {
    pub fn label(&self) -> SharedString {
        match self {
            Self::BelowMinimum { minimum, actual } => {
                format!("Needs at least {minimum} layers; contains {actual}").into()
            }
            Self::AboveMaximum { maximum, actual } => {
                format!("Recommended maximum is {maximum} layers; currently {actual}").into()
            }
            Self::NonPreferredValue { component_name, .. } => {
                format!("{component_name} is not a preferred value").into()
            }
            Self::NonPreferredChild { child_name, .. } => {
                format!("{child_name} is not a preferred instance").into()
            }
            Self::MissingMainComponent { .. } => {
                "An inserted instance has no main component".into()
            }
        }
    }
}

/// Validation and reset state supplied for a slot instance.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DesignSlotState {
    pub violations: Vec<DesignSlotViolation>,
    pub reset_state: DesignComponentResetState,
}

impl Default for DesignSlotState {
    fn default() -> Self {
        Self {
            violations: Vec::new(),
            reset_state: DesignComponentResetState::Clean,
        }
    }
}

/// Which exact Figma component-property field receives a variable alias.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DesignComponentPropertyVariableField {
    DefinitionDefaultValue,
    InstanceValue,
}

impl DesignComponentPropertyVariableField {
    pub const fn api_name(self) -> &'static str {
        match self {
            Self::DefinitionDefaultValue => "defaultValue",
            Self::InstanceValue => "value",
        }
    }
}

/// Lossless bind target carried by component-property variable intents.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DesignComponentPropertyVariableTarget {
    pub property_id: SharedString,
    pub property_name: SharedString,
    pub property_kind: DesignComponentPropertyKind,
    pub field: DesignComponentPropertyVariableField,
    pub resolved_type: DesignVariableResolvedType,
}

/// Current variable alias retained even when it is absent from the browser.
#[derive(Clone, Debug, PartialEq)]
pub struct DesignComponentPropertyVariableBinding {
    pub variable_id: SharedString,
    pub variable_name: SharedString,
    pub resolved_value: DesignVariableResolvedValue,
    pub can_detach: bool,
}

impl DesignComponentPropertyVariableBinding {
    pub fn new(
        variable_id: impl Into<SharedString>,
        variable_name: impl Into<SharedString>,
        resolved_value: DesignVariableResolvedValue,
    ) -> Self {
        Self {
            variable_id: variable_id.into(),
            variable_name: variable_name.into(),
            resolved_value,
            can_detach: true,
        }
    }

    pub const fn detachable(mut self, can_detach: bool) -> Self {
        self.can_detach = can_detach;
        self
    }
}

/// Whether a row is authored on the selected component or surfaced from one
/// exact nested instance.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DesignComponentPropertyOrigin {
    SelectedNode,
    NestedInstance {
        instance_id: SharedString,
        instance_name: SharedString,
        main_component: Option<DesignComponentReference>,
    },
}

impl DesignComponentPropertyOrigin {
    pub const fn nested_instance_id(&self) -> Option<&SharedString> {
        match self {
            Self::SelectedNode => None,
            Self::NestedInstance { instance_id, .. } => Some(instance_id),
        }
    }
}

/// Type-specific definition of a component property.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DesignComponentPropertyDefinition {
    Boolean {
        default_value: bool,
    },
    Text {
        default_value: SharedString,
        multiline: bool,
    },
    InstanceSwap {
        default_value: Option<DesignComponentReference>,
        preferred_values: Vec<DesignComponentReference>,
    },
    Variant {
        default_value: SharedString,
        options: Vec<SharedString>,
    },
    Slot {
        default_value: DesignSlotValue,
        settings: DesignSlotSettings,
    },
}

impl DesignComponentPropertyDefinition {
    pub const fn kind(&self) -> DesignComponentPropertyKind {
        match self {
            Self::Boolean { .. } => DesignComponentPropertyKind::Boolean,
            Self::Text { .. } => DesignComponentPropertyKind::Text,
            Self::InstanceSwap { .. } => DesignComponentPropertyKind::InstanceSwap,
            Self::Variant { .. } => DesignComponentPropertyKind::Variant,
            Self::Slot { .. } => DesignComponentPropertyKind::Slot,
        }
    }

    pub fn option_labels(&self) -> Vec<SharedString> {
        match self {
            Self::Boolean { .. } => vec!["True".into(), "False".into()],
            Self::Variant { options, .. } => options.clone(),
            Self::InstanceSwap {
                preferred_values, ..
            } => preferred_values
                .iter()
                .filter(|reference| reference.availability.is_available())
                .map(|reference| reference.name.clone())
                .collect(),
            Self::Text { .. } | Self::Slot { .. } => Vec::new(),
        }
    }

    pub fn default_value(&self) -> DesignComponentPropertyValue {
        match self {
            Self::Boolean { default_value } => {
                DesignComponentPropertyValue::Boolean(*default_value)
            }
            Self::Text { default_value, .. } => {
                DesignComponentPropertyValue::Text(default_value.clone())
            }
            Self::InstanceSwap { default_value, .. } => {
                DesignComponentPropertyValue::InstanceSwap(default_value.clone())
            }
            Self::Variant { default_value, .. } => {
                DesignComponentPropertyValue::Variant(default_value.clone())
            }
            Self::Slot { default_value, .. } => {
                DesignComponentPropertyValue::Slot(default_value.clone())
            }
        }
    }
}

/// Type-safe current value of a component property.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DesignComponentPropertyValue {
    Boolean(bool),
    Text(SharedString),
    InstanceSwap(Option<DesignComponentReference>),
    Variant(SharedString),
    Slot(DesignSlotValue),
}

impl DesignComponentPropertyValue {
    pub const fn kind(&self) -> DesignComponentPropertyKind {
        match self {
            Self::Boolean(_) => DesignComponentPropertyKind::Boolean,
            Self::Text(_) => DesignComponentPropertyKind::Text,
            Self::InstanceSwap(_) => DesignComponentPropertyKind::InstanceSwap,
            Self::Variant(_) => DesignComponentPropertyKind::Variant,
            Self::Slot(_) => DesignComponentPropertyKind::Slot,
        }
    }

    pub fn display_value(&self) -> SharedString {
        match self {
            Self::Boolean(value) => {
                if *value {
                    "True".into()
                } else {
                    "False".into()
                }
            }
            Self::Text(value) | Self::Variant(value) => value.clone(),
            Self::InstanceSwap(Some(reference)) => reference.name.clone(),
            Self::InstanceSwap(None) => "None".into(),
            Self::Slot(value) => {
                let count = value.children.len();
                format!("{count} layer{}", if count == 1 { "" } else { "s" }).into()
            }
        }
    }
}

/// Whether an individual component property differs from its main value.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DesignComponentPropertyOverrideState {
    Default,
    Overridden,
    Mixed,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DesignComponentProperty {
    /// Stable property identity. Hosts should not use the display name as a key.
    pub id: SharedString,
    pub name: SharedString,
    pub description: Option<SharedString>,
    pub documentation_links: Vec<DesignDocumentationLink>,
    pub definition: DesignComponentPropertyDefinition,
    pub resolved_value: DesignComponentPropertyValue,
    /// Alias on `componentPropertyDefinitions[property].defaultValue`.
    pub default_value_binding: Option<DesignComponentPropertyVariableBinding>,
    /// Alias on `componentProperties[property].value`.
    pub resolved_value_binding: Option<DesignComponentPropertyVariableBinding>,
    pub origin: DesignComponentPropertyOrigin,
    pub override_state: DesignComponentPropertyOverrideState,
    pub reset_state: DesignComponentResetState,
    pub slot_state: Option<DesignSlotState>,
    /// Compatibility display projection used by early story adapters.
    pub value: SharedString,
    /// Compatibility projection of [`Self::definition`].
    pub kind: DesignComponentPropertyKind,
    /// Compatibility display projection of typed variant/instance options.
    pub preferred_values: Vec<SharedString>,
}

impl DesignComponentProperty {
    fn new(
        id: impl Into<SharedString>,
        name: impl Into<SharedString>,
        definition: DesignComponentPropertyDefinition,
        resolved_value: DesignComponentPropertyValue,
    ) -> Self {
        debug_assert_eq!(definition.kind(), resolved_value.kind());
        let kind = definition.kind();
        let preferred_values = definition.option_labels();
        let value = resolved_value.display_value();
        Self {
            id: id.into(),
            name: name.into(),
            description: None,
            documentation_links: Vec::new(),
            definition,
            resolved_value,
            default_value_binding: None,
            resolved_value_binding: None,
            origin: DesignComponentPropertyOrigin::SelectedNode,
            override_state: DesignComponentPropertyOverrideState::Default,
            reset_state: DesignComponentResetState::NotApplicable,
            slot_state: None,
            value,
            kind,
            preferred_values,
        }
    }

    pub fn boolean(
        id: impl Into<SharedString>,
        name: impl Into<SharedString>,
        default_value: bool,
        value: bool,
    ) -> Self {
        Self::new(
            id,
            name,
            DesignComponentPropertyDefinition::Boolean { default_value },
            DesignComponentPropertyValue::Boolean(value),
        )
    }

    pub fn text(
        id: impl Into<SharedString>,
        name: impl Into<SharedString>,
        default_value: impl Into<SharedString>,
        value: impl Into<SharedString>,
    ) -> Self {
        Self::new(
            id,
            name,
            DesignComponentPropertyDefinition::Text {
                default_value: default_value.into(),
                multiline: false,
            },
            DesignComponentPropertyValue::Text(value.into()),
        )
    }

    pub fn variant(
        id: impl Into<SharedString>,
        name: impl Into<SharedString>,
        default_value: impl Into<SharedString>,
        value: impl Into<SharedString>,
        options: Vec<SharedString>,
    ) -> Self {
        Self::new(
            id,
            name,
            DesignComponentPropertyDefinition::Variant {
                default_value: default_value.into(),
                options,
            },
            DesignComponentPropertyValue::Variant(value.into()),
        )
    }

    pub fn instance_swap(
        id: impl Into<SharedString>,
        name: impl Into<SharedString>,
        default_value: Option<DesignComponentReference>,
        value: Option<DesignComponentReference>,
        preferred_values: Vec<DesignComponentReference>,
    ) -> Self {
        Self::new(
            id,
            name,
            DesignComponentPropertyDefinition::InstanceSwap {
                default_value,
                preferred_values,
            },
            DesignComponentPropertyValue::InstanceSwap(value),
        )
    }

    pub fn slot(
        id: impl Into<SharedString>,
        name: impl Into<SharedString>,
        default_value: DesignSlotValue,
        value: DesignSlotValue,
        settings: DesignSlotSettings,
        state: DesignSlotState,
    ) -> Self {
        let mut property = Self::new(
            id,
            name,
            DesignComponentPropertyDefinition::Slot {
                default_value,
                settings,
            },
            DesignComponentPropertyValue::Slot(value),
        );
        property.slot_state = Some(state);
        property
    }

    pub fn with_description(mut self, description: impl Into<SharedString>) -> Self {
        if matches!(
            self.definition,
            DesignComponentPropertyDefinition::Slot { .. }
        ) {
            self.description = Some(description.into());
        }
        self
    }

    pub fn with_multiline(mut self, multiline: bool) -> Self {
        if let DesignComponentPropertyDefinition::Text {
            multiline: current, ..
        } = &mut self.definition
        {
            *current = multiline;
        }
        self
    }

    pub fn with_default_value_binding(
        mut self,
        binding: DesignComponentPropertyVariableBinding,
    ) -> Self {
        self.default_value_binding = Some(binding);
        self
    }

    pub fn with_resolved_value_binding(
        mut self,
        binding: DesignComponentPropertyVariableBinding,
    ) -> Self {
        self.resolved_value_binding = Some(binding);
        self
    }

    pub fn from_nested_instance(
        mut self,
        instance_id: impl Into<SharedString>,
        instance_name: impl Into<SharedString>,
        main_component: Option<DesignComponentReference>,
    ) -> Self {
        self.origin = DesignComponentPropertyOrigin::NestedInstance {
            instance_id: instance_id.into(),
            instance_name: instance_name.into(),
            main_component,
        };
        self
    }

    /// Returns the exact bindable API field for this row in the selected
    /// component-system role. Definition defaults and instance values have
    /// intentionally different support matrices.
    pub fn variable_target(
        &self,
        role: DesignComponentRole,
    ) -> Option<DesignComponentPropertyVariableTarget> {
        let field = match role {
            DesignComponentRole::StandaloneMain
            | DesignComponentRole::VariantChild
            | DesignComponentRole::ComponentSet => {
                DesignComponentPropertyVariableField::DefinitionDefaultValue
            }
            DesignComponentRole::Instance | DesignComponentRole::SlotInstance => {
                DesignComponentPropertyVariableField::InstanceValue
            }
            DesignComponentRole::SlotDefinition => return None,
        };
        let resolved_type = match (field, self.definition.kind()) {
            (_, DesignComponentPropertyKind::Boolean) => DesignVariableResolvedType::Boolean,
            (
                DesignComponentPropertyVariableField::DefinitionDefaultValue,
                DesignComponentPropertyKind::Text | DesignComponentPropertyKind::InstanceSwap,
            )
            | (
                DesignComponentPropertyVariableField::InstanceValue,
                DesignComponentPropertyKind::Text
                | DesignComponentPropertyKind::InstanceSwap
                | DesignComponentPropertyKind::Variant,
            ) => DesignVariableResolvedType::String,
            (
                DesignComponentPropertyVariableField::DefinitionDefaultValue,
                DesignComponentPropertyKind::Variant | DesignComponentPropertyKind::Slot,
            )
            | (
                DesignComponentPropertyVariableField::InstanceValue,
                DesignComponentPropertyKind::Slot,
            ) => return None,
        };
        Some(DesignComponentPropertyVariableTarget {
            property_id: self.id.clone(),
            property_name: self.name.clone(),
            property_kind: self.definition.kind(),
            field,
            resolved_type,
        })
    }

    pub const fn variable_binding(
        &self,
        field: DesignComponentPropertyVariableField,
    ) -> Option<&DesignComponentPropertyVariableBinding> {
        match field {
            DesignComponentPropertyVariableField::DefinitionDefaultValue => {
                self.default_value_binding.as_ref()
            }
            DesignComponentPropertyVariableField::InstanceValue => {
                self.resolved_value_binding.as_ref()
            }
        }
    }

    pub fn active_variable_binding(
        &self,
        role: DesignComponentRole,
    ) -> Option<&DesignComponentPropertyVariableBinding> {
        self.variable_target(role)
            .and_then(|target| self.variable_binding(target.field))
    }

    pub fn with_reset_state(mut self, reset_state: DesignComponentResetState) -> Self {
        self.override_state = if reset_state.is_overridden() {
            DesignComponentPropertyOverrideState::Overridden
        } else {
            DesignComponentPropertyOverrideState::Default
        };
        self.reset_state = reset_state;
        self
    }

    /// Reads typed view data while accepting the original story adapter's
    /// direct mutation of the compatibility `value` projection.
    pub fn effective_value(&self) -> DesignComponentPropertyValue {
        if self.value == self.resolved_value.display_value() {
            return self.resolved_value.clone();
        }
        self.value_from_display(self.value.clone())
            .unwrap_or_else(|| self.resolved_value.clone())
    }

    pub fn value_from_display(
        &self,
        value: impl Into<SharedString>,
    ) -> Option<DesignComponentPropertyValue> {
        let value = value.into();
        match &self.definition {
            DesignComponentPropertyDefinition::Boolean { .. } => {
                if value.eq_ignore_ascii_case("true") {
                    Some(DesignComponentPropertyValue::Boolean(true))
                } else if value.eq_ignore_ascii_case("false") {
                    Some(DesignComponentPropertyValue::Boolean(false))
                } else {
                    None
                }
            }
            DesignComponentPropertyDefinition::Text { .. } => {
                Some(DesignComponentPropertyValue::Text(value))
            }
            DesignComponentPropertyDefinition::Variant { options, .. } => options
                .contains(&value)
                .then_some(DesignComponentPropertyValue::Variant(value)),
            DesignComponentPropertyDefinition::InstanceSwap {
                preferred_values, ..
            } => {
                if value.eq_ignore_ascii_case("none") || value.is_empty() {
                    return Some(DesignComponentPropertyValue::InstanceSwap(None));
                }
                preferred_values
                    .iter()
                    .find(|reference| reference.id == value || reference.name == value)
                    .cloned()
                    .map(Some)
                    .map(DesignComponentPropertyValue::InstanceSwap)
            }
            DesignComponentPropertyDefinition::Slot { .. } => None,
        }
    }

    pub fn slot_settings(&self) -> Option<&DesignSlotSettings> {
        match &self.definition {
            DesignComponentPropertyDefinition::Slot { settings, .. } => Some(settings),
            _ => None,
        }
    }

    /// Accepts a host-applied typed value and synchronizes compatibility
    /// projections. Returns false instead of coercing a mismatched kind.
    pub fn set_resolved_value(&mut self, value: DesignComponentPropertyValue) -> bool {
        if value.kind() != self.definition.kind() {
            return false;
        }
        self.value = value.display_value();
        self.resolved_value = value;
        self.kind = self.definition.kind();
        self.preferred_values = self.definition.option_labels();
        true
    }

    /// Restores the typed authored default and clears property-level override
    /// state. The surrounding host still owns undo and document mutation.
    pub fn reset_to_default(&mut self) {
        let default_value = self.definition.default_value();
        let _ = self.set_resolved_value(default_value);
        self.override_state = DesignComponentPropertyOverrideState::Default;
        self.reset_state = DesignComponentResetState::Clean;
        if let Some(slot_state) = self.slot_state.as_mut() {
            slot_state.reset_state = DesignComponentResetState::Clean;
        }
        self.refresh_slot_violations();
    }

    pub fn mark_overridden(&mut self) {
        self.override_state = DesignComponentPropertyOverrideState::Overridden;
        self.reset_state = DesignComponentResetState::Resettable;
        if let Some(slot_state) = self.slot_state.as_mut() {
            slot_state.reset_state = DesignComponentResetState::Resettable;
        }
    }

    /// Applies a host-accepted settings intent to this view-data adapter.
    ///
    /// The inspector itself never calls this method; it is provided for mock
    /// hosts and adapters that want one canonical projection update path.
    pub fn apply_slot_settings_change(&mut self, change: &DesignSlotSettingsChange) -> bool {
        let DesignComponentPropertyDefinition::Slot { settings, .. } = &mut self.definition else {
            return false;
        };
        match change {
            DesignSlotSettingsChange::StretchChildOnInsert(value) => {
                settings.stretch_child_on_insert = *value;
            }
            DesignSlotSettingsChange::DisplayEmpty(value) => {
                settings.display_empty = *value;
            }
            DesignSlotSettingsChange::MinimumInstances(value) => {
                settings.minimum_children = *value;
            }
            DesignSlotSettingsChange::MaximumInstances(value) => {
                settings.maximum_children = *value;
            }
            DesignSlotSettingsChange::PreferredValuesOnly(value) => {
                settings.preferred_values_only = *value;
            }
            DesignSlotSettingsChange::PreferredValues(values) => {
                settings.preferred_values = values.clone();
            }
            DesignSlotSettingsChange::Replace(replacement) => {
                *settings = replacement.clone();
            }
        }
        self.preferred_values = self.definition.option_labels();
        self.refresh_slot_violations();
        true
    }

    /// Recomputes current slot guidance from the typed value and definition.
    pub fn refresh_slot_violations(&mut self) {
        let DesignComponentPropertyDefinition::Slot { settings, .. } = &self.definition else {
            return;
        };
        let DesignComponentPropertyValue::Slot(value) = &self.resolved_value else {
            return;
        };
        let actual = u32::try_from(value.children.len()).unwrap_or(u32::MAX);
        let mut violations = Vec::new();
        if let Some(minimum) = settings.minimum_children
            && actual < minimum
        {
            violations.push(DesignSlotViolation::BelowMinimum { minimum, actual });
        }
        if settings
            .maximum_children
            .is_some_and(|maximum| actual > maximum)
        {
            violations.push(DesignSlotViolation::AboveMaximum {
                maximum: settings.maximum_children.unwrap_or_default(),
                actual,
            });
        }
        if settings.preferred_values_only {
            for child in &value.children {
                match child.main_component.as_ref() {
                    Some(main)
                        if !settings
                            .preferred_values
                            .iter()
                            .any(|preferred| preferred.id == main.id) =>
                    {
                        violations.push(DesignSlotViolation::NonPreferredValue {
                            instance_id: child.node_id.clone(),
                            component_name: main.name.clone(),
                        });
                    }
                    None if child.is_instance() => {
                        violations.push(DesignSlotViolation::MissingMainComponent {
                            instance_id: child.node_id.clone(),
                        });
                    }
                    Some(_) => {}
                    None => violations.push(DesignSlotViolation::NonPreferredChild {
                        child_id: child.node_id.clone(),
                        child_name: child.name.clone(),
                    }),
                }
            }
        }
        self.slot_state
            .get_or_insert_with(DesignSlotState::default)
            .violations = violations;
    }
}

/// One typed mutation to slot-definition settings.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DesignSlotSettingsChange {
    StretchChildOnInsert(bool),
    DisplayEmpty(bool),
    MinimumInstances(Option<u32>),
    MaximumInstances(Option<u32>),
    PreferredValuesOnly(bool),
    PreferredValues(Vec<DesignComponentReference>),
    /// Atomic replacement used by Figma's Edit Slot property modal.
    Replace(DesignSlotSettings),
}
