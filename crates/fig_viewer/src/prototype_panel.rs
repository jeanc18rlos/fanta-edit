use editor::{Editor, EditorEvent, actions::SelectAll};
use fanta_doc::{
    Action, AnimationClipId, Color as FantaColor, ComponentId, Direction, Doc, Easing, NodeData,
    NodeId, Operation, OverlayPosition, OverlaySettings, PrototypeAnimation, Reaction, ReactionId,
    Transition, TransitionStyle, Trigger, VarValue, VariableId, VariableType,
};
use gpui::{
    AnyElement, App, Context, ElementId, Entity, EventEmitter, FocusHandle, Focusable,
    KeyDownEvent, Render, SharedString, Subscription, Window,
};
use ui::{
    ContextMenu, ContextMenuEntry, Divider, DropdownMenu, DropdownStyle, IconPosition, Switch,
    ToggleState, Tooltip, prelude::*,
};
use util::ResultExt;

use crate::document::{DocChange, FigItem, FigItemEvent};
use crate::inspector_components::{InspectorMessage, InspectorPropertyRow, InspectorSectionHeader};
use crate::prototype_player::prototype_entry_frame;
use crate::view::PlayPrototype;

const DEFAULT_TRANSITION_DURATION_MS: u32 = 300;

#[derive(Clone)]
struct PrototypeTarget {
    id: NodeId,
    name: SharedString,
}

#[derive(Clone)]
struct PrototypeVariable {
    id: VariableId,
    name: SharedString,
}

#[derive(Clone)]
struct PrototypeComponent {
    id: ComponentId,
    name: SharedString,
}

#[derive(Clone)]
struct PrototypeClip {
    id: AnimationClipId,
    name: SharedString,
}

enum PrototypeSnapshot {
    Message(SharedString),
    Selection {
        editable: bool,
        node: NodeId,
        name: SharedString,
        can_start_flow: bool,
        can_present: bool,
        is_flow_start: bool,
        reactions: Vec<Reaction>,
        frame_targets: Vec<PrototypeTarget>,
        scroll_targets: Vec<PrototypeTarget>,
        variables: Vec<PrototypeVariable>,
        components: Vec<PrototypeComponent>,
        clips: Vec<PrototypeClip>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TriggerChoice {
    Click,
    Drag,
    Hover,
    WhilePressing,
    AfterDelay,
    Key,
}

const TRIGGER_CHOICES: [(TriggerChoice, &str); 6] = [
    (TriggerChoice::Click, "On click"),
    (TriggerChoice::Drag, "On drag"),
    (TriggerChoice::Hover, "On hover"),
    (TriggerChoice::WhilePressing, "While pressing"),
    (TriggerChoice::AfterDelay, "After delay"),
    (TriggerChoice::Key, "On key press"),
];

impl TriggerChoice {
    fn from_trigger(trigger: &Trigger) -> Self {
        match trigger {
            Trigger::Click => Self::Click,
            Trigger::Drag => Self::Drag,
            Trigger::Hover => Self::Hover,
            Trigger::WhilePressing => Self::WhilePressing,
            // The hover-family triggers imported from Figma present as the
            // closest editable choice; re-selecting in the panel normalizes.
            Trigger::MouseEnter | Trigger::WhileHovering => Self::Hover,
            Trigger::MouseLeave => Self::Hover,
            Trigger::AfterDelay { .. } => Self::AfterDelay,
            Trigger::Key { .. } => Self::Key,
        }
    }

    fn into_trigger(self) -> Trigger {
        match self {
            Self::Click => Trigger::Click,
            Self::Drag => Trigger::Drag,
            Self::Hover => Trigger::Hover,
            Self::WhilePressing => Trigger::WhilePressing,
            Self::AfterDelay => Trigger::AfterDelay { delay_ms: 300 },
            Self::Key => Trigger::Key {
                keys: vec!["Enter".to_string()],
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ActionChoice {
    Navigate,
    OpenOverlay,
    ScrollTo,
    SetVariable,
    UpdateVariant,
    OpenLink,
    Back,
    Close,
}

const ACTION_CHOICES: [(ActionChoice, &str); 8] = [
    (ActionChoice::Navigate, "Navigate to"),
    (ActionChoice::OpenOverlay, "Open overlay"),
    (ActionChoice::ScrollTo, "Scroll to"),
    (ActionChoice::SetVariable, "Set variable"),
    (ActionChoice::UpdateVariant, "Change variant"),
    (ActionChoice::OpenLink, "Open link"),
    (ActionChoice::Back, "Back"),
    (ActionChoice::Close, "Close"),
];

impl ActionChoice {
    fn from_action(action: &Action) -> Option<Self> {
        match action {
            Action::Navigate { .. } => Some(Self::Navigate),
            Action::OpenOverlay { .. } => Some(Self::OpenOverlay),
            Action::ScrollTo { .. } => Some(Self::ScrollTo),
            Action::SetVariable { .. } => Some(Self::SetVariable),
            Action::UpdateVariant { .. } => Some(Self::UpdateVariant),
            Action::OpenLink { .. } => Some(Self::OpenLink),
            Action::Back => Some(Self::Back),
            Action::Close => Some(Self::Close),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OverlayPositionChoice {
    Center,
    Manual,
    TopLeft,
    TopCenter,
    TopRight,
    BottomLeft,
    BottomCenter,
    BottomRight,
}

const OVERLAY_POSITION_CHOICES: [(OverlayPositionChoice, &str); 8] = [
    (OverlayPositionChoice::Center, "Center"),
    (OverlayPositionChoice::Manual, "Manual"),
    (OverlayPositionChoice::TopLeft, "Top left"),
    (OverlayPositionChoice::TopCenter, "Top center"),
    (OverlayPositionChoice::TopRight, "Top right"),
    (OverlayPositionChoice::BottomLeft, "Bottom left"),
    (OverlayPositionChoice::BottomCenter, "Bottom center"),
    (OverlayPositionChoice::BottomRight, "Bottom right"),
];

impl OverlayPositionChoice {
    fn from_position(position: &OverlayPosition) -> Self {
        match position {
            OverlayPosition::Center => Self::Center,
            OverlayPosition::Manual { .. } => Self::Manual,
            OverlayPosition::TopLeft => Self::TopLeft,
            OverlayPosition::TopCenter => Self::TopCenter,
            OverlayPosition::TopRight => Self::TopRight,
            OverlayPosition::BottomLeft => Self::BottomLeft,
            OverlayPosition::BottomCenter => Self::BottomCenter,
            OverlayPosition::BottomRight => Self::BottomRight,
        }
    }

    fn label(self) -> &'static str {
        OVERLAY_POSITION_CHOICES
            .iter()
            .find_map(|(choice, label)| (*choice == self).then_some(*label))
            .unwrap_or("Center")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ParameterKind {
    Delay,
    AnimationDelay,
    Keys,
    VariableValue,
    Variant,
    Url,
    OverlayOffsetX,
    OverlayOffsetY,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ReactionParameter {
    node: NodeId,
    reaction: ReactionId,
    kind: ParameterKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TransitionChoice {
    Instant,
    Dissolve,
    SmartAnimate,
    SlideIn,
    Push,
    MoveIn,
}

const TRANSITION_CHOICES: [(TransitionChoice, &str); 6] = [
    (TransitionChoice::Instant, "Instant"),
    (TransitionChoice::Dissolve, "Dissolve"),
    (TransitionChoice::SmartAnimate, "Smart animate"),
    (TransitionChoice::SlideIn, "Slide in"),
    (TransitionChoice::Push, "Push"),
    (TransitionChoice::MoveIn, "Move in"),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EasingChoice {
    Linear,
    EaseIn,
    EaseOut,
    EaseInOut,
}

const EASING_CHOICES: [(EasingChoice, &str); 4] = [
    (EasingChoice::Linear, "Linear"),
    (EasingChoice::EaseIn, "Ease in"),
    (EasingChoice::EaseOut, "Ease out"),
    (EasingChoice::EaseInOut, "Ease in and out"),
];

const DURATION_CHOICES: [(u32, &str); 5] = [
    (100, "100 ms"),
    (200, "200 ms"),
    (300, "300 ms"),
    (500, "500 ms"),
    (800, "800 ms"),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DirectionChoice {
    Left,
    Right,
    Up,
    Down,
}

const DIRECTION_CHOICES: [(DirectionChoice, &str); 4] = [
    (DirectionChoice::Left, "Left"),
    (DirectionChoice::Right, "Right"),
    (DirectionChoice::Up, "Up"),
    (DirectionChoice::Down, "Down"),
];

impl TransitionChoice {
    fn from_transition(transition: Option<&Transition>) -> Self {
        match transition.map(|transition| transition.style) {
            None | Some(TransitionStyle::Instant) => Self::Instant,
            Some(TransitionStyle::Dissolve) => Self::Dissolve,
            Some(TransitionStyle::SmartAnimate) => Self::SmartAnimate,
            Some(TransitionStyle::SlideIn { .. }) => Self::SlideIn,
            Some(TransitionStyle::Push { .. }) => Self::Push,
            Some(TransitionStyle::MoveIn { .. }) => Self::MoveIn,
            // Out-transitions and animated scroll present as their nearest
            // editable choice; re-selecting in the panel normalizes.
            Some(TransitionStyle::SlideOut { .. }) => Self::SlideIn,
            Some(TransitionStyle::MoveOut { .. }) => Self::MoveIn,
            Some(TransitionStyle::ScrollAnimate) => Self::SmartAnimate,
        }
    }
}

impl EasingChoice {
    fn from_easing(easing: Easing) -> Option<Self> {
        match easing {
            Easing::Linear => Some(Self::Linear),
            Easing::EaseIn => Some(Self::EaseIn),
            Easing::EaseOut => Some(Self::EaseOut),
            Easing::EaseInOut => Some(Self::EaseInOut),
            Easing::CubicBezier { .. } | Easing::Spring { .. } => None,
        }
    }

    fn into_easing(self) -> Easing {
        match self {
            Self::Linear => Easing::Linear,
            Self::EaseIn => Easing::EaseIn,
            Self::EaseOut => Easing::EaseOut,
            Self::EaseInOut => Easing::EaseInOut,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Linear => "Linear",
            Self::EaseIn => "Ease in",
            Self::EaseOut => "Ease out",
            Self::EaseInOut => "Ease in and out",
        }
    }
}

impl DirectionChoice {
    fn from_direction(direction: Direction) -> Self {
        match direction {
            Direction::Left => Self::Left,
            Direction::Right => Self::Right,
            Direction::Up => Self::Up,
            Direction::Down => Self::Down,
        }
    }

    fn into_direction(self) -> Direction {
        match self {
            Self::Left => Direction::Left,
            Self::Right => Direction::Right,
            Self::Up => Direction::Up,
            Self::Down => Direction::Down,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Left => "Left",
            Self::Right => "Right",
            Self::Up => "Up",
            Self::Down => "Down",
        }
    }
}

pub struct FantaPrototypePanel {
    item: Entity<FigItem>,
    focus_handle: FocusHandle,
    parameter_editor: Option<Entity<Editor>>,
    editing_parameter: Option<ReactionParameter>,
    parameter_edit_baseline: Option<Reaction>,
    parameter_edit_expected: Option<Reaction>,
    parameter_edit_previewed: bool,
    suppress_parameter_editor_events: bool,
    parameter_error: Option<SharedString>,
    parameter_editor_subscription: Option<Subscription>,
    _item_subscription: Subscription,
}

impl FantaPrototypePanel {
    pub fn new(item: Entity<FigItem>, cx: &mut Context<Self>) -> Self {
        let item_subscription =
            cx.subscribe(&item, |this: &mut Self, item, event: &FigItemEvent, cx| {
                let restore = if matches!(event, FigItemEvent::StateChanged) {
                    Some(false)
                } else if matches!(event, FigItemEvent::SourceEditLockChanged)
                    && item.read(cx).source_edit_locked()
                {
                    Some(true)
                } else {
                    None
                };
                if let Some(restore) = restore
                    && this.editing_parameter.is_some()
                {
                    let panel = cx.weak_entity();
                    cx.defer(move |cx| {
                        panel
                            .update(cx, |panel, cx| panel.abandon_parameter_edit(restore, cx))
                            .log_err();
                    });
                }
                cx.notify();
            });
        Self {
            item,
            focus_handle: cx.focus_handle(),
            parameter_editor: None,
            editing_parameter: None,
            parameter_edit_baseline: None,
            parameter_edit_expected: None,
            parameter_edit_previewed: false,
            suppress_parameter_editor_events: false,
            parameter_error: None,
            parameter_editor_subscription: None,
            _item_subscription: item_subscription,
        }
    }

    fn ensure_parameter_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.parameter_editor.is_some() {
            return;
        }
        let editor = cx.new(|cx| Editor::single_line(window, cx));
        let subscription = cx.subscribe_in(
            &editor,
            window,
            |this: &mut Self, _, event: &EditorEvent, _, cx| match event {
                EditorEvent::BufferEdited if this.editing_parameter.is_some() => {
                    this.preview_parameter_edit(cx);
                }
                EditorEvent::Blurred if this.editing_parameter.is_some() => {
                    this.commit_parameter_edit_value(cx);
                }
                _ => {}
            },
        );
        self.parameter_editor = Some(editor);
        self.parameter_editor_subscription = Some(subscription);
    }

    fn start_parameter_edit(
        &mut self,
        parameter: ReactionParameter,
        initial: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.commit_parameter_edit_value(cx) {
            self.abandon_parameter_edit(true, cx);
        }
        let baseline = {
            let item = self.item.read(cx);
            if !item.is_editable() {
                return;
            }
            let Some(document) = item.document() else {
                return;
            };
            reaction_by_id(&document.doc, parameter.node, parameter.reaction).cloned()
        };
        let (Some(editor), Some(baseline)) = (self.parameter_editor.clone(), baseline) else {
            return;
        };
        self.editing_parameter = Some(parameter);
        self.parameter_edit_baseline = Some(baseline.clone());
        self.parameter_edit_expected = Some(baseline);
        self.parameter_edit_previewed = false;
        self.parameter_error = None;
        self.suppress_parameter_editor_events = true;
        editor.update(cx, |editor, cx| {
            editor.set_text(initial, window, cx);
            editor.select_all(&SelectAll, window, cx);
        });
        self.suppress_parameter_editor_events = false;
        editor.focus_handle(cx).focus(window, cx);
        cx.notify();
    }

    fn preview_parameter_edit(&mut self, cx: &mut Context<Self>) {
        if self.suppress_parameter_editor_events {
            return;
        }
        let editable = self.item.read(cx).is_editable();
        if !editable {
            self.abandon_parameter_edit(true, cx);
            return;
        }
        let (Some(parameter), Some(baseline), Some(expected), Some(editor)) = (
            self.editing_parameter,
            self.parameter_edit_baseline.clone(),
            self.parameter_edit_expected.clone(),
            self.parameter_editor.clone(),
        ) else {
            return;
        };
        let text = editor.read(cx).text(cx);
        let reaction = {
            let item = self.item.read(cx);
            let Some(document) = item.document() else {
                self.parameter_error = Some("The document is no longer available".into());
                cx.notify();
                return;
            };
            parameter_reaction_from_text(&document.doc, &baseline, parameter.kind, text.as_ref())
        };
        let reaction = match reaction {
            Ok(reaction) => reaction,
            Err(error) => {
                self.parameter_error = Some(error.into());
                cx.notify();
                return;
            }
        };
        let preview_owner = cx.entity_id();
        let preview_result = self.item.update(cx, |item, cx| {
            if !item.is_editable() {
                return None;
            }
            item.with_document_for_preview_owner(preview_owner, cx, |document| {
                let result = replace_reaction_preview_if_current(
                    &mut document.doc,
                    parameter.node,
                    parameter.reaction,
                    &expected,
                    reaction.clone(),
                );
                let change = if result == Some(true) {
                    DocChange::ContentPreview
                } else {
                    DocChange::None
                };
                (result, change)
            })
            .flatten()
        });
        match preview_result {
            Some(changed) => {
                self.parameter_edit_expected = Some(reaction);
                self.parameter_edit_previewed |= changed;
                self.parameter_error = None;
            }
            None => {
                self.abandon_parameter_edit(false, cx);
                self.parameter_error =
                    Some("The interaction changed elsewhere; reopen the field to continue".into());
            }
        }
        cx.notify();
    }

    fn commit_parameter_edit_value(&mut self, cx: &mut Context<Self>) -> bool {
        if self.suppress_parameter_editor_events {
            return false;
        }
        let editable = self.item.read(cx).is_editable();
        if !editable {
            self.abandon_parameter_edit(true, cx);
            return true;
        }
        let (Some(parameter), Some(baseline), Some(editor)) = (
            self.editing_parameter,
            self.parameter_edit_baseline.clone(),
            self.parameter_editor.clone(),
        ) else {
            return true;
        };
        let text = editor.read(cx).text(cx);
        let reaction = {
            let item = self.item.read(cx);
            let Some(document) = item.document() else {
                self.parameter_error = Some("The document is no longer available".into());
                cx.notify();
                return false;
            };
            parameter_reaction_from_text(&document.doc, &baseline, parameter.kind, text.as_ref())
        };
        let reaction = match reaction {
            Ok(reaction) => reaction,
            Err(error) => {
                self.parameter_error = Some(error.into());
                cx.notify();
                return false;
            }
        };
        let previewed = self.parameter_edit_previewed;
        if previewed && !self.restore_parameter_preview(cx) {
            self.abandon_parameter_edit(false, cx);
            self.parameter_error =
                Some("The interaction changed elsewhere; your preview was not restored".into());
            cx.notify();
            return true;
        }
        let still_matches_baseline = self.item.read(cx).document().is_some_and(|document| {
            reaction_by_id(&document.doc, parameter.node, parameter.reaction) == Some(&baseline)
        });
        if !still_matches_baseline {
            self.abandon_parameter_edit(false, cx);
            self.parameter_error =
                Some("The interaction changed elsewhere; reopen the field to continue".into());
            cx.notify();
            return true;
        }
        self.editing_parameter = None;
        self.parameter_edit_baseline = None;
        self.parameter_edit_expected = None;
        self.parameter_edit_previewed = false;
        self.parameter_error = None;
        let committed = self.apply_operation(
            |doc| {
                if reaction_by_id(doc, parameter.node, parameter.reaction) != Some(&baseline) {
                    return None;
                }
                set_reaction_operation(doc, parameter.node, parameter.reaction, |current| {
                    *current = reaction;
                })
            },
            cx,
        );
        if previewed {
            let preview_owner = cx.entity_id();
            self.item.update(cx, |item, cx| {
                item.finish_content_preview(preview_owner, committed, cx);
            });
        }
        cx.notify();
        true
    }

    pub(crate) fn finish_parameter_edit(&mut self, cx: &mut Context<Self>) {
        if !self.commit_parameter_edit_value(cx) {
            self.abandon_parameter_edit(true, cx);
        }
    }

    fn restore_parameter_preview(&mut self, cx: &mut Context<Self>) -> bool {
        let (Some(parameter), Some(baseline), Some(expected)) = (
            self.editing_parameter,
            self.parameter_edit_baseline.clone(),
            self.parameter_edit_expected.clone(),
        ) else {
            return false;
        };
        let preview_owner = cx.entity_id();
        let result = self.item.update(cx, |item, cx| {
            item.with_document_for_preview_owner(preview_owner, cx, |document| {
                let result = replace_reaction_preview_if_current(
                    &mut document.doc,
                    parameter.node,
                    parameter.reaction,
                    &expected,
                    baseline.clone(),
                );
                let change = if result == Some(true) {
                    DocChange::ContentPreview
                } else {
                    DocChange::None
                };
                (result, change)
            })
            .flatten()
        });
        if result.is_some() {
            self.parameter_edit_expected = Some(baseline);
            true
        } else {
            false
        }
    }

    fn abandon_parameter_edit(&mut self, restore: bool, cx: &mut Context<Self>) {
        let previewed = self.parameter_edit_previewed;
        if restore && previewed {
            self.restore_parameter_preview(cx);
        }
        self.editing_parameter = None;
        self.parameter_edit_baseline = None;
        self.parameter_edit_expected = None;
        self.parameter_edit_previewed = false;
        self.parameter_error = None;
        if previewed {
            let preview_owner = cx.entity_id();
            self.item.update(cx, |item, cx| {
                item.finish_content_preview(preview_owner, false, cx);
            });
        }
        cx.notify();
    }

    fn commit_parameter_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.commit_parameter_edit_value(cx) && self.editing_parameter.is_none() {
            self.focus_handle.focus(window, cx);
        }
    }

    fn cancel_parameter_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.abandon_parameter_edit(true, cx);
        self.focus_handle.focus(window, cx);
    }

    fn handle_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.editing_parameter.is_none() {
            return;
        }
        match event.keystroke.key.as_str() {
            "enter" => {
                cx.stop_propagation();
                self.commit_parameter_edit(window, cx);
            }
            "escape" => {
                cx.stop_propagation();
                self.cancel_parameter_edit(window, cx);
            }
            _ => {}
        }
    }

    fn snapshot(&self, cx: &App) -> PrototypeSnapshot {
        let item = self.item.read(cx);
        if let Some(message) = item.document.loading_message() {
            return PrototypeSnapshot::Message(message);
        }
        if let Some(error) = item.document.error() {
            return PrototypeSnapshot::Message(error.to_string().into());
        }
        let Some(document) = item.document() else {
            return PrototypeSnapshot::Message("The document is still loading".into());
        };
        let doc = &document.doc;
        let mut selected = doc.selection.iter().copied();
        let Some(node_id) = selected.next() else {
            return PrototypeSnapshot::Message(
                "Select a frame or layer to add an interaction".into(),
            );
        };
        if selected.next().is_some() {
            return PrototypeSnapshot::Message("Select one layer to edit its interactions".into());
        }
        let Some(node) = doc.scene.get(node_id) else {
            return PrototypeSnapshot::Message("The selected layer no longer exists".into());
        };
        let frame_targets = prototype_frame_targets(doc, node_id);
        let scroll_targets = prototype_scroll_targets(doc, node_id);
        let variables = prototype_variables(doc);
        let components = prototype_components(doc);
        let clips = prototype_clips(doc);
        PrototypeSnapshot::Selection {
            editable: item.is_editable(),
            node: node_id,
            name: node.name.clone().into(),
            can_start_flow: matches!(&node.data, NodeData::Group(group) if group.is_frame_surface()),
            can_present: prototype_entry_frame(doc).is_some(),
            is_flow_start: doc.flow_start() == Some(node_id),
            reactions: node.reactions.clone(),
            frame_targets,
            scroll_targets,
            variables,
            components,
            clips,
        }
    }

    fn apply_operation(
        &mut self,
        build: impl FnOnce(&Doc) -> Option<Operation>,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.editing_parameter.is_some() {
            self.finish_parameter_edit(cx);
        }
        let operation = {
            let item = self.item.read(cx);
            if !item.is_editable() {
                return false;
            }
            let Some(document) = item.document() else {
                return false;
            };
            build(&document.doc)
        };
        let Some(operation) = operation else {
            return false;
        };
        let preview_owner = cx.entity_id();
        self.item.update(cx, |item, cx| {
            match item.apply_for_preview_owner(preview_owner, operation, cx) {
                Ok(()) => true,
                Err(error) => {
                    log::error!("Fanta prototype panel failed to apply operation: {error:#}");
                    false
                }
            }
        })
    }

    fn add_interaction(&mut self, node: NodeId, cx: &mut Context<Self>) {
        self.apply_operation(|doc| add_reaction_operation(doc, node), cx);
    }

    fn remove_interaction(&mut self, node: NodeId, reaction: ReactionId, cx: &mut Context<Self>) {
        self.apply_operation(|doc| remove_reaction_operation(doc, node, reaction), cx);
    }

    fn set_flow_start(&mut self, node: NodeId, enabled: bool, cx: &mut Context<Self>) {
        self.apply_operation(|doc| flow_start_operation(doc, enabled.then_some(node)), cx);
    }

    fn set_trigger_choice(
        &mut self,
        node: NodeId,
        reaction: ReactionId,
        choice: TriggerChoice,
        cx: &mut Context<Self>,
    ) {
        self.apply_operation(
            |doc| {
                set_reaction_operation(doc, node, reaction, |reaction| {
                    reaction.trigger = choice.into_trigger();
                })
            },
            cx,
        );
    }

    fn set_action_choice(
        &mut self,
        node: NodeId,
        reaction: ReactionId,
        choice: ActionChoice,
        cx: &mut Context<Self>,
    ) {
        self.apply_operation(
            |doc| {
                let action = action_for_choice(doc, node, choice)?;
                set_reaction_operation(doc, node, reaction, |reaction| {
                    reaction.action = action;
                })
            },
            cx,
        );
    }

    fn set_action_variable(
        &mut self,
        node: NodeId,
        reaction: ReactionId,
        variable: VariableId,
        cx: &mut Context<Self>,
    ) {
        self.apply_operation(
            |doc| {
                let value = variable_literal_default(doc, variable)?;
                set_reaction_operation(doc, node, reaction, |reaction| {
                    reaction.action = Action::SetVariable { variable, value };
                })
            },
            cx,
        );
    }

    fn set_action_component(
        &mut self,
        node: NodeId,
        reaction: ReactionId,
        component: ComponentId,
        cx: &mut Context<Self>,
    ) {
        self.apply_operation(
            |doc| {
                let variant = default_variant_value(doc, component);
                set_reaction_operation(doc, node, reaction, |reaction| {
                    reaction.action = Action::UpdateVariant { component, variant };
                })
            },
            cx,
        );
    }

    fn set_overlay_position(
        &mut self,
        node: NodeId,
        reaction: ReactionId,
        choice: OverlayPositionChoice,
        cx: &mut Context<Self>,
    ) {
        self.apply_operation(
            |doc| {
                set_reaction_operation(doc, node, reaction, |reaction| {
                    let Action::OpenOverlay { overlay, .. } = &mut reaction.action else {
                        return;
                    };
                    overlay.position = match choice {
                        OverlayPositionChoice::Center => OverlayPosition::Center,
                        OverlayPositionChoice::Manual => match &overlay.position {
                            OverlayPosition::Manual { offset } => {
                                OverlayPosition::Manual { offset: *offset }
                            }
                            _ => OverlayPosition::Manual { offset: [0.0, 0.0] },
                        },
                        OverlayPositionChoice::TopLeft => OverlayPosition::TopLeft,
                        OverlayPositionChoice::TopCenter => OverlayPosition::TopCenter,
                        OverlayPositionChoice::TopRight => OverlayPosition::TopRight,
                        OverlayPositionChoice::BottomLeft => OverlayPosition::BottomLeft,
                        OverlayPositionChoice::BottomCenter => OverlayPosition::BottomCenter,
                        OverlayPositionChoice::BottomRight => OverlayPosition::BottomRight,
                    };
                })
            },
            cx,
        );
    }

    fn set_overlay_background_dim(
        &mut self,
        node: NodeId,
        reaction: ReactionId,
        enabled: bool,
        cx: &mut Context<Self>,
    ) {
        self.apply_operation(
            |doc| {
                set_reaction_operation(doc, node, reaction, |reaction| {
                    if let Action::OpenOverlay { overlay, .. } = &mut reaction.action {
                        overlay.background_dim = enabled;
                    }
                })
            },
            cx,
        );
    }

    fn set_overlay_close_outside(
        &mut self,
        node: NodeId,
        reaction: ReactionId,
        enabled: bool,
        cx: &mut Context<Self>,
    ) {
        self.apply_operation(
            |doc| {
                set_reaction_operation(doc, node, reaction, |reaction| {
                    if let Action::OpenOverlay { overlay, .. } = &mut reaction.action {
                        overlay.close_on_click_outside = enabled;
                    }
                })
            },
            cx,
        );
    }

    fn set_action_target(
        &mut self,
        node: NodeId,
        reaction: ReactionId,
        target: NodeId,
        cx: &mut Context<Self>,
    ) {
        self.apply_operation(
            |doc| {
                set_reaction_operation(doc, node, reaction, |reaction| match &mut reaction.action {
                    Action::Navigate { to } => *to = target,
                    Action::OpenOverlay { frame, .. } => *frame = target,
                    Action::ScrollTo {
                        target: scroll_target,
                    } => *scroll_target = target,
                    _ => {}
                })
            },
            cx,
        );
    }

    fn set_transition_choice(
        &mut self,
        node: NodeId,
        reaction: ReactionId,
        choice: TransitionChoice,
        cx: &mut Context<Self>,
    ) {
        self.apply_operation(
            |doc| {
                set_reaction_operation(doc, node, reaction, |reaction| {
                    if choice == TransitionChoice::Instant {
                        reaction.transition = None;
                        return;
                    }
                    let previous = reaction.transition.as_ref();
                    reaction.transition = Some(Transition {
                        style: transition_style(choice),
                        duration_ms: previous
                            .map(|transition| transition.duration_ms)
                            .unwrap_or(DEFAULT_TRANSITION_DURATION_MS),
                        easing: previous
                            .map(|transition| transition.easing)
                            .unwrap_or(Easing::EaseInOut),
                    });
                })
            },
            cx,
        );
    }

    fn set_transition_duration(
        &mut self,
        node: NodeId,
        reaction: ReactionId,
        duration_ms: u32,
        cx: &mut Context<Self>,
    ) {
        self.apply_operation(
            |doc| {
                set_reaction_operation(doc, node, reaction, |reaction| {
                    if let Some(transition) = &mut reaction.transition {
                        transition.duration_ms = duration_ms;
                    }
                })
            },
            cx,
        );
    }

    fn set_transition_easing(
        &mut self,
        node: NodeId,
        reaction: ReactionId,
        easing: EasingChoice,
        cx: &mut Context<Self>,
    ) {
        self.apply_operation(
            |doc| {
                set_reaction_operation(doc, node, reaction, |reaction| {
                    if let Some(transition) = &mut reaction.transition {
                        transition.easing = easing.into_easing();
                    }
                })
            },
            cx,
        );
    }

    fn set_transition_direction(
        &mut self,
        node: NodeId,
        reaction: ReactionId,
        direction: DirectionChoice,
        cx: &mut Context<Self>,
    ) {
        self.apply_operation(
            |doc| {
                set_reaction_operation(doc, node, reaction, |reaction| {
                    let Some(transition) = &mut reaction.transition else {
                        return;
                    };
                    let direction = direction.into_direction();
                    transition.style = match transition.style {
                        TransitionStyle::SlideIn { .. } => TransitionStyle::SlideIn { direction },
                        TransitionStyle::Push { .. } => TransitionStyle::Push { direction },
                        TransitionStyle::MoveIn { .. } => TransitionStyle::MoveIn { direction },
                        style => style,
                    };
                })
            },
            cx,
        );
    }

    fn set_animation_clip(
        &mut self,
        node: NodeId,
        reaction: ReactionId,
        clip: Option<AnimationClipId>,
        cx: &mut Context<Self>,
    ) {
        self.apply_operation(
            |doc| {
                set_reaction_operation(doc, node, reaction, |reaction| {
                    bind_reaction_animation(reaction, clip);
                })
            },
            cx,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn render_choice_dropdown<T: Copy + PartialEq + 'static>(
        &self,
        key: &'static str,
        _index: usize,
        aria_label: &'static str,
        label: SharedString,
        node: NodeId,
        reaction: ReactionId,
        current: Option<T>,
        options: &'static [(T, &'static str)],
        apply: fn(&mut Self, NodeId, ReactionId, T, &mut Context<Self>),
        editable: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let panel = cx.weak_entity();
        let menu = ContextMenu::build(window, cx, move |mut menu, _, _| {
            for (value, name) in options {
                let panel = panel.clone();
                let value = *value;
                menu.push_item(
                    ContextMenuEntry::new(*name)
                        .toggleable(IconPosition::End, current == Some(value))
                        .handler(move |_, cx| {
                            panel
                                .update(cx, |panel, cx| apply(panel, node, reaction, value, cx))
                                .log_err();
                        }),
                );
            }
            menu
        });
        DropdownMenu::new(reaction_element_id(key, reaction), label, menu)
            .style(DropdownStyle::Outlined)
            .trigger_size(ButtonSize::Compact)
            .full_width(true)
            .disabled(!editable)
            .aria_label(aria_label)
            .into_any_element()
    }

    fn render_target_dropdown(
        &self,
        _index: usize,
        node: NodeId,
        reaction: &Reaction,
        targets: &[PrototypeTarget],
        editable: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let current = action_target(&reaction.action)?;
        let label = targets
            .iter()
            .find(|target| target.id == current)
            .map(|target| target.name.clone())
            .unwrap_or_else(|| "Missing target".into());
        let panel = cx.weak_entity();
        let reaction_id = reaction.id;
        let choices = targets.to_vec();
        let menu = ContextMenu::build(window, cx, move |mut menu, _, _| {
            for target in &choices {
                let panel = panel.clone();
                let target_id = target.id;
                menu.push_item(
                    ContextMenuEntry::new(target.name.clone())
                        .toggleable(IconPosition::End, target_id == current)
                        .handler(move |_, cx| {
                            panel
                                .update(cx, |panel, cx| {
                                    panel.set_action_target(node, reaction_id, target_id, cx)
                                })
                                .log_err();
                        }),
                );
            }
            menu
        });
        Some(
            DropdownMenu::new(
                reaction_element_id("fanta-prototype-target", reaction.id),
                label,
                menu,
            )
            .style(DropdownStyle::Outlined)
            .trigger_size(ButtonSize::Compact)
            .full_width(true)
            .disabled(!editable || targets.is_empty())
            .aria_label("Prototype target")
            .into_any_element(),
        )
    }

    fn render_clip_dropdown(
        &self,
        node: NodeId,
        reaction: &Reaction,
        clips: &[PrototypeClip],
        editable: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let current = reaction.animation.as_ref().map(|animation| animation.clip);
        let label = current
            .and_then(|current| {
                clips
                    .iter()
                    .find(|clip| clip.id == current)
                    .map(|clip| clip.name.clone())
            })
            .unwrap_or_else(|| {
                if current.is_some() {
                    "Missing clip".into()
                } else if clips.is_empty() {
                    "No motion clips".into()
                } else {
                    "None".into()
                }
            });
        let panel = cx.weak_entity();
        let reaction_id = reaction.id;
        let choices = clips.to_vec();
        let menu = ContextMenu::build(window, cx, move |mut menu, _, _| {
            let clear_panel = panel.clone();
            menu.push_item(
                ContextMenuEntry::new("None")
                    .toggleable(IconPosition::End, current.is_none())
                    .handler(move |_, cx| {
                        clear_panel
                            .update(cx, |panel, cx| {
                                panel.set_animation_clip(node, reaction_id, None, cx)
                            })
                            .log_err();
                    }),
            );
            if !choices.is_empty() {
                menu = menu.separator();
            }
            for clip in &choices {
                let panel = panel.clone();
                let clip_id = clip.id;
                menu.push_item(
                    ContextMenuEntry::new(clip.name.clone())
                        .toggleable(IconPosition::End, current == Some(clip_id))
                        .handler(move |_, cx| {
                            panel
                                .update(cx, |panel, cx| {
                                    panel.set_animation_clip(node, reaction_id, Some(clip_id), cx)
                                })
                                .log_err();
                        }),
                );
            }
            menu
        });
        DropdownMenu::new(
            reaction_element_id("fanta-prototype-motion-clip", reaction.id),
            label,
            menu,
        )
        .style(DropdownStyle::Outlined)
        .trigger_size(ButtonSize::Compact)
        .full_width(true)
        .disabled(!editable || (clips.is_empty() && current.is_none()))
        .aria_label("Prototype property animation")
        .into_any_element()
    }

    fn render_variable_dropdown(
        &self,
        _index: usize,
        node: NodeId,
        reaction: &Reaction,
        variables: &[PrototypeVariable],
        editable: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let Action::SetVariable { variable, .. } = &reaction.action else {
            return None;
        };
        let current = *variable;
        let label = variables
            .iter()
            .find(|variable| variable.id == current)
            .map(|variable| variable.name.clone())
            .unwrap_or_else(|| "Missing variable".into());
        let panel = cx.weak_entity();
        let reaction_id = reaction.id;
        let choices = variables.to_vec();
        let menu = ContextMenu::build(window, cx, move |mut menu, _, _| {
            for variable in &choices {
                let panel = panel.clone();
                let variable_id = variable.id;
                menu.push_item(
                    ContextMenuEntry::new(variable.name.clone())
                        .toggleable(IconPosition::End, variable_id == current)
                        .handler(move |_, cx| {
                            panel
                                .update(cx, |panel, cx| {
                                    panel.set_action_variable(node, reaction_id, variable_id, cx)
                                })
                                .log_err();
                        }),
                );
            }
            menu
        });
        Some(
            DropdownMenu::new(
                reaction_element_id("fanta-prototype-variable", reaction.id),
                label,
                menu,
            )
            .style(DropdownStyle::Outlined)
            .trigger_size(ButtonSize::Compact)
            .full_width(true)
            .disabled(!editable || variables.is_empty())
            .aria_label("Prototype variable")
            .into_any_element(),
        )
    }

    fn render_component_dropdown(
        &self,
        _index: usize,
        node: NodeId,
        reaction: &Reaction,
        components: &[PrototypeComponent],
        editable: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let Action::UpdateVariant { component, .. } = &reaction.action else {
            return None;
        };
        let current = *component;
        let label = components
            .iter()
            .find(|component| component.id == current)
            .map(|component| component.name.clone())
            .unwrap_or_else(|| "Missing component".into());
        let panel = cx.weak_entity();
        let reaction_id = reaction.id;
        let choices = components.to_vec();
        let menu = ContextMenu::build(window, cx, move |mut menu, _, _| {
            for component in &choices {
                let panel = panel.clone();
                let component_id = component.id;
                menu.push_item(
                    ContextMenuEntry::new(component.name.clone())
                        .toggleable(IconPosition::End, component_id == current)
                        .handler(move |_, cx| {
                            panel
                                .update(cx, |panel, cx| {
                                    panel.set_action_component(node, reaction_id, component_id, cx)
                                })
                                .log_err();
                        }),
                );
            }
            menu
        });
        Some(
            DropdownMenu::new(
                reaction_element_id("fanta-prototype-component", reaction.id),
                label,
                menu,
            )
            .style(DropdownStyle::Outlined)
            .trigger_size(ButtonSize::Compact)
            .full_width(true)
            .disabled(!editable || components.is_empty())
            .aria_label("Prototype component")
            .into_any_element(),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn render_inline_parameter(
        &self,
        _index: usize,
        node: NodeId,
        reaction: ReactionId,
        kind: ParameterKind,
        current: String,
        placeholder: &'static str,
        editable: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let parameter = ReactionParameter {
            node,
            reaction,
            kind,
        };
        let key = parameter_key(kind);
        if self.editing_parameter == Some(parameter)
            && let Some(editor) = self.parameter_editor.as_ref()
        {
            return div()
                .id(reaction_element_id(key, reaction))
                .h(px(28.0))
                .w_full()
                .px_1()
                .py_0p5()
                .rounded_sm()
                .border_1()
                .border_color(cx.theme().colors().border_focused)
                .child(editor.clone())
                .into_any_element();
        }
        let display: SharedString = if current.is_empty() {
            placeholder.into()
        } else {
            current.clone().into()
        };
        Button::new(reaction_element_id(key, reaction), display)
            .style(ButtonStyle::Subtle)
            .size(ButtonSize::Compact)
            .full_width()
            .disabled(!editable)
            .on_click(cx.listener(move |panel, _, window, cx| {
                panel.start_parameter_edit(parameter, current.clone(), window, cx)
            }))
            .into_any_element()
    }

    #[allow(clippy::too_many_arguments)]
    fn render_switch_parameter(
        &self,
        key: &'static str,
        _index: usize,
        node: NodeId,
        reaction: ReactionId,
        enabled: bool,
        editable: bool,
        apply: fn(&mut Self, NodeId, ReactionId, bool, &mut Context<Self>),
        cx: &mut Context<Self>,
    ) -> AnyElement {
        Switch::new(
            reaction_element_id(key, reaction),
            ToggleState::from(enabled),
        )
        .disabled(!editable)
        .on_click(cx.listener(move |panel, _: &ToggleState, _, cx| {
            apply(panel, node, reaction, !enabled, cx)
        }))
        .into_any_element()
    }

    fn render_reaction(
        &self,
        index: usize,
        node: NodeId,
        reaction: &Reaction,
        frame_targets: &[PrototypeTarget],
        scroll_targets: &[PrototypeTarget],
        variables: &[PrototypeVariable],
        components: &[PrototypeComponent],
        clips: &[PrototypeClip],
        editable: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let trigger = TriggerChoice::from_trigger(&reaction.trigger);
        let action = ActionChoice::from_action(&reaction.action);
        let transition = TransitionChoice::from_transition(reaction.transition.as_ref());
        let reaction_id = reaction.id;
        let mut card = v_flex()
            .id(reaction_element_id("fanta-prototype-reaction", reaction_id))
            .mx_3()
            .my_1()
            .p_2()
            .gap_2()
            .rounded_md()
            .border_1()
            .border_color(cx.theme().colors().border_variant)
            .child(
                h_flex()
                    .justify_between()
                    .child(Label::new(format!("Interaction {}", index + 1)).size(LabelSize::Small))
                    .when(editable, |row| {
                        row.child(
                            IconButton::new(
                                reaction_element_id("fanta-prototype-remove", reaction_id),
                                IconName::Close,
                            )
                            .icon_size(IconSize::XSmall)
                            .tooltip(Tooltip::text("Remove interaction"))
                            .on_click(cx.listener(
                                move |panel, _, _, cx| {
                                    panel.remove_interaction(node, reaction_id, cx)
                                },
                            )),
                        )
                    }),
            )
            .child(self.render_labeled_row(
                "Trigger",
                self.render_choice_dropdown(
                    "fanta-prototype-trigger",
                    index,
                    "Prototype trigger",
                    trigger_label(&reaction.trigger).into(),
                    node,
                    reaction.id,
                    Some(trigger),
                    &TRIGGER_CHOICES,
                    Self::set_trigger_choice,
                    editable,
                    window,
                    cx,
                ),
            ));
        match &reaction.trigger {
            // Hover-family triggers carry no editable parameter row.
            Trigger::MouseEnter | Trigger::MouseLeave | Trigger::WhileHovering => {}
            Trigger::AfterDelay { delay_ms } => {
                card = card.child(self.render_labeled_row(
                    "Delay",
                    self.render_inline_parameter(
                        index,
                        node,
                        reaction.id,
                        ParameterKind::Delay,
                        delay_ms.to_string(),
                        "Enter delay",
                        editable,
                        cx,
                    ),
                ));
            }
            Trigger::Key { keys } => {
                card = card.child(self.render_labeled_row(
                    "Keys",
                    self.render_inline_parameter(
                        index,
                        node,
                        reaction.id,
                        ParameterKind::Keys,
                        keys.join(" + "),
                        "Enter keys",
                        editable,
                        cx,
                    ),
                ));
            }
            Trigger::Click | Trigger::Drag | Trigger::Hover | Trigger::WhilePressing => {}
        }
        card = card.child(self.render_labeled_row(
            "Action",
            self.render_choice_dropdown(
                "fanta-prototype-action",
                index,
                "Prototype action",
                action_label(&reaction.action).into(),
                node,
                reaction.id,
                action,
                &ACTION_CHOICES,
                Self::set_action_choice,
                editable,
                window,
                cx,
            ),
        ));
        let targets = if matches!(reaction.action, Action::ScrollTo { .. }) {
            scroll_targets
        } else {
            frame_targets
        };
        if let Some(target) =
            self.render_target_dropdown(index, node, reaction, targets, editable, window, cx)
        {
            card = card.child(self.render_labeled_row("Destination", target));
        }
        if let Some(variable) =
            self.render_variable_dropdown(index, node, reaction, variables, editable, window, cx)
        {
            card = card.child(self.render_labeled_row("Variable", variable));
        }
        if let Action::SetVariable { value, .. } = &reaction.action {
            let (value_text, value_editable) = variable_value_text(value);
            card = card.child(self.render_labeled_row(
                "Value",
                self.render_inline_parameter(
                    index,
                    node,
                    reaction.id,
                    ParameterKind::VariableValue,
                    value_text,
                    "Enter value",
                    editable && value_editable,
                    cx,
                ),
            ));
        }
        if let Some(component) =
            self.render_component_dropdown(index, node, reaction, components, editable, window, cx)
        {
            card = card.child(self.render_labeled_row("Component", component));
        }
        if let Action::UpdateVariant { variant, .. } = &reaction.action {
            card = card.child(self.render_labeled_row(
                "Variant",
                self.render_inline_parameter(
                    index,
                    node,
                    reaction.id,
                    ParameterKind::Variant,
                    variant.clone(),
                    "Enter variant",
                    editable,
                    cx,
                ),
            ));
        }
        if let Action::OpenLink { url } = &reaction.action {
            card = card.child(self.render_labeled_row(
                "URL",
                self.render_inline_parameter(
                    index,
                    node,
                    reaction.id,
                    ParameterKind::Url,
                    url.clone(),
                    "Enter URL",
                    editable,
                    cx,
                ),
            ));
        }
        if let Action::OpenOverlay { overlay, .. } = &reaction.action {
            let position = OverlayPositionChoice::from_position(&overlay.position);
            card = card.child(self.render_labeled_row(
                "Position",
                self.render_choice_dropdown(
                    "fanta-prototype-overlay-position",
                    index,
                    "Prototype overlay position",
                    position.label().into(),
                    node,
                    reaction.id,
                    Some(position),
                    &OVERLAY_POSITION_CHOICES,
                    Self::set_overlay_position,
                    editable,
                    window,
                    cx,
                ),
            ));
            if let OverlayPosition::Manual { offset } = &overlay.position {
                card = card
                    .child(self.render_labeled_row(
                        "Offset X",
                        self.render_inline_parameter(
                            index,
                            node,
                            reaction.id,
                            ParameterKind::OverlayOffsetX,
                            format_number(offset[0]),
                            "0",
                            editable,
                            cx,
                        ),
                    ))
                    .child(self.render_labeled_row(
                        "Offset Y",
                        self.render_inline_parameter(
                            index,
                            node,
                            reaction.id,
                            ParameterKind::OverlayOffsetY,
                            format_number(offset[1]),
                            "0",
                            editable,
                            cx,
                        ),
                    ));
            }
            card = card
                .child(self.render_labeled_row(
                    "Dim background",
                    self.render_switch_parameter(
                        "fanta-prototype-overlay-dim",
                        index,
                        node,
                        reaction.id,
                        overlay.background_dim,
                        editable,
                        Self::set_overlay_background_dim,
                        cx,
                    ),
                ))
                .child(self.render_labeled_row(
                    "Close outside",
                    self.render_switch_parameter(
                        "fanta-prototype-overlay-close-outside",
                        index,
                        node,
                        reaction.id,
                        overlay.close_on_click_outside,
                        editable,
                        Self::set_overlay_close_outside,
                        cx,
                    ),
                ));
        }
        card = card.child(self.render_labeled_row(
            "Transition",
            self.render_choice_dropdown(
                "fanta-prototype-transition",
                index,
                "Prototype animation",
                transition_label(reaction.transition.as_ref()).into(),
                node,
                reaction.id,
                Some(transition),
                &TRANSITION_CHOICES,
                Self::set_transition_choice,
                editable,
                window,
                cx,
            ),
        ));
        if let Some(transition) = &reaction.transition {
            let easing = EasingChoice::from_easing(transition.easing);
            let duration_label: SharedString = format!("{} ms", transition.duration_ms).into();
            card = card
                .child(self.render_labeled_row(
                    "Duration",
                    self.render_choice_dropdown(
                        "fanta-prototype-duration",
                        index,
                        "Prototype transition duration",
                        duration_label,
                        node,
                        reaction.id,
                        Some(transition.duration_ms),
                        &DURATION_CHOICES,
                        Self::set_transition_duration,
                        editable,
                        window,
                        cx,
                    ),
                ))
                .child(
                    self.render_labeled_row(
                        "Easing",
                        self.render_choice_dropdown(
                            "fanta-prototype-easing",
                            index,
                            "Prototype transition easing",
                            easing
                                .map(EasingChoice::label)
                                .unwrap_or("Custom curve")
                                .into(),
                            node,
                            reaction.id,
                            easing,
                            &EASING_CHOICES,
                            Self::set_transition_easing,
                            editable,
                            window,
                            cx,
                        ),
                    ),
                );
            if let Some(direction) = direction_choice(transition.style) {
                card = card.child(self.render_labeled_row(
                    "Direction",
                    self.render_choice_dropdown(
                        "fanta-prototype-direction",
                        index,
                        "Prototype transition direction",
                        direction.label().into(),
                        node,
                        reaction.id,
                        Some(direction),
                        &DIRECTION_CHOICES,
                        Self::set_transition_direction,
                        editable,
                        window,
                        cx,
                    ),
                ));
            }
        }
        card = card.child(self.render_labeled_row(
            "Motion clip",
            self.render_clip_dropdown(node, reaction, clips, editable, window, cx),
        ));
        if let Some(animation) = &reaction.animation {
            card = card.child(self.render_labeled_row(
                "Clip delay",
                self.render_inline_parameter(
                    index,
                    node,
                    reaction.id,
                    ParameterKind::AnimationDelay,
                    animation.delay_ms.to_string(),
                    "0",
                    editable,
                    cx,
                ),
            ));
        }
        card.into_any_element()
    }

    fn render_labeled_row(&self, label: &'static str, control: AnyElement) -> AnyElement {
        InspectorPropertyRow::new(label, control)
            .inset(false)
            .into_any_element()
    }
}

impl Render for FantaPrototypePanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.ensure_parameter_editor(window, cx);
        let mut root = v_flex()
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(Self::handle_key_down))
            .size_full()
            .bg(cx.theme().colors().panel_background);
        if let Some(error) = self.parameter_error.clone() {
            root = root.child(
                h_flex()
                    .flex_none()
                    .px_3()
                    .py_1()
                    .bg(cx.theme().status().error_background)
                    .child(Label::new(error).size(LabelSize::Small)),
            );
        }
        match self.snapshot(cx) {
            PrototypeSnapshot::Message(message) => root.child(InspectorMessage::new(message)),
            PrototypeSnapshot::Selection {
                editable,
                node,
                name,
                can_start_flow,
                can_present,
                is_flow_start,
                reactions,
                frame_targets,
                scroll_targets,
                variables,
                components,
                clips,
            } => {
                let header = v_flex()
                    .px_4()
                    .py_2()
                    .gap_1()
                    .child(
                        Label::new("Prototype")
                            .size(LabelSize::XSmall)
                            .color(Color::Muted),
                    )
                    .child(
                        h_flex()
                            .justify_between()
                            .gap_2()
                            .child(Label::new(name).single_line())
                            .child(
                                IconButton::new(
                                    "fanta-prototype-present-from-panel",
                                    IconName::PlayFilled,
                                )
                                .icon_size(IconSize::Small)
                                .tooltip(Tooltip::text("Present prototype"))
                                .disabled(!can_present)
                                .on_click(|_, window, cx| {
                                    window.dispatch_action(Box::new(PlayPrototype), cx);
                                }),
                            ),
                    )
                    .when(can_start_flow, |header| {
                        header.child(
                            Button::new(
                                "fanta-prototype-flow-start",
                                if is_flow_start {
                                    "Starting point"
                                } else {
                                    "Set as starting point"
                                },
                            )
                            .start_icon(Icon::new(IconName::PlayFilled).size(IconSize::XSmall))
                            .size(ButtonSize::Compact)
                            .disabled(!editable)
                            .on_click(cx.listener(
                                move |panel, _, _, cx| {
                                    panel.set_flow_start(node, !is_flow_start, cx)
                                },
                            )),
                        )
                    });
                let interactions_header = if editable {
                    InspectorSectionHeader::new("Interactions")
                        .action(
                            IconButton::new("fanta-prototype-add", IconName::Plus)
                                .icon_size(IconSize::Small)
                                .tooltip(Tooltip::text("Add interaction"))
                                .on_click(cx.listener(move |panel, _, _, cx| {
                                    panel.add_interaction(node, cx)
                                })),
                        )
                        .into_any_element()
                } else {
                    InspectorSectionHeader::new("Interactions").into_any_element()
                };
                let mut content = v_flex()
                    .id("fanta-prototype-content")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .pb_4()
                    .child(header)
                    .child(Divider::horizontal())
                    .child(interactions_header);
                if reactions.is_empty() {
                    content = content.child(
                        v_flex()
                            .px_4()
                            .py_2()
                            .gap_3()
                            .child(
                                v_flex()
                                    .gap_1()
                                    .child(Label::new("Create a connection").size(LabelSize::Small))
                                    .child(
                                        Label::new(
                                            "Add an interaction, then choose the destination frame and transition.",
                                        )
                                        .size(LabelSize::XSmall)
                                        .color(Color::Muted),
                                    ),
                            )
                            .child(
                                v_flex()
                                    .gap_1()
                                    .child(Label::new("Run your prototype").size(LabelSize::Small))
                                    .child(
                                        Label::new(
                                            "Use Present to play from the starting point. Frames remain browsable without connections.",
                                        )
                                        .size(LabelSize::XSmall)
                                        .color(Color::Muted),
                                    ),
                            ),
                    );
                } else {
                    for (index, reaction) in reactions.iter().enumerate() {
                        content = content.child(self.render_reaction(
                            index,
                            node,
                            reaction,
                            &frame_targets,
                            &scroll_targets,
                            &variables,
                            &components,
                            &clips,
                            editable,
                            window,
                            cx,
                        ));
                    }
                }
                root.child(content)
            }
        }
    }
}

impl EventEmitter<()> for FantaPrototypePanel {}

impl Focusable for FantaPrototypePanel {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

fn action_for_choice(doc: &Doc, node: NodeId, choice: ActionChoice) -> Option<Action> {
    match choice {
        ActionChoice::Navigate => prototype_frame_targets(doc, node)
            .first()
            .map(|target| Action::Navigate { to: target.id }),
        ActionChoice::OpenOverlay => {
            prototype_frame_targets(doc, node)
                .first()
                .map(|target| Action::OpenOverlay {
                    frame: target.id,
                    overlay: default_overlay_settings(),
                })
        }
        ActionChoice::ScrollTo => prototype_scroll_targets(doc, node)
            .first()
            .map(|target| Action::ScrollTo { target: target.id }),
        ActionChoice::SetVariable => {
            let variable = doc
                .variables
                .variables
                .values()
                .find(|variable| variable.ty != VariableType::Typography)
                .or_else(|| doc.variables.variables.values().next())?;
            Some(Action::SetVariable {
                variable: variable.id,
                value: variable_literal_default(doc, variable.id)?,
            })
        }
        ActionChoice::UpdateVariant => {
            let component = doc.components.sets.keys().next().copied().or_else(|| {
                doc.components
                    .defs
                    .values()
                    .find(|component| component.variant_of.is_some())
                    .map(|component| component.id)
            })?;
            Some(Action::UpdateVariant {
                component,
                variant: default_variant_value(doc, component),
            })
        }
        ActionChoice::OpenLink => Some(Action::OpenLink { url: String::new() }),
        ActionChoice::Back => Some(Action::Back),
        ActionChoice::Close => Some(Action::Close),
    }
}

fn prototype_variables(doc: &Doc) -> Vec<PrototypeVariable> {
    doc.variables
        .variables
        .values()
        .map(|variable| {
            let collection_name = doc
                .variables
                .collections
                .get(&variable.collection)
                .map(|collection| collection.name.as_str())
                .filter(|name| !name.is_empty());
            let variable_name = if variable.name.is_empty() {
                "Untitled variable"
            } else {
                variable.name.as_str()
            };
            let name = match collection_name {
                Some(collection) => format!("{collection} / {variable_name}"),
                None => variable_name.to_owned(),
            };
            PrototypeVariable {
                id: variable.id,
                name: name.into(),
            }
        })
        .collect()
}

fn prototype_components(doc: &Doc) -> Vec<PrototypeComponent> {
    let mut components = Vec::new();
    for component in doc.components.sets.values() {
        components.push(PrototypeComponent {
            id: component.id,
            name: if component.name.is_empty() {
                "Untitled component set".into()
            } else {
                component.name.clone().into()
            },
        });
    }
    for component in doc
        .components
        .defs
        .values()
        .filter(|component| component.variant_of.is_some())
    {
        components.push(PrototypeComponent {
            id: component.id,
            name: if component.name.is_empty() {
                "Untitled variant".into()
            } else {
                component.name.clone().into()
            },
        });
    }
    components
}

fn variable_literal_default(doc: &Doc, variable: VariableId) -> Option<VarValue> {
    let variable = doc.variables.variables.get(&variable)?;
    let default_mode = doc
        .variables
        .collections
        .get(&variable.collection)
        .map(|collection| collection.default_mode);
    let stored = default_mode
        .and_then(|mode| variable.values_by_mode.get(&mode))
        .into_iter()
        .chain(variable.values_by_mode.values())
        .find(|value| {
            !matches!(value, VarValue::Alias { .. }) && value.variable_type() == Some(variable.ty)
        });
    stored.cloned().or_else(|| {
        Some(match variable.ty {
            VariableType::Color => VarValue::Color {
                value: FantaColor::BLACK,
            },
            VariableType::Float => VarValue::Float { value: 0.0 },
            VariableType::String => VarValue::String {
                value: String::new(),
            },
            VariableType::Boolean => VarValue::Boolean { value: false },
            VariableType::Typography => VarValue::TextStyle {
                value: fanta_doc::TextStyle::default(),
            },
        })
    })
}

fn default_variant_value(doc: &Doc, component: ComponentId) -> String {
    if let Some(set) = doc.components.sets.get(&component) {
        return set
            .axes
            .iter()
            .find_map(|axis| axis.values.first())
            .cloned()
            .or_else(|| {
                doc.components
                    .defs
                    .get(&set.default_variant)
                    .map(|component| component.name.clone())
            })
            .unwrap_or_default();
    }
    let Some(component) = doc.components.defs.get(&component) else {
        return String::new();
    };
    component
        .variant_of
        .as_ref()
        .and_then(|membership| membership.axis_values.values().next())
        .cloned()
        .unwrap_or_else(|| component.name.clone())
}

fn reaction_by_id(doc: &Doc, node: NodeId, reaction: ReactionId) -> Option<&Reaction> {
    doc.scene
        .get(node)?
        .reactions
        .iter()
        .find(|candidate| candidate.id == reaction)
}

fn replace_reaction_preview_if_current(
    doc: &mut Doc,
    node: NodeId,
    reaction: ReactionId,
    expected: &Reaction,
    mut replacement: Reaction,
) -> Option<bool> {
    let Some(node) = doc.scene.get_mut(node) else {
        return None;
    };
    let Some(current) = node
        .reactions
        .iter_mut()
        .find(|candidate| candidate.id == reaction)
    else {
        return None;
    };
    replacement.id = reaction;
    if current != expected {
        return None;
    }
    if *current == replacement {
        return Some(false);
    }
    *current = replacement;
    Some(true)
}

fn parameter_reaction_from_text(
    doc: &Doc,
    baseline: &Reaction,
    kind: ParameterKind,
    text: &str,
) -> Result<Reaction, String> {
    let mut reaction = baseline.clone();
    match kind {
        ParameterKind::Delay => {
            let delay_ms = text
                .trim()
                .parse::<u32>()
                .map_err(|_| "Enter a whole-number delay in milliseconds".to_owned())?;
            let Trigger::AfterDelay { delay_ms: current } = &mut reaction.trigger else {
                return Err("This interaction no longer uses an after-delay trigger".to_owned());
            };
            *current = delay_ms;
        }
        ParameterKind::AnimationDelay => {
            let delay_ms = text
                .trim()
                .parse::<u32>()
                .map_err(|_| "Enter a whole-number animation delay in milliseconds".to_owned())?;
            let Some(animation) = &mut reaction.animation else {
                return Err("Bind a motion clip before editing its delay".to_owned());
            };
            animation.delay_ms = delay_ms;
        }
        ParameterKind::Keys => {
            let keys: Vec<String> = text
                .split(|character| character == '+' || character == ',')
                .map(str::trim)
                .filter(|key| !key.is_empty())
                .map(str::to_owned)
                .collect();
            if keys.is_empty() {
                return Err("Enter at least one key".to_owned());
            }
            let Trigger::Key { keys: current } = &mut reaction.trigger else {
                return Err("This interaction no longer uses a key trigger".to_owned());
            };
            *current = keys;
        }
        ParameterKind::VariableValue => {
            let Action::SetVariable { variable, value } = &mut reaction.action else {
                return Err("This interaction no longer sets a variable".to_owned());
            };
            *value = parse_variable_literal(doc, *variable, text)?;
        }
        ParameterKind::Variant => {
            let variant = text.trim();
            if variant.is_empty() {
                return Err("Enter a variant value".to_owned());
            }
            let Action::UpdateVariant {
                variant: current, ..
            } = &mut reaction.action
            else {
                return Err("This interaction no longer changes a variant".to_owned());
            };
            *current = variant.to_owned();
        }
        ParameterKind::Url => {
            let url = text.trim();
            if url.is_empty() {
                return Err("Enter a URL".to_owned());
            }
            let Action::OpenLink { url: current } = &mut reaction.action else {
                return Err("This interaction no longer opens a link".to_owned());
            };
            *current = url.to_owned();
        }
        ParameterKind::OverlayOffsetX | ParameterKind::OverlayOffsetY => {
            let value = text
                .trim()
                .parse::<f64>()
                .ok()
                .filter(|value| value.is_finite())
                .ok_or_else(|| "Enter a finite overlay offset".to_owned())?;
            let Action::OpenOverlay { overlay, .. } = &mut reaction.action else {
                return Err("This interaction no longer opens an overlay".to_owned());
            };
            let OverlayPosition::Manual { offset } = &mut overlay.position else {
                return Err("Choose manual overlay positioning first".to_owned());
            };
            let axis = if kind == ParameterKind::OverlayOffsetX {
                0
            } else {
                1
            };
            offset[axis] = value;
        }
    }
    reaction.id = baseline.id;
    Ok(reaction)
}

fn parse_variable_literal(doc: &Doc, variable: VariableId, text: &str) -> Result<VarValue, String> {
    let variable = doc
        .variables
        .variables
        .get(&variable)
        .ok_or_else(|| "The selected variable no longer exists".to_owned())?;
    match variable.ty {
        VariableType::Color => {
            let text = text.trim();
            let color = FantaColor::from_hex(text)
                .or_else(|| FantaColor::from_hex(&format!("#{text}")))
                .ok_or_else(|| "Enter a color as #RRGGBB or #RRGGBBAA".to_owned())?;
            Ok(VarValue::Color { value: color })
        }
        VariableType::Float => {
            let value = text
                .trim()
                .parse::<f64>()
                .ok()
                .filter(|value| value.is_finite())
                .ok_or_else(|| "Enter a finite number".to_owned())?;
            Ok(VarValue::Float { value })
        }
        VariableType::String => Ok(VarValue::String {
            value: text.to_owned(),
        }),
        VariableType::Boolean => match text.trim().to_ascii_lowercase().as_str() {
            "true" => Ok(VarValue::Boolean { value: true }),
            "false" => Ok(VarValue::Boolean { value: false }),
            _ => Err("Enter true or false".to_owned()),
        },
        VariableType::Typography => {
            Err("Typography action values are not editable as text".to_owned())
        }
    }
}

fn variable_value_text(value: &VarValue) -> (String, bool) {
    match value {
        VarValue::Color { value } => (value.to_hex(), true),
        VarValue::Float { value } => (format_number(*value), true),
        VarValue::String { value } => (value.clone(), true),
        VarValue::Boolean { value } => (value.to_string(), true),
        VarValue::TextStyle { .. } => ("Typography value".to_owned(), false),
        VarValue::Alias { .. } => ("Variable alias".to_owned(), false),
    }
}

fn reaction_element_id(key: &'static str, reaction: ReactionId) -> ElementId {
    (ElementId::from(key), reaction.to_string()).into()
}

fn parameter_key(kind: ParameterKind) -> &'static str {
    match kind {
        ParameterKind::Delay => "fanta-prototype-delay",
        ParameterKind::AnimationDelay => "fanta-prototype-animation-delay",
        ParameterKind::Keys => "fanta-prototype-keys",
        ParameterKind::VariableValue => "fanta-prototype-variable-value",
        ParameterKind::Variant => "fanta-prototype-variant",
        ParameterKind::Url => "fanta-prototype-url",
        ParameterKind::OverlayOffsetX => "fanta-prototype-overlay-offset-x",
        ParameterKind::OverlayOffsetY => "fanta-prototype-overlay-offset-y",
    }
}

fn format_number(value: f64) -> String {
    if value.fract().abs() < f64::EPSILON {
        format!("{value:.0}")
    } else {
        value.to_string()
    }
}

fn prototype_clips(doc: &Doc) -> Vec<PrototypeClip> {
    let mut clips: Vec<_> = doc
        .motion
        .clips
        .values()
        .map(|clip| PrototypeClip {
            id: clip.id,
            name: if clip.name.trim().is_empty() {
                "Untitled animation".into()
            } else {
                clip.name.clone().into()
            },
        })
        .collect();
    clips.sort_by(|left, right| {
        left.name
            .to_lowercase()
            .cmp(&right.name.to_lowercase())
            .then_with(|| left.id.cmp(&right.id))
    });
    clips
}

fn prototype_frame_targets(doc: &Doc, selected: NodeId) -> Vec<PrototypeTarget> {
    let containing_frame = containing_presentation_frame(doc, selected);
    let candidates = page_root_for_node(doc, selected)
        .map(|page| doc.scene.children_of(Some(page)).to_vec())
        .unwrap_or_else(|| doc.scene.roots().to_vec());
    candidates
        .into_iter()
        .filter(|id| *id != selected && Some(*id) != containing_frame)
        .filter_map(|id| {
            let node = doc.scene.get(id)?;
            matches!(&node.data, NodeData::Group(group) if group.is_frame_surface()).then(|| {
                PrototypeTarget {
                    id,
                    name: if node.name.is_empty() {
                        "Untitled frame".into()
                    } else {
                        node.name.clone().into()
                    },
                }
            })
        })
        .collect()
}

fn prototype_scroll_targets(doc: &Doc, selected: NodeId) -> Vec<PrototypeTarget> {
    let Some(frame) = containing_presentation_frame(doc, selected) else {
        return Vec::new();
    };
    doc.scene
        .descendants_of(frame)
        .filter(|id| *id != selected && *id != frame)
        .filter(|id| doc.scene.world_bounds(*id).is_some())
        .filter_map(|id| {
            let node = doc.scene.get(id)?;
            Some(PrototypeTarget {
                id,
                name: if node.name.is_empty() {
                    "Untitled layer".into()
                } else {
                    node.name.clone().into()
                },
            })
        })
        .collect()
}

fn containing_presentation_frame(doc: &Doc, node: NodeId) -> Option<NodeId> {
    let page = page_root_for_node(doc, node);
    std::iter::once(node)
        .chain(doc.scene.ancestors_of(node).map(|ancestor| ancestor.id))
        .find(|id| {
            doc.scene.get(*id).is_some_and(|candidate| {
                candidate.parent == page
                    && matches!(
                        &candidate.data,
                        NodeData::Group(group) if group.is_frame_surface()
                    )
            })
        })
}

fn page_root_for_node(doc: &Doc, node: NodeId) -> Option<NodeId> {
    doc.pages().iter().copied().find(|page| {
        *page == node
            || doc
                .scene
                .ancestors_of(node)
                .any(|ancestor| ancestor.id == *page)
    })
}

fn add_reaction_operation(doc: &Doc, node: NodeId) -> Option<Operation> {
    let selected = doc.scene.get(node)?;
    let action = prototype_frame_targets(doc, node)
        .first()
        .map(|target| Action::Navigate { to: target.id })
        .unwrap_or(Action::Back);
    Some(Operation::AddReaction {
        node: selected.id,
        reaction: Reaction {
            id: ReactionId::new(),
            trigger: Trigger::Click,
            action,
            extra_actions: Vec::new(),
            transition: None,
            animation: None,
        },
    })
}

fn remove_reaction_operation(doc: &Doc, node: NodeId, reaction: ReactionId) -> Option<Operation> {
    let node = doc.scene.get(node)?;
    let index = node
        .reactions
        .iter()
        .position(|candidate| candidate.id == reaction)?;
    Some(Operation::RemoveReaction {
        node: node.id,
        index,
        reaction: node.reactions.get(index)?.clone(),
    })
}

fn set_reaction_operation(
    doc: &Doc,
    node: NodeId,
    reaction: ReactionId,
    mutate: impl FnOnce(&mut Reaction),
) -> Option<Operation> {
    let node = doc.scene.get(node)?;
    let index = node
        .reactions
        .iter()
        .position(|candidate| candidate.id == reaction)?;
    let old = node.reactions.get(index)?.clone();
    let mut new = old.clone();
    mutate(&mut new);
    (new != old).then_some(Operation::SetReaction {
        node: node.id,
        index,
        old,
        new,
    })
}

fn bind_reaction_animation(reaction: &mut Reaction, clip: Option<AnimationClipId>) {
    let delay_ms = reaction
        .animation
        .as_ref()
        .map_or(0, |animation| animation.delay_ms);
    reaction.animation = clip.map(|clip| PrototypeAnimation { clip, delay_ms });
}

fn flow_start_operation(doc: &Doc, new: Option<NodeId>) -> Option<Operation> {
    let old = doc.flow_start();
    (old != new).then_some(Operation::SetFlowStart { old, new })
}

fn default_overlay_settings() -> OverlaySettings {
    OverlaySettings {
        position: OverlayPosition::Center,
        background_dim: true,
        close_on_click_outside: true,
    }
}

fn transition_style(choice: TransitionChoice) -> TransitionStyle {
    match choice {
        TransitionChoice::Instant => TransitionStyle::Instant,
        TransitionChoice::Dissolve => TransitionStyle::Dissolve,
        TransitionChoice::SmartAnimate => TransitionStyle::SmartAnimate,
        TransitionChoice::SlideIn => TransitionStyle::SlideIn {
            direction: Direction::Left,
        },
        TransitionChoice::Push => TransitionStyle::Push {
            direction: Direction::Left,
        },
        TransitionChoice::MoveIn => TransitionStyle::MoveIn {
            direction: Direction::Left,
        },
    }
}

fn direction_choice(style: TransitionStyle) -> Option<DirectionChoice> {
    match style {
        TransitionStyle::SlideIn { direction }
        | TransitionStyle::Push { direction }
        | TransitionStyle::MoveIn { direction } => Some(DirectionChoice::from_direction(direction)),
        TransitionStyle::SlideOut { direction } | TransitionStyle::MoveOut { direction } => {
            Some(DirectionChoice::from_direction(direction))
        }
        TransitionStyle::Instant
        | TransitionStyle::Dissolve
        | TransitionStyle::SmartAnimate
        | TransitionStyle::ScrollAnimate => None,
    }
}

fn action_target(action: &Action) -> Option<NodeId> {
    match action {
        Action::Navigate { to } => Some(*to),
        Action::OpenOverlay { frame, .. } => Some(*frame),
        Action::ScrollTo { target } => Some(*target),
        _ => None,
    }
}

fn trigger_label(trigger: &Trigger) -> String {
    match trigger {
        Trigger::Click => "On click".to_string(),
        Trigger::Drag => "On drag".to_string(),
        Trigger::Hover => "On hover".to_string(),
        Trigger::AfterDelay { delay_ms } => format!("After {delay_ms} ms"),
        Trigger::Key { keys } if keys.is_empty() => "On key press".to_string(),
        Trigger::Key { keys } => format!("On {}", keys.join(" + ")),
        Trigger::WhilePressing => "While pressing".to_string(),
        Trigger::MouseEnter => "On mouse enter".to_string(),
        Trigger::MouseLeave => "On mouse leave".to_string(),
        Trigger::WhileHovering => "While hovering".to_string(),
    }
}

fn action_label(action: &Action) -> &'static str {
    match action {
        Action::Navigate { .. } => "Navigate to",
        Action::Back => "Back",
        Action::Close => "Close",
        Action::OpenOverlay { .. } => "Open overlay",
        Action::ScrollTo { .. } => "Scroll to",
        Action::SetVariable { .. } => "Set variable",
        Action::UpdateVariant { .. } => "Change variant",
        Action::OpenLink { .. } => "Open link",
    }
}

fn transition_label(transition: Option<&Transition>) -> &'static str {
    match TransitionChoice::from_transition(transition) {
        TransitionChoice::Instant => "Instant",
        TransitionChoice::Dissolve => "Dissolve",
        TransitionChoice::SmartAnimate => "Smart animate",
        TransitionChoice::SlideIn => "Slide in",
        TransitionChoice::Push => "Push",
        TransitionChoice::MoveIn => "Move in",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::ready_item_for_test;
    use fanta_doc::{
        AnimationClip, CanvasNode, ComponentSet, GroupNode, Mode, ModeId, Variable,
        VariableCollection, VariableCollectionId, VariantAxis,
    };
    use gpui::TestAppContext;
    use project::{FakeFs, Project};
    use settings::SettingsStore;
    use std::{collections::BTreeMap, path::PathBuf};

    fn init_test(cx: &mut TestAppContext) {
        cx.update(|cx| {
            zlog::init_test();
            assets::Assets.load_test_fonts(cx);
            let settings_store = SettingsStore::test(cx);
            cx.set_global(settings_store);
            theme_settings::init(theme::LoadThemes::JustBase, cx);
            release_channel::init(semver::Version::new(0, 0, 0), cx);
            editor::init(cx);
        });
    }

    fn document_with_two_frames() -> (Doc, NodeId, NodeId) {
        let mut doc = Doc::new();
        let mut first = CanvasNode::new(NodeData::Group(GroupNode {
            clip_size: Some([100.0, 100.0]),
            ..GroupNode::default()
        }));
        first.name = "First".to_string();
        let first_id = first.id;
        doc.apply(Operation::create_node(first)).unwrap();
        let mut second = CanvasNode::new(NodeData::Group(GroupNode {
            clip_size: Some([100.0, 100.0]),
            ..GroupNode::default()
        }));
        second.name = "Second".to_string();
        let second_id = second.id;
        doc.apply(Operation::create_node(second)).unwrap();
        (doc, first_id, second_id)
    }

    #[test]
    fn prototype_destinations_are_action_specific_and_page_scoped() {
        let mut doc = Doc::new();
        let page_one = CanvasNode::new(NodeData::Group(GroupNode::default()));
        let page_one_id = page_one.id;
        doc.apply(Operation::create_node(page_one)).unwrap();
        doc.add_page(page_one_id);
        let page_two = CanvasNode::new(NodeData::Group(GroupNode::default()));
        let page_two_id = page_two.id;
        doc.apply(Operation::create_node(page_two)).unwrap();
        doc.add_page(page_two_id);

        let mut selected = CanvasNode::new(NodeData::Group(GroupNode {
            clip_size: Some([100.0, 100.0]),
            ..GroupNode::default()
        }));
        let selected_id = selected.id;
        selected.parent = Some(page_one_id);
        selected.name = "Selected".into();
        doc.apply(Operation::create_node(selected)).unwrap();
        let mut scroll_target = CanvasNode::new(NodeData::Group(GroupNode {
            local_size: Some([20.0, 20.0]),
            ..GroupNode::default()
        }));
        let scroll_target_id = scroll_target.id;
        scroll_target.parent = Some(selected_id);
        scroll_target.name = "Scroll target".into();
        doc.apply(Operation::create_node(scroll_target)).unwrap();
        let mut sibling = CanvasNode::new(NodeData::Group(GroupNode {
            clip_size: Some([100.0, 100.0]),
            ..GroupNode::default()
        }));
        let sibling_id = sibling.id;
        sibling.parent = Some(page_one_id);
        sibling.name = "Sibling".into();
        doc.apply(Operation::create_node(sibling)).unwrap();
        let mut sibling_child = CanvasNode::new(NodeData::Group(GroupNode {
            local_size: Some([10.0, 10.0]),
            ..GroupNode::default()
        }));
        let sibling_child_id = sibling_child.id;
        sibling_child.parent = Some(sibling_id);
        sibling_child.name = "Sibling child".into();
        doc.apply(Operation::create_node(sibling_child)).unwrap();
        let mut other_page = CanvasNode::new(NodeData::Group(GroupNode {
            clip_size: Some([100.0, 100.0]),
            ..GroupNode::default()
        }));
        let other_page_id = other_page.id;
        other_page.parent = Some(page_two_id);
        other_page.name = "Other page".into();
        doc.apply(Operation::create_node(other_page)).unwrap();

        let frames = prototype_frame_targets(&doc, selected_id);
        assert_eq!(
            frames.iter().map(|target| target.id).collect::<Vec<_>>(),
            vec![sibling_id]
        );
        let scroll = prototype_scroll_targets(&doc, selected_id);
        let scroll_ids = scroll.iter().map(|target| target.id).collect::<Vec<_>>();
        assert!(scroll_ids.contains(&scroll_target_id));
        assert!(!scroll_ids.contains(&sibling_child_id));
        assert!(!scroll_ids.contains(&other_page_id));
        assert_eq!(
            prototype_frame_targets(&doc, scroll_target_id)
                .iter()
                .map(|target| target.id)
                .collect::<Vec<_>>(),
            vec![sibling_id],
            "a child interaction must not default back to its containing frame"
        );
        assert_eq!(
            action_for_choice(&doc, selected_id, ActionChoice::ScrollTo),
            Some(Action::ScrollTo {
                target: scroll_target_id
            })
        );
    }

    fn add_float_variable(doc: &mut Doc, value: f64) -> VariableId {
        let collection = VariableCollectionId::new();
        let mode = ModeId::new();
        let variable = VariableId::new();
        doc.variables.collections.insert(
            collection,
            VariableCollection {
                id: collection,
                name: "Prototype".into(),
                modes: vec![Mode {
                    id: mode,
                    name: "Default".into(),
                }],
                default_mode: mode,
                variable_order: vec![variable],
            },
        );
        doc.variables.variables.insert(
            variable,
            Variable {
                id: variable,
                collection,
                name: "Count".into(),
                ty: VariableType::Float,
                values_by_mode: BTreeMap::from([(mode, VarValue::Float { value })]),
                scopes: Vec::new(),
            },
        );
        variable
    }

    fn add_variant_set(doc: &mut Doc) -> ComponentId {
        let component = ComponentId::new();
        doc.components.sets.insert(
            component,
            ComponentSet {
                id: component,
                name: "Button".into(),
                axes: vec![VariantAxis {
                    name: "State".into(),
                    values: vec!["Default".into(), "Hover".into()],
                }],
                members: Vec::new(),
                default_variant: ComponentId::new(),
            },
        );
        component
    }

    #[test]
    fn adding_an_interaction_targets_another_frame() {
        let (doc, first, second) = document_with_two_frames();
        let Some(Operation::AddReaction { node, reaction }) = add_reaction_operation(&doc, first)
        else {
            panic!("expected add reaction operation");
        };
        assert_eq!(node, first);
        assert_eq!(reaction.trigger, Trigger::Click);
        assert_eq!(reaction.action, Action::Navigate { to: second });
    }

    #[test]
    fn motion_clip_choices_are_readable_and_deterministic() {
        let mut doc = Doc::new();
        let zoom = AnimationClip::new(AnimationClipId::new(), "Zoom", 500);
        let untitled = AnimationClip::new(AnimationClipId::new(), "  ", 500);
        let alpha = AnimationClip::new(AnimationClipId::new(), "alpha", 500);
        doc.motion.clips.insert(zoom.id, zoom);
        doc.motion.clips.insert(untitled.id, untitled);
        doc.motion.clips.insert(alpha.id, alpha);

        assert_eq!(
            prototype_clips(&doc)
                .iter()
                .map(|clip| clip.name.as_ref())
                .collect::<Vec<_>>(),
            vec!["alpha", "Untitled animation", "Zoom"]
        );
    }

    #[test]
    fn property_animation_binding_preserves_delay_and_reaction_identity() {
        let (doc, _, _) = document_with_two_frames();
        let first_clip = AnimationClipId::new();
        let second_clip = AnimationClipId::new();
        let reaction_id = ReactionId::new();
        let mut reaction = Reaction {
            id: reaction_id,
            trigger: Trigger::Click,
            action: Action::Back,
            extra_actions: Vec::new(),
            transition: None,
            animation: Some(PrototypeAnimation {
                clip: first_clip,
                delay_ms: 125,
            }),
        };

        bind_reaction_animation(&mut reaction, Some(second_clip));
        assert_eq!(reaction.id, reaction_id);
        assert_eq!(
            reaction.animation,
            Some(PrototypeAnimation {
                clip: second_clip,
                delay_ms: 125,
            })
        );

        let edited =
            parameter_reaction_from_text(&doc, &reaction, ParameterKind::AnimationDelay, "275")
                .expect("valid property-animation delay");
        assert_eq!(edited.id, reaction_id);
        assert_eq!(
            edited
                .animation
                .as_ref()
                .map(|animation| animation.delay_ms),
            Some(275)
        );

        bind_reaction_animation(&mut reaction, None);
        assert!(reaction.animation.is_none());
        assert!(
            parameter_reaction_from_text(&doc, &reaction, ParameterKind::AnimationDelay, "300",)
                .is_err()
        );
    }

    #[test]
    fn reaction_edits_resolve_by_stable_id_not_stale_index() {
        let (mut doc, first, _) = document_with_two_frames();
        let reaction_id = ReactionId::new();
        doc.apply(Operation::AddReaction {
            node: first,
            reaction: Reaction {
                id: reaction_id,
                trigger: Trigger::Click,
                action: Action::Back,
                extra_actions: Vec::new(),
                transition: None,
                animation: None,
            },
        })
        .unwrap();
        let operation = set_reaction_operation(&doc, first, reaction_id, |reaction| {
            reaction.trigger = Trigger::Hover;
        })
        .expect("reaction exists");
        doc.apply(operation).unwrap();
        assert_eq!(
            doc.scene.get(first).unwrap().reactions[0].trigger,
            Trigger::Hover
        );
    }

    #[test]
    fn new_action_defaults_use_document_compatible_parameters() {
        let (mut doc, first, _) = document_with_two_frames();
        let variable = add_float_variable(&mut doc, 42.5);
        let component = add_variant_set(&mut doc);

        assert_eq!(
            action_for_choice(&doc, first, ActionChoice::SetVariable),
            Some(Action::SetVariable {
                variable,
                value: VarValue::Float { value: 42.5 },
            })
        );
        assert_eq!(
            action_for_choice(&doc, first, ActionChoice::UpdateVariant),
            Some(Action::UpdateVariant {
                component,
                variant: "Default".into(),
            })
        );
        assert_eq!(
            action_for_choice(&doc, first, ActionChoice::OpenLink),
            Some(Action::OpenLink { url: String::new() })
        );
    }

    #[test]
    fn authored_parameters_preserve_reaction_identity_and_typed_values() {
        let (mut doc, first, _) = document_with_two_frames();
        let variable = add_float_variable(&mut doc, 1.0);
        let reaction_id = ReactionId::new();
        let baseline = Reaction {
            id: reaction_id,
            trigger: Trigger::AfterDelay { delay_ms: 300 },
            action: Action::SetVariable {
                variable,
                value: VarValue::Float { value: 1.0 },
            },
            extra_actions: Vec::new(),
            transition: None,
            animation: None,
        };
        doc.apply(Operation::AddReaction {
            node: first,
            reaction: baseline.clone(),
        })
        .expect("add reaction");

        let delayed = parameter_reaction_from_text(&doc, &baseline, ParameterKind::Delay, "725")
            .expect("valid delay");
        assert_eq!(delayed.id, reaction_id);
        assert_eq!(delayed.trigger, Trigger::AfterDelay { delay_ms: 725 });

        let valued =
            parameter_reaction_from_text(&doc, &baseline, ParameterKind::VariableValue, "12.5")
                .expect("valid typed variable value");
        assert_eq!(valued.id, reaction_id);
        assert_eq!(
            valued.action,
            Action::SetVariable {
                variable,
                value: VarValue::Float { value: 12.5 },
            }
        );

        let operation = set_reaction_operation(&doc, first, reaction_id, |reaction| {
            *reaction = valued;
        })
        .expect("parameter edit changes reaction");
        let Operation::SetReaction { old, new, .. } = &operation else {
            panic!("expected a SetReaction operation");
        };
        assert_eq!(old.id, reaction_id);
        assert_eq!(new.id, reaction_id);
        doc.apply(operation).expect("apply parameter edit");
        assert_eq!(
            reaction_by_id(&doc, first, reaction_id).map(|reaction| reaction.id),
            Some(reaction_id)
        );
    }

    #[test]
    fn preview_restore_refuses_to_overwrite_a_newer_reaction() {
        let (mut doc, first, _) = document_with_two_frames();
        let reaction_id = ReactionId::new();
        let baseline = Reaction {
            id: reaction_id,
            trigger: Trigger::Click,
            action: Action::Back,
            extra_actions: Vec::new(),
            transition: None,
            animation: None,
        };
        doc.apply(Operation::AddReaction {
            node: first,
            reaction: baseline.clone(),
        })
        .expect("add reaction");
        let preview = Reaction {
            trigger: Trigger::Hover,
            ..baseline.clone()
        };
        assert_eq!(
            replace_reaction_preview_if_current(
                &mut doc,
                first,
                reaction_id,
                &baseline,
                preview.clone(),
            ),
            Some(true)
        );
        let newer = Reaction {
            trigger: Trigger::WhilePressing,
            ..baseline.clone()
        };
        doc.scene.get_mut(first).expect("selected frame").reactions[0] = newer.clone();

        assert_eq!(
            replace_reaction_preview_if_current(&mut doc, first, reaction_id, &preview, baseline,),
            None
        );
        assert_eq!(reaction_by_id(&doc, first, reaction_id), Some(&newer));
    }

    #[test]
    fn key_link_and_manual_overlay_parameters_are_editable() {
        let (doc, _, _) = document_with_two_frames();
        let reaction_id = ReactionId::new();
        let key = Reaction {
            id: reaction_id,
            trigger: Trigger::Key {
                keys: vec!["Enter".into()],
            },
            action: Action::OpenLink { url: String::new() },
            extra_actions: Vec::new(),
            transition: None,
            animation: None,
        };
        let keyed = parameter_reaction_from_text(&doc, &key, ParameterKind::Keys, "Shift + K")
            .expect("valid key chord");
        assert_eq!(
            keyed.trigger,
            Trigger::Key {
                keys: vec!["Shift".into(), "K".into()],
            }
        );
        let linked =
            parameter_reaction_from_text(&doc, &key, ParameterKind::Url, "https://example.com")
                .expect("valid link");
        assert_eq!(
            linked.action,
            Action::OpenLink {
                url: "https://example.com".into(),
            }
        );

        let overlay = Reaction {
            id: reaction_id,
            trigger: Trigger::Click,
            action: Action::OpenOverlay {
                frame: NodeId::new(),
                overlay: OverlaySettings {
                    position: OverlayPosition::Manual { offset: [0.0, 4.0] },
                    background_dim: true,
                    close_on_click_outside: true,
                },
            },
            extra_actions: Vec::new(),
            transition: None,
            animation: None,
        };
        let offset =
            parameter_reaction_from_text(&doc, &overlay, ParameterKind::OverlayOffsetX, "18.25")
                .expect("valid manual offset");
        let Action::OpenOverlay { overlay, .. } = offset.action else {
            panic!("expected overlay action");
        };
        assert_eq!(
            overlay.position,
            OverlayPosition::Manual {
                offset: [18.25, 4.0]
            }
        );
    }

    #[test]
    fn flow_start_builder_avoids_noop_history_entries() {
        let (mut doc, first, _) = document_with_two_frames();
        let operation = flow_start_operation(&doc, Some(first)).expect("flow start changes");
        doc.apply(operation).unwrap();
        assert!(flow_start_operation(&doc, Some(first)).is_none());
        assert!(flow_start_operation(&doc, None).is_some());
    }

    #[test]
    fn directional_transition_edits_preserve_duration_and_easing() {
        let (mut doc, first, _) = document_with_two_frames();
        let reaction_id = ReactionId::new();
        doc.apply(Operation::AddReaction {
            node: first,
            reaction: Reaction {
                id: reaction_id,
                trigger: Trigger::Click,
                action: Action::Back,
                extra_actions: Vec::new(),
                transition: Some(Transition {
                    style: TransitionStyle::SlideIn {
                        direction: Direction::Left,
                    },
                    duration_ms: 500,
                    easing: Easing::EaseOut,
                }),
                animation: None,
            },
        })
        .expect("add reaction");
        let operation = set_reaction_operation(&doc, first, reaction_id, |reaction| {
            let Some(transition) = &mut reaction.transition else {
                return;
            };
            transition.style = TransitionStyle::SlideIn {
                direction: Direction::Right,
            };
        })
        .expect("direction changed");
        doc.apply(operation).expect("apply direction change");

        let transition = doc
            .scene
            .get(first)
            .and_then(|node| node.reactions.first())
            .and_then(|reaction| reaction.transition.as_ref())
            .expect("transition remains");
        assert_eq!(transition.duration_ms, 500);
        assert_eq!(transition.easing, Easing::EaseOut);
        assert_eq!(
            direction_choice(transition.style),
            Some(DirectionChoice::Right)
        );
    }

    #[gpui::test]
    async fn property_animation_binding_is_one_undoable_panel_edit(cx: &mut TestAppContext) {
        init_test(cx);
        let (mut doc, first, _) = document_with_two_frames();
        let clip = AnimationClip::new(AnimationClipId::new(), "Entrance", 500);
        let clip_id = clip.id;
        doc.motion.clips.insert(clip_id, clip);
        let reaction_id = ReactionId::new();
        doc.apply(Operation::AddReaction {
            node: first,
            reaction: Reaction {
                id: reaction_id,
                trigger: Trigger::Click,
                action: Action::Back,
                extra_actions: Vec::new(),
                transition: None,
                animation: None,
            },
        })
        .expect("add reaction");
        doc.selection.select_only(first);
        doc.history = Default::default();

        let file_system = FakeFs::new(cx.executor());
        let roots: [&std::path::Path; 0] = [];
        let project = Project::test(file_system, roots, cx).await;
        let item = ready_item_for_test(
            &project,
            PathBuf::from("/tmp/PrototypeMotionBinding.fanta"),
            doc,
            cx,
        );
        let panel_item = item.clone();
        let panel = cx.add_window(move |_, cx| FantaPrototypePanel::new(panel_item, cx));
        cx.update_window(panel.into(), |_, window, cx| {
            window.draw(cx).clear();
        })
        .expect("draw prototype panel");

        panel
            .update(cx, |panel, _, cx| {
                panel.set_animation_clip(first, reaction_id, Some(clip_id), cx)
            })
            .expect("bind property animation");
        cx.run_until_parked();
        item.read_with(cx, |item, _| {
            let reaction = reaction_by_id(
                &item.document().expect("ready document").doc,
                first,
                reaction_id,
            )
            .expect("reaction remains");
            assert_eq!(
                reaction.animation,
                Some(PrototypeAnimation {
                    clip: clip_id,
                    delay_ms: 0,
                })
            );
        });

        assert!(
            item.update(cx, |item, cx| item.undo(cx))
                .expect("undo binding")
        );
        item.read_with(cx, |item, _| {
            assert!(
                reaction_by_id(
                    &item.document().expect("ready document").doc,
                    first,
                    reaction_id,
                )
                .expect("reaction remains after undo")
                .animation
                .is_none()
            );
        });
    }

    #[gpui::test]
    async fn inline_parameter_previews_live_and_commits_as_one_stable_id_operation(
        cx: &mut TestAppContext,
    ) {
        init_test(cx);
        let (mut doc, first, _) = document_with_two_frames();
        let reaction_id = ReactionId::new();
        doc.apply(Operation::AddReaction {
            node: first,
            reaction: Reaction {
                id: reaction_id,
                trigger: Trigger::AfterDelay { delay_ms: 300 },
                action: Action::Back,
                extra_actions: Vec::new(),
                transition: None,
                animation: None,
            },
        })
        .expect("add reaction");
        doc.selection.select_only(first);

        let file_system = FakeFs::new(cx.executor());
        let roots: [&std::path::Path; 0] = [];
        let project = Project::test(file_system, roots, cx).await;
        let item = ready_item_for_test(&project, PathBuf::from("/tmp/Prototype.fanta"), doc, cx);
        let panel_item = item.clone();
        let panel = cx.add_window(move |_, cx| FantaPrototypePanel::new(panel_item, cx));
        cx.update_window(panel.into(), |_, window, cx| {
            window.draw(cx).clear();
        })
        .expect("draw prototype panel");

        panel
            .update(cx, |panel, window, cx| {
                panel.start_parameter_edit(
                    ReactionParameter {
                        node: first,
                        reaction: reaction_id,
                        kind: ParameterKind::Delay,
                    },
                    "300".into(),
                    window,
                    cx,
                );
            })
            .expect("start delay edit");
        let editor = panel
            .read_with(cx, |panel, _| panel.parameter_editor.clone())
            .expect("read prototype panel")
            .expect("parameter editor exists");
        cx.update_window(panel.into(), |_, window, cx| {
            editor.update(cx, |editor, cx| editor.set_text("725", window, cx));
        })
        .expect("type a new delay");
        cx.run_until_parked();

        item.read_with(cx, |item, _| {
            let reaction = reaction_by_id(
                &item.document().expect("ready document").doc,
                first,
                reaction_id,
            )
            .expect("reaction remains");
            assert_eq!(reaction.id, reaction_id);
            assert_eq!(reaction.trigger, Trigger::AfterDelay { delay_ms: 725 });
        });

        panel
            .update(cx, |panel, _, cx| panel.commit_parameter_edit_value(cx))
            .expect("commit delay edit");
        cx.run_until_parked();
        let undone = item
            .update(cx, |item, cx| item.undo(cx))
            .expect("undo delay edit");
        assert!(undone, "the committed field edit should be one undo step");
        item.read_with(cx, |item, _| {
            let reaction = reaction_by_id(
                &item.document().expect("ready document").doc,
                first,
                reaction_id,
            )
            .expect("reaction remains after undo");
            assert_eq!(reaction.id, reaction_id);
            assert_eq!(reaction.trigger, Trigger::AfterDelay { delay_ms: 300 });
        });
    }

    #[gpui::test]
    async fn explicit_finish_restores_the_last_valid_preview_when_input_is_invalid(
        cx: &mut TestAppContext,
    ) {
        init_test(cx);
        let (mut doc, first, _) = document_with_two_frames();
        let reaction_id = ReactionId::new();
        doc.apply(Operation::AddReaction {
            node: first,
            reaction: Reaction {
                id: reaction_id,
                trigger: Trigger::AfterDelay { delay_ms: 300 },
                action: Action::Back,
                extra_actions: Vec::new(),
                transition: None,
                animation: None,
            },
        })
        .expect("add reaction");
        doc.selection.select_only(first);
        doc.history = Default::default();

        let file_system = FakeFs::new(cx.executor());
        let roots: [&std::path::Path; 0] = [];
        let project = Project::test(file_system, roots, cx).await;
        let item = ready_item_for_test(
            &project,
            PathBuf::from("/tmp/PrototypeInvalidFinish.fanta"),
            doc,
            cx,
        );
        let panel_item = item.clone();
        let panel = cx.add_window(move |_, cx| FantaPrototypePanel::new(panel_item, cx));
        cx.update_window(panel.into(), |_, window, cx| {
            window.draw(cx).clear();
        })
        .expect("draw prototype panel");
        panel
            .update(cx, |panel, window, cx| {
                panel.start_parameter_edit(
                    ReactionParameter {
                        node: first,
                        reaction: reaction_id,
                        kind: ParameterKind::Delay,
                    },
                    "300".into(),
                    window,
                    cx,
                );
            })
            .expect("start delay edit");
        let editor = panel
            .read_with(cx, |panel, _| panel.parameter_editor.clone())
            .expect("read prototype panel")
            .expect("parameter editor exists");
        cx.update_window(panel.into(), |_, window, cx| {
            editor.update(cx, |editor, cx| editor.set_text("725", window, cx));
        })
        .expect("preview valid delay");
        cx.run_until_parked();
        cx.update_window(panel.into(), |_, window, cx| {
            editor.update(cx, |editor, cx| editor.set_text("invalid", window, cx));
        })
        .expect("type invalid delay");
        cx.run_until_parked();

        item.read_with(cx, |item, _| {
            assert_eq!(
                reaction_by_id(
                    &item.document().expect("ready document").doc,
                    first,
                    reaction_id,
                )
                .map(|reaction| &reaction.trigger),
                Some(&Trigger::AfterDelay { delay_ms: 725 })
            );
        });
        panel
            .update(cx, |panel, _, cx| panel.finish_parameter_edit(cx))
            .expect("finish invalid delay edit");
        cx.run_until_parked();

        assert!(
            panel
                .read_with(cx, |panel, _| panel.editing_parameter.is_none())
                .expect("read prototype panel")
        );
        item.read_with(cx, |item, _| {
            let doc = &item.document().expect("ready document").doc;
            assert_eq!(
                reaction_by_id(doc, first, reaction_id).map(|reaction| &reaction.trigger),
                Some(&Trigger::AfterDelay { delay_ms: 300 })
            );
            assert_eq!(doc.history.undo_depth(), 0);
            assert!(!item.is_dirty());
        });

        panel
            .update(cx, |panel, window, cx| {
                panel.start_parameter_edit(
                    ReactionParameter {
                        node: first,
                        reaction: reaction_id,
                        kind: ParameterKind::Delay,
                    },
                    "300".into(),
                    window,
                    cx,
                );
            })
            .expect("restart delay edit");
        cx.update_window(panel.into(), |_, window, cx| {
            editor.update(cx, |editor, cx| editor.set_text("725", window, cx));
        })
        .expect("preview another valid delay");
        cx.run_until_parked();
        cx.update_window(panel.into(), |_, window, cx| {
            editor.update(cx, |editor, cx| editor.set_text("invalid", window, cx));
        })
        .expect("type another invalid delay");
        cx.run_until_parked();
        panel
            .update(cx, |panel, _, cx| {
                panel.set_transition_choice(first, reaction_id, TransitionChoice::Dissolve, cx)
            })
            .expect("apply a discrete reaction edit");
        cx.run_until_parked();

        item.read_with(cx, |item, _| {
            let doc = &item.document().expect("ready document").doc;
            let reaction = reaction_by_id(doc, first, reaction_id).expect("reaction remains");
            assert_eq!(reaction.trigger, Trigger::AfterDelay { delay_ms: 300 });
            assert_eq!(
                reaction
                    .transition
                    .as_ref()
                    .map(|transition| transition.style),
                Some(TransitionStyle::Dissolve)
            );
            assert_eq!(doc.history.undo_depth(), 1);
        });
        assert!(
            item.update(cx, |item, cx| item.undo(cx))
                .expect("undo discrete edit")
        );
        item.read_with(cx, |item, _| {
            let reaction = reaction_by_id(
                &item.document().expect("ready document").doc,
                first,
                reaction_id,
            )
            .expect("reaction remains after undo");
            assert_eq!(reaction.trigger, Trigger::AfterDelay { delay_ms: 300 });
            assert!(reaction.transition.is_none());
        });
    }

    #[gpui::test]
    async fn source_lock_cancels_and_restores_an_active_parameter_preview(cx: &mut TestAppContext) {
        init_test(cx);
        let (mut doc, first, _) = document_with_two_frames();
        let reaction_id = ReactionId::new();
        doc.apply(Operation::AddReaction {
            node: first,
            reaction: Reaction {
                id: reaction_id,
                trigger: Trigger::AfterDelay { delay_ms: 300 },
                action: Action::Back,
                extra_actions: Vec::new(),
                transition: None,
                animation: None,
            },
        })
        .expect("add reaction");
        doc.selection.select_only(first);

        let file_system = FakeFs::new(cx.executor());
        let roots: [&std::path::Path; 0] = [];
        let project = Project::test(file_system, roots, cx).await;
        let item = ready_item_for_test(
            &project,
            PathBuf::from("/tmp/PrototypeLocked.fanta"),
            doc,
            cx,
        );
        let panel_item = item.clone();
        let panel = cx.add_window(move |_, cx| FantaPrototypePanel::new(panel_item, cx));
        cx.update_window(panel.into(), |_, window, cx| {
            window.draw(cx).clear();
        })
        .expect("draw prototype panel");
        panel
            .update(cx, |panel, window, cx| {
                panel.start_parameter_edit(
                    ReactionParameter {
                        node: first,
                        reaction: reaction_id,
                        kind: ParameterKind::Delay,
                    },
                    "300".into(),
                    window,
                    cx,
                );
            })
            .expect("start delay edit");
        let editor = panel
            .read_with(cx, |panel, _| panel.parameter_editor.clone())
            .expect("read prototype panel")
            .expect("parameter editor exists");
        cx.update_window(panel.into(), |_, window, cx| {
            editor.update(cx, |editor, cx| editor.set_text("900", window, cx));
        })
        .expect("preview delay");
        cx.run_until_parked();

        item.update(cx, |item, cx| item.set_source_edit_locked(true, cx));
        cx.run_until_parked();
        assert!(
            panel
                .read_with(cx, |panel, _| panel.editing_parameter.is_none())
                .expect("read prototype panel"),
            "source locking should abandon the inline edit"
        );
        item.read_with(cx, |item, _| {
            let reaction = reaction_by_id(
                &item.document().expect("ready document").doc,
                first,
                reaction_id,
            )
            .expect("reaction remains");
            assert_eq!(reaction.trigger, Trigger::AfterDelay { delay_ms: 300 });
        });
    }
}
