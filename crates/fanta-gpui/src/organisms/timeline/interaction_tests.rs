use gpui::{Modifiers, TestAppContext, point, px, size};

use crate::test_support::{assert_pointer_and_keyboard_parity, mount_component};

use super::*;

fn mount(cx: &mut TestAppContext) -> crate::test_support::Mounted<'_, Timeline, TimelineAction> {
    mount_component(cx, |_, cx| {
        Timeline::new("test-timeline", TimelineViewData::default(), cx)
    })
}

#[gpui::test]
fn transport_controls_share_pointer_enter_and_space_activation(cx: &mut TestAppContext) {
    let (_host, actions, cx) = mount(cx);

    assert_pointer_and_keyboard_parity(
        cx,
        "timeline-play",
        &actions,
        TimelineAction::PlayStateChangeRequested { playing: true },
    );
    assert_pointer_and_keyboard_parity(
        cx,
        "timeline-keyframe",
        &actions,
        TimelineAction::AddKeyframeRequested { time_ms: 0 },
    );
    assert_pointer_and_keyboard_parity(
        cx,
        "timeline-loop",
        &actions,
        TimelineAction::LoopChangeRequested { looping: false },
    );
    assert_pointer_and_keyboard_parity(
        cx,
        "timeline-help",
        &actions,
        TimelineAction::HelpRequested,
    );
}

#[gpui::test]
fn zoom_control_cycles_zoom_from_pointer_and_keyboard(cx: &mut TestAppContext) {
    let (_host, actions, cx) = mount(cx);

    assert_pointer_and_keyboard_parity(
        cx,
        "timeline-zoom",
        &actions,
        TimelineAction::ZoomChangeRequested { zoom: 0.75 },
    );
}

#[gpui::test]
fn rails_and_empty_card_compress_together_on_narrow_timelines(cx: &mut TestAppContext) {
    let (host, actions, cx) = mount(cx);

    // Wide: both rails and the zoom cluster hold the reference design width.
    cx.simulate_resize(size(px(1200.), px(333.)));
    cx.run_until_parked();
    let transport = cx
        .debug_bounds("timeline-transport-rail")
        .expect("transport rail should render");
    let track = cx
        .debug_bounds("timeline-track-rail")
        .expect("track rail should render");
    assert_eq!(f32::from(transport.size.width), RAIL_WIDTH);
    assert_eq!(f32::from(track.size.width), RAIL_WIDTH);
    assert_eq!(
        cx.debug_bounds("timeline-zoom-cluster")
            .expect("zoom cluster should render")
            .size
            .width,
        px(ZOOM_WIDTH)
    );
    assert_eq!(
        cx.debug_bounds("timeline-zoom")
            .expect("zoom slider should render")
            .size
            .width,
        px(ZOOM_TRACK_WIDTH)
    );

    // Mid: the rail floors first, then the zoom cluster and its slider
    // track compress from the measured strip width.
    cx.simulate_resize(size(px(460.), px(300.)));
    cx.run_until_parked();
    cx.run_until_parked();
    let strip = cx.read(|app| {
        host.read(app)
            .component
            .read(app)
            .strip_width
            .expect("the strip should be measured after a draw")
    });
    assert_eq!(
        cx.debug_bounds("timeline-transport-rail")
            .expect("transport rail should render at 460")
            .size
            .width,
        px(RAIL_MIN_WIDTH)
    );
    assert_eq!(
        cx.debug_bounds("timeline-zoom-cluster")
            .expect("compressed zoom cluster should render")
            .size
            .width,
        px(strip - RAIL_MIN_WIDTH - RULER_MIN_WIDTH)
    );
    assert_eq!(
        cx.debug_bounds("timeline-zoom")
            .expect("compressed zoom slider should render")
            .size
            .width,
        px(strip - RAIL_MIN_WIDTH - RULER_MIN_WIDTH - ZOOM_TRACK_CHROME)
    );

    // Narrow: the slider collapses to the icon-only trigger, its freed width
    // flows back to the shared rail, and the empty-state card caps to the
    // available width instead of overflowing.
    cx.simulate_resize(size(px(400.), px(300.)));
    cx.run_until_parked();
    cx.run_until_parked();
    let strip = cx.read(|app| {
        host.read(app)
            .component
            .read(app)
            .strip_width
            .expect("the strip should stay measured when narrow")
    });
    let transport = cx
        .debug_bounds("timeline-transport-rail")
        .expect("transport rail should render when narrow");
    let track = cx
        .debug_bounds("timeline-track-rail")
        .expect("track rail should render when narrow");
    assert_eq!(
        f32::from(transport.size.width),
        strip - ZOOM_COMPACT_WIDTH - RULER_MIN_WIDTH,
        "the collapsed zoom cluster must hand its width back to the rail"
    );
    assert_eq!(
        f32::from(transport.size.width),
        f32::from(track.size.width),
        "both rows must share one rail width so their border stays aligned"
    );
    let cluster = cx
        .debug_bounds("timeline-zoom-cluster")
        .expect("icon-only zoom cluster should render");
    assert_eq!(f32::from(cluster.size.width), ZOOM_COMPACT_WIDTH);
    let trigger = cx
        .debug_bounds("timeline-zoom")
        .expect("icon-only zoom trigger should render");
    assert!(
        trigger.is_contained_within(&cluster),
        "the compact trigger must stay inside its cluster"
    );

    // The compact trigger keeps the zoom-cycle activation.
    actions.borrow_mut().clear();
    cx.simulate_click(trigger.center(), Modifiers::none());
    cx.run_until_parked();
    assert_eq!(
        actions.borrow().as_slice(),
        &[TimelineAction::ZoomChangeRequested { zoom: 0.75 }]
    );

    let card = cx
        .debug_bounds("timeline-empty-card")
        .expect("empty-state card should render when narrow");
    assert!(
        f32::from(card.size.width) < EMPTY_CARD_WIDTH,
        "the card must cap below its design width, got {:?}",
        card.size.width
    );
    assert!(
        f32::from(card.right()) <= 400.,
        "the card must stay inside the timeline, got {:?}",
        card.right()
    );

    // At the exported floor the rail and ruler both hold their minimums.
    cx.simulate_resize(size(px(TIMELINE_MIN_WIDTH), px(300.)));
    cx.run_until_parked();
    cx.run_until_parked();
    assert_eq!(
        cx.debug_bounds("timeline-transport-rail")
            .expect("transport rail should render at the floor")
            .size
            .width,
        px(RAIL_MIN_WIDTH)
    );
    assert_eq!(
        cx.debug_bounds("timeline-ruler")
            .expect("ruler should render at the floor")
            .size
            .width,
        px(RULER_MIN_WIDTH)
    );
}

#[gpui::test]
fn empty_state_controls_emit_agent_and_dismiss_intents(cx: &mut TestAppContext) {
    let (_host, actions, cx) = mount(cx);

    assert_pointer_and_keyboard_parity(
        cx,
        "timeline-ask-agent",
        &actions,
        TimelineAction::AskAgentRequested,
    );

    // Dismissal mutates component-local state, so assert the single pointer
    // path directly instead of using the parity matrix.
    let bounds = cx
        .debug_bounds("timeline-dismiss-empty")
        .expect("dismiss control should render");
    cx.simulate_click(bounds.center(), Modifiers::none());
    cx.run_until_parked();
    assert_eq!(
        actions.borrow().as_slice(),
        &[TimelineAction::EmptyStateDismissed]
    );
    assert!(
        cx.debug_bounds("timeline-dismiss-empty").is_none(),
        "empty state should hide after dismissal"
    );
}

#[gpui::test]
fn ruler_click_seeks_to_the_time_under_the_tick(cx: &mut TestAppContext) {
    let (host, actions, cx) = mount(cx);

    // Dismiss the empty-state overlay so it cannot intercept ruler clicks.
    let dismiss = cx
        .debug_bounds("timeline-dismiss-empty")
        .expect("dismiss control should render");
    cx.simulate_click(dismiss.center(), Modifiers::none());
    cx.run_until_parked();
    actions.borrow_mut().clear();

    let (segment_width, duration_ms) = cx.read(|app| {
        let timeline = host.read(app).component.read(app);
        (timeline.segment_width(), timeline.view_data.duration_ms)
    });
    let ruler = cx
        .debug_bounds("timeline-ruler")
        .expect("ruler should render");

    // Click exactly under the 30% tick position in ruler content coordinates.
    let target = point(
        ruler.left() + px(RULER_GUTTER) + px(segment_width * 3.),
        ruler.center().y,
    );
    cx.simulate_click(target, Modifiers::none());
    cx.run_until_parked();

    let expected = (duration_ms as f32 * 0.3).round() as u32;
    assert_eq!(
        actions.borrow().as_slice(),
        &[TimelineAction::SeekRequested { time_ms: expected }]
    );
}
