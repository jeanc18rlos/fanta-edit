//! Prototype reactions (pass 4): triggers, actions, transitions.

use super::{
    Action, Doc, Easing, HashMap, KiwiValue, MapReport, NodeId, Reaction, ReactionId, Transition,
    TransitionStyle, Trigger, guid_key, guid_to_variable_id, read_var_value,
};

/// Attach prototype reactions to their nodes, resolving `transitionNodeID`
/// guids to scene `NodeId`s.
pub(crate) fn apply_reactions(
    doc: &mut Doc,
    report: &mut MapReport,
    pending: &[(NodeId, Vec<KiwiValue>)],
    guid_to_node: &HashMap<String, Option<NodeId>>,
) {
    for (node_id, interactions) in pending {
        // Skip nodes that were dropped (inside an instance).
        if doc.scene.get(*node_id).is_none() {
            continue;
        }
        let mut reactions: Vec<Reaction> = Vec::new();
        for interaction in interactions {
            if matches!(interaction.get("isDeleted"), Some(KiwiValue::Bool(true))) {
                continue;
            }
            let trigger = interaction
                .get("event")
                .and_then(read_trigger)
                .unwrap_or(Trigger::Click);
            // Take the first action with a navigable target (or set-variable);
            // multi-action interactions collapse to their primary action in v1.
            let Some(action_change) = interaction
                .get("actions")
                .and_then(KiwiValue::as_array)
                .and_then(|a| a.first())
            else {
                continue;
            };
            let Some((action, transition)) = read_action(action_change, guid_to_node) else {
                continue;
            };
            reactions.push(Reaction {
                id: ReactionId::new(),
                trigger,
                action,
                transition,
            });
        }
        if !reactions.is_empty() {
            report.reactions += reactions.len();
            if let Some(node) = doc.scene.get_mut(*node_id) {
                node.reactions = reactions;
            }
        }
    }
}

/// Map a Figma `PrototypeEvent` to a [`Trigger`].
pub(crate) fn read_trigger(event: &KiwiValue) -> Option<Trigger> {
    let kind = event.get("interactionType").and_then(KiwiValue::as_str)?;
    Some(match kind {
        "ON_CLICK" | "ON_PRESS" | "MOUSE_DOWN" | "MOUSE_UP" => Trigger::Click,
        "DRAG" => Trigger::Drag,
        "ON_HOVER" | "MOUSE_IN" | "MOUSE_ENTER" | "MOUSE_OUT" | "MOUSE_LEAVE" => Trigger::Hover,
        "AFTER_TIMEOUT" => {
            let delay = event
                .get("interactionDuration")
                .and_then(KiwiValue::as_f64)
                .unwrap_or(0.0);
            Trigger::AfterDelay {
                delay_ms: (delay * 1000.0).round().max(0.0) as u32,
            }
        }
        "ON_KEY_DOWN" => Trigger::Key { keys: Vec::new() },
        _ => Trigger::Click,
    })
}

/// Map a Figma `PrototypeAction` to an [`Action`] (+ optional [`Transition`]).
pub(crate) fn read_action(
    action: &KiwiValue,
    guid_to_node: &HashMap<String, Option<NodeId>>,
) -> Option<(Action, Option<Transition>)> {
    let transition = read_transition(action);
    // Navigate is by far the most common: a transitionNodeID resolving to a
    // frame. Resolve the guid to a scene NodeId; tolerate a stale target by
    // skipping rather than fabricating.
    if let Some(target_guid) = action.get("transitionNodeID").and_then(guid_key) {
        if let Some(Some(to)) = guid_to_node.get(&target_guid) {
            return Some((Action::Navigate { to: *to }, transition));
        }
        // Target not in scene (deleted / unmapped) — still record a Navigate to
        // a placeholder so the interaction isn't silently lost? No: validate
        // tolerates dangling Navigate, but we'd need a NodeId. Drop instead.
        return None;
    }
    // SetVariable action (targetVariableID + value).
    if let Some(var_guid) = action.get("targetVariableID").and_then(guid_key) {
        if let Some(value) = action.get("targetVariableData").and_then(read_var_value) {
            return Some((
                Action::SetVariable {
                    variable: guid_to_variable_id(&var_guid),
                    value,
                },
                transition,
            ));
        }
    }
    None
}

/// Map a Figma action's transition fields to a [`Transition`]. Returns `None`
/// for an instant transition (no animation), which is also the model default.
pub(crate) fn read_transition(action: &KiwiValue) -> Option<Transition> {
    let ty = action.get("transitionType").and_then(KiwiValue::as_str)?;
    let style = match ty {
        "INSTANT_TRANSITION" => return None,
        "DISSOLVE" | "FADE" => TransitionStyle::Dissolve,
        "SLIDE_FROM_LEFT" => TransitionStyle::SlideIn {
            direction: fanta_doc::node::Direction::Left,
        },
        "SLIDE_FROM_RIGHT" => TransitionStyle::SlideIn {
            direction: fanta_doc::node::Direction::Right,
        },
        "SLIDE_FROM_TOP" => TransitionStyle::SlideIn {
            direction: fanta_doc::node::Direction::Up,
        },
        "SLIDE_FROM_BOTTOM" => TransitionStyle::SlideIn {
            direction: fanta_doc::node::Direction::Down,
        },
        "PUSH_FROM_LEFT" => TransitionStyle::Push {
            direction: fanta_doc::node::Direction::Left,
        },
        "PUSH_FROM_RIGHT" => TransitionStyle::Push {
            direction: fanta_doc::node::Direction::Right,
        },
        "PUSH_FROM_TOP" => TransitionStyle::Push {
            direction: fanta_doc::node::Direction::Up,
        },
        "PUSH_FROM_BOTTOM" => TransitionStyle::Push {
            direction: fanta_doc::node::Direction::Down,
        },
        "MOVE_FROM_LEFT" => TransitionStyle::MoveIn {
            direction: fanta_doc::node::Direction::Left,
        },
        "MOVE_FROM_RIGHT" => TransitionStyle::MoveIn {
            direction: fanta_doc::node::Direction::Right,
        },
        "MOVE_FROM_TOP" => TransitionStyle::MoveIn {
            direction: fanta_doc::node::Direction::Up,
        },
        "MOVE_FROM_BOTTOM" => TransitionStyle::MoveIn {
            direction: fanta_doc::node::Direction::Down,
        },
        _ => return None,
    };
    let duration_ms = action
        .get("transitionDuration")
        .and_then(KiwiValue::as_f64)
        .map(|s| (s * 1000.0).round().max(0.0) as u32)
        .unwrap_or(300);
    let easing = match action.get("easingType").and_then(KiwiValue::as_str) {
        Some("LINEAR") => Easing::Linear,
        Some("IN_CUBIC") | Some("IN_BACK_CUBIC") => Easing::EaseIn,
        Some("OUT_CUBIC") | Some("OUT_BACK_CUBIC") => Easing::EaseOut,
        _ => Easing::EaseInOut,
    };
    Some(Transition {
        style,
        duration_ms,
        easing,
    })
}
