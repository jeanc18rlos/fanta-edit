use gpui::SharedString;

/// The Figma-like layer type rendered by [`super::LayersPanel`].
///
/// The variants intentionally describe UI capabilities rather than any Fanta
/// document schema. Hosts are free to map their own node types onto them.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum LayersPanelNodeKind {
    Frame,
    Group,
    Section,
    Component,
    ComponentSet,
    Instance,
    Text,
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
    Pen,
    Pencil,
    Other,
}

impl LayersPanelNodeKind {
    /// User-facing name used by stories, accessibility labels, and diagnostics.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Frame => "Frame",
            Self::Group => "Group",
            Self::Section => "Section",
            Self::Component => "Component",
            Self::ComponentSet => "Component set",
            Self::Instance => "Instance",
            Self::Text => "Text",
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
            Self::Pen => "Pen",
            Self::Pencil => "Pencil",
            Self::Other => "Other",
        }
    }

    pub(crate) const fn is_container(self) -> bool {
        matches!(
            self,
            Self::Frame
                | Self::Group
                | Self::Section
                | Self::Component
                | Self::ComponentSet
                | Self::Instance
                | Self::BooleanOperation
                | Self::Mask
        )
    }

    /// Whether a layer dropped onto this kind may become its child. Instances
    /// are containers for layout purposes but never accept real children —
    /// their subtree comes from the main component.
    pub(crate) const fn accepts_dropped_children(self) -> bool {
        self.is_container() && !matches!(self, Self::Instance)
    }

    pub(crate) const fn can_flip(self) -> bool {
        !matches!(self, Self::Section | Self::Slice)
    }

    pub(crate) const fn supports_auto_layout(self) -> bool {
        !matches!(self, Self::Section | Self::Slice | Self::Line | Self::Arrow)
    }

    pub(crate) const fn can_create_component(self) -> bool {
        !matches!(
            self,
            Self::Section | Self::Slice | Self::Component | Self::ComponentSet | Self::Instance
        )
    }
}

/// Immutable node data displayed by [`super::LayersPanel`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LayersPanelItem {
    /// Opaque host identifier. It is returned unchanged in emitted intents.
    pub id: SharedString,
    /// User-facing layer name.
    pub title: SharedString,
    pub kind: LayersPanelNodeKind,
    /// Child order is render order in the tree.
    pub children: Vec<Self>,
    pub visible: bool,
    pub locked: bool,
}

impl LayersPanelItem {
    /// Creates a visible, unlocked leaf node.
    pub fn new(
        id: impl Into<SharedString>,
        title: impl Into<SharedString>,
        kind: LayersPanelNodeKind,
    ) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            kind,
            children: Vec::new(),
            visible: true,
            locked: false,
        }
    }

    /// Supplies child nodes.
    pub fn children(mut self, children: Vec<Self>) -> Self {
        self.children = children;
        self
    }

    /// Sets host-provided visibility.
    pub const fn visible(mut self, visible: bool) -> Self {
        self.visible = visible;
        self
    }

    /// Sets host-provided lock state.
    pub const fn locked(mut self, locked: bool) -> Self {
        self.locked = locked;
        self
    }
}

/// Selection behavior requested by a layer-row activation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LayersPanelSelectionMode {
    Replace,
    Toggle,
    Range,
}

/// Placement requested when a layer row is dropped onto another row.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LayersPanelDropPosition {
    /// Insert the dragged node immediately before the target node.
    Before,
    /// Reparent the dragged node as the last child of the target container.
    Inside,
    /// Insert the dragged node immediately after the target node.
    After,
}

/// Figma-like context actions exposed by a layer menu.
///
/// Actions with a submenu in Figma are still emitted as typed intents. A host
/// can open a deeper application-native picker for them.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum LayersPanelContextAction {
    Copy,
    PasteToReplace,
    CopyPasteAs,
    SendToFigmaMake,
    FindSimilarDesigns,
    AddMotion,
    MoveToPage,
    BringToFront,
    SendToBack,
    ConvertToFrame,
    ConvertToSection,
    RemoveFrame,
    GroupSelection,
    FrameSelection,
    Ungroup,
    Rename,
    RenameLayers,
    Flatten,
    OutlineStroke,
    UseAsMask,
    SetAsThumbnail,
    EditText,
    CropImage,
    ReplaceMedia,
    AddAutoLayout,
    MoreLayoutOptions,
    CreateComponent,
    GoToMainComponent,
    DetachInstance,
    ResetInstance,
    Plugins,
    Widgets,
    ShowHide,
    LockUnlock,
    FlipHorizontal,
    FlipVertical,
}

impl LayersPanelContextAction {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Copy => "Copy",
            Self::PasteToReplace => "Paste to replace",
            Self::CopyPasteAs => "Copy/Paste as",
            Self::SendToFigmaMake => "Send to Figma Make",
            Self::FindSimilarDesigns => "Find similar designs",
            Self::AddMotion => "Add motion",
            Self::MoveToPage => "Move to page",
            Self::BringToFront => "Bring to front",
            Self::SendToBack => "Send to back",
            Self::ConvertToFrame => "Convert to frame",
            Self::ConvertToSection => "Convert to section",
            Self::RemoveFrame => "Remove frame",
            Self::GroupSelection => "Group selection",
            Self::FrameSelection => "Frame selection",
            Self::Ungroup => "Ungroup",
            Self::Rename => "Rename",
            Self::RenameLayers => "Rename layers",
            Self::Flatten => "Flatten",
            Self::OutlineStroke => "Outline stroke",
            Self::UseAsMask => "Use as mask",
            Self::SetAsThumbnail => "Set as thumbnail",
            Self::EditText => "Edit text",
            Self::CropImage => "Crop image",
            Self::ReplaceMedia => "Replace media",
            Self::AddAutoLayout => "Add auto layout",
            Self::MoreLayoutOptions => "More layout options",
            Self::CreateComponent => "Create component",
            Self::GoToMainComponent => "Main component",
            Self::DetachInstance => "Detach instance",
            Self::ResetInstance => "Reset all overrides",
            Self::Plugins => "Plugins",
            Self::Widgets => "Widgets",
            Self::ShowHide => "Show/Hide",
            Self::LockUnlock => "Lock/Unlock",
            Self::FlipHorizontal => "Flip horizontal",
            Self::FlipVertical => "Flip vertical",
        }
    }

    pub(crate) const fn shortcut(self) -> Option<&'static str> {
        match self {
            Self::Copy => Some("⌘C"),
            Self::PasteToReplace => Some("⇧⌘R"),
            Self::GroupSelection => Some("⌘G"),
            Self::FrameSelection => Some("⌥⌘G"),
            Self::Ungroup => Some("⇧⌘G"),
            Self::Rename => Some("⌘R"),
            Self::Flatten => Some("⌥⇧⌘F"),
            Self::OutlineStroke => Some("⌥⇧⌘O"),
            Self::UseAsMask => Some("⌃⌘M"),
            Self::AddAutoLayout => Some("⇧A"),
            Self::CreateComponent => Some("⌥⌘K"),
            Self::ShowHide => Some("⇧⌘H"),
            Self::LockUnlock => Some("⇧⌘L"),
            Self::FlipHorizontal => Some("⇧H"),
            Self::FlipVertical => Some("⇧V"),
            _ => None,
        }
    }

    pub(crate) const fn has_submenu(self) -> bool {
        matches!(
            self,
            Self::CopyPasteAs
                | Self::AddMotion
                | Self::MoveToPage
                | Self::MoreLayoutOptions
                | Self::Plugins
                | Self::Widgets
        )
    }

    pub(crate) const fn selector_slug(self) -> &'static str {
        match self {
            Self::Copy => "copy",
            Self::PasteToReplace => "paste-to-replace",
            Self::CopyPasteAs => "copy-paste-as",
            Self::SendToFigmaMake => "send-to-figma-make",
            Self::FindSimilarDesigns => "find-similar-designs",
            Self::AddMotion => "add-motion",
            Self::MoveToPage => "move-to-page",
            Self::BringToFront => "bring-to-front",
            Self::SendToBack => "send-to-back",
            Self::ConvertToFrame => "convert-to-frame",
            Self::ConvertToSection => "convert-to-section",
            Self::RemoveFrame => "remove-frame",
            Self::GroupSelection => "group-selection",
            Self::FrameSelection => "frame-selection",
            Self::Ungroup => "ungroup",
            Self::Rename => "rename",
            Self::RenameLayers => "rename-layers",
            Self::Flatten => "flatten",
            Self::OutlineStroke => "outline-stroke",
            Self::UseAsMask => "use-as-mask",
            Self::SetAsThumbnail => "set-as-thumbnail",
            Self::EditText => "edit-text",
            Self::CropImage => "crop-image",
            Self::ReplaceMedia => "replace-media",
            Self::AddAutoLayout => "add-auto-layout",
            Self::MoreLayoutOptions => "more-layout-options",
            Self::CreateComponent => "create-component",
            Self::GoToMainComponent => "main-component",
            Self::DetachInstance => "detach-instance",
            Self::ResetInstance => "reset-instance",
            Self::Plugins => "plugins",
            Self::Widgets => "widgets",
            Self::ShowHide => "show-hide",
            Self::LockUnlock => "lock-unlock",
            Self::FlipHorizontal => "flip-horizontal",
            Self::FlipVertical => "flip-vertical",
        }
    }
}

/// Domain-relevant intents emitted by [`super::LayersPanel`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LayersPanelAction {
    PanelExpansionChanged {
        expanded: bool,
    },
    SelectRequested {
        node_id: SharedString,
        mode: LayersPanelSelectionMode,
    },
    ExpansionChanged {
        node_id: SharedString,
        expanded: bool,
    },
    CollapseAllRequested,
    RenameRequested {
        node_id: SharedString,
        title: SharedString,
    },
    VisibilityChanged {
        node_id: SharedString,
        visible: bool,
    },
    LockChanged {
        node_id: SharedString,
        locked: bool,
    },
    MoveRequested {
        node_id: SharedString,
        target_node_id: SharedString,
        position: LayersPanelDropPosition,
    },
    ContextActionRequested {
        node_id: SharedString,
        action: LayersPanelContextAction,
    },
}
