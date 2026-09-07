//! Runtime tests for [`PresentSession`]. Each builds a tiny doc of frames with
//! authored reactions and drives the session headlessly.

use super::*;
use fanta_doc::component::{
    ComponentDef, ComponentPropDef, ComponentPropFormatter, ComponentPropKind, ComponentSet,
    ComponentSetMembership, VariantAxis,
};
use fanta_doc::id::{AnimationClipId, AnimationTrackId, ComponentId, ComponentPropId, KeyframeId};
use fanta_doc::{
    Action, AnimationClip, AnimationTrack, BoundProp, CanvasNode, Color, Direction, Doc, Easing,
    Fill, GroupNode, InstanceNode, Keyframe, Mode, ModeId, MotionProperty, MotionTarget, NodeData,
    Operation, OverlayPosition, OverlaySettings, PrototypeAnimation, Reaction, ReactionId,
    ResolvedVarValue, Transform2D, Transition, TransitionStyle, Trigger, VarValue, Variable,
    VariableCollection, VariableCollectionId, VariableId, VariableType, VectorNode,
};
use std::collections::BTreeMap;

/// A `w`×`h` colored frame translated to `(x, y)` in world space.
fn frame_sized(x: f64, y: f64, w: f64, h: f64, color: Color) -> CanvasNode {
    let mut node = CanvasNode::new(NodeData::Group(GroupNode {
        clip_size: Some([w, h]),
        background: Some(Fill::solid(color)),
        ..GroupNode::default()
    }));
    node.transform = Transform2D::translation(x, y);
    node
}

/// A 200×200 frame (a group with a clip box, so it hit-tests as a surface)
/// translated to `x` in world space.
fn frame(x: f64) -> CanvasNode {
    let mut node = CanvasNode::new(NodeData::Group(GroupNode {
        clip_size: Some([200.0, 200.0]),
        ..GroupNode::default()
    }));
    node.transform = Transform2D::translation(x, 0.0);
    node
}

/// A 200×200 frame with an opaque solid background, so it renders distinct
/// pixels (used by the transition tests).
fn frame_colored(x: f64, color: Color) -> CanvasNode {
    let mut node = CanvasNode::new(NodeData::Group(GroupNode {
        clip_size: Some([200.0, 200.0]),
        background: Some(Fill::solid(color)),
        ..GroupNode::default()
    }));
    node.transform = Transform2D::translation(x, 0.0);
    node
}

fn click(reaction_id: u128, trigger: Trigger, action: Action) -> Reaction {
    Reaction {
        id: ReactionId::from_u128(reaction_id),
        trigger,
        action,
        extra_actions: Vec::new(),
        transition: None,
        animation: None,
    }
}

fn click_with(reaction_id: u128, action: Action, transition: Transition) -> Reaction {
    Reaction {
        id: ReactionId::from_u128(reaction_id),
        trigger: Trigger::Click,
        action,
        extra_actions: Vec::new(),
        transition: Some(transition),
        animation: None,
    }
}

#[test]
fn click_navigates_between_frames_and_back() {
    // Frame A at the origin, frame B off to the right. A → Navigate(B) on click;
    // B → Back on click.
    let mut doc = Doc::new();
    let mut b = frame(500.0);
    let b_id = b.id;
    b.reactions.push(click(1, Trigger::Click, Action::Back));
    doc.apply(Operation::create_node(b)).unwrap();

    let mut a = frame(0.0);
    let a_id = a.id;
    a.reactions
        .push(click(2, Trigger::Click, Action::Navigate { to: b_id }));
    doc.apply(Operation::create_node(a)).unwrap();

    let scene_len_before = doc.scene.len();

    let mut session = PresentSession::new(&doc, Some(a_id), DVec2::new(200.0, 200.0)).unwrap();
    assert_eq!(session.current_frame(), a_id, "starts on A");

    // A click at the surface centre lands inside the fitted frame and fires its
    // reaction.
    let r = session.handle_pointer(DVec2::new(100.0, 100.0), PointerEvent::Click);
    assert!(r.navigated && r.needs_redraw, "navigate response: {r:?}");
    assert_eq!(session.current_frame(), b_id, "navigated to B");

    // Clicking B goes back to A.
    let r = session.handle_pointer(DVec2::new(100.0, 100.0), PointerEvent::Click);
    assert!(r.navigated, "back response: {r:?}");
    assert_eq!(session.current_frame(), a_id, "back to A");

    // Back with an empty stack is a no-op (A has no Back reaction, but drive it
    // directly by clicking A which navigates forward again first).
    let r = session.handle_pointer(DVec2::new(100.0, 100.0), PointerEvent::Click);
    assert_eq!(session.current_frame(), b_id);
    let _ = r;

    // The doc is borrowed immutably, so playback cannot have mutated it.
    assert_eq!(doc.scene.len(), scene_len_before, "doc scene unchanged");
}

#[test]
fn open_overlay_stacks_then_close_returns_to_base() {
    // Base A (200²) opens overlay O (100², green) centered; O closes on click.
    let mut doc = Doc::new();
    let o = {
        let mut o = frame_sized(1000.0, 0.0, 100.0, 100.0, Color::rgb(0, 255, 0));
        o.reactions.push(click(1, Trigger::Click, Action::Close));
        o
    };
    let o_id = o.id;
    doc.apply(Operation::create_node(o)).unwrap();

    let mut a = frame_sized(0.0, 0.0, 200.0, 200.0, Color::WHITE);
    let a_id = a.id;
    a.reactions.push(click(
        2,
        Trigger::Click,
        Action::OpenOverlay {
            frame: o_id,
            overlay: OverlaySettings {
                position: OverlayPosition::Center,
                background_dim: true,
                close_on_click_outside: false,
            },
        },
    ));
    doc.apply(Operation::create_node(a)).unwrap();

    let screen = DVec2::new(200.0, 200.0);
    let mut session = PresentSession::new(&doc, Some(a_id), screen).unwrap();

    // Click the base: the overlay opens (base frame unchanged).
    let r = session.handle_pointer(DVec2::new(100.0, 100.0), PointerEvent::Click);
    assert!(r.navigated && r.needs_redraw, "overlay opened: {r:?}");
    assert_eq!(
        session.current_frame(),
        a_id,
        "base frame stays A under the overlay"
    );

    // The overlay (green) covers the centre; the dimmed backdrop shows at a
    // corner (darkened white, never pure white or black).
    let pixels = session.present_rgba();
    let center = center_pixel(&pixels, 200, 200);
    assert!(
        center[1] > 200 && center[0] < 60 && center[2] < 60,
        "overlay centre is green, got {center:?}"
    );
    let corner = pixel_at(&pixels, 200, 5, 5);
    assert!(
        corner[0] > 100 && corner[0] < 220 && corner[0] == corner[1] && corner[1] == corner[2],
        "backdrop corner is dimmed grey, got {corner:?}"
    );

    // Clicking inside the overlay fires its Close, popping back to the bare base.
    let r = session.handle_pointer(DVec2::new(100.0, 100.0), PointerEvent::Click);
    assert!(r.navigated, "overlay closed: {r:?}");
    let center = center_pixel(&session.present_rgba(), 200, 200);
    assert!(
        center[0] > 240 && center[1] > 240,
        "base A (white) shows again, got {center:?}"
    );
}

#[test]
fn a_click_outside_a_dismissable_overlay_closes_it() {
    let mut doc = Doc::new();
    let o = frame_sized(1000.0, 0.0, 100.0, 100.0, Color::rgb(0, 255, 0));
    let o_id = o.id;
    doc.apply(Operation::create_node(o)).unwrap();

    let mut a = frame_sized(0.0, 0.0, 200.0, 200.0, Color::WHITE);
    let a_id = a.id;
    a.reactions.push(click(
        1,
        Trigger::Click,
        Action::OpenOverlay {
            frame: o_id,
            overlay: OverlaySettings {
                position: OverlayPosition::Center,
                background_dim: false,
                close_on_click_outside: true,
            },
        },
    ));
    doc.apply(Operation::create_node(a)).unwrap();

    let screen = DVec2::new(200.0, 200.0);
    let mut session = PresentSession::new(&doc, Some(a_id), screen).unwrap();
    session.handle_pointer(DVec2::new(100.0, 100.0), PointerEvent::Click);

    // The overlay occupies [50,50]..[150,150]; a corner click is outside it.
    let r = session.handle_pointer(DVec2::new(5.0, 5.0), PointerEvent::Click);
    assert!(r.navigated, "click outside dismisses: {r:?}");
    // Now the base shows at the centre again (overlay gone).
    let center = center_pixel(&session.present_rgba(), 200, 200);
    assert!(
        center[0] > 240 && center[1] > 240,
        "overlay dismissed, got {center:?}"
    );
}

#[test]
fn overlay_transitions_sample_real_intermediate_frames() {
    let styles = [
        TransitionStyle::Dissolve,
        TransitionStyle::SmartAnimate,
        TransitionStyle::SlideIn {
            direction: Direction::Left,
        },
        TransitionStyle::MoveIn {
            direction: Direction::Right,
        },
        TransitionStyle::Push {
            direction: Direction::Up,
        },
    ];

    for (index, style) in styles.into_iter().enumerate() {
        let mut doc = Doc::new();
        let overlay = frame_sized(500.0, 0.0, 100.0, 100.0, Color::rgb(0, 255, 0));
        let overlay_id = overlay.id;
        doc.apply(Operation::create_node(overlay)).unwrap();

        let mut base = frame_sized(0.0, 0.0, 200.0, 200.0, Color::WHITE);
        let base_id = base.id;
        base.reactions.push(click_with(
            40 + index as u128,
            Action::OpenOverlay {
                frame: overlay_id,
                overlay: OverlaySettings {
                    position: OverlayPosition::Center,
                    background_dim: true,
                    close_on_click_outside: false,
                },
            },
            Transition {
                style,
                duration_ms: 100,
                easing: Easing::Linear,
            },
        ));
        doc.apply(Operation::create_node(base)).unwrap();

        let mut session = PresentSession::new(&doc, Some(base_id), DVec2::splat(200.0)).unwrap();
        let before = session.present_rgba();
        let response = session.handle_pointer(DVec2::splat(100.0), PointerEvent::Click);
        assert!(
            response.navigated && response.animating,
            "{style:?} should start an overlay animation: {response:?}"
        );
        assert_eq!(
            session.present_rgba(),
            before,
            "{style:?} starts without a visual cut"
        );

        let response = session.tick(0.05);
        assert!(response.animating, "{style:?} remains active at midpoint");
        let midpoint = session.present_rgba();
        assert_ne!(midpoint, before, "{style:?} produces a midpoint frame");

        let response = session.tick(0.05);
        assert!(!response.animating, "{style:?} completes on duration");
        let completed = session.present_rgba();
        assert_ne!(
            midpoint, completed,
            "{style:?} midpoint is not the endpoint"
        );
        let center = center_pixel(&completed, 200, 200);
        assert!(
            center[1] > 200 && center[0] < 60 && center[2] < 60,
            "{style:?} settles on the green overlay: {center:?}"
        );
    }
}

#[test]
fn close_overlay_transitions_reverse_without_a_visual_cut() {
    let styles = [
        TransitionStyle::Dissolve,
        TransitionStyle::SmartAnimate,
        TransitionStyle::SlideIn {
            direction: Direction::Left,
        },
        TransitionStyle::MoveIn {
            direction: Direction::Right,
        },
        TransitionStyle::Push {
            direction: Direction::Down,
        },
    ];

    for (index, style) in styles.into_iter().enumerate() {
        let mut doc = Doc::new();
        let mut overlay = frame_sized(500.0, 0.0, 100.0, 100.0, Color::rgb(0, 255, 0));
        let overlay_id = overlay.id;
        overlay.reactions.push(click_with(
            60 + index as u128,
            Action::Close,
            Transition {
                style,
                duration_ms: 100,
                easing: Easing::Linear,
            },
        ));
        doc.apply(Operation::create_node(overlay)).unwrap();

        let mut base = frame_sized(0.0, 0.0, 200.0, 200.0, Color::WHITE);
        let base_id = base.id;
        base.reactions.push(click(
            70 + index as u128,
            Trigger::Click,
            Action::OpenOverlay {
                frame: overlay_id,
                overlay: OverlaySettings {
                    position: OverlayPosition::Center,
                    background_dim: true,
                    close_on_click_outside: false,
                },
            },
        ));
        doc.apply(Operation::create_node(base)).unwrap();

        let mut session = PresentSession::new(&doc, Some(base_id), DVec2::splat(200.0)).unwrap();
        let base_pixels = session.present_rgba();
        session.handle_pointer(DVec2::splat(100.0), PointerEvent::Click);
        let open_pixels = session.present_rgba();
        let response = session.handle_pointer(DVec2::splat(100.0), PointerEvent::Click);
        assert!(
            response.navigated && response.animating,
            "{style:?} close should animate: {response:?}"
        );
        assert_eq!(
            session.present_rgba(),
            open_pixels,
            "{style:?} close begins from the fully open overlay"
        );
        session.tick(0.05);
        let midpoint = session.present_rgba();
        assert_ne!(midpoint, open_pixels, "{style:?} leaves the overlay state");
        assert_ne!(midpoint, base_pixels, "{style:?} has a midpoint");
        let response = session.tick(0.05);
        assert!(!response.animating);
        assert_eq!(
            session.present_rgba(),
            base_pixels,
            "{style:?} settles on the revealed base"
        );
    }
}

#[test]
fn interrupting_an_overlay_entry_closes_from_its_displayed_progress() {
    for (index, style) in [
        TransitionStyle::Dissolve,
        TransitionStyle::MoveIn {
            direction: Direction::Left,
        },
    ]
    .into_iter()
    .enumerate()
    {
        let mut doc = Doc::new();
        let mut overlay = frame_sized(500.0, 0.0, 100.0, 100.0, Color::rgb(0, 255, 0));
        let overlay_id = overlay.id;
        overlay.reactions.push(Reaction {
            id: ReactionId::from_u128(900 + index as u128),
            trigger: Trigger::Key {
                keys: vec!["Escape".into()],
            },
            action: Action::Close,
            extra_actions: Vec::new(),
            transition: Some(Transition {
                style,
                duration_ms: 100,
                easing: Easing::Linear,
            }),
            animation: None,
        });
        doc.apply(Operation::create_node(overlay)).unwrap();

        let mut base = frame_sized(0.0, 0.0, 200.0, 200.0, Color::WHITE);
        let base_id = base.id;
        base.reactions.push(Reaction {
            id: ReactionId::from_u128(910 + index as u128),
            trigger: Trigger::Key {
                keys: vec!["Enter".into()],
            },
            action: Action::OpenOverlay {
                frame: overlay_id,
                overlay: OverlaySettings {
                    position: OverlayPosition::Center,
                    background_dim: true,
                    close_on_click_outside: false,
                },
            },
            extra_actions: Vec::new(),
            transition: Some(Transition {
                style,
                duration_ms: 100,
                easing: Easing::Linear,
            }),
            animation: None,
        });
        doc.apply(Operation::create_node(base)).unwrap();

        let mut session = PresentSession::new(&doc, Some(base_id), DVec2::splat(200.0)).unwrap();
        assert!(session.handle_key("Enter", KeyEvent::Down).animating);
        session.tick(0.04);
        let interrupted = session.present_rgba();
        let response = session.handle_key("Escape", KeyEvent::Down);
        assert!(response.navigated && response.animating);
        assert_eq!(
            session.present_rgba(),
            interrupted,
            "{style:?} close must start at the entry's current visual state"
        );
        session.tick(0.02);
        assert_ne!(session.present_rgba(), interrupted);
        session.tick(0.08);
        let settled = session.present_rgba();
        assert!(center_pixel(&settled, 200, 200)[0] > 240);
    }
}

#[test]
fn directional_navigation_hit_testing_tracks_the_incoming_geometry() {
    let mut doc = Doc::new();
    let destination = frame_sized(500.0, 0.0, 200.0, 200.0, Color::WHITE);
    let destination_id = destination.id;
    doc.apply(Operation::create_node(destination)).unwrap();
    let mut button = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        80.0,
        80.0,
        40.0,
        40.0,
        Color::rgb(0, 0, 255),
    )));
    button.parent = Some(destination_id);
    button.reactions.push(click(
        920,
        Trigger::Click,
        Action::OpenLink {
            url: "https://example.com/incoming-frame".into(),
        },
    ));
    doc.apply(Operation::create_node(button)).unwrap();

    let mut source = frame_sized(0.0, 0.0, 200.0, 200.0, Color::WHITE);
    let source_id = source.id;
    source.reactions.push(Reaction {
        id: ReactionId::from_u128(921),
        trigger: Trigger::Key {
            keys: vec!["Enter".into()],
        },
        action: Action::Navigate { to: destination_id },
        extra_actions: Vec::new(),
        transition: Some(Transition {
            style: TransitionStyle::MoveIn {
                direction: Direction::Left,
            },
            duration_ms: 100,
            easing: Easing::Linear,
        }),
        animation: None,
    });
    doc.apply(Operation::create_node(source)).unwrap();

    let mut session = PresentSession::new(&doc, Some(source_id), DVec2::splat(200.0)).unwrap();
    session.handle_key("Enter", KeyEvent::Down);
    session.handle_pointer(DVec2::splat(100.0), PointerEvent::Click);
    assert!(
        session.take_open_url().is_none(),
        "the offscreen destination is not clickable at its final bounds"
    );

    session.tick(0.05);
    session.handle_pointer(DVec2::new(10.0, 100.0), PointerEvent::Click);
    assert_eq!(
        session.take_open_url().as_deref(),
        Some("https://example.com/incoming-frame"),
        "the half-entered button is clickable at its displayed position"
    );
    session.handle_pointer(DVec2::splat(100.0), PointerEvent::Click);
    assert!(
        session.take_open_url().is_none(),
        "the button's eventual position remains empty at the midpoint"
    );
}

#[test]
fn directional_overlay_hit_testing_tracks_the_incoming_geometry() {
    let mut doc = Doc::new();
    let overlay = frame_sized(500.0, 0.0, 100.0, 100.0, Color::WHITE);
    let overlay_id = overlay.id;
    doc.apply(Operation::create_node(overlay)).unwrap();
    let mut button = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        30.0,
        30.0,
        40.0,
        40.0,
        Color::rgb(0, 255, 0),
    )));
    button.parent = Some(overlay_id);
    button.reactions.push(click(
        930,
        Trigger::Click,
        Action::OpenLink {
            url: "https://example.com/incoming-overlay".into(),
        },
    ));
    doc.apply(Operation::create_node(button)).unwrap();

    let mut base = frame_sized(0.0, 0.0, 200.0, 200.0, Color::WHITE);
    let base_id = base.id;
    base.reactions.push(Reaction {
        id: ReactionId::from_u128(931),
        trigger: Trigger::Key {
            keys: vec!["Enter".into()],
        },
        action: Action::OpenOverlay {
            frame: overlay_id,
            overlay: OverlaySettings {
                position: OverlayPosition::Center,
                background_dim: false,
                close_on_click_outside: false,
            },
        },
        extra_actions: Vec::new(),
        transition: Some(Transition {
            style: TransitionStyle::MoveIn {
                direction: Direction::Left,
            },
            duration_ms: 100,
            easing: Easing::Linear,
        }),
        animation: None,
    });
    doc.apply(Operation::create_node(base)).unwrap();

    let mut session = PresentSession::new(&doc, Some(base_id), DVec2::splat(200.0)).unwrap();
    session.handle_key("Enter", KeyEvent::Down);
    session.handle_pointer(DVec2::splat(100.0), PointerEvent::Click);
    assert!(session.take_open_url().is_none());
    session.tick(0.05);
    session.handle_pointer(DVec2::new(10.0, 100.0), PointerEvent::Click);
    assert_eq!(
        session.take_open_url().as_deref(),
        Some("https://example.com/incoming-overlay")
    );
}

/// The RGBA of pixel `(x, y)` in a `width`-wide straight-alpha buffer.
fn pixel_at(buf: &[u8], width: usize, x: usize, y: usize) -> [u8; 4] {
    let idx = (y * width + x) * 4;
    [buf[idx], buf[idx + 1], buf[idx + 2], buf[idx + 3]]
}

#[test]
fn close_at_base_level_exits() {
    let mut doc = Doc::new();
    let mut a = frame(0.0);
    let a_id = a.id;
    a.reactions.push(click(1, Trigger::Click, Action::Close));
    doc.apply(Operation::create_node(a)).unwrap();

    let mut session = PresentSession::new(&doc, Some(a_id), DVec2::new(200.0, 200.0)).unwrap();
    let r = session.handle_pointer(DVec2::new(100.0, 100.0), PointerEvent::Click);
    assert!(r.exited, "Close with no overlay exits present mode: {r:?}");
}

#[test]
fn set_variable_rebinds_a_bound_fill_without_touching_the_doc() {
    // A one-mode collection with a Color variable `bg` = white; a frame whose
    // background fill is bound to `bg`; a click that sets `bg` to red.
    let coll_id = VariableCollectionId::from_u128(1);
    let mode = ModeId::from_u128(10);
    let var_id = VariableId::from_u128(100);

    let mut doc = Doc::new();
    doc.variables.collections.insert(
        coll_id,
        VariableCollection {
            id: coll_id,
            name: "Theme".into(),
            modes: vec![Mode {
                id: mode,
                name: "M".into(),
            }],
            default_mode: mode,
            variable_order: vec![var_id],
        },
    );
    doc.variables.variables.insert(
        var_id,
        Variable {
            id: var_id,
            collection: coll_id,
            name: "bg".into(),
            ty: VariableType::Color,
            values_by_mode: std::collections::BTreeMap::from([(
                mode,
                VarValue::Color {
                    value: Color::WHITE,
                },
            )]),
            scopes: Vec::new(),
        },
    );

    let mut a = frame_colored(0.0, Color::WHITE);
    let a_id = a.id;
    a.bindings.insert(BoundProp::FillColor { index: 0 }, var_id);
    a.reactions.push(click(
        1,
        Trigger::Click,
        Action::SetVariable {
            variable: var_id,
            value: VarValue::Color {
                value: Color::rgb(255, 0, 0),
            },
        },
    ));
    doc.apply(Operation::create_node(a)).unwrap();

    let screen = DVec2::new(64.0, 64.0);
    let mut session = PresentSession::new(&doc, Some(a_id), screen).unwrap();
    let before = session.present_rgba();
    // Centre pixel is inside the fitted frame; it renders the bound white.
    let center = center_pixel(&before, 64, 64);
    assert!(
        center[0] > 200 && center[1] > 200 && center[2] > 200,
        "bound fill starts white, got {center:?}"
    );

    let r = session.handle_pointer(DVec2::new(32.0, 32.0), PointerEvent::Click);
    assert!(
        r.needs_redraw && !r.navigated,
        "set-variable redraws in place: {r:?}"
    );

    let after = session.present_rgba();
    let center = center_pixel(&after, 64, 64);
    assert!(
        center[0] > 200 && center[1] < 60 && center[2] < 60,
        "after set-variable the bound fill is red, got {center:?}"
    );

    // The document's own variable is untouched — playback is ephemeral.
    assert_eq!(
        doc.variables.variable(var_id).unwrap().value_for_mode(mode),
        Some(&VarValue::Color {
            value: Color::WHITE
        }),
        "SetVariable must not mutate the doc"
    );
}

#[test]
fn set_variable_uses_the_active_layers_innermost_mode_override() {
    let collection_id = VariableCollectionId::from_u128(201);
    let light_mode = ModeId::from_u128(202);
    let dark_mode = ModeId::from_u128(203);
    let variable_id = VariableId::from_u128(204);
    let mut doc = Doc::new();
    doc.variables.collections.insert(
        collection_id,
        VariableCollection {
            id: collection_id,
            name: "Theme".into(),
            modes: vec![
                Mode {
                    id: light_mode,
                    name: "Light".into(),
                },
                Mode {
                    id: dark_mode,
                    name: "Dark".into(),
                },
            ],
            default_mode: light_mode,
            variable_order: vec![variable_id],
        },
    );
    doc.variables.variables.insert(
        variable_id,
        Variable {
            id: variable_id,
            collection: collection_id,
            name: "Count".into(),
            ty: VariableType::Float,
            values_by_mode: BTreeMap::from([
                (light_mode, VarValue::Float { value: 1.0 }),
                (dark_mode, VarValue::Float { value: 2.0 }),
            ]),
            scopes: Vec::new(),
        },
    );
    let frame = frame(0.0);
    let frame_id = frame.id;
    doc.apply(Operation::create_node(frame)).unwrap();
    let mut inner = CanvasNode::new(NodeData::Group(GroupNode {
        clip_size: Some([100.0, 100.0]),
        ..GroupNode::default()
    }));
    inner.parent = Some(frame_id);
    inner.transform = Transform2D::translation(50.0, 50.0);
    let NodeData::Group(group) = &mut inner.data else {
        panic!("expected inner group");
    };
    group.explicit_modes.insert(collection_id, dark_mode);
    inner.reactions.push(click(
        205,
        Trigger::Click,
        Action::SetVariable {
            variable: variable_id,
            value: VarValue::Float { value: 3.0 },
        },
    ));
    doc.apply(Operation::create_node(inner)).unwrap();

    let mut session = PresentSession::new(&doc, Some(frame_id), DVec2::new(200.0, 200.0)).unwrap();
    session.handle_pointer(DVec2::new(100.0, 100.0), PointerEvent::Click);
    let variable = session.variables.variable(variable_id).unwrap();
    assert_eq!(
        variable.value_for_mode(light_mode),
        Some(&VarValue::Float { value: 1.0 })
    );
    assert_eq!(
        variable.value_for_mode(dark_mode),
        Some(&VarValue::Float { value: 3.0 })
    );
}

/// The RGBA of the centre pixel of a `width × height` straight-alpha buffer.
fn center_pixel(buf: &[u8], width: usize, height: usize) -> [u8; 4] {
    let (x, y) = (width / 2, height / 2);
    let idx = (y * width + x) * 4;
    [buf[idx], buf[idx + 1], buf[idx + 2], buf[idx + 3]]
}

#[test]
fn an_after_delay_navigates_once_its_timer_elapses() {
    // A auto-navigates to B after 200ms.
    let mut doc = Doc::new();
    let b = frame(500.0);
    let b_id = b.id;
    doc.apply(Operation::create_node(b)).unwrap();

    let mut a = frame(0.0);
    let a_id = a.id;
    a.reactions.push(click(
        1,
        Trigger::AfterDelay { delay_ms: 200 },
        Action::Navigate { to: b_id },
    ));
    doc.apply(Operation::create_node(a)).unwrap();

    let mut session = PresentSession::new(&doc, Some(a_id), DVec2::new(200.0, 200.0)).unwrap();
    assert_eq!(session.current_frame(), a_id);

    // Before the delay: nothing fires.
    let r = session.tick(0.1);
    assert!(!r.navigated, "not yet: {r:?}");
    assert_eq!(session.current_frame(), a_id);

    // Crossing the delay fires the navigate exactly once.
    let r = session.tick(0.15);
    assert!(r.navigated, "after-delay fired: {r:?}");
    assert_eq!(session.current_frame(), b_id);

    // The timer is consumed — further ticks are inert on B (no reactions).
    let r = session.tick(1.0);
    assert!(!r.navigated, "no repeat fire: {r:?}");
    assert_eq!(session.current_frame(), b_id);
}

#[test]
fn leaving_a_frame_cancels_its_pending_after_delay() {
    // A has a slow AfterDelay to C, but a click navigates to B first; the stale
    // timer must not later yank the session to C.
    let mut doc = Doc::new();
    let c = frame(1000.0);
    let c_id = c.id;
    doc.apply(Operation::create_node(c)).unwrap();
    let b = frame(500.0);
    let b_id = b.id;
    doc.apply(Operation::create_node(b)).unwrap();

    let mut a = frame(0.0);
    let a_id = a.id;
    a.reactions.push(click(
        1,
        Trigger::AfterDelay { delay_ms: 500 },
        Action::Navigate { to: c_id },
    ));
    a.reactions
        .push(click(2, Trigger::Click, Action::Navigate { to: b_id }));
    doc.apply(Operation::create_node(a)).unwrap();

    let mut session = PresentSession::new(&doc, Some(a_id), DVec2::new(200.0, 200.0)).unwrap();
    session.handle_pointer(DVec2::new(100.0, 100.0), PointerEvent::Click);
    assert_eq!(session.current_frame(), b_id, "click won the race");

    // Long after A's timer would have fired: we stay on B, not jump to C.
    let r = session.tick(2.0);
    assert!(!r.navigated, "cancelled timer stays quiet: {r:?}");
    assert_eq!(session.current_frame(), b_id);
}

#[test]
fn navigation_skips_other_due_timers_owned_by_the_departed_frame() {
    let mut doc = Doc::new();
    let b = frame(500.0);
    let b_id = b.id;
    doc.apply(Operation::create_node(b)).unwrap();
    let c = frame(1000.0);
    let c_id = c.id;
    doc.apply(Operation::create_node(c)).unwrap();

    let mut a = frame(0.0);
    let a_id = a.id;
    a.reactions.push(click(
        1,
        Trigger::AfterDelay { delay_ms: 100 },
        Action::Navigate { to: b_id },
    ));
    a.reactions.push(click(
        2,
        Trigger::AfterDelay { delay_ms: 100 },
        Action::Navigate { to: c_id },
    ));
    doc.apply(Operation::create_node(a)).unwrap();

    let mut session = PresentSession::new(&doc, Some(a_id), DVec2::new(200.0, 200.0)).unwrap();
    let response = session.tick(0.1);

    assert!(response.navigated);
    assert_eq!(
        session.current_frame(),
        b_id,
        "the second timer was already due, but its owning frame left after the first navigation"
    );
}

#[test]
fn after_delay_overshoot_advances_the_started_transition_and_clip() {
    let mut doc = Doc::new();
    let destination = frame_colored(500.0, Color::WHITE);
    let destination_id = destination.id;
    doc.apply(Operation::create_node(destination)).unwrap();
    let mut moving = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        0.0,
        0.0,
        20.0,
        20.0,
        Color::rgb(0, 0, 255),
    )));
    let moving_id = moving.id;
    moving.parent = Some(destination_id);
    moving.transform = Transform2D::translation(10.0, 80.0);
    doc.apply(Operation::create_node(moving)).unwrap();
    let clip_id = AnimationClipId::from_u128(940);
    doc.motion
        .clips
        .insert(clip_id, position_clip(clip_id, moving_id, 10.0, 150.0));

    let mut source = frame_colored(0.0, Color::rgb(255, 0, 0));
    let source_id = source.id;
    source.reactions.push(Reaction {
        id: ReactionId::from_u128(941),
        trigger: Trigger::AfterDelay { delay_ms: 40 },
        action: Action::Navigate { to: destination_id },
        extra_actions: Vec::new(),
        transition: Some(Transition {
            style: TransitionStyle::Dissolve,
            duration_ms: 100,
            easing: Easing::Linear,
        }),
        animation: Some(PrototypeAnimation {
            clip: clip_id,
            delay_ms: 0,
        }),
    });
    doc.apply(Operation::create_node(source)).unwrap();

    let mut session = PresentSession::new(&doc, Some(source_id), DVec2::splat(200.0)).unwrap();
    let response = session.tick(0.1);
    assert!(response.navigated && response.animating);
    assert_eq!(session.current_frame(), destination_id);
    let transition = session
        .active_transition
        .expect("transition remains active");
    assert!((transition.elapsed - 0.06).abs() < 1e-9);
    let animation = session
        .active_prototype_animation
        .expect("clip remains active");
    assert!((animation.elapsed - 0.06).abs() < 1e-9);
    assert!(
        pixel_at(&session.present_rgba(), 200, 104, 90)[2] > 100,
        "the destination clip receives the 60ms timer remainder"
    );
}

#[test]
fn after_delay_timers_scheduled_by_a_due_action_use_the_same_tick_remainder() {
    let mut doc = Doc::new();
    let mut destination = frame(500.0);
    let destination_id = destination.id;
    destination.reactions.push(click(
        952,
        Trigger::AfterDelay { delay_ms: 30 },
        Action::OpenLink {
            url: "https://example.com/destination-timer".into(),
        },
    ));
    doc.apply(Operation::create_node(destination)).unwrap();

    let mut source = frame(0.0);
    let source_id = source.id;
    source.reactions.push(click(
        951,
        Trigger::AfterDelay { delay_ms: 20 },
        Action::Navigate { to: destination_id },
    ));
    doc.apply(Operation::create_node(source)).unwrap();

    let mut session = PresentSession::new(&doc, Some(source_id), DVec2::splat(200.0)).unwrap();
    session.tick(0.1);
    assert_eq!(session.current_frame(), destination_id);
    assert_eq!(
        session.take_open_url().as_deref(),
        Some("https://example.com/destination-timer")
    );
}

#[test]
fn simultaneous_after_delay_timers_fire_in_stable_reaction_id_order() {
    let mut doc = Doc::new();
    let mut root = frame(0.0);
    let root_id = root.id;
    root.reactions.extend([
        click(
            962,
            Trigger::AfterDelay { delay_ms: 10 },
            Action::OpenLink {
                url: "https://example.com/second".into(),
            },
        ),
        click(
            961,
            Trigger::AfterDelay { delay_ms: 10 },
            Action::OpenLink {
                url: "https://example.com/first".into(),
            },
        ),
    ]);
    doc.apply(Operation::create_node(root)).unwrap();

    let mut session = PresentSession::new(&doc, Some(root_id), DVec2::splat(200.0)).unwrap();
    session.tick(0.01);
    assert_eq!(
        session.take_open_url().as_deref(),
        Some("https://example.com/second"),
        "reaction 961 fires first and reaction 962 deterministically overwrites it"
    );
}

#[test]
fn missing_start_frame_is_an_error() {
    let doc = Doc::new(); // no pages, no flow start
    assert!(matches!(
        PresentSession::new(&doc, None, DVec2::new(100.0, 100.0)),
        Err(PresentError::NoStartFrame)
    ));
}

#[test]
fn a_dissolve_transition_animates_then_settles_on_the_destination() {
    // A (red) → Navigate(B, blue) over a 100ms dissolve.
    let mut doc = Doc::new();
    let b = frame_colored(500.0, Color::rgb(0, 0, 255));
    let b_id = b.id;
    doc.apply(Operation::create_node(b)).unwrap();

    let mut a = frame_colored(0.0, Color::rgb(255, 0, 0));
    let a_id = a.id;
    a.reactions.push(click_with(
        1,
        Action::Navigate { to: b_id },
        Transition {
            style: TransitionStyle::Dissolve,
            duration_ms: 100,
            easing: Easing::Linear,
        },
    ));
    doc.apply(Operation::create_node(a)).unwrap();

    let screen = DVec2::new(200.0, 200.0);
    let mut session = PresentSession::new(&doc, Some(a_id), screen).unwrap();

    // A reference frame: what B alone renders to, fit to the same surface.
    let reference_b = PresentSession::new(&doc, Some(b_id), screen)
        .unwrap()
        .present_rgba();
    let reference_a = session.present_rgba();
    assert_ne!(
        reference_a, reference_b,
        "the two frames must render differently"
    );

    // Clicking A starts the transition; the session's current frame is already B
    // but the composite still shows mostly A at t≈0.
    let r = session.handle_pointer(DVec2::new(100.0, 100.0), PointerEvent::Click);
    assert!(
        r.navigated && r.animating,
        "transition should be animating: {r:?}"
    );
    assert_eq!(session.current_frame(), b_id);
    assert_eq!(
        session.present_rgba(),
        reference_a,
        "start of dissolve == frame A"
    );

    // Halfway: still animating, and a genuine blend (neither endpoint).
    let r = session.tick(0.05);
    assert!(r.animating, "still animating at 50ms: {r:?}");
    let mid = session.present_rgba();
    assert_ne!(mid, reference_a);
    assert_ne!(mid, reference_b);

    // The final tick completes it; the composite is gone and B renders plainly.
    let r = session.tick(0.05);
    assert!(!r.animating && r.needs_redraw, "transition done: {r:?}");
    assert_eq!(
        session.present_rgba(),
        reference_b,
        "end of dissolve == frame B"
    );
}

#[test]
fn smart_animate_without_matched_layers_falls_back_to_a_visible_dissolve() {
    let mut doc = Doc::new();
    let destination = frame_colored(500.0, Color::rgb(0, 0, 255));
    let destination_id = destination.id;
    doc.apply(Operation::create_node(destination)).unwrap();
    let mut source = frame_colored(0.0, Color::rgb(255, 0, 0));
    let source_id = source.id;
    source.reactions.push(click_with(
        25,
        Action::Navigate { to: destination_id },
        Transition {
            style: TransitionStyle::SmartAnimate,
            duration_ms: 100,
            easing: Easing::Linear,
        },
    ));
    doc.apply(Operation::create_node(source)).unwrap();

    let mut session = PresentSession::new(&doc, Some(source_id), DVec2::splat(200.0)).unwrap();
    let source_pixels = session.present_rgba();
    let response = session.handle_pointer(DVec2::splat(100.0), PointerEvent::Click);
    assert!(response.animating);
    assert_eq!(session.present_rgba(), source_pixels);

    session.tick(0.05);
    let midpoint = center_pixel(&session.present_rgba(), 200, 200);
    assert!(
        midpoint[0] > 80 && midpoint[2] > 80,
        "unmatched Smart Animate produces a crossfade midpoint: {midpoint:?}"
    );
    session.tick(0.05);
    let completed = center_pixel(&session.present_rgba(), 200, 200);
    assert!(completed[2] > 200 && completed[0] < 60);
}

#[test]
fn directional_navigation_styles_produce_time_sampled_frames() {
    let styles = [
        TransitionStyle::SlideIn {
            direction: Direction::Left,
        },
        TransitionStyle::MoveIn {
            direction: Direction::Right,
        },
        TransitionStyle::Push {
            direction: Direction::Down,
        },
    ];
    for (index, style) in styles.into_iter().enumerate() {
        let mut doc = Doc::new();
        let destination = frame_colored(500.0, Color::rgb(0, 0, 255));
        let destination_id = destination.id;
        doc.apply(Operation::create_node(destination)).unwrap();
        let mut source = frame_colored(0.0, Color::rgb(255, 0, 0));
        let source_id = source.id;
        source.reactions.push(click_with(
            30 + index as u128,
            Action::Navigate { to: destination_id },
            Transition {
                style,
                duration_ms: 100,
                easing: Easing::Linear,
            },
        ));
        doc.apply(Operation::create_node(source)).unwrap();

        let mut session = PresentSession::new(&doc, Some(source_id), DVec2::splat(200.0)).unwrap();
        let source_pixels = session.present_rgba();
        let destination_pixels =
            PresentSession::new(&doc, Some(destination_id), DVec2::splat(200.0))
                .unwrap()
                .present_rgba();
        let response = session.handle_pointer(DVec2::splat(100.0), PointerEvent::Click);
        assert!(response.navigated && response.animating, "{style:?}");
        assert_eq!(session.present_rgba(), source_pixels, "{style:?} at t=0");
        session.tick(0.05);
        let midpoint = session.present_rgba();
        assert_ne!(midpoint, source_pixels, "{style:?} leaves its source");
        assert_ne!(
            midpoint, destination_pixels,
            "{style:?} does not jump to its destination"
        );
        let response = session.tick(0.05);
        assert!(!response.animating);
        assert_eq!(
            session.present_rgba(),
            destination_pixels,
            "{style:?} settles exactly"
        );
    }
}

#[test]
fn back_action_uses_its_authored_transition() {
    let mut doc = Doc::new();
    let mut destination = frame_colored(500.0, Color::rgb(0, 0, 255));
    let destination_id = destination.id;
    destination.reactions.push(click_with(
        35,
        Action::Back,
        Transition {
            style: TransitionStyle::Dissolve,
            duration_ms: 100,
            easing: Easing::Linear,
        },
    ));
    doc.apply(Operation::create_node(destination)).unwrap();
    let mut source = frame_colored(0.0, Color::rgb(255, 0, 0));
    let source_id = source.id;
    source.reactions.push(click(
        36,
        Trigger::Click,
        Action::Navigate { to: destination_id },
    ));
    doc.apply(Operation::create_node(source)).unwrap();

    let mut session = PresentSession::new(&doc, Some(source_id), DVec2::splat(200.0)).unwrap();
    let source_pixels = session.present_rgba();
    session.handle_pointer(DVec2::splat(100.0), PointerEvent::Click);
    let destination_pixels = session.present_rgba();
    let response = session.handle_pointer(DVec2::splat(100.0), PointerEvent::Click);
    assert!(response.navigated && response.animating);
    assert_eq!(session.present_rgba(), destination_pixels);
    session.tick(0.05);
    let midpoint = session.present_rgba();
    assert_ne!(midpoint, source_pixels);
    assert_ne!(midpoint, destination_pixels);
    session.tick(0.05);
    assert_eq!(session.present_rgba(), source_pixels);
}

/// Build a frame (200×200, translated to `frame_x` in world) containing one
/// 40×40 red vector named "Box" at frame-local `(box_x, 40)`. Returns the frame.
fn frame_with_box(doc: &mut Doc, frame_x: f64, box_x: f64) -> NodeId {
    let mut frame = CanvasNode::new(NodeData::Group(GroupNode {
        clip_size: Some([200.0, 200.0]),
        ..GroupNode::default()
    }));
    frame.name = "Frame".into();
    frame.transform = Transform2D::translation(frame_x, 0.0);
    let frame_id = frame.id;
    doc.apply(Operation::create_node(frame)).unwrap();

    let mut b = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        0.0,
        0.0,
        40.0,
        40.0,
        Color::rgb(255, 0, 0),
    )));
    b.name = "Box".into();
    b.transform = Transform2D::translation(box_x, 40.0);
    b.parent = Some(frame_id);
    b.index = doc.scene.next_child_index(Some(frame_id));
    doc.apply(Operation::create_node(b)).unwrap();

    frame_id
}

fn add_component_master(doc: &mut Doc, component: ComponentId, color: Color) {
    let mut root = frame_sized(1200.0 + component.to_u128() as f64, 0.0, 40.0, 40.0, color);
    root.name = "Badge".into();
    let root_id = root.id;
    doc.apply(Operation::create_node(root)).unwrap();
    doc.components
        .defs
        .insert(component, ComponentDef::new(component, root_id, "Badge"));
}

fn frame_with_instance(
    doc: &mut Doc,
    frame_x: f64,
    instance_x: f64,
    component: ComponentId,
) -> NodeId {
    let mut frame = CanvasNode::new(NodeData::Group(GroupNode {
        clip_size: Some([200.0, 200.0]),
        background: Some(Fill::solid(Color::WHITE)),
        ..GroupNode::default()
    }));
    frame.name = "Frame".into();
    frame.transform = Transform2D::translation(frame_x, 0.0);
    let frame_id = frame.id;
    doc.apply(Operation::create_node(frame)).unwrap();

    let mut instance = CanvasNode::new(NodeData::Instance(InstanceNode {
        component,
        overrides: Vec::new(),
        prop_values: BTreeMap::new(),
        derived: Vec::new(),
        local_size: [40.0, 40.0],
    }));
    instance.name = "Badge".into();
    instance.transform = Transform2D::translation(instance_x, 40.0);
    instance.parent = Some(frame_id);
    instance.index = doc.scene.next_child_index(Some(frame_id));
    doc.apply(Operation::create_node(instance)).unwrap();

    frame_id
}

#[test]
fn smart_animate_tweens_a_matched_layer_between_frames() {
    // A and B each hold a "Box"; only its position differs. Smart-animate A→B
    // should slide the box, so mid-transition differs from both endpoints while
    // the endpoints match plain renders of A and B.
    let mut doc = Doc::new();
    let a = frame_with_box(&mut doc, 0.0, 10.0); // box at frame-local x=10
    let b = frame_with_box(&mut doc, 500.0, 150.0); // box at frame-local x=150
    // A → Navigate(B) with a 100ms linear smart-animate on click.
    doc.scene.get_mut(a).unwrap().reactions.push(click_with(
        1,
        Action::Navigate { to: b },
        Transition {
            style: TransitionStyle::SmartAnimate,
            duration_ms: 100,
            easing: Easing::Linear,
        },
    ));

    let screen = DVec2::new(200.0, 200.0);
    let reference_a = PresentSession::new(&doc, Some(a), screen)
        .unwrap()
        .present_rgba();
    let reference_b = PresentSession::new(&doc, Some(b), screen)
        .unwrap()
        .present_rgba();
    assert_ne!(reference_a, reference_b, "the two frames differ");

    let mut session = PresentSession::new(&doc, Some(a), screen).unwrap();
    let r = session.handle_pointer(DVec2::new(100.0, 100.0), PointerEvent::Click);
    assert!(r.animating, "smart animate should animate: {r:?}");

    // At the start (progress 0) the matched box sits at A's position.
    assert_eq!(
        session.present_rgba(),
        reference_a,
        "start of smart-animate == frame A"
    );

    // Halfway the box is between the two — neither endpoint.
    session.tick(0.05);
    let mid = session.present_rgba();
    assert_ne!(mid, reference_a, "mid-animation moved off A");
    assert_ne!(mid, reference_b, "mid-animation not yet at B");

    // Completed: the box has arrived; the frame renders like B.
    let r = session.tick(0.05);
    assert!(!r.animating, "smart animate finished: {r:?}");
    assert_eq!(
        session.present_rgba(),
        reference_b,
        "end of smart-animate == frame B"
    );
}

#[test]
fn smart_animate_interpolates_solid_frame_backgrounds_with_matched_layers() {
    let mut doc = Doc::new();
    let source = frame_with_box(&mut doc, 0.0, 10.0);
    let destination = frame_with_box(&mut doc, 500.0, 150.0);
    let Some(NodeData::Group(source_group)) = doc.scene.get_mut(source).map(|node| &mut node.data)
    else {
        panic!("source frame is a group");
    };
    source_group.background = Some(Fill::solid(Color::rgb(255, 0, 0)));
    let Some(NodeData::Group(destination_group)) =
        doc.scene.get_mut(destination).map(|node| &mut node.data)
    else {
        panic!("destination frame is a group");
    };
    destination_group.background = Some(Fill::solid(Color::rgb(0, 0, 255)));
    doc.scene
        .get_mut(source)
        .unwrap()
        .reactions
        .push(click_with(
            31,
            Action::Navigate { to: destination },
            Transition {
                style: TransitionStyle::SmartAnimate,
                duration_ms: 100,
                easing: Easing::Linear,
            },
        ));

    let mut session = PresentSession::new(&doc, Some(source), DVec2::splat(200.0)).unwrap();
    session.handle_pointer(DVec2::splat(100.0), PointerEvent::Click);
    let start = pixel_at(&session.present_rgba(), 200, 190, 190);
    assert!(start[0] > 200 && start[2] < 60);
    session.tick(0.05);
    let midpoint = pixel_at(&session.present_rgba(), 200, 190, 190);
    assert!(
        midpoint[0] > 80 && midpoint[2] > 80,
        "background color has a real midpoint: {midpoint:?}"
    );
    session.tick(0.05);
    let completed = pixel_at(&session.present_rgba(), 200, 190, 190);
    assert!(completed[2] > 200 && completed[0] < 60);
}

#[test]
fn smart_animate_renders_component_instances_from_scratch_scene() {
    let mut doc = Doc::new();
    let component = ComponentId::from_u128(810);
    add_component_master(&mut doc, component, Color::rgb(255, 0, 0));
    let a = frame_with_instance(&mut doc, 0.0, 10.0, component);
    let b = frame_with_instance(&mut doc, 500.0, 150.0, component);
    doc.scene.get_mut(a).unwrap().reactions.push(click_with(
        1,
        Action::Navigate { to: b },
        Transition {
            style: TransitionStyle::SmartAnimate,
            duration_ms: 100,
            easing: Easing::Linear,
        },
    ));

    let screen = DVec2::new(200.0, 200.0);
    let reference_a = PresentSession::new(&doc, Some(a), screen)
        .unwrap()
        .present_rgba();
    let reference_b = PresentSession::new(&doc, Some(b), screen)
        .unwrap()
        .present_rgba();
    assert_ne!(
        reference_a, reference_b,
        "the instance moves between frames"
    );

    let mut session = PresentSession::new(&doc, Some(a), screen).unwrap();
    let r = session.handle_pointer(DVec2::new(100.0, 100.0), PointerEvent::Click);
    assert!(r.animating, "smart animate should animate: {r:?}");
    assert_eq!(
        session.present_rgba(),
        reference_a,
        "smart scratch scene can expand the source component instance"
    );

    session.tick(0.05);
    let mid = session.present_rgba();
    assert_ne!(mid, reference_a, "mid-animation moved off A");
    assert_ne!(mid, reference_b, "mid-animation not yet at B");

    let r = session.tick(0.05);
    assert!(!r.animating, "smart animate finished: {r:?}");
    assert_eq!(
        session.present_rgba(),
        reference_b,
        "smart animate settles on the component-rendered destination"
    );
}

#[test]
fn an_instant_transition_is_a_cut_not_an_animation() {
    let mut doc = Doc::new();
    let b = frame_colored(500.0, Color::rgb(0, 0, 255));
    let b_id = b.id;
    doc.apply(Operation::create_node(b)).unwrap();

    let mut a = frame_colored(0.0, Color::rgb(255, 0, 0));
    let a_id = a.id;
    a.reactions.push(click_with(
        1,
        Action::Navigate { to: b_id },
        Transition {
            style: TransitionStyle::Instant,
            duration_ms: 300,
            easing: Easing::Linear,
        },
    ));
    doc.apply(Operation::create_node(a)).unwrap();

    let screen = DVec2::new(200.0, 200.0);
    let mut session = PresentSession::new(&doc, Some(a_id), screen).unwrap();
    let reference_b = PresentSession::new(&doc, Some(b_id), screen)
        .unwrap()
        .present_rgba();

    let r = session.handle_pointer(DVec2::new(100.0, 100.0), PointerEvent::Click);
    assert!(r.navigated && !r.animating, "instant is a cut: {r:?}");
    // No further ticking needed — B shows immediately.
    assert_eq!(session.present_rgba(), reference_b);
    assert!(!session.tick(0.1).animating, "nothing left to animate");
}

fn position_clip(clip_id: AnimationClipId, node: NodeId, from: f64, to: f64) -> AnimationClip {
    let mut clip = AnimationClip::new(clip_id, "Enter", 100);
    let mut track = AnimationTrack::new(
        AnimationTrackId::from_u128(700),
        MotionTarget::new(node, MotionProperty::PositionX),
    );
    let first = Keyframe::new(
        KeyframeId::from_u128(701),
        0,
        ResolvedVarValue::Float { value: from },
    );
    let last = Keyframe::new(
        KeyframeId::from_u128(702),
        100,
        ResolvedVarValue::Float { value: to },
    );
    track.keyframes.insert(first.id, first);
    track.keyframes.insert(last.id, last);
    clip.tracks.insert(track.id, track);
    clip
}

#[test]
fn reaction_animation_delays_samples_then_holds_its_final_frame() {
    let mut doc = Doc::new();
    let destination = frame_colored(500.0, Color::WHITE);
    let destination_id = destination.id;
    doc.apply(Operation::create_node(destination)).unwrap();

    let mut moving = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        0.0,
        0.0,
        20.0,
        20.0,
        Color::rgb(255, 0, 0),
    )));
    let moving_id = moving.id;
    moving.name = "Moving target".into();
    moving.parent = Some(destination_id);
    moving.transform = Transform2D::translation(10.0, 80.0);
    moving.reactions.push(click(
        704,
        Trigger::Click,
        Action::OpenLink {
            url: "https://example.com/moved".into(),
        },
    ));
    doc.apply(Operation::create_node(moving)).unwrap();

    let clip_id = AnimationClipId::from_u128(703);
    doc.motion
        .clips
        .insert(clip_id, position_clip(clip_id, moving_id, 10.0, 150.0));

    let mut source = frame_colored(0.0, Color::WHITE);
    let source_id = source.id;
    source.reactions.push(Reaction {
        id: ReactionId::from_u128(705),
        trigger: Trigger::Click,
        action: Action::Navigate { to: destination_id },
        extra_actions: Vec::new(),
        transition: None,
        animation: Some(PrototypeAnimation {
            clip: clip_id,
            delay_ms: 50,
        }),
    });
    doc.apply(Operation::create_node(source)).unwrap();

    let mut session = PresentSession::new(&doc, Some(source_id), DVec2::splat(200.0)).unwrap();
    let response = session.handle_pointer(DVec2::splat(100.0), PointerEvent::Click);
    assert!(response.navigated && response.animating);
    assert!(pixel_at(&session.present_rgba(), 200, 20, 90)[0] > 200);

    let before_delay = session.tick(0.049);
    assert!(before_delay.animating && !before_delay.needs_redraw);
    assert!(pixel_at(&session.present_rgba(), 200, 20, 90)[0] > 200);

    let delay_boundary = session.tick(0.001);
    assert!(delay_boundary.animating && delay_boundary.needs_redraw);
    assert!(pixel_at(&session.present_rgba(), 200, 20, 90)[0] > 200);

    session.tick(0.05);
    let midpoint = session.present_rgba();
    assert!(
        pixel_at(&midpoint, 200, 90, 90)[0] > 200,
        "the 50ms sample moves the target to its interpolated x position"
    );
    session.handle_pointer(DVec2::new(20.0, 90.0), PointerEvent::Click);
    assert!(
        session.take_open_url().is_none(),
        "old bounds no longer hit"
    );
    session.handle_pointer(DVec2::new(90.0, 90.0), PointerEvent::Click);
    assert_eq!(
        session.take_open_url().as_deref(),
        Some("https://example.com/moved"),
        "hit testing follows the sampled transform"
    );

    let completed = session.tick(0.05);
    assert!(!completed.animating && completed.needs_redraw);
    let final_pixels = session.present_rgba();
    assert!(pixel_at(&final_pixels, 200, 160, 90)[0] > 200);
    session.tick(1.0);
    assert_eq!(
        session.present_rgba(),
        final_pixels,
        "the final clip sample is held"
    );

    assert!(session.restart_at(destination_id));
    assert!(
        pixel_at(&session.present_rgba(), 200, 20, 90)[0] > 200,
        "restart clears the held prototype animation"
    );
}

#[test]
fn destination_clip_and_frame_transition_advance_on_the_same_clock() {
    let mut doc = Doc::new();
    let destination = frame_colored(500.0, Color::WHITE);
    let destination_id = destination.id;
    doc.apply(Operation::create_node(destination)).unwrap();
    let mut moving = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        0.0,
        0.0,
        20.0,
        20.0,
        Color::rgb(0, 0, 255),
    )));
    let moving_id = moving.id;
    moving.parent = Some(destination_id);
    moving.transform = Transform2D::translation(10.0, 80.0);
    doc.apply(Operation::create_node(moving)).unwrap();
    let clip_id = AnimationClipId::from_u128(710);
    doc.motion
        .clips
        .insert(clip_id, position_clip(clip_id, moving_id, 10.0, 150.0));

    let mut source = frame_colored(0.0, Color::rgb(255, 0, 0));
    let source_id = source.id;
    source.reactions.push(Reaction {
        id: ReactionId::from_u128(711),
        trigger: Trigger::Click,
        action: Action::Navigate { to: destination_id },
        extra_actions: Vec::new(),
        transition: Some(Transition {
            style: TransitionStyle::Dissolve,
            duration_ms: 100,
            easing: Easing::Linear,
        }),
        animation: Some(PrototypeAnimation {
            clip: clip_id,
            delay_ms: 0,
        }),
    });
    doc.apply(Operation::create_node(source)).unwrap();

    let mut session = PresentSession::new(&doc, Some(source_id), DVec2::splat(200.0)).unwrap();
    let source_pixels = session.present_rgba();
    let response = session.handle_pointer(DVec2::splat(100.0), PointerEvent::Click);
    assert!(response.navigated && response.animating && response.needs_redraw);
    assert_eq!(
        session.present_rgba(),
        source_pixels,
        "both effects begin at t=0 without a cut"
    );

    let response = session.tick(0.05);
    assert!(response.animating);
    let midpoint = session.present_rgba();
    let moving_pixel = pixel_at(&midpoint, 200, 90, 90);
    assert!(
        moving_pixel[0] > 80 && moving_pixel[2] > 80,
        "destination motion is sampled before the 50% dissolve: {moving_pixel:?}"
    );

    let response = session.tick(0.05);
    assert!(!response.animating);
    let completed = session.present_rgba();
    let final_pixel = pixel_at(&completed, 200, 160, 90);
    assert!(final_pixel[2] > 200 && final_pixel[0] < 60);
}

#[test]
fn a_post_action_clip_never_mutates_the_outgoing_transition_raster() {
    let mut doc = Doc::new();
    let destination = frame(500.0);
    let destination_id = destination.id;
    doc.apply(Operation::create_node(destination)).unwrap();

    let mut source = frame(0.0);
    let source_id = source.id;
    let clip_id = AnimationClipId::from_u128(970);
    source.reactions.push(Reaction {
        id: ReactionId::from_u128(971),
        trigger: Trigger::Key {
            keys: vec!["Enter".into()],
        },
        action: Action::Navigate { to: destination_id },
        extra_actions: Vec::new(),
        transition: Some(Transition {
            style: TransitionStyle::Dissolve,
            duration_ms: 100,
            easing: Easing::Linear,
        }),
        animation: Some(PrototypeAnimation {
            clip: clip_id,
            delay_ms: 0,
        }),
    });
    doc.apply(Operation::create_node(source)).unwrap();

    let mut moving = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        0.0,
        0.0,
        20.0,
        20.0,
        Color::rgb(255, 0, 0),
    )));
    let moving_id = moving.id;
    moving.parent = Some(source_id);
    moving.transform = Transform2D::translation(10.0, 80.0);
    doc.apply(Operation::create_node(moving)).unwrap();
    doc.motion
        .clips
        .insert(clip_id, position_clip(clip_id, moving_id, 10.0, 150.0));

    let mut session = PresentSession::new(&doc, Some(source_id), DVec2::splat(200.0)).unwrap();
    session.handle_key("Enter", KeyEvent::Down);
    session.tick(0.05);
    let midpoint = session.present_rgba();
    let authored_position = pixel_at(&midpoint, 200, 20, 90);
    assert!(
        authored_position[0] > 240 && authored_position[3] > 100,
        "the outgoing square remains at its authored x position: {authored_position:?}"
    );
    assert_eq!(
        pixel_at(&midpoint, 200, 90, 90)[3],
        0,
        "the destination-only clip must not move the outgoing square"
    );
}

#[test]
fn transition_and_property_animation_share_figma_easing_samples() {
    let mut doc = Doc::new();
    let destination = frame_colored(500.0, Color::WHITE);
    let destination_id = destination.id;
    doc.apply(Operation::create_node(destination)).unwrap();
    let mut moving = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        0.0,
        0.0,
        20.0,
        20.0,
        Color::rgb(0, 0, 255),
    )));
    let moving_id = moving.id;
    moving.parent = Some(destination_id);
    doc.apply(Operation::create_node(moving)).unwrap();

    let clip_id = AnimationClipId::from_u128(980);
    let mut clip = position_clip(clip_id, moving_id, 0.0, 100.0);
    let first = clip
        .tracks
        .values_mut()
        .next()
        .and_then(|track| {
            track
                .keyframes
                .values_mut()
                .min_by_key(|keyframe| keyframe.time_ms)
        })
        .expect("position clip has a first keyframe");
    first.easing = Easing::EaseOut;
    doc.motion.clips.insert(clip_id, clip);

    let mut source = frame_colored(0.0, Color::rgb(255, 0, 0));
    let source_id = source.id;
    source.reactions.push(Reaction {
        id: ReactionId::from_u128(981),
        trigger: Trigger::Key {
            keys: vec!["Enter".into()],
        },
        action: Action::Navigate { to: destination_id },
        extra_actions: Vec::new(),
        transition: Some(Transition {
            style: TransitionStyle::Dissolve,
            duration_ms: 100,
            easing: Easing::EaseOut,
        }),
        animation: Some(PrototypeAnimation {
            clip: clip_id,
            delay_ms: 0,
        }),
    });
    doc.apply(Operation::create_node(source)).unwrap();

    let mut session = PresentSession::new(&doc, Some(source_id), DVec2::splat(200.0)).unwrap();
    session.handle_key("Enter", KeyEvent::Down);
    session.tick(0.025);
    let transition_progress = session
        .active_transition
        .expect("active transition")
        .progress();
    let transition_sample = ease(Easing::EaseOut, transition_progress);
    let motion = session
        .prototype_motion_evaluation()
        .expect("active property animation");
    let value = motion
        .get(MotionTarget::new(moving_id, MotionProperty::PositionX))
        .expect("position sample");
    let ResolvedVarValue::Float { value } = value else {
        panic!("expected a float position sample, got {value:?}");
    };
    assert!((value / 100.0 - transition_sample).abs() < 1e-6);
    assert!((transition_sample - 0.378_138_13).abs() < 1e-6);
}

#[test]
fn a_new_reaction_animation_replaces_the_held_clip() {
    let mut doc = Doc::new();
    let frame = frame_colored(0.0, Color::WHITE);
    let frame_id = frame.id;
    doc.apply(Operation::create_node(frame)).unwrap();

    let mut moving = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        0.0,
        0.0,
        20.0,
        20.0,
        Color::rgb(255, 0, 0),
    )));
    let moving_id = moving.id;
    moving.parent = Some(frame_id);
    moving.transform = Transform2D::translation(10.0, 80.0);
    doc.apply(Operation::create_node(moving)).unwrap();

    let first_clip = AnimationClipId::from_u128(720);
    let second_clip = AnimationClipId::from_u128(721);
    let missing_clip = AnimationClipId::from_u128(729);
    doc.motion.clips.insert(
        first_clip,
        position_clip(first_clip, moving_id, 10.0, 150.0),
    );
    doc.motion.clips.insert(
        second_clip,
        position_clip(second_clip, moving_id, 10.0, 70.0),
    );

    for (index, (x, clip)) in [
        (10.0, first_clip),
        (40.0, second_clip),
        (70.0, missing_clip),
    ]
    .into_iter()
    .enumerate()
    {
        let mut button = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
            0.0,
            0.0,
            20.0,
            20.0,
            Color::rgb(0, 0, 255),
        )));
        button.parent = Some(frame_id);
        button.index = fanta_doc::IndexKey::from_raw(10.0 + index as f64);
        button.transform = Transform2D::translation(x, 10.0);
        button.reactions.push(Reaction {
            id: ReactionId::from_u128(722 + index as u128),
            trigger: Trigger::Click,
            action: Action::OpenLink {
                url: format!("https://example.com/{index}"),
            },
            extra_actions: Vec::new(),
            transition: None,
            animation: Some(PrototypeAnimation { clip, delay_ms: 0 }),
        });
        doc.apply(Operation::create_node(button)).unwrap();
    }

    let mut session = PresentSession::new(&doc, Some(frame_id), DVec2::splat(200.0)).unwrap();
    assert!(
        session
            .handle_pointer(DVec2::new(15.0, 15.0), PointerEvent::Click)
            .animating
    );
    session.tick(0.1);
    assert!(pixel_at(&session.present_rgba(), 200, 160, 90)[0] > 200);

    assert!(
        session
            .handle_pointer(DVec2::new(45.0, 15.0), PointerEvent::Click)
            .animating
    );
    assert!(
        pixel_at(&session.present_rgba(), 200, 20, 90)[0] > 200,
        "replacement begins at the new clip's first keyframe"
    );
    session.tick(0.1);
    assert!(
        pixel_at(&session.present_rgba(), 200, 80, 90)[0] > 200,
        "the second clip owns the held final sample"
    );

    let missing = session.handle_pointer(DVec2::new(75.0, 15.0), PointerEvent::Click);
    assert!(missing.needs_redraw && !missing.animating);
    assert!(
        pixel_at(&session.present_rgba(), 200, 20, 90)[0] > 200,
        "a dangling replacement clears the prior held sample without panicking"
    );
}

#[test]
fn trigger_delay_and_property_animation_delay_are_independent() {
    let mut doc = Doc::new();
    let frame = frame_colored(0.0, Color::WHITE);
    let frame_id = frame.id;
    doc.apply(Operation::create_node(frame)).unwrap();
    let mut moving = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        0.0,
        0.0,
        20.0,
        20.0,
        Color::rgb(255, 0, 0),
    )));
    let moving_id = moving.id;
    moving.parent = Some(frame_id);
    moving.transform = Transform2D::translation(10.0, 80.0);
    doc.apply(Operation::create_node(moving)).unwrap();

    let clip_id = AnimationClipId::from_u128(730);
    doc.motion
        .clips
        .insert(clip_id, position_clip(clip_id, moving_id, 10.0, 150.0));
    doc.scene
        .get_mut(frame_id)
        .unwrap()
        .reactions
        .push(Reaction {
            id: ReactionId::from_u128(731),
            trigger: Trigger::AfterDelay { delay_ms: 40 },
            action: Action::OpenLink {
                url: "https://example.com/timer-fired".into(),
            },
            extra_actions: Vec::new(),
            transition: None,
            animation: Some(PrototypeAnimation {
                clip: clip_id,
                delay_ms: 30,
            }),
        });
    let mut session = PresentSession::new(&doc, Some(frame_id), DVec2::splat(200.0)).unwrap();
    session.tick(0.039);
    assert!(
        session.take_open_url().is_none(),
        "trigger is still pending"
    );

    let trigger_boundary = session.tick(0.001);
    assert!(trigger_boundary.animating);
    assert_eq!(
        session.take_open_url().as_deref(),
        Some("https://example.com/timer-fired"),
        "the action fires at the trigger's own delay"
    );
    session.tick(0.029);
    assert!(pixel_at(&session.present_rgba(), 200, 20, 90)[0] > 200);
    let animation_boundary = session.tick(0.001);
    assert!(animation_boundary.animating && animation_boundary.needs_redraw);
    assert!(pixel_at(&session.present_rgba(), 200, 20, 90)[0] > 200);

    session.tick(0.05);
    assert!(
        pixel_at(&session.present_rgba(), 200, 90, 90)[0] > 200,
        "the clip midpoint begins 30ms after the action, not at the trigger delay"
    );
}

#[test]
fn a_click_that_hits_nothing_does_not_navigate() {
    let mut doc = Doc::new();
    let mut a = frame(0.0);
    let a_id = a.id;
    a.reactions
        .push(click(1, Trigger::Click, Action::Navigate { to: a_id }));
    doc.apply(Operation::create_node(a)).unwrap();

    let mut session = PresentSession::new(&doc, Some(a_id), DVec2::new(200.0, 200.0)).unwrap();
    // Way outside the frame in world space (top-left corner maps far off A).
    let r = session.handle_pointer(DVec2::new(-9999.0, -9999.0), PointerEvent::Click);
    assert!(!r.navigated, "a miss must not fire: {r:?}");
}

#[test]
fn media_time_advances_on_tick_when_playing() {
    let mut doc = Doc::new();
    let frame = frame(0.0);
    let frame_id = frame.id;
    doc.apply(Operation::create_node(frame)).unwrap();

    let screen = DVec2::new(100.0, 100.0);
    let mut session = PresentSession::new(&doc, Some(frame_id), screen).unwrap();

    session.play_media();
    let r = session.tick(1.5);
    assert!(session.media_time() > 1.0);
    assert!(r.media_updated);
    session.pause_media();
    let before = session.media_time();
    let r2 = session.tick(10.0);
    assert_eq!(session.media_time(), before); // no advance when paused
    assert!(!r2.media_updated);
}

#[test]
fn media_playback_exposes_per_video_progress() {
    use fanta_doc::id::AssetId;
    use fanta_doc::{AudioNode, VideoNode};
    let mut doc = Doc::new();
    let frame = frame(0.0);
    let frame_id = frame.id;
    doc.apply(Operation::create_node(frame)).unwrap();

    // A simple 10s video clip node
    let mut vid = CanvasNode::new(NodeData::Video(VideoNode {
        asset: AssetId::new(),
        natural_size: [640, 360],
        local_size: [320.0, 180.0],
        time_range_us: [0, 10_000_000],
        speed: 1.0,
        muted: false,
        volume: 1.0,
        poster_frame_us: None,
        poster: None,
        fit: fanta_doc::style::ImageFitMode::Fill,
    }));
    vid.parent = Some(frame_id);
    vid.index = doc.scene.next_child_index(Some(frame_id));
    doc.apply(Operation::create_node(vid)).unwrap();

    // Also exercise AudioNode path
    let mut aud = CanvasNode::new(NodeData::Audio(AudioNode {
        asset: AssetId::new(),
        local_size: [100.0, 20.0],
        time_range_us: [0, 5_000_000],
        volume: 0.8,
        muted: false,
        waveform_color: fanta_doc::color::Color::rgb(0, 0, 0),
    }));
    aud.parent = Some(frame_id);
    aud.index = doc.scene.next_child_index(Some(frame_id));
    let audio_id = aud.id;
    doc.apply(Operation::create_node(aud)).unwrap();

    let mut session = PresentSession::new(&doc, Some(frame_id), DVec2::new(400.0, 300.0)).unwrap();
    assert!(session.media_playback().is_empty());

    session.play_media();
    session.set_media_time(2.5);
    let pb = session.media_playback();
    assert_eq!(pb.len(), 2, "should have entries for video and audio");
    // progress ~0.25 for video
    // for audio at 2.5s in 0..5s => 0.5
    for (is_audio, p) in pb.values().map(|p| (p.progress > 0.4, p)) {
        if is_audio {
            assert!(
                (p.progress - 0.5).abs() < 0.02,
                "audio progress approx 0.5, got {}",
                p.progress
            );
        } else {
            assert!(
                (p.progress - 0.25).abs() < 0.02,
                "video progress approx 0.25, got {}",
                p.progress
            );
        }
    }

    // Per-clip override: seek only the audio independently
    session.set_node_media_time(audio_id, 4.0); // near end of 5s audio
    let pb3 = session.media_playback();
    // video still at ~2.5 (0.25), audio now at 4.0 /5 = 0.8
    let audio_p = pb3.get(&audio_id).expect("audio entry");
    assert!(
        (audio_p.progress - 0.8).abs() < 0.02,
        "per-clip audio seek to 0.8, got {}",
        audio_p.progress
    );
    session.pause_media();
}

#[test]
fn scrollto_inside_scrollable_frame_sets_container_offset() {
    // Root surface (clickable) containing a scrollable frame with a target
    // positioned "below" the visible clip. Clicking the root fires ScrollTo
    // to the target; we assert that a per-container scroll offset is recorded
    // on the scrollable group.
    let mut doc = Doc::new();

    // Scrollable container (acts as a scrolling frame)
    let mut scroll_frame = CanvasNode::new(NodeData::Group(GroupNode {
        clip_size: Some([200.0, 200.0]),
        scrollable: true,
        ..GroupNode::default()
    }));
    scroll_frame.transform = Transform2D::translation(0.0, 0.0);
    let scroll_id = scroll_frame.id;
    doc.apply(Operation::create_node(scroll_frame)).unwrap();

    // Target child deep inside the scrollable (positioned low so scroll matters)
    let mut target = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        10.0,
        300.0,
        50.0,
        30.0,
        Color::rgb(255, 0, 0),
    )));
    target.parent = Some(scroll_id);
    let target_id = target.id;
    doc.apply(Operation::create_node(target)).unwrap();

    // Root clickable frame that contains the scrollable and has the ScrollTo reaction
    let mut root = frame(0.0);
    let root_id = root.id;
    root.reactions.push(click(
        99,
        Trigger::Click,
        Action::ScrollTo { target: target_id },
    ));
    root.parent = None;
    doc.apply(Operation::create_node(root)).unwrap();

    // Make the scrollable a child of root for a realistic tree
    if let Some(sf) = doc.scene.get_mut(scroll_id) {
        sf.parent = Some(root_id);
    }

    let mut session = PresentSession::new(&doc, Some(root_id), DVec2::new(400.0, 400.0)).unwrap();
    assert_eq!(
        session.scroll_offset(scroll_id),
        [0.0, 0.0],
        "no scroll initially"
    );

    // Click in the center of the root surface -> should fire ScrollTo
    let r = session.handle_pointer(DVec2::new(100.0, 100.0), PointerEvent::Click);
    assert!(r.needs_redraw, "ScrollTo should request redraw: {r:?}");

    // The scrollable container should now have a non-zero offset recorded
    let off = session.scroll_offset(scroll_id);
    assert!(
        off[1].abs() > 1.0,
        "expected non-trivial vertical scroll offset for target inside scrollable, got {:?}",
        off
    );

    // Clearing works
    session.clear_scroll_offset(scroll_id);
    assert_eq!(session.scroll_offset(scroll_id), [0.0, 0.0]);
}

fn add_variant_member(
    doc: &mut Doc,
    set_id: ComponentId,
    member_id: ComponentId,
    prop_id: ComponentPropId,
    state: &str,
    color: Color,
) {
    let mut root = frame_sized(1000.0 + member_id.to_u128() as f64, 0.0, 40.0, 40.0, color);
    root.name = format!("State={state}");
    let root_id = root.id;
    doc.apply(Operation::create_node(root)).unwrap();

    let mut axis_values = BTreeMap::new();
    axis_values.insert("State".to_owned(), state.to_owned());
    let mut def = ComponentDef::new(member_id, root_id, format!("State={state}"));
    def.variant_of = Some(ComponentSetMembership {
        set: set_id,
        axis_values,
    });
    def.props.push(ComponentPropDef {
        id: prop_id,
        name: "State".to_owned(),
        kind: ComponentPropKind::Variant {
            axis: "State".to_owned(),
        },
        formatter: ComponentPropFormatter::Auto,
        default: VarValue::String {
            value: state.to_owned(),
        },
        bindings: Vec::new(),
    });
    doc.components.defs.insert(member_id, def);
}

fn add_variant_member_with_moving_child(
    doc: &mut Doc,
    set_id: ComponentId,
    member_id: ComponentId,
    prop_id: ComponentPropId,
    state: &str,
    child_x: f64,
) {
    let mut root = CanvasNode::new(NodeData::Group(GroupNode {
        clip_size: Some([40.0, 40.0]),
        ..GroupNode::default()
    }));
    root.name = "Toggle".into();
    root.transform = Transform2D::translation(1000.0 + member_id.to_u128() as f64, 0.0);
    let root_id = root.id;
    doc.apply(Operation::create_node(root)).unwrap();

    let mut child = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        0.0,
        0.0,
        10.0,
        10.0,
        Color::rgb(255, 0, 0),
    )));
    child.name = "Knob".into();
    child.parent = Some(root_id);
    child.transform = Transform2D::translation(child_x, 15.0);
    doc.apply(Operation::create_node(child)).unwrap();

    let mut axis_values = BTreeMap::new();
    axis_values.insert("State".to_owned(), state.to_owned());
    let mut definition = ComponentDef::new(member_id, root_id, format!("State={state}"));
    definition.variant_of = Some(ComponentSetMembership {
        set: set_id,
        axis_values,
    });
    definition.props.push(ComponentPropDef {
        id: prop_id,
        name: "State".to_owned(),
        kind: ComponentPropKind::Variant {
            axis: "State".to_owned(),
        },
        formatter: ComponentPropFormatter::Auto,
        default: VarValue::String {
            value: state.to_owned(),
        },
        bindings: Vec::new(),
    });
    doc.components.defs.insert(member_id, definition);
}

#[test]
fn update_variant_action_visually_switches_component_member() {
    let set_id = ComponentId::from_u128(700);
    let off_id = ComponentId::from_u128(701);
    let on_id = ComponentId::from_u128(702);
    let prop_id = ComponentPropId::from_u128(703);

    let mut doc = Doc::new();
    add_variant_member(
        &mut doc,
        set_id,
        off_id,
        prop_id,
        "Off",
        Color::rgb(255, 0, 0),
    );
    add_variant_member(
        &mut doc,
        set_id,
        on_id,
        prop_id,
        "On",
        Color::rgb(0, 255, 0),
    );
    doc.components.sets.insert(
        set_id,
        ComponentSet {
            id: set_id,
            name: "Toggle".to_owned(),
            axes: vec![VariantAxis {
                name: "State".to_owned(),
                values: vec!["Off".to_owned(), "On".to_owned()],
            }],
            members: vec![off_id, on_id],
            default_variant: off_id,
        },
    );

    let mut frame = frame_sized(0.0, 0.0, 80.0, 80.0, Color::WHITE);
    let frame_id = frame.id;
    frame.reactions.push(Reaction {
        id: ReactionId::from_u128(90),
        trigger: Trigger::Click,
        action: Action::UpdateVariant {
            component: set_id,
            variant: "On".to_owned(),
        },
        extra_actions: Vec::new(),
        transition: None,
        animation: None,
    });
    doc.apply(Operation::create_node(frame)).unwrap();

    let mut instance = CanvasNode::new(NodeData::Instance(InstanceNode {
        component: set_id,
        overrides: Vec::new(),
        prop_values: BTreeMap::new(),
        derived: Vec::new(),
        local_size: [40.0, 40.0],
    }));
    let instance_id = instance.id;
    instance.parent = Some(frame_id);
    instance.index = doc.scene.next_child_index(Some(frame_id));
    instance.transform = Transform2D::translation(20.0, 20.0);
    doc.apply(Operation::create_node(instance)).unwrap();

    let mut session = PresentSession::new(&doc, Some(frame_id), DVec2::new(80.0, 80.0)).unwrap();
    let before = center_pixel(&session.present_rgba(), 80, 80);
    assert!(
        before[0] > 200 && before[1] < 80 && before[2] < 80,
        "default variant renders red, got {before:?}"
    );

    let response = session.handle_pointer(DVec2::new(40.0, 40.0), PointerEvent::Click);
    assert!(
        response.needs_redraw,
        "UpdateVariant requests redraw: {response:?}"
    );
    let after = center_pixel(&session.present_rgba(), 80, 80);
    assert!(
        after[1] > 200 && after[0] < 80 && after[2] < 80,
        "selected variant renders green, got {after:?}"
    );

    match &doc.scene.get(instance_id).unwrap().data {
        NodeData::Instance(inst) => {
            assert_eq!(
                inst.component, set_id,
                "source doc instance was not swapped"
            );
            assert!(
                inst.prop_values.is_empty(),
                "source doc variant props were not mutated"
            );
        }
        other => panic!("expected instance, got {other:?}"),
    }
}

#[test]
fn update_variant_action_triggers_redraw_for_components() {
    // Prototype UpdateVariant (for component variant switching in motion) triggers needs_redraw.
    let mut doc = Doc::new();
    let mut frame = frame(0.0);
    let frame_id = frame.id;
    // Add reaction with UpdateVariant (component placeholder)
    frame.reactions.push(Reaction {
        id: ReactionId::from_u128(99),
        trigger: Trigger::Click,
        action: Action::UpdateVariant {
            component: fanta_doc::id::ComponentId::from_u128(1),
            variant: "VariantA".to_string(),
        },
        extra_actions: Vec::new(),
        transition: None,
        animation: None,
    });
    doc.apply(Operation::create_node(frame)).unwrap();

    let mut session = PresentSession::new(&doc, Some(frame_id), DVec2::new(200.0, 200.0)).unwrap();
    let r = session.handle_pointer(DVec2::new(100.0, 100.0), PointerEvent::Click);
    assert!(r.needs_redraw, "UpdateVariant should request redraw");
    // Make sure variant override state is recorded for fidelity (prototype component switch).
    let comp = fanta_doc::id::ComponentId::from_u128(1);
    assert_eq!(
        session.variant_overrides().get(&comp),
        Some(&"VariantA".to_string())
    );

    // Exercise the patched render path (variant overrides cause a scratch scene
    // with prop_values updated for live component variant switching in playback).
    // This path is taken in render_current / present_rgba for full motion+component fidelity.
    let _metrics = session.render_current();

    session.clear_variant_override(comp);
    assert!(session.variant_overrides().get(&comp).is_none());
}

#[test]
fn open_link_action_triggers_redraw_for_prototype() {
    let mut doc = Doc::new();
    let mut frame = frame(0.0);
    let frame_id = frame.id;
    frame.reactions.push(Reaction {
        id: ReactionId::from_u128(100),
        trigger: Trigger::Click,
        action: Action::OpenLink {
            url: "https://example.com".to_string(),
        },
        extra_actions: Vec::new(),
        transition: None,
        animation: None,
    });
    doc.apply(Operation::create_node(frame)).unwrap();

    let mut session = PresentSession::new(&doc, Some(frame_id), DVec2::new(200.0, 200.0)).unwrap();
    let r = session.handle_pointer(DVec2::new(100.0, 100.0), PointerEvent::Click);
    assert!(r.needs_redraw, "OpenLink should request redraw");
    assert_eq!(
        session.take_open_url().as_deref(),
        Some("https://example.com")
    );
    assert!(session.take_open_url().is_none());
}

#[test]
fn resizing_preserves_session_state_and_uses_physical_display_scale() {
    let mut doc = Doc::new();
    let frame = frame(0.0);
    let frame_id = frame.id;
    doc.apply(Operation::create_node(frame)).unwrap();
    let mut session = PresentSession::new(&doc, Some(frame_id), DVec2::new(200.0, 100.0)).unwrap();
    session.set_variant_override(ComponentId::from_u128(9), "Hover".into());

    session
        .resize_with_scale(DVec2::new(300.0, 180.0), 2.0)
        .unwrap();
    assert_eq!(session.current_frame(), frame_id);
    assert_eq!(session.pixel_size(), (600, 360));
    assert_eq!(session.present_rgba().len(), 600 * 360 * 4);
    assert_eq!(
        session
            .variant_overrides()
            .get(&ComponentId::from_u128(9))
            .map(String::as_str),
        Some("Hover")
    );
}

#[test]
fn navigating_from_an_overlay_dismisses_the_overlay_stack() {
    let mut doc = Doc::new();
    let destination = frame_sized(600.0, 0.0, 200.0, 200.0, Color::rgb(0, 0, 255));
    let destination_id = destination.id;
    doc.apply(Operation::create_node(destination)).unwrap();

    let mut overlay = frame_sized(300.0, 0.0, 100.0, 100.0, Color::rgb(0, 255, 0));
    let overlay_id = overlay.id;
    overlay.reactions.push(click(
        301,
        Trigger::Click,
        Action::Navigate { to: destination_id },
    ));
    doc.apply(Operation::create_node(overlay)).unwrap();

    let mut base = frame_sized(0.0, 0.0, 200.0, 200.0, Color::WHITE);
    let base_id = base.id;
    base.reactions.push(click(
        302,
        Trigger::Click,
        Action::OpenOverlay {
            frame: overlay_id,
            overlay: OverlaySettings {
                position: OverlayPosition::Center,
                background_dim: false,
                close_on_click_outside: false,
            },
        },
    ));
    doc.apply(Operation::create_node(base)).unwrap();

    let mut session = PresentSession::new(&doc, Some(base_id), DVec2::splat(200.0)).unwrap();
    session.handle_pointer(DVec2::splat(100.0), PointerEvent::Click);
    assert_eq!(session.overlays.len(), 1);
    session.handle_pointer(DVec2::splat(100.0), PointerEvent::Click);
    assert_eq!(session.current_frame(), destination_id);
    assert!(session.overlays.is_empty());
    let center = center_pixel(&session.present_rgba(), 200, 200);
    assert!(center[2] > 200 && center[0] < 60 && center[1] < 60);
}

#[test]
fn nested_scroll_to_moves_only_scroll_content_for_render_and_hit_testing() {
    let mut doc = Doc::new();
    let mut root = frame_sized(0.0, 0.0, 200.0, 200.0, Color::WHITE);
    let root_id = root.id;
    doc.apply(Operation::create_node(root.clone())).unwrap();

    let mut scrollable = CanvasNode::new(NodeData::Group(GroupNode {
        clip_size: Some([100.0, 100.0]),
        scrollable: true,
        ..GroupNode::default()
    }));
    let scrollable_id = scrollable.id;
    scrollable.parent = Some(root_id);
    scrollable.transform = Transform2D::translation(50.0, 50.0);
    doc.apply(Operation::create_node(scrollable)).unwrap();

    let mut wrapper = CanvasNode::new(NodeData::Group(GroupNode::default()));
    let wrapper_id = wrapper.id;
    wrapper.parent = Some(scrollable_id);
    doc.apply(Operation::create_node(wrapper)).unwrap();

    let mut target = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        10.0,
        200.0,
        20.0,
        20.0,
        Color::rgb(255, 0, 0),
    )));
    let target_id = target.id;
    target.parent = Some(wrapper_id);
    target.reactions.push(click(
        303,
        Trigger::Click,
        Action::OpenLink {
            url: "https://example.com/scrolled".into(),
        },
    ));
    doc.apply(Operation::create_node(target)).unwrap();

    root.reactions.push(click(
        304,
        Trigger::Click,
        Action::ScrollTo { target: target_id },
    ));
    doc.scene.get_mut(root_id).unwrap().reactions = root.reactions;

    let mut session = PresentSession::new(&doc, Some(root_id), DVec2::splat(200.0)).unwrap();
    let before = pixel_at(&session.present_rgba(), 200, 100, 100);
    assert!(before[0] > 240 && before[1] > 240 && before[2] > 240);
    session.handle_pointer(DVec2::new(10.0, 10.0), PointerEvent::Click);
    // ScrollTo is clamped to the container's real scroll range: content max-y
    // is 220 against a 100-tall viewport ⇒ y pins at 120 (not the unclamped
    // 160 centering overshoot), and x never scrolls negative past the content
    // start. The target lands fully visible at local (10..30, 80..100) —
    // world (60..80, 130..150).
    assert_eq!(session.scroll_offset(scrollable_id), [0.0, 120.0]);
    let after = pixel_at(&session.present_rgba(), 200, 70, 140);
    assert!(
        after[0] > 200 && after[1] < 60 && after[2] < 60,
        "{after:?}"
    );
    session.handle_pointer(DVec2::new(70.0, 140.0), PointerEvent::Click);
    assert_eq!(
        session.take_open_url().as_deref(),
        Some("https://example.com/scrolled")
    );
}

#[test]
fn hover_fires_on_entry_only_and_reenters_after_leaving() {
    let mut doc = Doc::new();
    let mut surface = frame(0.0);
    let surface_id = surface.id;
    surface.reactions.push(click(
        305,
        Trigger::Hover,
        Action::OpenLink {
            url: "https://example.com/hover".into(),
        },
    ));
    doc.apply(Operation::create_node(surface)).unwrap();
    let mut session = PresentSession::new(&doc, Some(surface_id), DVec2::splat(200.0)).unwrap();

    session.handle_pointer(DVec2::splat(100.0), PointerEvent::Move);
    assert!(session.take_open_url().is_some());
    session.handle_pointer(DVec2::splat(100.0), PointerEvent::Move);
    assert!(session.take_open_url().is_none());
    session.handle_pointer(DVec2::new(-10.0, -10.0), PointerEvent::Move);
    session.handle_pointer(DVec2::splat(100.0), PointerEvent::Move);
    assert!(session.take_open_url().is_some());
}

fn interactive_variant_document() -> (Doc, NodeId, NodeId, NodeId) {
    let set_id = ComponentId::from_u128(800);
    let default_id = ComponentId::from_u128(801);
    let hover_id = ComponentId::from_u128(802);
    let pressed_id = ComponentId::from_u128(803);
    let prop_id = ComponentPropId::from_u128(804);
    let mut doc = Doc::new();
    add_variant_member(
        &mut doc,
        set_id,
        default_id,
        prop_id,
        "Default",
        Color::rgb(255, 0, 0),
    );
    add_variant_member(
        &mut doc,
        set_id,
        hover_id,
        prop_id,
        "Hover",
        Color::rgb(0, 255, 0),
    );
    add_variant_member(
        &mut doc,
        set_id,
        pressed_id,
        prop_id,
        "Pressed",
        Color::rgb(0, 0, 255),
    );
    doc.components.sets.insert(
        set_id,
        ComponentSet {
            id: set_id,
            name: "Button".into(),
            axes: vec![VariantAxis {
                name: "State".into(),
                values: vec!["Default".into(), "Hover".into(), "Pressed".into()],
            }],
            members: vec![default_id, hover_id, pressed_id],
            default_variant: default_id,
        },
    );

    let frame = frame_sized(0.0, 0.0, 100.0, 50.0, Color::WHITE);
    let frame_id = frame.id;
    doc.apply(Operation::create_node(frame)).unwrap();
    let destination = frame_sized(300.0, 0.0, 100.0, 50.0, Color::WHITE);
    let destination_id = destination.id;
    doc.apply(Operation::create_node(destination)).unwrap();

    let mut first = CanvasNode::new(NodeData::Instance(InstanceNode {
        component: set_id,
        overrides: Vec::new(),
        prop_values: BTreeMap::new(),
        derived: Vec::new(),
        local_size: [40.0, 40.0],
    }));
    let first_id = first.id;
    first.parent = Some(frame_id);
    first.transform = Transform2D::translation(5.0, 5.0);
    first.reactions.extend([
        click(
            306,
            Trigger::Hover,
            Action::UpdateVariant {
                component: set_id,
                variant: "Hover".into(),
            },
        ),
        click(
            307,
            Trigger::WhilePressing,
            Action::UpdateVariant {
                component: set_id,
                variant: "Pressed".into(),
            },
        ),
        click(308, Trigger::Click, Action::Navigate { to: destination_id }),
    ]);
    doc.apply(Operation::create_node(first)).unwrap();

    let mut second = CanvasNode::new(NodeData::Instance(InstanceNode {
        component: set_id,
        overrides: Vec::new(),
        prop_values: BTreeMap::new(),
        derived: Vec::new(),
        local_size: [40.0, 40.0],
    }));
    let second_id = second.id;
    second.parent = Some(frame_id);
    second.transform = Transform2D::translation(55.0, 5.0);
    doc.apply(Operation::create_node(second)).unwrap();
    (doc, frame_id, first_id, second_id)
}

#[test]
fn hover_and_press_variants_are_instance_local_and_transient() {
    let (doc, frame_id, first_id, _) = interactive_variant_document();
    let mut session = PresentSession::new(&doc, Some(frame_id), DVec2::new(100.0, 50.0)).unwrap();

    session.handle_pointer(DVec2::new(25.0, 25.0), PointerEvent::Move);
    let hover = session.present_rgba();
    assert!(pixel_at(&hover, 100, 25, 25)[1] > 200);
    assert!(pixel_at(&hover, 100, 75, 25)[0] > 200);

    let down = session.handle_pointer(DVec2::new(25.0, 25.0), PointerEvent::Down);
    assert!(!down.suppress_click);
    let pressed = session.present_rgba();
    assert!(pixel_at(&pressed, 100, 25, 25)[2] > 200);
    assert!(pixel_at(&pressed, 100, 75, 25)[0] > 200);

    let up = session.handle_pointer(DVec2::new(25.0, 25.0), PointerEvent::Up);
    assert!(up.needs_redraw);
    assert_eq!(
        session.instance_variant_overrides.get(&first_id),
        Some(&(ComponentId::from_u128(800), "Hover".into()))
    );
    let released = session.present_rgba();
    assert!(pixel_at(&released, 100, 25, 25)[1] > 200);

    session.handle_pointer(DVec2::new(25.0, 25.0), PointerEvent::Click);
    assert_ne!(session.current_frame(), frame_id);
}

#[test]
fn update_variant_smart_animate_crossfades_resolved_component_pixels() {
    let (mut doc, frame_id, first_id, second_id) = interactive_variant_document();
    let set_id = ComponentId::from_u128(800);
    doc.scene.get_mut(first_id).unwrap().reactions = vec![click_with(
        313,
        Action::UpdateVariant {
            component: set_id,
            variant: "Hover".into(),
        },
        Transition {
            style: TransitionStyle::SmartAnimate,
            duration_ms: 100,
            easing: Easing::Linear,
        },
    )];

    let mut session = PresentSession::new(&doc, Some(frame_id), DVec2::new(100.0, 50.0)).unwrap();
    let before = session.present_rgba();
    let before_first = pixel_at(&before, 100, 25, 25);
    assert!(before_first[0] > 200 && before_first[1] < 60);

    let response = session.handle_pointer(DVec2::new(25.0, 25.0), PointerEvent::Click);
    assert!(
        response.animating,
        "variant swap should animate: {response:?}"
    );
    assert_eq!(
        session.present_rgba(),
        before,
        "the resolved variant does not cut in at t=0"
    );

    session.tick(0.05);
    let midpoint = session.present_rgba();
    let midpoint_first = pixel_at(&midpoint, 100, 25, 25);
    assert!(
        midpoint_first[0] > 80 && midpoint_first[1] > 80 && midpoint_first[2] < 60,
        "midpoint blends the red and green variants: {midpoint_first:?}"
    );
    let midpoint_second = pixel_at(&midpoint, 100, 75, 25);
    assert!(
        midpoint_second[0] > 200 && midpoint_second[1] < 60,
        "the sibling instance remains unchanged: {midpoint_second:?}"
    );

    let response = session.tick(0.05);
    assert!(!response.animating);
    let completed = session.present_rgba();
    let completed_first = pixel_at(&completed, 100, 25, 25);
    assert!(completed_first[1] > 200 && completed_first[0] < 60);
    assert_eq!(
        session.instance_variant_overrides.get(&first_id),
        Some(&(set_id, "Hover".into()))
    );
    assert!(!session.instance_variant_overrides.contains_key(&second_id));
}

#[test]
fn update_variant_smart_animate_interpolates_matched_component_layers_locally() {
    let set_id = ComponentId::from_u128(990);
    let left_id = ComponentId::from_u128(991);
    let right_id = ComponentId::from_u128(992);
    let prop_id = ComponentPropId::from_u128(993);
    let mut doc = Doc::new();
    add_variant_member_with_moving_child(&mut doc, set_id, left_id, prop_id, "Left", 2.0);
    add_variant_member_with_moving_child(&mut doc, set_id, right_id, prop_id, "Right", 28.0);
    doc.components.sets.insert(
        set_id,
        ComponentSet {
            id: set_id,
            name: "Toggle".into(),
            axes: vec![VariantAxis {
                name: "State".into(),
                values: vec!["Left".into(), "Right".into()],
            }],
            members: vec![left_id, right_id],
            default_variant: left_id,
        },
    );

    let frame = frame_sized(0.0, 0.0, 100.0, 50.0, Color::WHITE);
    let frame_id = frame.id;
    doc.apply(Operation::create_node(frame)).unwrap();
    let mut instance = CanvasNode::new(NodeData::Instance(InstanceNode {
        component: set_id,
        overrides: Vec::new(),
        prop_values: BTreeMap::new(),
        derived: Vec::new(),
        local_size: [40.0, 40.0],
    }));
    instance.parent = Some(frame_id);
    instance.transform = Transform2D::translation(5.0, 5.0);
    instance.reactions.push(click_with(
        994,
        Action::UpdateVariant {
            component: set_id,
            variant: "Right".into(),
        },
        Transition {
            style: TransitionStyle::SmartAnimate,
            duration_ms: 100,
            easing: Easing::Linear,
        },
    ));
    doc.apply(Operation::create_node(instance)).unwrap();
    let mut stable = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        60.0,
        5.0,
        30.0,
        40.0,
        Color::rgb(0, 0, 255),
    )));
    stable.parent = Some(frame_id);
    doc.apply(Operation::create_node(stable)).unwrap();

    let mut session = PresentSession::new(&doc, Some(frame_id), DVec2::new(100.0, 50.0)).unwrap();
    let before = session.present_rgba();
    assert!(pixel_at(&before, 100, 12, 25)[0] > 200);
    let response = session.handle_pointer(DVec2::new(12.0, 25.0), PointerEvent::Click);
    assert!(response.animating);
    assert!(
        session
            .active_variant_transition
            .as_ref()
            .is_some_and(|active| active
                .layers
                .iter()
                .all(|layer| layer.smart_animate.is_some())),
        "matched component children use property interpolation, not a surface dissolve"
    );
    assert_eq!(session.present_rgba(), before);

    session.tick(0.05);
    let midpoint = session.present_rgba();
    assert!(
        pixel_at(&midpoint, 100, 25, 25)[0] > 200,
        "the matched Knob layer reaches its interpolated midpoint"
    );
    for y in 0..50 {
        for x in 50..100 {
            assert_eq!(
                pixel_at(&midpoint, 100, x, y),
                pixel_at(&before, 100, x, y),
                "unaffected pixels changed at ({x}, {y})"
            );
        }
    }

    session.tick(0.05);
    let completed = session.present_rgba();
    assert!(pixel_at(&completed, 100, 38, 25)[0] > 200);
}

#[test]
fn directional_update_variant_moves_only_the_affected_component_layer() {
    let set_id = ComponentId::from_u128(1000);
    let off_id = ComponentId::from_u128(1001);
    let on_id = ComponentId::from_u128(1002);
    let prop_id = ComponentPropId::from_u128(1003);
    let mut doc = Doc::new();
    add_variant_member(
        &mut doc,
        set_id,
        off_id,
        prop_id,
        "Off",
        Color::rgb(255, 0, 0),
    );
    add_variant_member(
        &mut doc,
        set_id,
        on_id,
        prop_id,
        "On",
        Color::rgb(0, 255, 0),
    );
    doc.components.sets.insert(
        set_id,
        ComponentSet {
            id: set_id,
            name: "Toggle".into(),
            axes: vec![VariantAxis {
                name: "State".into(),
                values: vec!["Off".into(), "On".into()],
            }],
            members: vec![off_id, on_id],
            default_variant: off_id,
        },
    );

    let frame = frame_sized(0.0, 0.0, 100.0, 100.0, Color::WHITE);
    let frame_id = frame.id;
    doc.apply(Operation::create_node(frame)).unwrap();
    let mut instance = CanvasNode::new(NodeData::Instance(InstanceNode {
        component: set_id,
        overrides: Vec::new(),
        prop_values: BTreeMap::new(),
        derived: Vec::new(),
        local_size: [40.0, 40.0],
    }));
    instance.parent = Some(frame_id);
    instance.transform = Transform2D::translation(5.0, 5.0);
    instance.reactions.push(click_with(
        1004,
        Action::UpdateVariant {
            component: set_id,
            variant: "On".into(),
        },
        Transition {
            style: TransitionStyle::Push {
                direction: Direction::Left,
            },
            duration_ms: 100,
            easing: Easing::Linear,
        },
    ));
    doc.apply(Operation::create_node(instance)).unwrap();
    let mut stable = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        10.0,
        60.0,
        30.0,
        30.0,
        Color::rgb(0, 0, 255),
    )));
    stable.parent = Some(frame_id);
    doc.apply(Operation::create_node(stable)).unwrap();

    let mut session = PresentSession::new(&doc, Some(frame_id), DVec2::splat(100.0)).unwrap();
    let before = session.present_rgba();
    assert!(
        session
            .handle_pointer(DVec2::new(25.0, 25.0), PointerEvent::Click)
            .animating
    );
    assert_eq!(session.present_rgba(), before);
    session.tick(0.05);
    let midpoint = session.present_rgba();
    for y in 55..100 {
        let start = y * 100 * 4;
        let end = start + 100 * 4;
        assert_eq!(
            &midpoint[start..end],
            &before[start..end],
            "the stable lower half moved during a component-only Push"
        );
    }
    session.tick(0.05);
    let completed = session.present_rgba();
    assert!(pixel_at(&completed, 100, 25, 25)[1] > 200);
}

#[test]
fn context_changing_while_pressing_suppresses_the_release_click() {
    let mut doc = Doc::new();
    let destination = frame(400.0);
    let destination_id = destination.id;
    doc.apply(Operation::create_node(destination)).unwrap();
    let mut source = frame(0.0);
    let source_id = source.id;
    source.reactions.extend([
        click(
            309,
            Trigger::WhilePressing,
            Action::OpenLink {
                url: "https://example.com/press".into(),
            },
        ),
        click(310, Trigger::Click, Action::Navigate { to: destination_id }),
    ]);
    doc.apply(Operation::create_node(source)).unwrap();
    let mut session = PresentSession::new(&doc, Some(source_id), DVec2::splat(200.0)).unwrap();

    let down = session.handle_pointer(DVec2::splat(100.0), PointerEvent::Down);
    assert!(down.suppress_click);
    session.handle_pointer(DVec2::splat(100.0), PointerEvent::Up);
    session.handle_pointer(DVec2::splat(100.0), PointerEvent::Click);
    assert_eq!(session.current_frame(), source_id);
}

#[test]
fn resize_rebases_history_and_an_in_flight_transition() {
    let mut doc = Doc::new();
    let mut destination = frame_sized(500.0, 0.0, 200.0, 100.0, Color::WHITE);
    let destination_id = destination.id;
    destination
        .reactions
        .push(click(311, Trigger::Click, Action::Back));
    doc.apply(Operation::create_node(destination)).unwrap();
    let mut source = frame_sized(0.0, 0.0, 100.0, 100.0, Color::WHITE);
    let source_id = source.id;
    source.reactions.push(click_with(
        312,
        Action::Navigate { to: destination_id },
        Transition {
            style: TransitionStyle::Dissolve,
            duration_ms: 1000,
            easing: Easing::Linear,
        },
    ));
    doc.apply(Operation::create_node(source)).unwrap();
    let mut session = PresentSession::new(&doc, Some(source_id), DVec2::splat(200.0)).unwrap();
    session.handle_pointer(DVec2::splat(100.0), PointerEvent::Click);
    session.tick(0.25);
    let elapsed = session.active_transition.unwrap().elapsed;

    let new_size = DVec2::new(400.0, 200.0);
    session.resize_with_scale(new_size, 2.0).unwrap();
    let active = session.active_transition.unwrap();
    assert_eq!(
        active.from_viewport,
        frame_viewport(&doc.scene, source_id, new_size)
    );
    assert_eq!(
        active.to_viewport,
        frame_viewport(&doc.scene, destination_id, new_size)
    );
    assert_eq!(active.elapsed, elapsed);
    assert_eq!(
        session.back_stack[0].viewport,
        frame_viewport(&doc.scene, source_id, new_size)
    );
    session.handle_pointer(DVec2::new(200.0, 100.0), PointerEvent::Click);
    assert_eq!(session.current_frame(), source_id);
    assert_eq!(
        session.viewport,
        frame_viewport(&doc.scene, source_id, new_size)
    );
}

#[test]
fn restart_at_resets_ephemeral_state_without_rebuilding_the_renderer() {
    let mut doc = Doc::new();
    let frame = frame(0.0);
    let frame_id = frame.id;
    doc.apply(Operation::create_node(frame)).unwrap();
    let mut session = PresentSession::new(&doc, Some(frame_id), DVec2::new(200.0, 100.0)).unwrap();
    session
        .resize_with_scale(DVec2::new(300.0, 180.0), 2.0)
        .unwrap();
    session.set_variant_override(ComponentId::from_u128(999), "Changed".into());
    session.scroll_offsets.insert(frame_id, [10.0, 20.0]);
    assert!(session.restart_at(frame_id));
    assert_eq!(session.pixel_size(), (600, 360));
    assert!(session.variant_overrides.is_empty());
    assert!(session.instance_variant_overrides.is_empty());
    assert!(session.scroll_offsets.is_empty());
    assert!(session.back_stack.is_empty());
}

// ---------------------------------------------------------------------------
// Scroll semantics (D3/F2): direction axes, clamping, fixed, sticky, seeding
// ---------------------------------------------------------------------------

use fanta_doc::{ScrollBehavior, ScrollDirection};

/// A 100×100 clipping container at (50,50) inside a 200×200 white root, with
/// an authored scroll direction. Returns (doc, root, container).
fn scrolling_fixture(direction: ScrollDirection) -> (Doc, NodeId, NodeId) {
    let mut doc = Doc::new();
    let root = frame_sized(0.0, 0.0, 200.0, 200.0, Color::WHITE);
    let root_id = root.id;
    doc.apply(Operation::create_node(root)).unwrap();

    let mut container = CanvasNode::new(NodeData::Group(GroupNode {
        clip_size: Some([100.0, 100.0]),
        scroll_direction: Some(direction),
        ..GroupNode::default()
    }));
    let container_id = container.id;
    container.parent = Some(root_id);
    container.transform = Transform2D::translation(50.0, 50.0);
    doc.apply(Operation::create_node(container)).unwrap();
    (doc, root_id, container_id)
}

fn colored_child(
    doc: &mut Doc,
    parent: NodeId,
    y: f64,
    height: f64,
    color: Color,
    behavior: ScrollBehavior,
) -> NodeId {
    // Positioned via TRANSFORM (like imported children), not baked path
    // coordinates — sticky pinning reads the child's translation.
    let mut child = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        0.0, 0.0, 100.0, height, color,
    )));
    child.parent = Some(parent);
    child.transform = Transform2D::translation(0.0, y);
    child.scroll_behavior = behavior;
    let id = child.id;
    doc.apply(Operation::create_node(child)).unwrap();
    id
}

#[test]
fn pr2_fixed_header_stays_pinned_while_content_scrolls() {
    let (mut doc, root, container) = scrolling_fixture(ScrollDirection::Vertical);
    colored_child(
        &mut doc,
        container,
        0.0,
        20.0,
        Color::rgb(255, 0, 0),
        ScrollBehavior::Fixed,
    );
    colored_child(
        &mut doc,
        container,
        130.0,
        40.0,
        Color::rgb(0, 0, 255),
        ScrollBehavior::Scrolls,
    );

    let mut session = PresentSession::new(&doc, Some(root), DVec2::splat(200.0)).unwrap();
    let scrolled = session.scroll_by([60.0, 60.0], [0.0, 50.0]);
    assert_eq!(scrolled, Some(container));
    assert_eq!(session.scroll_offset(container), [0.0, 50.0]);

    let frame = session.present_rgba();
    // Fixed header: still at the container top (world y 50..70).
    let header = pixel_at(&frame, 200, 60, 60);
    assert!(
        header[0] > 200 && header[2] < 60,
        "header moved: {header:?}"
    );
    // Content scrolled up 50: local 130..170 → 80..120, visible band 80..100
    // inside the clip → world 130..150.
    let content = pixel_at(&frame, 200, 60, 140);
    assert!(
        content[2] > 200 && content[0] < 60,
        "content did not scroll into view: {content:?}"
    );
}

#[test]
fn sticky_child_pins_at_the_container_edge() {
    let (mut doc, root, container) = scrolling_fixture(ScrollDirection::Vertical);
    colored_child(
        &mut doc,
        container,
        30.0,
        10.0,
        Color::rgb(0, 200, 0),
        ScrollBehavior::Sticky,
    );
    colored_child(
        &mut doc,
        container,
        150.0,
        40.0,
        Color::rgb(0, 0, 255),
        ScrollBehavior::Scrolls,
    );

    let mut session = PresentSession::new(&doc, Some(root), DVec2::splat(200.0)).unwrap();
    // Scroll past the sticky origin (30): it pins at the container top.
    session.scroll_by([60.0, 60.0], [0.0, 60.0]);
    let frame = session.present_rgba();
    let pinned = pixel_at(&frame, 200, 60, 55);
    assert!(pinned[1] > 150 && pinned[2] < 60, "not pinned: {pinned:?}");

    // A smaller scroll (10 < origin 30) still moves it like plain content.
    session.set_scroll_offset(container, [0.0, 10.0]);
    let frame = session.present_rgba();
    let moving = pixel_at(&frame, 200, 60, 75); // local 30-10=20 → world 70..80
    assert!(moving[1] > 150 && moving[2] < 60, "not moving: {moving:?}");
}

#[test]
fn scroll_axes_and_range_are_clamped() {
    let (mut doc, root, container) = scrolling_fixture(ScrollDirection::Vertical);
    colored_child(
        &mut doc,
        container,
        150.0,
        40.0,
        Color::rgb(0, 0, 255),
        ScrollBehavior::Scrolls,
    );
    let mut session = PresentSession::new(&doc, Some(root), DVec2::splat(200.0)).unwrap();

    // A horizontal delta on a vertical-only container scrolls nothing.
    assert_eq!(session.scroll_by([60.0, 60.0], [30.0, 0.0]), None);
    // Diagonal input: only the allowed axis is consumed.
    session.scroll_by([60.0, 60.0], [30.0, 30.0]);
    assert_eq!(session.scroll_offset(container), [0.0, 30.0]);
    // Overshoot clamps to content range (content max-y 190 − viewport 100).
    session.set_scroll_offset(container, [0.0, 500.0]);
    assert_eq!(session.scroll_offset(container), [0.0, 90.0]);
    // Negative overscroll clamps to the content start.
    session.set_scroll_offset(container, [-20.0, -20.0]);
    assert_eq!(session.scroll_offset(container), [0.0, 0.0]);
}

#[test]
fn pr12_authored_initial_scroll_offset_seeds_on_frame_enter() {
    let mut doc = Doc::new();
    let root = frame_sized(0.0, 0.0, 200.0, 200.0, Color::WHITE);
    let root_id = root.id;
    doc.apply(Operation::create_node(root)).unwrap();
    let mut container = CanvasNode::new(NodeData::Group(GroupNode {
        clip_size: Some([100.0, 100.0]),
        scroll_direction: Some(ScrollDirection::Vertical),
        scroll_offset: Some([0.0, 40.0]),
        ..GroupNode::default()
    }));
    let container_id = container.id;
    container.parent = Some(root_id);
    container.transform = Transform2D::translation(50.0, 50.0);
    doc.apply(Operation::create_node(container)).unwrap();
    colored_child(
        &mut doc,
        container_id,
        150.0,
        40.0,
        Color::rgb(0, 0, 255),
        ScrollBehavior::Scrolls,
    );

    let session = PresentSession::new(&doc, Some(root_id), DVec2::splat(200.0)).unwrap();
    assert_eq!(session.scroll_offset(container_id), [0.0, 40.0]);
}

#[test]
fn frame_without_authored_overflow_does_not_scroll() {
    // The legacy heuristic made every frame scrollable; authored axes say NO.
    let (mut doc, root, container) = scrolling_fixture(ScrollDirection::None);
    colored_child(
        &mut doc,
        container,
        150.0,
        40.0,
        Color::rgb(0, 0, 255),
        ScrollBehavior::Scrolls,
    );
    let mut session = PresentSession::new(&doc, Some(root), DVec2::splat(200.0)).unwrap();
    assert_eq!(session.scroll_by([60.0, 60.0], [0.0, 50.0]), None);
    session.set_scroll_offset(container, [0.0, 50.0]);
    assert_eq!(session.scroll_offset(container), [0.0, 0.0]);
}

// =============================================================================
// PR-5..PR-8: reaction completeness (multi-action sequences, out-transitions,
// spring easing, enter/leave/while-hovering triggers)
// =============================================================================

#[test]
fn pr5_multi_action_sets_the_variable_then_navigates_in_one_click() {
    // The canonical Figma pair: SetVariable + Navigate authored on ONE click.
    // Both must run — the navigate lands on the destination with the variable
    // already holding the new value.
    let coll_id = VariableCollectionId::from_u128(41);
    let mode = ModeId::from_u128(42);
    let var_id = VariableId::from_u128(43);
    let mut doc = Doc::new();
    doc.variables.collections.insert(
        coll_id,
        VariableCollection {
            id: coll_id,
            name: "Theme".into(),
            modes: vec![Mode {
                id: mode,
                name: "M".into(),
            }],
            default_mode: mode,
            variable_order: vec![var_id],
        },
    );
    doc.variables.variables.insert(
        var_id,
        Variable {
            id: var_id,
            collection: coll_id,
            name: "chosen".into(),
            ty: VariableType::Boolean,
            values_by_mode: BTreeMap::from([(mode, VarValue::Boolean { value: false })]),
            scopes: Vec::new(),
        },
    );

    let b = frame(500.0);
    let b_id = b.id;
    doc.apply(Operation::create_node(b)).unwrap();
    let mut a = frame(0.0);
    let a_id = a.id;
    a.reactions.push(Reaction {
        id: ReactionId::from_u128(400),
        trigger: Trigger::Click,
        action: Action::SetVariable {
            variable: var_id,
            value: VarValue::Boolean { value: true },
        },
        extra_actions: vec![Action::Navigate { to: b_id }],
        transition: None,
        animation: None,
    });
    doc.apply(Operation::create_node(a)).unwrap();

    let mut session = PresentSession::new(&doc, Some(a_id), DVec2::splat(200.0)).unwrap();
    let r = session.handle_pointer(DVec2::splat(100.0), PointerEvent::Click);
    assert!(r.navigated && r.needs_redraw, "both actions ran: {r:?}");
    assert_eq!(session.current_frame(), b_id, "second action navigated");
    assert_eq!(
        session
            .variables
            .variable(var_id)
            .unwrap()
            .value_for_mode(mode),
        Some(&VarValue::Boolean { value: true }),
        "first action's variable write is visible after the navigation"
    );
    // Playback stays ephemeral: the doc's own variable is untouched.
    assert_eq!(
        doc.variables.variable(var_id).unwrap().value_for_mode(mode),
        Some(&VarValue::Boolean { value: false })
    );
}

#[test]
fn pr5_actions_after_a_frame_change_do_not_fire() {
    // THE STOP RULE: a Navigate that changes the current frame ends the
    // sequence — actions after it were authored against the departed frame.
    let mut doc = Doc::new();
    let b = frame(500.0);
    let b_id = b.id;
    doc.apply(Operation::create_node(b)).unwrap();
    let mut a = frame(0.0);
    let a_id = a.id;
    a.reactions.push(Reaction {
        id: ReactionId::from_u128(401),
        trigger: Trigger::Click,
        action: Action::Navigate { to: b_id },
        extra_actions: vec![Action::OpenLink {
            url: "https://example.com/should-not-open".into(),
        }],
        transition: None,
        animation: None,
    });
    doc.apply(Operation::create_node(a)).unwrap();

    let mut session = PresentSession::new(&doc, Some(a_id), DVec2::splat(200.0)).unwrap();
    session.handle_pointer(DVec2::splat(100.0), PointerEvent::Click);
    assert_eq!(session.current_frame(), b_id);
    assert_eq!(
        session.take_open_url(),
        None,
        "actions sequenced after a frame change must not fire"
    );
}

#[test]
fn pr6_out_transitions_render_the_outgoing_frame_departing() {
    // SlideOut{Left}: the red source exits toward the left, revealing blue.
    // MoveOut{Down}: the red source exits downward over a pinned blue.
    // Mid-transition pixel probes prove it is the OUTGOING layer that moves.
    struct Probe {
        style: TransitionStyle,
        // (x, y) still covered by the departing red frame at e=0.5.
        red: (usize, usize),
        // (x, y) already revealing the blue destination at e=0.5.
        blue: (usize, usize),
    }
    let probes = [
        Probe {
            style: TransitionStyle::SlideOut {
                direction: Direction::Left,
            },
            red: (50, 100),
            blue: (150, 100),
        },
        Probe {
            style: TransitionStyle::MoveOut {
                direction: Direction::Down,
            },
            red: (100, 150),
            blue: (100, 50),
        },
    ];
    for (index, probe) in probes.into_iter().enumerate() {
        let mut doc = Doc::new();
        let destination = frame_colored(500.0, Color::rgb(0, 0, 255));
        let destination_id = destination.id;
        doc.apply(Operation::create_node(destination)).unwrap();
        let mut source = frame_colored(0.0, Color::rgb(255, 0, 0));
        let source_id = source.id;
        source.reactions.push(click_with(
            410 + index as u128,
            Action::Navigate { to: destination_id },
            Transition {
                style: probe.style,
                duration_ms: 100,
                easing: Easing::Linear,
            },
        ));
        doc.apply(Operation::create_node(source)).unwrap();

        let mut session = PresentSession::new(&doc, Some(source_id), DVec2::splat(200.0)).unwrap();
        let source_pixels = session.present_rgba();
        let destination_pixels =
            PresentSession::new(&doc, Some(destination_id), DVec2::splat(200.0))
                .unwrap()
                .present_rgba();
        let response = session.handle_pointer(DVec2::splat(100.0), PointerEvent::Click);
        assert!(
            response.navigated && response.animating,
            "{:?}",
            probe.style
        );
        assert_eq!(
            session.present_rgba(),
            source_pixels,
            "{:?} at t=0 still shows the source",
            probe.style
        );

        session.tick(0.05);
        let mid = session.present_rgba();
        let red = pixel_at(&mid, 200, probe.red.0, probe.red.1);
        assert!(
            red[0] > 200 && red[2] < 60,
            "{:?}: departing frame still covers {:?}, got {red:?}",
            probe.style,
            probe.red
        );
        let blue = pixel_at(&mid, 200, probe.blue.0, probe.blue.1);
        assert!(
            blue[2] > 200 && blue[0] < 60,
            "{:?}: destination revealed at {:?}, got {blue:?}",
            probe.style,
            probe.blue
        );

        let response = session.tick(0.05);
        assert!(!response.animating);
        assert_eq!(
            session.present_rgba(),
            destination_pixels,
            "{:?} settles exactly",
            probe.style
        );
    }
}

#[test]
fn pr6_scroll_animate_eases_the_offset_instead_of_jumping() {
    let (mut doc, root, container) = scrolling_fixture(ScrollDirection::Vertical);
    let child = colored_child(
        &mut doc,
        container,
        150.0,
        40.0,
        Color::rgb(0, 0, 255),
        ScrollBehavior::Scrolls,
    );
    doc.scene.get_mut(root).unwrap().reactions.push(click_with(
        420,
        Action::ScrollTo { target: child },
        Transition {
            style: TransitionStyle::ScrollAnimate,
            duration_ms: 100,
            easing: Easing::Linear,
        },
    ));

    let mut session = PresentSession::new(&doc, Some(root), DVec2::splat(200.0)).unwrap();
    let r = session.handle_pointer(DVec2::splat(100.0), PointerEvent::Click);
    assert!(r.animating, "scroll-animate is a timed animation: {r:?}");
    assert_eq!(
        session.scroll_offset(container),
        [0.0, 0.0],
        "no jump at t=0"
    );

    // Halfway toward the clamped destination (target centers at local y 170,
    // viewport center 50 → 120, clamped to max scroll 90; linear → 45).
    let r = session.tick(0.05);
    assert!(r.animating && r.needs_redraw);
    assert_eq!(session.scroll_offset(container), [0.0, 45.0]);

    // Completed: exactly the offset a plain (jumping) ScrollTo would set.
    let r = session.tick(0.05);
    assert!(!r.animating);
    assert_eq!(session.scroll_offset(container), [0.0, 90.0]);
}

#[test]
fn pr8_mouse_enter_fires_on_entry_only_and_reenters_after_leaving() {
    // MouseEnter rides the same once-per-entry machinery as Hover.
    let mut doc = Doc::new();
    let mut surface = frame(0.0);
    let surface_id = surface.id;
    surface.reactions.push(click(
        430,
        Trigger::MouseEnter,
        Action::OpenLink {
            url: "https://example.com/enter".into(),
        },
    ));
    doc.apply(Operation::create_node(surface)).unwrap();
    let mut session = PresentSession::new(&doc, Some(surface_id), DVec2::splat(200.0)).unwrap();

    session.handle_pointer(DVec2::splat(100.0), PointerEvent::Move);
    assert!(session.take_open_url().is_some(), "fires on entry");
    session.handle_pointer(DVec2::splat(100.0), PointerEvent::Move);
    assert!(
        session.take_open_url().is_none(),
        "does not repeat while inside"
    );
    session.handle_pointer(DVec2::new(-10.0, -10.0), PointerEvent::Move);
    session.handle_pointer(DVec2::splat(100.0), PointerEvent::Move);
    assert!(session.take_open_url().is_some(), "re-arms after leaving");
}

#[test]
fn pr8_mouse_leave_fires_on_exit_only() {
    // The PR-8 conflation fix, runtime side: a MouseLeave reaction stays
    // silent on entry and while inside, fires exactly once when the pointer
    // leaves the node's bounds, and re-arms on re-entry. Mirrors
    // `hover_fires_on_entry_only_and_reenters_after_leaving` on the opposite
    // edge.
    let mut doc = Doc::new();
    let mut surface = frame(0.0);
    let surface_id = surface.id;
    surface.reactions.push(click(
        431,
        Trigger::MouseLeave,
        Action::OpenLink {
            url: "https://example.com/leave".into(),
        },
    ));
    doc.apply(Operation::create_node(surface)).unwrap();
    let mut session = PresentSession::new(&doc, Some(surface_id), DVec2::splat(200.0)).unwrap();

    session.handle_pointer(DVec2::splat(100.0), PointerEvent::Move);
    assert!(session.take_open_url().is_none(), "silent on entry");
    session.handle_pointer(DVec2::splat(120.0), PointerEvent::Move);
    assert!(session.take_open_url().is_none(), "silent while inside");
    session.handle_pointer(DVec2::new(-10.0, -10.0), PointerEvent::Move);
    assert!(session.take_open_url().is_some(), "fires on the exit edge");
    session.handle_pointer(DVec2::new(-20.0, -20.0), PointerEvent::Move);
    assert!(session.take_open_url().is_none(), "fires once per exit");

    // Re-enter, then leave the whole surface (host Leave event): same edge.
    session.handle_pointer(DVec2::splat(100.0), PointerEvent::Move);
    assert!(session.take_open_url().is_none());
    session.handle_pointer(DVec2::splat(100.0), PointerEvent::Leave);
    assert!(
        session.take_open_url().is_some(),
        "surface leave is an exit too"
    );
}

#[test]
fn pr8_while_hovering_variant_applies_on_entry_and_reverts_on_leave() {
    // WhileHovering mirrors WhilePressing's press/release pattern on the
    // enter/leave edges: the variant swap applies while the pointer is inside
    // and is restored when it leaves.
    let (mut doc, frame_id, first_id, _) = interactive_variant_document();
    doc.scene.get_mut(first_id).unwrap().reactions[0].trigger = Trigger::WhileHovering;

    let mut session = PresentSession::new(&doc, Some(frame_id), DVec2::new(100.0, 50.0)).unwrap();
    let before = session.present_rgba();
    assert!(
        pixel_at(&before, 100, 25, 25)[0] > 200,
        "starts on Default (red)"
    );

    session.handle_pointer(DVec2::new(25.0, 25.0), PointerEvent::Move);
    let hovering = session.present_rgba();
    assert!(
        pixel_at(&hovering, 100, 25, 25)[1] > 200,
        "Hover variant (green) while inside"
    );

    // Move off the instance (still on the frame): the effect reverts.
    session.handle_pointer(DVec2::new(50.0, 48.0), PointerEvent::Move);
    let after = session.present_rgba();
    assert!(
        pixel_at(&after, 100, 25, 25)[0] > 200,
        "variant restored on leave"
    );
    assert!(!session.instance_variant_overrides.contains_key(&first_id));
}
