use std::{error::Error, fmt};

use gpui::SharedString;

use super::model::{DesignPanelNode, DesignPanelValue};

/// Cardinality of the host-controlled Design-panel selection.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DesignPanelSelectionKind {
    None,
    Single,
    Multiple,
}

/// A selection containing at least two items.
///
/// Keeping this invariant in a dedicated type prevents a `Multiple` selection
/// from accidentally representing zero or one item.
#[derive(Clone, Debug, PartialEq)]
pub struct DesignPanelMultipleSelection<T = DesignPanelNode> {
    items: Vec<T>,
}

impl<T> DesignPanelMultipleSelection<T> {
    pub fn new(first: T, second: T) -> Self {
        Self {
            items: vec![first, second],
        }
    }

    pub fn with_remaining(first: T, second: T, remaining: impl IntoIterator<Item = T>) -> Self {
        let mut items = vec![first, second];
        items.extend(remaining);
        Self { items }
    }

    pub fn try_from_items(items: Vec<T>) -> Result<Self, DesignPanelSelectionCardinalityError> {
        if items.len() < 2 {
            return Err(DesignPanelSelectionCardinalityError {
                actual: items.len(),
            });
        }

        Ok(Self { items })
    }

    pub fn items(&self) -> &[T] {
        &self.items
    }

    pub fn into_items(self) -> Vec<T> {
        self.items
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub const fn is_empty(&self) -> bool {
        false
    }
}

impl<T> TryFrom<Vec<T>> for DesignPanelMultipleSelection<T> {
    type Error = DesignPanelSelectionCardinalityError;

    fn try_from(items: Vec<T>) -> Result<Self, Self::Error> {
        Self::try_from_items(items)
    }
}

/// Error returned when fewer than two items are used for a multiple selection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DesignPanelSelectionCardinalityError {
    actual: usize,
}

impl DesignPanelSelectionCardinalityError {
    pub const fn actual(self) -> usize {
        self.actual
    }
}

impl fmt::Display for DesignPanelSelectionCardinalityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "a multiple selection requires at least 2 items, got {}",
            self.actual
        )
    }
}

impl Error for DesignPanelSelectionCardinalityError {}

/// Host-controlled Design-panel selection.
///
/// `from_items` is useful at adapter boundaries because it canonicalizes a
/// dynamic list into `None`, `Single`, or a valid `Multiple` selection.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum DesignPanelSelection<T = DesignPanelNode> {
    #[default]
    None,
    Single(T),
    Multiple(DesignPanelMultipleSelection<T>),
}

impl<T> DesignPanelSelection<T> {
    pub fn from_items(mut items: Vec<T>) -> Self {
        match items.len() {
            0 => Self::None,
            1 => Self::Single(
                items
                    .pop()
                    .expect("a one-item selection must contain one item"),
            ),
            _ => Self::Multiple(DesignPanelMultipleSelection { items }),
        }
    }

    pub const fn kind(&self) -> DesignPanelSelectionKind {
        match self {
            Self::None => DesignPanelSelectionKind::None,
            Self::Single(_) => DesignPanelSelectionKind::Single,
            Self::Multiple(_) => DesignPanelSelectionKind::Multiple,
        }
    }

    pub fn items(&self) -> &[T] {
        match self {
            Self::None => &[],
            Self::Single(item) => std::slice::from_ref(item),
            Self::Multiple(selection) => selection.items(),
        }
    }

    pub fn len(&self) -> usize {
        self.items().len()
    }

    pub fn is_empty(&self) -> bool {
        matches!(self, Self::None)
    }
}

impl<T> From<T> for DesignPanelSelection<T> {
    fn from(item: T) -> Self {
        Self::Single(item)
    }
}

impl<T> From<DesignPanelMultipleSelection<T>> for DesignPanelSelection<T> {
    fn from(selection: DesignPanelMultipleSelection<T>) -> Self {
        Self::Multiple(selection)
    }
}

/// Direction of the selected item's parent auto-layout container.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DesignPanelAutoLayoutDirection {
    Horizontal,
    Vertical,
    Grid,
}

/// Whether a horizontal parent auto-layout container wraps its children.
///
/// Figma does not expose wrapping for vertical or grid flows. The
/// [`DesignPanelParentLayout::auto_layout`] constructor canonicalizes those
/// combinations to [`DesignPanelAutoLayoutWrap::NoWrap`].
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DesignPanelAutoLayoutWrap {
    NoWrap,
    Wrap,
}

/// How a selected child participates in its parent auto-layout container.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DesignPanelAutoLayoutParticipation {
    InFlow,
    Ignored,
}

/// Aggregate layout context supplied by the selected item's parent.
///
/// `Mixed` represents a multi-selection whose items do not share one parent
/// layout context.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DesignPanelParentLayout {
    Canvas,
    Freeform,
    AutoLayout {
        direction: DesignPanelAutoLayoutDirection,
        wrap: DesignPanelAutoLayoutWrap,
        participation: DesignPanelAutoLayoutParticipation,
    },
    Mixed,
}

impl DesignPanelParentLayout {
    pub const fn auto_layout(
        direction: DesignPanelAutoLayoutDirection,
        wrap: DesignPanelAutoLayoutWrap,
        participation: DesignPanelAutoLayoutParticipation,
    ) -> Self {
        let wrap = match direction {
            DesignPanelAutoLayoutDirection::Horizontal => wrap,
            DesignPanelAutoLayoutDirection::Vertical | DesignPanelAutoLayoutDirection::Grid => {
                DesignPanelAutoLayoutWrap::NoWrap
            }
        };
        Self::AutoLayout {
            direction,
            wrap,
            participation,
        }
    }

    pub const fn is_auto_layout(self) -> bool {
        matches!(self, Self::AutoLayout { .. })
    }

    pub const fn participates_in_auto_layout(self) -> bool {
        matches!(
            self,
            Self::AutoLayout {
                participation: DesignPanelAutoLayoutParticipation::InFlow,
                ..
            }
        )
    }

    pub const fn is_ignored_by_auto_layout(self) -> bool {
        matches!(
            self,
            Self::AutoLayout {
                participation: DesignPanelAutoLayoutParticipation::Ignored,
                ..
            }
        )
    }

    pub const fn auto_layout_direction(self) -> Option<DesignPanelAutoLayoutDirection> {
        match self {
            Self::AutoLayout { direction, .. } => Some(direction),
            Self::Canvas | Self::Freeform | Self::Mixed => None,
        }
    }
}

/// Broad access level for Design-panel operations.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DesignPanelAccessMode {
    Edit,
    View,
}

/// Host-provided permissions that affect inspector controls.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct DesignPanelPermissions {
    access_mode: DesignPanelAccessMode,
    can_copy: bool,
    can_export: bool,
}

impl DesignPanelPermissions {
    pub const fn new(access_mode: DesignPanelAccessMode, can_copy: bool, can_export: bool) -> Self {
        Self {
            access_mode,
            can_copy,
            can_export,
        }
    }

    pub const fn editor() -> Self {
        Self::new(DesignPanelAccessMode::Edit, true, true)
    }

    pub const fn viewer() -> Self {
        Self::new(DesignPanelAccessMode::View, true, true)
    }

    pub const fn restricted_viewer() -> Self {
        Self::new(DesignPanelAccessMode::View, false, false)
    }

    pub const fn access_mode(self) -> DesignPanelAccessMode {
        self.access_mode
    }

    pub const fn can_edit(self) -> bool {
        matches!(self.access_mode, DesignPanelAccessMode::Edit)
    }

    pub const fn can_copy(self) -> bool {
        self.can_copy
    }

    pub const fn can_export(self) -> bool {
        self.can_export
    }
}

impl Default for DesignPanelPermissions {
    fn default() -> Self {
        Self::viewer()
    }
}

/// Active canvas or sub-selection editing context.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DesignPanelEditMode {
    Canvas,
    Object,
    Text,
    Vector,
}

/// Complete host-provided context used to resolve Design-panel controls.
#[derive(Clone, Debug, PartialEq)]
pub struct DesignPanelInspectionContext<T = DesignPanelNode> {
    selection: DesignPanelSelection<T>,
    parent_layout: DesignPanelParentLayout,
    permissions: DesignPanelPermissions,
    edit_mode: DesignPanelEditMode,
    text_range_revision: Option<u64>,
}

impl<T> DesignPanelInspectionContext<T> {
    pub fn try_new(
        selection: DesignPanelSelection<T>,
        parent_layout: DesignPanelParentLayout,
        permissions: DesignPanelPermissions,
        edit_mode: DesignPanelEditMode,
    ) -> Result<Self, DesignPanelContextError> {
        let selection_kind = selection.kind();
        let edit_mode_matches_selection = matches!(
            (selection_kind, edit_mode),
            (DesignPanelSelectionKind::None, DesignPanelEditMode::Canvas)
                | (
                    DesignPanelSelectionKind::Single,
                    DesignPanelEditMode::Object
                        | DesignPanelEditMode::Text
                        | DesignPanelEditMode::Vector
                )
                | (
                    DesignPanelSelectionKind::Multiple,
                    DesignPanelEditMode::Object
                )
        );

        if !edit_mode_matches_selection {
            return Err(DesignPanelContextError::EditModeSelectionMismatch {
                selection: selection_kind,
                edit_mode,
            });
        }

        if matches!(
            edit_mode,
            DesignPanelEditMode::Text | DesignPanelEditMode::Vector
        ) && !permissions.can_edit()
        {
            return Err(DesignPanelContextError::EditModeRequiresEditPermission { edit_mode });
        }

        if selection_kind == DesignPanelSelectionKind::None
            && parent_layout != DesignPanelParentLayout::Canvas
        {
            return Err(DesignPanelContextError::NoSelectionHasParentLayout { parent_layout });
        }

        Ok(Self {
            selection,
            parent_layout,
            permissions,
            edit_mode,
            text_range_revision: None,
        })
    }

    pub fn page(permissions: DesignPanelPermissions) -> Self {
        Self {
            selection: DesignPanelSelection::None,
            parent_layout: DesignPanelParentLayout::Canvas,
            permissions,
            edit_mode: DesignPanelEditMode::Canvas,
            text_range_revision: None,
        }
    }

    pub fn single(
        item: T,
        parent_layout: DesignPanelParentLayout,
        permissions: DesignPanelPermissions,
    ) -> Self {
        Self {
            selection: DesignPanelSelection::Single(item),
            parent_layout,
            permissions,
            edit_mode: DesignPanelEditMode::Object,
            text_range_revision: None,
        }
    }

    pub fn multiple(
        selection: DesignPanelMultipleSelection<T>,
        parent_layout: DesignPanelParentLayout,
        permissions: DesignPanelPermissions,
    ) -> Self {
        Self {
            selection: DesignPanelSelection::Multiple(selection),
            parent_layout,
            permissions,
            edit_mode: DesignPanelEditMode::Object,
            text_range_revision: None,
        }
    }

    pub fn with_edit_mode(
        self,
        edit_mode: DesignPanelEditMode,
    ) -> Result<Self, DesignPanelContextError> {
        let text_range_revision = (edit_mode == DesignPanelEditMode::Text)
            .then_some(self.text_range_revision)
            .flatten();
        let mut context = Self::try_new(
            self.selection,
            self.parent_layout,
            self.permissions,
            edit_mode,
        )?;
        context.text_range_revision = text_range_revision;
        Ok(context)
    }

    /// Supplies the exact host-owned character-range revision for Text mode.
    ///
    /// Replacing this revision makes the range a new interaction target even
    /// when the selected node remains unchanged.
    pub fn with_text_range_revision(
        mut self,
        revision: u64,
    ) -> Result<Self, DesignPanelContextError> {
        if self.edit_mode != DesignPanelEditMode::Text {
            return Err(
                DesignPanelContextError::TextRangeRevisionRequiresTextEditMode {
                    edit_mode: self.edit_mode,
                },
            );
        }
        self.text_range_revision = Some(revision);
        Ok(self)
    }

    pub const fn selection(&self) -> &DesignPanelSelection<T> {
        &self.selection
    }

    pub const fn parent_layout(&self) -> DesignPanelParentLayout {
        self.parent_layout
    }

    pub const fn permissions(&self) -> DesignPanelPermissions {
        self.permissions
    }

    pub const fn edit_mode(&self) -> DesignPanelEditMode {
        self.edit_mode
    }

    pub const fn text_range_revision(&self) -> Option<u64> {
        self.text_range_revision
    }

    pub fn property_is_editable<U>(&self, value: &DesignPanelPropertyValueState<U>) -> bool {
        self.permissions.can_edit() && !value.is_read_only()
    }
}

/// Invalid combination of selection, parent layout, permission, and edit mode.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DesignPanelContextError {
    EditModeSelectionMismatch {
        selection: DesignPanelSelectionKind,
        edit_mode: DesignPanelEditMode,
    },
    EditModeRequiresEditPermission {
        edit_mode: DesignPanelEditMode,
    },
    NoSelectionHasParentLayout {
        parent_layout: DesignPanelParentLayout,
    },
    TextRangeRevisionRequiresTextEditMode {
        edit_mode: DesignPanelEditMode,
    },
}

impl fmt::Display for DesignPanelContextError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EditModeSelectionMismatch {
                selection,
                edit_mode,
            } => write!(
                formatter,
                "{edit_mode:?} edit mode is incompatible with a {selection:?} selection"
            ),
            Self::EditModeRequiresEditPermission { edit_mode } => {
                write!(
                    formatter,
                    "{edit_mode:?} edit mode requires edit permission"
                )
            }
            Self::NoSelectionHasParentLayout { parent_layout } => write!(
                formatter,
                "a page context cannot have {parent_layout:?} parent layout"
            ),
            Self::TextRangeRevisionRequiresTextEditMode { edit_mode } => write!(
                formatter,
                "a selected-text-range revision requires Text edit mode, not {edit_mode:?}"
            ),
        }
    }
}

impl Error for DesignPanelContextError {}

/// Kind of external source bound to a Design-panel property.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DesignPanelBindingKind {
    Variable,
    Style,
}

/// A bound property and the value currently resolved from its source.
#[derive(Clone, Debug, PartialEq)]
pub struct DesignPanelPropertyBinding<T = DesignPanelValue> {
    id: SharedString,
    name: SharedString,
    kind: DesignPanelBindingKind,
    resolved: T,
}

impl<T> DesignPanelPropertyBinding<T> {
    pub fn new(
        id: impl Into<SharedString>,
        name: impl Into<SharedString>,
        kind: DesignPanelBindingKind,
        resolved: T,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            kind,
            resolved,
        }
    }

    pub const fn id(&self) -> &SharedString {
        &self.id
    }

    pub const fn name(&self) -> &SharedString {
        &self.name
    }

    pub const fn kind(&self) -> DesignPanelBindingKind {
        self.kind
    }

    pub const fn resolved(&self) -> &T {
        &self.resolved
    }

    pub fn into_resolved(self) -> T {
        self.resolved
    }

    fn map<U>(self, transform: &mut impl FnMut(T) -> U) -> DesignPanelPropertyBinding<U> {
        DesignPanelPropertyBinding {
            id: self.id,
            name: self.name,
            kind: self.kind,
            resolved: transform(self.resolved),
        }
    }
}

/// Read-only wrapper that retains the property's underlying display state.
#[derive(Clone, Debug, PartialEq)]
pub struct DesignPanelReadOnlyValue<T = DesignPanelValue> {
    value: Box<DesignPanelPropertyValueState<T>>,
    reason: Option<SharedString>,
}

impl<T> DesignPanelReadOnlyValue<T> {
    pub const fn value(&self) -> &DesignPanelPropertyValueState<T> {
        &self.value
    }

    pub fn into_value(self) -> DesignPanelPropertyValueState<T> {
        *self.value
    }

    pub fn reason(&self) -> Option<&str> {
        self.reason.as_deref()
    }
}

/// Display and interaction state for an inspector property.
///
/// The generic payload is the host's display value. `ReadOnly` wraps another
/// state so a read-only mixed or bound value does not lose information.
#[derive(Clone, Debug, PartialEq)]
pub enum DesignPanelPropertyValueState<T = DesignPanelValue> {
    Unset,
    Uniform(T),
    Mixed,
    Bound(DesignPanelPropertyBinding<T>),
    ReadOnly(DesignPanelReadOnlyValue<T>),
}

impl<T> DesignPanelPropertyValueState<T> {
    pub fn bound(binding: DesignPanelPropertyBinding<T>) -> Self {
        Self::Bound(binding)
    }

    pub fn read_only(self) -> Self {
        self.into_read_only(None)
    }

    pub fn read_only_with_reason(self, reason: impl Into<SharedString>) -> Self {
        self.into_read_only(Some(reason.into()))
    }

    fn into_read_only(self, reason: Option<SharedString>) -> Self {
        match self {
            Self::ReadOnly(mut read_only) => {
                if reason.is_some() {
                    read_only.reason = reason;
                }
                Self::ReadOnly(read_only)
            }
            value => Self::ReadOnly(DesignPanelReadOnlyValue {
                value: Box::new(value),
                reason,
            }),
        }
    }

    pub const fn is_read_only(&self) -> bool {
        matches!(self, Self::ReadOnly(_))
    }

    pub fn is_unset(&self) -> bool {
        match self {
            Self::Unset => true,
            Self::ReadOnly(read_only) => read_only.value().is_unset(),
            _ => false,
        }
    }

    pub fn is_mixed(&self) -> bool {
        match self {
            Self::Mixed => true,
            Self::ReadOnly(read_only) => read_only.value().is_mixed(),
            _ => false,
        }
    }

    pub fn binding(&self) -> Option<&DesignPanelPropertyBinding<T>> {
        match self {
            Self::Bound(binding) => Some(binding),
            Self::ReadOnly(read_only) => read_only.value().binding(),
            _ => None,
        }
    }

    pub fn resolved(&self) -> Option<&T> {
        match self {
            Self::Uniform(value) => Some(value),
            Self::Bound(binding) => Some(binding.resolved()),
            Self::ReadOnly(read_only) => read_only.value().resolved(),
            Self::Unset | Self::Mixed => None,
        }
    }

    pub fn map<U>(self, mut transform: impl FnMut(T) -> U) -> DesignPanelPropertyValueState<U> {
        self.map_with(&mut transform)
    }

    fn map_with<U>(self, transform: &mut impl FnMut(T) -> U) -> DesignPanelPropertyValueState<U> {
        match self {
            Self::Unset => DesignPanelPropertyValueState::Unset,
            Self::Uniform(value) => DesignPanelPropertyValueState::Uniform(transform(value)),
            Self::Mixed => DesignPanelPropertyValueState::Mixed,
            Self::Bound(binding) => DesignPanelPropertyValueState::Bound(binding.map(transform)),
            Self::ReadOnly(read_only) => {
                let DesignPanelReadOnlyValue { value, reason } = read_only;
                DesignPanelPropertyValueState::ReadOnly(DesignPanelReadOnlyValue {
                    value: Box::new(value.map_with(transform)),
                    reason,
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selection_canonicalizes_each_cardinality() {
        let none = DesignPanelSelection::<u8>::from_items(vec![]);
        let single = DesignPanelSelection::from_items(vec![7]);
        let multiple = DesignPanelSelection::from_items(vec![1, 2, 3]);

        assert_eq!(none.kind(), DesignPanelSelectionKind::None);
        assert!(none.is_empty());
        assert_eq!(single.kind(), DesignPanelSelectionKind::Single);
        assert_eq!(single.items(), &[7]);
        assert_eq!(multiple.kind(), DesignPanelSelectionKind::Multiple);
        assert_eq!(multiple.items(), &[1, 2, 3]);
    }

    #[test]
    fn multiple_selection_rejects_fewer_than_two_items() {
        let empty = DesignPanelMultipleSelection::<u8>::try_from_items(vec![])
            .expect_err("an empty selection must not be multiple");
        let single = DesignPanelMultipleSelection::try_from_items(vec![1])
            .expect_err("a one-item selection must not be multiple");
        let multiple = DesignPanelMultipleSelection::try_from_items(vec![1, 2])
            .expect("two items form a multiple selection");

        assert_eq!(empty.actual(), 0);
        assert_eq!(single.actual(), 1);
        assert_eq!(multiple.items(), &[1, 2]);
    }

    #[test]
    fn parent_layout_distinguishes_flow_ignored_and_mixed_contexts() {
        let in_flow = DesignPanelParentLayout::auto_layout(
            DesignPanelAutoLayoutDirection::Horizontal,
            DesignPanelAutoLayoutWrap::Wrap,
            DesignPanelAutoLayoutParticipation::InFlow,
        );
        let ignored = DesignPanelParentLayout::auto_layout(
            DesignPanelAutoLayoutDirection::Grid,
            DesignPanelAutoLayoutWrap::Wrap,
            DesignPanelAutoLayoutParticipation::Ignored,
        );

        assert!(in_flow.is_auto_layout());
        assert!(in_flow.participates_in_auto_layout());
        assert!(!in_flow.is_ignored_by_auto_layout());
        assert_eq!(
            in_flow.auto_layout_direction(),
            Some(DesignPanelAutoLayoutDirection::Horizontal)
        );
        assert!(ignored.is_ignored_by_auto_layout());
        assert!(matches!(
            ignored,
            DesignPanelParentLayout::AutoLayout {
                wrap: DesignPanelAutoLayoutWrap::NoWrap,
                ..
            }
        ));
        assert_eq!(
            ignored.auto_layout_direction(),
            Some(DesignPanelAutoLayoutDirection::Grid)
        );
        assert!(!DesignPanelParentLayout::Freeform.is_auto_layout());
        assert_eq!(
            DesignPanelParentLayout::Freeform.auto_layout_direction(),
            None
        );
        assert!(!DesignPanelParentLayout::Mixed.is_auto_layout());
    }

    #[test]
    fn inspection_context_covers_page_single_and_multiple_selection() {
        let page = DesignPanelInspectionContext::<u8>::page(DesignPanelPermissions::viewer());
        let single = DesignPanelInspectionContext::single(
            7,
            DesignPanelParentLayout::Freeform,
            DesignPanelPermissions::editor(),
        )
        .with_edit_mode(DesignPanelEditMode::Text)
        .expect("an editable single selection supports text mode");
        let multiple = DesignPanelInspectionContext::multiple(
            DesignPanelMultipleSelection::new(1, 2),
            DesignPanelParentLayout::Mixed,
            DesignPanelPermissions::editor(),
        );

        assert_eq!(page.selection().kind(), DesignPanelSelectionKind::None);
        assert_eq!(page.edit_mode(), DesignPanelEditMode::Canvas);
        assert_eq!(single.selection().items(), &[7]);
        assert_eq!(single.edit_mode(), DesignPanelEditMode::Text);
        assert_eq!(
            multiple.selection().kind(),
            DesignPanelSelectionKind::Multiple
        );
        assert_eq!(multiple.parent_layout(), DesignPanelParentLayout::Mixed);
    }

    #[test]
    fn text_range_revision_is_exact_text_mode_context_identity() {
        let text = DesignPanelInspectionContext::single(
            7,
            DesignPanelParentLayout::Freeform,
            DesignPanelPermissions::editor(),
        )
        .with_edit_mode(DesignPanelEditMode::Text)
        .expect("Text mode")
        .with_text_range_revision(41)
        .expect("Text mode accepts an exact range revision");
        assert_eq!(text.text_range_revision(), Some(41));

        let same_mode = text
            .clone()
            .with_edit_mode(DesignPanelEditMode::Text)
            .expect("remaining in Text mode preserves the range");
        assert_eq!(same_mode.text_range_revision(), Some(41));

        let object = text
            .with_edit_mode(DesignPanelEditMode::Object)
            .expect("leaving Text mode is valid");
        assert_eq!(object.text_range_revision(), None);
        assert!(matches!(
            object.with_text_range_revision(42),
            Err(
                DesignPanelContextError::TextRangeRevisionRequiresTextEditMode {
                    edit_mode: DesignPanelEditMode::Object
                }
            )
        ));
    }

    #[test]
    fn inspection_context_rejects_incompatible_edit_states() {
        let multiple_text = DesignPanelInspectionContext::try_new(
            DesignPanelSelection::from_items(vec![1, 2]),
            DesignPanelParentLayout::Mixed,
            DesignPanelPermissions::editor(),
            DesignPanelEditMode::Text,
        );
        let viewer_vector = DesignPanelInspectionContext::try_new(
            DesignPanelSelection::Single(1),
            DesignPanelParentLayout::Freeform,
            DesignPanelPermissions::viewer(),
            DesignPanelEditMode::Vector,
        );
        let page_with_parent = DesignPanelInspectionContext::<u8>::try_new(
            DesignPanelSelection::None,
            DesignPanelParentLayout::Freeform,
            DesignPanelPermissions::viewer(),
            DesignPanelEditMode::Canvas,
        );

        assert!(matches!(
            multiple_text,
            Err(DesignPanelContextError::EditModeSelectionMismatch { .. })
        ));
        assert!(matches!(
            viewer_vector,
            Err(DesignPanelContextError::EditModeRequiresEditPermission { .. })
        ));
        assert!(matches!(
            page_with_parent,
            Err(DesignPanelContextError::NoSelectionHasParentLayout { .. })
        ));
    }

    #[test]
    fn property_states_preserve_bound_mixed_and_read_only_information() {
        let unset = DesignPanelPropertyValueState::<u8>::Unset;
        let uniform = DesignPanelPropertyValueState::Uniform(12_u8);
        let binding = DesignPanelPropertyBinding::new(
            "variable:spacing",
            "Spacing/Large",
            DesignPanelBindingKind::Variable,
            24_u8,
        );
        let bound = DesignPanelPropertyValueState::bound(binding);
        let read_only_bound = bound.read_only_with_reason("Library component");
        let read_only_mixed = DesignPanelPropertyValueState::<u8>::Mixed.read_only();

        assert!(unset.is_unset());
        assert_eq!(unset.resolved(), None);
        assert_eq!(uniform.resolved(), Some(&12));
        assert_eq!(read_only_bound.resolved(), Some(&24));
        assert_eq!(
            read_only_bound
                .binding()
                .expect("the binding remains visible")
                .name()
                .as_ref(),
            "Spacing/Large"
        );
        assert!(read_only_bound.is_read_only());
        assert_eq!(
            match &read_only_bound {
                DesignPanelPropertyValueState::ReadOnly(value) => value.reason(),
                _ => None,
            },
            Some("Library component")
        );
        assert!(read_only_mixed.is_mixed());
        assert_eq!(read_only_mixed.resolved(), None);
    }

    #[test]
    fn permissions_and_value_state_both_gate_property_editing() {
        let restricted = DesignPanelPermissions::restricted_viewer();
        let editor = DesignPanelInspectionContext::single(
            1_u8,
            DesignPanelParentLayout::Canvas,
            DesignPanelPermissions::editor(),
        );
        let viewer = DesignPanelInspectionContext::single(
            1_u8,
            DesignPanelParentLayout::Canvas,
            DesignPanelPermissions::viewer(),
        );
        let uniform = DesignPanelPropertyValueState::Uniform(12_u8);
        let read_only = DesignPanelPropertyValueState::Uniform(12_u8).read_only();

        assert!(editor.property_is_editable(&uniform));
        assert!(!editor.property_is_editable(&read_only));
        assert!(!viewer.property_is_editable(&uniform));
        assert!(!restricted.can_edit());
        assert!(!restricted.can_copy());
        assert!(!restricted.can_export());
    }

    #[test]
    fn mapping_property_state_keeps_binding_and_read_only_metadata() {
        let state = DesignPanelPropertyValueState::bound(DesignPanelPropertyBinding::new(
            "style:opacity",
            "Muted",
            DesignPanelBindingKind::Style,
            0.5_f32,
        ))
        .read_only_with_reason("View access");

        let mapped = state.map(|value| (value * 100.0) as u8);

        assert_eq!(mapped.resolved(), Some(&50));
        assert_eq!(
            mapped.binding().expect("mapping keeps the binding").kind(),
            DesignPanelBindingKind::Style
        );
        assert_eq!(
            match mapped {
                DesignPanelPropertyValueState::ReadOnly(value) => value.reason,
                _ => None,
            },
            Some(SharedString::from("View access"))
        );
    }
}
