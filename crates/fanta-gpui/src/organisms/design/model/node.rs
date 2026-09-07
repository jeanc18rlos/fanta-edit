use gpui::SharedString;

use super::*;

/// A Figma Design-panel selection kind.
///
/// These variants model inspector capabilities, not a Fanta document schema.
/// Hosts can map their own node kinds onto the closest visual contract.
///
/// `Image`, `Video`, `Arrow`, `Mask`, `Table`, `Pen`, `Pencil`, and
/// `MultipleSelection` are retained as source-compatible legacy adapters. They
/// are intentionally absent from [`Self::ALL`]: in Figma Design, image/video
/// are paint payloads, arrows are lines with an arrow cap, masks are an
/// orthogonal scene-node flag, Pen/Pencil are creation tools that yield vector
/// nodes, and multiple selection is inspection context rather than a node.
/// Tables belong to FigJam.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum DesignPanelNodeKind {
    Frame,
    Group,
    TransformGroup,
    Section,
    Component,
    ComponentSet,
    Instance,
    Slot,
    Widget,
    Text,
    TextPath,
    Image,
    Video,
    Rectangle,
    Ellipse,
    Polygon,
    Star,
    Line,
    Arrow,
    Vector,
    BooleanOperation,
    Slice,
    Mask,
    Table,
    Pen,
    Pencil,
    MultipleSelection,
    Other,
}

impl DesignPanelNodeKind {
    /// Canonical Figma Design node kinds exposed by node-preset surfaces.
    pub const ALL: [Self; 20] = [
        Self::Frame,
        Self::Group,
        Self::TransformGroup,
        Self::Section,
        Self::Component,
        Self::ComponentSet,
        Self::Instance,
        Self::Slot,
        Self::Text,
        Self::TextPath,
        Self::Rectangle,
        Self::Ellipse,
        Self::Polygon,
        Self::Star,
        Self::Line,
        Self::Vector,
        Self::BooleanOperation,
        Self::Slice,
        Self::Widget,
        Self::Other,
    ];

    /// Complete compatibility set for hosts migrating from the original
    /// inspector taxonomy. New preset UIs should iterate [`Self::ALL`].
    pub const COMPATIBILITY_ALL: [Self; 28] = [
        Self::Frame,
        Self::Group,
        Self::TransformGroup,
        Self::Section,
        Self::Component,
        Self::ComponentSet,
        Self::Instance,
        Self::Slot,
        Self::Text,
        Self::TextPath,
        Self::Image,
        Self::Video,
        Self::Rectangle,
        Self::Ellipse,
        Self::Polygon,
        Self::Star,
        Self::Line,
        Self::Arrow,
        Self::Vector,
        Self::BooleanOperation,
        Self::Slice,
        Self::Mask,
        Self::Table,
        Self::Pen,
        Self::Pencil,
        Self::MultipleSelection,
        Self::Widget,
        Self::Other,
    ];

    /// Whether this is a compatibility-only taxonomy value.
    pub const fn is_compatibility_alias(self) -> bool {
        matches!(
            self,
            Self::Image
                | Self::Video
                | Self::Arrow
                | Self::Mask
                | Self::Table
                | Self::Pen
                | Self::Pencil
                | Self::MultipleSelection
        )
    }

    /// Closest canonical Design-node kind for a legacy adapter value.
    ///
    /// Hosts should retain their real underlying node kind whenever it is
    /// available; this mapping exists for incremental migration only.
    pub const fn canonical_kind(self) -> Self {
        match self {
            Self::Image | Self::Video => Self::Rectangle,
            Self::Arrow => Self::Line,
            Self::Mask | Self::Pen | Self::Pencil => Self::Vector,
            Self::Table => Self::Frame,
            Self::MultipleSelection => Self::Other,
            kind => kind,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Frame => "Frame",
            Self::Group => "Group",
            Self::TransformGroup => "Transform group",
            Self::Section => "Section",
            Self::Component => "Component",
            Self::ComponentSet => "Component set",
            Self::Instance => "Instance",
            Self::Slot => "Slot",
            Self::Widget => "Widget",
            Self::Text => "Text",
            Self::TextPath => "Text path",
            Self::Image => "Image",
            Self::Video => "Video",
            Self::Rectangle => "Rectangle",
            Self::Ellipse => "Ellipse",
            Self::Polygon => "Polygon",
            Self::Star => "Star",
            Self::Line => "Line",
            Self::Arrow => "Arrow",
            Self::Vector => "Vector",
            Self::BooleanOperation => "Boolean operation",
            Self::Slice => "Slice",
            Self::Mask => "Mask",
            Self::Table => "Table",
            Self::Pen => "Pen path",
            Self::Pencil => "Pencil path",
            Self::MultipleSelection => "Multiple selection",
            Self::Other => "Other",
        }
    }

    pub const fn glyph(self) -> &'static str {
        match self {
            Self::Frame => "#",
            Self::Group => "⌗",
            Self::TransformGroup => "⌖",
            Self::Section => "▣",
            Self::Component => "◆",
            Self::ComponentSet => "⁙",
            Self::Instance => "◇",
            Self::Slot => "◈",
            Self::Widget => "W",
            Self::Text => "T",
            Self::TextPath => "T⌁",
            Self::Image => "▧",
            Self::Video => "▶",
            Self::Rectangle => "□",
            Self::Ellipse => "○",
            Self::Polygon => "△",
            Self::Star => "☆",
            Self::Line => "—",
            Self::Arrow => "↗",
            Self::Vector => "⌁",
            Self::BooleanOperation => "∩",
            Self::Slice => "◩",
            Self::Mask => "◐",
            Self::Table => "▦",
            Self::Pen => "⌒",
            Self::Pencil => "∿",
            Self::MultipleSelection => "⁘",
            Self::Other => "•",
        }
    }

    /// Compatibility alias for the universal dimensions surface.
    ///
    /// Use [`Self::supports_auto_layout_container`] when deciding whether to
    /// show direction, padding, gap, and container-alignment controls.
    pub const fn supports_layout(self) -> bool {
        self.supports_dimensions()
    }

    /// Whether the layer exposes width and height in Figma's Layout section.
    ///
    /// This deliberately includes lines, slices, sections, and aggregate
    /// selections. Owning an auto-layout flow is a separate capability.
    pub const fn supports_dimensions(self) -> bool {
        true
    }

    /// Whether the layer exposes Figma's scene-node visibility field.
    ///
    /// Visibility is independent from opacity/blend/effects. In particular,
    /// Section, Slice, and Widget nodes all inherit writable `visible` even
    /// though their other Appearance capabilities differ.
    pub const fn supports_visibility(self) -> bool {
        true
    }

    /// Whether the Position section exposes writable X/Y coordinates.
    pub const fn supports_position_coordinates(self) -> bool {
        true
    }

    /// Whether selection alignment, distribution, and tidy actions apply.
    pub const fn supports_arrange(self) -> bool {
        !matches!(self, Self::Widget)
    }

    /// Whether direct rotation and flip transforms apply.
    pub const fn supports_transforms(self) -> bool {
        !matches!(self, Self::Section | Self::Widget)
    }

    /// Whether the Layout section exposes the aspect-ratio lock.
    pub const fn supports_aspect_ratio_lock(self) -> bool {
        !matches!(self, Self::Widget)
    }

    /// Whether parent auto-layout participation exposes child controls.
    pub const fn supports_auto_layout_child(self) -> bool {
        !matches!(self, Self::Section | Self::Widget)
    }

    /// Whether an eligible selection may request Figma's Add auto layout
    /// structural operation.
    pub const fn supports_add_auto_layout(self) -> bool {
        !matches!(self, Self::Section | Self::Widget)
    }

    /// Whether the layer can own and configure an auto-layout flow.
    ///
    /// Other layer kinds still participate in layout as children and expose
    /// sizing/positioning controls, but they must not inherit the container
    /// direction, padding, gap, or alignment controls.
    pub const fn supports_auto_layout_container(self) -> bool {
        matches!(
            self,
            Self::Frame | Self::Component | Self::ComponentSet | Self::Instance | Self::Slot
        )
    }

    /// Whether Figma accepts Grid as this auto-layout owner's flow.
    ///
    /// Slot nodes support horizontal and vertical auto layout, but the Plugin
    /// API rejects applying Grid to a Slot.
    pub const fn supports_grid_auto_layout(self) -> bool {
        matches!(
            self,
            Self::Frame | Self::Component | Self::ComponentSet | Self::Instance
        )
    }

    pub const fn supports_resize_to_fit(self) -> bool {
        matches!(
            self,
            Self::Frame
                | Self::Group
                | Self::TransformGroup
                | Self::Component
                | Self::ComponentSet
                | Self::Instance
                | Self::Slot
                | Self::Table
        )
    }

    pub const fn supports_clip_content(self) -> bool {
        matches!(
            self,
            Self::Frame | Self::Component | Self::ComponentSet | Self::Instance | Self::Slot
        )
    }

    pub const fn supports_fill(self) -> bool {
        !matches!(
            self,
            Self::Group
                | Self::TransformGroup
                | Self::Slice
                | Self::Line
                | Self::Arrow
                | Self::Widget
                | Self::MultipleSelection
        )
    }

    pub const fn supports_stroke(self) -> bool {
        !matches!(
            self,
            Self::Group
                | Self::TransformGroup
                | Self::Slice
                | Self::Widget
                | Self::MultipleSelection
        )
    }

    /// Whether the layer exposes Figma's opacity, blend-mode, and effects
    /// appearance contract.
    ///
    /// Sections have fills, strokes, and corners, but do not implement
    /// Figma's BlendMixin/EffectsMixin. Slice likewise has no appearance
    /// controls.
    pub const fn supports_layer_appearance(self) -> bool {
        !matches!(self, Self::Section | Self::Slice | Self::Widget)
    }

    /// Whether Figma exposes the container-only `Pass through` blend mode.
    ///
    /// Leaf shapes and text use the 18 compositing modes beginning with
    /// `Normal`; structural containers may preserve child compositing with
    /// `Pass through`.
    pub const fn supports_pass_through_blend(self) -> bool {
        matches!(
            self,
            Self::Frame
                | Self::Group
                | Self::TransformGroup
                | Self::Component
                | Self::ComponentSet
                | Self::Instance
                | Self::Slot
                | Self::Table
        )
    }

    pub const fn supports_effects(self) -> bool {
        self.supports_layer_appearance()
    }

    pub const fn supports_corner_radius(self) -> bool {
        matches!(
            self,
            Self::Frame
                | Self::Section
                | Self::Component
                | Self::ComponentSet
                | Self::Instance
                | Self::Slot
                | Self::Rectangle
                | Self::Image
                | Self::Video
                | Self::Ellipse
                | Self::Polygon
                | Self::Star
                | Self::Vector
                | Self::BooleanOperation
        )
    }

    pub const fn supports_constraints(self) -> bool {
        matches!(
            self,
            Self::Frame
                | Self::Group
                | Self::TransformGroup
                | Self::Component
                | Self::ComponentSet
                | Self::Instance
                | Self::Slot
                | Self::Text
                | Self::TextPath
                | Self::Image
                | Self::Video
                | Self::Rectangle
                | Self::Ellipse
                | Self::Polygon
                | Self::Star
                | Self::Line
                | Self::Arrow
                | Self::Vector
                | Self::Mask
                | Self::Pen
                | Self::Pencil
        )
    }
}

/// Figma mask evaluation mode for a scene node with `isMask = true`.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DesignMaskType {
    Alpha,
    Vector,
    Luminance,
}

impl DesignMaskType {
    pub const ALL: [Self; 3] = [Self::Alpha, Self::Vector, Self::Luminance];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Alpha => "Alpha",
            Self::Vector => "Vector",
            Self::Luminance => "Luminance",
        }
    }

    pub fn parse_compatibility_label(label: &str) -> Option<Self> {
        match label.trim().to_ascii_lowercase().as_str() {
            "alpha" => Some(Self::Alpha),
            "vector" => Some(Self::Vector),
            "luminance" => Some(Self::Luminance),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DesignSectionDevStatusKind {
    ReadyForDev,
    Completed,
}

impl DesignSectionDevStatusKind {
    pub const ALL: [Self; 2] = [Self::ReadyForDev, Self::Completed];

    pub const fn label(self) -> &'static str {
        match self {
            Self::ReadyForDev => "Ready for dev",
            Self::Completed => "Completed",
        }
    }
}

/// Current host-resolved Dev Mode status for a section.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DesignSectionDevStatus {
    pub kind: DesignSectionDevStatusKind,
    pub description: Option<SharedString>,
    /// Figma's UI-only Changed indicator. The plugin API does not currently
    /// expose this bit, so a host can leave it false when unavailable.
    pub changed: bool,
}

impl DesignSectionDevStatus {
    pub const fn new(kind: DesignSectionDevStatusKind) -> Self {
        Self {
            kind,
            description: None,
            changed: false,
        }
    }

    pub fn with_description(mut self, description: impl Into<SharedString>) -> Self {
        self.description = Some(description.into());
        self
    }

    pub const fn changed(mut self, changed: bool) -> Self {
        self.changed = changed;
        self
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct DesignSectionCapabilities {
    pub share: bool,
    pub set_dev_status: bool,
    pub completed_status: bool,
    pub resolve_changed_status: bool,
}

impl Default for DesignSectionCapabilities {
    fn default() -> Self {
        Self {
            share: true,
            set_dev_status: true,
            completed_status: true,
            resolve_changed_status: true,
        }
    }
}

/// Section-only properties from Figma's current inspector contract.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DesignSectionProperties {
    pub contents_hidden: bool,
    pub dev_status: Option<DesignSectionDevStatus>,
    pub capabilities: DesignSectionCapabilities,
}

/// Immutable data displayed by [`super::DesignPanel`].
#[derive(Clone, Debug, PartialEq)]
pub struct DesignPanelNode {
    pub id: SharedString,
    pub name: SharedString,
    pub kind: DesignPanelNodeKind,
    /// Optional authoritative inspector capabilities for this exact node.
    ///
    /// When absent, the compatibility presets on [`DesignPanelNodeKind`] are
    /// used. Hosts should provide this for `Other` and for future node kinds
    /// whose inspector surface cannot be inferred losslessly from the coarse
    /// compatibility taxonomy.
    pub capabilities: Option<DesignPanelNodeCapabilities>,
    /// Host-owned scene-node visibility shown by the Appearance eye control.
    pub visible: bool,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub rotation: f32,
    pub lock_aspect_ratio: bool,
    /// Whether this layer is nested below a component instance.
    ///
    /// Figma keeps width and height editable for these descendants but does
    /// not expose the aspect-ratio setting; it must be changed on the
    /// corresponding layer in the main component.
    pub is_component_instance_child: bool,
    pub horizontal_constraint: DesignConstraint,
    pub vertical_constraint: DesignConstraint,
    pub layout: Option<DesignLayout>,
    pub opacity: f32,
    pub blend_mode: DesignBlendMode,
    pub corner_radii: [f32; 4],
    pub independent_corners: bool,
    /// Figma `cornerSmoothing`, normalized to `0..=1`.
    pub corner_smoothing: f32,
    pub corner_capabilities: DesignCornerCapabilities,
    /// Node-level Figma `fillStyleId` binding for the complete ordered Fill
    /// paint collection.
    pub fill_style_binding: Option<DesignPaintStyleBinding>,
    pub fills: Vec<DesignPaint>,
    /// Host-controlled native Frame-fill export visibility.
    ///
    /// `None` means the inspected host/node does not expose Figma's
    /// "Show in exports" control. The public Plugin API has no corresponding
    /// field, so the inspector deliberately keeps this capability opaque.
    pub fill_shows_in_exports: Option<bool>,
    /// One shared stroke geometry/style with zero or more indexed paint fills.
    pub stroke: Option<DesignStroke>,
    /// Node-level Figma `strokeStyleId` binding for the complete ordered Stroke
    /// paint collection.
    pub stroke_style_binding: Option<DesignPaintStyleBinding>,
    pub effects: Vec<DesignEffect>,
    pub effect_capabilities: DesignEffectCapabilities,
    pub effect_style_binding: Option<DesignEffectStyleBinding>,
    pub layout_grids: Vec<DesignLayoutGrid>,
    /// Node-level Figma `gridStyleId` binding for the complete ordered guide
    /// collection.
    pub layout_grid_style_binding: Option<DesignLayoutGridStyleBinding>,
    pub export_settings: Vec<DesignExportSetting>,
    pub typography: Option<DesignTypography>,
    /// Present only when the host exposes native TextPath orientation state.
    pub text_path: Option<DesignTextPathViewData>,
    /// Present only for a TextPath node.
    pub text_path_start_data: Option<DesignTextPathStartData>,
    /// Present only while the host supplies Vector/TextPath sub-selection data.
    pub vector_edit: Option<DesignVectorEditViewData>,
    pub component_context: Option<DesignComponentContext>,
    pub component_properties: Vec<DesignComponentProperty>,
    /// Compatibility-only duplicate of image/video paint data.
    pub media: Option<DesignMedia>,
    pub shape_geometry: DesignShapeGeometry,
    pub section: Option<DesignSectionProperties>,
    pub transform_modifiers: Vec<DesignRepeatModifier>,
    /// Canonical orthogonal mask flag, independent of [`Self::kind`].
    pub is_mask: bool,
    /// Typed mask evaluation mode used when [`Self::is_mask`] is true.
    pub mask_mode: DesignMaskType,
    /// Legacy string projection retained for source compatibility. New hosts
    /// should use [`Self::is_mask`] and [`Self::mask_mode`].
    pub mask_type: Option<SharedString>,
    /// Canonical multiple-selection color aggregate.
    pub selection_color_aggregate: DesignSelectionColors,
    /// Legacy aggregate encoded as fake paints. New hosts should populate
    /// [`Self::selection_color_aggregate`].
    pub selection_colors: Vec<DesignPaint>,
}

impl DesignPanelNode {
    /// Creates a representative controlled inspector read model for a node kind.
    ///
    /// Hosts normally populate these fields from their own document read model;
    /// the preset is also useful for stories and integration smoke tests.
    pub fn new(
        id: impl Into<SharedString>,
        name: impl Into<SharedString>,
        kind: DesignPanelNodeKind,
    ) -> Self {
        let mut node = Self {
            id: id.into(),
            name: name.into(),
            kind,
            capabilities: None,
            visible: true,
            x: 120.,
            y: 96.,
            width: 320.,
            height: 180.,
            rotation: 0.,
            lock_aspect_ratio: false,
            is_component_instance_child: false,
            horizontal_constraint: DesignConstraint::Left,
            vertical_constraint: DesignConstraint::Top,
            layout: kind.supports_dimensions().then(DesignLayout::default),
            opacity: 100.,
            blend_mode: if kind.supports_pass_through_blend() {
                DesignBlendMode::PassThrough
            } else {
                DesignBlendMode::Normal
            },
            corner_radii: [0.; 4],
            independent_corners: false,
            corner_smoothing: 0.,
            corner_capabilities: DesignCornerCapabilities::for_node_kind(kind),
            fill_style_binding: None,
            fills: if kind.supports_fill() {
                vec![DesignPaint::solid(DesignColor::WHITE)]
            } else {
                Vec::new()
            },
            fill_shows_in_exports: (kind == DesignPanelNodeKind::Frame).then_some(true),
            stroke: None,
            stroke_style_binding: None,
            effects: Vec::new(),
            effect_capabilities: DesignEffectCapabilities::for_node_kind(kind),
            effect_style_binding: None,
            layout_grids: Vec::new(),
            layout_grid_style_binding: None,
            export_settings: Vec::new(),
            typography: matches!(
                kind,
                DesignPanelNodeKind::Text | DesignPanelNodeKind::TextPath
            )
            .then(DesignTypography::default),
            text_path: (kind == DesignPanelNodeKind::TextPath)
                .then_some(DesignTextPathViewData::default()),
            text_path_start_data: (kind == DesignPanelNodeKind::TextPath)
                .then_some(DesignTextPathStartData::DEFAULT),
            vector_edit: None,
            component_context: None,
            component_properties: Vec::new(),
            media: None,
            shape_geometry: DesignShapeGeometry::for_node_kind(kind),
            section: (kind == DesignPanelNodeKind::Section).then(DesignSectionProperties::default),
            transform_modifiers: if kind == DesignPanelNodeKind::TransformGroup {
                vec![DesignRepeatModifier::linear(
                    "repeat-0",
                    DesignRepeatAxis::Horizontal,
                )]
            } else {
                Vec::new()
            },
            is_mask: kind == DesignPanelNodeKind::Mask,
            mask_mode: DesignMaskType::Alpha,
            mask_type: (kind == DesignPanelNodeKind::Mask).then(|| "Alpha".into()),
            selection_color_aggregate: DesignSelectionColors::default(),
            selection_colors: Vec::new(),
        };
        node.apply_kind_preset();
        node.assign_preset_paint_ids();
        node
    }

    fn assign_preset_paint_ids(&mut self) {
        let node_id = self.id.clone();
        let assign = |paints: &mut [DesignPaint], collection: &str| {
            for (index, paint) in paints.iter_mut().enumerate() {
                if paint.id.is_empty() {
                    paint.id = format!("{node_id}-{collection}-{index}").into();
                }
                if let DesignPaintPayload::Gradient(gradient) = &mut paint.payload {
                    for (stop_index, stop) in gradient.stops.iter_mut().enumerate() {
                        if stop.id.is_empty() {
                            stop.id = format!("{}-stop-{stop_index}", paint.id).into();
                        }
                    }
                    paint.sync_legacy_projection();
                }
            }
        };
        assign(&mut self.fills, "fill");
        assign(&mut self.selection_colors, "selection-color");
        if let Some(stroke) = self.stroke.as_mut() {
            assign(&mut stroke.paints, "stroke");
        }
    }

    pub fn effect_count(&self, kind: DesignEffectKind) -> usize {
        self.effects
            .iter()
            .filter(|effect| effect.settings.kind() == kind)
            .count()
    }

    /// Whether changing/adding one row to `kind` is valid for the exact
    /// host-authored snapshot. `replacing_index` excludes the current row from
    /// the count when the user changes its kind.
    pub fn can_use_effect_kind(
        &self,
        kind: DesignEffectKind,
        replacing_index: Option<usize>,
    ) -> bool {
        if kind == DesignEffectKind::Unsupported
            || !self.effect_capabilities.kind_is_available(kind)
        {
            return false;
        }
        let replacing_same_kind = replacing_index
            .and_then(|index| self.effects.get(index))
            .is_some_and(|effect| effect.settings.kind() == kind);
        let count = self
            .effect_count(kind)
            .saturating_sub(usize::from(replacing_same_kind));
        count < usize::from(kind.maximum_per_node())
    }

    pub fn first_addable_effect_kind(&self) -> Option<DesignEffectKind> {
        DesignEffectKind::ALL
            .into_iter()
            .find(|kind| self.can_use_effect_kind(*kind, None))
    }

    pub fn effect_index_by_id(&self, effect_id: &str) -> Option<usize> {
        (!effect_id.is_empty()).then_some(())?;
        self.effects
            .iter()
            .position(|effect| effect.id.as_ref() == effect_id)
    }

    fn apply_kind_preset(&mut self) {
        match self.kind {
            DesignPanelNodeKind::Frame => {
                self.layout_grids.push(DesignLayoutGrid::columns(
                    DesignColumnLayoutGrid::default(),
                    DesignColor::rgb(0xf2, 0x48, 0x22),
                ));
            }
            DesignPanelNodeKind::Component => {
                let arrow = DesignComponentReference::local("icon-arrow-right", "Arrow right");
                let plus = DesignComponentReference::local("icon-plus", "Plus");
                let avatar = DesignComponentReference::remote(
                    "avatar-user",
                    "User avatar",
                    "Product foundations",
                );
                self.component_context = Some(DesignComponentContext {
                    role: DesignComponentRole::StandaloneMain,
                    main_component: Some(DesignComponentReference::local(
                        self.id.clone(),
                        self.name.clone(),
                    )),
                    description: Some(
                        "Primary action component with label, icon, and content-slot APIs.".into(),
                    ),
                    documentation_links: vec![DesignDocumentationLink::new(
                        "Button usage",
                        "https://example.com/components/button",
                    )],
                    overrides: DesignComponentOverrideSummary::default(),
                    authoring: None,
                });
                self.component_properties = vec![
                    DesignComponentProperty::text("label", "Label", "Button", "Button")
                        .with_multiline(true),
                    DesignComponentProperty::boolean("show-icon", "Show icon", true, true),
                    DesignComponentProperty::instance_swap(
                        "leading-icon",
                        "Leading icon",
                        Some(arrow.clone()),
                        Some(arrow.clone()),
                        vec![arrow, plus],
                    ),
                    DesignComponentProperty::slot(
                        "content",
                        "Content",
                        DesignSlotValue::default(),
                        DesignSlotValue::default(),
                        DesignSlotSettings {
                            preferred_values: vec![avatar],
                            ..DesignSlotSettings::default()
                        },
                        DesignSlotState::default(),
                    )
                    .with_description("Optional content inserted into the button"),
                ];
            }
            DesignPanelNodeKind::ComponentSet => {
                self.component_context = Some(DesignComponentContext {
                    role: DesignComponentRole::ComponentSet,
                    main_component: Some(DesignComponentReference::local(
                        self.id.clone(),
                        self.name.clone(),
                    )),
                    description: Some("Interactive button variants.".into()),
                    documentation_links: Vec::new(),
                    overrides: DesignComponentOverrideSummary::default(),
                    authoring: None,
                });
                self.component_properties = vec![
                    DesignComponentProperty::variant(
                        "state",
                        "State",
                        "Default",
                        "Default",
                        vec![
                            "Default".into(),
                            "Hover".into(),
                            "Pressed".into(),
                            "Disabled".into(),
                        ],
                    ),
                    DesignComponentProperty::variant(
                        "size",
                        "Size",
                        "Medium",
                        "Medium",
                        vec!["Small".into(), "Medium".into(), "Large".into()],
                    ),
                ];
            }
            DesignPanelNodeKind::Instance => {
                let arrow = DesignComponentReference::local("icon-arrow-right", "Arrow right");
                let plus = DesignComponentReference::local("icon-plus", "Plus");
                let check =
                    DesignComponentReference::remote("icon-check", "Check", "Product foundations");
                self.component_context = Some(DesignComponentContext {
                    role: DesignComponentRole::Instance,
                    main_component: Some(DesignComponentReference::remote(
                        "primary-button",
                        "Primary button",
                        "Product foundations",
                    )),
                    description: Some("Instance properties and nested overrides.".into()),
                    documentation_links: Vec::new(),
                    overrides: DesignComponentOverrideSummary {
                        overridden_property_count: 2,
                        nested_override_count: 1,
                        reset_state: DesignComponentResetState::Resettable,
                    },
                    authoring: None,
                });
                self.component_properties = vec![
                    DesignComponentProperty::variant(
                        "state",
                        "State",
                        "Default",
                        "Default",
                        vec!["Default".into(), "Hover".into(), "Pressed".into()],
                    ),
                    DesignComponentProperty::text("label", "Label", "Button", "Continue")
                        .with_reset_state(DesignComponentResetState::Resettable),
                    DesignComponentProperty::boolean("show-icon", "Show icon", true, true),
                    DesignComponentProperty::instance_swap(
                        "leading-icon",
                        "Leading icon",
                        Some(arrow.clone()),
                        Some(arrow.clone()),
                        vec![arrow, plus, check],
                    )
                    .with_reset_state(DesignComponentResetState::Resettable),
                ];
            }
            DesignPanelNodeKind::Text | DesignPanelNodeKind::TextPath => {
                self.width = 280.;
                self.height = 48.;
                self.fills = vec![DesignPaint::solid(DesignColor::BLACK)];
            }
            DesignPanelNodeKind::TransformGroup => {
                self.width = 280.;
                self.height = 180.;
                self.rotation = 8.;
            }
            DesignPanelNodeKind::Slot => {
                self.width = 240.;
                self.height = 120.;
                self.corner_radii = [8.; 4];
                let card = DesignComponentReference::local("content-card", "Content card");
                let empty = DesignComponentReference::local("empty-state", "Empty state");
                self.component_context = Some(DesignComponentContext {
                    role: DesignComponentRole::SlotDefinition,
                    main_component: Some(DesignComponentReference::local(
                        self.id.clone(),
                        self.name.clone(),
                    )),
                    description: Some(
                        "Defines which component instances can be inserted into this slot.".into(),
                    ),
                    documentation_links: Vec::new(),
                    overrides: DesignComponentOverrideSummary::default(),
                    authoring: None,
                });
                self.component_properties = vec![DesignComponentProperty::slot(
                    "content",
                    "Content",
                    DesignSlotValue::default(),
                    DesignSlotValue::default(),
                    DesignSlotSettings {
                        stretch_child_on_insert: true,
                        display_empty: true,
                        minimum_children: None,
                        maximum_children: Some(4),
                        preferred_values_only: false,
                        preferred_values: vec![card, empty],
                    },
                    DesignSlotState::default(),
                )];
            }
            DesignPanelNodeKind::Image | DesignPanelNodeKind::Video => {
                let media_paint = if self.kind == DesignPanelNodeKind::Image {
                    DesignPaint::image(DesignPaintSource::new("preset-image", "Image source"))
                } else {
                    DesignPaint::video(DesignPaintSource::new("preset-video", "Video source"))
                };
                self.fills = vec![media_paint.with_id("media-fill")];
                self.corner_radii = [12.; 4];
            }
            DesignPanelNodeKind::Rectangle => {
                self.corner_radii = [12.; 4];
                self.fills = vec![DesignPaint::gradient(
                    DesignPaintKind::LinearGradient,
                    vec![
                        DesignGradientStop::new(0., DesignColor::PURPLE),
                        DesignGradientStop::new(1., DesignColor::BLUE),
                    ],
                )];
            }
            DesignPanelNodeKind::Line | DesignPanelNodeKind::Arrow => {
                self.width = 240.;
                self.height = 0.;
                let mut stroke = DesignStroke::for_node(
                    self.kind,
                    DesignPaint::solid(DesignColor::BLACK),
                    2.,
                    DesignStrokeAlign::Inside,
                );
                stroke.end_cap = if self.kind == DesignPanelNodeKind::Arrow {
                    DesignStrokeCap::LineArrow
                } else {
                    DesignStrokeCap::None
                };
                self.stroke = Some(stroke);
            }
            DesignPanelNodeKind::Vector
            | DesignPanelNodeKind::BooleanOperation
            | DesignPanelNodeKind::Pen
            | DesignPanelNodeKind::Pencil => {
                self.fills = vec![DesignPaint::solid(DesignColor::PURPLE)];
                let mut stroke = DesignStroke::for_node(
                    self.kind,
                    DesignPaint::solid(DesignColor::BLACK),
                    1.,
                    DesignStrokeAlign::Center,
                );
                stroke.start_cap = DesignStrokeCap::Round;
                stroke.end_cap = DesignStrokeCap::Round;
                stroke.endpoint_cap = DesignStrokeCap::Round;
                self.stroke = Some(stroke);
            }
            DesignPanelNodeKind::MultipleSelection => {
                self.name = "3 layers selected".into();
                self.selection_color_aggregate = DesignSelectionColors::new([
                    DesignSelectionColor::new("purple", DesignColor::PURPLE, [])
                        .with_occurrence_count(1),
                    DesignSelectionColor::new("blue", DesignColor::BLUE, [])
                        .with_occurrence_count(1),
                    DesignSelectionColor::new("white", DesignColor::WHITE, [])
                        .with_occurrence_count(1),
                ]);
                self.selection_colors = vec![
                    DesignPaint::solid(DesignColor::PURPLE),
                    DesignPaint::solid(DesignColor::BLUE),
                    DesignPaint::solid(DesignColor::WHITE),
                ];
                self.fills.clear();
            }
            _ => {}
        }
    }

    /// Selection-color rows in canonical form.
    ///
    /// The legacy fake-paint projection is adapted only when the canonical
    /// aggregate is empty, allowing old hosts to migrate without teaching the
    /// panel to treat aggregate colors as normal fills.
    pub fn resolved_selection_colors(&self) -> Vec<DesignSelectionColor> {
        if !self.selection_color_aggregate.colors.is_empty() {
            return self.selection_color_aggregate.colors.clone();
        }
        self.selection_colors
            .iter()
            .enumerate()
            .map(|(index, paint)| {
                let binding = match &paint.payload {
                    DesignPaintPayload::Solid(solid) => solid.binding.clone(),
                    DesignPaintPayload::Gradient(gradient) => {
                        gradient.stops.first().and_then(|stop| stop.binding.clone())
                    }
                    DesignPaintPayload::Pattern(_)
                    | DesignPaintPayload::Image(_)
                    | DesignPaintPayload::Video(_)
                    | DesignPaintPayload::Shader(_)
                    | DesignPaintPayload::Unsupported(_) => None,
                };
                let mut color = DesignSelectionColor::new(
                    if paint.id.is_empty() {
                        format!("{}-legacy-selection-color-{index}", self.id).into()
                    } else {
                        paint.id.clone()
                    },
                    paint.color,
                    [],
                )
                .with_paint(paint.clone())
                .read_only(paint.read_only);
                color.binding = binding;
                color
            })
            .collect()
    }

    /// Canonical mask state with a compatibility fallback for hosts still
    /// populating the original string-only `mask_type` field.
    pub fn effective_is_mask(&self) -> bool {
        self.is_mask
            || self
                .mask_type
                .as_deref()
                .and_then(DesignMaskType::parse_compatibility_label)
                .is_some()
    }

    pub fn effective_mask_type(&self) -> Option<DesignMaskType> {
        if self.is_mask {
            Some(self.mask_mode)
        } else {
            self.mask_type
                .as_deref()
                .and_then(DesignMaskType::parse_compatibility_label)
        }
    }

    pub fn with_capabilities(mut self, capabilities: DesignPanelNodeCapabilities) -> Self {
        self.capabilities = Some(capabilities);
        self
    }

    pub const fn with_component_instance_child(mut self, is_child: bool) -> Self {
        self.is_component_instance_child = is_child;
        self
    }

    pub fn supports_dimensions(&self) -> bool {
        self.capabilities
            .as_ref()
            .map_or_else(|| self.kind.supports_dimensions(), |value| value.dimensions)
    }

    pub fn supports_visibility(&self) -> bool {
        self.capabilities
            .as_ref()
            .map_or_else(|| self.kind.supports_visibility(), |value| value.visibility)
    }

    pub fn supports_position_coordinates(&self) -> bool {
        self.capabilities.as_ref().map_or_else(
            || self.kind.supports_position_coordinates(),
            |value| value.position_coordinates,
        )
    }

    pub fn supports_arrange(&self) -> bool {
        self.capabilities
            .as_ref()
            .map_or_else(|| self.kind.supports_arrange(), |value| value.arrange)
    }

    pub fn supports_transforms(&self) -> bool {
        self.capabilities
            .as_ref()
            .map_or_else(|| self.kind.supports_transforms(), |value| value.transforms)
    }

    pub fn supports_aspect_ratio_lock(&self) -> bool {
        self.capabilities.as_ref().map_or_else(
            || self.kind.supports_aspect_ratio_lock(),
            |value| value.aspect_ratio_lock,
        )
    }

    pub fn supports_auto_layout_child(&self) -> bool {
        self.capabilities.as_ref().map_or_else(
            || self.kind.supports_auto_layout_child(),
            |value| value.auto_layout_child,
        )
    }

    pub fn supports_add_auto_layout(&self) -> bool {
        self.capabilities.as_ref().map_or_else(
            || self.kind.supports_add_auto_layout(),
            |value| value.add_auto_layout,
        )
    }

    pub fn supports_auto_layout_container(&self) -> bool {
        self.capabilities.as_ref().map_or_else(
            || self.kind.supports_auto_layout_container(),
            |value| value.auto_layout_container,
        )
    }

    /// Whether the current host snapshot can expose a numeric Max lines value.
    ///
    /// Figma additionally requires a text layer inside auto layout to use
    /// vertical Hug sizing. `inside_auto_layout` comes from the host's exact
    /// parent-layout context rather than being inferred from this node.
    pub fn text_max_lines_are_available(&self, inside_auto_layout: bool) -> bool {
        self.typography
            .as_ref()
            .is_some_and(DesignTypography::text_max_lines_are_available)
            && (!inside_auto_layout
                || self
                    .layout
                    .as_ref()
                    .is_some_and(|layout| layout.vertical_sizing == DesignSizingMode::Hug))
    }

    /// Applies a host-accepted resize value and restores the nullable Max
    /// lines value to Auto when the new mode cannot support a line limit.
    pub fn set_text_resize(&mut self, resize: DesignTextResize) -> bool {
        let Some(typography) = self.typography.as_mut() else {
            return false;
        };
        typography.resize = resize;
        if resize == DesignTextResize::Fixed {
            typography.max_lines = None;
        }
        true
    }

    /// Applies host-accepted truncation and restores Max lines to Auto when
    /// ending truncation is disabled.
    pub fn set_text_truncation(&mut self, truncate: bool) -> bool {
        let Some(typography) = self.typography.as_mut() else {
            return false;
        };
        typography.truncate = truncate;
        if !truncate {
            typography.max_lines = None;
        }
        true
    }

    /// Applies Figma's node-level nullable `maxLines` contract.
    ///
    /// A numeric line limit removes Max height atomically. `None` is the
    /// native Auto value and leaves an independently supplied Max height
    /// untouched.
    pub fn set_text_max_lines(&mut self, max_lines: Option<u32>, inside_auto_layout: bool) -> bool {
        if max_lines.is_some_and(|lines| lines == 0)
            || (max_lines.is_some() && !self.text_max_lines_are_available(inside_auto_layout))
        {
            return false;
        }
        let Some(typography) = self.typography.as_mut() else {
            return false;
        };
        typography.max_lines = max_lines;
        if max_lines.is_some()
            && let Some(layout) = self.layout.as_mut()
        {
            layout.item.max_height = None;
        }
        true
    }

    /// Applies a nullable Max height and atomically resets numeric Max lines
    /// to Auto when Figma's mutually-exclusive height limit becomes active.
    pub fn set_layout_max_height(&mut self, max_height: Option<f32>) -> bool {
        if max_height.is_some_and(|height| !height.is_finite() || height <= 0.) {
            return false;
        }
        let Some(layout) = self.layout.as_mut() else {
            return false;
        };
        layout.item.max_height = max_height;
        if max_height.is_some()
            && let Some(typography) = self.typography.as_mut()
        {
            typography.max_lines = None;
        }
        true
    }

    /// Revalidates a previously supplied numeric Max lines value after a
    /// resize or parent-layout change. Invalid values become native Auto.
    pub fn normalize_text_max_lines(&mut self, inside_auto_layout: bool) {
        if !self.text_max_lines_are_available(inside_auto_layout)
            && let Some(typography) = self.typography.as_mut()
        {
            typography.max_lines = None;
        }
    }

    pub fn supports_grid_auto_layout(&self) -> bool {
        self.capabilities.as_ref().map_or_else(
            || self.kind.supports_grid_auto_layout(),
            |value| value.grid_auto_layout,
        )
    }

    pub fn supports_resize_to_fit(&self) -> bool {
        self.capabilities.as_ref().map_or_else(
            || self.kind.supports_resize_to_fit(),
            |value| value.resize_to_fit,
        )
    }

    pub fn supports_clip_content(&self) -> bool {
        self.capabilities.as_ref().map_or_else(
            || self.kind.supports_clip_content(),
            |value| value.clip_content,
        )
    }

    pub fn supports_fill(&self) -> bool {
        self.capabilities
            .as_ref()
            .map_or_else(|| self.kind.supports_fill(), |value| value.fill)
    }

    pub fn supports_stroke(&self) -> bool {
        self.capabilities
            .as_ref()
            .map_or_else(|| self.kind.supports_stroke(), |value| value.stroke)
    }

    pub fn supports_layer_appearance(&self) -> bool {
        self.capabilities.as_ref().map_or_else(
            || self.kind.supports_layer_appearance(),
            |value| value.layer_appearance,
        )
    }

    pub fn supports_pass_through_blend(&self) -> bool {
        self.capabilities.as_ref().map_or_else(
            || self.kind.supports_pass_through_blend(),
            |value| value.pass_through_blend,
        )
    }

    pub fn supports_effects(&self) -> bool {
        self.capabilities
            .as_ref()
            .map_or_else(|| self.kind.supports_effects(), |value| value.effects)
    }

    pub fn supports_constraints(&self) -> bool {
        self.capabilities.as_ref().map_or_else(
            || self.kind.supports_constraints(),
            |value| value.constraints,
        )
    }

    pub fn supports_layout_guides(&self) -> bool {
        self.capabilities.as_ref().map_or_else(
            || {
                matches!(
                    self.kind,
                    DesignPanelNodeKind::Frame
                        | DesignPanelNodeKind::Component
                        | DesignPanelNodeKind::ComponentSet
                        | DesignPanelNodeKind::Instance
                        | DesignPanelNodeKind::Slot
                )
            },
            |value| value.layout_guides,
        )
    }

    /// Whether an exact known section belongs to this node's inspector.
    ///
    /// An explicit capability snapshot preserves the host's order and
    /// visibility. The semantic checks keep contradictory snapshots from
    /// exposing controls whose intents the same snapshot forbids.
    pub fn supports_section(&self, section: DesignPanelSection) -> bool {
        if let Some(capabilities) = &self.capabilities {
            return capabilities.sections.contains(&section)
                && match section {
                    DesignPanelSection::Layout => capabilities.dimensions,
                    DesignPanelSection::Layer => {
                        capabilities.visibility
                            || capabilities.layer_appearance
                            || self.corner_capabilities.has_any()
                            || self.shape_geometry.has_appearance_controls()
                    }
                    DesignPanelSection::Fill => capabilities.fill,
                    DesignPanelSection::Stroke => capabilities.stroke,
                    DesignPanelSection::Effects => capabilities.effects,
                    DesignPanelSection::LayoutGrid => capabilities.layout_guides,
                    DesignPanelSection::Constraints => capabilities.constraints,
                    DesignPanelSection::Selection
                    | DesignPanelSection::Component
                    | DesignPanelSection::Instance
                    | DesignPanelSection::Position
                    | DesignPanelSection::Section
                    | DesignPanelSection::Transform
                    | DesignPanelSection::Geometry
                    | DesignPanelSection::Typography
                    | DesignPanelSection::Media
                    | DesignPanelSection::Export => true,
                    DesignPanelSection::Mask => self.effective_is_mask(),
                };
        }

        match section {
            DesignPanelSection::Selection => {
                !self.selection_color_aggregate.colors.is_empty()
                    || !self.selection_colors.is_empty()
            }
            DesignPanelSection::Component => self
                .component_role()
                .is_some_and(|role| !role.uses_instance_section()),
            DesignPanelSection::Instance => self
                .component_role()
                .is_some_and(DesignComponentRole::uses_instance_section),
            DesignPanelSection::Position => true,
            DesignPanelSection::Layout => self.supports_dimensions(),
            DesignPanelSection::Constraints => self.supports_constraints(),
            DesignPanelSection::Layer => {
                self.kind != DesignPanelNodeKind::Slice
                    && (self.supports_visibility()
                        || self.supports_layer_appearance()
                        || self.corner_capabilities.has_any()
                        || self.shape_geometry.has_appearance_controls())
            }
            DesignPanelSection::Section => self.section.is_some(),
            DesignPanelSection::Transform => self.kind == DesignPanelNodeKind::TransformGroup,
            DesignPanelSection::Geometry => {
                matches!(self.shape_geometry, DesignShapeGeometry::Table(_))
            }
            DesignPanelSection::Mask => self.effective_is_mask(),
            DesignPanelSection::Typography => self.typography.is_some(),
            DesignPanelSection::Media => false,
            DesignPanelSection::Fill => self.supports_fill(),
            DesignPanelSection::Stroke => self.supports_stroke(),
            DesignPanelSection::Effects => self.supports_effects(),
            DesignPanelSection::LayoutGrid => self.supports_layout_guides(),
            DesignPanelSection::Export => true,
        }
    }

    pub fn with_layout_mode(mut self, mode: DesignLayoutMode) -> Self {
        let supports_auto_layout = self.supports_auto_layout_container();
        let supports_grid = self.supports_grid_auto_layout();
        if let Some(layout) = self.layout.as_mut()
            && layout.set_mode_for_capabilities(supports_auto_layout, supports_grid, mode)
            && mode != DesignLayoutMode::None
        {
            layout.horizontal_sizing = DesignSizingMode::Hug;
            layout.vertical_sizing = DesignSizingMode::Hug;
        }
        self
    }

    pub fn with_corner_capabilities(mut self, capabilities: DesignCornerCapabilities) -> Self {
        self.corner_capabilities = capabilities;
        self
    }

    pub fn with_shape_geometry(mut self, geometry: DesignShapeGeometry) -> Self {
        self.shape_geometry = geometry;
        self
    }

    pub fn with_component_context(mut self, context: DesignComponentContext) -> Self {
        self.component_context = Some(context);
        self
    }

    /// Returns explicit host context first, then a compatibility inference for
    /// the original node-kind-only adapter.
    pub fn component_role(&self) -> Option<DesignComponentRole> {
        self.component_context
            .as_ref()
            .map(|context| context.role)
            .or(match self.kind {
                DesignPanelNodeKind::Component => Some(DesignComponentRole::StandaloneMain),
                DesignPanelNodeKind::ComponentSet => Some(DesignComponentRole::ComponentSet),
                DesignPanelNodeKind::Instance => Some(DesignComponentRole::Instance),
                DesignPanelNodeKind::Slot => Some(DesignComponentRole::SlotDefinition),
                _ if !self.component_properties.is_empty() => {
                    Some(DesignComponentRole::StandaloneMain)
                }
                _ => None,
            })
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DesignPanelSection {
    Selection,
    Component,
    Instance,
    Position,
    Layout,
    Constraints,
    Layer,
    Section,
    Transform,
    /// Compatibility-only shape section. Canonical shape controls compose
    /// inside [`Self::Layer`] (Appearance); FigJam Table and exact legacy host
    /// snapshots may still request this identifier.
    Geometry,
    Mask,
    Typography,
    /// Compatibility-only section; media controls now belong to Fill paints.
    Media,
    Fill,
    Stroke,
    Effects,
    LayoutGrid,
    Export,
}

impl DesignPanelSection {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Selection => "Selection colors",
            Self::Component => "Component properties",
            Self::Instance => "Instance",
            Self::Position => "Position",
            Self::Layout => "Layout",
            Self::Constraints => "Constraints",
            Self::Layer => "Appearance",
            Self::Section => "Section",
            Self::Transform => "Transform",
            Self::Geometry => "Geometry",
            Self::Mask => "Mask",
            Self::Typography => "Typography",
            Self::Media => "Media",
            Self::Fill => "Fill",
            Self::Stroke => "Stroke",
            Self::Effects => "Effects",
            Self::LayoutGrid => "Layout guides",
            Self::Export => "Export",
        }
    }
}

/// Host-authored inspector capabilities for one exact scene node.
///
/// `sections` is the authoritative ordered set of known Design-panel sections.
/// The remaining fields gate the semantics inside those sections and the
/// corresponding intents. This separation is intentional: for example,
/// Section nodes can expose corner controls in `Layer` without implementing
/// layer opacity/blend/effects, while a future host node may expose layout
/// guides without owning auto layout.
///
/// A host can start from [`Self::for_node_kind`] and replace `sections` or
/// individual semantic flags. [`DesignPanelNode`] uses kind-derived behavior
/// only when its optional capability snapshot is absent.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DesignPanelNodeCapabilities {
    pub sections: Vec<DesignPanelSection>,
    pub dimensions: bool,
    pub visibility: bool,
    pub position_coordinates: bool,
    pub arrange: bool,
    pub transforms: bool,
    pub aspect_ratio_lock: bool,
    pub auto_layout_child: bool,
    pub add_auto_layout: bool,
    pub auto_layout_container: bool,
    pub grid_auto_layout: bool,
    pub resize_to_fit: bool,
    pub clip_content: bool,
    pub fill: bool,
    pub stroke: bool,
    pub layer_appearance: bool,
    pub pass_through_blend: bool,
    pub effects: bool,
    pub constraints: bool,
    pub layout_guides: bool,
}

impl DesignPanelNodeCapabilities {
    /// Compatibility capability snapshot for a coarse node kind.
    ///
    /// Type-specific sections reflect the kind's representative preset.
    /// Hosts that attach additional controlled type data should replace the
    /// ordered `sections` list explicitly.
    pub fn for_node_kind(kind: DesignPanelNodeKind) -> Self {
        let dimensions = kind.supports_dimensions();
        let visibility = kind.supports_visibility();
        let fill = kind.supports_fill();
        let stroke = kind.supports_stroke();
        let layer_appearance = kind.supports_layer_appearance();
        let effects = kind.supports_effects();
        let layout_guides = matches!(
            kind,
            DesignPanelNodeKind::Frame
                | DesignPanelNodeKind::Component
                | DesignPanelNodeKind::ComponentSet
                | DesignPanelNodeKind::Instance
                | DesignPanelNodeKind::Slot
        );
        let mut sections = vec![DesignPanelSection::Position];
        if dimensions {
            sections.push(DesignPanelSection::Layout);
        }
        if kind != DesignPanelNodeKind::Slice
            && (visibility
                || layer_appearance
                || DesignCornerCapabilities::for_node_kind(kind).has_any()
                || DesignShapeGeometry::for_node_kind(kind).has_appearance_controls())
        {
            sections.push(DesignPanelSection::Layer);
        }
        if kind == DesignPanelNodeKind::MultipleSelection {
            sections.push(DesignPanelSection::Selection);
        }
        match kind {
            DesignPanelNodeKind::Component | DesignPanelNodeKind::ComponentSet => {
                sections.push(DesignPanelSection::Component);
            }
            DesignPanelNodeKind::Instance => sections.push(DesignPanelSection::Instance),
            DesignPanelNodeKind::Slot => sections.push(DesignPanelSection::Component),
            _ => {}
        }
        if kind == DesignPanelNodeKind::Section {
            sections.push(DesignPanelSection::Section);
        }
        if kind == DesignPanelNodeKind::TransformGroup {
            sections.push(DesignPanelSection::Transform);
        }
        if kind == DesignPanelNodeKind::Table {
            sections.push(DesignPanelSection::Geometry);
        }
        if kind == DesignPanelNodeKind::Mask {
            sections.push(DesignPanelSection::Mask);
        }
        if matches!(
            kind,
            DesignPanelNodeKind::Text | DesignPanelNodeKind::TextPath
        ) {
            sections.push(DesignPanelSection::Typography);
        }
        if fill {
            sections.push(DesignPanelSection::Fill);
        }
        if stroke {
            sections.push(DesignPanelSection::Stroke);
        }
        if effects {
            sections.push(DesignPanelSection::Effects);
        }
        if layout_guides {
            sections.push(DesignPanelSection::LayoutGrid);
        }
        sections.push(DesignPanelSection::Export);

        Self {
            sections,
            dimensions,
            visibility,
            position_coordinates: kind.supports_position_coordinates(),
            arrange: kind.supports_arrange(),
            transforms: kind.supports_transforms(),
            aspect_ratio_lock: kind.supports_aspect_ratio_lock(),
            auto_layout_child: kind.supports_auto_layout_child(),
            add_auto_layout: kind.supports_add_auto_layout(),
            auto_layout_container: kind.supports_auto_layout_container(),
            grid_auto_layout: kind.supports_grid_auto_layout(),
            resize_to_fit: kind.supports_resize_to_fit(),
            clip_content: kind.supports_clip_content(),
            fill,
            stroke,
            layer_appearance,
            pass_through_blend: kind.supports_pass_through_blend(),
            effects,
            constraints: kind.supports_constraints(),
            layout_guides,
        }
    }

    pub fn with_sections(mut self, sections: impl IntoIterator<Item = DesignPanelSection>) -> Self {
        self.sections = sections.into_iter().collect();
        self
    }

    pub const fn with_dimensions(mut self, dimensions: bool) -> Self {
        self.dimensions = dimensions;
        self
    }

    pub const fn with_visibility(mut self, supported: bool) -> Self {
        self.visibility = supported;
        self
    }

    pub const fn with_position_coordinates(mut self, supported: bool) -> Self {
        self.position_coordinates = supported;
        self
    }

    pub const fn with_arrange(mut self, supported: bool) -> Self {
        self.arrange = supported;
        self
    }

    pub const fn with_transforms(mut self, supported: bool) -> Self {
        self.transforms = supported;
        self
    }

    pub const fn with_aspect_ratio_lock(mut self, supported: bool) -> Self {
        self.aspect_ratio_lock = supported;
        self
    }

    pub const fn with_auto_layout_child(mut self, supported: bool) -> Self {
        self.auto_layout_child = supported;
        self
    }

    pub const fn with_add_auto_layout(mut self, supported: bool) -> Self {
        self.add_auto_layout = supported;
        self
    }

    pub const fn with_auto_layout_container(mut self, supported: bool) -> Self {
        self.auto_layout_container = supported;
        self
    }

    pub const fn with_grid_auto_layout(mut self, supported: bool) -> Self {
        self.grid_auto_layout = supported;
        self
    }

    pub const fn with_resize_to_fit(mut self, supported: bool) -> Self {
        self.resize_to_fit = supported;
        self
    }

    pub const fn with_clip_content(mut self, supported: bool) -> Self {
        self.clip_content = supported;
        self
    }

    pub const fn with_fill(mut self, supported: bool) -> Self {
        self.fill = supported;
        self
    }

    pub const fn with_stroke(mut self, supported: bool) -> Self {
        self.stroke = supported;
        self
    }

    pub const fn with_layer_appearance(mut self, supported: bool) -> Self {
        self.layer_appearance = supported;
        self
    }

    pub const fn with_pass_through_blend(mut self, supported: bool) -> Self {
        self.pass_through_blend = supported;
        self
    }

    pub const fn with_effects(mut self, supported: bool) -> Self {
        self.effects = supported;
        self
    }

    pub const fn with_constraints(mut self, supported: bool) -> Self {
        self.constraints = supported;
        self
    }

    pub const fn with_layout_guides(mut self, supported: bool) -> Self {
        self.layout_guides = supported;
        self
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DesignPanelCollection {
    Fill,
    Stroke,
    Effect,
    LayoutGrid,
    Export,
}

impl DesignPanelCollection {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Fill => "Fill",
            Self::Stroke => "Stroke",
            Self::Effect => "Effect",
            Self::LayoutGrid => "Layout guide",
            Self::Export => "Export",
        }
    }
}

impl DesignPanelNode {
    /// Maps one visible inspector property to the exact Figma variable fields
    /// that a host must bind. Composite padding and independent-corner
    /// controls deliberately return every affected leaf.
    pub fn property_variable_target(
        &self,
        property: DesignPanelProperty,
    ) -> Option<DesignPropertyVariableTarget> {
        use DesignVariableBindableNodeField as NodeField;
        use DesignVariableBindableTextField as TextField;
        use DesignVariableResolvedType as Type;
        use DesignVariableScope as Scope;

        macro_rules! node {
            ($fields:expr, $resolved_type:expr, $scope:expr $(,)?) => {
                DesignPropertyVariableTarget::node(property, $fields, $resolved_type, $scope)
            };
        }
        let text = |field, resolved_type, scope| {
            DesignPropertyVariableTarget::text(property, field, resolved_type, scope)
        };
        let capability_allows_property = match property {
            DesignPanelProperty::Visible => {
                self.supports_section(DesignPanelSection::Layer) && self.supports_visibility()
            }
            DesignPanelProperty::Opacity => {
                self.supports_section(DesignPanelSection::Layer) && self.supports_layer_appearance()
            }
            DesignPanelProperty::Width | DesignPanelProperty::Height => {
                self.supports_section(DesignPanelSection::Layout) && self.supports_dimensions()
            }
            DesignPanelProperty::Gap
            | DesignPanelProperty::CounterAxisGap
            | DesignPanelProperty::PaddingVertical
            | DesignPanelProperty::PaddingHorizontal
            | DesignPanelProperty::PaddingShorthand
            | DesignPanelProperty::PaddingTop
            | DesignPanelProperty::PaddingRight
            | DesignPanelProperty::PaddingBottom
            | DesignPanelProperty::PaddingLeft => {
                self.supports_section(DesignPanelSection::Layout)
                    && self.supports_auto_layout_container()
            }
            DesignPanelProperty::MinWidth
            | DesignPanelProperty::MaxWidth
            | DesignPanelProperty::MinHeight
            | DesignPanelProperty::MaxHeight => {
                self.supports_section(DesignPanelSection::Layout)
                    && self.supports_dimensions()
                    && (self.supports_auto_layout_container() || self.supports_auto_layout_child())
            }
            DesignPanelProperty::CornerRadius
            | DesignPanelProperty::CornerRadiusTopLeft
            | DesignPanelProperty::CornerRadiusTopRight
            | DesignPanelProperty::CornerRadiusBottomRight
            | DesignPanelProperty::CornerRadiusBottomLeft => {
                self.supports_section(DesignPanelSection::Layer)
            }
            DesignPanelProperty::StrokeWeight
            | DesignPanelProperty::StrokeWeightTop
            | DesignPanelProperty::StrokeWeightRight
            | DesignPanelProperty::StrokeWeightBottom
            | DesignPanelProperty::StrokeWeightLeft => {
                self.supports_section(DesignPanelSection::Stroke) && self.supports_stroke()
            }
            property if property.is_typography() => {
                self.supports_section(DesignPanelSection::Typography)
            }
            _ => true,
        };
        if !capability_allows_property {
            return None;
        }
        match property {
            DesignPanelProperty::Visible if self.supports_visibility() => {
                Some(node!([NodeField::Visible], Type::Boolean, None,))
            }
            DesignPanelProperty::Width if self.supports_dimensions() => Some(node!(
                [NodeField::Width],
                Type::Float,
                Some(Scope::WidthHeight),
            )),
            DesignPanelProperty::Height if self.supports_dimensions() => Some(node!(
                [NodeField::Height],
                Type::Float,
                Some(Scope::WidthHeight),
            )),
            DesignPanelProperty::Gap => {
                let layout = self.layout.as_ref()?;
                match layout.mode {
                    DesignLayoutMode::Horizontal | DesignLayoutMode::Vertical => Some(node!(
                        [NodeField::ItemSpacing],
                        Type::Float,
                        Some(Scope::Gap),
                    )),
                    DesignLayoutMode::Grid => Some(node!(
                        [NodeField::GridColumnGap],
                        Type::Float,
                        Some(Scope::Gap),
                    )),
                    DesignLayoutMode::None => None,
                }
            }
            DesignPanelProperty::CounterAxisGap => {
                let layout = self.layout.as_ref()?;
                match layout.mode {
                    DesignLayoutMode::Grid => Some(node!(
                        [NodeField::GridRowGap],
                        Type::Float,
                        Some(Scope::Gap),
                    )),
                    DesignLayoutMode::Horizontal if layout.wrap => Some(node!(
                        [NodeField::CounterAxisSpacing],
                        Type::Float,
                        Some(Scope::Gap),
                    )),
                    DesignLayoutMode::None
                    | DesignLayoutMode::Horizontal
                    | DesignLayoutMode::Vertical => None,
                }
            }
            DesignPanelProperty::PaddingVertical
                if self.layout.as_ref()?.mode != DesignLayoutMode::None =>
            {
                Some(node!(
                    [NodeField::PaddingTop, NodeField::PaddingBottom],
                    Type::Float,
                    Some(Scope::Gap),
                ))
            }
            DesignPanelProperty::PaddingHorizontal
                if self.layout.as_ref()?.mode != DesignLayoutMode::None =>
            {
                Some(node!(
                    [NodeField::PaddingLeft, NodeField::PaddingRight],
                    Type::Float,
                    Some(Scope::Gap),
                ))
            }
            DesignPanelProperty::PaddingShorthand
                if self.layout.as_ref()?.mode != DesignLayoutMode::None =>
            {
                Some(node!(
                    [
                        NodeField::PaddingTop,
                        NodeField::PaddingRight,
                        NodeField::PaddingBottom,
                        NodeField::PaddingLeft,
                    ],
                    Type::Float,
                    Some(Scope::Gap),
                ))
            }
            DesignPanelProperty::PaddingTop
                if self.layout.as_ref()?.mode != DesignLayoutMode::None =>
            {
                Some(node!(
                    [NodeField::PaddingTop],
                    Type::Float,
                    Some(Scope::Gap),
                ))
            }
            DesignPanelProperty::PaddingRight
                if self.layout.as_ref()?.mode != DesignLayoutMode::None =>
            {
                Some(node!(
                    [NodeField::PaddingRight],
                    Type::Float,
                    Some(Scope::Gap),
                ))
            }
            DesignPanelProperty::PaddingBottom
                if self.layout.as_ref()?.mode != DesignLayoutMode::None =>
            {
                Some(node!(
                    [NodeField::PaddingBottom],
                    Type::Float,
                    Some(Scope::Gap),
                ))
            }
            DesignPanelProperty::PaddingLeft
                if self.layout.as_ref()?.mode != DesignLayoutMode::None =>
            {
                Some(node!(
                    [NodeField::PaddingLeft],
                    Type::Float,
                    Some(Scope::Gap),
                ))
            }
            DesignPanelProperty::MinWidth if self.layout.is_some() => Some(node!(
                [NodeField::MinWidth],
                Type::Float,
                Some(Scope::WidthHeight),
            )),
            DesignPanelProperty::MaxWidth if self.layout.is_some() => Some(node!(
                [NodeField::MaxWidth],
                Type::Float,
                Some(Scope::WidthHeight),
            )),
            DesignPanelProperty::MinHeight if self.layout.is_some() => Some(node!(
                [NodeField::MinHeight],
                Type::Float,
                Some(Scope::WidthHeight),
            )),
            DesignPanelProperty::MaxHeight if self.layout.is_some() => Some(node!(
                [NodeField::MaxHeight],
                Type::Float,
                Some(Scope::WidthHeight),
            )),
            DesignPanelProperty::Opacity if self.supports_layer_appearance() => Some(node!(
                [NodeField::Opacity],
                Type::Float,
                Some(Scope::Opacity),
            )),
            DesignPanelProperty::CornerRadius if self.corner_capabilities.uniform_radius => {
                let fields = if self.corner_capabilities.independent_radii {
                    vec![
                        NodeField::TopLeftRadius,
                        NodeField::TopRightRadius,
                        NodeField::BottomRightRadius,
                        NodeField::BottomLeftRadius,
                    ]
                } else {
                    vec![NodeField::CornerRadius]
                };
                Some(node!(fields, Type::Float, Some(Scope::CornerRadius)))
            }
            DesignPanelProperty::CornerRadiusTopLeft
                if self.corner_capabilities.independent_radii =>
            {
                Some(node!(
                    [NodeField::TopLeftRadius],
                    Type::Float,
                    Some(Scope::CornerRadius),
                ))
            }
            DesignPanelProperty::CornerRadiusTopRight
                if self.corner_capabilities.independent_radii =>
            {
                Some(node!(
                    [NodeField::TopRightRadius],
                    Type::Float,
                    Some(Scope::CornerRadius),
                ))
            }
            DesignPanelProperty::CornerRadiusBottomRight
                if self.corner_capabilities.independent_radii =>
            {
                Some(node!(
                    [NodeField::BottomRightRadius],
                    Type::Float,
                    Some(Scope::CornerRadius),
                ))
            }
            DesignPanelProperty::CornerRadiusBottomLeft
                if self.corner_capabilities.independent_radii =>
            {
                Some(node!(
                    [NodeField::BottomLeftRadius],
                    Type::Float,
                    Some(Scope::CornerRadius),
                ))
            }
            DesignPanelProperty::StrokeWeight if self.stroke.is_some() => Some(node!(
                [NodeField::StrokeWeight],
                Type::Float,
                Some(Scope::StrokeFloat),
            )),
            DesignPanelProperty::StrokeWeightTop
                if self
                    .stroke
                    .as_ref()
                    .is_some_and(|stroke| stroke.capabilities.individual_weights) =>
            {
                Some(node!(
                    [NodeField::StrokeTopWeight],
                    Type::Float,
                    Some(Scope::StrokeFloat),
                ))
            }
            DesignPanelProperty::StrokeWeightRight
                if self
                    .stroke
                    .as_ref()
                    .is_some_and(|stroke| stroke.capabilities.individual_weights) =>
            {
                Some(node!(
                    [NodeField::StrokeRightWeight],
                    Type::Float,
                    Some(Scope::StrokeFloat),
                ))
            }
            DesignPanelProperty::StrokeWeightBottom
                if self
                    .stroke
                    .as_ref()
                    .is_some_and(|stroke| stroke.capabilities.individual_weights) =>
            {
                Some(node!(
                    [NodeField::StrokeBottomWeight],
                    Type::Float,
                    Some(Scope::StrokeFloat),
                ))
            }
            DesignPanelProperty::StrokeWeightLeft
                if self
                    .stroke
                    .as_ref()
                    .is_some_and(|stroke| stroke.capabilities.individual_weights) =>
            {
                Some(node!(
                    [NodeField::StrokeLeftWeight],
                    Type::Float,
                    Some(Scope::StrokeFloat),
                ))
            }
            DesignPanelProperty::FontFamily if self.typography.is_some() => {
                Some(text(TextField::FontFamily, Type::String, Scope::FontFamily))
            }
            DesignPanelProperty::FontStyle if self.typography.is_some() => {
                Some(text(TextField::FontStyle, Type::String, Scope::FontStyle))
            }
            DesignPanelProperty::FontWeight if self.typography.is_some() => {
                Some(text(TextField::FontWeight, Type::Float, Scope::FontWeight))
            }
            DesignPanelProperty::FontSize if self.typography.is_some() => {
                Some(text(TextField::FontSize, Type::Float, Scope::FontSize))
            }
            DesignPanelProperty::LineHeight if self.typography.is_some() => {
                Some(text(TextField::LineHeight, Type::Float, Scope::LineHeight))
            }
            DesignPanelProperty::LetterSpacing if self.typography.is_some() => Some(text(
                TextField::LetterSpacing,
                Type::Float,
                Scope::LetterSpacing,
            )),
            DesignPanelProperty::ParagraphSpacing if self.typography.is_some() => Some(text(
                TextField::ParagraphSpacing,
                Type::Float,
                Scope::ParagraphSpacing,
            )),
            DesignPanelProperty::ParagraphIndent if self.typography.is_some() => Some(text(
                TextField::ParagraphIndent,
                Type::Float,
                Scope::ParagraphIndent,
            )),
            _ => None,
        }
    }
}
