//! Controlled inspector fields, typed edit phases, and transient draft state.

use gpui::{
    App, Div, ElementId, Hsla, SharedString, Stateful, Styled as _, prelude::FluentBuilder as _,
};

use super::{
    InspectorControlledEdit, InspectorEdit, InspectorFieldPresentation, InspectorMetrics,
    InspectorValue, inspector_field_frame_with_presentation,
};

/// Controlled, domain-neutral field state plus its orthogonal presentation.
///
/// The frame does not retain a document value. Hosts replace [`value`](Self::value)
/// through [`sync`](Self::sync); specialized fields below only retain the
/// transient interaction that exists between begin and commit/cancel.
#[derive(Clone, Debug, PartialEq)]
pub struct InspectorFieldFrame<T> {
    value: InspectorValue<T>,
    presentation: InspectorFieldPresentation,
}

impl<T> InspectorFieldFrame<T> {
    pub fn new(value: InspectorValue<T>, presentation: InspectorFieldPresentation) -> Self {
        Self {
            value,
            presentation,
        }
    }

    pub const fn value(&self) -> &InspectorValue<T> {
        &self.value
    }

    pub const fn presentation(&self) -> &InspectorFieldPresentation {
        &self.presentation
    }

    pub const fn is_editable(&self) -> bool {
        self.presentation.access.is_editable()
    }

    /// Reconciles the next host snapshot without creating local document
    /// state. Specialized fields keep an already-active transient draft.
    pub fn sync(&mut self, value: InspectorValue<T>, presentation: InspectorFieldPresentation) {
        self.value = value;
        self.presentation = presentation;
    }

    /// Produces a terminal one-shot edit for controls such as pickers,
    /// toggles, and checkboxes. Read-only and disabled access are both gated.
    pub fn commit(&self, value: T) -> Option<InspectorControlledEdit<T>> {
        self.is_editable()
            .then(|| InspectorEdit::commit(InspectorValue::Uniform(value)))
    }

    /// Renders the common field chrome while preserving value semantics in
    /// this controlled object rather than encoding them in a `Div` alias.
    pub fn render(
        &self,
        id: impl Into<ElementId>,
        metrics: InspectorMetrics,
        cx: &App,
    ) -> Stateful<Div> {
        inspector_field_frame_with_presentation(id, &self.presentation, metrics, cx)
    }
}

#[derive(Clone, Debug, PartialEq)]
struct InspectorFieldSession<T> {
    original: InspectorValue<T>,
    current: InspectorValue<T>,
    last_preview: Option<InspectorValue<T>>,
}

impl<T: Clone + PartialEq> InspectorFieldSession<T> {
    fn begin(original: InspectorValue<T>, initial: T) -> (Self, InspectorControlledEdit<T>) {
        (
            Self {
                original,
                current: InspectorValue::Uniform(initial.clone()),
                last_preview: None,
            },
            InspectorEdit::begin(InspectorValue::Uniform(initial)),
        )
    }

    fn preview(&mut self, value: InspectorValue<T>) -> Option<InspectorControlledEdit<T>> {
        self.current = value.clone();
        if self.last_preview.as_ref() == Some(&value) {
            return None;
        }
        self.last_preview = Some(value.clone());
        Some(InspectorEdit::preview(value))
    }

    fn commit(self) -> InspectorControlledEdit<T> {
        InspectorEdit::commit(self.current)
    }

    fn cancel(self) -> InspectorControlledEdit<T> {
        InspectorEdit::cancel(self.original)
    }
}

/// Controlled exact-text field with a transient draft.
#[derive(Clone, Debug, PartialEq)]
pub struct InspectorTextField {
    frame: InspectorFieldFrame<SharedString>,
    draft: Option<SharedString>,
    session: Option<InspectorFieldSession<SharedString>>,
}

impl InspectorTextField {
    pub fn new(
        value: InspectorValue<SharedString>,
        presentation: InspectorFieldPresentation,
    ) -> Self {
        Self {
            frame: InspectorFieldFrame::new(value, presentation),
            draft: None,
            session: None,
        }
    }

    pub const fn frame(&self) -> &InspectorFieldFrame<SharedString> {
        &self.frame
    }

    pub fn draft(&self) -> Option<&str> {
        self.draft.as_deref()
    }

    pub const fn is_editing(&self) -> bool {
        self.session.is_some()
    }

    pub fn sync(
        &mut self,
        value: InspectorValue<SharedString>,
        presentation: InspectorFieldPresentation,
    ) {
        self.frame.sync(value, presentation);
    }

    pub fn begin(&mut self) -> Option<InspectorControlledEdit<SharedString>> {
        let initial = self.frame.value().as_uniform().cloned().unwrap_or_default();
        self.begin_with(initial)
    }

    pub fn begin_with(
        &mut self,
        initial: impl Into<SharedString>,
    ) -> Option<InspectorControlledEdit<SharedString>> {
        if !self.frame.is_editable() || self.session.is_some() {
            return None;
        }
        let initial = initial.into();
        let (session, begin) =
            InspectorFieldSession::begin(self.frame.value().clone(), initial.clone());
        self.draft = Some(initial);
        self.session = Some(session);
        Some(begin)
    }

    /// Replaces the transient text without emitting a preview. This keeps a
    /// host widget (for example a native text-input primitive) usable as the
    /// renderer while this controlled field remains the draft owner.
    pub fn set_text(&mut self, draft: impl Into<SharedString>) -> bool {
        if self.session.is_none() {
            return false;
        }
        self.draft = Some(draft.into());
        true
    }

    pub fn preview(
        &mut self,
        draft: impl Into<SharedString>,
    ) -> Option<InspectorControlledEdit<SharedString>> {
        let session = self.session.as_mut()?;
        let draft = draft.into();
        self.draft = Some(draft.clone());
        session.preview(InspectorValue::Uniform(draft))
    }

    pub fn commit(&mut self) -> Option<InspectorControlledEdit<SharedString>> {
        self.draft = None;
        self.session.take().map(InspectorFieldSession::commit)
    }

    pub fn cancel(&mut self) -> Option<InspectorControlledEdit<SharedString>> {
        self.draft = None;
        self.session.take().map(InspectorFieldSession::cancel)
    }

    pub fn render(
        &self,
        id: impl Into<ElementId>,
        metrics: InspectorMetrics,
        cx: &App,
    ) -> Stateful<Div> {
        inspector_text_field(id, self.frame.presentation(), metrics, cx)
    }
}

/// Controlled exact-number field. It owns only draft text, parsing,
/// keyboard nudging, and scrubbing for the active interaction.
#[derive(Clone, Debug, PartialEq)]
pub struct InspectorNumberField {
    frame: InspectorFieldFrame<f64>,
    draft: Option<InspectorNumberDraft>,
    session: Option<InspectorFieldSession<f64>>,
}

impl InspectorNumberField {
    pub fn new(value: InspectorValue<f64>, presentation: InspectorFieldPresentation) -> Self {
        Self {
            frame: InspectorFieldFrame::new(value, presentation),
            draft: None,
            session: None,
        }
    }

    pub const fn frame(&self) -> &InspectorFieldFrame<f64> {
        &self.frame
    }

    pub fn draft(&self) -> Option<&str> {
        self.draft.as_ref().map(InspectorNumberDraft::text)
    }

    pub const fn is_editing(&self) -> bool {
        self.session.is_some()
    }

    pub fn sync(&mut self, value: InspectorValue<f64>, presentation: InspectorFieldPresentation) {
        self.frame.sync(value, presentation);
    }

    pub fn begin(&mut self, displayed: impl Into<String>) -> Option<InspectorControlledEdit<f64>> {
        let initial = self.frame.value().as_uniform().copied().unwrap_or(0.);
        self.begin_with(initial, displayed)
    }

    pub fn begin_with(
        &mut self,
        initial: f64,
        displayed: impl Into<String>,
    ) -> Option<InspectorControlledEdit<f64>> {
        if !self.frame.is_editable() || self.session.is_some() || !initial.is_finite() {
            return None;
        }
        let (draft, _) = InspectorNumberDraft::begin(initial, displayed);
        let (session, begin) = InspectorFieldSession::begin(self.frame.value().clone(), initial);
        self.draft = Some(draft);
        self.session = Some(session);
        Some(begin)
    }

    pub fn set_text(&mut self, text: impl Into<String>) -> bool {
        let Some(draft) = self.draft.as_mut() else {
            return false;
        };
        draft.set_text(text);
        true
    }

    pub fn preview_with(
        &mut self,
        parse: impl FnOnce(&str) -> Option<f64>,
    ) -> Option<InspectorControlledEdit<f64>> {
        let value = self.draft.as_mut()?.preview_with(parse)?.value;
        self.session
            .as_mut()?
            .preview(InspectorValue::Uniform(value))
    }

    pub fn nudge_with(
        &mut self,
        delta: f64,
        parse: impl FnOnce(&str) -> Option<f64>,
        normalize: impl FnOnce(f64) -> Option<f64>,
        format: impl FnOnce(f64) -> String,
    ) -> Option<InspectorControlledEdit<f64>> {
        let value = self
            .draft
            .as_mut()?
            .nudge_with(delta, parse, normalize, format)?
            .value;
        self.session
            .as_mut()?
            .preview(InspectorValue::Uniform(value))
    }

    pub fn scrub_to(
        &mut self,
        value: f64,
        format: impl FnOnce(f64) -> String,
    ) -> Option<InspectorControlledEdit<f64>> {
        let value = self.draft.as_mut()?.scrub_to(value, format)?.value;
        self.session
            .as_mut()?
            .preview(InspectorValue::Uniform(value))
    }

    /// Presents an intentionally empty numeric value while keeping absence
    /// distinct from an invalid numeric draft. The surrounding domain decides
    /// whether empty is legal for a particular property.
    pub fn preview_unset(&mut self) -> Option<InspectorControlledEdit<f64>> {
        self.draft.as_mut()?.set_text(String::new());
        self.session.as_mut()?.preview(InspectorValue::Unset)
    }

    pub fn commit_with(
        &mut self,
        parse: impl FnOnce(&str) -> Option<f64>,
    ) -> Option<InspectorControlledEdit<f64>> {
        let outcome = self.draft.take()?.finish(true, parse);
        let session = self.session.take()?;
        Some(match outcome.phase {
            super::InspectorEditPhase::Commit => {
                let mut session = session;
                session.current = InspectorValue::Uniform(outcome.value);
                session.commit()
            }
            super::InspectorEditPhase::Cancel => session.cancel(),
            super::InspectorEditPhase::Begin | super::InspectorEditPhase::Preview => {
                unreachable!("finishing a number draft always emits a terminal edit")
            }
        })
    }

    /// Commits an intentionally empty numeric value. Callers should use this
    /// only after their domain-specific codec has accepted an empty draft.
    pub fn commit_unset(&mut self) -> Option<InspectorControlledEdit<f64>> {
        self.draft = None;
        let mut session = self.session.take()?;
        session.current = InspectorValue::Unset;
        Some(session.commit())
    }

    pub fn cancel(&mut self) -> Option<InspectorControlledEdit<f64>> {
        self.draft = None;
        self.session.take().map(InspectorFieldSession::cancel)
    }

    pub fn render(
        &self,
        id: impl Into<ElementId>,
        metrics: InspectorMetrics,
        cx: &App,
    ) -> Stateful<Div> {
        inspector_number_field(id, self.frame.presentation(), metrics, cx)
    }
}

/// Controlled picker whose selection is an immediate terminal edit.
#[derive(Clone, Debug, PartialEq)]
pub struct InspectorPickerField<T> {
    frame: InspectorFieldFrame<T>,
}

impl<T> InspectorPickerField<T> {
    pub fn new(value: InspectorValue<T>, presentation: InspectorFieldPresentation) -> Self {
        Self {
            frame: InspectorFieldFrame::new(value, presentation),
        }
    }

    pub const fn frame(&self) -> &InspectorFieldFrame<T> {
        &self.frame
    }

    pub fn sync(&mut self, value: InspectorValue<T>, presentation: InspectorFieldPresentation) {
        self.frame.sync(value, presentation);
    }

    pub fn select(&self, value: T) -> Option<InspectorControlledEdit<T>> {
        self.frame.commit(value)
    }

    pub fn render(
        &self,
        id: impl Into<ElementId>,
        metrics: InspectorMetrics,
        cx: &App,
    ) -> Stateful<Div> {
        inspector_picker_field(id, self.frame.presentation(), metrics, cx)
    }
}

/// Controlled bounded direct-manipulation field.
#[derive(Clone, Debug, PartialEq)]
pub struct InspectorSliderField {
    frame: InspectorFieldFrame<f64>,
    session: Option<InspectorFieldSession<f64>>,
}

impl InspectorSliderField {
    pub fn new(value: InspectorValue<f64>, presentation: InspectorFieldPresentation) -> Self {
        Self {
            frame: InspectorFieldFrame::new(value, presentation),
            session: None,
        }
    }

    pub const fn frame(&self) -> &InspectorFieldFrame<f64> {
        &self.frame
    }

    pub fn sync(&mut self, value: InspectorValue<f64>, presentation: InspectorFieldPresentation) {
        self.frame.sync(value, presentation);
    }

    pub fn begin(&mut self, value: f64) -> Option<InspectorControlledEdit<f64>> {
        if !self.frame.is_editable() || self.session.is_some() || !value.is_finite() {
            return None;
        }
        let (session, begin) = InspectorFieldSession::begin(self.frame.value().clone(), value);
        self.session = Some(session);
        Some(begin)
    }

    pub fn preview(&mut self, value: f64) -> Option<InspectorControlledEdit<f64>> {
        if !value.is_finite() {
            return None;
        }
        self.session
            .as_mut()?
            .preview(InspectorValue::Uniform(value))
    }

    pub fn commit(&mut self) -> Option<InspectorControlledEdit<f64>> {
        self.session.take().map(InspectorFieldSession::commit)
    }

    pub fn cancel(&mut self) -> Option<InspectorControlledEdit<f64>> {
        self.session.take().map(InspectorFieldSession::cancel)
    }

    pub fn render(
        &self,
        id: impl Into<ElementId>,
        metrics: InspectorMetrics,
        cx: &App,
    ) -> Stateful<Div> {
        inspector_slider_field(id, self.frame.presentation(), metrics, cx)
    }
}

/// Controlled two-state switch. Mixed values are intentionally not toggled;
/// use [`InspectorCheckboxField`] when an indeterminate state is meaningful.
#[derive(Clone, Debug, PartialEq)]
pub struct InspectorToggleField {
    frame: InspectorFieldFrame<bool>,
}

impl InspectorToggleField {
    pub fn new(value: InspectorValue<bool>, presentation: InspectorFieldPresentation) -> Self {
        Self {
            frame: InspectorFieldFrame::new(value, presentation),
        }
    }

    pub const fn frame(&self) -> &InspectorFieldFrame<bool> {
        &self.frame
    }

    pub fn sync(&mut self, value: InspectorValue<bool>, presentation: InspectorFieldPresentation) {
        self.frame.sync(value, presentation);
    }

    pub fn set(&self, value: bool) -> Option<InspectorControlledEdit<bool>> {
        self.frame.commit(value)
    }

    pub fn toggle(&self) -> Option<InspectorControlledEdit<bool>> {
        let value = match self.frame.value() {
            InspectorValue::Unset => true,
            InspectorValue::Uniform(value) => !value,
            InspectorValue::Mixed => return None,
        };
        self.set(value)
    }

    pub fn render(
        &self,
        id: impl Into<ElementId>,
        metrics: InspectorMetrics,
        cx: &App,
    ) -> Stateful<Div> {
        inspector_toggle_field(id, self.frame.presentation(), metrics, cx)
    }
}

/// Controlled checkbox with an explicit indeterminate mixed state.
#[derive(Clone, Debug, PartialEq)]
pub struct InspectorCheckboxField {
    frame: InspectorFieldFrame<bool>,
}

impl InspectorCheckboxField {
    pub fn new(value: InspectorValue<bool>, presentation: InspectorFieldPresentation) -> Self {
        Self {
            frame: InspectorFieldFrame::new(value, presentation),
        }
    }

    pub const fn frame(&self) -> &InspectorFieldFrame<bool> {
        &self.frame
    }

    pub fn sync(&mut self, value: InspectorValue<bool>, presentation: InspectorFieldPresentation) {
        self.frame.sync(value, presentation);
    }

    pub fn set(&self, value: bool) -> Option<InspectorControlledEdit<bool>> {
        self.frame.commit(value)
    }

    pub fn toggle(&self) -> Option<InspectorControlledEdit<bool>> {
        self.set(match self.frame.value() {
            InspectorValue::Uniform(value) => !value,
            InspectorValue::Unset | InspectorValue::Mixed => true,
        })
    }

    pub fn render(
        &self,
        id: impl Into<ElementId>,
        metrics: InspectorMetrics,
        cx: &App,
    ) -> Stateful<Div> {
        inspector_checkbox_field(id, self.frame.presentation(), metrics, cx)
    }
}

/// Controlled color swatch. Choosing a swatch is an immediate terminal edit.
#[derive(Clone, Debug, PartialEq)]
pub struct InspectorColorSwatch {
    frame: InspectorFieldFrame<Hsla>,
}

impl InspectorColorSwatch {
    pub fn new(value: InspectorValue<Hsla>, presentation: InspectorFieldPresentation) -> Self {
        Self {
            frame: InspectorFieldFrame::new(value, presentation),
        }
    }

    pub const fn frame(&self) -> &InspectorFieldFrame<Hsla> {
        &self.frame
    }

    pub fn sync(&mut self, value: InspectorValue<Hsla>, presentation: InspectorFieldPresentation) {
        self.frame.sync(value, presentation);
    }

    pub fn select(&self, value: Hsla) -> Option<InspectorControlledEdit<Hsla>> {
        self.frame.commit(value)
    }

    pub fn render(&self, fallback: Hsla, metrics: InspectorMetrics, cx: &App) -> Div {
        let color = self.frame.value().as_uniform().copied().unwrap_or(fallback);
        inspector_color_swatch(color, metrics, cx)
            .when(!self.frame.presentation().access.is_editable(), |swatch| {
                swatch.opacity(0.62)
            })
            .when(self.frame.presentation().invalid, |swatch| {
                swatch.border_color(gpui_component::ActiveTheme::theme(cx).red)
            })
    }
}

/// Controlled exact-color field with a transient continuous edit session.
#[derive(Clone, Debug, PartialEq)]
pub struct InspectorColorField {
    frame: InspectorFieldFrame<Hsla>,
    session: Option<InspectorFieldSession<Hsla>>,
}

impl InspectorColorField {
    pub fn new(value: InspectorValue<Hsla>, presentation: InspectorFieldPresentation) -> Self {
        Self {
            frame: InspectorFieldFrame::new(value, presentation),
            session: None,
        }
    }

    pub const fn frame(&self) -> &InspectorFieldFrame<Hsla> {
        &self.frame
    }

    pub fn sync(&mut self, value: InspectorValue<Hsla>, presentation: InspectorFieldPresentation) {
        self.frame.sync(value, presentation);
    }

    pub fn begin(&mut self, value: Hsla) -> Option<InspectorControlledEdit<Hsla>> {
        if !self.frame.is_editable() || self.session.is_some() {
            return None;
        }
        let (session, begin) = InspectorFieldSession::begin(self.frame.value().clone(), value);
        self.session = Some(session);
        Some(begin)
    }

    pub fn preview(&mut self, value: Hsla) -> Option<InspectorControlledEdit<Hsla>> {
        self.session
            .as_mut()?
            .preview(InspectorValue::Uniform(value))
    }

    pub fn commit(&mut self) -> Option<InspectorControlledEdit<Hsla>> {
        self.session.take().map(InspectorFieldSession::commit)
    }

    pub fn cancel(&mut self) -> Option<InspectorControlledEdit<Hsla>> {
        self.session.take().map(InspectorFieldSession::cancel)
    }

    pub fn render(
        &self,
        id: impl Into<ElementId>,
        metrics: InspectorMetrics,
        cx: &App,
    ) -> Stateful<Div> {
        inspector_color_field(id, self.frame.presentation(), metrics, cx)
    }
}

macro_rules! field_recipe {
    ($name:ident, $doc:literal) => {
        #[doc = $doc]
        pub fn $name(
            id: impl Into<ElementId>,
            presentation: &InspectorFieldPresentation,
            metrics: InspectorMetrics,
            cx: &App,
        ) -> Stateful<Div> {
            inspector_field_frame_with_presentation(id, presentation, metrics, cx)
        }
    };
}

field_recipe!(
    inspector_text_field,
    "Exact text-entry field frame. The caller mounts its retained input entity."
);
field_recipe!(
    inspector_number_field,
    "Exact numeric-entry field frame, distinct from direct range manipulation."
);
field_recipe!(
    inspector_picker_field,
    "Trigger field for choosing one value from a known option collection."
);
field_recipe!(
    inspector_slider_field,
    "Frame for bounded direct manipulation; exact entry belongs in a number field."
);
field_recipe!(
    inspector_toggle_field,
    "Binary immediate setting frame. Mixed state should use a checkbox instead."
);
field_recipe!(
    inspector_checkbox_field,
    "Boolean field frame that may render an indeterminate mixed state."
);
field_recipe!(
    inspector_color_field,
    "Exact color-entry field frame; a swatch or picker may be composed beside it."
);

/// Domain-neutral color preview. Selection behavior belongs to a surrounding
/// field or collection row.
pub fn inspector_color_swatch(color: Hsla, metrics: InspectorMetrics, cx: &App) -> Div {
    gpui::div()
        .size(metrics.row_height)
        .flex_none()
        .rounded(metrics.radius)
        .border_1()
        .border_color(gpui_component::ActiveTheme::theme(cx).border)
        .bg(color)
}

/// Transient draft for an exact numeric field.
///
/// Parsing and domain clamping are injected by the caller. Consuming
/// [`finish`](Self::finish) guarantees that one draft can produce only one
/// terminal edit.
#[derive(Clone, Debug, PartialEq)]
pub struct InspectorNumberDraft {
    original: f64,
    text: String,
    last_preview: Option<f64>,
}

impl InspectorNumberDraft {
    pub fn begin(original: f64, displayed: impl Into<String>) -> (Self, InspectorEdit<f64>) {
        (
            Self {
                original,
                text: displayed.into(),
                last_preview: None,
            },
            InspectorEdit::begin(original),
        )
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn set_text(&mut self, text: impl Into<String>) {
        self.text = text.into();
    }

    pub fn preview_with(
        &mut self,
        parse: impl FnOnce(&str) -> Option<f64>,
    ) -> Option<InspectorEdit<f64>> {
        let value = parse(&self.text).filter(|value| value.is_finite())?;
        self.preview_value(value)
    }

    /// Records a direct-manipulation candidate, such as a scrub or slider
    /// update, while coalescing repeated previews from one transaction.
    pub fn preview_value(&mut self, value: f64) -> Option<InspectorEdit<f64>> {
        if !value.is_finite() {
            return None;
        }
        if self.last_preview == Some(value) {
            return None;
        }
        self.last_preview = Some(value);
        Some(InspectorEdit::preview(value))
    }

    /// Applies one keyboard nudge to the current draft.
    ///
    /// Parsing, domain normalization, and formatting remain injected so the
    /// field owns the interaction without learning a document property's
    /// units or legal range.
    pub fn nudge_with(
        &mut self,
        delta: f64,
        parse: impl FnOnce(&str) -> Option<f64>,
        normalize: impl FnOnce(f64) -> Option<f64>,
        format: impl FnOnce(f64) -> String,
    ) -> Option<InspectorEdit<f64>> {
        if !delta.is_finite() {
            return None;
        }
        let base = parse(&self.text)
            .filter(|value| value.is_finite())
            .or(self.last_preview)
            .unwrap_or(self.original);
        let value = normalize(base + delta).filter(|value| value.is_finite())?;
        self.text = format(value);
        self.preview_value(value)
    }

    /// Updates the visible draft from a scrub candidate and emits its
    /// coalesced preview.
    pub fn scrub_to(
        &mut self,
        value: f64,
        format: impl FnOnce(f64) -> String,
    ) -> Option<InspectorEdit<f64>> {
        if !value.is_finite() {
            return None;
        }
        self.text = format(value);
        self.preview_value(value)
    }

    pub fn finish(
        self,
        commit: bool,
        parse: impl FnOnce(&str) -> Option<f64>,
    ) -> InspectorEdit<f64> {
        if commit && let Some(value) = parse(&self.text).filter(|value| value.is_finite()) {
            return InspectorEdit::commit(value);
        }
        InspectorEdit::cancel(self.original)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::molecules::{InspectorEditPhase, InspectorFieldAccess};

    fn editable() -> InspectorFieldPresentation {
        InspectorFieldPresentation::new(InspectorFieldAccess::Editable)
    }

    #[test]
    fn controlled_frame_keeps_value_access_binding_and_validation_orthogonal() {
        let presentation = InspectorFieldPresentation::new(InspectorFieldAccess::read_only(Some(
            "Inherited token".into(),
        )))
        .bound(true)
        .invalid(true)
        .message(Some("Resolve the token".into()));
        let mut field = InspectorFieldFrame::new(InspectorValue::<u8>::Mixed, presentation);

        assert_eq!(field.value(), &InspectorValue::Mixed);
        assert!(field.presentation().bound);
        assert!(field.presentation().invalid);
        assert_eq!(
            field.presentation().message.as_deref(),
            Some("Resolve the token")
        );
        assert!(!field.is_editable());
        assert_eq!(field.commit(3), None);

        field.sync(InspectorValue::Unset, editable());
        assert_eq!(field.value(), &InspectorValue::Unset);
        assert_eq!(
            field.commit(3),
            Some(InspectorEdit::commit(InspectorValue::Uniform(3)))
        );
    }

    #[test]
    fn text_field_balances_a_mixed_edit_and_preserves_original_on_cancel() {
        let mut field = InspectorTextField::new(InspectorValue::Mixed, editable());
        assert_eq!(
            field.begin_with("first"),
            Some(InspectorEdit::begin(InspectorValue::Uniform(
                SharedString::from("first")
            )))
        );
        assert!(field.is_editing());
        assert_eq!(field.draft(), Some("first"));
        assert_eq!(
            field.preview("second"),
            Some(InspectorEdit::preview(InspectorValue::Uniform(
                SharedString::from("second")
            )))
        );
        assert_eq!(field.preview("second"), None, "previews are coalesced");

        field.sync(
            InspectorValue::Uniform("host echo".into()),
            InspectorFieldPresentation::new(InspectorFieldAccess::read_only(Some(
                "Host locked this field".into(),
            ))),
        );
        assert_eq!(field.draft(), Some("second"));
        assert_eq!(
            field.cancel(),
            Some(InspectorEdit::cancel(InspectorValue::Mixed)),
            "a host echo must not replace the transaction's original value"
        );
        assert_eq!(field.cancel(), None, "one edit has one terminal event");
        assert!(!field.is_editing());
    }

    #[test]
    fn exact_number_field_owns_draft_nudging_scrubbing_and_terminal_phase() {
        let mut field = InspectorNumberField::new(InspectorValue::Mixed, editable());
        assert_eq!(
            field.begin_with(10., "10"),
            Some(InspectorEdit::begin(InspectorValue::Uniform(10.)))
        );
        assert_eq!(
            field.nudge_with(
                1.,
                |text| text.parse().ok(),
                |value| Some(value.clamp(0., 20.)),
                |value| value.to_string(),
            ),
            Some(InspectorEdit::preview(InspectorValue::Uniform(11.)))
        );
        assert_eq!(field.draft(), Some("11"));
        assert_eq!(
            field.scrub_to(18., |value| value.to_string()),
            Some(InspectorEdit::preview(InspectorValue::Uniform(18.)))
        );
        assert_eq!(field.scrub_to(18., |value| value.to_string()), None);
        assert_eq!(
            field.commit_with(|text| text.parse().ok()),
            Some(InspectorEdit::commit(InspectorValue::Uniform(18.)))
        );
        assert_eq!(field.commit_with(|text| text.parse().ok()), None);
    }

    #[test]
    fn exact_number_field_preserves_an_intentionally_empty_value() {
        let mut field = InspectorNumberField::new(InspectorValue::Unset, editable());
        assert_eq!(
            field.begin_with(0., ""),
            Some(InspectorEdit::begin(InspectorValue::Uniform(0.)))
        );
        assert_eq!(
            field.preview_unset(),
            Some(InspectorEdit::preview(InspectorValue::Unset))
        );
        assert_eq!(field.preview_unset(), None, "empty previews are coalesced");
        assert_eq!(
            field.commit_unset(),
            Some(InspectorEdit::commit(InspectorValue::Unset))
        );
        assert_eq!(
            field.cancel(),
            None,
            "the empty commit is the sole terminal event"
        );
    }

    #[test]
    fn invalid_number_commit_cancels_back_to_unset() {
        let mut field = InspectorNumberField::new(InspectorValue::Unset, editable());
        assert!(field.begin_with(12., "12").is_some());
        assert!(field.set_text("NaN"));
        assert_eq!(
            field.commit_with(|text| text.parse().ok()),
            Some(InspectorEdit::cancel(InspectorValue::Unset))
        );
        assert!(!field.set_text("13"));
    }

    #[test]
    fn continuous_slider_coalesces_previews_and_has_one_terminal_event() {
        let mut slider = InspectorSliderField::new(InspectorValue::Uniform(0.25), editable());
        assert_eq!(
            slider.begin(0.25),
            Some(InspectorEdit::begin(InspectorValue::Uniform(0.25)))
        );
        assert_eq!(
            slider.preview(0.5),
            Some(InspectorEdit::preview(InspectorValue::Uniform(0.5)))
        );
        assert_eq!(slider.preview(0.5), None);
        assert_eq!(slider.preview(f64::NAN), None);
        assert_eq!(
            slider.commit(),
            Some(InspectorEdit::commit(InspectorValue::Uniform(0.5)))
        );
        assert_eq!(slider.cancel(), None);

        let mut mixed = InspectorSliderField::new(InspectorValue::Mixed, editable());
        assert!(mixed.begin(0.75).is_some());
        assert_eq!(
            mixed.cancel(),
            Some(InspectorEdit::cancel(InspectorValue::Mixed))
        );
    }

    #[test]
    fn immediate_fields_gate_access_and_define_mixed_boolean_behavior() {
        let picker =
            InspectorPickerField::new(InspectorValue::Uniform(SharedString::from("A")), editable());
        assert_eq!(
            picker.select(SharedString::from("B")),
            Some(InspectorEdit::commit(InspectorValue::Uniform(
                SharedString::from("B")
            )))
        );
        let read_only_picker = InspectorPickerField::new(
            InspectorValue::Uniform(SharedString::from("A")),
            InspectorFieldPresentation::new(InspectorFieldAccess::read_only(Some(
                "Inherited".into(),
            ))),
        );
        assert_eq!(read_only_picker.select(SharedString::from("B")), None);

        let toggle = InspectorToggleField::new(InspectorValue::Mixed, editable());
        assert_eq!(
            toggle.toggle(),
            None,
            "switches do not resolve mixed values"
        );
        let unset_toggle = InspectorToggleField::new(InspectorValue::Unset, editable());
        assert_eq!(
            unset_toggle.toggle(),
            Some(InspectorEdit::commit(InspectorValue::Uniform(true)))
        );
        let checkbox = InspectorCheckboxField::new(InspectorValue::Mixed, editable());
        assert_eq!(
            checkbox.toggle(),
            Some(InspectorEdit::commit(InspectorValue::Uniform(true)))
        );

        let disabled = InspectorCheckboxField::new(
            InspectorValue::Uniform(false),
            InspectorFieldPresentation::new(InspectorFieldAccess::disabled(Some(
                "Unavailable".into(),
            ))),
        );
        assert_eq!(disabled.toggle(), None);
    }

    #[test]
    fn color_fields_support_immediate_and_continuous_edit_contracts() {
        let red = Hsla {
            h: 0.,
            s: 1.,
            l: 0.5,
            a: 1.,
        };
        let blue = Hsla {
            h: 2. / 3.,
            s: 1.,
            l: 0.5,
            a: 1.,
        };
        let swatch = InspectorColorSwatch::new(InspectorValue::Uniform(red), editable());
        assert_eq!(
            swatch.select(blue),
            Some(InspectorEdit::commit(InspectorValue::Uniform(blue)))
        );

        let mut field = InspectorColorField::new(InspectorValue::Mixed, editable());
        assert_eq!(
            field.begin(red),
            Some(InspectorEdit::begin(InspectorValue::Uniform(red)))
        );
        assert_eq!(
            field.preview(blue),
            Some(InspectorEdit::preview(InspectorValue::Uniform(blue)))
        );
        assert_eq!(field.preview(blue), None);
        assert_eq!(
            field.cancel(),
            Some(InspectorEdit::cancel(InspectorValue::Mixed))
        );
    }

    #[test]
    fn read_only_and_disabled_exact_fields_cannot_begin_transactions() {
        for access in [
            InspectorFieldAccess::read_only(Some("Bound".into())),
            InspectorFieldAccess::disabled(Some("Unavailable".into())),
        ] {
            let presentation = InspectorFieldPresentation::new(access);
            let mut text = InspectorTextField::new(
                InspectorValue::Uniform(SharedString::from("value")),
                presentation.clone(),
            );
            assert_eq!(text.begin(), None);
            let mut number =
                InspectorNumberField::new(InspectorValue::Uniform(2.), presentation.clone());
            assert_eq!(number.begin("2"), None);
            let mut slider =
                InspectorSliderField::new(InspectorValue::Uniform(2.), presentation.clone());
            assert_eq!(slider.begin(2.), None);
            let mut color =
                InspectorColorField::new(InspectorValue::Uniform(Hsla::default()), presentation);
            assert_eq!(color.begin(Hsla::default()), None);
        }
    }

    #[test]
    fn number_draft_emits_one_begin_and_one_terminal_when_consumed() {
        let (mut draft, begin) = InspectorNumberDraft::begin(12., "12");
        assert_eq!(begin.phase, InspectorEditPhase::Begin);
        draft.set_text("18");
        assert_eq!(
            draft.preview_with(|text| text.parse().ok()),
            Some(InspectorEdit::preview(18.))
        );
        assert_eq!(
            draft.preview_with(|text| text.parse().ok()),
            None,
            "unchanged previews are coalesced"
        );
        assert_eq!(
            draft.finish(true, |text| text.parse().ok()),
            InspectorEdit::commit(18.)
        );
    }

    #[test]
    fn invalid_or_cancelled_number_draft_restores_original() {
        let (mut draft, _) = InspectorNumberDraft::begin(12., "12");
        draft.set_text("not a number");
        assert_eq!(
            draft.finish(true, |text| text.parse().ok()),
            InspectorEdit::cancel(12.)
        );
    }

    #[test]
    fn cancelling_after_a_preview_restores_the_original() {
        let (mut draft, _) = InspectorNumberDraft::begin(12., "12");
        draft.set_text("18");
        assert_eq!(
            draft.preview_with(|text| text.parse().ok()),
            Some(InspectorEdit::preview(18.))
        );
        assert_eq!(
            draft.finish(false, |text| text.parse().ok()),
            InspectorEdit::cancel(12.)
        );
    }

    #[test]
    fn invalid_and_non_finite_previews_do_not_poison_a_later_value() {
        let (mut draft, _) = InspectorNumberDraft::begin(12., "12");
        for invalid in ["invalid", "NaN", "inf", "-inf"] {
            draft.set_text(invalid);
            assert_eq!(draft.preview_with(|text| text.parse().ok()), None);
            assert_eq!(draft.last_preview, None);
        }

        draft.set_text("18");
        assert_eq!(
            draft.preview_with(|text| text.parse().ok()),
            Some(InspectorEdit::preview(18.))
        );
        draft.set_text("19");
        assert_eq!(
            draft.preview_with(|text| text.parse().ok()),
            Some(InspectorEdit::preview(19.))
        );
    }

    #[test]
    fn invalid_commit_after_a_valid_preview_cancels_the_transaction() {
        let (mut draft, _) = InspectorNumberDraft::begin(12., "12");
        draft.set_text("18");
        assert!(draft.preview_with(|text| text.parse().ok()).is_some());
        draft.set_text("NaN");
        assert_eq!(
            draft.finish(true, |text| text.parse().ok()),
            InspectorEdit::cancel(12.)
        );
    }

    #[test]
    fn caller_supplied_parsing_and_clamping_define_the_candidate() {
        let parse_percent = |text: &str| {
            text.strip_suffix('%')
                .and_then(|number| number.parse::<f64>().ok())
                .map(|value| value.clamp(0., 100.))
        };
        let (mut draft, _) = InspectorNumberDraft::begin(50., "50%");
        draft.set_text("140%");
        assert_eq!(
            draft.preview_with(parse_percent),
            Some(InspectorEdit::preview(100.))
        );
        assert_eq!(
            draft.finish(true, parse_percent),
            InspectorEdit::commit(100.)
        );
    }

    #[test]
    fn keyboard_nudging_and_scrubbing_share_one_coalesced_preview_stream() {
        let (mut draft, _) = InspectorNumberDraft::begin(12., "12");
        assert_eq!(
            draft.nudge_with(
                1.,
                |text| text.parse().ok(),
                |value| Some(value.clamp(0., 20.)),
                |value| value.to_string(),
            ),
            Some(InspectorEdit::preview(13.))
        );
        assert_eq!(draft.text(), "13");
        assert_eq!(
            draft.scrub_to(18., |value| value.to_string()),
            Some(InspectorEdit::preview(18.))
        );
        assert_eq!(draft.scrub_to(18., |value| value.to_string()), None);
        assert_eq!(draft.text(), "18");
        assert_eq!(
            draft.finish(true, |text| text.parse().ok()),
            InspectorEdit::commit(18.)
        );
    }

    #[test]
    fn nudge_and_scrub_reject_non_finite_candidates() {
        let (mut draft, _) = InspectorNumberDraft::begin(12., "invalid");
        assert_eq!(
            draft.nudge_with(
                f64::NAN,
                |text| text.parse().ok(),
                Some,
                |value| value.to_string(),
            ),
            None
        );
        assert_eq!(
            draft.scrub_to(f64::INFINITY, |value| value.to_string()),
            None
        );
        assert_eq!(draft.text(), "invalid");
    }
}
