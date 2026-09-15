//! Compact, padded feedback and empty-state presentation for inspectors.

use gpui::{
    AnyElement, App, IntoElement, ParentElement as _, RenderOnce, SharedString, Styled as _,
    prelude::FluentBuilder as _,
};
use gpui_component::{ActiveTheme as _, StyledExt as _, v_flex};

use super::InspectorMetrics;

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum InspectorFeedbackKind {
    #[default]
    Neutral,
    Info,
    Warning,
    Error,
}

/// Inline message associated with one field rather than the whole section.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum InspectorFieldMessageKind {
    #[default]
    Help,
    Validation,
}

#[derive(IntoElement)]
pub struct InspectorFieldMessage {
    message: SharedString,
    kind: InspectorFieldMessageKind,
}

impl InspectorFieldMessage {
    pub fn help(message: impl Into<SharedString>) -> Self {
        Self {
            message: message.into(),
            kind: InspectorFieldMessageKind::Help,
        }
    }

    pub fn validation(message: impl Into<SharedString>) -> Self {
        Self {
            message: message.into(),
            kind: InspectorFieldMessageKind::Validation,
        }
    }
}

impl RenderOnce for InspectorFieldMessage {
    fn render(self, _: &mut gpui::Window, cx: &mut App) -> impl IntoElement {
        gpui::div()
            .w_full()
            .min_w(gpui::px(0.))
            .text_xs()
            .text_color(match self.kind {
                InspectorFieldMessageKind::Help => cx.theme().muted_foreground,
                InspectorFieldMessageKind::Validation => cx.theme().red,
            })
            .child(self.message)
    }
}

/// Non-modal message associated with the current inspector projection.
#[derive(IntoElement)]
pub struct InspectorFeedback {
    title: Option<SharedString>,
    message: SharedString,
    kind: InspectorFeedbackKind,
    action: Option<AnyElement>,
    metrics: InspectorMetrics,
}

impl InspectorFeedback {
    pub fn new(message: impl Into<SharedString>) -> Self {
        Self {
            title: None,
            message: message.into(),
            kind: InspectorFeedbackKind::Neutral,
            action: None,
            metrics: InspectorMetrics::default(),
        }
    }

    pub fn title(mut self, title: impl Into<SharedString>) -> Self {
        self.title = Some(title.into());
        self
    }

    pub const fn kind(mut self, kind: InspectorFeedbackKind) -> Self {
        self.kind = kind;
        self
    }

    pub fn action(mut self, action: impl IntoElement) -> Self {
        self.action = Some(action.into_any_element());
        self
    }

    pub const fn metrics(mut self, metrics: InspectorMetrics) -> Self {
        self.metrics = metrics;
        self
    }
}

impl RenderOnce for InspectorFeedback {
    fn render(self, _: &mut gpui::Window, cx: &mut App) -> impl IntoElement {
        let accent = match self.kind {
            InspectorFeedbackKind::Neutral => cx.theme().muted_foreground,
            InspectorFeedbackKind::Info => cx.theme().blue,
            InspectorFeedbackKind::Warning => cx.theme().warning,
            InspectorFeedbackKind::Error => cx.theme().red,
        };
        v_flex()
            .w_full()
            .min_w(gpui::px(0.))
            .gap_2()
            .p(self.metrics.compact_feedback_padding)
            .rounded(self.metrics.radius)
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().secondary)
            .when_some(self.title, |message, title| {
                message.child(
                    gpui::div()
                        .text_xs()
                        .font_semibold()
                        .text_color(accent)
                        .child(title),
                )
            })
            .child(
                gpui::div()
                    .min_w(gpui::px(0.))
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(self.message),
            )
            .when_some(self.action, |message, action| message.child(action))
    }
}

/// Centered empty/error-state anatomy with guaranteed narrow-panel padding.
#[derive(IntoElement)]
pub struct InspectorEmptyState {
    title: SharedString,
    description: Option<SharedString>,
    actions: Vec<AnyElement>,
    metrics: InspectorMetrics,
}

impl InspectorEmptyState {
    pub fn new(title: impl Into<SharedString>) -> Self {
        Self {
            title: title.into(),
            description: None,
            actions: Vec::new(),
            metrics: InspectorMetrics::default(),
        }
    }

    pub fn description(mut self, description: impl Into<SharedString>) -> Self {
        self.description = Some(description.into());
        self
    }

    pub fn action(mut self, action: impl IntoElement) -> Self {
        self.actions.push(action.into_any_element());
        self
    }

    pub const fn metrics(mut self, metrics: InspectorMetrics) -> Self {
        self.metrics = metrics;
        self
    }
}

impl RenderOnce for InspectorEmptyState {
    fn render(self, _: &mut gpui::Window, cx: &mut App) -> impl IntoElement {
        v_flex()
            .w_full()
            .min_w(gpui::px(0.))
            .items_center()
            .justify_center()
            .gap_3()
            .px(self.metrics.horizontal_padding)
            .py(self.metrics.compact_feedback_padding)
            .text_center()
            .child(gpui::div().text_sm().font_semibold().child(self.title))
            .when_some(self.description, |state, description| {
                state.child(
                    gpui::div()
                        .max_w(gpui::px(360.))
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(description),
                )
            })
            .when(!self.actions.is_empty(), |state| {
                state.child(
                    gpui_component::h_flex()
                        .items_center()
                        .justify_center()
                        .gap_2()
                        .children(self.actions),
                )
            })
    }
}
