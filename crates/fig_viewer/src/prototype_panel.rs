use fanta_doc::{
    Action, Direction, Doc, Easing, NodeData, NodeId, Operation, OverlayPosition, OverlaySettings,
    Reaction, ReactionId, Transition, TransitionStyle, Trigger,
};
use gpui::{
    AnyElement, App, Context, Entity, EventEmitter, FocusHandle, Focusable, Render, SharedString,
    Subscription, Window,
};
use ui::{
    ContextMenu, ContextMenuEntry, Divider, DropdownMenu, DropdownStyle, IconPosition, Tooltip,
    prelude::*,
};
use util::ResultExt;

use crate::document::{FigItem, FigItemEvent};
use crate::inspector_components::{InspectorMessage, InspectorPropertyRow, InspectorSectionHeader};

const DEFAULT_TRANSITION_DURATION_MS: u32 = 300;

#[derive(Clone)]
struct PrototypeTarget {
    id: NodeId,
    name: SharedString,
}

enum PrototypeSnapshot {
    Message(SharedString),
    Selection {
        editable: bool,
        node: NodeId,
        name: SharedString,
        can_start_flow: bool,
        is_flow_start: bool,
        reactions: Vec<Reaction>,
        targets: Vec<PrototypeTarget>,
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
    Back,
    Close,
}

const ACTION_CHOICES: [(ActionChoice, &str); 5] = [
    (ActionChoice::Navigate, "Navigate to"),
    (ActionChoice::OpenOverlay, "Open overlay"),
    (ActionChoice::ScrollTo, "Scroll to"),
    (ActionChoice::Back, "Back"),
    (ActionChoice::Close, "Close"),
];

impl ActionChoice {
    fn from_action(action: &Action) -> Option<Self> {
        match action {
            Action::Navigate { .. } => Some(Self::Navigate),
            Action::OpenOverlay { .. } => Some(Self::OpenOverlay),
            Action::ScrollTo { .. } => Some(Self::ScrollTo),
            Action::Back => Some(Self::Back),
            Action::Close => Some(Self::Close),
            Action::SetVariable { .. } | Action::UpdateVariant { .. } | Action::OpenLink { .. } => {
                None
            }
        }
    }
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
            Easing::CubicBezier { .. } => None,
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
    _item_subscription: Subscription,
}

impl FantaPrototypePanel {
    pub fn new(item: Entity<FigItem>, cx: &mut Context<Self>) -> Self {
        let item_subscription = cx.subscribe(&item, |_, _, _: &FigItemEvent, cx| cx.notify());
        Self {
            item,
            focus_handle: cx.focus_handle(),
            _item_subscription: item_subscription,
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
        let targets = prototype_targets(doc, node_id);
        PrototypeSnapshot::Selection {
            editable: item.is_editable(),
            node: node_id,
            name: node.name.clone().into(),
            can_start_flow: matches!(&node.data, NodeData::Group(group) if group.is_frame_surface()),
            is_flow_start: doc.flow_start() == Some(node_id),
            reactions: node.reactions.clone(),
            targets,
        }
    }

    fn apply_operation(
        &mut self,
        build: impl FnOnce(&Doc) -> Option<Operation>,
        cx: &mut Context<Self>,
    ) {
        let operation = {
            let item = self.item.read(cx);
            if !item.is_editable() {
                return;
            }
            let Some(document) = item.document() else {
                return;
            };
            build(&document.doc)
        };
        let Some(operation) = operation else {
            return;
        };
        self.item.update(cx, |item, cx| {
            if let Err(error) = item.apply(operation, cx) {
                log::error!("Fanta prototype panel failed to apply operation: {error:#}");
            }
        });
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
                let target = prototype_targets(doc, node).first().map(|target| target.id);
                let action = match choice {
                    ActionChoice::Navigate => Action::Navigate { to: target? },
                    ActionChoice::OpenOverlay => Action::OpenOverlay {
                        frame: target?,
                        overlay: default_overlay_settings(),
                    },
                    ActionChoice::ScrollTo => Action::ScrollTo { target: target? },
                    ActionChoice::Back => Action::Back,
                    ActionChoice::Close => Action::Close,
                };
                set_reaction_operation(doc, node, reaction, |reaction| {
                    reaction.action = action;
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

    #[allow(clippy::too_many_arguments)]
    fn render_choice_dropdown<T: Copy + PartialEq + 'static>(
        &self,
        key: &'static str,
        index: usize,
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
        DropdownMenu::new((key, index), label, menu)
            .style(DropdownStyle::Outlined)
            .trigger_size(ButtonSize::Compact)
            .full_width(true)
            .disabled(!editable)
            .aria_label(aria_label)
            .into_any_element()
    }

    fn render_target_dropdown(
        &self,
        index: usize,
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
            DropdownMenu::new(("fanta-prototype-target", index), label, menu)
                .style(DropdownStyle::Outlined)
                .trigger_size(ButtonSize::Compact)
                .full_width(true)
                .disabled(!editable || targets.is_empty())
                .aria_label("Prototype target")
                .into_any_element(),
        )
    }

    fn render_reaction(
        &self,
        index: usize,
        node: NodeId,
        reaction: &Reaction,
        targets: &[PrototypeTarget],
        editable: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let trigger = TriggerChoice::from_trigger(&reaction.trigger);
        let action = ActionChoice::from_action(&reaction.action);
        let transition = TransitionChoice::from_transition(reaction.transition.as_ref());
        let reaction_id = reaction.id;
        let mut card = v_flex()
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
                            IconButton::new(("fanta-prototype-remove", index), IconName::Close)
                                .icon_size(IconSize::XSmall)
                                .tooltip(Tooltip::text("Remove interaction"))
                                .on_click(cx.listener(move |panel, _, _, cx| {
                                    panel.remove_interaction(node, reaction_id, cx)
                                })),
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
            ))
            .child(self.render_labeled_row(
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
        if let Some(target) =
            self.render_target_dropdown(index, node, reaction, targets, editable, window, cx)
        {
            card = card.child(self.render_labeled_row("Destination", target));
        }
        card = card.child(self.render_labeled_row(
            "Animation",
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
        let root = v_flex()
            .track_focus(&self.focus_handle)
            .size_full()
            .bg(cx.theme().colors().panel_background);
        match self.snapshot(cx) {
            PrototypeSnapshot::Message(message) => root.child(InspectorMessage::new(message)),
            PrototypeSnapshot::Selection {
                editable,
                node,
                name,
                can_start_flow,
                is_flow_start,
                reactions,
                targets,
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
                    .child(Label::new(name).single_line())
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
                        h_flex().px_4().child(
                            Label::new("No interactions")
                                .size(LabelSize::Small)
                                .color(Color::Muted),
                        ),
                    );
                } else {
                    for (index, reaction) in reactions.iter().enumerate() {
                        content = content.child(self.render_reaction(
                            index, node, reaction, &targets, editable, window, cx,
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

fn prototype_targets(doc: &Doc, selected: NodeId) -> Vec<PrototypeTarget> {
    let mut targets = Vec::new();
    for root in doc.scene.roots() {
        for id in doc.scene.descendants_of(*root) {
            if id == selected {
                continue;
            }
            let Some(node) = doc.scene.get(id) else {
                continue;
            };
            if matches!(&node.data, NodeData::Group(group) if group.is_frame_surface()) {
                targets.push(PrototypeTarget {
                    id,
                    name: if node.name.is_empty() {
                        "Untitled frame".into()
                    } else {
                        node.name.clone().into()
                    },
                });
            }
        }
    }
    targets
}

fn add_reaction_operation(doc: &Doc, node: NodeId) -> Option<Operation> {
    let selected = doc.scene.get(node)?;
    let action = prototype_targets(doc, node)
        .first()
        .map(|target| Action::Navigate { to: target.id })
        .unwrap_or(Action::Back);
    Some(Operation::AddReaction {
        node: selected.id,
        reaction: Reaction {
            id: ReactionId::new(),
            trigger: Trigger::Click,
            action,
            transition: None,
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
        TransitionStyle::Instant | TransitionStyle::Dissolve | TransitionStyle::SmartAnimate => {
            None
        }
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
    use fanta_doc::{CanvasNode, GroupNode};

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
    fn reaction_edits_resolve_by_stable_id_not_stale_index() {
        let (mut doc, first, _) = document_with_two_frames();
        let reaction_id = ReactionId::new();
        doc.apply(Operation::AddReaction {
            node: first,
            reaction: Reaction {
                id: reaction_id,
                trigger: Trigger::Click,
                action: Action::Back,
                transition: None,
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
                transition: Some(Transition {
                    style: TransitionStyle::SlideIn {
                        direction: Direction::Left,
                    },
                    duration_ms: 500,
                    easing: Easing::EaseOut,
                }),
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
}
