//! Prototype reactions (pass 4): triggers, actions, transitions.

use super::{
    Action, ComponentMaps, Doc, Easing, HashMap, KiwiValue, MapReport, NodeId, Reaction,
    ReactionId, Transition, TransitionStyle, Trigger, guid_key, guid_to_variable_id,
    read_var_value,
};

/// Derive a deterministic [`ComponentId`] from a Figma variant/component guid string.
/// Mirrors the approach for VariableId so UpdateVariant actions roundtrip stably
/// even before full component map wiring.
pub(crate) fn guid_to_component_id(guid: &str) -> fanta_doc::id::ComponentId {
    fanta_doc::id::ComponentId::from_u128(stable_hash_u128(guid))
}

/// FNV-1a-ish stable 128-bit hash (copied shape from variables to avoid new dep).
fn stable_hash_u128(s: &str) -> u128 {
    let mut hi: u64 = 0xcbf2_9ce4_8422_2325;
    let mut lo: u64 = 0x84222325cbf29ce4u64.rotate_left(17);
    for (i, b) in s.bytes().enumerate() {
        hi ^= b as u64;
        hi = hi.wrapping_mul(0x100_0000_01b3);
        lo ^= (b as u64).rotate_left((i % 31) as u32);
        lo = lo.wrapping_mul(0x100_0000_01b3);
    }
    ((hi as u128) << 64) | (lo as u128)
}

/// Attach prototype reactions to their nodes, resolving `transitionNodeID`
/// guids to scene `NodeId`s.
pub(crate) fn apply_reactions(
    doc: &mut Doc,
    report: &mut MapReport,
    pending: &[(NodeId, &[KiwiValue])],
    guid_to_node: &HashMap<String, Option<NodeId>>,
    component_maps: &ComponentMaps,
) {
    for (node_id, interactions) in pending {
        // Skip nodes that were dropped (inside an instance).
        if doc.scene.get(*node_id).is_none() {
            continue;
        }
        let mut reactions: Vec<Reaction> = Vec::new();
        for interaction in interactions.iter() {
            if matches!(interaction.get("isDeleted"), Some(KiwiValue::Bool(true))) {
                continue;
            }
            // Trigger resolution distinguishes ABSENT from UNKNOWN. An absent
            // `event` keeps the historical Click default (sparse synthetic
            // fixtures rely on it, and a trigger-less interaction has no better
            // reading). An event whose `interactionType` we do NOT recognize
            // (ON_MEDIA_END, future types) DROPS the reaction and counts it:
            // an "on media end" navigation firing on click is actively worse
            // than not firing at all, and the count keeps the loss visible.
            let trigger = match interaction.get("event") {
                Some(event) => match read_trigger(event) {
                    Some(trigger) => trigger,
                    None => {
                        report.reactions_dropped_unknown_trigger += 1;
                        continue;
                    }
                },
                None => Trigger::Click,
            };
            // Parse EVERY action in the interaction's `actions` array (Figma's
            // "set variable then navigate" pairs). The first parsed action is
            // the primary `Reaction::action`; the rest ride additively in
            // `extra_actions` and the runtime executes them sequentially.
            let Some(action_changes) = interaction.get("actions").and_then(KiwiValue::as_array)
            else {
                continue;
            };
            let defs = &doc.components;
            let mut parsed: Vec<(Action, Option<Transition>)> = action_changes
                .iter()
                .filter_map(|change| read_action(change, guid_to_node, component_maps, defs))
                .collect();
            if parsed.is_empty() {
                continue;
            }
            if parsed.len() > 1 {
                report.reactions_multi_action += 1;
            }
            // ASSUMPTION: `Reaction` carries ONE transition for the whole
            // sequence. Figma stores transition fields per action; the primary
            // action's transition wins, but when the primary has none (the
            // common SetVariable-then-Navigate pair — SetVariable carries no
            // transitionType) we adopt the first transition any later action
            // authored so the pair's dissolve is not silently lost.
            let (action, mut transition) = parsed.remove(0);
            let extra_actions: Vec<Action> = parsed
                .into_iter()
                .map(|(action, extra_transition)| {
                    if transition.is_none() {
                        transition = extra_transition;
                    }
                    action
                })
                .collect();
            reactions.push(Reaction {
                id: ReactionId::new(),
                trigger,
                action,
                extra_actions,
                transition,
                animation: None,
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

/// Map a Figma `PrototypeEvent` to a [`Trigger`]. Returns `None` for an
/// unrecognized `interactionType` — the caller DROPS the reaction and counts
/// it in [`MapReport::reactions_dropped_unknown_trigger`], because defaulting
/// an unknown trigger (ON_MEDIA_END, future additions) to Click makes it fire
/// on the wrong gesture entirely.
pub(crate) fn read_trigger(event: &KiwiValue) -> Option<Trigger> {
    let kind = event.get("interactionType").and_then(KiwiValue::as_str)?;
    Some(match kind {
        // MOUSE_DOWN/MOUSE_UP stay Click: the present runtime dispatches one
        // Click per press-release and has no separate down/up trigger lanes.
        "ON_CLICK" | "ON_PRESS" | "MOUSE_DOWN" | "MOUSE_UP" => Trigger::Click,
        "DRAG" => Trigger::Drag,
        "ON_HOVER" => Trigger::Hover,
        // Previously conflated: MOUSE_OUT/MOUSE_LEAVE imported as Hover, so a
        // leave-triggered close fired the moment the pointer ENTERED. Enter
        // and leave are now distinct edges.
        "MOUSE_IN" | "MOUSE_ENTER" => Trigger::MouseEnter,
        "MOUSE_OUT" | "MOUSE_LEAVE" => Trigger::MouseLeave,
        "WHILE_HOVERING" => Trigger::WhileHovering,
        "WHILE_PRESSING" => Trigger::WhilePressing,
        "AFTER_TIMEOUT" => {
            let delay = event
                .get("interactionDuration")
                .and_then(KiwiValue::as_f64)
                .unwrap_or(0.0);
            Trigger::AfterDelay {
                delay_ms: (delay * 1000.0).round().max(0.0) as u32,
            }
        }
        "ON_KEY_DOWN" => {
            let keys = event
                .get("keyCodes")
                .and_then(KiwiValue::as_array)
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(|s| s.to_string()))
                        .collect()
                })
                .unwrap_or_default();
            Trigger::Key { keys }
        }
        // Unknown trigger type: refuse to guess (see the doc comment).
        _ => return None,
    })
}

/// Map a Figma `PrototypeAction` to an [`Action`] (+ optional [`Transition`]).
pub(crate) fn read_action(
    action: &KiwiValue,
    guid_to_node: &HashMap<String, Option<NodeId>>,
    component_maps: &ComponentMaps,
    defs: &fanta_doc::ComponentLibrary,
) -> Option<(Action, Option<Transition>)> {
    let transition = read_transition(action);

    // Interactive components (VERIFIED on a production template): a variant
    // swap ("Change to") ships as `navigationType: SWAP_STATE` with the
    // TARGET VARIANT's frame in `transitionNodeID`. Reading it as a plain
    // Navigate — the pre-verification behavior — made hovering a button jump
    // the whole presentation to the component master's page. It is an
    // UpdateVariant on the source instance, never a navigation.
    if matches!(
        action.get("navigationType").and_then(KiwiValue::as_str),
        Some("SWAP_STATE" | "CHANGE_TO" | "SWAP" | "SWAP_VARIANT")
    ) && let Some(target_guid) = action.get("transitionNodeID").and_then(guid_key)
        && let Some(&target_component) = component_maps.guid_to_component.get(&target_guid)
    {
        let variant = defs
            .defs
            .get(&target_component)
            .map(|def| def.name.clone())
            .unwrap_or_default();
        return Some((
            Action::UpdateVariant {
                component: target_component,
                variant,
            },
            transition,
        ));
    }

    // Back / Close actions (often under "navigation" { "type": "BACK" } or top level)
    if let Some(nav) = action.get("navigation") {
        if let Some(t) = nav.get("type").and_then(KiwiValue::as_str) {
            match t {
                "BACK" => return Some((Action::Back, transition)),
                "CLOSE" | "CLOSE_OVERLAY" => return Some((Action::Close, transition)),
                _ => {}
            }
        }
    }
    if action.get("type").and_then(KiwiValue::as_str) == Some("BACK") {
        return Some((Action::Back, transition));
    }
    if action.get("type").and_then(KiwiValue::as_str) == Some("CLOSE") {
        return Some((Action::Close, transition));
    }

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

    // Open Overlay
    if let Some(overlay_guid) = action.get("overlayNodeID").and_then(guid_key) {
        if let Some(Some(overlay_frame)) = guid_to_node.get(&overlay_guid) {
            let settings = read_overlay_settings(action);
            return Some((
                Action::OpenOverlay {
                    frame: *overlay_frame,
                    overlay: settings,
                },
                transition,
            ));
        }
    }

    // Scroll To
    if let Some(target_guid) = action
        .get("targetNodeID")
        .or_else(|| action.get("scrollToNodeID"))
        .and_then(guid_key)
    {
        if let Some(Some(target)) = guid_to_node.get(&target_guid) {
            return Some((Action::ScrollTo { target: *target }, transition));
        }
    }

    // Update Variant (for component prototype switching)
    if let Some(variant_guid) = action
        .get("variantNodeID")
        .or_else(|| action.get("targetNodeID"))
        .and_then(guid_key)
    {
        if let Some(value) = action.get("variantValue").and_then(KiwiValue::as_str) {
            // Prefer the real component/set id built in pass 4. Older/synthetic
            // fixtures may point at a guid outside the component maps; keep the
            // deterministic fallback so the action still round-trips.
            let comp_id = component_maps
                .guid_to_component
                .get(&variant_guid)
                .copied()
                .unwrap_or_else(|| guid_to_component_id(&variant_guid));
            return Some((
                Action::UpdateVariant {
                    component: comp_id,
                    variant: value.to_string(),
                },
                transition,
            ));
        }
    }

    // Open link
    if let Some(url) = action.get("url").and_then(KiwiValue::as_str) {
        return Some((
            Action::OpenLink {
                url: url.to_string(),
            },
            transition,
        ));
    }

    None
}

/// Read overlay settings from action (position, dim, close on outside).
fn read_overlay_settings(action: &KiwiValue) -> fanta_doc::node::OverlaySettings {
    use fanta_doc::node::{OverlayPosition, OverlaySettings};
    let position = match action.get("overlayPosition").and_then(KiwiValue::as_str) {
        Some("CENTER") => OverlayPosition::Center,
        Some("TOP_LEFT") => OverlayPosition::TopLeft,
        Some("TOP_CENTER") => OverlayPosition::TopCenter,
        Some("TOP_RIGHT") => OverlayPosition::TopRight,
        Some("BOTTOM_LEFT") => OverlayPosition::BottomLeft,
        Some("BOTTOM_CENTER") => OverlayPosition::BottomCenter,
        Some("BOTTOM_RIGHT") => OverlayPosition::BottomRight,
        Some("MANUAL") => {
            let off_x = action
                .get("overlayOffset")
                .and_then(|o| o.get("x"))
                .and_then(KiwiValue::as_f64)
                .unwrap_or(0.0);
            let off_y = action
                .get("overlayOffset")
                .and_then(|o| o.get("y"))
                .and_then(KiwiValue::as_f64)
                .unwrap_or(0.0);
            OverlayPosition::Manual {
                offset: [off_x, off_y],
            }
        }
        _ => OverlayPosition::Center,
    };
    OverlaySettings {
        position,
        background_dim: action
            .get("overlayBackgroundDim")
            .and_then(|v| {
                if let KiwiValue::Bool(b) = v {
                    Some(*b)
                } else {
                    None
                }
            })
            .unwrap_or(true),
        close_on_click_outside: action
            .get("overlayCloseOnClickOutside")
            .and_then(|v| {
                if let KiwiValue::Bool(b) = v {
                    Some(*b)
                } else {
                    None
                }
            })
            .unwrap_or(true),
    }
}

/// Map a Figma action's transition fields to a [`Transition`]. Returns `None`
/// for an instant transition (no animation), which is also the model default.
pub(crate) fn read_transition(action: &KiwiValue) -> Option<Transition> {
    let ty = action.get("transitionType").and_then(KiwiValue::as_str)?;
    let style = match ty {
        "INSTANT_TRANSITION" => return None,
        "DISSOLVE" | "FADE" => TransitionStyle::Dissolve,
        "SMART_ANIMATE" => TransitionStyle::SmartAnimate,
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
        // Out-transitions (PR-6): the OUTGOING frame animates away revealing
        // the incoming frame beneath. Note the direction language flips: the
        // In-styles' `*_FROM_X` names the entry edge, the Out-styles name the
        // EXIT edge ("slide out TO left"). ASSUMPTION: the internal Kiwi enum
        // member spelling is unverified against a real fixture — Figma's UI
        // and REST layers say `SLIDE_OUT_TO_LEFT`, older captures show
        // `SLIDE_OUT_LEFT` — so both spellings are accepted per family.
        "SLIDE_OUT_LEFT" | "SLIDE_OUT_TO_LEFT" => TransitionStyle::SlideOut {
            direction: fanta_doc::node::Direction::Left,
        },
        "SLIDE_OUT_RIGHT" | "SLIDE_OUT_TO_RIGHT" => TransitionStyle::SlideOut {
            direction: fanta_doc::node::Direction::Right,
        },
        "SLIDE_OUT_TOP" | "SLIDE_OUT_TO_TOP" => TransitionStyle::SlideOut {
            direction: fanta_doc::node::Direction::Up,
        },
        "SLIDE_OUT_BOTTOM" | "SLIDE_OUT_TO_BOTTOM" => TransitionStyle::SlideOut {
            direction: fanta_doc::node::Direction::Down,
        },
        "MOVE_OUT_LEFT" | "MOVE_OUT_TO_LEFT" => TransitionStyle::MoveOut {
            direction: fanta_doc::node::Direction::Left,
        },
        "MOVE_OUT_RIGHT" | "MOVE_OUT_TO_RIGHT" => TransitionStyle::MoveOut {
            direction: fanta_doc::node::Direction::Right,
        },
        "MOVE_OUT_TOP" | "MOVE_OUT_TO_TOP" => TransitionStyle::MoveOut {
            direction: fanta_doc::node::Direction::Up,
        },
        "MOVE_OUT_BOTTOM" | "MOVE_OUT_TO_BOTTOM" => TransitionStyle::MoveOut {
            direction: fanta_doc::node::Direction::Down,
        },
        // Scroll-animate (PR-6): on a ScrollTo action the runtime eases the
        // scroll offset over the duration instead of jumping.
        "SCROLL_ANIMATE" => TransitionStyle::ScrollAnimate,
        _ => return None,
    };
    let duration_ms = action
        .get("transitionDuration")
        .and_then(KiwiValue::as_f64)
        .map(|s| (s * 1000.0).round().max(0.0) as u32)
        .unwrap_or(300);
    let mut easing = match action.get("easingType").and_then(KiwiValue::as_str) {
        Some("LINEAR") => Easing::Linear,
        Some("IN_CUBIC") | Some("IN_BACK_CUBIC") => Easing::EaseIn,
        Some("OUT_CUBIC") | Some("OUT_BACK_CUBIC") => Easing::EaseOut,
        Some("EASE") | Some("CUSTOM_CUBIC") | Some("EASE_IN_OUT") | Some("CUSTOM") => {
            // Common custom; use a pleasant default bezier (can be refined when
            // Figma ships the control points under a separate key).
            Easing::CubicBezier {
                x1: 0.42,
                y1: 0.0,
                x2: 0.58,
                y2: 1.0,
            }
        }
        // Spring presets (PR-7): previously these degraded to a generic
        // ease-in-out, which loses the overshoot that IS the animation. The
        // real spelling — VERIFIED on a production template export — is
        // `SPRING_PRESET_ONE/TWO/…` with the parameters in the sibling
        // `easingFunction` array (read below); the triples here are only the
        // fallback when that array is absent. The pre-verification aliases
        // (GENTLE/QUICK/BOUNCY/SLOW) are kept for schema variants.
        Some(member) if member.contains("SPRING") => Easing::Spring {
            mass: 1.0,
            stiffness: 100.0,
            damping: 15.0,
        },
        Some("GENTLE") => Easing::Spring {
            mass: 1.0,
            stiffness: 100.0,
            damping: 15.0,
        },
        Some("QUICK") => Easing::Spring {
            mass: 1.0,
            stiffness: 300.0,
            damping: 20.0,
        },
        Some("BOUNCY") => Easing::Spring {
            mass: 1.0,
            stiffness: 600.0,
            damping: 15.0,
        },
        Some("SLOW") => Easing::Spring {
            mass: 1.0,
            stiffness: 80.0,
            damping: 20.0,
        },
        _ => Easing::EaseInOut,
    };

    // The `easingFunction` array is OVERLOADED by easing kind — VERIFIED on a
    // real export: for a spring easingType it carries
    // `[mass, stiffness, damping, initial_velocity]` (e.g. SPRING_PRESET_TWO
    // ships `[1, 600, 15, 0]`), NOT cubic-bezier control points. Reading it as
    // a bezier produced a nonsense curve (x1=1, y1=600) that snapped instead
    // of ringing. Only a non-spring easing reads the array as `[x1,y1,x2,y2]`.
    let is_spring = matches!(easing, Easing::Spring { .. });
    if let Some(curve) = action
        .get("easingFunction")
        .or_else(|| action.get("easingBezier"))
        .or_else(|| action.get("bezier"))
        .and_then(KiwiValue::as_array)
    {
        if curve.len() >= 4 {
            let a = curve.first().and_then(KiwiValue::as_f64).unwrap_or(0.42) as f32;
            let b = curve.get(1).and_then(KiwiValue::as_f64).unwrap_or(0.0) as f32;
            let c = curve.get(2).and_then(KiwiValue::as_f64).unwrap_or(0.58) as f32;
            let d = curve.get(3).and_then(KiwiValue::as_f64).unwrap_or(1.0) as f32;
            easing = if is_spring {
                let _initial_velocity = d; // not modeled; springs start at rest
                Easing::Spring {
                    mass: if a > 0.0 { a } else { 1.0 },
                    stiffness: if b > 0.0 { b } else { 100.0 },
                    damping: if c > 0.0 { c } else { 15.0 },
                }
            } else {
                Easing::CubicBezier {
                    x1: a,
                    y1: b,
                    x2: c,
                    y2: d,
                }
            };
        }
    }

    // Custom spring parameters (PR-7). ASSUMPTION: field names unverified
    // against a real custom-spring fixture; the two shapes accepted here are
    // `easingFunction` as an OBJECT carrying mass/stiffness/damping (the
    // object sibling of its bezier array form above — the array probe misses
    // objects, so both can coexist) and a dedicated `spring` object. A partial
    // object falls back per-key to the preset already selected, or to the
    // gentle triple when the preset was not a spring.
    let spring_params = action
        .get("easingFunction")
        .filter(|value| value.get("stiffness").is_some() || value.get("mass").is_some())
        .or_else(|| action.get("spring"));
    if let Some(params) = spring_params {
        let (preset_mass, preset_stiffness, preset_damping) = match easing {
            Easing::Spring {
                mass,
                stiffness,
                damping,
            } => (mass, stiffness, damping),
            _ => (1.0, 100.0, 15.0),
        };
        let read = |key: &str, fallback: f32| {
            params
                .get(key)
                .and_then(KiwiValue::as_f64)
                .map_or(fallback, |value| value as f32)
        };
        easing = Easing::Spring {
            mass: read("mass", preset_mass),
            stiffness: read("stiffness", preset_stiffness),
            damping: read("damping", preset_damping),
        };
    }
    Some(Transition {
        style,
        duration_ms,
        easing,
    })
}
