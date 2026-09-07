use gpui::SharedString;

use super::*;

/// Paint collection containing one occurrence in a Selection colors aggregate.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DesignSelectionPaintCollection {
    Fill,
    Stroke,
}

impl DesignSelectionPaintCollection {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Fill => "Fill",
            Self::Stroke => "Stroke",
        }
    }
}

/// Stable identity for one underlying use of an aggregate selection paint.
///
/// `paint_index` and `gradient_stop_index` are compatibility fallbacks only;
/// hosts should preserve `paint_id` and `gradient_stop_id` across reorders.
/// Canonical getSelectionColors-like rows target the complete Solid/Gradient
/// paint and leave the gradient-stop fields empty. The optional stop identity
/// is retained for older leaf-color adapters.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DesignSelectionPaintReference {
    pub node_id: SharedString,
    pub collection: DesignSelectionPaintCollection,
    pub paint_id: SharedString,
    pub paint_index: usize,
    pub gradient_stop_id: Option<SharedString>,
    pub gradient_stop_index: Option<usize>,
}

impl DesignSelectionPaintReference {
    pub fn paint(
        node_id: impl Into<SharedString>,
        collection: DesignSelectionPaintCollection,
        paint_id: impl Into<SharedString>,
        paint_index: usize,
    ) -> Self {
        Self {
            node_id: node_id.into(),
            collection,
            paint_id: paint_id.into(),
            paint_index,
            gradient_stop_id: None,
            gradient_stop_index: None,
        }
    }

    pub fn gradient_stop(mut self, stop_id: impl Into<SharedString>, stop_index: usize) -> Self {
        self.gradient_stop_id = Some(stop_id.into());
        self.gradient_stop_index = Some(stop_index);
        self
    }
}

/// One deduplicated Solid/Gradient paint row across a multiple selection.
///
/// `paint` retains the representative Figma `Paint` rather than projecting
/// the row down to RGBA. `color` is a compatibility projection used by older
/// color-only hosts. `style_paints` is the complete ordered Fill/Stroke
/// collection used by whole Paint-style creation, while `style_binding` and
/// `binding` deliberately model the collection-level Paint style and the
/// Solid paint's Color-variable binding as separate identities. Gradient
/// rows do not synthesize one aggregate Color-variable binding from their
/// independently bindable stops.
#[derive(Clone, Debug, PartialEq)]
pub struct DesignSelectionColor {
    /// Stable opaque aggregate-row identity.
    pub id: SharedString,
    /// Representative host paint for this aggregate row.
    pub paint: DesignPaint,
    pub color: DesignColor,
    /// Number shown by Figma when the color occurs more than once.
    pub occurrence_count: u32,
    /// Stable references the host will edit when accepting an edit.
    pub paint_references: Vec<DesignSelectionPaintReference>,
    /// Complete ordered collection snapshot for whole Paint-style operations.
    ///
    /// Empty means the compatibility adapter did not supply collection
    /// context, so the panel must not offer Paint-style creation.
    pub style_paints: Vec<DesignPaint>,
    /// Uniform collection-level `fillStyleId`/`strokeStyleId`, if every
    /// occurrence represented by this row shares the same binding.
    pub style_binding: Option<DesignPaintStyleBinding>,
    /// Uniform Solid-color variable binding.
    pub binding: Option<DesignPaintBinding>,
    pub read_only: bool,
}

impl DesignSelectionColor {
    pub fn new(
        id: impl Into<SharedString>,
        color: DesignColor,
        paint_references: impl IntoIterator<Item = DesignSelectionPaintReference>,
    ) -> Self {
        let paint_references = paint_references.into_iter().collect::<Vec<_>>();
        Self {
            id: id.into(),
            paint: DesignPaint::solid(color),
            color,
            occurrence_count: u32::try_from(paint_references.len()).unwrap_or(u32::MAX),
            paint_references,
            style_paints: Vec::new(),
            style_binding: None,
            binding: None,
            read_only: false,
        }
    }

    /// Sets a host-computed count for occurrences that cannot all be expanded
    /// into references at the inspector boundary.
    pub fn with_occurrence_count(mut self, occurrence_count: u32) -> Self {
        self.occurrence_count = occurrence_count;
        self
    }

    pub fn with_binding(mut self, binding: DesignPaintBinding) -> Self {
        self.binding = Some(binding);
        self
    }

    /// Retains the exact representative paint for getSelectionColors-like
    /// aggregation. Hosts should omit occurrence-specific IDs from their
    /// deduplication key while preserving all paint semantics.
    pub fn with_paint(mut self, paint: DesignPaint) -> Self {
        self.paint = paint;
        self
    }

    /// Supplies the complete ordered Paint-style context independently of the
    /// selected color leaf.
    pub fn with_style_context(
        mut self,
        paints: impl IntoIterator<Item = DesignPaint>,
        binding: Option<DesignPaintStyleBinding>,
    ) -> Self {
        self.style_paints = paints.into_iter().collect();
        self.style_binding = binding;
        self
    }

    pub const fn read_only(mut self, read_only: bool) -> Self {
        self.read_only = read_only;
        self
    }
}

/// Canonical host-owned aggregate behind Figma's Selection colors section.
///
/// Despite the historical type name, each canonical row represents one full
/// normal Solid/Gradient paint, not one row per gradient stop.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DesignSelectionColors {
    pub colors: Vec<DesignSelectionColor>,
}

impl DesignSelectionColors {
    pub fn new(colors: impl IntoIterator<Item = DesignSelectionColor>) -> Self {
        Self {
            colors: colors.into_iter().collect(),
        }
    }

    pub fn total_occurrences(&self) -> u32 {
        self.colors.iter().fold(0_u32, |total, color| {
            total.saturating_add(color.occurrence_count)
        })
    }

    pub fn color(&self, id: &str) -> Option<&DesignSelectionColor> {
        self.colors.iter().find(|color| color.id.as_ref() == id)
    }
}

/// Scope of a text-property intent.
///
/// Text edit mode targets the host's active character range; object mode
/// targets the complete text layer.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DesignTypographyTarget {
    WholeLayer,
    /// Compatibility target for hosts that do not yet identify text ranges.
    SelectedTextRange,
    /// Exact host-owned text-range revision.
    ///
    /// Hosts increment or otherwise replace this opaque revision whenever the
    /// active character range changes, even when the selected text node and
    /// Text edit mode remain unchanged.
    SelectedTextRangeRevision(u64),
}

impl DesignTypographyTarget {
    pub const fn is_selected_text_range(self) -> bool {
        matches!(
            self,
            Self::SelectedTextRange | Self::SelectedTextRangeRevision(_)
        )
    }

    pub const fn selected_text_range_revision(self) -> Option<u64> {
        match self {
            Self::SelectedTextRangeRevision(revision) => Some(revision),
            Self::WholeLayer | Self::SelectedTextRange => None,
        }
    }
}

/// Scope of a Fill/Stroke paint intent.
///
/// Fill controls follow Figma's text-edit semantics: while characters are
/// selected on a Text or TextPath node, the host applies the operation to that
/// exact character range. Stroke controls always target the complete layer.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DesignPaintTarget {
    WholeLayer,
    /// Compatibility target for hosts that do not identify text ranges.
    SelectedTextRange,
    /// Exact host-owned text-range revision captured when the interaction
    /// starts.
    ///
    /// Hosts must reject this target when it no longer matches their active
    /// character range, even when the selected node is unchanged.
    SelectedTextRangeRevision(u64),
}

impl DesignPaintTarget {
    pub const fn is_selected_text_range(self) -> bool {
        matches!(
            self,
            Self::SelectedTextRange | Self::SelectedTextRangeRevision(_)
        )
    }

    pub const fn selected_text_range_revision(self) -> Option<u64> {
        match self {
            Self::SelectedTextRangeRevision(revision) => Some(revision),
            Self::WholeLayer | Self::SelectedTextRange => None,
        }
    }
}

/// Stable host target for commands that may affect a page or many selected
/// nodes. Direct target-aware commands carry this value themselves; ordinary
/// multi-selection leaves carry it through
/// [`DesignPanelAction::TargetedNodeActionRequested`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DesignPanelTarget {
    Page { page_id: SharedString },
    Nodes { node_ids: Vec<SharedString> },
}

/// One leaf operation exposed by the selected-node header.
///
/// Header controls are intentionally commands rather than document mutations:
/// the host decides how a command changes selection, edit mode, or document
/// data and then echoes a fresh inspection context.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum DesignSelectionHeaderCommand {
    SelectMatchingLayers,
    CreateLink,
    ApplyTextContentVariable,
    CreateComponent,
    UseAsMask,
    Boolean(DesignBooleanOperation),
    Flatten,
    EditObject,
    TitleMenuItem { item_id: SharedString },
    HostDefined { command_id: SharedString },
}

impl DesignSelectionHeaderCommand {
    /// Default permission class for this command.
    ///
    /// Host-authored controls and menu items can override this default with
    /// [`DesignSelectionHeaderControl::with_access`] or
    /// [`DesignSelectionHeaderMenuItem::with_access`]. Built-in constructors
    /// retain these semantics.
    pub const fn default_access(&self) -> DesignSelectionHeaderCommandAccess {
        match self {
            Self::SelectMatchingLayers => DesignSelectionHeaderCommandAccess::ViewerSafe,
            Self::CreateLink
            | Self::ApplyTextContentVariable
            | Self::CreateComponent
            | Self::UseAsMask
            | Self::Boolean(_)
            | Self::Flatten
            | Self::EditObject
            | Self::TitleMenuItem { .. }
            | Self::HostDefined { .. } => DesignSelectionHeaderCommandAccess::EditRequired,
        }
    }

    /// Whether the command can change document data or enter an editing mode.
    ///
    /// Selecting matching layers is the only built-in command that remains
    /// available to a viewer. Hosts can still disable it explicitly.
    pub const fn requires_edit(&self) -> bool {
        self.default_access().requires_edit()
    }
}

/// Permission class for one selected-node header command leaf.
///
/// The default remains edit-required. Hosts should opt a custom direct
/// control, title-menu item, or menu leaf into `ViewerSafe` only when invoking
/// it cannot mutate the document or enter an editing mode.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum DesignSelectionHeaderCommandAccess {
    ViewerSafe,
    #[default]
    EditRequired,
}

impl DesignSelectionHeaderCommandAccess {
    pub const fn requires_edit(self) -> bool {
        matches!(self, Self::EditRequired)
    }
}

/// Visual/semantic role of an ordered selected-node header control.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DesignSelectionHeaderControlKind {
    SelectMatchingLayers,
    CreateLink,
    ApplyTextContentVariable,
    CreateComponent,
    UseAsMask,
    BooleanFlattenMenu,
    EditObject,
    HostDefined,
}

impl DesignSelectionHeaderControlKind {
    pub const fn label(self) -> &'static str {
        match self {
            Self::SelectMatchingLayers => "Select matching layers",
            Self::CreateLink => "Create link",
            Self::ApplyTextContentVariable => "Apply variable to text content",
            Self::CreateComponent => "Create component",
            Self::UseAsMask => "Use as mask",
            Self::BooleanFlattenMenu => "Boolean operations and flatten",
            Self::EditObject => "Edit object",
            Self::HostDefined => "More action",
        }
    }

    pub const fn is_menu(self) -> bool {
        matches!(self, Self::BooleanFlattenMenu)
    }
}

/// One enabled or disabled leaf in a selected-node header menu.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DesignSelectionHeaderMenuItem {
    pub id: SharedString,
    pub label: SharedString,
    pub command: DesignSelectionHeaderCommand,
    pub access: DesignSelectionHeaderCommandAccess,
    pub enabled: bool,
    pub disabled_reason: Option<SharedString>,
}

impl DesignSelectionHeaderMenuItem {
    pub fn new(
        id: impl Into<SharedString>,
        label: impl Into<SharedString>,
        command: DesignSelectionHeaderCommand,
    ) -> Self {
        let access = command.default_access();
        Self {
            id: id.into(),
            label: label.into(),
            command,
            access,
            enabled: true,
            disabled_reason: None,
        }
    }

    pub const fn with_access(mut self, access: DesignSelectionHeaderCommandAccess) -> Self {
        self.access = access;
        self
    }

    pub const fn viewer_safe(self) -> Self {
        self.with_access(DesignSelectionHeaderCommandAccess::ViewerSafe)
    }

    pub const fn edit_required(self) -> Self {
        self.with_access(DesignSelectionHeaderCommandAccess::EditRequired)
    }

    /// Access used by the panel after preserving built-in command semantics.
    pub const fn effective_access(&self) -> DesignSelectionHeaderCommandAccess {
        match &self.command {
            DesignSelectionHeaderCommand::TitleMenuItem { .. }
            | DesignSelectionHeaderCommand::HostDefined { .. } => self.access,
            _ => self.command.default_access(),
        }
    }

    pub fn disabled(mut self, reason: impl Into<SharedString>) -> Self {
        self.enabled = false;
        self.disabled_reason = Some(reason.into());
        self
    }
}

/// Ordered menu presented by the selected-node title.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DesignSelectionHeaderMenu {
    pub id: SharedString,
    pub items: Vec<DesignSelectionHeaderMenuItem>,
}

impl DesignSelectionHeaderMenu {
    pub fn new(
        id: impl Into<SharedString>,
        items: impl IntoIterator<Item = DesignSelectionHeaderMenuItem>,
    ) -> Self {
        Self {
            id: id.into(),
            items: items.into_iter().collect(),
        }
    }
}

/// Host-controlled icon presentation for one selected-node header control.
///
/// `Default` preserves the icon implied by the built-in control kind. The
/// remaining variants let an integration reproduce contextual or plugin
/// controls without pretending they are one of the built-in commands.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
pub enum DesignSelectionHeaderControlIcon {
    #[default]
    Default,
    Ellipsis,
    Plus,
    Minus,
    Check,
    Search,
    Settings,
    File,
    Inspector,
    Layout,
    /// A compact host-supplied glyph such as a plugin monogram.
    Glyph(SharedString),
}

/// One ordered primary or overflow control in the selected-node header.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DesignSelectionHeaderControl {
    pub id: SharedString,
    pub kind: DesignSelectionHeaderControlKind,
    pub icon: DesignSelectionHeaderControlIcon,
    pub tooltip: SharedString,
    /// Permission class for this control's direct command. Menu-bearing
    /// controls use the access class on each menu item instead.
    pub access: DesignSelectionHeaderCommandAccess,
    pub enabled: bool,
    pub disabled_reason: Option<SharedString>,
    /// Leaf items for menu-bearing controls. Direct controls leave this empty.
    pub menu_items: Vec<DesignSelectionHeaderMenuItem>,
}

impl DesignSelectionHeaderControl {
    pub fn new(id: impl Into<SharedString>, kind: DesignSelectionHeaderControlKind) -> Self {
        let access = match kind {
            DesignSelectionHeaderControlKind::SelectMatchingLayers => {
                DesignSelectionHeaderCommandAccess::ViewerSafe
            }
            DesignSelectionHeaderControlKind::CreateLink
            | DesignSelectionHeaderControlKind::ApplyTextContentVariable
            | DesignSelectionHeaderControlKind::CreateComponent
            | DesignSelectionHeaderControlKind::UseAsMask
            | DesignSelectionHeaderControlKind::BooleanFlattenMenu
            | DesignSelectionHeaderControlKind::EditObject
            | DesignSelectionHeaderControlKind::HostDefined => {
                DesignSelectionHeaderCommandAccess::EditRequired
            }
        };
        Self {
            id: id.into(),
            kind,
            icon: DesignSelectionHeaderControlIcon::Default,
            tooltip: kind.label().into(),
            access,
            enabled: true,
            disabled_reason: None,
            menu_items: Vec::new(),
        }
    }

    pub fn with_tooltip(mut self, tooltip: impl Into<SharedString>) -> Self {
        self.tooltip = tooltip.into();
        self
    }

    pub fn with_icon(mut self, icon: DesignSelectionHeaderControlIcon) -> Self {
        self.icon = icon;
        self
    }

    pub const fn with_access(mut self, access: DesignSelectionHeaderCommandAccess) -> Self {
        self.access = access;
        self
    }

    pub const fn viewer_safe(self) -> Self {
        self.with_access(DesignSelectionHeaderCommandAccess::ViewerSafe)
    }

    pub const fn edit_required(self) -> Self {
        self.with_access(DesignSelectionHeaderCommandAccess::EditRequired)
    }

    pub fn with_menu_items(
        mut self,
        items: impl IntoIterator<Item = DesignSelectionHeaderMenuItem>,
    ) -> Self {
        self.menu_items = items.into_iter().collect();
        self
    }

    pub fn disabled(mut self, reason: impl Into<SharedString>) -> Self {
        self.enabled = false;
        self.disabled_reason = Some(reason.into());
        self
    }

    pub fn command(&self) -> Option<DesignSelectionHeaderCommand> {
        if self.is_menu() {
            return None;
        }
        match self.kind {
            DesignSelectionHeaderControlKind::SelectMatchingLayers => {
                Some(DesignSelectionHeaderCommand::SelectMatchingLayers)
            }
            DesignSelectionHeaderControlKind::CreateLink => {
                Some(DesignSelectionHeaderCommand::CreateLink)
            }
            DesignSelectionHeaderControlKind::ApplyTextContentVariable => {
                Some(DesignSelectionHeaderCommand::ApplyTextContentVariable)
            }
            DesignSelectionHeaderControlKind::CreateComponent => {
                Some(DesignSelectionHeaderCommand::CreateComponent)
            }
            DesignSelectionHeaderControlKind::UseAsMask => {
                Some(DesignSelectionHeaderCommand::UseAsMask)
            }
            DesignSelectionHeaderControlKind::BooleanFlattenMenu => None,
            DesignSelectionHeaderControlKind::EditObject => {
                Some(DesignSelectionHeaderCommand::EditObject)
            }
            DesignSelectionHeaderControlKind::HostDefined => {
                Some(DesignSelectionHeaderCommand::HostDefined {
                    command_id: self.id.clone(),
                })
            }
        }
    }

    /// Whether this control opens a menu instead of emitting a direct command.
    ///
    /// Built-in Boolean/Flatten controls are always menus. Any host-defined
    /// control becomes a menu as soon as the host supplies menu items.
    pub fn is_menu(&self) -> bool {
        self.kind.is_menu() || !self.menu_items.is_empty()
    }

    /// Access used by the panel after preserving built-in control semantics.
    pub fn effective_access(&self) -> DesignSelectionHeaderCommandAccess {
        if self.kind == DesignSelectionHeaderControlKind::HostDefined {
            self.access
        } else {
            self.command().as_ref().map_or(
                DesignSelectionHeaderCommandAccess::EditRequired,
                |command| command.default_access(),
            )
        }
    }

    pub fn direct(kind: DesignSelectionHeaderControlKind) -> Self {
        let id = match kind {
            DesignSelectionHeaderControlKind::SelectMatchingLayers => "select-matching-layers",
            DesignSelectionHeaderControlKind::CreateLink => "create-link",
            DesignSelectionHeaderControlKind::ApplyTextContentVariable => {
                "apply-text-content-variable"
            }
            DesignSelectionHeaderControlKind::CreateComponent => "create-component",
            DesignSelectionHeaderControlKind::UseAsMask => "use-as-mask",
            DesignSelectionHeaderControlKind::BooleanFlattenMenu => "boolean-flatten",
            DesignSelectionHeaderControlKind::EditObject => "edit-object",
            DesignSelectionHeaderControlKind::HostDefined => "host-defined",
        };
        Self::new(id, kind)
    }

    pub fn boolean_flatten_menu() -> Self {
        Self::direct(DesignSelectionHeaderControlKind::BooleanFlattenMenu).with_menu_items(
            DesignBooleanOperation::ALL
                .into_iter()
                .map(|operation| {
                    DesignSelectionHeaderMenuItem::new(
                        format!(
                            "boolean-{}",
                            operation.label().to_ascii_lowercase().replace(' ', "-")
                        ),
                        operation.label(),
                        DesignSelectionHeaderCommand::Boolean(operation),
                    )
                })
                .chain([DesignSelectionHeaderMenuItem::new(
                    "flatten",
                    "Flatten",
                    DesignSelectionHeaderCommand::Flatten,
                )]),
        )
    }
}

/// Complete host-owned selected-node header presentation.
///
/// The vectors retain host order. Supplying this value is authoritative:
/// the panel does not add kind-derived controls or move controls between the
/// primary row and the transient More menu.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DesignSelectionHeaderViewData {
    pub title: SharedString,
    pub title_menu: Option<DesignSelectionHeaderMenu>,
    pub primary_controls: Vec<DesignSelectionHeaderControl>,
    pub overflow_controls: Vec<DesignSelectionHeaderControl>,
}

impl DesignSelectionHeaderViewData {
    pub fn new(
        title: impl Into<SharedString>,
        primary_controls: impl IntoIterator<Item = DesignSelectionHeaderControl>,
    ) -> Self {
        Self {
            title: title.into(),
            title_menu: None,
            primary_controls: primary_controls.into_iter().collect(),
            overflow_controls: Vec::new(),
        }
    }

    pub fn with_title_menu(mut self, menu: DesignSelectionHeaderMenu) -> Self {
        self.title_menu = Some(menu);
        self
    }

    pub fn with_overflow_controls(
        mut self,
        controls: impl IntoIterator<Item = DesignSelectionHeaderControl>,
    ) -> Self {
        self.overflow_controls = controls.into_iter().collect();
        self
    }

    /// Conservative compatibility header for an exact multiple selection.
    ///
    /// A multiple selection is not represented by any one member's node kind.
    /// In particular, borrowing the first member's preset can expose a Text
    /// content-variable command or a Frame title menu for an unrelated mixed
    /// selection. Hosts should supply exact target-bound data whenever they
    /// can resolve more contextual commands. This baseline intentionally keeps
    /// only Create component, which applies to arbitrary selected layers.
    pub fn for_multiple_selection(count: usize) -> Self {
        Self::new(
            format!("{count} layers"),
            [DesignSelectionHeaderControl::direct(
                DesignSelectionHeaderControlKind::CreateComponent,
            )],
        )
    }

    /// A representative fallback for stories and incremental integrations.
    ///
    /// Real Figma headers are contextual (for example Select matching layers
    /// depends on the current page). Hosts should supply exact view data when
    /// sibling availability, plugins, or document state affect the matrix.
    pub fn for_node_kind(kind: DesignPanelNodeKind) -> Self {
        use DesignSelectionHeaderControlKind as Kind;

        let direct = DesignSelectionHeaderControl::direct;
        let boolean = DesignSelectionHeaderControl::boolean_flatten_menu;
        let title = kind.label();
        match kind {
            DesignPanelNodeKind::Text | DesignPanelNodeKind::TextPath => Self::new(
                title,
                [
                    direct(Kind::SelectMatchingLayers),
                    direct(Kind::CreateLink),
                    direct(Kind::ApplyTextContentVariable),
                    direct(Kind::CreateComponent),
                ],
            )
            .with_overflow_controls([direct(Kind::UseAsMask), direct(Kind::EditObject)]),
            DesignPanelNodeKind::Frame => Self::new(
                title,
                [
                    direct(Kind::SelectMatchingLayers),
                    direct(Kind::CreateComponent),
                    direct(Kind::UseAsMask),
                    boolean(),
                ],
            )
            .with_title_menu(DesignSelectionHeaderMenu::new(
                "frame-type",
                ["frame", "group", "section"].into_iter().map(|id| {
                    let label = match id {
                        "frame" => "Frame",
                        "group" => "Group",
                        "section" => "Section",
                        _ => unreachable!(),
                    };
                    DesignSelectionHeaderMenuItem::new(
                        id,
                        label,
                        DesignSelectionHeaderCommand::TitleMenuItem { item_id: id.into() },
                    )
                }),
            )),
            DesignPanelNodeKind::Arrow | DesignPanelNodeKind::Line => Self::new(
                title,
                [
                    direct(Kind::SelectMatchingLayers),
                    direct(Kind::CreateComponent),
                    direct(Kind::UseAsMask),
                    boolean(),
                ],
            )
            .with_overflow_controls([direct(Kind::EditObject)]),
            DesignPanelNodeKind::Ellipse => Self::new(
                title,
                [
                    direct(Kind::CreateComponent),
                    direct(Kind::UseAsMask),
                    boolean(),
                    direct(Kind::EditObject),
                ],
            ),
            DesignPanelNodeKind::Widget => Self::new(title, []),
            _ => Self::new(
                title,
                [
                    direct(Kind::SelectMatchingLayers),
                    direct(Kind::CreateComponent),
                    direct(Kind::UseAsMask),
                    boolean(),
                    direct(Kind::EditObject),
                ],
            ),
        }
    }
}
