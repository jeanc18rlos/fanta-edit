//! Transient edit-session state for the Design inspector.
//!
//! The facade owns this controller, while feature sections start and finish
//! sessions through the existing typed action helpers. Keeping every draft in
//! one place makes context reconciliation and the one-terminal-event invariant
//! auditable without giving the component ownership of document values.

use super::super::DesignMediaPaintView;
use super::*;

/// One host-facing event produced by the guarded property-edit lifecycle.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct DesignPropertyEditEvent {
    pub(super) property: DesignPanelProperty,
    pub(super) value: DesignPanelValue,
    pub(super) phase: DesignPanelEditPhase,
}

/// One unchanged host-facing variable-axis edit projected from the shared
/// numeric field lifecycle.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct DesignVariableFontAxisEditEvent {
    pub(super) target: DesignTypographyTarget,
    pub(super) tag: SharedString,
    pub(super) value: f32,
    pub(super) phase: DesignPanelEditPhase,
}

/// One unchanged host-facing multiline component-property edit projected from
/// the shared exact-text lifecycle.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct DesignComponentMultilineEditEvent {
    pub(super) property_id: SharedString,
    pub(super) value: DesignComponentPropertyValue,
    pub(super) phase: DesignPanelEditPhase,
}

/// Stable identity for one phased Grid-dimensions edit.
///
/// The axis is part of the target even though the host action remains atomic
/// over both dimensions. This lets an access echo invalidate exactly the
/// field that started the transaction without decomposing the host intent.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct DesignGridDimensionsEditTarget {
    node_id: SharedString,
    axis: DesignGridTrackAxis,
}

impl DesignGridDimensionsEditTarget {
    pub(super) fn new(node_id: SharedString, axis: DesignGridTrackAxis) -> Self {
        Self { node_id, axis }
    }
}

/// One unchanged host-facing atomic Grid-dimensions edit produced by the
/// guarded lifecycle.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct DesignGridDimensionsEditEvent {
    pub(super) node_id: SharedString,
    pub(super) dimensions: DesignGridDimensions,
    pub(super) phase: DesignPanelEditPhase,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ActiveGridDimensionsEdit {
    target: DesignGridDimensionsEditTarget,
    original: DesignGridDimensions,
    expected: DesignGridDimensions,
}

/// Guard for the Grid count fields' `Begin -> Preview* -> Commit | Cancel`
/// lifecycle.
///
/// A Commit without Begin remains valid for the atomic picker and other
/// one-shot controls. Once Begin succeeds, however, only the captured node and
/// axis may preview or terminate it. Host echoes may report either the
/// original dimensions or the latest preview; any other value, target loss,
/// capability loss, or access loss consumes one Cancel restoring `original`.
#[derive(Default)]
struct DesignGridDimensionsEditLifecycle {
    active: Option<ActiveGridDimensionsEdit>,
}

impl DesignGridDimensionsEditLifecycle {
    fn edit(
        &mut self,
        target: DesignGridDimensionsEditTarget,
        current: DesignGridDimensions,
        candidate: DesignGridDimensions,
        phase: DesignPanelEditPhase,
    ) -> Option<DesignGridDimensionsEditEvent> {
        let matches_active = self
            .active
            .as_ref()
            .is_some_and(|active| active.target == target);
        let dimensions = match phase {
            DesignPanelEditPhase::Begin if self.active.is_none() => {
                self.active = Some(ActiveGridDimensionsEdit {
                    target: target.clone(),
                    original: current,
                    expected: candidate,
                });
                candidate
            }
            DesignPanelEditPhase::Begin => return None,
            DesignPanelEditPhase::Preview if matches_active => {
                self.active
                    .as_mut()
                    .expect("a matching Grid Preview has an active transaction")
                    .expected = candidate;
                candidate
            }
            DesignPanelEditPhase::Preview => return None,
            DesignPanelEditPhase::Commit if self.active.is_none() => candidate,
            DesignPanelEditPhase::Commit if matches_active => {
                self.active = None;
                candidate
            }
            DesignPanelEditPhase::Commit => return None,
            DesignPanelEditPhase::Cancel if matches_active => {
                self.active
                    .take()
                    .expect("a matching Grid Cancel has an active transaction")
                    .original
            }
            DesignPanelEditPhase::Cancel => return None,
        };
        Some(DesignGridDimensionsEditEvent {
            node_id: target.node_id,
            dimensions,
            phase,
        })
    }

    fn cancel(&mut self) -> Option<DesignGridDimensionsEditEvent> {
        let active = self.active.take()?;
        Some(DesignGridDimensionsEditEvent {
            node_id: active.target.node_id,
            dimensions: active.original,
            phase: DesignPanelEditPhase::Cancel,
        })
    }

    fn reconcile(
        &mut self,
        current_node_id: &SharedString,
        current: Option<DesignGridDimensions>,
        columns_editable: bool,
        rows_editable: bool,
    ) -> Option<DesignGridDimensionsEditEvent> {
        let active = self.active.as_mut()?;
        let target_editable = match active.target.axis {
            DesignGridTrackAxis::Column => columns_editable,
            DesignGridTrackAxis::Row => rows_editable,
        };
        let survives = active.target.node_id == *current_node_id
            && target_editable
            && current
                .is_some_and(|current| current == active.original || current == active.expected);
        if !survives {
            return self.cancel();
        }
        active.expected = current.expect("a surviving Grid edit has current dimensions");
        None
    }

    fn target_is_active(&self, node_id: &SharedString, axis: Option<DesignGridTrackAxis>) -> bool {
        self.active.as_ref().is_some_and(|active| {
            active.target.node_id == *node_id && axis.is_none_or(|axis| active.target.axis == axis)
        })
    }

    fn has_active_transaction(&self) -> bool {
        self.active.is_some()
    }
}

/// The latest candidate and stable target captured by one picker-driven paint
/// transaction.
///
/// This is edit coordination state, not feature presentation state. Keeping it
/// beside the other edit lifecycles ensures that a context or capability echo
/// cannot clear a picker without first consuming its one terminal event.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct ActivePaintEdit {
    pub(super) target: PickerEventTarget,
    pub(super) edit: DesignPaintEdit,
}

/// Guard for the paint picker's `Begin -> Preview* -> Commit | Cancel`
/// lifecycle.
///
/// Stable paint ids permit the active target to be rebased when a host echo
/// reorders rows. Id-less compatibility targets remain index-bound because
/// `PickerEventTarget` equality intentionally uses their index as identity.
#[derive(Default)]
struct DesignPaintEditLifecycle {
    active: Option<ActivePaintEdit>,
}

impl DesignPaintEditLifecycle {
    fn track(
        &mut self,
        target: &PickerEventTarget,
        edit: &DesignPaintEdit,
        phase: DesignPanelEditPhase,
    ) -> bool {
        let matches_active = self
            .active
            .as_ref()
            .is_some_and(|active| active.target == *target);
        match phase {
            DesignPanelEditPhase::Begin if self.active.is_none() => {
                self.active = Some(ActivePaintEdit {
                    target: target.clone(),
                    edit: edit.clone(),
                });
                true
            }
            DesignPanelEditPhase::Begin => false,
            DesignPanelEditPhase::Preview if matches_active => {
                if let Some(active) = self.active.as_mut() {
                    active.target = target.clone();
                    active.edit = edit.clone();
                }
                true
            }
            DesignPanelEditPhase::Preview => false,
            DesignPanelEditPhase::Commit if self.active.is_none() => {
                // Picker buttons and toggles are atomic Commit-only edits.
                true
            }
            DesignPanelEditPhase::Commit | DesignPanelEditPhase::Cancel if matches_active => {
                self.active = None;
                true
            }
            DesignPanelEditPhase::Commit | DesignPanelEditPhase::Cancel => false,
        }
    }

    fn active(&self) -> Option<&ActivePaintEdit> {
        self.active.as_ref()
    }

    fn cancel(&mut self) -> Option<ActivePaintEdit> {
        self.active.take()
    }

    /// Reconcile a host echo against the stable target resolved by the paint
    /// section. `None` consumes the transaction for one Cancel; a surviving
    /// target updates its current index without creating a second Begin.
    fn reconcile_target(&mut self, target: Option<PickerEventTarget>) -> Option<ActivePaintEdit> {
        let active = self.active.as_mut()?;
        let Some(target) = target.filter(|target| active.target == *target) else {
            return self.active.take();
        };
        active.target = target;
        None
    }

    fn has_active_transaction(&self) -> bool {
        self.active.is_some()
    }
}

/// The section-level validity decision for an active edit after a host echo.
///
/// Sections remain responsible for resolving stable Design-domain identities
/// and checking current capabilities. The controller owns applying that
/// decision to every piece of transient edit state as one atomic transition.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum DesignPropertyEditReconciliation {
    Keep,
    Rebase {
        previous: DesignPanelProperty,
        next: DesignPanelProperty,
    },
    Cancel,
}

/// Observable controller result used by the facade to emit the unchanged
/// host-facing action, if the reconciliation terminated a transaction.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum DesignPropertyEditReconciliationResult {
    Kept,
    Rebased {
        previous: DesignPanelProperty,
        next: DesignPanelProperty,
    },
    Cancelled(Option<DesignPropertyEditEvent>),
}

impl DesignPropertyEditReconciliationResult {
    pub(super) fn terminal_event(self) -> Option<DesignPropertyEditEvent> {
        match self {
            Self::Kept | Self::Rebased { .. } => None,
            Self::Cancelled(event) => event,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
struct ActivePropertyEditLifecycle {
    property: DesignPanelProperty,
    original: DesignPanelValue,
}

/// State machine for the ordinary field and numeric-scrub edit path.
///
/// An active transaction exists only after `begin` has produced its Begin
/// event. Commit and Cancel consume that transaction, which makes duplicate
/// terminal events impossible. Host invalidation uses the same Cancel path and
/// therefore restores the exact value captured at Begin.
#[derive(Default)]
struct DesignPropertyEditLifecycle {
    active: Option<ActivePropertyEditLifecycle>,
}

impl DesignPropertyEditLifecycle {
    fn begin(
        &mut self,
        property: DesignPanelProperty,
        original: DesignPanelValue,
    ) -> Option<DesignPropertyEditEvent> {
        if self.active.is_some() {
            return None;
        }
        self.active = Some(ActivePropertyEditLifecycle {
            property,
            original: original.clone(),
        });
        Some(DesignPropertyEditEvent {
            property,
            value: original,
            phase: DesignPanelEditPhase::Begin,
        })
    }

    fn preview(
        &self,
        property: DesignPanelProperty,
        value: DesignPanelValue,
    ) -> Option<DesignPropertyEditEvent> {
        self.active
            .as_ref()
            .filter(|active| active.property == property)?;
        Some(DesignPropertyEditEvent {
            property,
            value,
            phase: DesignPanelEditPhase::Preview,
        })
    }

    fn commit(
        &mut self,
        property: DesignPanelProperty,
        value: DesignPanelValue,
    ) -> Option<DesignPropertyEditEvent> {
        if self
            .active
            .as_ref()
            .is_none_or(|active| active.property != property)
        {
            return None;
        }
        self.active.take();
        Some(DesignPropertyEditEvent {
            property,
            value,
            phase: DesignPanelEditPhase::Commit,
        })
    }

    fn cancel(&mut self) -> Option<DesignPropertyEditEvent> {
        let active = self.active.take()?;
        Some(DesignPropertyEditEvent {
            property: active.property,
            value: active.original,
            phase: DesignPanelEditPhase::Cancel,
        })
    }

    fn rebase(&mut self, previous: DesignPanelProperty, next: DesignPanelProperty) -> bool {
        let Some(active) = self
            .active
            .as_mut()
            .filter(|active| active.property == previous)
        else {
            return false;
        };
        active.property = next;
        true
    }
}

/// Stable identity for one host-controlled media-crop transaction.
///
/// The collection and paint id survive row reordering, while the node id
/// prevents a delayed host echo from one selection suppressing a crop in the
/// next selection.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct DesignMediaCropEditTarget {
    node_id: SharedString,
    collection: DesignPanelCollection,
    paint_id: SharedString,
}

impl DesignMediaCropEditTarget {
    pub(super) fn new(
        node_id: SharedString,
        collection: DesignPanelCollection,
        paint_id: SharedString,
    ) -> Self {
        Self {
            node_id,
            collection,
            paint_id,
        }
    }

    fn matches_view(&self, node_id: &SharedString, view: &DesignMediaPaintView) -> bool {
        self.node_id == *node_id
            && self.collection == view.collection
            && self.paint_id == view.paint_id
    }
}

/// Guard for the crop tool's host-owned asynchronous lifecycle.
///
/// A terminal request stays pending until the host snapshot reports that the
/// crop tool is inactive. This matters because closing the picker immediately
/// after Commit/Cancel otherwise observes the previous `active: true` echo and
/// emits a second Cancel.
#[derive(Default)]
struct DesignMediaCropEditLifecycle {
    active: Option<DesignMediaCropEditTarget>,
    terminal_pending: Vec<DesignMediaCropEditTarget>,
}

impl DesignMediaCropEditLifecycle {
    fn should_forward(
        &mut self,
        target: DesignMediaCropEditTarget,
        action: &DesignMediaCropAction,
        host_active: bool,
    ) -> bool {
        if let Some(position) = self
            .terminal_pending
            .iter()
            .position(|pending| pending == &target)
        {
            // Seeing an inactive host echo means the previous terminal action
            // has settled. A new Begin may now start a fresh transaction even
            // if reconciliation has not otherwise run yet.
            if matches!(action, DesignMediaCropAction::Begin) && !host_active {
                self.terminal_pending.remove(position);
            } else {
                return false;
            }
        }

        match action {
            DesignMediaCropAction::Begin => {
                if self.active.as_ref() == Some(&target) {
                    return false;
                }
                self.active = Some(target);
                true
            }
            DesignMediaCropAction::Preview { .. } => {
                if self.active.as_ref() == Some(&target) {
                    true
                } else if self.active.is_none() && host_active {
                    // The host may have opened the crop tool externally.
                    self.active = Some(target);
                    true
                } else {
                    false
                }
            }
            DesignMediaCropAction::Commit { .. }
            | DesignMediaCropAction::Cancel
            | DesignMediaCropAction::ResizeToFit => {
                let locally_active = self.active.as_ref() == Some(&target);
                if !locally_active && !host_active {
                    return false;
                }
                if locally_active {
                    self.active = None;
                }
                self.terminal_pending.push(target);
                true
            }
        }
    }

    fn reconcile_host_snapshot(
        &mut self,
        node_id: &SharedString,
        view_data: &DesignMediaPaintViewData,
    ) {
        let is_active = |target: &DesignMediaCropEditTarget| {
            view_data
                .paints
                .iter()
                .find(|view| target.matches_view(node_id, view))
                .is_some_and(|view| view.crop_tool.active)
        };
        self.terminal_pending.retain(is_active);
        if self
            .active
            .as_ref()
            .is_some_and(|target| !is_active(target))
        {
            self.active = None;
        }
    }

    fn has_active_transaction(&self) -> bool {
        self.active.is_some()
    }
}

fn focus_origin_matches_property(
    origin: &EditorFocusOrigin,
    property: DesignPanelProperty,
) -> bool {
    matches!(
        origin,
        EditorFocusOrigin::ValueCell(origin_property)
            | EditorFocusOrigin::ShaderField {
                property: origin_property,
                ..
            }
            | EditorFocusOrigin::TypeSetting(origin_property)
            if *origin_property == property
    )
}

fn rebase_focus_origin(
    origin: &mut EditorFocusOrigin,
    previous: DesignPanelProperty,
    next: DesignPanelProperty,
) {
    match origin {
        EditorFocusOrigin::ValueCell(property)
        | EditorFocusOrigin::ShaderField { property, .. }
        | EditorFocusOrigin::TypeSetting(property)
            if *property == previous =>
        {
            *property = next;
        }
        EditorFocusOrigin::ValueCell(_)
        | EditorFocusOrigin::ShaderField { .. }
        | EditorFocusOrigin::TypeSetting(_)
        | EditorFocusOrigin::ComponentMultiline(_) => {}
    }
}

#[derive(Default)]
pub(super) struct DesignPropertyEditController {
    property_lifecycle: DesignPropertyEditLifecycle,
    paint_lifecycle: DesignPaintEditLifecycle,
    media_crop_lifecycle: DesignMediaCropEditLifecycle,
    pub(super) component_authoring: DesignComponentAuthoringEditController,
    pub(super) property: Option<PropertyEditor>,
    pub(super) numeric_scrub: Option<NumericPropertyScrub>,
    pub(super) focus_return: Option<EditorFocusReturn>,
    pub(super) draw_slider_property: Option<DesignPanelProperty>,
    /// Domain-neutral direct-manipulation session paired with the Design
    /// property lifecycle above. The shared field owns preview de-duplication
    /// and the single terminal transition; this controller only maps its
    /// scalar value back to a Design property intent.
    pub(super) draw_slider_field: Option<crate::molecules::InspectorSliderField>,
    grid_dimensions_lifecycle: DesignGridDimensionsEditLifecycle,
    pub(super) variable_font_axis: Option<VariableFontAxisEditor>,
    pub(super) variable_font_axis_scrub: Option<VariableFontAxisScrub>,
    pub(super) component_multiline: Option<ComponentMultilineEditor>,
    pub(super) vector_target_ids: Option<Vec<SharedString>>,
    pub(super) property_invalid: bool,
    pub(super) variable_font_axis_invalid: bool,
    pub(super) suppress_property_input_change: bool,
    pub(super) suppress_next_control_activation: bool,
}

impl DesignPropertyEditController {
    pub(super) fn edit_grid_dimensions(
        &mut self,
        target: DesignGridDimensionsEditTarget,
        current: DesignGridDimensions,
        candidate: DesignGridDimensions,
        phase: DesignPanelEditPhase,
    ) -> Option<DesignGridDimensionsEditEvent> {
        self.grid_dimensions_lifecycle
            .edit(target, current, candidate, phase)
    }

    pub(super) fn cancel_grid_dimensions_edit(&mut self) -> Option<DesignGridDimensionsEditEvent> {
        self.grid_dimensions_lifecycle.cancel()
    }

    pub(super) fn reconcile_grid_dimensions_edit(
        &mut self,
        current_node_id: &SharedString,
        current: Option<DesignGridDimensions>,
        columns_editable: bool,
        rows_editable: bool,
    ) -> Option<DesignGridDimensionsEditEvent> {
        self.grid_dimensions_lifecycle.reconcile(
            current_node_id,
            current,
            columns_editable,
            rows_editable,
        )
    }

    pub(super) fn grid_dimensions_target_is_active(
        &self,
        node_id: &SharedString,
        axis: Option<DesignGridTrackAxis>,
    ) -> bool {
        self.grid_dimensions_lifecycle
            .target_is_active(node_id, axis)
    }

    pub(super) fn track_paint_edit(
        &mut self,
        target: &PickerEventTarget,
        edit: &DesignPaintEdit,
        phase: DesignPanelEditPhase,
    ) -> bool {
        self.paint_lifecycle.track(target, edit, phase)
    }

    pub(super) fn active_paint_edit(&self) -> Option<&ActivePaintEdit> {
        self.paint_lifecycle.active()
    }

    pub(super) fn cancel_paint_edit(&mut self) -> Option<ActivePaintEdit> {
        self.paint_lifecycle.cancel()
    }

    pub(super) fn reconcile_paint_edit_target(
        &mut self,
        target: Option<PickerEventTarget>,
    ) -> Option<ActivePaintEdit> {
        self.paint_lifecycle.reconcile_target(target)
    }

    fn variable_font_axis_event(
        editor: &VariableFontAxisEditor,
        value: f32,
        phase: DesignPanelEditPhase,
    ) -> DesignVariableFontAxisEditEvent {
        DesignVariableFontAxisEditEvent {
            target: editor.target,
            tag: editor.tag.clone(),
            value,
            phase,
        }
    }

    pub(super) fn begin_variable_font_axis_edit(
        &mut self,
        mut editor: VariableFontAxisEditor,
        displayed: &str,
    ) -> Option<DesignVariableFontAxisEditEvent> {
        if self.variable_font_axis.is_some() {
            return None;
        }
        let begin = editor
            .field
            .begin_with(f64::from(editor.original), displayed)?;
        if begin.phase != crate::molecules::InspectorEditPhase::Begin {
            return None;
        }
        let event =
            Self::variable_font_axis_event(&editor, editor.original, DesignPanelEditPhase::Begin);
        self.variable_font_axis = Some(editor);
        self.variable_font_axis_invalid = false;
        Some(event)
    }

    pub(super) fn preview_variable_font_axis_edit(
        &mut self,
        tag: &str,
        value: f32,
        draft: Option<&str>,
    ) -> Option<DesignVariableFontAxisEditEvent> {
        let editor = self
            .variable_font_axis
            .as_mut()
            .filter(|editor| editor.tag.as_ref() == tag)?;
        if editor.last_preview.is_none() && editor.original == value {
            return None;
        }
        let preview = if let Some(draft) = draft {
            let _ = editor.field.set_text(draft);
            editor.field.preview_with(|_| Some(f64::from(value)))
        } else {
            editor
                .field
                .scrub_to(f64::from(value), |value| format_nudge_number(value as f32))
        }?;
        if preview.phase != crate::molecules::InspectorEditPhase::Preview {
            return None;
        }
        editor.last_preview = Some(value);
        Some(Self::variable_font_axis_event(
            editor,
            value,
            DesignPanelEditPhase::Preview,
        ))
    }

    pub(super) fn sync_variable_font_axis_draft(&mut self, draft: &str) -> bool {
        self.variable_font_axis
            .as_mut()
            .is_some_and(|editor| editor.field.set_text(draft))
    }

    pub(super) fn nudge_variable_font_axis_edit(
        &mut self,
        tag: &str,
        current: f32,
        value: f32,
        displayed: &str,
    ) -> Option<DesignVariableFontAxisEditEvent> {
        let editor = self
            .variable_font_axis
            .as_mut()
            .filter(|editor| editor.tag.as_ref() == tag)?;
        if editor.last_preview.is_none() && editor.original == value {
            return None;
        }
        let preview = editor.field.nudge_with(
            f64::from(value - current),
            |_| Some(f64::from(current)),
            |_| Some(f64::from(value)),
            |_| displayed.to_owned(),
        )?;
        if preview.phase != crate::molecules::InspectorEditPhase::Preview {
            return None;
        }
        editor.last_preview = Some(value);
        Some(Self::variable_font_axis_event(
            editor,
            value,
            DesignPanelEditPhase::Preview,
        ))
    }

    pub(super) fn finish_variable_font_axis_edit(
        &mut self,
        commit: Option<f32>,
    ) -> Option<DesignVariableFontAxisEditEvent> {
        let mut editor = self.variable_font_axis.take()?;
        let terminal = if let Some(value) = commit {
            editor.field.commit_with(|_| Some(f64::from(value)))
        } else {
            editor.field.cancel()
        }?;
        let (value, phase) = match terminal.phase {
            crate::molecules::InspectorEditPhase::Commit => (
                commit.expect("a shared Commit retains the accepted axis value"),
                DesignPanelEditPhase::Commit,
            ),
            crate::molecules::InspectorEditPhase::Cancel => {
                (editor.original, DesignPanelEditPhase::Cancel)
            }
            crate::molecules::InspectorEditPhase::Begin
            | crate::molecules::InspectorEditPhase::Preview => return None,
        };
        self.variable_font_axis_scrub = None;
        self.variable_font_axis_invalid = false;
        self.suppress_property_input_change = false;
        Some(Self::variable_font_axis_event(&editor, value, phase))
    }

    pub(super) fn begin_component_multiline_edit(
        &mut self,
        mut editor: ComponentMultilineEditor,
        value: SharedString,
    ) -> Option<DesignComponentMultilineEditEvent> {
        if self.component_multiline.is_some() {
            return None;
        }
        let begin = editor.field.begin_with(value)?;
        if begin.phase != crate::molecules::InspectorEditPhase::Begin {
            return None;
        }
        let event = DesignComponentMultilineEditEvent {
            property_id: editor.property_id.clone(),
            value: editor.original.clone(),
            phase: DesignPanelEditPhase::Begin,
        };
        self.component_multiline = Some(editor);
        Some(event)
    }

    pub(super) fn preview_component_multiline_edit(
        &mut self,
        property_id: &str,
        value: SharedString,
    ) -> Option<DesignComponentMultilineEditEvent> {
        let editor = self
            .component_multiline
            .as_mut()
            .filter(|editor| editor.property_id.as_ref() == property_id)?;
        let candidate = DesignComponentPropertyValue::Text(value.clone());
        if editor.last_preview.is_none() && editor.original == candidate {
            return None;
        }
        let preview = editor.field.preview(value)?;
        if preview.phase != crate::molecules::InspectorEditPhase::Preview {
            return None;
        }
        editor.last_preview = Some(candidate.clone());
        Some(DesignComponentMultilineEditEvent {
            property_id: editor.property_id.clone(),
            value: candidate,
            phase: DesignPanelEditPhase::Preview,
        })
    }

    pub(super) fn finish_component_multiline_edit(
        &mut self,
        commit: Option<SharedString>,
    ) -> Option<DesignComponentMultilineEditEvent> {
        let mut editor = self.component_multiline.take()?;
        let terminal = if commit.is_some() {
            editor.field.commit()
        } else {
            editor.field.cancel()
        }?;
        let (value, phase) = match terminal.phase {
            crate::molecules::InspectorEditPhase::Commit => (
                DesignComponentPropertyValue::Text(
                    commit.expect("a shared Commit retains the accepted multiline value"),
                ),
                DesignPanelEditPhase::Commit,
            ),
            crate::molecules::InspectorEditPhase::Cancel => {
                (editor.original, DesignPanelEditPhase::Cancel)
            }
            crate::molecules::InspectorEditPhase::Begin
            | crate::molecules::InspectorEditPhase::Preview => return None,
        };
        Some(DesignComponentMultilineEditEvent {
            property_id: editor.property_id,
            value,
            phase,
        })
    }

    /// Returns whether the unchanged host-facing crop action should be
    /// forwarded. Terminal actions consume local activity immediately and are
    /// then suppressed until the host reports the tool inactive.
    pub(super) fn should_forward_media_crop_action(
        &mut self,
        target: DesignMediaCropEditTarget,
        action: &DesignMediaCropAction,
        host_active: bool,
    ) -> bool {
        self.media_crop_lifecycle
            .should_forward(target, action, host_active)
    }

    pub(super) fn reconcile_media_crop_host_snapshot(
        &mut self,
        node_id: &SharedString,
        view_data: &DesignMediaPaintViewData,
    ) {
        self.media_crop_lifecycle
            .reconcile_host_snapshot(node_id, view_data);
    }

    pub(super) fn begin_property_edit(
        &mut self,
        property: DesignPanelProperty,
        original: DesignPanelValue,
    ) -> Option<DesignPropertyEditEvent> {
        self.property_lifecycle.begin(property, original)
    }

    pub(super) fn preview_property_edit(
        &self,
        property: DesignPanelProperty,
        value: DesignPanelValue,
    ) -> Option<DesignPropertyEditEvent> {
        self.property_lifecycle.preview(property, value)
    }

    pub(super) fn commit_property_edit(
        &mut self,
        property: DesignPanelProperty,
        value: DesignPanelValue,
    ) -> Option<DesignPropertyEditEvent> {
        self.property_lifecycle.commit(property, value)
    }

    /// Cancels an active edit because of either explicit user intent or a host
    /// echo that invalidated its target/access state.
    pub(super) fn cancel_property_edit(&mut self) -> Option<DesignPropertyEditEvent> {
        self.property_lifecycle.cancel()
    }

    /// Rebase the complete ordinary-property edit transaction, including its
    /// draft editor, an active scrub, slider ownership, and focus origin.
    ///
    /// This is deliberately the only place that changes an in-flight edit's
    /// property identity. Stable-ID resolution remains in the relevant Design
    /// section/controller and feeds this method a typed previous/next pair.
    pub(super) fn rebase_property_edit(
        &mut self,
        previous: DesignPanelProperty,
        next: DesignPanelProperty,
    ) -> bool {
        let lifecycle_rebased = self.property_lifecycle.rebase(previous, next);
        let mut presentation_rebased = false;
        if let Some(editor) = self
            .property
            .as_mut()
            .filter(|editor| editor.property == previous)
        {
            editor.property = next;
            presentation_rebased = true;
        }
        if let Some(scrub) = self
            .numeric_scrub
            .as_mut()
            .filter(|scrub| scrub.property == previous)
        {
            scrub.property = next;
            presentation_rebased = true;
        }
        if self.draw_slider_property == Some(previous) {
            self.draw_slider_property = Some(next);
            presentation_rebased = true;
        }
        if let Some(return_focus) = self.focus_return.as_mut() {
            let previous_origin = return_focus.origin.clone();
            rebase_focus_origin(&mut return_focus.origin, previous, next);
            presentation_rebased |= return_focus.origin != previous_origin;
        }
        lifecycle_rebased || presentation_rebased
    }

    /// Apply a host-echo validity decision as one transaction-level update.
    /// A cancellation consumes the lifecycle and clears its coupled transient
    /// state, so a later blur or pointer-up cannot emit a second terminal event.
    pub(super) fn reconcile_property_edit(
        &mut self,
        reconciliation: DesignPropertyEditReconciliation,
    ) -> DesignPropertyEditReconciliationResult {
        match reconciliation {
            DesignPropertyEditReconciliation::Keep => DesignPropertyEditReconciliationResult::Kept,
            DesignPropertyEditReconciliation::Rebase { previous, next } => {
                self.rebase_property_edit(previous, next);
                DesignPropertyEditReconciliationResult::Rebased { previous, next }
            }
            DesignPropertyEditReconciliation::Cancel => {
                let terminal_event = self.cancel_property_transaction();
                DesignPropertyEditReconciliationResult::Cancelled(terminal_event)
            }
        }
    }

    /// Consume the ordinary-property lifecycle and clear all presentation
    /// state coupled to it. The returned event is the sole terminal action the
    /// facade may emit for this cancellation.
    pub(super) fn cancel_property_transaction(&mut self) -> Option<DesignPropertyEditEvent> {
        if let Some(editor) = self.property.as_mut() {
            debug_assert_eq!(
                editor.finish_controlled(false, None),
                Some(crate::molecules::InspectorEditPhase::Cancel)
            );
        }
        let cancellation = self.cancel_property_edit();
        let focus_property = self
            .property
            .as_ref()
            .map(|editor| editor.property)
            .or_else(|| self.numeric_scrub.as_ref().map(|scrub| scrub.property))
            .or_else(|| cancellation.as_ref().map(|event| event.property));
        self.property = None;
        self.numeric_scrub = None;
        self.draw_slider_property = None;
        self.draw_slider_field = None;
        self.vector_target_ids = None;
        self.property_invalid = false;
        self.suppress_property_input_change = false;
        if let Some(property) = focus_property
            && self.focus_return.as_ref().is_some_and(|return_focus| {
                focus_origin_matches_property(&return_focus.origin, property)
            })
        {
            self.focus_return = None;
        }
        cancellation
    }

    pub(super) fn has_property_edit(&self) -> bool {
        self.property_lifecycle.active.is_some()
    }

    pub(super) fn active_property_edit_property(&self) -> Option<DesignPanelProperty> {
        self.property_lifecycle
            .active
            .as_ref()
            .map(|active| active.property)
    }

    pub(super) fn has_active_session(&self) -> bool {
        self.has_property_edit()
            || self.paint_lifecycle.has_active_transaction()
            || self.media_crop_lifecycle.has_active_transaction()
            || self.component_authoring.has_active_session()
            || self.property.is_some()
            || self.numeric_scrub.is_some()
            || self.draw_slider_property.is_some()
            || self.grid_dimensions_lifecycle.has_active_transaction()
            || self.variable_font_axis.is_some()
            || self.variable_font_axis_scrub.is_some()
            || self.component_multiline.is_some()
    }

    /// Drops only presentation state after the facade has emitted any needed
    /// Cancel events. Callers remain responsible for balancing host-facing
    /// edit lifecycles before invoking this method.
    pub(super) fn clear_after_cancellation(&mut self) {
        debug_assert!(
            !self.has_property_edit(),
            "an active property transaction must emit Cancel before presentation state is cleared"
        );
        debug_assert!(
            !self.paint_lifecycle.has_active_transaction(),
            "an active paint transaction must emit Cancel before presentation state is cleared"
        );
        debug_assert!(
            !self.component_authoring.has_active_session(),
            "active component-authoring transactions must emit Cancel before edit state is cleared"
        );
        debug_assert!(
            !self.grid_dimensions_lifecycle.has_active_transaction(),
            "an active Grid-dimensions transaction must emit Cancel before edit state is cleared"
        );
        self.property = None;
        self.numeric_scrub = None;
        self.focus_return = None;
        self.draw_slider_property = None;
        self.draw_slider_field = None;
        self.variable_font_axis = None;
        self.variable_font_axis_scrub = None;
        self.component_multiline = None;
        self.vector_target_ids = None;
        self.property_invalid = false;
        self.variable_font_axis_invalid = false;
        self.suppress_property_input_change = false;
        self.suppress_next_control_activation = false;
        self.component_authoring.clear_after_cancellation();
    }
}

#[cfg(test)]
mod tests {
    use crate::prelude::{DesignMediaCropAspectRatio, DesignPaintTransform};

    use super::{
        ComponentMultilineEditor, DesignComponentPropertyValue, DesignFontAxis,
        DesignGridDimensions, DesignGridDimensionsEditTarget, DesignGridTrackAxis,
        DesignMediaCropAction, DesignMediaCropEditTarget, DesignPaintEdit, DesignPaintProperty,
        DesignPaintValue, DesignPanelCollection, DesignPanelEditPhase, DesignPanelProperty,
        DesignPanelValue, DesignPropertyEditController, DesignPropertyEditReconciliation,
        DesignPropertyEditReconciliationResult, DesignTypographyTarget, PickerEventTarget,
        PropertyEditor, PropertyEditorKind, PropertyEditorSeed, VariableFontAxisEditor,
    };

    fn property_editor(
        property: DesignPanelProperty,
        original: DesignPanelValue,
    ) -> PropertyEditor {
        let (base, draft) = match &original {
            DesignPanelValue::Number(value) => (f64::from(*value), value.to_string()),
            _ => (0., String::new()),
        };
        PropertyEditor::new(PropertyEditorSeed {
            property,
            layout_grid_target: None,
            export_configuration_id: None,
            original: original.clone(),
            value: crate::molecules::InspectorValue::Uniform(original),
            base,
            kind: PropertyEditorKind::Number {
                integer: false,
                clamp: None,
            },
            draft,
        })
    }

    fn crop_target() -> DesignMediaCropEditTarget {
        DesignMediaCropEditTarget::new("node".into(), DesignPanelCollection::Fill, "paint".into())
    }

    fn crop_commit() -> DesignMediaCropAction {
        DesignMediaCropAction::Commit {
            transform: DesignPaintTransform::IDENTITY,
            zoom: 1.,
            aspect_ratio: DesignMediaCropAspectRatio::Free,
        }
    }

    fn paint_target(index: usize, paint_id: &str) -> PickerEventTarget {
        PickerEventTarget {
            node_id: "node".into(),
            collection: DesignPanelCollection::Fill,
            index,
            paint_id: paint_id.into(),
        }
    }

    fn paint_opacity(value: f32) -> DesignPaintEdit {
        DesignPaintEdit {
            property: DesignPaintProperty::Opacity,
            value: DesignPaintValue::Number(value),
        }
    }

    fn grid_target(axis: DesignGridTrackAxis) -> DesignGridDimensionsEditTarget {
        DesignGridDimensionsEditTarget::new("grid".into(), axis)
    }

    fn grid_dimensions(columns: usize, rows: usize) -> DesignGridDimensions {
        DesignGridDimensions { columns, rows }
    }

    #[test]
    fn empty_controller_has_no_document_session() {
        let controller = DesignPropertyEditController::default();
        assert!(!controller.has_active_session());
    }

    #[test]
    fn paint_lifecycle_requires_begin_and_consumes_exactly_one_terminal_phase() {
        let mut controller = DesignPropertyEditController::default();
        let target = paint_target(0, "paint");
        let original = paint_opacity(100.);
        let preview = paint_opacity(75.);

        assert!(!controller.track_paint_edit(&target, &preview, DesignPanelEditPhase::Preview,));
        assert!(controller.track_paint_edit(&target, &original, DesignPanelEditPhase::Begin,));
        assert!(controller.has_active_session());
        assert!(!controller.track_paint_edit(&target, &original, DesignPanelEditPhase::Begin,));
        assert!(controller.track_paint_edit(&target, &preview, DesignPanelEditPhase::Preview,));
        assert!(controller.track_paint_edit(&target, &preview, DesignPanelEditPhase::Commit,));
        assert!(!controller.has_active_session());
        assert!(!controller.track_paint_edit(&target, &original, DesignPanelEditPhase::Cancel,));
    }

    #[test]
    fn paint_host_echo_rebases_a_stable_id_and_cancels_latest_preview_once() {
        let mut controller = DesignPropertyEditController::default();
        let original_target = paint_target(0, "paint");
        let rebased_target = paint_target(2, "paint");
        let original = paint_opacity(100.);
        let preview = paint_opacity(64.);

        assert!(controller.track_paint_edit(
            &original_target,
            &original,
            DesignPanelEditPhase::Begin,
        ));
        assert!(controller.track_paint_edit(
            &original_target,
            &preview,
            DesignPanelEditPhase::Preview,
        ));
        assert!(
            controller
                .reconcile_paint_edit_target(Some(rebased_target.clone()))
                .is_none()
        );
        assert_eq!(
            controller
                .active_paint_edit()
                .expect("the stable paint survives its host reorder")
                .target
                .index,
            2
        );

        let cancel = controller
            .reconcile_paint_edit_target(None)
            .expect("access loss consumes one Cancel payload");
        assert_eq!(cancel.target, rebased_target);
        assert_eq!(cancel.edit, preview);
        assert!(controller.reconcile_paint_edit_target(None).is_none());
        assert!(!controller.has_active_session());
    }

    #[test]
    fn paint_host_echo_rejects_a_stale_or_idless_reordered_target() {
        let mut controller = DesignPropertyEditController::default();
        let stable_target = paint_target(0, "paint-a");
        assert!(controller.track_paint_edit(
            &stable_target,
            &paint_opacity(100.),
            DesignPanelEditPhase::Begin,
        ));
        assert_eq!(
            controller
                .reconcile_paint_edit_target(Some(paint_target(0, "paint-b")))
                .expect("a different stable id cancels the transaction")
                .target,
            stable_target
        );

        let idless_target = paint_target(0, "");
        assert!(controller.track_paint_edit(
            &idless_target,
            &paint_opacity(100.),
            DesignPanelEditPhase::Begin,
        ));
        assert_eq!(
            controller
                .reconcile_paint_edit_target(Some(paint_target(1, "")))
                .expect("id-less targets remain bound to their original row")
                .target,
            idless_target
        );
    }

    #[test]
    fn grid_dimensions_lifecycle_guards_target_and_exactly_one_terminal_phase() {
        let mut controller = DesignPropertyEditController::default();
        let target = grid_target(DesignGridTrackAxis::Column);
        let original = grid_dimensions(2, 2);
        let preview = grid_dimensions(4, 2);

        assert!(
            controller
                .edit_grid_dimensions(
                    target.clone(),
                    original,
                    preview,
                    DesignPanelEditPhase::Preview,
                )
                .is_none(),
            "Preview before Begin is rejected"
        );
        let begin = controller
            .edit_grid_dimensions(
                target.clone(),
                original,
                original,
                DesignPanelEditPhase::Begin,
            )
            .expect("an idle Grid lifecycle accepts Begin");
        assert_eq!(begin.dimensions, original);
        assert_eq!(begin.phase, DesignPanelEditPhase::Begin);
        assert!(
            controller
                .edit_grid_dimensions(
                    target.clone(),
                    original,
                    original,
                    DesignPanelEditPhase::Begin,
                )
                .is_none(),
            "duplicate Begin cannot replace the captured target"
        );
        assert!(
            controller
                .edit_grid_dimensions(
                    grid_target(DesignGridTrackAxis::Row),
                    original,
                    grid_dimensions(2, 4),
                    DesignPanelEditPhase::Preview,
                )
                .is_none(),
            "the other Grid axis cannot borrow an active transaction"
        );
        assert_eq!(
            controller
                .edit_grid_dimensions(
                    target.clone(),
                    original,
                    preview,
                    DesignPanelEditPhase::Preview,
                )
                .expect("the captured target accepts Preview")
                .phase,
            DesignPanelEditPhase::Preview
        );
        let commit = controller
            .edit_grid_dimensions(
                target.clone(),
                original,
                preview,
                DesignPanelEditPhase::Commit,
            )
            .expect("the first terminal phase consumes the Grid transaction");
        assert_eq!(commit.dimensions, preview);
        assert_eq!(commit.phase, DesignPanelEditPhase::Commit);
        assert!(
            controller
                .edit_grid_dimensions(target, original, original, DesignPanelEditPhase::Cancel,)
                .is_none(),
            "a second terminal phase is rejected"
        );

        let atomic_commit = controller
            .edit_grid_dimensions(
                grid_target(DesignGridTrackAxis::Row),
                original,
                grid_dimensions(2, 3),
                DesignPanelEditPhase::Commit,
            )
            .expect("the Grid picker retains its Commit-only contract");
        assert_eq!(atomic_commit.phase, DesignPanelEditPhase::Commit);
        assert!(!controller.has_active_session());
    }

    #[test]
    fn grid_dimensions_host_echo_accepts_expected_value_and_cancels_stale_value_once() {
        let mut controller = DesignPropertyEditController::default();
        let target = grid_target(DesignGridTrackAxis::Column);
        let original = grid_dimensions(2, 2);
        let preview = grid_dimensions(4, 2);
        assert!(
            controller
                .edit_grid_dimensions(
                    target.clone(),
                    original,
                    original,
                    DesignPanelEditPhase::Begin,
                )
                .is_some()
        );
        assert!(
            controller
                .edit_grid_dimensions(target, original, preview, DesignPanelEditPhase::Preview,)
                .is_some()
        );

        assert!(
            controller
                .reconcile_grid_dimensions_edit(&"grid".into(), Some(preview), true, false)
                .is_none(),
            "an accepted latest Preview survives its host echo"
        );
        let cancel = controller
            .reconcile_grid_dimensions_edit(
                &"grid".into(),
                Some(grid_dimensions(3, 2)),
                true,
                false,
            )
            .expect("an unrelated host value consumes one Cancel");
        assert_eq!(cancel.node_id.as_ref(), "grid");
        assert_eq!(cancel.dimensions, original);
        assert_eq!(cancel.phase, DesignPanelEditPhase::Cancel);
        assert!(
            controller
                .reconcile_grid_dimensions_edit(&"grid".into(), Some(original), true, false)
                .is_none(),
            "a consumed host invalidation cannot emit a second Cancel"
        );
    }

    #[test]
    fn grid_dimensions_context_and_axis_access_loss_cancel_the_original_once() {
        for (node_id, columns_editable, rows_editable) in
            [("other", true, true), ("grid", false, true)]
        {
            let mut controller = DesignPropertyEditController::default();
            let original = grid_dimensions(2, 2);
            assert!(
                controller
                    .edit_grid_dimensions(
                        grid_target(DesignGridTrackAxis::Column),
                        original,
                        original,
                        DesignPanelEditPhase::Begin,
                    )
                    .is_some()
            );
            let cancel = controller
                .reconcile_grid_dimensions_edit(
                    &node_id.into(),
                    Some(original),
                    columns_editable,
                    rows_editable,
                )
                .expect("target or active-axis access loss emits Cancel");
            assert_eq!(cancel.dimensions, original);
            assert_eq!(cancel.phase, DesignPanelEditPhase::Cancel);
            assert!(controller.cancel_grid_dimensions_edit().is_none());
        }
    }

    #[test]
    fn variable_axis_guard_rejects_out_of_order_and_duplicate_phases() {
        let axis = DesignFontAxis::new("wght", "Weight", 500., 100., 900., 400.);
        let make_editor = || {
            VariableFontAxisEditor::new("wght".into(), DesignTypographyTarget::WholeLayer, &axis)
        };
        let mut controller = DesignPropertyEditController::default();

        assert!(
            controller
                .preview_variable_font_axis_edit("wght", 510., Some("510"))
                .is_none(),
            "Preview before Begin is rejected"
        );
        assert!(
            controller
                .finish_variable_font_axis_edit(Some(510.))
                .is_none()
        );
        assert_eq!(
            controller
                .begin_variable_font_axis_edit(make_editor(), "500")
                .expect("idle axis accepts Begin")
                .phase,
            DesignPanelEditPhase::Begin
        );
        assert!(
            controller
                .begin_variable_font_axis_edit(make_editor(), "500")
                .is_none(),
            "duplicate Begin cannot replace the active stable tag"
        );
        assert_eq!(
            controller
                .preview_variable_font_axis_edit("wght", 510., Some("510"))
                .expect("Preview follows Begin")
                .phase,
            DesignPanelEditPhase::Preview
        );
        assert_eq!(
            controller
                .finish_variable_font_axis_edit(Some(510.))
                .expect("first terminal phase consumes the axis transaction")
                .phase,
            DesignPanelEditPhase::Commit
        );
        assert!(controller.finish_variable_font_axis_edit(None).is_none());
    }

    #[test]
    fn variable_axis_host_invalidation_cancels_once_with_original_value() {
        let axis = DesignFontAxis::new("wdth", "Width", 96.5, 75., 125., 100.);
        let editor = VariableFontAxisEditor::new(
            "wdth".into(),
            DesignTypographyTarget::SelectedTextRange,
            &axis,
        );
        let mut controller = DesignPropertyEditController::default();
        assert!(
            controller
                .begin_variable_font_axis_edit(editor, "96.5")
                .is_some()
        );
        assert!(
            controller
                .preview_variable_font_axis_edit("wdth", 101., Some("101"))
                .is_some()
        );

        let cancel = controller
            .finish_variable_font_axis_edit(None)
            .expect("host invalidation emits Cancel");
        assert_eq!(cancel.tag.as_ref(), "wdth");
        assert_eq!(cancel.target, DesignTypographyTarget::SelectedTextRange);
        assert_eq!(cancel.value, 96.5);
        assert_eq!(cancel.phase, DesignPanelEditPhase::Cancel);
        assert!(controller.finish_variable_font_axis_edit(None).is_none());
    }

    #[test]
    fn component_multiline_guard_preserves_stable_id_and_one_terminal_phase() {
        let make_editor = || ComponentMultilineEditor::new("description".into(), "Original".into());
        let mut controller = DesignPropertyEditController::default();

        assert!(
            controller
                .preview_component_multiline_edit("description", "Preview".into())
                .is_none()
        );
        assert!(
            controller
                .finish_component_multiline_edit(Some("Commit".into()))
                .is_none()
        );
        assert_eq!(
            controller
                .begin_component_multiline_edit(make_editor(), "Original".into())
                .expect("idle multiline field accepts Begin")
                .phase,
            DesignPanelEditPhase::Begin
        );
        assert!(
            controller
                .begin_component_multiline_edit(make_editor(), "Original".into())
                .is_none(),
            "the lowest-level guard cannot overwrite an active Begin"
        );
        let preview = controller
            .preview_component_multiline_edit("description", "Preview".into())
            .expect("Preview follows Begin");
        assert_eq!(preview.property_id.as_ref(), "description");
        assert_eq!(preview.phase, DesignPanelEditPhase::Preview);
        let cancel = controller
            .finish_component_multiline_edit(None)
            .expect("host invalidation consumes one Cancel");
        assert_eq!(cancel.property_id.as_ref(), "description");
        assert_eq!(
            cancel.value,
            DesignComponentPropertyValue::Text("Original".into())
        );
        assert_eq!(cancel.phase, DesignPanelEditPhase::Cancel);
        assert!(controller.finish_component_multiline_edit(None).is_none());
    }

    #[test]
    fn clearing_resets_transient_validation_and_suppression() {
        let mut controller = DesignPropertyEditController {
            property_invalid: true,
            variable_font_axis_invalid: true,
            suppress_property_input_change: true,
            suppress_next_control_activation: true,
            ..Default::default()
        };
        controller.clear_after_cancellation();
        assert!(!controller.property_invalid);
        assert!(!controller.variable_font_axis_invalid);
        assert!(!controller.suppress_property_input_change);
        assert!(!controller.suppress_next_control_activation);
    }

    #[test]
    fn property_lifecycle_requires_begin_and_consumes_one_terminal_event() {
        let mut controller = DesignPropertyEditController::default();
        let property = DesignPanelProperty::Width;

        assert!(
            controller
                .preview_property_edit(property, DesignPanelValue::Number(11.))
                .is_none()
        );
        let begin = controller
            .begin_property_edit(property, DesignPanelValue::Number(10.))
            .expect("an idle lifecycle accepts Begin");
        assert_eq!(begin.phase, DesignPanelEditPhase::Begin);
        assert!(
            controller
                .begin_property_edit(property, DesignPanelValue::Number(10.))
                .is_none(),
            "a second Begin cannot replace an active transaction"
        );
        let preview = controller
            .preview_property_edit(property, DesignPanelValue::Number(11.))
            .expect("Preview follows Begin");
        assert_eq!(preview.phase, DesignPanelEditPhase::Preview);
        let commit = controller
            .commit_property_edit(property, DesignPanelValue::Number(11.))
            .expect("the first terminal event consumes the transaction");
        assert_eq!(commit.phase, DesignPanelEditPhase::Commit);
        assert!(controller.cancel_property_edit().is_none());
        assert!(
            controller
                .commit_property_edit(property, DesignPanelValue::Number(12.))
                .is_none()
        );
    }

    #[test]
    fn crop_commit_consumes_the_transaction_before_picker_close() {
        let mut controller = DesignPropertyEditController::default();
        let target = crop_target();

        assert!(controller.should_forward_media_crop_action(
            target.clone(),
            &DesignMediaCropAction::Begin,
            false,
        ));
        assert!(controller.should_forward_media_crop_action(
            target.clone(),
            &DesignMediaCropAction::Preview {
                transform: DesignPaintTransform::IDENTITY,
                zoom: 1.25,
                aspect_ratio: DesignMediaCropAspectRatio::Free,
            },
            true,
        ));
        assert!(controller.should_forward_media_crop_action(target.clone(), &crop_commit(), true,));
        assert!(
            !controller.should_forward_media_crop_action(
                target,
                &DesignMediaCropAction::Cancel,
                true,
            ),
            "picker close cannot append Cancel while Commit awaits its host echo"
        );
    }

    #[test]
    fn crop_picker_close_emits_exactly_one_cancel() {
        let mut controller = DesignPropertyEditController::default();
        let target = crop_target();

        assert!(controller.should_forward_media_crop_action(
            target.clone(),
            &DesignMediaCropAction::Begin,
            false,
        ));
        assert!(controller.should_forward_media_crop_action(
            target.clone(),
            &DesignMediaCropAction::Cancel,
            true,
        ));
        assert!(!controller.should_forward_media_crop_action(
            target,
            &DesignMediaCropAction::Cancel,
            true,
        ));
    }

    #[test]
    fn externally_active_crop_picker_close_is_guarded() {
        let mut controller = DesignPropertyEditController::default();
        let target = crop_target();

        assert!(controller.should_forward_media_crop_action(
            target.clone(),
            &DesignMediaCropAction::Cancel,
            true,
        ));
        assert!(!controller.should_forward_media_crop_action(
            target,
            &DesignMediaCropAction::Cancel,
            true,
        ));
    }

    #[test]
    fn crop_context_loss_then_picker_close_does_not_duplicate_cancel() {
        let mut controller = DesignPropertyEditController::default();
        let target = crop_target();

        assert!(controller.should_forward_media_crop_action(
            target.clone(),
            &DesignMediaCropAction::Begin,
            false,
        ));
        assert!(controller.should_forward_media_crop_action(
            target.clone(),
            &DesignMediaCropAction::Cancel,
            true,
        ));
        assert!(!controller.should_forward_media_crop_action(
            target,
            &DesignMediaCropAction::Cancel,
            true,
        ));
    }

    #[test]
    fn host_invalidation_cancels_once_with_original_value_after_rebase() {
        let mut controller = DesignPropertyEditController::default();
        let previous = DesignPanelProperty::LayoutGridSize(1);
        let rebased = DesignPanelProperty::LayoutGridSize(0);
        controller
            .begin_property_edit(previous, DesignPanelValue::Number(8.))
            .expect("Begin");
        assert!(controller.rebase_property_edit(previous, rebased));

        let cancel = controller
            .cancel_property_edit()
            .expect("host invalidation produces one terminal Cancel");
        assert_eq!(cancel.property, rebased);
        assert_eq!(cancel.value, DesignPanelValue::Number(8.));
        assert_eq!(cancel.phase, DesignPanelEditPhase::Cancel);
        assert!(controller.cancel_property_edit().is_none());
    }

    #[test]
    fn host_echo_keep_preserves_the_active_transaction() {
        let mut controller = DesignPropertyEditController::default();
        let property = DesignPanelProperty::Width;
        let original = DesignPanelValue::Number(10.);
        controller
            .begin_property_edit(property, original.clone())
            .expect("Begin");
        controller.property = Some(property_editor(property, original));

        let result = controller.reconcile_property_edit(DesignPropertyEditReconciliation::Keep);

        assert_eq!(result, DesignPropertyEditReconciliationResult::Kept);
        assert_eq!(
            controller.property.as_ref().map(|editor| editor.property),
            Some(property)
        );
        assert_eq!(
            controller
                .commit_property_edit(property, DesignPanelValue::Number(12.))
                .expect("kept transaction can commit")
                .phase,
            DesignPanelEditPhase::Commit
        );
    }

    #[test]
    fn host_echo_rebase_moves_lifecycle_and_presentation_together() {
        let mut controller = DesignPropertyEditController::default();
        let previous = DesignPanelProperty::LayoutGridSize(1);
        let next = DesignPanelProperty::LayoutGridSize(0);
        let original = DesignPanelValue::Number(8.);
        controller
            .begin_property_edit(previous, original.clone())
            .expect("Begin");
        controller.property = Some(property_editor(previous, original));

        let result = controller
            .reconcile_property_edit(DesignPropertyEditReconciliation::Rebase { previous, next });

        assert_eq!(
            result,
            DesignPropertyEditReconciliationResult::Rebased { previous, next }
        );
        assert_eq!(
            controller.property.as_ref().map(|editor| editor.property),
            Some(next)
        );
        assert!(
            controller
                .commit_property_edit(previous, DesignPanelValue::Number(9.))
                .is_none(),
            "the stale index no longer owns the transaction"
        );
        assert_eq!(
            controller
                .commit_property_edit(next, DesignPanelValue::Number(9.))
                .expect("the stable target commits at its rebased index")
                .phase,
            DesignPanelEditPhase::Commit
        );
    }

    #[test]
    fn host_echo_cancel_emits_exactly_one_terminal_event_and_clears_draft() {
        let mut controller = DesignPropertyEditController::default();
        let property = DesignPanelProperty::Width;
        let original = DesignPanelValue::Number(10.);
        controller
            .begin_property_edit(property, original.clone())
            .expect("Begin");
        controller.property = Some(property_editor(property, original.clone()));
        controller.property_invalid = true;

        let result = controller.reconcile_property_edit(DesignPropertyEditReconciliation::Cancel);

        let DesignPropertyEditReconciliationResult::Cancelled(Some(terminal)) = result else {
            panic!("active Begin receives one terminal Cancel");
        };
        assert_eq!(terminal.phase, DesignPanelEditPhase::Cancel);
        assert_eq!(terminal.property, property);
        assert_eq!(terminal.value, original);
        assert!(controller.property.is_none());
        assert!(!controller.property_invalid);
        assert_eq!(
            controller.reconcile_property_edit(DesignPropertyEditReconciliation::Cancel),
            DesignPropertyEditReconciliationResult::Cancelled(None),
            "a second host echo cannot emit a second terminal event"
        );
    }
}
