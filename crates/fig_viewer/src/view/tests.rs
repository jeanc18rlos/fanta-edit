use super::*;
use crate::motion_edit::{motion_property, motion_value};
use std::collections::BTreeMap;

use fanta_doc::{
    AnimationTrack, AnimationTrackId, CanvasNode, Color, GroupNode, Mode, ModeId, MotionTarget,
    NodeData, Operation, TextNode, Transform2D, VarValue, Variable, VariableCollection,
    VariableCollectionId, VariableId, VariableType, VectorNode,
};
use gpui::{TestAppContext, point, size};
use project::FakeFs;
use settings::SettingsStore;

#[cfg(target_os = "macos")]
#[test]
fn video_fill_assets_collects_distinct_sources_from_shape_and_frame_paints() {
    let first = AssetId::new();
    let second = AssetId::new();
    let make_fill = |asset| Fill::Video {
        video: Box::new(fanta_doc::VideoFill {
            asset,
            poster: None,
            mode: fanta_doc::ImageFitMode::Fill,
            crop: None,
            scale: None,
            rotation: None,
            adjust: fanta_doc::ImageAdjust::default(),
        }),
        opacity: 1.0,
        blend: fanta_doc::BlendMode::Normal,
    };
    let mut vector = VectorNode::rect_solid(0.0, 0.0, 40.0, 40.0, Color::BLACK);
    vector.fills.clear();
    vector.fills.push(make_fill(first));
    vector.fills.push(make_fill(second));
    vector.strokes.push(fanta_doc::Stroke {
        paint: make_fill(first),
        ..fanta_doc::Stroke::solid(Color::BLACK, 1.0)
    });
    let shape = CanvasNode::new(NodeData::Vector(vector));
    let mut actual = video_fill_assets(&shape);
    actual.sort_unstable();
    let mut expected = vec![first, second];
    expected.sort_unstable();
    assert_eq!(actual, expected);

    let frame = CanvasNode::new(NodeData::Group(GroupNode {
        background: Some(make_fill(first)),
        background_fills: [make_fill(second)].into_iter().collect(),
        ..GroupNode::default()
    }));
    assert_eq!(video_fill_assets(&frame), expected);
}

#[cfg(target_os = "macos")]
#[gpui::test]
async fn video_fill_sessions_are_per_asset_and_stale_loads_cannot_reopen_them(
    cx: &mut TestAppContext,
) {
    let (_directory, _, item, view) = autosave_fixture(cx).await;
    let assets = [AssetId::new(), AssetId::new()];
    item.update(cx, |item, cx| {
        item.with_document(cx, |document| {
            let mut vector = VectorNode::rect_solid(0.0, 0.0, 40.0, 40.0, Color::BLACK);
            vector.fills.clear();
            for asset in assets {
                vector.fills.push(Fill::Video {
                    video: Box::new(fanta_doc::VideoFill {
                        asset,
                        poster: None,
                        mode: fanta_doc::ImageFitMode::Fill,
                        crop: None,
                        scale: None,
                        rotation: None,
                        adjust: fanta_doc::ImageAdjust::default(),
                    }),
                    opacity: 1.0,
                    blend: fanta_doc::BlendMode::Normal,
                });
            }
            let mut shape = CanvasNode::new(NodeData::Vector(vector));
            shape.parent = document.doc.active_page();
            let id = shape.id;
            document
                .doc
                .apply(Operation::create_node(shape))
                .expect("create video fill shape");
            document.doc.selection.replace_with([id]);
            ((), DocChange::Content)
        });
    });
    view.update(cx, |view, cx| {
        for asset in assets {
            view.ensure_video_fill_playback(asset, cx);
        }
        assert!(view.canvas_video_fills.is_empty());
        view.set_editor_mode(EditorMode::Motion, cx);
    });
    view.update(cx, |view, cx| {
        let sources = view.selected_canvas_video_fill_sources(cx);
        assert_eq!(sources.len(), 2);
        for source in sources {
            view.ensure_video_fill_playback(source.asset, cx);
        }
        assert_eq!(view.canvas_video_fills.len(), 2);
        assert_ne!(
            view.canvas_video_fills[&assets[0]].generation,
            view.canvas_video_fills[&assets[1]].generation
        );
    });
    item.update(cx, |item, cx| {
        item.with_document(cx, |document| {
            document.doc.selection.clear();
            ((), DocChange::Selection)
        });
    });
    view.update(cx, |view, cx| view.clear_canvas_video_fills(cx));
    cx.run_until_parked();
    view.read_with(cx, |view, cx| {
        assert!(view.canvas_video_fills.is_empty());
        assert!(view.canvas_video_fill_frames(cx).is_empty());
    });
}

#[cfg(target_os = "macos")]
async fn canvas_video_fixture(
    cx: &mut TestAppContext,
) -> (tempfile::TempDir, Entity<FigItem>, Entity<FigView>) {
    let (directory, _, item, view) = autosave_fixture(cx).await;
    item.update(cx, |item, cx| {
        item.with_document(cx, |document| {
            let mut video = CanvasNode::new(NodeData::Video(fanta_doc::VideoNode {
                asset: AssetId::new(),
                natural_size: [16, 16],
                local_size: [80., 80.],
                time_range_us: [0, 3_000_000],
                speed: 1.,
                muted: true,
                volume: 1.,
                poster_frame_us: None,
                poster: None,
                fit: fanta_doc::ImageFitMode::Fit,
            }));
            video.parent = document.doc.active_page();
            let node = video.id;
            document
                .doc
                .apply(Operation::create_node(video))
                .expect("create video");
            document.doc.selection.replace_with([node]);
            ((), DocChange::Selection)
        });
    });
    view.update(cx, |view, cx| view.set_editor_mode(EditorMode::Motion, cx));
    (directory, item, view)
}

#[cfg(target_os = "macos")]
async fn video_trim_view_fixture(
    cx: &mut TestAppContext,
) -> (
    tempfile::TempDir,
    Entity<FigItem>,
    gpui::WindowHandle<FigView>,
) {
    let (directory, item, previous_view) = canvas_video_fixture(cx).await;
    let project = previous_view.read_with(cx, |view, _| view.project.clone());
    let playback = cx.update(crate::video_playback::fake_playback);
    cx.run_until_parked();
    let window = cx.add_window(|window, cx| FigView::new(item.clone(), project, window, cx));
    window
        .update(cx, |view, _, cx| {
            view.set_editor_mode(EditorMode::Motion, cx)
        })
        .expect("set motion mode");
    window
        .update(cx, |_, window, _| window.activate_window())
        .expect("activate trim window");
    cx.run_until_parked();
    window
        .update(cx, |view, window, cx| {
            playback.update(cx, |playback, cx| playback.tick(window, cx));
            let (source, muted, volume) = view.selected_canvas_video_source(cx).expect("video");
            view.canvas_video = Some(CanvasVideoSession {
                source,
                loading: None,
                playback: Some(playback),
                observation: None,
                error: None,
                audio: (muted, volume.to_bits()),
                bytes: Some(std::sync::Arc::from(&b"original MP4 bytes"[..])),
                trim: None,
            });
            view.begin_video_trim(window, cx);
            let trim = view
                .canvas_video
                .as_ref()
                .and_then(|session| session.trim.as_ref())
                .expect("trim controls");
            trim.start
                .update(cx, |input, cx| input.set_text("0.5", window, cx));
            trim.end
                .update(cx, |input, cx| input.set_text("2", window, cx));
        })
        .expect("trim controls");
    (directory, item, window)
}

#[cfg(target_os = "macos")]
fn prepared_trim_for_view(range_us: [i64; 2]) -> crate::generation_media::PreparedVideoTrim {
    let mut png = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
        2,
        2,
        image::Rgba([225, 0, 225, 255]),
    ))
    .write_to(&mut png, image::ImageFormat::Png)
    .expect("poster");
    crate::generation_media::PreparedVideoTrim {
        range_us,
        source_duration_us: 3_000_000,
        poster: crate::generation_media::VideoPoster {
            png: png.into_inner().into(),
            time_us: range_us[0],
        },
    }
}

#[cfg(target_os = "macos")]
#[gpui::test]
async fn video_trim_apply_waits_for_poster_and_commits_one_history_step(
    cx: &mut TestAppContext,
) {
    let (_directory, item, window) = video_trim_view_fixture(cx).await;
    let before = item.read_with(cx, |item, _| {
        let doc = &item.document().expect("document").doc;
        let id = single_selection(doc).expect("video");
        (
            id,
            doc.scene.get(id).expect("video").clone(),
            doc.history.undo_depth(),
        )
    });
    let (send, receive) = futures::channel::oneshot::channel();
    window
        .update(cx, |view, _, cx| {
            view.apply_video_trim_with(
                |bytes, range| {
                    assert_eq!(bytes.as_ref(), b"original MP4 bytes");
                    assert_eq!(range, [500_000, 2_000_000]);
                    Box::pin(async move { receive.await.context("poster response")? })
                },
                cx,
            )
        })
        .expect("apply");
    cx.run_until_parked();
    item.read_with(cx, |item, _| {
        let doc = &item.document().expect("document").doc;
        assert_eq!(
            doc.scene.get(before.0),
            Some(&before.1),
            "no partial range mutation before the poster"
        );
        assert_eq!(doc.history.undo_depth(), before.2);
    });
    send.send(Ok(prepared_trim_for_view([500_000, 2_000_000])))
        .unwrap_or_else(|_| panic!("pending poster"));
    cx.run_until_parked();
    item.update(cx, |item, cx| {
        let doc = &item.document().expect("document").doc;
        let NodeData::Video(video) = &doc.scene.get(before.0).expect("video").data else {
            panic!("video")
        };
        assert_eq!(video.time_range_us, [500_000, 2_000_000]);
        assert_eq!(video.poster_frame_us, Some(500_000));
        assert!(video.poster.is_some());
        assert_eq!(doc.history.undo_depth(), before.2 + 1);
        assert!(item.undo(cx).expect("undo trim"));
        assert_eq!(
            item.document().expect("document").doc.scene.get(before.0),
            Some(&before.1)
        );
        assert!(item.redo(cx).expect("redo trim"));
        assert!(item.is_dirty());
    });
}

#[cfg(target_os = "macos")]
#[gpui::test]
async fn video_trim_cancel_error_timeout_and_stale_target_preserve_document(
    cx: &mut TestAppContext,
) {
    for case in [
        "cancel",
        "error",
        "timeout",
        "deactivate",
        "remove",
        "source-change",
        "input-change",
    ] {
        let (_directory, item, window) = video_trim_view_fixture(cx).await;
        let before = item.read_with(cx, |item, _| {
            let document = item.document().expect("document");
            let id = single_selection(&document.doc).expect("video");
            (
                id,
                document.doc.scene.get(id).expect("video").clone(),
                document.raw_assets.clone(),
                document.doc.history.undo_depth(),
            )
        });
        let (send, receive) = futures::channel::oneshot::channel::<
            Result<crate::generation_media::PreparedVideoTrim>,
        >();
        let (source, generation) = window
            .update(cx, |view, _, cx| {
                view.apply_video_trim_with(
                    |_, _| Box::pin(async move { receive.await.context("poster response")? }),
                    cx,
                );
                (
                    view.canvas_video.as_ref().expect("session").source,
                    view.canvas_video_generation,
                )
            })
            .expect("apply");
        cx.run_until_parked();
        match case {
            "cancel" => window
                .update(cx, |view, _, cx| view.cancel_video_trim(cx))
                .expect("cancel"),
            "deactivate" => window
                .update(cx, |view, _, cx| view.set_canvas_video_active(false, cx))
                .expect("deactivate"),
            "remove" => window
                .update(cx, |view, _, cx| Item::on_removed(view, cx))
                .expect("remove"),
            "timeout" => {
                cx.executor()
                    .advance_clock(std::time::Duration::from_secs(21));
                cx.run_until_parked();
            }
            "input-change" => window
                .update(cx, |view, window, cx| {
                    let trim = view
                        .canvas_video
                        .as_ref()
                        .and_then(|session| session.trim.as_ref())
                        .expect("trim");
                    trim.start
                        .update(cx, |input, cx| input.set_text("1", window, cx));
                })
                .expect("new trim time"),
            "source-change" => item.update(cx, |item, cx| {
                item.with_document(cx, |document| {
                    document.doc.selection.clear();
                    ((), DocChange::Selection)
                });
            }),
            "error" => {
                send.send(Err(anyhow::anyhow!("injected poster decode failure")))
                    .unwrap_or_else(|_| panic!("pending poster"));
                cx.run_until_parked();
            }
            _ => unreachable!(),
        }
        if case != "error" && case != "timeout" {
            // Even an already queued completion must not mutate a cancelled or rebound view.
            window
                .update(cx, |view, _, cx| {
                    view.finish_video_trim(
                        source,
                        generation,
                        Ok(prepared_trim_for_view([500_000, 2_000_000])),
                        cx,
                    )
                })
                .expect("late completion");
        }
        if matches!(case, "error" | "timeout" | "input-change") {
            window
                .update(cx, |view, _, _| {
                    let trim = view
                        .canvas_video
                        .as_ref()
                        .and_then(|session| session.trim.as_ref())
                        .expect("retryable trim inputs");
                    assert!(trim.error.is_some(), "{case}");
                    assert!(trim.task.borrow().is_none(), "{case}");
                })
                .expect("visible failure");
        }
        item.read_with(cx, |item, _| {
            let document = item.document().expect("document");
            assert_eq!(document.doc.scene.get(before.0), Some(&before.1), "{case}");
            assert_eq!(document.raw_assets, before.2, "{case}");
            assert_eq!(document.doc.history.undo_depth(), before.3, "{case}");
        });
    }
}

#[cfg(target_os = "macos")]
#[gpui::test]
async fn video_trim_controls_open_cancel_and_fit_the_canvas_footer(cx: &mut TestAppContext) {
    let (_directory, item, window) = video_trim_view_fixture(cx).await;
    let view = window.entity(cx).expect("view");
    let depth = item.read_with(cx, |item, _| {
        item.document().expect("document").doc.history.undo_depth()
    });
    let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
    for width in [1_000., 1_200.] {
        visual.simulate_resize(size(px(width), px(900.)));
        visual.update(|window, cx| window.draw(cx).clear());
        let footer = visual
            .debug_bounds("canvas-video-controls")
            .expect("video controls");
        for selector in [
            "video-trim-start",
            "video-trim-end",
            "video-trim-cancel-target",
        ] {
            let bounds = visual.debug_bounds(selector).expect("visible trim control");
            assert!(
                bounds.left() >= footer.left() && bounds.right() <= footer.right(),
                "{selector}: {bounds:?} outside {footer:?}"
            );
            assert!(
                bounds.top() >= footer.top() && bounds.bottom() <= footer.bottom(),
                "{selector}"
            );
        }
    }
    let cancel = visual
        .debug_bounds("video-trim-cancel-target")
        .expect("cancel");
    visual.simulate_click(cancel.center(), gpui::Modifiers::none());
    visual.update(|window, cx| window.draw(cx).clear());
    view.read_with(&visual, |view, _| {
        assert!(view.canvas_video.as_ref().expect("session").trim.is_none())
    });
    let open = visual
        .debug_bounds("video-trim-open-target")
        .expect("trim button");
    visual.simulate_click(open.center(), gpui::Modifiers::none());
    visual.update(|window, cx| window.draw(cx).clear());
    assert!(visual.debug_bounds("video-trim-start").is_some());
    item.read_with(&visual, |item, _| {
        assert_eq!(
            item.document().expect("document").doc.history.undo_depth(),
            depth
        )
    });
}

#[cfg(target_os = "macos")]
#[gpui::test]
async fn video_trim_invalid_input_never_starts_preparation(cx: &mut TestAppContext) {
    let (_directory, item, window) = video_trim_view_fixture(cx).await;
    let before = item.read_with(cx, |item, _| {
        item.document().expect("document").doc.history.undo_depth()
    });
    for (start, end) in [
        ("NaN", "2"),
        ("-1", "2"),
        ("2", "2"),
        ("2", "1"),
        ("0", "11"),
    ] {
        window
            .update(cx, |view, window, cx| {
                let trim = view
                    .canvas_video
                    .as_ref()
                    .and_then(|session| session.trim.as_ref())
                    .expect("trim");
                trim.start
                    .update(cx, |input, cx| input.set_text(start, window, cx));
                trim.end
                    .update(cx, |input, cx| input.set_text(end, window, cx));
                view.apply_video_trim_with(|_, _| panic!("invalid input reached decoder"), cx);
                let trim = view
                    .canvas_video
                    .as_ref()
                    .and_then(|session| session.trim.as_ref())
                    .expect("trim");
                assert!(trim.error.is_some());
                assert!(trim.task.borrow().is_none());
            })
            .expect("validate input");
    }
    assert_eq!(
        item.read_with(cx, |item, _| item
            .document()
            .expect("document")
            .doc
            .history
            .undo_depth()),
        before
    );
}

#[cfg(target_os = "macos")]
#[test]
fn video_trim_decimal_times_preserve_microseconds_without_float_rounding() {
    for (text, expected) in [
        ("0", 0),
        ("1.25", 1_250_000),
        ("0.000001", 1),
        ("9223372036854.775807", i64::MAX),
    ] {
        assert_eq!(parse_video_trim_time(text).expect("valid time"), expected);
        assert_eq!(
            parse_video_trim_time(&video_trim_time_text(expected)).expect("round trip"),
            expected
        );
    }
    for text in ["NaN", "-0.1", "1e3", "1.0000001", "9223372036854.775808"] {
        assert!(parse_video_trim_time(text).is_err(), "{text}");
    }
}

#[cfg(target_os = "macos")]
#[gpui::test]
async fn video_trim_authored_range_starts_canvas_loading(cx: &mut TestAppContext) {
    let (_directory, item, view) = canvas_video_fixture(cx).await;
    item.update(cx, |item, cx| {
        item.with_document(cx, |document| {
            let id = single_selection(&document.doc).expect("selected video");
            let old = document.doc.scene.get(id).expect("video").data.clone();
            let mut new = old.clone();
            let NodeData::Video(video) = &mut new else {
                panic!("video")
            };
            video.time_range_us = [500_000, 2_000_000];
            document
                .doc
                .apply(Operation::ReplaceData {
                    id,
                    old: Box::new(old),
                    new: Box::new(new),
                })
                .expect("author source range");
            ((), DocChange::Content)
        });
    });
    view.update(cx, |view, cx| {
        view.sync_canvas_video(true, cx);
        let session = view.canvas_video.as_ref().expect("video session");
        assert!(
            session.error.is_none(),
            "valid authored trim must be playable"
        );
        assert!(
            session.loading.is_some(),
            "load the original source without rewriting it"
        );
        view.clear_canvas_video(cx);
    });
}

#[cfg(target_os = "macos")]
#[gpui::test]
async fn canvas_video_controls_only_show_in_motion_and_stay_below_toolbar(
    cx: &mut TestAppContext,
) {
    init_visual_test(cx);
    #[cfg(feature = "fanta-gpui-ui")]
    cx.update(|cx| {
        gpui_component::init(cx);
        fanta_gpui::init(cx);
        crate::theme_bridge::init(cx);
    });
    let project = Project::test(FakeFs::new(cx.executor()), [], cx).await;
    let mut document = doc_with_one_page();
    let mut video = CanvasNode::new(NodeData::Video(fanta_doc::VideoNode {
        asset: AssetId::new(),
        natural_size: [16, 16],
        local_size: [80., 80.],
        time_range_us: [0, 10_000_000],
        speed: 1.,
        muted: true,
        volume: 1.,
        poster_frame_us: None,
        poster: None,
        fit: fanta_doc::ImageFitMode::Fit,
    }));
    video.parent = document.active_page();
    let node = video.id;
    document
        .apply(Operation::create_node(video))
        .expect("video");
    document.selection.replace_with([node]);
    let item = crate::document::ready_item_for_test(
        &project,
        std::path::PathBuf::from("/tmp/Video-layout.fig"),
        document,
        cx,
    );
    let window = cx.add_window(move |window, cx| FigView::new(item, project, window, cx));
    let view = window.entity(cx).expect("fig view");
    #[cfg(feature = "fanta-gpui-ui")]
    assert!(view.read_with(cx, |view, _| view.gpui_toolbar.is_some()));
    let playback = cx.update(crate::video_playback::fake_playback);
    cx.run_until_parked();
    view.update(cx, |view, cx| {
        let (source, muted, volume) = view.selected_canvas_video_source(cx).expect("video");
        view.canvas_video = Some(CanvasVideoSession {
            source,
            loading: None,
            playback: Some(playback),
            observation: None,
            error: None,
            audio: (muted, volume.to_bits()),
            bytes: None,
            trim: None,
        });
    });
    window
        .update(cx, |view, window, cx| {
            assert!(view.render_canvas_video_controls(window, cx).is_none());
            view.set_editor_mode(EditorMode::Motion, cx);
        })
        .expect("show motion controls");
    let mut visual_context = gpui::VisualTestContext::from_window(window.into(), cx);
    for width in [1_000., 1_400.] {
        visual_context.simulate_resize(size(px(width), px(800.)));
        visual_context.update(|window, cx| window.draw(cx).clear());
        let canvas = visual_context
            .debug_bounds("fig-container")
            .expect("canvas");
        let controls = visual_context
            .debug_bounds("canvas-video-controls")
            .expect("controls");
        let toolbar = visual_context
            .debug_bounds("fanta-canvas-toolbar")
            .expect("toolbar");
        assert!(
            toolbar.bottom() <= canvas.bottom(),
            "toolbar must stay in the canvas: {toolbar:?} vs {canvas:?}"
        );
        assert!(
            toolbar.bottom() < controls.top(),
            "toolbar covers video controls: {toolbar:?} vs {controls:?}"
        );
    }
}

#[cfg(target_os = "macos")]
#[gpui::test]
async fn canvas_video_rejects_late_loading_and_reports_missing_sources(
    cx: &mut TestAppContext,
) {
    let (_directory, item, view) = canvas_video_fixture(cx).await;
    let (source, generation) = view.update(cx, |view, cx| {
        view.sync_canvas_video(true, cx);
        (
            view.canvas_video.as_ref().expect("loading video").source,
            view.canvas_video_generation,
        )
    });
    cx.run_until_parked();
    view.read_with(cx, |view, _| {
        let session = view.canvas_video.as_ref().expect("failed video");
        assert!(session.playback.is_none());
        assert!(session.loading.is_none());
        assert!(
            session
                .error
                .as_ref()
                .expect("visible error")
                .contains("missing")
        );
    });
    item.update(cx, |item, cx| {
        item.with_document(cx, |document| {
            document.doc.selection.replace_with([]);
            ((), DocChange::Selection)
        });
    });
    cx.run_until_parked();
    view.update(cx, |view, cx| {
        assert!(view.canvas_video.is_none());
        view.finish_canvas_video_load(
            source,
            generation,
            Ok(std::sync::Arc::from([1_u8].as_slice())),
            cx,
        );
        assert!(
            view.canvas_video.is_none(),
            "old source cannot install after deselection"
        );
    });
}

#[cfg(target_os = "macos")]
#[gpui::test]
async fn canvas_video_removal_closes_retained_player_without_editing_document(
    cx: &mut TestAppContext,
) {
    let (_directory, item, view) = canvas_video_fixture(cx).await;
    let before = item.read_with(cx, |item, _cx| {
        (
            serde_json::to_value(&item.document().expect("document").doc).expect("serialize"),
            item.is_dirty(),
        )
    });
    let playback = cx.update(crate::video_playback::fake_playback);
    cx.run_until_parked();
    let original_revision = playback.read_with(cx, |playback, _| playback.frame_revision());
    view.update(cx, |view, cx| {
        let (source, muted, volume) = view
            .selected_canvas_video_source(cx)
            .expect("selected video");
        view.canvas_video = Some(CanvasVideoSession {
            source,
            loading: None,
            playback: Some(playback.clone()),
            observation: None,
            error: None,
            audio: (muted, volume.to_bits()),
            bytes: None,
            trim: None,
        });
        Item::on_removed(view, cx);
        assert!(view.canvas_video_frame(cx).is_none());
        view.finish_canvas_video_load(
            source,
            view.canvas_video_generation,
            Ok(std::sync::Arc::from([1_u8].as_slice())),
            cx,
        );
        assert_eq!(
            view.canvas_video
                .as_ref()
                .and_then(|session| session.playback.as_ref())
                .map(Entity::entity_id),
            Some(playback.entity_id())
        );
    });
    playback.read_with(cx, |playback, _| {
        assert!(playback.frame().is_none());
        assert!(
            playback.frame_revision() > original_revision,
            "retained player was closed"
        );
    });
    let project = view.read_with(cx, |view, _| view.project.clone());
    let scratch = cx.add_window(|_, _| gpui::Empty);
    scratch
        .update(cx, |_, window, cx| {
            let workspace = cx.new(|cx| workspace::Workspace::test_new(project, window, cx));
            workspace.update(cx, |workspace, cx| {
                view.update(cx, |view, cx| {
                    Item::added_to_workspace(view, workspace, window, cx);
                    assert!(
                        view.canvas_video.is_none(),
                        "moving a tab must discard its closed player"
                    );
                    assert!(!view.canvas_video_removed.get());
                    view.sync_canvas_video(true, cx);
                    assert!(
                        view.canvas_video
                            .as_ref()
                            .is_some_and(|session| session.loading.is_some()),
                        "re-added tab can prepare the selected source again"
                    );
                });
            });
        })
        .expect("re-add the same canvas tab");
    cx.run_until_parked();
    item.read_with(cx, |item, _| {
        assert_eq!(
            serde_json::to_value(&item.document().expect("document").doc).expect("serialize"),
            before.0
        );
        assert_eq!(item.is_dirty(), before.1);
    });
}

#[cfg(target_os = "macos")]
#[test]
fn video_trim_source_validation_accepts_bounded_ranges_and_rejects_unknown_duration() {
    assert!(canvas_video_duration_supported([0, 3_000_000], 3_000_000));
    assert!(canvas_video_duration_supported(
        [1_000_000, 3_000_000],
        3_000_000
    ));
    assert!(canvas_video_duration_supported([0, 2_000_000], 3_000_000));
    assert!(!canvas_video_duration_supported([0, 4_000_000], 3_000_000));
    assert!(!canvas_video_duration_supported([-1, 2_000_000], 3_000_000));
    assert!(!canvas_video_duration_supported(
        [2_000_000, 2_000_000],
        3_000_000
    ));
    assert!(!canvas_video_duration_supported([0, -1], 3_000_000));
    assert!(!canvas_video_duration_supported([0, 3_000_000], 0));
}

#[test]
fn prototype_links_allow_web_urls_and_block_local_or_executable_schemes() {
    assert!(validate_prototype_link("https://example.com/design").is_ok());
    assert!(validate_prototype_link("http://example.com").is_ok());
    assert!(validate_prototype_link("file:///tmp/private").is_err());
    assert!(validate_prototype_link("javascript:alert(1)").is_err());
    assert!(validate_prototype_link("not a url").is_err());
}

#[test]
fn prototype_clock_uses_measured_elapsed_time_and_resets_cleanly() {
    let start = std::time::Instant::now();
    let mut last_tick = Some(start);
    assert_eq!(
        prototype_tick_elapsed(&mut last_tick, start + std::time::Duration::from_millis(73)),
        std::time::Duration::from_millis(73)
    );
    last_tick = None;
    assert_eq!(
        prototype_tick_elapsed(&mut last_tick, start),
        std::time::Duration::from_millis(16)
    );
}

fn init_test(cx: &mut TestAppContext) {
    cx.update(|cx| {
        let settings_store = SettingsStore::test(cx);
        cx.set_global(settings_store);
    });
}

fn init_visual_test(cx: &mut TestAppContext) {
    cx.update(|cx| {
        zlog::init_test();
        assets::Assets.load_test_fonts(cx);
        let settings_store = SettingsStore::test(cx);
        cx.set_global(settings_store);
        theme_settings::init(theme::LoadThemes::JustBase, cx);
        release_channel::init(semver::Version::new(0, 0, 0), cx);
        editor::init(cx);
        #[cfg(feature = "fanta-gpui-ui")]
        {
            gpui_component::init(cx);
            fanta_gpui::init(cx);
            crate::theme_bridge::init(cx);
        }
    });
}

fn doc_with_one_page() -> fanta_doc::Doc {
    let mut doc = fanta_doc::Doc::new();
    let mut page = CanvasNode::new(NodeData::Group(GroupNode::default()));
    page.name = "Page 1".to_owned();
    let root = page.id;
    doc.apply(Operation::create_node(page))
        .expect("create page root node");
    doc.add_page(root);
    doc.set_active_page(Some(root));
    doc
}

fn text_selection_doc(wrapped: bool) -> (fanta_doc::Doc, NodeId, Option<NodeId>) {
    let mut doc = doc_with_one_page();
    let page = doc.active_page().expect("active page");
    let frame = wrapped.then(|| {
        let mut frame = CanvasNode::new(NodeData::Group(GroupNode {
            clip_size: Some([200.0, 100.0]),
            ..GroupNode::default()
        }));
        frame.parent = Some(page);
        frame.transform = Transform2D::translation(-100.0, -50.0);
        let id = frame.id;
        doc.apply(Operation::create_node(frame))
            .expect("create text wrapper");
        id
    });
    let mut text = CanvasNode::new(NodeData::Text(TextNode::new("Hello world", 120.0, 40.0)));
    text.parent = Some(frame.unwrap_or(page));
    text.transform = if wrapped {
        Transform2D::translation(20.0, 20.0)
    } else {
        Transform2D::translation(-60.0, -20.0)
    };
    let text_id = text.id;
    doc.apply(Operation::create_node(text))
        .expect("create text layer");
    (doc, text_id, frame)
}

fn canvas_interaction_fixture(
    project: &Entity<Project>,
    doc: Doc,
    cx: &mut TestAppContext,
) -> (
    Entity<FigItem>,
    Entity<FigView>,
    gpui::WindowHandle<gpui::Empty>,
) {
    let item = crate::document::ready_item_for_test(
        project,
        std::path::PathBuf::from("/tmp/CanvasInteraction.fig"),
        doc,
        cx,
    );
    let scratch = cx.add_window(|_, _| gpui::Empty);
    let view = scratch
        .update(cx, |_, window, cx| {
            cx.new(|cx| FigView::new(item.clone(), project.clone(), window, cx))
        })
        .expect("create interaction view");
    view.update(cx, |view, _| {
        view.set_container_bounds(Bounds {
            origin: point(px(0.0), px(0.0)),
            size: size(px(800.0), px(600.0)),
        });
        view.set_viewport_silent(Viewport::default());
    });
    (item, view, scratch)
}

fn send_canvas_click(
    scratch: gpui::WindowHandle<gpui::Empty>,
    view: &Entity<FigView>,
    position: Point<Pixels>,
    click_count: usize,
    cx: &mut TestAppContext,
) {
    send_canvas_click_with_modifiers(
        scratch,
        view,
        position,
        click_count,
        gpui::Modifiers::default(),
        cx,
    );
}

fn bind_canvas_default_macos_keys(cx: &mut TestAppContext) {
    cx.update(|cx| {
        cx.clear_key_bindings();
        let bindings = settings::KeymapFile::load_asset_allow_partial_failure(
            "keymaps/default-macos.json",
            cx,
        )
        .expect("actual default macOS keymap");
        cx.bind_keys(bindings);
    });
}

async fn assert_mounted_text_escape_default_keymap(
    text_path: bool,
    notification: bool,
    edit_content: bool,
    cx: &mut TestAppContext,
) {
    init_visual_test(cx);
    bind_canvas_default_macos_keys(cx);
    let project = Project::test(FakeFs::new(cx.executor()), [], cx).await;
    let (mut doc, node, _) = text_selection_doc(true);
    let hit = if text_path {
        let (path, ink, _, _) = curved_text_path_hit_fixture();
        let text = doc.scene.get_mut(node).expect("text node");
        text.data = NodeData::TextPath(path);
        text.transform = Transform2D::translation(-85.0, 20.0);
        [ink[0] - 185.0, ink[1] - 30.0]
    } else {
        [-20.0, -10.0]
    };
    doc.selection.select_only(node);
    doc.history = Default::default();
    let item = crate::document::ready_item_for_test(
        &project,
        "/tmp/DefaultKeymapTextEscape.fig".into(),
        doc,
        cx,
    );
    let window = cx.add_window(|window, cx| {
        workspace::MultiWorkspace::test_new(project.clone(), window, cx)
    });
    let multi_workspace = window.entity(cx).expect("multi workspace");
    let workspace =
        multi_workspace.read_with(cx, |multi_workspace, _| multi_workspace.workspace().clone());
    let view = window
        .update(cx, |_, window, cx| {
            let view = cx.new(|cx| FigView::new(item.clone(), project, window, cx));
            workspace.update(cx, |workspace, cx| {
                workspace.add_item_to_active_pane(
                    Box::new(view.clone()),
                    None,
                    true,
                    window,
                    cx,
                );
            });
            window.activate_window();
            view.update(cx, |view, cx| {
                view.set_viewport_silent(Viewport::default());
                view.focus_handle.focus(window, cx);
                cx.notify();
            });
            view
        })
        .expect("mount text view in a real workspace");
    let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
    visual.simulate_resize(size(px(1400.0), px(900.0)));
    visual.run_until_parked();
    visual.update(|window, cx| window.draw(cx).clear());
    let position = view.read_with(&visual, |view, _| {
        view.container_bounds.expect("canvas").center()
            + point(px(hit[0] as f32), px(hit[1] as f32))
    });
    mounted_canvas_click(&mut visual, position, 2);
    visual.run_until_parked();
    view.read_with(&visual, |view, _| {
        assert!(
            view.text_edit.is_some(),
            "actual double-click opens the text session"
        );
    });
    let before = item.read_with(&visual, |item, _| {
        let document = item.document().expect("document");
        (
            serde_json::to_value(&document.doc).expect("baseline document"),
            document.doc.scene.get(node).expect("original node").clone(),
            document.raw_assets.clone(),
            document.doc.history.undo_depth(),
            item.is_dirty(),
        )
    });
    let notice = NotificationId::named("canvas-text-escape-control".into());
    if notification {
        workspace.update(&mut visual, |workspace, cx| {
            workspace.show_toast(Toast::new(notice.clone(), "Unrelated workspace notice"), cx);
        });
        visual.run_until_parked();
        assert!(workspace.read_with(&visual, |workspace, _| {
            workspace.notification_ids().contains(&notice)
        }));
    }
    if edit_content {
        visual.update(|window, cx| {
            view.update(cx, |view, cx| {
                let end = view
                    .text_edit
                    .as_ref()
                    .expect("session")
                    .session
                    .buffer()
                    .encode_utf16()
                    .count();
                gpui::EntityInputHandler::replace_and_mark_text_in_range(
                    view,
                    Some(0..end),
                    "draft",
                    Some(5..5),
                    window,
                    cx,
                );
                assert_eq!(
                    gpui::EntityInputHandler::marked_text_range(view, window, cx),
                    Some(0..5),
                );
                gpui::EntityInputHandler::replace_text_in_range(
                    view, None, "QA text", window, cx,
                );
                gpui::EntityInputHandler::unmark_text(view, window, cx);
            });
        });
        visual.simulate_keystrokes("enter");
        view.read_with(&visual, |view, _| {
            assert_eq!(
                view.text_edit
                    .as_ref()
                    .expect("Enter keeps the editor open")
                    .session
                    .buffer(),
                "QA text\n"
            );
        });
    }
    visual.simulate_keystrokes("escape");
    visual.run_until_parked();
    view.read_with(&visual, |view, _| {
        assert!(
            view.text_edit.is_none(),
            "one Escape must close text before any blank click"
        );
    });
    if notification {
        assert!(
            workspace.read_with(&visual, |workspace, _| {
                workspace.notification_ids().contains(&notice)
            }),
            "text Escape must not be consumed by unrelated workspace notification dismissal"
        );
    }
    item.read_with(&visual, |item, _| {
        let document = item.document().expect("document");
        assert!(
            !item.content_preview_active(),
            "Escape releases the preview barrier"
        );
        assert_eq!(document.raw_assets.as_ref(), before.2.as_ref());
        if edit_content {
            let actual = document.doc.scene.get(node).expect("edited node");
            let content = match &actual.data {
                NodeData::Text(text) => &text.content,
                NodeData::TextPath(text) => &text.content,
                _ => panic!("text kind preserved"),
            };
            assert_eq!(content, "QA text\n");
            assert_eq!(document.doc.history.undo_depth(), before.3 + 1);
        } else {
            assert_eq!(
                serde_json::to_value(&document.doc).expect("unchanged document"),
                before.0
            );
            assert_eq!(document.doc.history.undo_depth(), before.3);
            assert_eq!(item.is_dirty(), before.4);
        }
    });
    if edit_content {
        visual.simulate_keystrokes("cmd-z");
        visual.run_until_parked();
        item.read_with(&visual, |item, _| {
            let document = item.document().expect("document");
            assert_eq!(document.doc.scene.get(node), Some(&before.1));
            assert_eq!(document.doc.history.undo_depth(), before.3);
            assert_eq!(document.raw_assets.as_ref(), before.2.as_ref());
        });
    }
    workspace.update(&mut visual, |workspace, cx| {
        workspace.clear_all_notifications(cx)
    });
}

#[gpui::test]
async fn mounted_text_escape_default_keymap_without_notification(cx: &mut TestAppContext) {
    for text_path in [false, true] {
        assert_mounted_text_escape_default_keymap(text_path, false, false, cx).await;
    }
}

#[gpui::test]
async fn mounted_text_escape_default_keymap_with_notification(cx: &mut TestAppContext) {
    for text_path in [false, true] {
        assert_mounted_text_escape_default_keymap(text_path, true, false, cx).await;
    }
}

#[gpui::test]
async fn mounted_text_escape_default_keymap_commits_ime_and_enter_once(
    cx: &mut TestAppContext,
) {
    for text_path in [false, true] {
        assert_mounted_text_escape_default_keymap(text_path, true, true, cx).await;
    }
}

fn mounted_canvas_interaction_fixture(
    project: &Entity<Project>,
    doc: Doc,
    images: BTreeMap<AssetId, Vec<u8>>,
    cx: &mut TestAppContext,
) -> (Entity<FigItem>, Entity<FigView>, gpui::VisualTestContext) {
    cx.update(|cx| {
        cx.bind_keys([
            gpui::KeyBinding::new("escape", Cancel, Some("FigViewer")),
            gpui::KeyBinding::new("enter", Confirm, Some("FigViewer")),
            gpui::KeyBinding::new("cmd-z", Undo, Some("FigViewer")),
            gpui::KeyBinding::new("cmd-shift-z", Redo, Some("FigViewer")),
        ]);
    });
    let item = crate::document::ready_item_for_test(
        project,
        std::path::PathBuf::from("/tmp/MountedCanvasInteraction.fig"),
        doc,
        cx,
    );
    item.update(cx, |item, cx| {
        item.with_document(cx, |document| {
            let (_, mut assets) = document.doc_and_assets();
            for (expected, bytes) in images {
                let (actual, _) = assets.add_image(bytes).expect("fixture image");
                assert_eq!(actual, expected);
            }
            ((), DocChange::None)
        });
    });
    let window = cx.add_window({
        let item = item.clone();
        let project = project.clone();
        move |window, cx| FigView::new(item, project, window, cx)
    });
    let view = window.entity(cx).expect("mounted view");
    let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
    visual.simulate_resize(size(px(1400.0), px(900.0)));
    view.update(&mut visual, |view, cx| {
        view.set_viewport_silent(Viewport::default());
        cx.notify();
    });
    visual.run_until_parked();
    visual.update(|window, cx| window.draw(cx).clear());
    (item, view, visual)
}

fn mounted_canvas_click(
    visual: &mut gpui::VisualTestContext,
    position: Point<Pixels>,
    click_count: usize,
) {
    visual.simulate_event(MouseDownEvent {
        position,
        button: MouseButton::Left,
        modifiers: gpui::Modifiers::none(),
        click_count,
        first_mouse: false,
    });
    visual.simulate_event(MouseUpEvent {
        position,
        button: MouseButton::Left,
        modifiers: gpui::Modifiers::none(),
        click_count,
    });
}

fn bitmap_canvas_doc() -> (Doc, NodeId, BTreeMap<AssetId, Vec<u8>>) {
    let mut encoded = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
        2,
        2,
        image::Rgba([48, 96, 192, 255]),
    ))
    .write_to(&mut encoded, image::ImageFormat::Png)
    .expect("fixture PNG");
    let bytes = encoded.into_inner();
    let asset = fanta_format::asset_id_for_bytes(&bytes);
    let mut doc = doc_with_one_page();
    let mut bitmap = CanvasNode::new(NodeData::Bitmap(fanta_doc::BitmapNode {
        asset,
        natural_size: [2, 2],
        local_size: [200.0, 100.0],
        crop: Some([0.1, 0.1, 0.8, 0.8]),
        fit: fanta_doc::ImageFitMode::Fill,
        tint: Some(Color::rgba(255, 192, 128, 200)),
    }));
    bitmap.parent = doc.active_page();
    bitmap.transform = Transform2D::translation(-100.0, -50.0);
    let bitmap_id = bitmap.id;
    doc.apply(Operation::create_node(bitmap)).expect("bitmap");
    doc.history = Default::default();
    (doc, bitmap_id, BTreeMap::from([(asset, bytes)]))
}

fn curved_text_path_hit_fixture() -> (fanta_doc::TextPathNode, [f64; 2], [f64; 2], [f64; 2]) {
    let mut baseline = fanta_doc::PathData::new();
    baseline
        .move_to(0.0, 55.0)
        .quad_to(185.0, -35.0, 370.0, 55.0);
    let mut text_path = fanta_doc::TextPathNode::new(baseline, "PATH EDIT ME");
    text_path.style.size_px = 32.0;
    let glyph_quad = fanta_render::text_path_selection_quads(&text_path, 6..7)
        .into_iter()
        .next()
        .expect("D has shaped geometry");
    let bounds = glyph_quad.points.into_iter().fold(
        fanta_doc::Bounds {
            min_x: f64::INFINITY,
            min_y: f64::INFINITY,
            max_x: f64::NEG_INFINITY,
            max_y: f64::NEG_INFINITY,
        },
        |bounds, point| fanta_doc::Bounds {
            min_x: bounds.min_x.min(point[0]),
            min_y: bounds.min_y.min(point[1]),
            max_x: bounds.max_x.max(point[0]),
            max_y: bounds.max_y.max(point[1]),
        },
    );
    let points = (0..=bounds.height().ceil() as usize * 2).flat_map(|y| {
        (0..=bounds.width().ceil() as usize * 2)
            .map(move |x| [bounds.min_x + x as f64 / 2.0, bounds.min_y + y as f64 / 2.0])
    });
    let contains = |point| fanta_render::text_path_contains_point(&text_path, point);
    let in_d = |point| {
        fanta_render::text_path_hit_test(&text_path, point)
            .is_some_and(|byte| (6..=7).contains(&byte))
    };
    let ink = points
        .clone()
        .find(|point| {
            in_d(*point)
                && [[0.0, 0.0], [-0.5, 0.0], [0.5, 0.0], [0.0, -0.5], [0.0, 0.5]]
                    .into_iter()
                    .all(|offset| contains([point[0] + offset[0], point[1] + offset[1]]))
        })
        .expect("interior ink on the D in EDIT");
    let counter = points
        .filter(|point| {
            in_d(*point)
                && [[0.0, 0.0], [-0.5, 0.0], [0.5, 0.0], [0.0, -0.5], [0.0, 0.5]]
                    .into_iter()
                    .all(|offset| !contains([point[0] + offset[0], point[1] + offset[1]]))
        })
        .find(|point| {
            [[-1.0, 0.0], [1.0, 0.0], [0.0, -1.0], [0.0, 1.0]]
                .into_iter()
                .all(|direction| {
                    (1..=12).any(|distance| {
                        contains([
                            point[0] + direction[0] * f64::from(distance),
                            point[1] + direction[1] * f64::from(distance),
                        ])
                    })
                })
        })
        .expect("enclosed counter surrounded by glyph ink");
    let whitespace_quad = fanta_render::text_path_selection_quads(&text_path, 4..5)
        .into_iter()
        .next()
        .expect("space has shaped geometry");
    let whitespace = whitespace_quad
        .points
        .into_iter()
        .fold([0.0, 0.0], |sum, point| {
            [sum[0] + point[0] / 4.0, sum[1] + point[1] / 4.0]
        });
    assert!(!contains(whitespace));
    (text_path, ink, counter, whitespace)
}

fn send_canvas_click_with_modifiers(
    scratch: gpui::WindowHandle<gpui::Empty>,
    view: &Entity<FigView>,
    position: Point<Pixels>,
    click_count: usize,
    modifiers: gpui::Modifiers,
    cx: &mut TestAppContext,
) {
    scratch
        .update(cx, |_, window, cx| {
            view.update(cx, |view, cx| {
                view.handle_mouse_down(
                    &MouseDownEvent {
                        button: MouseButton::Left,
                        position,
                        modifiers,
                        click_count,
                        first_mouse: false,
                    },
                    window,
                    cx,
                );
                view.handle_mouse_up(
                    &MouseUpEvent {
                        button: MouseButton::Left,
                        position,
                        modifiers,
                        click_count,
                    },
                    window,
                    cx,
                );
            });
        })
        .expect("dispatch canvas click");
}

/// A project on disk with one page and no children, plus the view that
/// owns its autosave timer.
async fn autosave_fixture(
    cx: &mut TestAppContext,
) -> (
    tempfile::TempDir,
    std::path::PathBuf,
    Entity<FigItem>,
    Entity<FigView>,
) {
    init_visual_test(cx);
    let project = Project::test(FakeFs::new(cx.executor()), [], cx).await;
    let dir = tempfile::tempdir().expect("temp dir");
    let root = dir.path().join("Design");
    let initial = doc_with_one_page();
    crate::document::write_project(&root, &initial, &BTreeMap::new())
        .expect("materialize project fixture");
    let item = crate::document::ready_item_with_root_for_test(
        &project,
        dir.path().join("Design.fig"),
        Some(root.clone()),
        initial,
        cx,
    );
    let scratch = cx.add_window(|_, _| gpui::Empty);
    let view = scratch
        .update(cx, |_, window, cx| {
            cx.new(|cx| FigView::new(item.clone(), project.clone(), window, cx))
        })
        .expect("create fig view");
    (dir, root, item, view)
}

fn add_rect(item: &Entity<FigItem>, cx: &mut TestAppContext) {
    item.update(cx, |item, cx| {
        item.with_document(cx, |document| {
            let page = document.doc.active_page().expect("active page");
            let mut rect = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
                0.0,
                0.0,
                10.0,
                10.0,
                Color::BLACK,
            )));
            rect.parent = Some(page);
            document
                .doc
                .apply(Operation::create_node(rect))
                .expect("create rect");
            ((), DocChange::Content)
        });
    });
}

#[cfg(target_os = "macos")]
async fn assert_native_figview_window_close(
    save: bool,
    autosave_while_prompt_open: bool,
    cx: &mut TestAppContext,
) -> Result<()> {
    use gpui::UpdateGlobal;
    use project::ProjectItem as _;

    init_visual_test(cx);
    cx.executor().allow_parking();
    cx.update(|cx| {
        cx.set_global(db::AppDatabase::test_new());
        SettingsStore::update_global(cx, |store, cx| {
            store.update_user_settings(cx, |settings| {
                settings.workspace.autosave = Some(settings::AutosaveSetting::Off);
                settings
                    .session
                    .get_or_insert_default()
                    .restore_unsaved_buffers = Some(false);
            });
        });
    });
    let directory = tempfile::tempdir()?;
    let root = directory.path().join("Close Matrix");
    let (mut initial, text_id, _) = text_selection_doc(false);
    let mut control = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        150.0,
        100.0,
        40.0,
        30.0,
        Color::BLACK,
    )));
    control.parent = initial.active_page();
    control.name = "Unchanged control".into();
    initial.apply(Operation::create_node(control))?;
    crate::document::write_project(&root, &initial, &BTreeMap::new())?;
    let (original_on_disk, original_assets) = fanta_format::read_project_tree(&root)?;
    let original_on_disk = serde_json::to_value(original_on_disk)?;

    // Source buffers must observe the same files that the canvas writer changes.
    let file_system = Arc::new(fs::RealFs::new(None, cx.executor()));
    let project = Project::test(file_system, [root.as_path()], cx).await;
    let worktree_id = project.read_with(cx, |project, cx| {
        project
            .worktrees(cx)
            .next()
            .expect("fixture worktree")
            .read(cx)
            .id()
    });
    let item = cx
        .update(|cx| {
            FigItem::try_open(
                &project,
                &project::ProjectPath {
                    worktree_id,
                    path: util::rel_path::rel_path("fanta.json").into(),
                },
                cx,
            )
        })
        .expect("native project opener")
        .await?;
    cx.run_until_parked();
    item.read_with(cx, |item, _| {
        assert!(item.has_ready_document());
        assert_eq!(item.project_root(), Some(root.as_path()));
        assert!(!item.is_dirty());
    });

    let window = cx.add_window(|window, cx| {
        workspace::MultiWorkspace::test_new(project.clone(), window, cx)
    });
    let multi_workspace = window.entity(cx)?;
    let workspace =
        multi_workspace.read_with(cx, |multi_workspace, _| multi_workspace.workspace().clone());
    let session_id = workspace.read_with(cx, |workspace, _| workspace.session_id());
    assert!(session_id.is_some());
    let view = window.update(cx, |_, window, cx| {
        let view = cx.new(|cx| FigView::new(item.clone(), project.clone(), window, cx));
        workspace.update(cx, |workspace, cx| {
            workspace.add_item_to_active_pane(Box::new(view.clone()), None, true, window, cx);
        });
        window.activate_window();
        view.update(cx, |view, cx| {
            view.set_viewport_silent(Viewport::default());
            view.focus_handle.focus(window, cx);
        });
        view
    })?;
    let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
    visual.simulate_resize(size(px(1400.0), px(900.0)));
    visual.run_until_parked();
    visual.update(|window, cx| window.draw(cx).clear());
    visual.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.open_text_edit(text_id, TextEditSeed::SelectAll, window, cx);
            assert!(view.text_edit.is_some(), "real text editor is open");
            gpui::EntityInputHandler::replace_text_in_range(
                view,
                Some(0..11),
                "Close draft",
                window,
                cx,
            );
            view.commit_text_edit(cx);
            assert!(view.text_edit.is_none());
        });
    });
    visual.run_until_parked();
    let (edited, assets) = item.read_with(&visual, |item, _| {
        assert!(item.is_dirty(), "committed edit has not yet been saved");
        let document = item.document().expect("edited document");
        let NodeData::Text(text) = &document.doc.scene.get(text_id).expect("text node").data
        else {
            panic!("fixture text kind preserved")
        };
        assert_eq!(text.content, "Close draft");
        (
            document.doc.clone_for_persist(),
            document.raw_assets.clone(),
        )
    });
    let expected_root = directory.path().join("Expected saved projection");
    crate::document::write_project(&expected_root, &edited, &assets)?;
    let (expected_on_disk, expected_assets) = fanta_format::read_project_tree(&expected_root)?;
    let expected_on_disk = serde_json::to_value(expected_on_disk)?;
    assert_ne!(expected_on_disk, original_on_disk);
    assert_eq!(
        serde_json::to_value(fanta_format::read_project_tree(&root)?.0)?,
        original_on_disk,
        "the pre-prompt state must still contain the original text"
    );

    multi_workspace.update_in(&mut visual, |multi_workspace, window, cx| {
        multi_workspace.close_window(&workspace::CloseWindow, window, cx);
    });
    visual.run_until_parked();
    let prompt = visual.pending_prompt().expect("dirty native close prompt");
    assert!(prompt.0.contains("save"), "actual save prompt: {prompt:?}");
    workspace.read_with(&visual, |workspace, cx| {
        assert_eq!(workspace.session_id(), session_id);
        assert_eq!(workspace.project().entity_id(), project.entity_id());
        assert_eq!(
            workspace
                .active_item_as::<FigView>(cx)
                .map(|view| view.entity_id()),
            Some(view.entity_id())
        );
    });
    if autosave_while_prompt_open {
        visual.executor().advance_clock(AUTOSAVE_DEBOUNCE * 2);
        visual.run_until_parked();
        assert!(
            visual.has_pending_prompt(),
            "autosave does not answer the prompt"
        );
        item.read_with(&visual, |item, _| assert!(!item.is_dirty()));
        view.read_with(&visual, |view, _| assert!(view.autosave_task.is_none()));
        let (saved, saved_assets) = fanta_format::read_project_tree(&root)?;
        assert_eq!(serde_json::to_value(saved)?, expected_on_disk);
        assert_eq!(saved_assets, expected_assets);
    }
    visual.simulate_prompt_answer(if save { "Save" } else { "Don't Save" });
    visual.run_until_parked();

    // Window-scoped updates require a live window; retained entities remain readable after close.
    assert!(!visual.has_pending_prompt());
    assert!(!visual.read(|cx| cx.windows().contains(&window.into())));
    workspace.read_with(&visual, |workspace, cx| {
        assert_eq!(
            workspace.session_id(),
            None,
            "the original session was removed"
        );
        assert_eq!(workspace.project().entity_id(), project.entity_id());
        assert_eq!(
            workspace
                .active_item_as::<FigView>(cx)
                .map(|view| view.entity_id()),
            Some(view.entity_id()),
            "window close must not merely close the canvas tab"
        );
    });
    view.read_with(&visual, |view, _| {
        assert_eq!(view.item.entity_id(), item.entity_id())
    });
    item.read_with(&visual, |item, _| {
        assert!(!item.is_dirty());
        assert_eq!(item.project_root(), Some(root.as_path()));
        let NodeData::Text(text) = &item
            .doc()
            .expect("retained document")
            .scene
            .get(text_id)
            .expect("text")
            .data
        else {
            panic!("fixture text kind preserved")
        };
        assert_eq!(
            text.content,
            if save { "Close draft" } else { "Hello world" }
        );
    });
    let (actual, actual_assets) = fanta_format::read_project_tree(&root)?;
    assert_eq!(
        serde_json::to_value(actual)?,
        if save {
            expected_on_disk
        } else {
            original_on_disk
        }
    );
    assert_eq!(
        actual_assets,
        if save {
            expected_assets
        } else {
            original_assets
        }
    );
    Ok(())
}

#[cfg(target_os = "macos")]
#[gpui::test]
async fn mounted_native_figview_save_closes_multi_workspace_window(cx: &mut TestAppContext) {
    assert_native_figview_window_close(true, false, cx)
        .await
        .expect("Save persists the native canvas and removes its window");
}

#[cfg(target_os = "macos")]
#[gpui::test]
async fn mounted_native_figview_discard_closes_multi_workspace_window(cx: &mut TestAppContext) {
    assert_native_figview_window_close(false, false, cx)
        .await
        .expect("Don't Save reloads the native canvas and removes its window");
}

#[cfg(target_os = "macos")]
#[gpui::test]
async fn mounted_native_figview_autosave_during_prompt_then_save_closes_window(
    cx: &mut TestAppContext,
) {
    assert_native_figview_window_close(true, true, cx)
        .await
        .expect("Save finishes closing after the canvas autosaved while prompted");
}

#[cfg(all(target_os = "macos", feature = "fanta-gpui-ui"))]
async fn assert_property_draft_persistence(
    draft: &str,
    choice: &str,
    shared_tab: bool,
    cx: &mut TestAppContext,
) -> Result<()> {
    use fanta_gpui::design::DesignPropertyDraftState;
    use gpui::UpdateGlobal;
    use project::ProjectItem as _;
    use workspace::item::ItemHandle;

    init_visual_test(cx);
    cx.executor().allow_parking();
    cx.update(|cx| {
        cx.set_global(db::AppDatabase::test_new());
        SettingsStore::update_global(cx, |store, cx| {
            store.update_user_settings(cx, |settings| {
                settings.workspace.autosave = Some(settings::AutosaveSetting::Off);
                settings
                    .session
                    .get_or_insert_default()
                    .restore_unsaved_buffers = Some(false);
            });
        });
    });
    let directory = tempfile::tempdir()?;
    let root = directory.path().join("Property draft");
    let (mut initial, text_id, _) = text_selection_doc(false);
    initial
        .scene
        .set_transform(text_id, Transform2D::translation(40.0, 60.0))?;
    let asset_id = AssetId::new();
    initial.asset_library.insert(
        asset_id,
        fanta_doc::ProjectAsset {
            name: "Unchanged asset".into(),
            kind: fanta_doc::ProjectAssetKind::Image,
        },
    );
    let raw_assets = BTreeMap::from([(asset_id, b"unchanged fixture asset".to_vec())]);
    crate::document::write_project(&root, &initial, &raw_assets)?;
    let (original_disk, original_assets) = fanta_format::read_project_tree(&root)?;
    let original_disk = serde_json::to_value(original_disk)?;
    let project = Project::test(
        Arc::new(fs::RealFs::new(None, cx.executor())),
        [root.as_path()],
        cx,
    )
    .await;
    let worktree_id = project.read_with(cx, |project, cx| {
        project
            .worktrees(cx)
            .next()
            .expect("fixture worktree")
            .read(cx)
            .id()
    });
    let item = cx
        .update(|cx| {
            FigItem::try_open(
                &project,
                &project::ProjectPath {
                    worktree_id,
                    path: util::rel_path::rel_path("fanta.json").into(),
                },
                cx,
            )
        })
        .expect("native project opener")
        .await?;
    cx.run_until_parked();
    item.read_with(cx, |item, _| {
        assert!(item.has_ready_document());
        assert_eq!(item.project_root(), Some(root.as_path()));
        assert!(!item.is_dirty());
    });
    let window = cx.add_window(|window, cx| {
        workspace::MultiWorkspace::test_new(project.clone(), window, cx)
    });
    let multi_workspace = window.entity(cx)?;
    let workspace = multi_workspace.read_with(cx, |root, _| root.workspace().clone());
    let (view, other) = window.update(cx, |_, window, cx| {
        let view = cx.new(|cx| FigView::new(item.clone(), project.clone(), window, cx));
        let other = if shared_tab {
            let other = cx.new(|cx| FigView::new(item.clone(), project.clone(), window, cx));
            other.update(cx, |other, cx| {
                other.inspector_sidebar_visible = false;
                cx.notify();
            });
            workspace.update(cx, |workspace, cx| {
                workspace.add_item_to_active_pane(
                    Box::new(other.clone()),
                    None,
                    false,
                    window,
                    cx,
                );
                workspace.split_item(
                    workspace::SplitDirection::Right,
                    Box::new(view.clone()),
                    window,
                    cx,
                );
            });
            item.update(cx, |item, cx| {
                let old = item
                    .doc()
                    .expect("document")
                    .scene
                    .get(text_id)
                    .expect("text")
                    .name
                    .clone();
                item.apply(
                    Operation::SetName {
                        id: text_id,
                        old,
                        new: "Shared committed edit".into(),
                    },
                    cx,
                )
            })
            .expect("shared committed edit");
            Some(other)
        } else {
            workspace.update(cx, |workspace, cx| {
                workspace.add_item_to_active_pane(
                    Box::new(view.clone()),
                    None,
                    true,
                    window,
                    cx,
                );
            });
            None
        };
        window.activate_window();
        view.update(cx, |view, cx| {
            view.inspector_sidebar_visible = true;
            view.set_viewport_silent(Viewport::default());
            view.focus_handle.focus(window, cx);
            cx.notify();
        });
        (view, other)
    })?;
    let assert_shared_views = |visual: &gpui::VisualTestContext| {
        if let Some(other) = &other {
            visual.read(|cx| {
                let panes = workspace.read(cx).panes();
                assert_eq!(panes.len(), 2);
                let mounted = panes
                    .iter()
                    .map(|pane| {
                        pane.read(cx)
                            .items()
                            .map(|item| item.item_id())
                            .collect::<Vec<_>>()
                    })
                    .collect::<Vec<_>>();
                assert_eq!(
                    mounted,
                    vec![vec![other.entity_id()], vec![view.entity_id()]]
                );
                assert_ne!(view.entity_id(), other.entity_id());
                assert_eq!(view.read(cx).item.entity_id(), item.entity_id());
                assert_eq!(other.read(cx).item.entity_id(), item.entity_id());
            });
        }
    };
    let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
    assert_shared_views(&visual);
    visual.simulate_resize(size(px(1400.0), px(1000.0)));
    visual.run_until_parked();
    item.update(&mut visual, |item, cx| {
        item.with_document(cx, |document| {
            assert!(document.doc.scene.contains(text_id));
            document.doc.selection.select_only(text_id);
            ((), DocChange::Selection)
        })
    })
    .context("select the loaded text in its mounted canvas")?;
    visual.run_until_parked();
    view.update_in(&mut visual, |view, window, cx| {
        assert_eq!(view.editor_workspace(cx), EditorWorkspace::Canvas);
        assert_eq!(view.editor_mode(cx), EditorMode::Design);
        assert!(view.is_editable(cx));
        assert!(view.inspector_sidebar_visible);
        view.refresh_gpui_design(cx);
        view.refresh_properties_inspector(window, cx);
        view.focus_handle.focus(window, cx);
    });
    visual.run_until_parked();
    visual.read(|cx| {
        assert_eq!(
            workspace
                .read(cx)
                .active_item_as::<FigView>(cx)
                .expect("active inspector draft owner")
                .entity_id(),
            view.entity_id()
        );
        let document = item.read(cx).doc().expect("loaded document");
        assert_eq!(
            document.selection.iter().copied().collect::<Vec<_>>(),
            vec![text_id]
        );
        assert_eq!(
            document.scene.get(text_id).expect("selected text").parent,
            document.active_page()
        );
    });
    visual.update(|window, cx| {
        window.refresh();
        window.draw(cx).clear();
    });
    let panel = view.read_with(&visual, |view, _| {
        view.gpui_design
            .as_ref()
            .expect("Design panel")
            .panel
            .clone()
    });
    panel.read_with(&visual, |panel, _| {
        let selection = panel.inspection_context().selection().items();
        assert_eq!(selection.len(), 1, "mounted inspector selection");
        assert_eq!(panel.node().id.as_ref(), text_id.to_string());
        assert!(panel.inspection_context().permissions().can_edit());
        assert_eq!((panel.node().x, panel.node().y), (40.0, 60.0));
    });
    let before = item.read_with(&visual, |item, _| item.doc().expect("document").clone());
    let before_history = (before.history.undo_depth(), before.history.redo_depth());
    let replace_input = |visual: &mut gpui::VisualTestContext, text: &str| {
        visual.update(|window, cx| {
            window.refresh();
            window.draw(cx).clear();
        });
        let field = visual
            .debug_bounds("fig-gpui-design-x")
            .expect("rendered X field");
        visual.simulate_click(field.center(), gpui::Modifiers::none());
        visual.simulate_keystrokes("cmd-a");
        visual.simulate_input(text);
        visual.run_until_parked();
    };
    if draft == "40" {
        let field = visual
            .debug_bounds("fig-gpui-design-x")
            .expect("rendered X field");
        visual.simulate_click(field.center(), gpui::Modifiers::none());
        visual.run_until_parked();
    } else {
        replace_input(&mut visual, "48");
        if draft != "48" {
            replace_input(&mut visual, draft);
        }
    }
    let changed = draft != "40";
    let invalid = draft == "-";
    let state = if invalid {
        DesignPropertyDraftState::Invalid
    } else if changed {
        DesignPropertyDraftState::Valid
    } else {
        DesignPropertyDraftState::Unchanged
    };
    assert_eq!(
        panel.read_with(&visual, |panel, cx| panel.property_draft_state(cx)),
        state
    );
    view.read_with(&visual, |view, cx| {
        assert_eq!(view.has_pending_view_edits(cx), changed);
        assert_eq!(view.is_dirty(cx), changed || shared_tab);
        assert!(!view.autosave_allowed(cx));
        if !shared_tab {
            assert!(
                view.autosave_task.is_none(),
                "preview and draft status must not schedule saving"
            );
        }
    });
    item.read_with(&visual, |item, _| {
        assert_eq!(
            item.is_dirty(),
            changed || shared_tab,
            "an active preview is dirty without committing history"
        );
        let doc = item.doc().expect("preview document");
        assert_eq!(
            (doc.history.undo_depth(), doc.history.redo_depth()),
            before_history
        );
        let mut preview = before.clone();
        preview
            .scene
            .set_transform(
                text_id,
                Transform2D::translation(if changed { 48.0 } else { 40.0 }, 60.0),
            )
            .expect("expected preview");
        assert_eq!(
            serde_json::to_value(doc).expect("preview document"),
            serde_json::to_value(preview).expect("expected preview document")
        );
        assert_eq!(
            item.document().expect("document").raw_assets.as_ref(),
            &original_assets
        );
    });
    assert_eq!(
        serde_json::to_value(fanta_format::read_project_tree(&root)?.0)?,
        original_disk
    );
    if shared_tab {
        workspace.read_with(&visual, |workspace, cx| {
            assert!(
                !workspace::Pane::skip_save_on_close(&view, workspace, cx),
                "another tab retains the shared edit, but cannot retain this input draft"
            );
        });
    }
    if changed {
        visual.read(|cx| assert!(!ItemHandle::can_autosave(&view, cx)));
        view.update_in(&mut visual, |view, window, cx| {
            Item::save(
                view,
                SaveOptions {
                    autosave: true,
                    format: false,
                    force_format: false,
                },
                project.clone(),
                window,
                cx,
            )
        })
        .await?;
        assert_eq!(
            panel.read_with(&visual, |panel, cx| panel.property_draft_state(cx)),
            state
        );
        assert_eq!(
            serde_json::to_value(fanta_format::read_project_tree(&root)?.0)?,
            original_disk
        );
    }
    let mut saved = false;
    if choice == "Explicit" {
        let result = workspace
            .update_in(&mut visual, |workspace, window, cx| {
                workspace.save_active_item(workspace::SaveIntent::Save, window, cx)
            })
            .await;
        if invalid {
            assert!(
                result
                    .expect_err("invalid Save must refuse")
                    .to_string()
                    .contains("invalid property")
            );
            assert_eq!(
                panel.read_with(&visual, |panel, cx| panel.property_draft_state(cx)),
                state
            );
            assert_eq!(
                serde_json::to_value(fanta_format::read_project_tree(&root)?.0)?,
                original_disk
            );
            replace_input(&mut visual, "48");
            workspace
                .update_in(&mut visual, |workspace, window, cx| {
                    workspace.save_active_item(workspace::SaveIntent::Save, window, cx)
                })
                .await?;
        } else {
            result?;
        }
        saved = changed;
    } else {
        multi_workspace.update_in(&mut visual, |root, window, cx| {
            root.close_window(&workspace::CloseWindow, window, cx)
        });
        visual.run_until_parked();
        if changed {
            let prompt = visual
                .pending_prompt()
                .expect("normal three-choice dirty prompt");
            if shared_tab {
                assert_eq!(
                    prompt.0,
                    "Do you want to save all changes in the following files?"
                );
            } else {
                assert!(prompt.0.contains("save"));
            }
            assert_shared_views(&visual);
            assert_eq!(
                panel.read_with(&visual, |panel, cx| panel.property_draft_state(cx)),
                state,
                "prompt activation must not consume a valid or invalid draft"
            );
            assert_eq!(
                serde_json::to_value(fanta_format::read_project_tree(&root)?.0)?,
                original_disk
            );
            visual.executor().advance_clock(AUTOSAVE_DEBOUNCE * 2);
            visual.run_until_parked();
            assert!(visual.has_pending_prompt());
            assert_eq!(
                serde_json::to_value(fanta_format::read_project_tree(&root)?.0)?,
                original_disk
            );
            visual.simulate_prompt_answer(choice);
            visual.run_until_parked();
            let save_requested = matches!(choice, "Save" | "Save all");
            if choice == "Cancel" || (save_requested && invalid) {
                if invalid && save_requested {
                    let error = visual
                        .pending_prompt()
                        .expect("visible close Save validation error");
                    assert!(format!("{} {}", error.0, error.1).contains("invalid property"));
                    visual.simulate_prompt_answer("OK");
                    visual.run_until_parked();
                }
                assert!(visual.read(|cx| cx.windows().contains(&window.into())));
                assert_shared_views(&visual);
                assert_eq!(
                    panel.read_with(&visual, |panel, cx| panel.property_draft_state(cx)),
                    state
                );
                view.read_with(&visual, |view, cx| assert!(view.is_dirty(cx)));
                assert_eq!(
                    serde_json::to_value(fanta_format::read_project_tree(&root)?.0)?,
                    original_disk
                );
                multi_workspace.update_in(&mut visual, |root, window, cx| {
                    root.close_window(&workspace::CloseWindow, window, cx)
                });
                visual.run_until_parked();
                assert!(visual.has_pending_prompt(), "retry still offers Discard");
                visual.simulate_prompt_answer(if shared_tab {
                    "Discard all"
                } else {
                    "Don't Save"
                });
                visual.run_until_parked();
            } else {
                saved = save_requested;
            }
        }
        assert!(!visual.has_pending_prompt());
        assert!(
            !visual.read(|cx| cx.windows().contains(&window.into())),
            "the original whole window closed"
        );
    }
    let (actual_disk, actual_assets) = fanta_format::read_project_tree(&root)?;
    assert_eq!(actual_assets, original_assets);
    if saved {
        let mut expected = before.clone();
        expected.apply(Operation::SetTransform {
            id: text_id,
            old: Transform2D::translation(40.0, 60.0),
            new: Transform2D::translation(48.0, 60.0),
        })?;
        let now = i64::try_from(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_secs(),
        )?;
        item.read_with(&visual, |item, _| {
            let actual = item.doc().expect("saved document");
            assert!((before.metadata.modified_at..=now).contains(&actual.metadata.modified_at));
            expected.metadata.modified_at = actual.metadata.modified_at;
            assert_eq!(
                serde_json::to_value(actual).expect("actual"),
                serde_json::to_value(&expected).expect("expected")
            );
            assert_eq!(actual.history.undo_depth(), before_history.0 + 1);
            assert_eq!(actual.history.redo_depth(), 0);
            assert!(!item.is_dirty());
        });
        let expected_root = directory.path().join("Expected projection");
        crate::document::write_project(&expected_root, &expected, &original_assets)?;
        assert_eq!(
            serde_json::to_value(actual_disk)?,
            serde_json::to_value(fanta_format::read_project_tree(&expected_root)?.0)?
        );
    } else {
        assert_eq!(serde_json::to_value(actual_disk)?, original_disk);
        item.read_with(&visual, |item, _| {
            assert!(!item.is_dirty());
            let doc = item.doc().expect("restored document");
            assert_eq!(doc.history.undo_depth(), 0);
            assert_eq!(doc.history.redo_depth(), 0);
        });
    }
    Ok(())
}

#[cfg(all(target_os = "macos", feature = "fanta-gpui-ui"))]
#[gpui::test]
async fn property_draft_explicit_save_commits_focused_input(cx: &mut TestAppContext) {
    assert_property_draft_persistence("48", "Explicit", false, cx)
        .await
        .expect("focused Save");
}

#[cfg(all(target_os = "macos", feature = "fanta-gpui-ui"))]
#[gpui::test]
async fn property_draft_invalid_save_preserves_then_corrects_input(cx: &mut TestAppContext) {
    assert_property_draft_persistence("-", "Explicit", false, cx)
        .await
        .expect("invalid Save and correction");
}

#[cfg(all(target_os = "macos", feature = "fanta-gpui-ui"))]
#[gpui::test]
async fn property_draft_window_save_commits_after_prompt_focus(cx: &mut TestAppContext) {
    assert_property_draft_persistence("48", "Save", false, cx)
        .await
        .expect("window Save");
}

#[cfg(all(target_os = "macos", feature = "fanta-gpui-ui"))]
#[gpui::test]
async fn property_draft_invalid_window_save_keeps_discard_available(cx: &mut TestAppContext) {
    assert_property_draft_persistence("-", "Save", false, cx)
        .await
        .expect("invalid close Save and Discard");
}

#[cfg(all(target_os = "macos", feature = "fanta-gpui-ui"))]
#[gpui::test]
async fn property_draft_window_cancel_then_discard_preserves_shared_item(
    cx: &mut TestAppContext,
) {
    assert_property_draft_persistence("48", "Cancel", true, cx)
        .await
        .expect("shared pending view");
}

#[cfg(all(target_os = "macos", feature = "fanta-gpui-ui"))]
#[gpui::test]
async fn property_draft_shared_save_all_commits_later_pane_owner_first(
    cx: &mut TestAppContext,
) {
    assert_property_draft_persistence("48", "Save all", true, cx)
        .await
        .expect("shared Save all settles the later pane's draft first");
}

#[cfg(all(target_os = "macos", feature = "fanta-gpui-ui"))]
#[gpui::test]
async fn property_draft_invalid_shared_save_all_keeps_discard_available(
    cx: &mut TestAppContext,
) {
    assert_property_draft_persistence("-", "Save all", true, cx)
        .await
        .expect("invalid shared Save all preserves both views and offers Discard");
}

#[cfg(all(target_os = "macos", feature = "fanta-gpui-ui"))]
#[gpui::test]
async fn property_draft_invalid_window_cancel_then_discard(cx: &mut TestAppContext) {
    assert_property_draft_persistence("-", "Cancel", false, cx)
        .await
        .expect("invalid Cancel and Discard");
}

#[cfg(all(target_os = "macos", feature = "fanta-gpui-ui"))]
#[gpui::test]
async fn property_draft_unchanged_focused_field_closes_without_prompt(cx: &mut TestAppContext) {
    assert_property_draft_persistence("40", "Close", false, cx)
        .await
        .expect("unchanged field");
    use crate::properties_snapshot::selection_world_geometry;
    use fanta_gpui::design::DesignPropertyDraftState;

    init_visual_test(cx);
    let project = Project::test(FakeFs::new(cx.executor()), [], cx).await;
    for horizontal in [true, false] {
        let (mut doc, _, images) = bitmap_canvas_doc();
        let mut baseline = fanta_doc::PathData::new();
        baseline
            .move_to(0.0, 55.0)
            .quad_to(185.0, -35.0, 370.0, 55.0);
        let mut text_path = fanta_doc::TextPathNode::new(baseline, "QA PATH");
        text_path.style.size_px = 32.0;
        let bounds = fanta_render::text_path_visual_bounds(&text_path).expect("shaped text");
        let mut node = CanvasNode::new(NodeData::TextPath(text_path));
        node.parent = doc.active_page();
        node.transform =
            Transform2D::translation(55.375 - bounds.min_x, 356.375 - bounds.min_y);
        let id = node.id;
        doc.apply(Operation::create_node(node)).expect("TextPath");
        doc.history = Default::default();
        let (item, view, _host, mut visual) = pane_layout_fixture(&project, doc, images, cx);
        item.update(&mut visual, |item, cx| {
            item.with_document(cx, |document| {
                document.doc.selection.select_only(id);
                ((), DocChange::Selection)
            })
            .expect("select mounted TextPath");
        });
        draw_pane_layout(&mut visual);
        let panel = view.read_with(&visual, |view, _| {
            view.gpui_design.as_ref().expect("Design").panel.clone()
        });
        let snapshot = |item: &FigItem| {
            let document = item.document().expect("document");
            (
                serde_json::to_value(&document.doc).expect("full document"),
                document.raw_assets.clone(),
                document.doc.scene.get(id).expect("TextPath").transform,
                selection_world_geometry(&document.doc, id).expect("visual bounds"),
                document.doc.history.undo_depth(),
                document.doc.history.redo_depth(),
                item.is_dirty(),
            )
        };
        let before = item.read_with(&visual, |item, _| {
            assert!(!item.is_dirty());
            assert_eq!(item.doc().expect("document").history.undo_depth(), 0);
            assert_eq!(item.doc().expect("document").history.redo_depth(), 0);
            snapshot(item)
        });
        let original_doc =
            item.read_with(&visual, |item, _| item.doc().expect("document").clone());
        let original = before.3;
        let coordinate = if horizontal { original.x } else { original.y };
        let initial_text = if horizontal { "55.38" } else { "356.38" };
        assert_ne!(
            coordinate as f32,
            initial_text.parse::<f32>().expect("rounded display")
        );
        let selector = if horizontal {
            "fig-gpui-design-x"
        } else {
            "fig-gpui-design-y"
        };
        for terminal in ["blur", "enter", "invalid-cancel"] {
            let field_center = visual
                .debug_bounds(selector)
                .expect("mounted position input")
                .center();
            visual.simulate_click(field_center, gpui::Modifiers::none());
            draw_pane_layout(&mut visual);
            assert_eq!(
                panel.read_with(&visual, |panel, cx| panel.property_draft_state(cx)),
                DesignPropertyDraftState::Unchanged
            );
            item.read_with(&visual, |item, _| {
                assert_eq!(
                    snapshot(item),
                    before,
                    "focus must not author a rounded seed"
                )
            });
            view.read_with(&visual, |view, cx| {
                assert!(!view.is_dirty(cx));
                assert!(!view.has_pending_view_edits(cx));
            });
            visual.simulate_keystrokes("cmd-a cmd-c");
            assert_eq!(
                visual
                    .read_from_clipboard()
                    .expect("actual input")
                    .text()
                    .as_deref(),
                Some(initial_text)
            );
            match terminal {
                "blur" => view.update_in(&mut visual, |view, window, cx| {
                    view.focus_handle.focus(window, cx)
                }),
                "enter" => visual.simulate_keystrokes("enter"),
                "invalid-cancel" => {
                    visual.simulate_input("bad");
                    draw_pane_layout(&mut visual);
                    assert_eq!(
                        panel.read_with(&visual, |panel, cx| panel.property_draft_state(cx)),
                        DesignPropertyDraftState::Invalid
                    );
                    item.read_with(&visual, |item, _| {
                        assert_eq!(snapshot(item), before, "invalid text must not preview")
                    });
                    view.read_with(&visual, |view, cx| {
                        assert!(view.has_pending_view_edits(cx))
                    });
                    visual.simulate_keystrokes("escape");
                }
                _ => unreachable!(),
            }
            draw_pane_layout(&mut visual);
            assert_eq!(
                panel.read_with(&visual, |panel, cx| panel.property_draft_state(cx)),
                DesignPropertyDraftState::None
            );
            item.read_with(&visual, |item, _| {
                assert_eq!(
                    snapshot(item),
                    before,
                    "unchanged terminal must preserve authored state"
                )
            });
            view.read_with(&visual, |view, cx| assert!(!view.is_dirty(cx)));
        }
        let field_center = visual
            .debug_bounds(selector)
            .expect("position input")
            .center();
        visual.simulate_click(field_center, gpui::Modifiers::none());
        visual.simulate_keystrokes("cmd-a");
        let target = if horizontal { 72.125 } else { 392.125 };
        visual.simulate_input(&target.to_string());
        draw_pane_layout(&mut visual);
        item.read_with(&visual, |item, _| {
            let preview = snapshot(item);
            assert!(preview.6, "a genuine preview remains dirty");
            assert_eq!((preview.4, preview.5), (0, 0));
            let actual = if horizontal { preview.3.x } else { preview.3.y };
            assert!((actual - target).abs() < 1.0e-9);
        });
        visual.simulate_keystrokes("enter");
        draw_pane_layout(&mut visual);
        assert_eq!(
            panel.read_with(&visual, |panel, cx| panel.property_draft_state(cx)),
            DesignPropertyDraftState::None
        );
        item.read_with(&visual, |item, _| {
            let after = snapshot(item);
            assert!(after.6);
            assert_eq!((after.4, after.5), (1, 0));
            assert_eq!(after.1, before.1);
            let geometry = after.3;
            let actual = if horizontal { geometry.x } else { geometry.y };
            let other = if horizontal {
                geometry.y - original.y
            } else {
                geometry.x - original.x
            };
            assert!((actual - target).abs() < 1.0e-9);
            assert!(other.abs() < 1.0e-9);
            assert!((geometry.width - original.width).abs() < 1.0e-9);
            assert!((geometry.height - original.height).abs() < 1.0e-9);
            let doc = item.doc().expect("committed document");
            let mut expected_scene = original_doc.scene.clone();
            expected_scene
                .set_transform(id, after.2)
                .expect("expected position");
            assert_eq!(
                serde_json::to_value(&doc.scene).expect("committed scene"),
                serde_json::to_value(expected_scene).expect("expected scene")
            );
            assert_eq!(doc.selection.iter().copied().collect::<Vec<_>>(), vec![id]);
        });
    }
}

#[cfg(target_os = "macos")]
async fn canvas_audio_fixture(
    cx: &mut TestAppContext,
) -> Result<(
    Entity<FigItem>,
    Entity<FigView>,
    NodeId,
    NodeId,
    &mut gpui::VisualTestContext,
)> {
    init_visual_test(cx);
    let file_system = FakeFs::new(cx.executor());
    let project = Project::test(file_system, [], cx).await;
    let mut doc = Doc::new();
    let page = CanvasNode::new(NodeData::Group(GroupNode::default()));
    let page_id = page.id;
    doc.apply(Operation::create_node(page))?;
    doc.add_page(page_id);
    doc.set_active_page(Some(page_id));
    let asset = AssetId::new();
    doc.asset_library.insert(
        asset,
        fanta_doc::ProjectAsset {
            name: "One second tone.mp3".into(),
            kind: ProjectAssetKind::Audio,
        },
    );
    let mut audio = CanvasNode::new(NodeData::Audio(fanta_doc::AudioNode {
        asset,
        local_size: [320.0, 80.0],
        time_range_us: [0, 1_000_000],
        volume: 1.0,
        muted: false,
        waveform_color: Color::WHITE,
    }));
    audio.parent = Some(page_id);
    audio.name = "Selected audio".into();
    let audio_id = audio.id;
    doc.apply(Operation::create_node(audio))?;
    let mut control = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        0.0,
        0.0,
        40.0,
        40.0,
        Color::WHITE,
    )));
    control.parent = Some(page_id);
    control.name = "Untouched control".into();
    let control_id = control.id;
    doc.apply(Operation::create_node(control))?;
    doc.selection.select_only(audio_id);
    doc.history = Default::default();
    let item = crate::document::ready_item_for_test(
        &project,
        "/tmp/CanvasAudioLifecycle.fig".into(),
        doc,
        cx,
    );
    item.update(cx, |item, cx| {
        item.with_document(cx, |document| {
            Arc::make_mut(&mut document.raw_assets).insert(
                asset,
                include_bytes!("../../tests/fixtures/one_second_tone.mp3").to_vec(),
            );
            ((), DocChange::None)
        })
    })
    .context("seed the mounted Audio asset")?;
    let (workspace, visual) = cx.add_window_view(|window, cx| {
        workspace::Workspace::test_new(project.clone(), window, cx)
    });
    let view = visual.update(|window, cx| {
        let view = cx.new(|cx| FigView::new(item.clone(), project, window, cx));
        workspace.update(cx, |workspace, cx| {
            workspace.add_item_to_active_pane(Box::new(view.clone()), None, true, window, cx);
        });
        view.update(cx, |view, cx| {
            view.set_editor_workspace(EditorWorkspace::Canvas, cx);
            view.set_editor_mode(EditorMode::Design, cx);
        });
        window.activate_window();
        view
    });
    visual.run_until_parked();
    Ok((item, view, audio_id, control_id, visual))
}

#[cfg(target_os = "macos")]
fn assert_canvas_audio_document(
    item: &Entity<FigItem>,
    expected: &Doc,
    assets: &BTreeMap<AssetId, Vec<u8>>,
    cx: &mut gpui::VisualTestContext,
) -> Result<()> {
    item.read_with(cx, |item, _| {
        let document = item.document().context("mounted Audio document")?;
        assert_eq!(
            serde_json::to_value(&document.doc)?,
            serde_json::to_value(expected)?
        );
        assert_eq!(document.raw_assets.as_ref(), assets);
        assert_eq!(document.doc.history.undo_depth(), 0);
        assert_eq!(document.doc.history.redo_depth(), 0);
        assert!(!item.is_dirty());
        Ok(())
    })
}

#[cfg(target_os = "macos")]
fn assert_canvas_audio_panel(
    view: &Entity<FigView>,
    selected: Option<NodeId>,
    cx: &mut gpui::VisualTestContext,
) -> Result<Option<gpui::EntityId>> {
    let playback = view.read_with(cx, |view, _| {
        assert_eq!(
            view.canvas_audio
                .as_ref()
                .map(|session| session.source.node),
            selected
        );
        view.canvas_audio
            .as_ref()
            .map(|session| session.playback.entity_id())
    });
    let bounds = cx.debug_bounds("canvas-audio-controls");
    if selected.is_some() {
        let bounds = bounds.context("mounted Audio controls must be rendered")?;
        assert!(bounds.size.width > px(0.0) && bounds.size.height > px(0.0));
    } else {
        assert!(
            bounds.is_none(),
            "ineligible selection must not keep stale Audio controls"
        );
    }
    Ok(playback)
}

#[cfg(target_os = "macos")]
#[gpui::test]
async fn canvas_audio_window_activation_restores_selected_preview(cx: &mut TestAppContext) {
    let result: Result<()> = async {
        let (item, view, audio, _, visual) = canvas_audio_fixture(cx).await?;
        let (expected, assets) = item.read_with(visual, |item, _| {
            let document = item.document().context("Audio baseline")?;
            Ok::<_, anyhow::Error>((document.doc.clone(), document.raw_assets.clone()))
        })?;
        assert!(visual.update(|window, _| window.is_window_active()));
        view.update(visual, |view, cx| {
            view.set_editor_mode(EditorMode::Motion, cx)
        });
        visual.run_until_parked();
        let mut previous = assert_canvas_audio_panel(&view, Some(audio), visual)?;
        assert_canvas_audio_document(&item, &expected, assets.as_ref(), visual)?;
        for _ in 0..2 {
            visual.deactivate_window();
            visual.run_until_parked();
            assert!(!visual.update(|window, _| window.is_window_active()));
            assert_canvas_audio_panel(&view, None, visual)?;
            assert_canvas_audio_document(&item, &expected, assets.as_ref(), visual)?;
            visual.update(|window, _| window.activate_window());
            visual.run_until_parked();
            assert!(visual.update(|window, _| window.is_window_active()));
            let current = assert_canvas_audio_panel(&view, Some(audio), visual)?;
            assert_ne!(current, previous, "activation must create a fresh preview");
            previous = current;
            assert_canvas_audio_document(&item, &expected, assets.as_ref(), visual)?;
        }
        Ok(())
    }
    .await;
    assert!(
        result.is_ok(),
        "Audio activation regression setup/check failed: {result:?}"
    );
}

#[cfg(target_os = "macos")]
#[gpui::test]
async fn canvas_audio_eligibility_tracks_mode_and_selection_across_activation(
    cx: &mut TestAppContext,
) {
    let result: Result<()> = async {
        let (item, view, audio, control, visual) = canvas_audio_fixture(cx).await?;
        let (mut expected, assets) = item.read_with(visual, |item, _| {
            let document = item.document().context("Audio baseline")?;
            Ok::<_, anyhow::Error>((document.doc.clone(), document.raw_assets.clone()))
        })?;
        assert_canvas_audio_panel(&view, None, visual)?;
        view.update(visual, |view, cx| {
            view.set_editor_mode(EditorMode::Motion, cx)
        });
        visual.run_until_parked();
        assert_canvas_audio_panel(&view, Some(audio), visual)?;
        assert_canvas_audio_document(&item, &expected, assets.as_ref(), visual)?;

        visual.deactivate_window();
        item.update(visual, |item, cx| {
            item.with_document(cx, |document| {
                document.doc.selection.select_only(control);
                ((), DocChange::Selection)
            })
        })
        .context("select the control through FigItem")?;
        expected.selection.select_only(control);
        visual.run_until_parked();
        assert_canvas_audio_panel(&view, None, visual)?;
        assert_canvas_audio_document(&item, &expected, assets.as_ref(), visual)?;
        visual.update(|window, _| window.activate_window());
        visual.run_until_parked();
        assert_canvas_audio_panel(&view, None, visual)?;
        item.update(visual, |item, cx| {
            item.with_document(cx, |document| {
                document.doc.selection.select_only(audio);
                ((), DocChange::Selection)
            })
        })
        .context("restore Audio selection through FigItem")?;
        expected.selection.select_only(audio);
        visual.run_until_parked();
        assert_canvas_audio_panel(&view, Some(audio), visual)?;
        assert_canvas_audio_document(&item, &expected, assets.as_ref(), visual)?;

        visual.deactivate_window();
        for mode in [EditorMode::Design, EditorMode::Motion] {
            view.update(visual, |view, cx| view.set_editor_mode(mode, cx));
            visual.run_until_parked();
            assert!(!visual.update(|window, _| window.is_window_active()));
            assert_canvas_audio_panel(&view, None, visual)?;
            assert_canvas_audio_document(&item, &expected, assets.as_ref(), visual)?;
        }
        visual.update(|window, _| window.activate_window());
        visual.run_until_parked();
        assert_canvas_audio_panel(&view, Some(audio), visual)?;
        view.update(visual, |view, cx| {
            view.set_editor_mode(EditorMode::Design, cx)
        });
        visual.run_until_parked();
        assert_canvas_audio_panel(&view, None, visual)?;
        assert_canvas_audio_document(&item, &expected, assets.as_ref(), visual)?;
        Ok(())
    }
    .await;
    assert!(
        result.is_ok(),
        "Audio eligibility regression setup/check failed: {result:?}"
    );
}

async fn local_media_picker_fixture(
    cx: &mut TestAppContext,
) -> (
    tempfile::TempDir,
    std::path::PathBuf,
    Entity<workspace::Workspace>,
    Entity<FigItem>,
    Entity<FigView>,
    &mut gpui::VisualTestContext,
) {
    init_visual_test(cx);
    let file_system = FakeFs::new(cx.executor());
    let project = Project::test(file_system.clone(), [], cx).await;
    let directory = tempfile::tempdir().expect("media directory");
    let path = directory.path().join("Placed image.png");
    image::RgbaImage::from_pixel(8, 4, image::Rgba([24, 96, 192, 255]))
        .save(&path)
        .expect("write picker image");
    let (mut doc, first_page, _) = doc_with_two_pages();
    let mut text = CanvasNode::new(NodeData::Text(TextNode::new("Saved text", 120., 40.)));
    text.parent = Some(first_page);
    doc.scene.insert(text).expect("initial text");
    let root = directory.path().join("New design");
    crate::document::write_project(&root, &doc, &BTreeMap::new()).expect("saved new design");
    file_system.insert_tree_from_real_fs(&root, &root).await;
    let item = crate::document::ready_item_with_root_for_test(
        &project,
        root.join("fanta.json"),
        Some(root),
        doc,
        cx,
    );
    let (workspace, cx) = cx.add_window_view(|window, cx| {
        workspace::Workspace::test_new(project.clone(), window, cx)
    });
    let view = cx.update(|window, cx| {
        let view = cx.new(|cx| FigView::new(item.clone(), project, window, cx));
        workspace.update(cx, |workspace, cx| {
            workspace.add_item_to_active_pane(Box::new(view.clone()), None, true, window, cx);
        });
        window.activate_window();
        view
    });
    cx.run_until_parked();
    (directory, path, workspace, item, view, cx)
}

#[gpui::test]
async fn local_media_picker_survives_native_window_deactivation(cx: &mut TestAppContext) {
    let (_directory, path, _workspace, item, view, cx) = local_media_picker_fixture(cx).await;
    let baseline = item.read_with(cx, |item, _| item.doc().expect("document").scene.clone());
    view.update(cx, |view, cx| view.choose_local_media(cx));
    assert!(cx.did_prompt_for_paths());
    cx.deactivate_window();
    cx.simulate_path_prompt_response(move |_| Some(vec![path]));
    cx.update(|window, _| window.activate_window());
    cx.run_until_parked();
    item.read_with(cx, |item, _| {
        let document = item.document().expect("document");
        assert_eq!(document.doc.scene.len(), baseline.len() + 1);
        let bitmap = document
            .doc
            .scene
            .roots()
            .iter()
            .flat_map(|root| document.doc.scene.descendants_of(*root))
            .filter_map(|id| document.doc.scene.get(id))
            .find(|node| matches!(node.data, NodeData::Bitmap(_)))
            .expect("picker places its image after focus returns");
        assert_eq!(bitmap.parent, document.doc.active_page());
        assert_eq!(bitmap.name, "Placed image.png");
        assert_eq!(document.raw_assets.len(), 1);
    });
    view.read_with(cx, |view, _| assert!(view.media_import_task.is_none()));
    item.update(cx, |item, cx| item.undo(cx))
        .expect("undo placement");
    item.read_with(cx, |item, _| {
        let document = item.document().expect("document");
        assert_eq!(document.doc.scene.len(), baseline.len());
        for id in baseline
            .roots()
            .iter()
            .flat_map(|root| baseline.descendants_of(*root))
        {
            assert_eq!(document.doc.scene.get(id), baseline.get(id));
        }
    });
}

#[gpui::test]
async fn local_media_picker_rejects_page_round_trip_during_preparation(
    cx: &mut TestAppContext,
) {
    let (_directory, path, _workspace, item, view, cx) = local_media_picker_fixture(cx).await;
    let prepared = crate::generation_media::prepare_local_media(&path)
        .await
        .expect("prepare image");
    let baseline_count =
        item.read_with(cx, |item, _| item.doc().expect("document").scene.len());
    let (prepared_sender, prepared_receiver) = futures::channel::oneshot::channel();
    let started = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let preparation_started = started.clone();
    view.update(cx, |view, cx| {
        view.choose_local_media_with(
            move |_| async move {
                preparation_started.store(true, std::sync::atomic::Ordering::SeqCst);
                prepared_receiver.await.context("preparation canceled")
            },
            cx,
        );
    });
    cx.deactivate_window();
    cx.simulate_path_prompt_response(move |_| Some(vec![path]));
    cx.run_until_parked();
    assert!(
        started.load(std::sync::atomic::Ordering::SeqCst),
        "unchanged document reaches preparation"
    );
    view.update(cx, |view, cx| {
        view.select_page(1, cx);
        view.select_page(0, cx);
    });
    assert!(prepared_sender.send(prepared).is_ok());
    cx.run_until_parked();
    item.read_with(cx, |item, _| {
        let document = item.document().expect("document");
        assert_eq!(document.doc.scene.len(), baseline_count);
        assert!(document.raw_assets.is_empty());
    });
    view.read_with(cx, |view, _| assert!(view.media_import_task.is_none()));
}

#[gpui::test]
async fn local_media_picker_rejects_switching_to_another_document(cx: &mut TestAppContext) {
    let (_directory, path, workspace, item, view, cx) = local_media_picker_fixture(cx).await;
    let baseline_count =
        item.read_with(cx, |item, _| item.doc().expect("document").scene.len());
    view.update(cx, |view, cx| view.choose_local_media(cx));
    cx.deactivate_window();
    let project = view.read_with(cx, |view, _| view.project.clone());
    let other_item = crate::document::ready_item_for_test(
        &project,
        "/tmp/Other-media-design.fig".into(),
        doc_with_one_page(),
        cx,
    );
    let other_view = cx.update(|window, cx| {
        let other_view = cx.new(|cx| FigView::new(other_item.clone(), project, window, cx));
        workspace.update(cx, |workspace, cx| {
            workspace.add_item_to_active_pane(
                Box::new(other_view.clone()),
                None,
                true,
                window,
                cx,
            );
        });
        window.activate_window();
        other_view
    });
    cx.run_until_parked();
    assert_eq!(
        workspace.read_with(cx, |workspace, cx| workspace
            .active_item(cx)
            .map(|item| item.item_id())),
        Some(other_view.entity_id())
    );
    cx.simulate_path_prompt_response(move |_| Some(vec![path]));
    cx.run_until_parked();
    for (item, expected_count) in [(item, baseline_count), (other_item, 1)] {
        item.read_with(cx, |item, _| {
            let document = item.document().expect("document");
            assert_eq!(document.doc.scene.len(), expected_count);
            assert!(document.raw_assets.is_empty());
        });
    }
    view.read_with(cx, |view, _| assert!(view.media_import_task.is_none()));
}

#[gpui::test]
async fn save_as_updates_shared_views_and_their_project_entries(cx: &mut TestAppContext) {
    let result: Result<()> = async {
        init_visual_test(cx);
        let directory = tempfile::tempdir()?;
        let original = directory.path().join("Original");
        let copy = directory.path().join("Copy");
        let document = doc_with_one_page();
        crate::document::write_project(&original, &document, &BTreeMap::new())?;
        let file_system = FakeFs::new(cx.executor());
        file_system
            .insert_tree(
                directory.path(),
                serde_json::json!({"Original": {"fanta.json": "{}"}}),
            )
            .await;
        let project = Project::test(file_system.clone(), [directory.path()], cx).await;
        let original_entry = project.read_with(cx, |project, cx| {
            let path = project
                .find_project_path(original.join("fanta.json"), cx)
                .expect("original path");
            project
                .entry_for_path(&path, cx)
                .expect("original entry")
                .id
        });
        let destination = project
            .read_with(cx, |project, cx| project.find_project_path(&copy, cx))
            .context("copy path")?;
        let item = crate::document::ready_item_with_root_for_test(
            &project,
            original.join("fanta.json"),
            Some(original.clone()),
            document,
            cx,
        );
        let scratch = cx.add_window(|_, _| gpui::Empty);
        let views = scratch.update(cx, |_, window, cx| {
            (0..2)
                .map(|_| {
                    cx.new(|cx| {
                        let mut view = FigView::new(item.clone(), project.clone(), window, cx);
                        view.opened_entry_id = Some(original_entry);
                        view
                    })
                })
                .collect::<Vec<_>>()
        })?;
        let view = views.first().context("first view")?;
        view.read_with(cx, |view, cx| {
            assert!(view.can_save_as(cx));
            assert_eq!(view.suggested_filename(cx).as_ref(), "Original Copy");
        });
        add_rect(&item, cx);
        scratch
            .update(cx, |_, window, cx| {
                view.update(cx, |view, cx| {
                    Item::save_as(view, project.clone(), destination, window, cx)
                })
            })?
            .await?;
        file_system
            .insert_tree(&copy, serde_json::json!({"fanta.json": "{}"}))
            .await;
        cx.run_until_parked();
        let copied_entry = project.read_with(cx, |project, cx| {
            let path = project
                .find_project_path(copy.join("fanta.json"), cx)
                .expect("copied path");
            project.entry_for_path(&path, cx).expect("copied entry").id
        });
        for view in &views {
            view.read_with(cx, |view, cx| {
                assert_eq!(view.tab_content_text(0, cx).as_ref(), "Copy");
                assert_eq!(view.item.entity_id(), item.entity_id());
                assert_eq!(view.project_entry_ids(cx).as_slice(), &[copied_entry]);
                assert_ne!(copied_entry, original_entry);
                assert!(!view.is_dirty(cx));
            });
        }
        let (original_document, _) = fanta_format::read_project_tree(&original)?;
        let (copied_document, _) = fanta_format::read_project_tree(&copy)?;
        assert_eq!(original_document.scene.len(), 1);
        assert_eq!(copied_document.scene.len(), 2);
        Ok(())
    }
    .await;
    result.expect("Save As updates shared views and their project entries");
}

#[gpui::test]
async fn save_as_native_picker_starts_beside_the_original_project(cx: &mut TestAppContext) {
    let result: Result<()> = async {
        init_visual_test(cx);
        let directory = tempfile::tempdir()?;
        let original = directory.path().join("Original");
        let document = doc_with_one_page();
        crate::document::write_project(&original, &document, &BTreeMap::new())?;
        let file_system = FakeFs::new(cx.executor());
        file_system
            .insert_tree(&original, serde_json::json!({"fanta.json": "{}"}))
            .await;
        let project = Project::test(file_system, [original.as_path()], cx).await;
        let item = crate::document::ready_item_with_root_for_test(
            &project,
            original.join("fanta.json"),
            Some(original.clone()),
            document,
            cx,
        );
        item.update(cx, |item, cx| {
            item.path = project
                .read(cx)
                .find_project_path(item.abs_path(), cx)
                .expect("original path");
        });
        let scratch = cx.add_window(|_, _| gpui::Empty);
        let (workspace, view) = scratch.update(cx, |_, window, cx| {
            let workspace =
                cx.new(|cx| workspace::Workspace::test_new(project.clone(), window, cx));
            let view = cx.new(|cx| FigView::new(item.clone(), project, window, cx));
            workspace.update(cx, |workspace, cx| {
                workspace.add_item_to_active_pane(
                    Box::new(view.clone()),
                    None,
                    true,
                    window,
                    cx,
                )
            });
            (workspace, view)
        })?;
        assert_eq!(
            view.read_with(cx, |view, cx| view.suggested_filename(cx)),
            "Original Copy"
        );
        let save = scratch.update(cx, |_, window, cx| {
            workspace.update(cx, |workspace, cx| {
                workspace.save_active_item(workspace::SaveIntent::SaveAs, window, cx)
            })
        })?;
        cx.run_until_parked();
        assert!(
            cx.did_prompt_for_new_path(),
            "Save As reaches the native path prompt"
        );
        cx.simulate_new_path_selection(|initial_directory| {
            assert_eq!(
                initial_directory,
                directory.path(),
                "a copied design belongs beside its original, not inside it"
            );
            None
        });
        save.await?;
        assert_eq!(
            item.read_with(cx, |item, _| item
                .project_root()
                .map(std::path::Path::to_path_buf)),
            Some(original)
        );
        assert!(
            !directory.path().join("Original Copy").exists(),
            "cancel does not create a copy"
        );
        Ok(())
    }
    .await;
    result.expect("Save As uses the project parent as its native initial directory");
}

#[gpui::test]
async fn save_as_rejects_existing_design_before_opening_its_worktree(cx: &mut TestAppContext) {
    let result: Result<()> = async {
        init_visual_test(cx);
        cx.update(|cx| {
            workspace::register_project_item::<FigView>(cx);
            crate::workspace_hooks::init(cx);
        });
        let directory = tempfile::tempdir()?;
        let original = directory.path().join("Original");
        let copy = directory.path().join("Original Copy");
        let document = doc_with_one_page();
        let page = document.active_page().context("active page")?;
        for root in [&original, &copy] {
            crate::document::write_project(root, &document, &BTreeMap::new())?;
        }
        let original_manifest = std::fs::read(original.join("fanta.json"))?;
        let original_source =
            fanta_format::locate_page_source(&original, page).context("original page")?;
        let original_bytes = std::fs::read(&original_source)?;
        let copy_source =
            fanta_format::locate_page_source(&copy, page).context("copied page")?;
        let copy_bytes = std::fs::read(&copy_source)?;
        let file_system = FakeFs::new(cx.executor());
        file_system
            .insert_tree(
                directory.path(),
                serde_json::json!({
                    "Original": {"fanta.json": std::fs::read_to_string(original.join("fanta.json"))?},
                    "Original Copy": {"fanta.json": std::fs::read_to_string(copy.join("fanta.json"))?},
                }),
            )
            .await;
        let project = Project::test(file_system, [copy.as_path()], cx).await;
        let item = crate::document::ready_item_with_root_for_test(
            &project,
            copy.join("fanta.json"),
            Some(copy.clone()),
            document,
            cx,
        );
        item.update(cx, |item, cx| {
            item.path = project
                .read(cx)
                .find_project_path(item.abs_path(), cx)
                .expect("copied design path");
        });
        let scratch = cx.add_window(|_, _| gpui::Empty);
        let (workspace, view) = scratch.update(cx, |_, window, cx| {
            let workspace =
                cx.new(|cx| workspace::Workspace::test_new(project.clone(), window, cx));
            let view = cx.new(|cx| FigView::new(item.clone(), project.clone(), window, cx));
            workspace.update(cx, |workspace, cx| {
                workspace.add_item_to_active_pane(
                    Box::new(view.clone()),
                    None,
                    true,
                    window,
                    cx,
                );
            });
            (workspace, view)
        })?;
        cx.run_until_parked();
        assert_eq!(
            project.read_with(cx, |project, cx| project.worktrees(cx).count()),
            1
        );
        assert_eq!(
            workspace.read_with(cx, |workspace, cx| workspace.items_of_type::<FigView>(cx).count()),
            1
        );
        add_rect(&item, cx);
        let save = scratch.update(cx, |_, window, cx| {
            workspace.update(cx, |workspace, cx| {
                workspace.save_active_item(workspace::SaveIntent::SaveAs, window, cx)
            })
        })?;
        cx.run_until_parked();
        assert!(cx.did_prompt_for_new_path());
        cx.simulate_new_path_selection(|_| Some(original.clone()));
        let error = save.await.expect_err("an existing design rejects Save As");
        assert!(error.to_string().contains("is not empty"));
        cx.run_until_parked();
        project.read_with(cx, |project, cx| {
            assert_eq!(
                project.worktrees(cx).count(),
                1,
                "a rejected destination must not become a worktree"
            );
            assert!(project.find_project_path(&original, cx).is_none());
        });
        workspace.read_with(cx, |workspace, cx| {
            assert_eq!(
                workspace.items_of_type::<FigView>(cx).count(),
                1,
                "the destination must not open a tab"
            );
            assert_eq!(
                workspace.active_item(cx).map(|item| item.item_id()),
                Some(view.entity_id())
            );
        });
        assert_eq!(std::fs::read(original.join("fanta.json"))?, original_manifest);
        assert_eq!(std::fs::read(original_source)?, original_bytes);
        assert_eq!(std::fs::read(copy_source)?, copy_bytes);
        view.read_with(cx, |view, cx| {
            assert_eq!(view.item.read(cx).project_root(), Some(copy.as_path()));
            assert!(view.is_dirty(cx));
            assert!(view.autosave_task.is_some());
        });
        cx.executor().advance_clock(AUTOSAVE_DEBOUNCE * 2);
        cx.run_until_parked();
        assert_eq!(fanta_format::read_project_tree(&copy)?.0.scene.len(), 2);
        assert_eq!(fanta_format::read_project_tree(&original)?.0.scene.len(), 1);
        Ok(())
    }
    .await;
    result
        .expect("rejected Save As preserves the active design without opening its destination");
}

#[gpui::test]
async fn save_as_failure_resumes_autosave_on_the_original_design(cx: &mut TestAppContext) {
    let result: Result<()> = async {
        init_visual_test(cx);
        let directory = tempfile::tempdir()?;
        let original = directory.path().join("Original");
        let occupied = directory.path().join("Occupied");
        let document = doc_with_one_page();
        crate::document::write_project(&original, &document, &BTreeMap::new())?;
        std::fs::create_dir(&occupied)?;
        std::fs::write(occupied.join("keep.txt"), "untouched")?;
        let file_system = FakeFs::new(cx.executor());
        file_system
            .insert_tree(
                directory.path(),
                serde_json::json!({"Original": {"fanta.json": "{}"}, "Occupied": {}}),
            )
            .await;
        let project = Project::test(file_system, [directory.path()], cx).await;
        let destination = project
            .read_with(cx, |project, cx| project.find_project_path(&occupied, cx))
            .context("occupied path")?;
        let item = crate::document::ready_item_with_root_for_test(
            &project,
            original.join("fanta.json"),
            Some(original.clone()),
            document,
            cx,
        );
        let scratch = cx.add_window(|_, _| gpui::Empty);
        let view = scratch.update(cx, |_, window, cx| {
            cx.new(|cx| FigView::new(item.clone(), project.clone(), window, cx))
        })?;
        add_rect(&item, cx);
        let save_as = scratch.update(cx, |_, window, cx| {
            view.update(cx, |view, cx| {
                Item::save_as(view, project, destination, window, cx)
            })
        })?;
        assert!(
            save_as.await.is_err(),
            "the occupied destination must reject Save As"
        );
        cx.run_until_parked();
        view.read_with(cx, |view, cx| {
            assert!(
                view.autosave_task.is_some(),
                "the failed copy re-arms autosave"
            );
            assert!(view.is_dirty(cx));
            assert_eq!(view.item.read(cx).project_root(), Some(original.as_path()));
        });
        assert_eq!(fanta_format::read_project_tree(&original)?.0.scene.len(), 1);
        cx.executor().advance_clock(AUTOSAVE_DEBOUNCE * 2);
        cx.run_until_parked();
        assert_eq!(fanta_format::read_project_tree(&original)?.0.scene.len(), 2);
        assert!(!item.read_with(cx, |item, _| item.is_dirty()));
        assert_eq!(
            std::fs::read_to_string(occupied.join("keep.txt"))?,
            "untouched"
        );
        assert!(!occupied.join("fanta.json").exists());
        Ok(())
    }
    .await;
    result.expect("a failed Save As must not disable autosaving the original design");
}

/// The hero promise: an edit reaches the project tree on its own, so
/// `git diff` shows it without the user pressing cmd-s.
#[gpui::test]
async fn an_edit_autosaves_the_project_after_the_debounce(cx: &mut TestAppContext) {
    let (_dir, root, item, view) = autosave_fixture(cx).await;
    add_rect(&item, cx);
    item.read_with(cx, |item, _| assert!(item.is_dirty()));

    cx.executor().advance_clock(AUTOSAVE_DEBOUNCE * 2);
    cx.run_until_parked();

    item.read_with(cx, |item, _| {
        assert!(
            !item.is_dirty(),
            "the debounce elapsed and the canvas saved"
        )
    });
    assert!(
        root.join("fanta.json").is_file(),
        "the project tree was written to disk"
    );
    view.read_with(cx, |view, _| assert!(view.autosave_task.is_none()));
}

/// A failing autosave is reported once, however many debounced retries
/// keep failing, and the report clears when a save succeeds.
#[cfg(unix)]
#[gpui::test]
async fn a_failing_autosave_is_reported_until_a_save_succeeds(cx: &mut TestAppContext) {
    use std::os::unix::fs::PermissionsExt as _;

    let (_dir, root, item, view) = autosave_fixture(cx).await;
    // A loaded project knows the disk it came from; a failed save resyncs
    // against it.
    let (on_disk, assets) = fanta_format::read_project_tree(&root).expect("project on disk");
    item.update(cx, |item, _| item.set_merge_base_for_test(on_disk, assets));
    // Every directory, so no write (temp file plus rename) can land anywhere.
    fn collect_directories(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        out.push(dir.to_path_buf());
        for entry in std::fs::read_dir(dir)
            .expect("read project directory")
            .flatten()
        {
            if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                collect_directories(&entry.path(), out);
            }
        }
    }
    let mut directories = Vec::new();
    collect_directories(&root, &mut directories);
    let set_mode = |mode| {
        for directory in &directories {
            std::fs::set_permissions(directory, std::fs::Permissions::from_mode(mode))
                .expect("set directory permissions");
        }
    };
    set_mode(0o555);

    add_rect(&item, cx);
    cx.executor().advance_clock(AUTOSAVE_DEBOUNCE * 2);
    cx.run_until_parked();
    let reported = view.read_with(cx, |view, _| view.reported_save_failure.clone());
    assert!(reported.is_some(), "a read-only project fails the autosave");
    item.read_with(cx, |item, _| assert!(item.is_dirty()));

    add_rect(&item, cx);
    cx.executor().advance_clock(AUTOSAVE_DEBOUNCE * 2);
    cx.run_until_parked();
    view.read_with(cx, |view, _| {
        assert_eq!(
            view.reported_save_failure, reported,
            "a retry that keeps failing does not raise the notice again"
        )
    });

    set_mode(0o755);
    add_rect(&item, cx);
    cx.executor().advance_clock(AUTOSAVE_DEBOUNCE * 2);
    cx.run_until_parked();
    item.read_with(cx, |item, _| {
        assert!(!item.is_dirty(), "the save went through")
    });
    view.read_with(cx, |view, _| {
        assert!(
            view.reported_save_failure.is_none(),
            "a successful save clears the report"
        )
    });
}

struct SaveFailureNoticeFixture {
    _directory: tempfile::TempDir,
    root: std::path::PathBuf,
    project: Entity<Project>,
    item: Entity<FigItem>,
    view: Entity<FigView>,
    workspace: Entity<workspace::Workspace>,
    window: gpui::WindowHandle<MultiWorkspace>,
}

async fn save_failure_notice_fixture(
    name: &str,
    cx: &mut TestAppContext,
) -> Result<SaveFailureNoticeFixture> {
    use project::ProjectItem as _;

    cx.executor().allow_parking();
    let directory = tempfile::tempdir()?;
    let root = directory.path().join(name);
    crate::document::write_project(&root, &doc_with_one_page(), &BTreeMap::new())?;
    let file_system = Arc::new(fs::RealFs::new(None, cx.executor()));
    let project = Project::test(file_system, [root.as_path()], cx).await;
    let worktree_id = project.read_with(cx, |project, cx| {
        project
            .worktrees(cx)
            .next()
            .expect("fixture worktree")
            .read(cx)
            .id()
    });
    let item = cx
        .update(|cx| {
            FigItem::try_open(
                &project,
                &project::ProjectPath {
                    worktree_id,
                    path: util::rel_path::rel_path("fanta.json").into(),
                },
                cx,
            )
        })
        .context("native fixture opener")?
        .await?;
    cx.run_until_parked();
    assert!(item.read_with(cx, |item, _| item.has_ready_document()));
    let window =
        cx.add_window(|window, cx| MultiWorkspace::test_new(project.clone(), window, cx));
    let workspace = window
        .entity(cx)?
        .read_with(cx, |root, _| root.workspace().clone());
    let view = window.update(cx, |_, window, cx| {
        let view = cx.new(|cx| FigView::new(item.clone(), project.clone(), window, cx));
        workspace.update(cx, |workspace, cx| {
            workspace.add_item_to_active_pane(Box::new(view.clone()), None, true, window, cx);
        });
        window.activate_window();
        view
    })?;
    cx.run_until_parked();
    Ok(SaveFailureNoticeFixture {
        _directory: directory,
        root,
        project,
        item,
        view,
        workspace,
        window,
    })
}

fn save_failure_notice_document(
    item: &Entity<FigItem>,
    cx: &TestAppContext,
) -> (
    serde_json::Value,
    BTreeMap<AssetId, Vec<u8>>,
    usize,
    usize,
    bool,
) {
    item.read_with(cx, |item, _| {
        let document = item.document().expect("ready notification fixture");
        (
            serde_json::to_value(&document.doc).expect("complete document"),
            document.raw_assets.as_ref().clone(),
            document.doc.history.undo_depth(),
            document.doc.history.redo_depth(),
            item.is_dirty(),
        )
    })
}

fn assert_save_failure_notice(
    fixture: &SaveFailureNoticeFixture,
    present: bool,
    cx: &TestAppContext,
) {
    assert_eq!(
        fixture.workspace.read_with(cx, |workspace, _| {
            workspace
                .notification_ids()
                .contains(&save_failure_notice_id(fixture.item.entity_id()))
        }),
        present
    );
}

fn report_notice_fixture_failure(view: &Entity<FigView>, cx: &mut TestAppContext) {
    let error = anyhow::Error::new(std::io::Error::from(std::io::ErrorKind::PermissionDenied))
        .context("writing /private/fixture/.fanta-transaction-build-private");
    view.update(cx, |view, cx| view.report_save_failure(&error, cx));
}

#[cfg(unix)]
#[gpui::test]
async fn save_failure_notice_real_write_error_is_concise_and_recovery_is_scoped(
    cx: &mut TestAppContext,
) {
    let result: Result<()> = async {
        init_visual_test(cx);
        cx.update(|cx| cx.set_global(db::AppDatabase::test_new()));
        let first = save_failure_notice_fixture("First design", cx).await?;
        let other = save_failure_notice_fixture("Other design", cx).await?;
        report_notice_fixture_failure(&other.view, cx);
        cx.run_until_parked();
        let files = autosave_project_files(&first.root)?;
        let mut fault = AutosaveDirectoryFault::apply(&first.root)?;
        add_rect(&first.item, cx);
        let document = save_failure_notice_document(&first.item, cx);
        settle_autosave_and_resync(cx);
        assert_save_failure_notice(&first, true, cx);
        assert_save_failure_notice(&other, true, cx);
        first.view.read_with(cx, |view, _| {
            assert!(
                view.reported_save_failure
                    .as_ref()
                    .is_some_and(|error| error.contains("Permission denied"))
            );
            let state = view.save_failure_notice.borrow();
            let message = state.as_ref().expect("persistent concise notice");
            assert!(message.contains("First design"));
            assert!(message.contains("Write access was denied"));
            assert!(message.contains("try Save again"));
            assert!(message.len() < 200);
            assert!(!message.contains(first.root.to_string_lossy().as_ref()));
            assert!(!message.contains(".fanta-transaction"));
        });
        assert_eq!(autosave_project_files(&first.root)?, files);
        assert_eq!(save_failure_notice_document(&first.item, cx), document);
        fault.restore()?;
        first
            .view
            .update(cx, |view, cx| view.save_document(cx))
            .await?;
        cx.run_until_parked();
        assert_save_failure_notice(&first, false, cx);
        assert_save_failure_notice(&other, true, cx);
        assert_autosave_persisted_draft(&first.root, &first.item, cx)?;
        Ok(())
    }
    .await;
    result.expect(
        "a real failed write is explained briefly without clearing another design's warning",
    );
}

#[gpui::test]
async fn save_failure_notice_discard_removes_only_the_closed_document(cx: &mut TestAppContext) {
    let result: Result<()> = async {
        init_visual_test(cx);
        cx.update(|cx| cx.set_global(db::AppDatabase::test_new()));
        let first = save_failure_notice_fixture("Discard design", cx).await?;
        let other = save_failure_notice_fixture("Keep design", cx).await?;
        let files = autosave_project_files(&first.root)?;
        let other_document = save_failure_notice_document(&other.item, cx);
        add_rect(&first.item, cx);
        report_notice_fixture_failure(&first.view, cx);
        report_notice_fixture_failure(&other.view, cx);
        cx.run_until_parked();
        assert_save_failure_notice(&first, true, cx);
        assert_save_failure_notice(&other, true, cx);
        let close = first.window.update(cx, |_, window, cx| {
            let pane = first.workspace.read(cx).active_pane().clone();
            pane.update(cx, |pane, cx| {
                pane.close_item_by_id(
                    first.view.entity_id(),
                    workspace::SaveIntent::Skip,
                    window,
                    cx,
                )
            })
        })?;
        close.await?;
        cx.run_until_parked();
        assert_eq!(
            first.workspace.read_with(cx, |workspace, cx| workspace
                .items_of_type::<FigView>(cx)
                .count()),
            0
        );
        assert_save_failure_notice(&first, false, cx);
        assert_save_failure_notice(&other, true, cx);
        assert!(
            first
                .view
                .read_with(cx, |view, _| view.save_failure_notice.borrow().is_none())
        );
        assert_eq!(autosave_project_files(&first.root)?, files);
        assert_eq!(
            save_failure_notice_document(&other.item, cx),
            other_document
        );
        Ok(())
    }
    .await;
    result.expect(
        "Discard clears only its document's notice even while the removed view is retained",
    );
}

#[gpui::test]
async fn save_failure_notice_survives_one_shared_tab_until_the_last_removal(
    cx: &mut TestAppContext,
) {
    let result: Result<()> = async {
        init_visual_test(cx);
        cx.update(|cx| cx.set_global(db::AppDatabase::test_new()));
        let fixture = save_failure_notice_fixture("Shared design", cx).await?;
        let sibling = fixture.window.update(cx, |_, window, cx| {
            let sibling = cx.new(|cx| {
                FigView::new(fixture.item.clone(), fixture.project.clone(), window, cx)
            });
            fixture.workspace.update(cx, |workspace, cx| {
                workspace.split_item(
                    workspace::SplitDirection::Right,
                    Box::new(sibling.clone()),
                    window,
                    cx,
                )
            });
            sibling
        })?;
        assert_ne!(fixture.view.entity_id(), sibling.entity_id());
        assert_eq!(
            sibling.read_with(cx, |view, _| view.item.entity_id()),
            fixture.item.entity_id()
        );
        assert_eq!(
            fixture.workspace.read_with(cx, |workspace, cx| workspace
                .items_of_type::<FigView>(cx)
                .count()),
            2
        );
        let document = save_failure_notice_document(&fixture.item, cx);
        let files = autosave_project_files(&fixture.root)?;
        report_notice_fixture_failure(&fixture.view, cx);
        cx.run_until_parked();
        assert_save_failure_notice(&fixture, true, cx);
        for (view, expected) in [(&fixture.view, true), (&sibling, false)] {
            fixture.window.update(cx, |_, window, cx| {
                let panes = fixture.workspace.read(cx).panes().to_vec();
                for pane in panes {
                    pane.update(cx, |pane, cx| {
                        pane.remove_item(view.entity_id(), false, false, window, cx)
                    });
                }
            })?;
            cx.run_until_parked();
            assert_save_failure_notice(&fixture, expected, cx);
        }
        assert!(
            fixture
                .view
                .read_with(cx, |view, _| view.save_failure_notice.borrow().is_none())
        );
        assert_eq!(save_failure_notice_document(&fixture.item, cx), document);
        assert_eq!(autosave_project_files(&fixture.root)?, files);
        Ok(())
    }
    .await;
    result.expect("removing one tab preserves a shared document warning until its last mounted owner leaves");
}

#[gpui::test]
async fn save_failure_notice_deferred_show_cannot_outlive_save_or_removal(
    cx: &mut TestAppContext,
) {
    let result: Result<()> = async {
        init_visual_test(cx);
        cx.update(|cx| cx.set_global(db::AppDatabase::test_new()));
        let fixture = save_failure_notice_fixture("Deferred design", cx).await?;
        let document = save_failure_notice_document(&fixture.item, cx);
        let files = autosave_project_files(&fixture.root)?;
        fixture.window.update(cx, |_, _, cx| {
            let error = anyhow::anyhow!("queued diagnostic");
            fixture
                .view
                .update(cx, |view, cx| view.report_save_failure(&error, cx));
            fixture
                .item
                .update(cx, |_, cx| cx.emit(FigItemEvent::Saved));
        })?;
        cx.run_until_parked();
        assert_save_failure_notice(&fixture, false, cx);
        fixture.window.update(cx, |_, window, cx| {
            let error = anyhow::anyhow!("queued diagnostic before removal");
            fixture
                .view
                .update(cx, |view, cx| view.report_save_failure(&error, cx));
            let pane = fixture.workspace.read(cx).active_pane().clone();
            pane.update(cx, |pane, cx| {
                pane.remove_item(fixture.view.entity_id(), false, false, window, cx)
            });
        })?;
        cx.run_until_parked();
        assert_save_failure_notice(&fixture, false, cx);
        assert!(
            fixture
                .view
                .read_with(cx, |view, _| view.save_failure_notice.borrow().is_none())
        );
        assert_eq!(save_failure_notice_document(&fixture.item, cx), document);
        assert_eq!(autosave_project_files(&fixture.root)?, files);
        Ok(())
    }
    .await;
    result.expect("queued notification work reconciles current document ownership and successful-save state");
}

#[cfg(unix)]
struct AutosaveDirectoryFault {
    permissions: Vec<(std::path::PathBuf, std::fs::Permissions)>,
}

#[cfg(unix)]
impl AutosaveDirectoryFault {
    fn apply(root: &std::path::Path) -> Result<Self> {
        use std::os::unix::fs::PermissionsExt as _;

        fn collect(
            directory: &std::path::Path,
            permissions: &mut Vec<(std::path::PathBuf, std::fs::Permissions)>,
        ) -> Result<()> {
            permissions.push((
                directory.to_path_buf(),
                std::fs::metadata(directory)?.permissions(),
            ));
            for entry in std::fs::read_dir(directory)? {
                let entry = entry?;
                let kind = entry.file_type()?;
                anyhow::ensure!(!kind.is_symlink(), "unexpected symlink in save fixture");
                if kind.is_dir() {
                    collect(&entry.path(), permissions)?;
                }
            }
            Ok(())
        }
        let mut fault = Self {
            permissions: Vec::new(),
        };
        collect(root, &mut fault.permissions)?;
        for (directory, permissions) in &fault.permissions {
            std::fs::set_permissions(
                directory,
                std::fs::Permissions::from_mode(permissions.mode() & !0o222),
            )?;
        }
        Ok(fault)
    }

    fn restore(&mut self) -> Result<()> {
        for (directory, permissions) in &self.permissions {
            std::fs::set_permissions(directory, permissions.clone())?;
        }
        self.permissions.clear();
        Ok(())
    }
}

#[cfg(unix)]
impl Drop for AutosaveDirectoryFault {
    fn drop(&mut self) {
        for (directory, permissions) in &self.permissions {
            if let Err(error) = std::fs::set_permissions(directory, permissions.clone()) {
                log::error!(
                    "restoring save fixture {} failed: {error}",
                    directory.display()
                );
            }
        }
    }
}

fn autosave_project_files(
    root: &std::path::Path,
) -> Result<BTreeMap<std::path::PathBuf, Vec<u8>>> {
    fn collect(
        root: &std::path::Path,
        directory: &std::path::Path,
        files: &mut BTreeMap<std::path::PathBuf, Vec<u8>>,
    ) -> Result<()> {
        for entry in std::fs::read_dir(directory)? {
            let entry = entry?;
            let kind = entry.file_type()?;
            anyhow::ensure!(!kind.is_symlink(), "unexpected symlink in save fixture");
            if kind.is_dir() {
                collect(root, &entry.path(), files)?;
            } else {
                files.insert(
                    entry.path().strip_prefix(root)?.to_path_buf(),
                    std::fs::read(entry.path())?,
                );
            }
        }
        Ok(())
    }
    let mut files = BTreeMap::new();
    collect(root, root, &mut files)?;
    Ok(files)
}

#[cfg(unix)]
fn assert_autosave_persisted_draft(
    root: &std::path::Path,
    item: &Entity<FigItem>,
    cx: &TestAppContext,
) -> Result<()> {
    let (persisted, assets) = fanta_format::read_project_tree(root)?;
    item.read_with(cx, |item, _| -> Result<()> {
        let document = item.document.ready().context("ready save fixture")?;
        assert_eq!(
            serde_json::to_value(&persisted.scene)?,
            serde_json::to_value(&document.doc.scene)?
        );
        assert_eq!(&assets, document.raw_assets.as_ref());
        Ok(())
    })
}

#[cfg(unix)]
fn settle_autosave_and_resync(cx: &mut TestAppContext) {
    // Advance several separate deadlines so the resync's Edited event can
    // expose a re-armed debounce, rather than only testing the first timer.
    for _ in 0..4 {
        cx.executor().advance_clock(AUTOSAVE_DEBOUNCE * 2);
        cx.run_until_parked();
    }
}

#[cfg(unix)]
#[gpui::test]
async fn unchanged_failed_autosave_stays_dirty_without_repeated_writes(
    cx: &mut TestAppContext,
) {
    let result: Result<()> = async {
        let (_directory, root, item, view) = autosave_fixture(cx).await;
        let (baseline, assets) = fanta_format::read_project_tree(&root)?;
        item.update(cx, |item, _| item.set_merge_base_for_test(baseline, assets));
        let files = autosave_project_files(&root)?;
        let mut fault = AutosaveDirectoryFault::apply(&root)?;
        assert_eq!(
            item.read_with(cx, |item, _| item.save_attempts_for_test()),
            0
        );
        add_rect(&item, cx);
        let generation = item.read_with(cx, |item, _| {
            item.document
                .ready()
                .expect("ready fixture")
                .render_generation()
        });
        settle_autosave_and_resync(cx);
        assert_eq!(
            item.read_with(cx, |item, _| {
                item.document
                    .ready()
                    .expect("ready fixture")
                    .render_generation()
            }),
            generation,
            "unchanged-disk resync preserves the failed authored generation"
        );
        let notice = view.read_with(cx, |view, _| view.reported_save_failure.clone());
        assert!(
            notice
                .as_ref()
                .is_some_and(|error| error.to_ascii_lowercase().contains("permission denied"))
        );
        item.read_with(cx, |item, _| {
            assert_eq!(item.save_attempts_for_test(), 1);
            assert!(item.is_dirty());
            assert!(!item.external_reconciliation_pending());
            assert!(!item.has_conflict());
        });
        assert_eq!(autosave_project_files(&root)?, files);
        fault.restore()?;
        settle_autosave_and_resync(cx);
        item.read_with(cx, |item, _| {
            assert_eq!(
                item.save_attempts_for_test(),
                1,
                "permission recovery alone does not retry"
            );
            assert!(item.is_dirty());
        });
        view.read_with(cx, |view, _| {
            assert_eq!(view.reported_save_failure, notice);
            assert!(view.autosave_task.is_none());
        });
        assert_eq!(autosave_project_files(&root)?, files);
        Ok(())
    }
    .await;
    result.expect("an unchanged failed autosave retains its draft and notice without looping");
}

#[cfg(unix)]
#[gpui::test]
async fn explicit_save_retries_the_unchanged_failed_autosave(cx: &mut TestAppContext) {
    let result: Result<()> = async {
        let (_directory, root, item, view) = autosave_fixture(cx).await;
        let (baseline, assets) = fanta_format::read_project_tree(&root)?;
        item.update(cx, |item, _| item.set_merge_base_for_test(baseline, assets));
        let files = autosave_project_files(&root)?;
        let mut fault = AutosaveDirectoryFault::apply(&root)?;
        add_rect(&item, cx);
        settle_autosave_and_resync(cx);
        let error = view
            .update(cx, |view, cx| view.save_document(cx))
            .await
            .expect_err("explicit retry still reaches the read-only directory");
        assert!(error.chain().any(|error| {
            error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|error| error.kind() == std::io::ErrorKind::PermissionDenied)
        }));
        settle_autosave_and_resync(cx);
        assert_eq!(
            item.read_with(cx, |item, _| item.save_attempts_for_test()),
            2
        );
        assert_eq!(autosave_project_files(&root)?, files);
        assert!(view.read_with(cx, |view, _| view.reported_save_failure.is_some()));
        fault.restore()?;
        view.update(cx, |view, cx| view.save_document(cx)).await?;
        cx.run_until_parked();
        assert_eq!(
            item.read_with(cx, |item, _| item.save_attempts_for_test()),
            3
        );
        assert!(!item.read_with(cx, |item, _| item.is_dirty()));
        assert!(view.read_with(cx, |view, _| view.reported_save_failure.is_none()));
        assert_eq!(fanta_format::read_project_tree(&root)?.0.scene.len(), 2);
        assert_autosave_persisted_draft(&root, &item, cx)?;
        add_rect(&item, cx);
        settle_autosave_and_resync(cx);
        assert_eq!(
            item.read_with(cx, |item, _| item.save_attempts_for_test()),
            4
        );
        assert!(!item.read_with(cx, |item, _| item.is_dirty()));
        assert_eq!(fanta_format::read_project_tree(&root)?.0.scene.len(), 3);
        assert_autosave_persisted_draft(&root, &item, cx)?;
        Ok(())
    }
    .await;
    result.expect("explicit Save bypasses the failure stamp and success restores autosaving");
}

#[cfg(unix)]
#[gpui::test]
async fn new_edit_retries_after_an_autosave_failure(cx: &mut TestAppContext) {
    let result: Result<()> = async {
        let (_directory, root, item, view) = autosave_fixture(cx).await;
        let (baseline, assets) = fanta_format::read_project_tree(&root)?;
        item.update(cx, |item, _| item.set_merge_base_for_test(baseline, assets));
        let mut fault = AutosaveDirectoryFault::apply(&root)?;
        add_rect(&item, cx);
        settle_autosave_and_resync(cx);
        assert_eq!(
            item.read_with(cx, |item, _| item.save_attempts_for_test()),
            1
        );
        assert!(view.read_with(cx, |view, _| view.reported_save_failure.is_some()));
        fault.restore()?;
        add_rect(&item, cx);
        settle_autosave_and_resync(cx);
        assert_eq!(
            item.read_with(cx, |item, _| item.save_attempts_for_test()),
            2
        );
        assert!(!item.read_with(cx, |item, _| item.is_dirty()));
        assert!(view.read_with(cx, |view, _| view.reported_save_failure.is_none()));
        assert_eq!(fanta_format::read_project_tree(&root)?.0.scene.len(), 3);
        assert_autosave_persisted_draft(&root, &item, cx)?;
        Ok(())
    }
    .await;
    result.expect("a new committed edit remains eligible for automatic persistence");
}

#[cfg(unix)]
#[gpui::test]
async fn newer_edit_during_failed_autosave_remains_eligible(cx: &mut TestAppContext) {
    let result: Result<()> = async {
        let (_directory, root, item, view) = autosave_fixture(cx).await;
        let (baseline, assets) = fanta_format::read_project_tree(&root)?;
        item.update(cx, |item, _| item.set_merge_base_for_test(baseline, assets));
        let mut fault = AutosaveDirectoryFault::apply(&root)?;
        let (arrived, resume) = item.read_with(cx, |item, _| item.pause_next_save_for_test());
        add_rect(&item, cx);
        cx.executor().advance_clock(AUTOSAVE_DEBOUNCE * 2);
        cx.run_until_parked();
        arrived.await?;
        add_rect(&item, cx);
        resume.send(()).expect("resume the older snapshot's writer");
        cx.run_until_parked();
        assert_eq!(
            item.read_with(cx, |item, _| item.save_attempts_for_test()),
            1
        );
        assert!(view.read_with(cx, |view, _| {
            view.reported_save_failure
                .as_ref()
                .is_some_and(|error| error.to_ascii_lowercase().contains("permission denied"))
        }));
        assert_eq!(fanta_format::read_project_tree(&root)?.0.scene.len(), 1);
        fault.restore()?;
        settle_autosave_and_resync(cx);
        assert_eq!(
            item.read_with(cx, |item, _| item.save_attempts_for_test()),
            2
        );
        assert!(!item.read_with(cx, |item, _| item.is_dirty()));
        assert!(view.read_with(cx, |view, _| view.reported_save_failure.is_none()));
        assert_eq!(fanta_format::read_project_tree(&root)?.0.scene.len(), 3);
        assert_autosave_persisted_draft(&root, &item, cx)?;
        Ok(())
    }
    .await;
    result.expect("an older failed snapshot must not suppress newer in-flight edits");
}

#[cfg(unix)]
#[gpui::test]
async fn queued_identical_autosave_is_suppressed_after_the_first_failure(
    cx: &mut TestAppContext,
) {
    let result: Result<()> = async {
        let (_directory, root, item, _view) = autosave_fixture(cx).await;
        let (baseline, assets) = fanta_format::read_project_tree(&root)?;
        item.update(cx, |item, _| item.set_merge_base_for_test(baseline, assets));
        let files = autosave_project_files(&root)?;
        let mut fault = AutosaveDirectoryFault::apply(&root)?;
        add_rect(&item, cx);
        let (arrived, resume) = item.read_with(cx, |item, _| item.pause_next_save_for_test());
        let first = item.update(cx, |item, cx| item.save(SaveKind::Auto, cx));
        arrived.await?;
        let queued = item.update(cx, |item, cx| item.save(SaveKind::Auto, cx));
        add_rect(&item, cx);
        resume.send(()).expect("resume the first automatic writer");
        let error = first
            .await
            .expect_err("the first write reaches the denied directory");
        assert!(error.chain().any(|error| {
            error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|error| error.kind() == std::io::ErrorKind::PermissionDenied)
        }));
        assert!(queued.await?.is_none());
        assert_eq!(
            item.read_with(cx, |item, _| item.save_attempts_for_test()),
            1
        );
        assert!(item.read_with(cx, |item, _| item.is_dirty()));
        assert_eq!(autosave_project_files(&root)?, files);
        fault.restore()?;
        settle_autosave_and_resync(cx);
        assert_eq!(
            item.read_with(cx, |item, _| item.save_attempts_for_test()),
            2
        );
        assert!(!item.read_with(cx, |item, _| item.is_dirty()));
        assert_eq!(fanta_format::read_project_tree(&root)?.0.scene.len(), 3);
        assert_autosave_persisted_draft(&root, &item, cx)?;
        Ok(())
    }
    .await;
    result.expect("a queued duplicate must not enter the writer after an unchanged failure");
}

/// A drag longer than the debounce must not have an intermediate position
/// written under it: `mark_edited` announces the dirty transition on the
/// first preview frame, so the timer expires mid-gesture.
#[gpui::test]
async fn the_autosave_waits_for_the_pointer_to_come_up(cx: &mut TestAppContext) {
    let (_dir, root, item, view) = autosave_fixture(cx).await;
    let scratch = cx.add_window(|_, _| gpui::Empty);
    let position = point(px(10.), px(10.));
    scratch
        .update(cx, |_, window, cx| {
            view.update(cx, |view, cx| {
                view.handle_mouse_down(
                    &MouseDownEvent {
                        button: MouseButton::Left,
                        position,
                        modifiers: gpui::Modifiers::default(),
                        click_count: 1,
                        first_mouse: false,
                    },
                    window,
                    cx,
                );
            });
        })
        .expect("press the canvas");
    add_rect(&item, cx);

    cx.executor().advance_clock(AUTOSAVE_DEBOUNCE * 2);
    cx.run_until_parked();
    item.read_with(cx, |item, _| {
        assert!(item.is_dirty(), "a save mid-gesture would disrupt the drag")
    });
    assert_eq!(
        fanta_format::read_project_tree(&root)
            .expect("project on disk")
            .0
            .scene
            .len(),
        1,
        "the drag has not reached disk while the pointer is down"
    );

    scratch
        .update(cx, |_, window, cx| {
            view.update(cx, |view, cx| {
                view.handle_mouse_up(
                    &MouseUpEvent {
                        button: MouseButton::Left,
                        position,
                        modifiers: gpui::Modifiers::default(),
                        click_count: 1,
                    },
                    window,
                    cx,
                );
            });
        })
        .expect("release the canvas");
    cx.executor().advance_clock(AUTOSAVE_DEBOUNCE * 2);
    cx.run_until_parked();

    item.read_with(cx, |item, _| {
        assert!(
            !item.is_dirty(),
            "the release let the pending autosave through"
        )
    });
    assert!(root.join("fanta.json").is_file());
}

#[gpui::test]
async fn fill_mode_keeps_what_the_template_locks_against_delete_and_duplicate(
    cx: &mut TestAppContext,
) {
    init_test(cx);
    let project = Project::test(FakeFs::new(cx.executor()), [], cx).await;
    let mut doc = doc_with_one_page();
    let page = doc.active_page().expect("page");
    let mut logo = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        0.0,
        0.0,
        20.0,
        20.0,
        Color::BLACK,
    )));
    logo.parent = Some(page);
    logo.editing = Some(Box::new(fanta_doc::EditingRules {
        disabled_scopes: [Scope::LifecycleDestroy, Scope::LifecycleDuplicate].into(),
        placeholder: None,
    }));
    let logo_id = logo.id;
    doc.apply(Operation::create_node(logo)).expect("logo");
    doc.selection.select_only(logo_id);
    doc.history = Default::default();
    let item = crate::document::ready_item_for_test(
        &project,
        std::path::PathBuf::from("/tmp/Template.fig"),
        doc,
        cx,
    );
    let scratch = cx.add_window(|_, _| gpui::Empty);
    let view = scratch
        .update(cx, |_, window, cx| {
            cx.new(|cx| FigView::new(item.clone(), project.clone(), window, cx))
        })
        .expect("create fig view");
    let layers =
        |cx: &mut TestAppContext| item.read_with(cx, |item, _| item.doc().unwrap().scene.len());
    let before = layers(cx);

    scratch
        .update(cx, |_, window, cx| {
            view.update(cx, |view, cx| {
                view.toggle_template_fill(&ToggleTemplateFill, window, cx)
            })
        })
        .expect("fill mode");
    view.update(cx, |view, cx| {
        assert!(view.template_fill);
        view.duplicate_selected_nodes(cx);
        view.delete_selected_nodes(cx);
    });
    assert_eq!(layers(cx), before, "the template keeps its locked logo");

    scratch
        .update(cx, |_, window, cx| {
            view.update(cx, |view, cx| {
                view.toggle_template_fill(&ToggleTemplateFill, window, cx)
            })
        })
        .expect("authoring");
    view.update(cx, |view, cx| view.delete_selected_nodes(cx));
    assert_eq!(layers(cx), before - 1, "authoring deletes it");
}

/// A bare `.fig` has no project directory; the first save would scaffold
/// one next to it, which a timer must never do behind the user's back.
#[gpui::test]
async fn a_fig_without_a_project_is_never_autosaved(cx: &mut TestAppContext) {
    init_test(cx);
    let project = Project::test(FakeFs::new(cx.executor()), [], cx).await;
    let dir = tempfile::tempdir().expect("temp dir");
    let item = crate::document::ready_item_for_test(
        &project,
        dir.path().join("Design.fig"),
        doc_with_one_page(),
        cx,
    );
    let scratch = cx.add_window(|_, _| gpui::Empty);
    let view = scratch
        .update(cx, |_, window, cx| {
            cx.new(|cx| FigView::new(item.clone(), project.clone(), window, cx))
        })
        .expect("create fig view");
    add_rect(&item, cx);

    cx.executor().advance_clock(AUTOSAVE_DEBOUNCE * 2);
    cx.run_until_parked();

    item.read_with(cx, |item, _| assert!(item.is_dirty()));
    assert!(
        !dir.path().join("Design").exists(),
        "the autosave must not materialize a project directory"
    );
    view.read_with(cx, |view, _| assert!(view.autosave_task.is_none()));
}

/// The cursor must promise only what a press would actually start, and
/// the hover result is cached for `render` — never recomputed per frame.
#[gpui::test]
async fn hovering_a_handle_of_the_single_selected_node_caches_a_resize_cursor(
    cx: &mut TestAppContext,
) {
    init_test(cx);
    let project = Project::test(FakeFs::new(cx.executor()), [], cx).await;
    let mut doc = doc_with_one_page();
    let page = doc.active_page().expect("active page");
    let mut rect = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        0.0,
        0.0,
        100.0,
        50.0,
        Color::BLACK,
    )));
    rect.parent = Some(page);
    let rect_id = rect.id;
    doc.apply(Operation::create_node(rect))
        .expect("create rect");
    doc.selection.add(rect_id);
    let item = crate::document::ready_item_for_test(
        &project,
        std::path::PathBuf::from("/tmp/Design.fig"),
        doc,
        cx,
    );
    let scratch = cx.add_window(|_, _| gpui::Empty);
    let view = scratch
        .update(cx, |_, window, cx| {
            cx.new(|cx| FigView::new(item.clone(), project.clone(), window, cx))
        })
        .expect("create fig view");

    let screen_size = DVec2::new(800.0, 600.0);
    let viewport = Viewport {
        center: [0.0, 0.0],
        zoom: 1.0,
    };
    view.update(cx, |view, _| {
        view.viewport = Some(viewport);
        view.container_bounds = Some(Bounds {
            origin: point(px(0.), px(0.)),
            size: size(px(800.), px(600.)),
        });
    });
    let world = item
        .read_with(cx, |item, _| {
            item.doc().and_then(|doc| doc.scene.world_bounds(rect_id))
        })
        .expect("world bounds");

    for handle in fanta_canvas::ResizeHandle::ALL {
        let at = fanta_canvas::handles::handle_screen_position(
            handle,
            world,
            &viewport,
            screen_size,
        );
        view.update(cx, |view, cx| view.update_hover_resize_handle(at, cx));
        view.read_with(cx, |view, _| {
            assert_eq!(view.hover_resize_handle, Some(handle));
        });
    }

    // The middle of the node is not a handle.
    let north_west = fanta_canvas::handles::handle_screen_position(
        fanta_canvas::ResizeHandle::NorthWest,
        world,
        &viewport,
        screen_size,
    );
    let south_east = fanta_canvas::handles::handle_screen_position(
        fanta_canvas::ResizeHandle::SouthEast,
        world,
        &viewport,
        screen_size,
    );
    view.update(cx, |view, cx| {
        view.update_hover_resize_handle((north_west + south_east) / 2.0, cx)
    });
    view.read_with(cx, |view, _| assert!(view.hover_resize_handle.is_none()));

    // Two selected nodes: the Select tool would move them, not resize.
    item.update(cx, |item, cx| {
        item.with_document(cx, |document| {
            let page = document.doc.active_page().expect("active page");
            document.doc.selection.add(page);
            ((), DocChange::Selection)
        });
    });
    let corner = fanta_canvas::handles::handle_screen_position(
        fanta_canvas::ResizeHandle::NorthWest,
        world,
        &viewport,
        screen_size,
    );
    view.update(cx, |view, cx| view.update_hover_resize_handle(corner, cx));
    view.read_with(cx, |view, _| assert!(view.hover_resize_handle.is_none()));
}

/// The hint is the only thing on an empty canvas, so it must disappear the
/// moment the page has content — and never claim an unloaded document is
/// empty.
#[gpui::test]
async fn the_empty_page_hint_tracks_the_active_pages_children(cx: &mut TestAppContext) {
    init_test(cx);
    let project = Project::test(FakeFs::new(cx.executor()), [], cx).await;
    let item = crate::document::ready_item_for_test(
        &project,
        std::path::PathBuf::from("/tmp/Design.fig"),
        doc_with_one_page(),
        cx,
    );
    let scratch = cx.add_window(|_, _| gpui::Empty);
    let view = scratch
        .update(cx, |_, window, cx| {
            cx.new(|cx| FigView::new(item.clone(), project.clone(), window, cx))
        })
        .expect("create fig view");

    view.read_with(cx, |view, cx| assert!(view.active_page_is_empty(cx)));
    add_rect(&item, cx);
    view.read_with(cx, |view, cx| assert!(!view.active_page_is_empty(cx)));
}

#[test]
fn every_selection_handle_maps_to_its_own_resize_cursor() {
    use fanta_canvas::ResizeHandle;
    assert_eq!(
        resize_cursor(ResizeHandle::North),
        CursorStyle::ResizeUpDown
    );
    assert_eq!(
        resize_cursor(ResizeHandle::South),
        CursorStyle::ResizeUpDown
    );
    assert_eq!(
        resize_cursor(ResizeHandle::East),
        CursorStyle::ResizeLeftRight
    );
    assert_eq!(
        resize_cursor(ResizeHandle::West),
        CursorStyle::ResizeLeftRight
    );
    assert_eq!(
        resize_cursor(ResizeHandle::NorthWest),
        CursorStyle::ResizeUpLeftDownRight
    );
    assert_eq!(
        resize_cursor(ResizeHandle::SouthEast),
        CursorStyle::ResizeUpLeftDownRight
    );
    assert_eq!(
        resize_cursor(ResizeHandle::NorthEast),
        CursorStyle::ResizeUpRightDownLeft
    );
    assert_eq!(
        resize_cursor(ResizeHandle::SouthWest),
        CursorStyle::ResizeUpRightDownLeft
    );
}

#[test]
fn zooming_at_an_anchor_keeps_the_anchored_world_point_fixed() {
    let viewport = Viewport {
        center: [100.0, -50.0],
        zoom: 2.0,
    };
    let screen_size = (800.0, 600.0);
    let anchor = (120.0, 90.0);
    let world_before = screen_to_world(anchor, viewport, screen_size);

    let zoomed = zoom_viewport_at(viewport, anchor, 1.5, screen_size);
    let world_after = screen_to_world(anchor, zoomed, screen_size);

    assert!((world_before.0 - world_after.0).abs() < 1e-9);
    assert!((world_before.1 - world_after.1).abs() < 1e-9);
    assert!((zoomed.zoom - 3.0).abs() < 1e-9);
}

#[test]
fn zoom_clamps_to_viewer_limits() {
    let viewport = Viewport {
        center: [0.0, 0.0],
        zoom: 1.0,
    };
    let screen_size = (800.0, 600.0);
    let zoomed_out = zoom_viewport_at(viewport, (400.0, 300.0), 1e-6, screen_size);
    assert!((zoomed_out.zoom - f64::from(MIN_ZOOM)).abs() < 1e-9);
    let zoomed_in = zoom_viewport_at(viewport, (400.0, 300.0), 1e6, screen_size);
    assert!((zoomed_in.zoom - f64::from(MAX_ZOOM)).abs() < 1e-9);
}

#[test]
fn panning_moves_the_center_against_the_screen_delta() {
    let viewport = Viewport {
        center: [10.0, 20.0],
        zoom: 2.0,
    };
    let panned = pan_viewport(viewport, (30.0, -10.0));
    assert!((panned.center[0] - (10.0 - 15.0)).abs() < 1e-9);
    assert!((panned.center[1] - (20.0 + 5.0)).abs() < 1e-9);
    assert!((panned.zoom - viewport.zoom).abs() < 1e-9);
}

#[test]
fn motion_values_are_sampled_from_authored_node_state() {
    let mut node = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        0.0,
        0.0,
        20.0,
        20.0,
        Color::rgb(12, 34, 56),
    )));
    node.transform = fanta_doc::Transform2D::translation(12.0, 34.0);

    assert_eq!(
        motion_value(&node, MotionProperty::PositionX),
        Some(ResolvedVarValue::Float { value: 12.0 })
    );
    assert_eq!(
        motion_property(TimelineProperty::ScaleX),
        MotionProperty::ScaleX
    );
    assert_eq!(
        motion_property(TimelineProperty::ScaleY),
        MotionProperty::ScaleY
    );
    assert_eq!(
        motion_value(
            &node,
            MotionProperty::bound(BoundProp::FillColor { index: 0 })
        ),
        Some(ResolvedVarValue::Color {
            value: Color::rgb(12, 34, 56)
        })
    );
}

#[test]
fn motion_keyframes_sample_the_variable_resolved_value() {
    let collection_id = VariableCollectionId::new();
    let mode_id = ModeId::new();
    let variable_id = VariableId::new();
    let mut doc = fanta_doc::Doc::new();
    doc.variables.collections.insert(
        collection_id,
        VariableCollection {
            id: collection_id,
            name: "Theme".to_owned(),
            modes: vec![Mode {
                id: mode_id,
                name: "Default".to_owned(),
            }],
            default_mode: mode_id,
            variable_order: vec![variable_id],
        },
    );
    doc.variables.variables.insert(
        variable_id,
        Variable {
            id: variable_id,
            collection: collection_id,
            name: "Accent".to_owned(),
            ty: VariableType::Color,
            values_by_mode: BTreeMap::from([(
                mode_id,
                VarValue::Color {
                    value: Color::rgb(255, 0, 0),
                },
            )]),
            scopes: Vec::new(),
            options: Vec::new(),
        },
    );
    let mut node = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        0.0,
        0.0,
        20.0,
        20.0,
        Color::WHITE,
    )));
    node.bindings
        .insert(BoundProp::FillColor { index: 0 }, variable_id);
    let node_id = node.id;
    doc.apply(Operation::create_node(node))
        .expect("create bound node");

    let resolved = motion_source_node(&doc, node_id).expect("resolved node");
    assert_eq!(
        motion_value(
            &resolved,
            MotionProperty::bound(BoundProp::FillColor { index: 0 })
        ),
        Some(ResolvedVarValue::Color {
            value: Color::rgb(255, 0, 0)
        })
    );
}

#[test]
fn timeline_model_projects_engine_milliseconds_to_ui_microseconds() {
    let mut doc = fanta_doc::Doc::new();
    let mut node = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        0.0,
        0.0,
        20.0,
        20.0,
        Color::WHITE,
    )));
    node.name = "Star".to_owned();
    let node_id = node.id;
    doc.apply(Operation::create_node(node))
        .expect("create motion target");

    let clip_id = AnimationClipId::new();
    let track_id = AnimationTrackId::new();
    let mut track = AnimationTrack::new(
        track_id,
        MotionTarget::new(node_id, MotionProperty::PositionX),
    );
    let keyframe_id = KeyframeId::new();
    let mut keyframe = Keyframe::new(keyframe_id, 250, ResolvedVarValue::Float { value: 10.0 });
    keyframe.interpolation = Interpolation::Hold;
    keyframe.easing = Easing::CubicBezier {
        x1: 0.25,
        y1: -0.5,
        x2: 0.75,
        y2: 1.5,
    };
    track.keyframes.insert(keyframe_id, keyframe);
    let early_keyframe_id = KeyframeId::new();
    track.keyframes.insert(
        early_keyframe_id,
        Keyframe::new(
            early_keyframe_id,
            100,
            ResolvedVarValue::Float { value: 2.0 },
        ),
    );
    let mut clip = AnimationClip::new(clip_id, "Entrance", 1_500);
    clip.tracks.insert(track_id, track);
    doc.motion.clips.insert(clip_id, clip);

    let model = motion_timeline_model(&doc, Some(clip_id));
    assert_eq!(model.clip_name.as_deref(), Some("Entrance"));
    assert_eq!(model.duration_us, 1_500_000);
    assert_eq!(model.tracks.len(), 1);
    assert_eq!(model.tracks[0].node_id, node_id);
    assert_eq!(model.tracks[0].label.as_ref(), "Star · Position X");
    assert_eq!(model.tracks[0].keyframes.len(), 2);
    assert_eq!(
        model.tracks[0].keyframes[0].id.as_ref(),
        early_keyframe_id.to_string()
    );
    assert_eq!(model.tracks[0].keyframes[0].time_us, 100_000);
    assert_eq!(
        model.tracks[0].keyframes[1].id.as_ref(),
        keyframe_id.to_string()
    );
    assert_eq!(model.tracks[0].keyframes[1].time_us, 250_000);
    assert_eq!(
        model.tracks[0].keyframes[1].interpolation,
        Interpolation::Hold
    );
    assert_eq!(
        model.tracks[0].keyframes[1].easing,
        Easing::CubicBezier {
            x1: 0.25,
            y1: -0.5,
            x2: 0.75,
            y2: 1.5,
        }
    );
}

fn doc_with_two_pages() -> (fanta_doc::Doc, NodeId, NodeId) {
    let mut doc = doc_with_one_page();
    let page_one = doc.active_page().expect("page one");
    let mut second = CanvasNode::new(NodeData::Group(GroupNode::default()));
    second.name = "Page 2".to_owned();
    let page_two = second.id;
    doc.apply(Operation::create_node(second))
        .expect("create page two");
    doc.add_page(page_two);
    (doc, page_one, page_two)
}

fn group_under(doc: &mut fanta_doc::Doc, parent: NodeId) -> NodeId {
    let mut group = CanvasNode::new(NodeData::Group(GroupNode::default()));
    group.parent = Some(parent);
    let id = group.id;
    doc.apply(Operation::create_node(group))
        .expect("create group");
    id
}

#[gpui::test]
async fn following_an_agent_moves_between_pages_and_stops_on_request(cx: &mut TestAppContext) {
    init_test(cx);
    let project = Project::test(FakeFs::new(cx.executor()), [], cx).await;
    let (doc, _, _) = doc_with_two_pages();
    let item = crate::document::ready_item_for_test(
        &project,
        std::path::PathBuf::from("/tmp/Design.fig"),
        doc,
        cx,
    );
    let scratch = cx.add_window(|_, _| gpui::Empty);
    let view = scratch
        .update(cx, |_, window, cx| {
            cx.new(|cx| FigView::new(item, project, window, cx))
        })
        .expect("view");
    view.update(cx, |view, _| {
        view.viewport = Some(Viewport {
            center: [0.0, 0.0],
            zoom: 2.0,
        });
    });
    let activity = design_surface::AgentActivity {
        agent_id: "reviewer".to_string(),
        agent_name: "Reviewer".to_string(),
        action: "Inspecting the hero".to_string(),
        page: Some(1),
        node: None,
        world: Some([300.0, 400.0]),
        active: true,
        project_root: None,
        source_path: None,
        workspace: None,
    };
    let state = cx.update(design_surface::activity_state);
    state.update(cx, |state, cx| {
        state.record(activity.clone(), cx);
        state.follow(Some(activity.agent_id.clone()), cx);
    });
    cx.run_until_parked();
    view.read_with(cx, |view, _| {
        assert_eq!(view.selected_page_index, Some(1));
        assert_eq!(
            view.viewport,
            Some(Viewport {
                center: [300.0, 400.0],
                zoom: 2.0
            })
        );
    });
    view.update(cx, |view, _| {
        view.container_bounds = Some(Bounds::new(
            gpui::point(px(0.0), px(0.0)),
            gpui::size(px(1000.0), px(800.0)),
        ));
    });
    state.update(cx, |state, cx| {
        state.record(
            design_surface::AgentActivity {
                world: Some([350.0, 420.0]),
                workspace: Some(design_surface::AgentWorkspace::Code),
                ..activity.clone()
            },
            cx,
        );
    });
    cx.run_until_parked();
    view.read_with(cx, |view, cx| {
        assert_eq!(view.editor_workspace(cx), EditorWorkspace::Code);
        assert_eq!(
            view.viewport.map(|viewport| viewport.center),
            Some([300.0, 400.0])
        );
    });
    state.update(cx, |state, cx| {
        state.record(
            design_surface::AgentActivity {
                workspace: Some(design_surface::AgentWorkspace::Variables),
                ..activity.clone()
            },
            cx,
        );
    });
    cx.run_until_parked();
    view.read_with(cx, |view, cx| {
        assert_eq!(view.editor_workspace(cx), EditorWorkspace::Variables);
    });
    state.update(cx, |state, cx| {
        state.follow(None, cx);
        state.record(
            design_surface::AgentActivity {
                world: Some([900.0, 700.0]),
                ..activity
            },
            cx,
        );
    });
    cx.run_until_parked();
    view.read_with(cx, |view, _| {
        assert_eq!(
            view.viewport.map(|viewport| viewport.center),
            Some([300.0, 400.0])
        );
    });
}

#[gpui::test]
async fn mounted_saved_instance_drag_undo_redo_preserves_zoomed_viewport(
    cx: &mut TestAppContext,
) {
    use gpui::InputEvent as _;

    init_visual_test(cx);
    cx.update(|cx| {
        cx.bind_keys([
            gpui::KeyBinding::new("cmd-z", Undo, Some("FigViewer")),
            gpui::KeyBinding::new("cmd-shift-z", Redo, Some("FigViewer")),
        ]);
    });
    let file_system = FakeFs::new(cx.executor());
    let project = Project::test(file_system.clone(), [], cx).await;
    let directory = tempfile::tempdir().expect("viewport project directory");
    let root = directory.path().join("Saved viewport");
    let (mut doc, first_page, second_page) = doc_with_two_pages();
    let mut master = CanvasNode::new(NodeData::Vector(VectorNode {
        local_size: Some([20.0, 20.0]),
        ..VectorNode::rect_solid(0.0, 0.0, 20.0, 20.0, Color::BLACK)
    }));
    master.parent = Some(first_page);
    let master_id = master.id;
    doc.scene.insert(master).expect("component master");
    let component = fanta_doc::ComponentId::new();
    doc.components.defs.insert(
        component,
        fanta_doc::ComponentDef::new(component, master_id, "ChevronDown"),
    );
    let mut frame = CanvasNode::new(NodeData::Group(GroupNode {
        clip_size: Some([20.0, 24.0]),
        auto_layout: Some(fanta_doc::AutoLayout {
            counter_align: fanta_doc::CounterAlign::Center,
            ..Default::default()
        }),
        ..Default::default()
    }));
    frame.name = "Chevron".into();
    frame.parent = Some(second_page);
    frame.transform = Transform2D::translation(7712.0, 2263.0);
    let frame_id = frame.id;
    doc.scene.insert(frame).expect("fixed layout parent");
    let mut instance = CanvasNode::new(NodeData::Instance(fanta_doc::InstanceNode {
        component,
        local_size: [20.0, 20.0],
        overrides: Vec::new(),
        prop_values: BTreeMap::new(),
        derived: Vec::new(),
    }));
    instance.name = "ChevronDown".into();
    instance.parent = Some(frame_id);
    instance.transform = Transform2D::translation(0.0, 1.5);
    let instance_id = instance.id;
    doc.scene.insert(instance).expect("placed instance");
    doc.history = Default::default();
    crate::document::write_project(&root, &doc, &BTreeMap::new())
        .expect("write viewport fixture");
    file_system.insert_tree_from_real_fs(&root, &root).await;
    let item = crate::document::ready_item_with_root_for_test(
        &project,
        root.join("fanta.json"),
        Some(root.clone()),
        doc,
        cx,
    );
    let window = cx.add_window({
        let item = item.clone();
        move |window, cx| FigView::new(item, project, window, cx)
    });
    let view = window.entity(cx).expect("mounted viewport view");
    let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
    visual.simulate_resize(size(px(1400.0), px(900.0)));
    visual.run_until_parked();
    let zoomed = Viewport {
        center: [7722.0, 2274.5],
        zoom: 2.0,
    };
    view.update(&mut visual, |view, cx| {
        view.select_page(1, cx);
        view.set_viewport(zoomed, cx);
    });
    item.update(&mut visual, |item, cx| {
        item.with_document(cx, |document| {
            document.doc.selection.replace_with([instance_id]);
            ((), DocChange::Selection)
        });
    });
    visual.update(|window, cx| {
        let focus_handle = view.read(cx).focus_handle.clone();
        focus_handle.focus(window, cx);
        window.draw(cx).clear();
    });
    visual.run_until_parked();
    let baseline = item.read_with(&visual, |item, _| {
        serde_json::to_value(&item.doc().expect("document").scene).expect("baseline scene")
    });
    let assert_camera = |visual: &gpui::VisualTestContext| {
        view.read_with(visual, |view, cx| {
            assert_eq!(
                view.viewport,
                Some(zoomed),
                "document edits retain the camera"
            );
            assert_eq!(view.selected_page_root, Some(second_page));
            assert_eq!(view.last_seen_root, Some(second_page));
            assert_eq!(view.scope, Some(FigScope::Page(second_page)));
            let doc = item.read(cx).doc().expect("document");
            assert_eq!(doc.active_page(), Some(second_page));
            assert_eq!(doc.selection.as_slice(), &[instance_id]);
            assert_eq!(
                doc.scene.get(instance_id).expect("instance").parent,
                Some(frame_id)
            );
        });
    };
    assert_camera(&visual);
    let start = view.read_with(&visual, |view, _| {
        view.container_bounds
            .expect("mounted canvas bounds")
            .center()
    });
    let end = start + point(px(16.0), px(0.0));
    visual.update(|window, cx| {
        window.dispatch_event(
            MouseDownEvent {
                position: start,
                button: MouseButton::Left,
                modifiers: gpui::Modifiers::none(),
                click_count: 1,
                first_mouse: false,
            }
            .to_platform_input(),
            cx,
        );
        window.dispatch_event(
            MouseMoveEvent {
                position: end,
                pressed_button: Some(MouseButton::Left),
                modifiers: gpui::Modifiers::none(),
            }
            .to_platform_input(),
            cx,
        );
        window.dispatch_event(
            MouseUpEvent {
                position: end,
                button: MouseButton::Left,
                modifiers: gpui::Modifiers::none(),
                click_count: 1,
            }
            .to_platform_input(),
            cx,
        );
    });
    visual.run_until_parked();
    let moved = item.read_with(&visual, |item, _| {
        let doc = item.doc().expect("document");
        assert_eq!(doc.history.undo_depth(), 1);
        assert!(!item.content_preview_active());
        serde_json::to_value(&doc.scene).expect("moved scene")
    });
    assert_ne!(moved, baseline, "the mounted drag must commit geometry");
    assert_camera(&visual);
    view.update(&mut visual, |view, cx| view.save_document(cx))
        .await
        .expect("save committed drag");
    visual.run_until_parked();
    visual.update(|window, cx| window.draw(cx).clear());
    assert!(!item.read_with(&visual, |item, _| item.is_dirty()));
    assert_camera(&visual);
    for (keystroke, expected) in [("cmd-z", &baseline), ("cmd-shift-z", &moved)] {
        visual.simulate_keystrokes(keystroke);
        visual.update(|window, cx| window.draw(cx).clear());
        item.read_with(&visual, |item, _| {
            assert_eq!(
                &serde_json::to_value(&item.doc().expect("document").scene)
                    .expect("history scene"),
                expected,
            );
        });
        assert_camera(&visual);
        view.update(&mut visual, |view, cx| view.save_document(cx))
            .await
            .expect("save history result");
        visual.run_until_parked();
        visual.update(|window, cx| window.draw(cx).clear());
        assert_camera(&visual);
        let (reopened, _) =
            fanta_format::read_project_tree(&root).expect("reopen history result");
        assert_eq!(
            &serde_json::to_value(&reopened.scene).expect("saved scene"),
            expected
        );
    }
}

#[gpui::test]
async fn selecting_the_current_page_preserves_its_viewport(cx: &mut TestAppContext) {
    init_test(cx);
    let project = Project::test(FakeFs::new(cx.executor()), [], cx).await;
    let (doc, first_page, second_page) = doc_with_two_pages();
    let item = crate::document::ready_item_for_test(
        &project,
        std::path::PathBuf::from("/tmp/Design.fig"),
        doc,
        cx,
    );
    let scratch = cx.add_window(|_, _| gpui::Empty);
    let view = scratch
        .update(cx, |_, window, cx| {
            cx.new(|cx| FigView::new(item.clone(), project.clone(), window, cx))
        })
        .expect("view");
    let zoomed = Viewport {
        center: [120.0, -40.0],
        zoom: 3.0,
    };

    view.update(cx, |view, cx| {
        view.viewport = Some(zoomed);
        view.select_page(0, cx);
    });
    view.read_with(cx, |view, cx| {
        assert_eq!(view.viewport, Some(zoomed));
        assert_eq!(view.last_seen_root, Some(first_page));
        assert_eq!(
            view.item.read(cx).doc().and_then(|doc| doc.active_page()),
            Some(first_page)
        );
    });

    view.update(cx, |view, cx| view.select_page(1, cx));
    view.read_with(cx, |view, cx| {
        assert_eq!(view.viewport, None);
        assert_eq!(view.last_seen_root, Some(second_page));
        assert_eq!(
            view.item.read(cx).doc().and_then(|doc| doc.active_page()),
            Some(second_page)
        );
    });

    view.update(cx, |view, cx| {
        view.viewport = Some(zoomed);
        view.select_page(1, cx);
    });
    view.read_with(cx, |view, _| assert_eq!(view.viewport, Some(zoomed)));
}

#[cfg(target_os = "macos")]
#[gpui::test]
async fn mounted_page_tab_activation_preserves_scene_generation_and_gpu_epoch(
    cx: &mut TestAppContext,
) {
    init_visual_test(cx);
    let project = Project::test(FakeFs::new(cx.executor()), [], cx).await;
    let (doc, first_page, second_page) = doc_with_two_pages();
    let item =
        crate::document::ready_item_for_test(&project, "/tmp/PageTabs.fig".into(), doc, cx);
    item.update(cx, |item, cx| {
        item.with_document(cx, |document| {
            document.ensure_page_solved(0);
            document.ensure_page_solved(1);
            ((), DocChange::None)
        });
    });
    let (workspace, visual) = cx.add_window_view(|window, cx| {
        workspace::Workspace::test_new(project.clone(), window, cx)
    });
    let (first, second) = visual.update(|window, cx| {
        let first = cx.new(|cx| FigView::new(item.clone(), project.clone(), window, cx));
        let second = cx.new(|cx| FigView::new(item.clone(), project, window, cx));
        first.update(cx, |view, _| {
            view.scope = Some(FigScope::Page(first_page));
            view.gpu_canvas = Some(GpuCanvas::for_test());
        });
        second.update(cx, |view, _| {
            view.scope = Some(FigScope::Page(second_page));
            view.gpu_canvas = Some(GpuCanvas::for_test());
        });
        workspace.update(cx, |workspace, cx| {
            workspace.add_item_to_active_pane(Box::new(first.clone()), None, true, window, cx);
            workspace.add_item_to_active_pane(Box::new(second.clone()), None, true, window, cx);
        });
        window.activate_window();
        (first, second)
    });
    visual.run_until_parked();
    for (view, center) in [(&first, [120.0, -40.0]), (&second, [-80.0, 30.0])] {
        view.update(visual, |view, _| {
            view.viewport = Some(Viewport { center, zoom: 2.0 })
        });
    }
    let baseline = item.read_with(visual, |item, _| {
        let document = item.document().expect("document");
        (
            document.doc.scene.instance_id(),
            document.doc.scene.revision(),
            document.render_generation(),
        )
    });
    for (view, root, center) in [
        (&first, first_page, [120.0, -40.0]),
        (&second, second_page, [-80.0, 30.0]),
        (&first, first_page, [120.0, -40.0]),
    ] {
        visual.update(|window, cx| {
            workspace.update(cx, |workspace, cx| {
                assert!(workspace.activate_item(view, true, true, window, cx));
            });
            window.draw(cx).clear();
        });
        visual.run_until_parked();
        view.read_with(visual, |view, cx| {
            assert_eq!(view.viewport, Some(Viewport { center, zoom: 2.0 }));
            assert_eq!(view.last_seen_root, Some(root));
            assert_eq!(
                view.gpu_canvas
                    .as_ref()
                    .expect("GPU state")
                    .epoch_for_test(),
                0
            );
            let document = view.item.read(cx).document().expect("document");
            assert_eq!(document.doc.active_page(), Some(root));
            assert_eq!(
                (
                    document.doc.scene.instance_id(),
                    document.doc.scene.revision(),
                    document.render_generation()
                ),
                baseline
            );
        });
    }
    first.update(visual, |view, cx| {
        view.select_page(1, cx);
        view.select_page(0, cx);
        assert_eq!(
            view.gpu_canvas
                .as_ref()
                .expect("GPU state")
                .epoch_for_test(),
            0
        );
    });
}

#[gpui::test]
async fn source_edit_lock_keeps_page_navigation_on_the_edited_page(cx: &mut TestAppContext) {
    init_test(cx);
    let project = Project::test(FakeFs::new(cx.executor()), [], cx).await;
    let (doc, first_page, second_page) = doc_with_two_pages();
    let item = crate::document::ready_item_for_test(
        &project,
        std::path::PathBuf::from("/tmp/Design.fig"),
        doc,
        cx,
    );
    let scratch = cx.add_window(|_, _| gpui::Empty);
    let view = scratch
        .update(cx, |_, window, cx| {
            cx.new(|cx| FigView::new(item.clone(), project.clone(), window, cx))
        })
        .expect("view");

    item.update(cx, |item, cx| item.set_source_edit_locked(true, cx));
    view.update(cx, |view, cx| {
        view.select_page(0, cx);
        view.select_page(1, cx);
    });
    item.read_with(cx, |item, _| {
        assert_eq!(
            item.doc().and_then(|doc| doc.active_page()),
            Some(first_page)
        );
    });
    view.read_with(cx, |view, _| {
        assert_eq!(view.selected_page_root, Some(first_page));
    });

    item.update(cx, |item, cx| {
        item.request_scope(FigScope::Page(second_page), ScopeRequester::Open, cx);
    });
    item.read_with(cx, |item, _| {
        assert_eq!(
            item.doc().and_then(|doc| doc.active_page()),
            Some(first_page)
        );
    });

    item.update(cx, |item, cx| item.set_source_edit_locked(false, cx));
    view.update(cx, |view, cx| view.select_page(1, cx));
    item.read_with(cx, |item, _| {
        assert_eq!(
            item.doc().and_then(|doc| doc.active_page()),
            Some(second_page)
        );
    });
}

#[test]
fn structure_commands_ignore_layers_left_on_another_page() {
    let (mut doc, page_one, page_two) = doc_with_two_pages();
    let on_page_one = group_under(&mut doc, page_one);
    doc.selection.replace_with(vec![on_page_one]);
    assert_eq!(structure_targets(&doc, None), vec![on_page_one]);

    doc.set_active_page(Some(page_two));
    assert!(
        structure_targets(&doc, None).is_empty(),
        "a selection left on page one is not grouped from page two"
    );
    assert!(structure_targets(&doc, Some(on_page_one)).is_empty());

    let on_page_two = group_under(&mut doc, page_two);
    doc.selection.replace_with(vec![on_page_one, on_page_two]);
    assert_eq!(
        structure_targets(&doc, None),
        vec![on_page_two],
        "a selection spanning pages keeps only this page's layers"
    );
}

#[test]
fn ungrouping_skips_component_masters_and_page_roots() {
    let (mut doc, page_one, _page_two) = doc_with_two_pages();
    let plain = group_under(&mut doc, page_one);
    let master = group_under(&mut doc, page_one);
    let component = fanta_doc::ComponentId::new();
    doc.components.defs.insert(
        component,
        fanta_doc::ComponentDef::new(component, master, "Card"),
    );
    assert_eq!(ungroupable_targets(&doc, &[plain, master]), vec![plain]);
    assert!(ungroupable_targets(&doc, &[master, page_one]).is_empty());
}

/// The headline new-tab walk: tab A shows page 1 zoomed and still reports
/// `is_focused` when the scoped open of `pages/<p2>/page.fnx` re-targets
/// the shared document (focus listeners only run at the next draw, so the
/// flag is stale at event delivery). Tab A must keep its viewport and
/// page mirrors — both at the scoped open and when focus later returns
/// and re-asserts its own scope.
#[gpui::test]
async fn a_scoped_open_for_a_new_tab_never_clobbers_the_previous_tabs_viewport(
    cx: &mut TestAppContext,
) {
    init_test(cx);
    let project = Project::test(FakeFs::new(cx.executor()), [], cx).await;
    let (doc, page_one, page_two) = doc_with_two_pages();
    let item = crate::document::ready_item_for_test(
        &project,
        std::path::PathBuf::from("/tmp/Design.fig"),
        doc,
        cx,
    );
    let scratch = cx.add_window(|_, _| gpui::Empty);
    let tab_a = scratch
        .update(cx, |_, window, cx| {
            cx.new(|cx| FigView::new(item.clone(), project.clone(), window, cx))
        })
        .expect("tab A");
    let zoomed = Viewport {
        center: [120.0, -40.0],
        zoom: 3.0,
    };
    tab_a.update(cx, |view, _| {
        view.is_focused = true;
        view.scope = Some(FigScope::Page(page_one));
        view.selected_page_index = Some(0);
        view.selected_page_root = Some(page_one);
        view.viewport = Some(zoomed);
    });

    // The scoped open's `try_open` runs before any draw delivers tab A's
    // focus-out, so A still reports `is_focused` when this event lands.
    item.update(cx, |item, cx| {
        item.request_scope(FigScope::Page(page_two), ScopeRequester::Open, cx);
    });
    cx.run_until_parked();
    tab_a.read_with(cx, |view, _| {
        assert_eq!(
            view.viewport,
            Some(zoomed),
            "a sibling's scoped open must not clobber tab A's viewport"
        );
        assert_eq!(view.last_seen_root, Some(page_one));
        assert_eq!(view.selected_page_index, Some(0));
        assert_eq!(view.selected_page_root, Some(page_one));
    });

    // Focus returns to tab A: its on-focus re-assert re-roots the shared
    // document back to page 1. Restoring our OWN root must keep the
    // saved viewport.
    tab_a.update(cx, |view, cx| {
        let requester = ScopeRequester::View(cx.entity_id());
        view.item.update(cx, |item, cx| {
            item.request_scope(FigScope::Page(page_one), requester, cx)
        });
    });
    cx.run_until_parked();
    tab_a.read_with(cx, |view, cx| {
        assert_eq!(
            view.viewport,
            Some(zoomed),
            "switching back to tab A must keep its zoom"
        );
        assert_eq!(
            view.item.read(cx).doc().and_then(|doc| doc.active_page()),
            Some(page_one),
            "the re-assert re-roots the shared document to tab A's page"
        );
    });
}

/// The load-time apply carries no requesting view (the scoped tab was
/// created while the document was still loading), so the focused view
/// must still follow it.
#[gpui::test]
async fn the_load_time_scope_apply_still_refits_the_focused_view(cx: &mut TestAppContext) {
    init_test(cx);
    let project = Project::test(FakeFs::new(cx.executor()), [], cx).await;
    let (doc, _page_one, page_two) = doc_with_two_pages();
    let item = crate::document::ready_item_for_test(
        &project,
        std::path::PathBuf::from("/tmp/Design.fig"),
        doc,
        cx,
    );
    let scratch = cx.add_window(|_, _| gpui::Empty);
    let view = scratch
        .update(cx, |_, window, cx| {
            cx.new(|cx| FigView::new(item.clone(), project.clone(), window, cx))
        })
        .expect("view");
    view.update(cx, |view, _| {
        view.is_focused = true;
        view.viewport = Some(Viewport {
            center: [10.0, 20.0],
            zoom: 2.0,
        });
    });

    item.update(cx, |item, cx| {
        item.with_document(cx, |document| {
            document.doc.set_active_page(Some(page_two));
            ((), DocChange::Selection)
        });
        cx.emit(FigItemEvent::ScopeApplied(
            FigScope::Page(page_two),
            ScopeRequester::Load,
        ));
    });
    cx.run_until_parked();
    view.read_with(cx, |view, _| {
        assert_eq!(
            view.viewport, None,
            "the focused view refits to the load-applied scope"
        );
        assert_eq!(view.last_seen_root, Some(page_two));
        assert_eq!(view.selected_page_root, Some(page_two));
        assert_eq!(view.selected_page_index, Some(1));
    });
}

#[gpui::test]
async fn external_viewport_changes_follow_the_active_canvas_despite_stale_focus(
    cx: &mut TestAppContext,
) {
    init_test(cx);
    cx.update(crate::agent_surface::init);
    let project = Project::test(FakeFs::new(cx.executor()), [], cx).await;
    let (doc, _, _) = doc_with_two_pages();
    let item = crate::document::ready_item_for_test(
        &project,
        std::path::PathBuf::from("/tmp/Design.fig"),
        doc,
        cx,
    );
    let scratch = cx.add_window(|_, _| gpui::Empty);
    let views = scratch
        .update(cx, |_, window, cx| {
            [
                cx.new(|cx| FigView::new(item.clone(), project.clone(), window, cx)),
                cx.new(|cx| FigView::new(item.clone(), project.clone(), window, cx)),
            ]
        })
        .expect("views");
    let original = Viewport {
        center: [10.0, 20.0],
        zoom: 2.0,
    };
    let requested = Viewport {
        center: [100.0, 200.0],
        zoom: 0.5,
    };
    for (index, view) in views.iter().enumerate() {
        view.update(cx, |view, _| {
            view.is_focused = index == 1;
            view.viewport = Some(original);
        });
    }
    views.first().expect("active view").update(cx, |view, cx| {
        view.activate_design_scope(cx);
    });
    let initial_undo_depth = item.read_with(cx, |item, _| {
        item.doc().expect("document").history.undo_depth()
    });
    item.update(cx, |item, cx| {
        item.with_document(cx, |document| {
            document.doc.viewport = requested;
            ((), DocChange::None)
        });
    });
    cx.run_until_parked();
    for (index, view) in views.iter().enumerate() {
        view.read_with(cx, |view, _| {
            assert_eq!(
                view.viewport,
                Some(if index == 0 { requested } else { original })
            );
        });
    }
    let [active_view, other_view] = views;
    cx.update(|cx| {
        crate::agent_surface::clear_active_view(other_view.entity_id(), cx);
        assert_eq!(
            crate::agent_surface::active_view_for_item(item.entity_id(), cx),
            Some(active_view.entity_id())
        );
        crate::agent_surface::clear_active_view(active_view.entity_id(), cx);
        assert_eq!(
            crate::agent_surface::active_view_for_item(item.entity_id(), cx),
            None
        );
    });
    let fallback_requested = Viewport {
        center: [300.0, 400.0],
        zoom: 0.75,
    };
    item.update(cx, |item, cx| {
        item.with_document(cx, |document| {
            document.doc.viewport = fallback_requested;
            ((), DocChange::None)
        });
    });
    cx.run_until_parked();
    active_view.read_with(cx, |view, _| assert_eq!(view.viewport, Some(requested)));
    other_view.read_with(cx, |view, _| {
        assert_eq!(view.viewport, Some(fallback_requested));
    });
    other_view.update(cx, |view, cx| view.activate_design_scope(cx));
    active_view.update(cx, |view, cx| view.activate_design_scope(cx));
    let other_weak = other_view.downgrade();
    // GPUI runs release callbacks when application effects flush, not
    // when the dispatcher merely parks after an out-of-update handle drop.
    cx.update(|_| drop(other_view));
    cx.run_until_parked();
    assert!(other_weak.upgrade().is_none());
    cx.read(|cx| {
        assert_eq!(
            crate::agent_surface::active_view_for_item(item.entity_id(), cx),
            Some(active_view.entity_id())
        );
    });
    let active_weak = active_view.downgrade();
    cx.update(|_| drop(active_view));
    cx.run_until_parked();
    assert!(active_weak.upgrade().is_none());
    cx.read(|cx| {
        assert_eq!(
            crate::agent_surface::active_view_for_item(item.entity_id(), cx),
            None
        );
    });
    item.read_with(cx, |item, _| {
        assert!(!item.is_dirty());
        assert!(!item.content_preview_active());
        assert_eq!(
            item.doc().expect("document").history.undo_depth(),
            initial_undo_depth
        );
    });
}

#[gpui::test]
async fn motion_panel_clip_selection_synchronizes_the_timeline(cx: &mut TestAppContext) {
    init_test(cx);
    let project = Project::test(FakeFs::new(cx.executor()), [], cx).await;
    let mut doc = doc_with_one_page();
    let first_clip = AnimationClipId::from_u128(100);
    let second_clip = AnimationClipId::from_u128(200);
    doc.motion.clips.insert(
        first_clip,
        AnimationClip::new(first_clip, "Entrance", 1_000),
    );
    doc.motion
        .clips
        .insert(second_clip, AnimationClip::new(second_clip, "Exit", 2_000));
    let item = crate::document::ready_item_for_test(
        &project,
        std::path::PathBuf::from("/tmp/Design.fig"),
        doc,
        cx,
    );
    let scratch = cx.add_window(|_, _| gpui::Empty);
    let view = scratch
        .update(cx, |_, window, cx| {
            cx.new(|cx| FigView::new(item, project, window, cx))
        })
        .expect("scratch window");
    let motion_sidebar = view.read_with(cx, |view, _| view.motion_sidebar.clone());

    motion_sidebar.update(cx, |_, cx| {
        cx.emit(MotionPanelEvent::SelectClip(second_clip));
    });
    cx.run_until_parked();

    view.read_with(cx, |view, cx| {
        assert_eq!(view.active_motion_clip, Some(second_clip));
        assert_eq!(
            view.timeline_shell
                .read(cx)
                .view_model()
                .clip_name
                .as_deref(),
            Some("Exit")
        );
    });
}

#[gpui::test]
async fn timeline_drag_previews_realtime_and_commits_one_undo_step(cx: &mut TestAppContext) {
    init_test(cx);
    let project = Project::test(FakeFs::new(cx.executor()), [], cx).await;
    let mut doc = doc_with_one_page();
    let clip_id = AnimationClipId::from_u128(100);
    let track_id = AnimationTrackId::from_u128(101);
    let keyframe_id = KeyframeId::from_u128(102);
    let target = MotionTarget::new(
        doc.active_page().expect("active page"),
        MotionProperty::PositionX,
    );
    let mut track = AnimationTrack::new(track_id, target);
    track.keyframes.insert(
        keyframe_id,
        Keyframe::new(keyframe_id, 250, ResolvedVarValue::Float { value: 10.0 }),
    );
    let mut clip = AnimationClip::new(clip_id, "Entrance", 1_000);
    clip.tracks.insert(track_id, track);
    doc.motion.clips.insert(clip_id, clip);
    doc.history = Default::default();
    let item = crate::document::ready_item_for_test(
        &project,
        std::path::PathBuf::from("/tmp/Design.fig"),
        doc,
        cx,
    );
    let scratch = cx.add_window(|_, _| gpui::Empty);
    let view = scratch
        .update(cx, |_, window, cx| {
            cx.new(|cx| FigView::new(item.clone(), project.clone(), window, cx))
        })
        .expect("scratch window");
    let selection = TimelineKeyframeSelection {
        track_id: track_id.to_string().into(),
        keyframe_id: keyframe_id.to_string().into(),
    };

    view.update(cx, |view, cx| {
        view.active_motion_clip = Some(clip_id);
        view.begin_motion_keyframe_drag(&selection, cx);
        view.preview_motion_keyframe_drag(&selection, 700_000, cx);
    });
    item.read_with(cx, |item, _| {
        let doc = item.doc().expect("ready document");
        assert_eq!(doc.history.undo_depth(), 0);
        assert_eq!(
            doc.motion.clips[&clip_id].tracks[&track_id].keyframes[&keyframe_id].time_ms,
            700
        );
    });

    view.update(cx, |view, cx| {
        view.commit_motion_keyframe_drag(&selection, 700_000, cx)
    });
    item.update(cx, |item, cx| {
        let doc = item.doc().expect("ready document");
        assert_eq!(doc.history.undo_depth(), 1);
        assert_eq!(
            doc.motion.clips[&clip_id].tracks[&track_id].keyframes[&keyframe_id].time_ms,
            700
        );
        assert!(item.undo(cx).expect("undo motion drag"));
        let doc = item.doc().expect("ready document");
        assert_eq!(
            doc.motion.clips[&clip_id].tracks[&track_id].keyframes[&keyframe_id].time_ms,
            250
        );
    });

    view.update(cx, |view, cx| {
        view.set_editor_mode(EditorMode::Motion, cx);
        view.begin_motion_keyframe_drag(&selection, cx);
        view.preview_motion_keyframe_drag(&selection, 800_000, cx);
    });
    item.update(cx, |item, cx| item.set_source_edit_locked(true, cx));
    cx.run_until_parked();
    view.read_with(cx, |view, cx| {
        assert!(view.motion_keyframe_drag.is_none());
        assert!(!view.timeline_shell.read(cx).authoring_enabled());
    });
    item.read_with(cx, |item, _| {
        assert_eq!(
            item.doc().expect("ready document").motion.clips[&clip_id].tracks[&track_id]
                .keyframes[&keyframe_id]
                .time_ms,
            250
        );
    });
    item.update(cx, |item, cx| item.set_source_edit_locked(false, cx));
    cx.run_until_parked();
    view.read_with(cx, |view, cx| {
        assert!(view.timeline_shell.read(cx).authoring_enabled());
    });
}

#[gpui::test]
async fn mode_and_workspace_switches_finish_custom_easing_edits(cx: &mut TestAppContext) {
    init_visual_test(cx);
    let project = Project::test(FakeFs::new(cx.executor()), [], cx).await;
    let mut doc = doc_with_one_page();
    let clip_id = AnimationClipId::from_u128(200);
    let track_id = AnimationTrackId::from_u128(201);
    let keyframe_id = KeyframeId::from_u128(202);
    let target = MotionTarget::new(
        doc.active_page().expect("active page"),
        MotionProperty::PositionX,
    );
    let original = Easing::CubicBezier {
        x1: 0.42,
        y1: 0.0,
        x2: 0.58,
        y2: 1.0,
    };
    let committed = Easing::CubicBezier {
        x1: 0.1,
        y1: -2.0,
        x2: 0.9,
        y2: 3.0,
    };
    let mut keyframe = Keyframe::new(keyframe_id, 250, ResolvedVarValue::Float { value: 10.0 });
    keyframe.easing = original;
    let mut track = AnimationTrack::new(track_id, target);
    track.keyframes.insert(keyframe_id, keyframe);
    let mut clip = AnimationClip::new(clip_id, "Entrance", 1_000);
    clip.tracks.insert(track_id, track);
    doc.motion.clips.insert(clip_id, clip);
    doc.history = Default::default();
    let item = crate::document::ready_item_for_test(
        &project,
        std::path::PathBuf::from("/tmp/Custom-easing.fig"),
        doc,
        cx,
    );
    let item_for_window = item.clone();
    let window =
        cx.add_window(move |window, cx| FigView::new(item_for_window, project, window, cx));
    let view = window.entity(cx).expect("fig view");
    let selection = TimelineKeyframeSelection {
        track_id: track_id.to_string().into(),
        keyframe_id: keyframe_id.to_string().into(),
    };

    window
        .update(cx, |view, window, cx| {
            view.active_motion_clip = Some(clip_id);
            view.set_editor_mode(EditorMode::Motion, cx);
            let timeline = view.timeline_shell.clone();
            timeline.update(cx, |timeline, cx| {
                timeline.begin_easing_edit_for_test(selection.clone(), original, window, cx)
            });
        })
        .expect("begin valid easing edit");
    cx.run_until_parked();
    window
        .update(cx, |view, window, cx| {
            view.timeline_shell.update(cx, |timeline, cx| {
                assert!(timeline.set_easing_edit_text_for_test("0.1, -2, 0.9, 3", window, cx));
            });
        })
        .expect("preview valid easing edit");
    cx.run_until_parked();
    view.update(cx, |view, cx| view.set_editor_mode(EditorMode::Design, cx));

    item.read_with(cx, |item, _| {
        let doc = item.doc().expect("ready document");
        assert_eq!(
            doc.motion.clips[&clip_id].tracks[&track_id].keyframes[&keyframe_id].easing,
            committed
        );
        assert_eq!(doc.history.undo_depth(), 1);
    });

    window
        .update(cx, |view, window, cx| {
            view.set_editor_mode(EditorMode::Motion, cx);
            let timeline = view.timeline_shell.clone();
            timeline.update(cx, |timeline, cx| {
                timeline.begin_easing_edit_for_test(selection.clone(), committed, window, cx)
            });
        })
        .expect("begin invalid easing edit");
    cx.run_until_parked();
    window
        .update(cx, |view, window, cx| {
            view.timeline_shell.update(cx, |timeline, cx| {
                assert!(timeline.set_easing_edit_text_for_test("invalid", window, cx));
            });
            view.set_editor_workspace(EditorWorkspace::Variables, cx);
        })
        .expect("finish invalid easing edit on workspace switch");

    item.read_with(cx, |item, _| {
        let doc = item.doc().expect("ready document");
        assert_eq!(
            doc.motion.clips[&clip_id].tracks[&track_id].keyframes[&keyframe_id].easing,
            committed
        );
        assert_eq!(doc.history.undo_depth(), 1);
    });
    view.read_with(cx, |view, cx| {
        assert!(view.motion_keyframe_drag.is_none());
        assert_eq!(view.editor_workspace(cx), EditorWorkspace::Variables);
    });
}

#[test]
fn pane_layout_reserves_canvas_without_changing_hidden_panel_preferences() {
    let preferred_layers = Some(px(280.0));
    let preferred_inspector = Some(px(400.0));
    assert_eq!(
        pane_sidebar_layout(Some(px(1400.0)), preferred_layers, preferred_inspector),
        PaneSidebarLayout {
            layers: preferred_layers,
            inspector: preferred_inspector
        }
    );
    assert_eq!(
        pane_sidebar_layout(Some(px(588.0)), preferred_layers, preferred_inspector),
        PaneSidebarLayout {
            layers: None,
            inspector: Some(px(348.0))
        }
    );
    assert_eq!(
        pane_sidebar_layout(Some(px(450.0)), preferred_layers, preferred_inspector),
        PaneSidebarLayout {
            layers: None,
            inspector: None
        }
    );
    assert_eq!(
        pane_sidebar_layout(Some(px(588.0)), preferred_layers, None),
        PaneSidebarLayout {
            layers: preferred_layers,
            inspector: None
        }
    );
    assert_eq!(
        pane_sidebar_layout(Some(px(588.0)), None, preferred_inspector),
        PaneSidebarLayout {
            layers: None,
            inspector: Some(px(348.0))
        }
    );
}

#[cfg(feature = "fanta-gpui-ui")]
struct PaneLayoutTestHost {
    view: Entity<FigView>,
    pane_width: Pixels,
    mouse_trace: Option<std::rc::Rc<std::cell::RefCell<Vec<serde_json::Value>>>>,
}

#[cfg(feature = "fanta-gpui-ui")]
impl Render for PaneLayoutTestHost {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .size_full()
            .when_some(self.mouse_trace.clone(), |host, mouse_trace| {
                host.child(
                    gpui::canvas(
                        |_, _, _| (),
                        move |_, _, window, _| {
                            window.on_mouse_event(
                                move |event: &MouseDownEvent, phase, window, _| {
                                    let mut records = mouse_trace.borrow_mut();
                                    if records.len() < 64 {
                                        records.push(serde_json::json!({
                                            "phase": format!("{phase:?}"),
                                            "button": format!("{:?}", event.button),
                                            "position": [f32::from(event.position.x), f32::from(event.position.y)],
                                            "window_position": [f32::from(window.mouse_position().x), f32::from(window.mouse_position().y)],
                                            "viewport": [f32::from(window.viewport_size().width), f32::from(window.viewport_size().height)],
                                            "captured_hitbox": format!("{:?}", window.captured_hitbox()),
                                        }));
                                    }
                                },
                            );
                        },
                    )
                    .absolute()
                    .size_full(),
                )
            })
            .child(
                div()
                    .w(self.pane_width)
                    .h_full()
                    .flex_shrink_0()
                    .child(self.view.clone()),
            )
            .child(div().flex_1().h_full())
    }
}

#[cfg(feature = "fanta-gpui-ui")]
fn draw_pane_layout(visual: &mut gpui::VisualTestContext) {
    for _ in 0..3 {
        visual.run_until_parked();
        visual.update(|window, cx| {
            window.refresh();
            window.draw(cx).clear();
        });
    }
}

#[cfg(feature = "fanta-gpui-ui")]
fn pane_layout_fixture(
    project: &Entity<Project>,
    doc: Doc,
    images: BTreeMap<AssetId, Vec<u8>>,
    cx: &mut TestAppContext,
) -> (
    Entity<FigItem>,
    Entity<FigView>,
    Entity<PaneLayoutTestHost>,
    gpui::VisualTestContext,
) {
    let item = crate::document::ready_item_for_test(
        project,
        std::path::PathBuf::from("/tmp/PaneLayout.fig"),
        doc,
        cx,
    );
    item.update(cx, |item, cx| {
        item.with_document(cx, |document| {
            let (_, mut assets) = document.doc_and_assets();
            for (expected, bytes) in images {
                let (actual, _) = assets.add_image(bytes).expect("fixture image");
                assert_eq!(actual, expected);
            }
            ((), DocChange::None)
        })
        .expect("ready fixture document");
    });
    let window = cx.add_window({
        let item = item.clone();
        let project = project.clone();
        move |window, cx| {
            let view = cx.new(|cx| FigView::new(item, project, window, cx));
            view.update(cx, |view, cx| {
                view.layers_sidebar_visible = true;
                view.inspector_sidebar_visible = true;
                view.layers_sidebar_width = px(280.0);
                view.inspector_sidebar_width = px(400.0);
                view.focus_handle.focus(window, cx);
            });
            window.activate_window();
            PaneLayoutTestHost {
                view,
                pane_width: px(1400.0),
                mouse_trace: None,
            }
        }
    });
    let host = window.entity(cx).expect("pane host");
    let view = host.read_with(cx, |host, _| host.view.clone());
    let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
    visual.simulate_resize(size(px(1536.0), px(1084.0)));
    draw_pane_layout(&mut visual);
    (item, view, host, visual)
}

/// GPUI keeps a painted image in the sprite atlas until `drop_image`, which `FigView` calls
/// with the frame it painted before. The atlas is private to GPUI, so this checks the
/// bookkeeping that decides which frame gets dropped.
#[cfg(feature = "fanta-gpui-ui")]
#[gpui::test]
async fn cpu_canvas_tracks_the_frame_it_painted(cx: &mut TestAppContext) {
    init_visual_test(cx);
    let project = Project::test(FakeFs::new(cx.executor()), [], cx).await;
    let (doc, _, images) = bitmap_canvas_doc();
    let (_item, view, _host, mut visual) = pane_layout_fixture(&project, doc, images, cx);
    let painted = |visual: &mut gpui::VisualTestContext| {
        view.read_with(visual, |view, _| {
            view.painted_canvas_image
                .clone()
                .expect("the canvas painted a CPU frame")
        })
    };

    let first = painted(&mut visual);
    draw_pane_layout(&mut visual);
    assert!(
        Arc::ptr_eq(&first, &painted(&mut visual)),
        "an unchanged viewport repaints the cached frame"
    );

    view.update(&mut visual, |view, cx| {
        let mut viewport = view.viewport().expect("viewport");
        viewport.center[0] += 40.0;
        view.set_viewport(viewport, cx);
    });
    draw_pane_layout(&mut visual);
    let second = painted(&mut visual);
    assert!(!Arc::ptr_eq(&first, &second), "a pan renders a new frame");
    assert_eq!(
        Arc::strong_count(&first),
        1,
        "the view keeps no frame it stopped painting"
    );
}

#[cfg(feature = "fanta-gpui-ui")]
#[gpui::test]
async fn pane_layout_measures_the_pane_and_restores_wide_sidebars(cx: &mut TestAppContext) {
    init_visual_test(cx);
    let project = Project::test(FakeFs::new(cx.executor()), [], cx).await;
    let (doc, _, images) = bitmap_canvas_doc();
    let (item, view, host, mut visual) = pane_layout_fixture(&project, doc, images, cx);
    let before = item.read_with(&visual, |item, _| {
        let document = item.document().expect("document");
        (
            serde_json::to_value(&document.doc).expect("document snapshot"),
            document.raw_assets.clone(),
            document.doc.history.undo_depth(),
            document.doc.history.redo_depth(),
            item.is_dirty(),
        )
    });
    for (width, canvas_width, inspector_width) in [
        (1400.0, 720.0, Some(400.0)),
        (588.0, 240.0, Some(348.0)),
        (450.0, 450.0, None),
    ] {
        host.update(&mut visual, |host, cx| {
            host.pane_width = px(width);
            cx.notify();
        });
        draw_pane_layout(&mut visual);
        view.read_with(&visual, |view, _| {
            assert_eq!(
                view.pane_width,
                Some(px(width)),
                "the window remains 1536 points wide"
            );
            assert!(view.layers_sidebar_visible && view.inspector_sidebar_visible);
            assert_eq!(
                (view.layers_sidebar_width, view.inspector_sidebar_width),
                (px(280.0), px(400.0))
            );
        });
        assert_eq!(
            visual
                .debug_bounds("fig-container")
                .expect("canvas")
                .size
                .width,
            px(canvas_width)
        );
        let inspector = visual.debug_bounds("fanta-inspector-sidebar");
        assert_eq!(
            inspector.map(|bounds| bounds.size.width),
            inspector_width.map(px)
        );
        if width < 1400.0 {
            assert!(visual.debug_bounds("file-inspector-toggle").is_some());
        }
    }
    let toggle = visual
        .debug_bounds("properties-inspector-toggle")
        .expect("compact Inspector toggle")
        .center();
    visual.simulate_click(toggle, gpui::Modifiers::none());
    draw_pane_layout(&mut visual);
    let inspector = visual
        .debug_bounds("fanta-inspector-sidebar")
        .expect("revealed Inspector");
    assert_eq!(inspector.size.width, px(400.0));
    assert_eq!(inspector.right(), px(450.0));
    assert_eq!(
        visual
            .debug_bounds("fig-container")
            .expect("canvas remains allocated")
            .size
            .width,
        px(450.0)
    );
    let toggle = visual
        .debug_bounds("properties-inspector-toggle")
        .expect("expanded Inspector toggle")
        .center();
    visual.simulate_click(toggle, gpui::Modifiers::none());
    draw_pane_layout(&mut visual);
    assert!(visual.debug_bounds("fanta-inspector-sidebar").is_none());
    let toggle = visual
        .debug_bounds("file-inspector-toggle")
        .expect("compact Layers toggle")
        .center();
    visual.simulate_click(toggle, gpui::Modifiers::none());
    draw_pane_layout(&mut visual);
    assert_eq!(
        visual
            .debug_bounds("fanta-layers-sidebar")
            .expect("revealed Layers")
            .size
            .width,
        px(280.0)
    );
    host.update(&mut visual, |host, cx| {
        host.pane_width = px(1400.0);
        cx.notify();
    });
    draw_pane_layout(&mut visual);
    assert_eq!(
        visual
            .debug_bounds("fanta-layers-sidebar")
            .expect("restored Layers")
            .size
            .width,
        px(280.0)
    );
    assert_eq!(
        visual
            .debug_bounds("fanta-inspector-sidebar")
            .expect("restored Inspector")
            .size
            .width,
        px(400.0)
    );
    assert_eq!(
        visual
            .debug_bounds("fig-container")
            .expect("restored canvas")
            .size
            .width,
        px(720.0)
    );
    item.read_with(&visual, |item, _| {
        let document = item.document().expect("document");
        assert_eq!(
            (
                serde_json::to_value(&document.doc).expect("document snapshot"),
                document.raw_assets.clone(),
                document.doc.history.undo_depth(),
                document.doc.history.redo_depth(),
                item.is_dirty()
            ),
            before
        );
    });
    view.read_with(&visual, |view, _| {
        assert!(view.layers_sidebar_visible && view.inspector_sidebar_visible);
        assert_eq!(
            (view.layers_sidebar_width, view.inspector_sidebar_width),
            (px(280.0), px(400.0))
        );
    });
}

#[cfg(feature = "fanta-gpui-ui")]
async fn assert_pane_layout_preserves_property_draft(draft: &str, cx: &mut TestAppContext) {
    use fanta_gpui::design::DesignPropertyDraftState;
    init_visual_test(cx);
    let project = Project::test(FakeFs::new(cx.executor()), [], cx).await;
    let (doc, text, _) = text_selection_doc(false);
    let (item, view, host, mut visual) =
        pane_layout_fixture(&project, doc, BTreeMap::new(), cx);
    item.update(&mut visual, |item, cx| {
        item.with_document(cx, |document| {
            document.doc.selection.select_only(text);
            ((), DocChange::Selection)
        })
        .expect("select mounted text");
    });
    draw_pane_layout(&mut visual);
    let panel = view.read_with(&visual, |view, _| {
        view.gpui_design.as_ref().expect("Design").panel.clone()
    });
    let field = visual
        .debug_bounds("fig-gpui-design-x")
        .expect("X input")
        .center();
    visual.simulate_click(field, gpui::Modifiers::none());
    visual.simulate_keystrokes("secondary-a");
    visual.simulate_input(draft);
    visual.run_until_parked();
    let focus = visual.update(|window, cx| window.focused(cx).expect("property input focus"));
    let expected_state = if draft == "-" {
        DesignPropertyDraftState::Invalid
    } else {
        DesignPropertyDraftState::Valid
    };
    assert_eq!(
        panel.read_with(&visual, |panel, cx| panel.property_draft_state(cx)),
        expected_state
    );
    let before = item.read_with(&visual, |item, _| {
        let document = item.document().expect("document");
        (
            serde_json::to_value(&document.doc).expect("draft preview"),
            document.raw_assets.clone(),
            document.doc.history.undo_depth(),
            document.doc.history.redo_depth(),
            item.is_dirty(),
        )
    });
    for width in [588.0, 450.0, 1400.0] {
        host.update(&mut visual, |host, cx| {
            host.pane_width = px(width);
            cx.notify();
        });
        draw_pane_layout(&mut visual);
        visual.update(|window, _| {
            assert!(
                focus.is_focused(window),
                "resizing preserves the actual input focus"
            )
        });
        assert_eq!(
            panel.read_with(&visual, |panel, cx| panel.property_draft_state(cx)),
            expected_state
        );
        view.read_with(&visual, |view, _| {
            assert_eq!(
                view.gpui_design
                    .as_ref()
                    .expect("same Design")
                    .panel
                    .entity_id(),
                panel.entity_id()
            );
            assert!(view.inspector_sidebar_visible && view.layers_sidebar_visible);
            assert_eq!(
                (view.layers_sidebar_width, view.inspector_sidebar_width),
                (px(280.0), px(400.0))
            );
        });
        let inspector = visual
            .debug_bounds("fanta-inspector-sidebar")
            .expect("draft remains visible");
        let field = visual
            .debug_bounds("fig-gpui-design-x")
            .expect("same mounted input");
        assert!(inspector.size.width >= px(MIN_INSPECTOR_SIDEBAR_WIDTH));
        assert!(field.left() >= inspector.left() && field.right() <= inspector.right());
        assert!(inspector.right() <= px(width));
        visual.simulate_keystrokes("secondary-a secondary-c");
        assert_eq!(
            visual
                .read_from_clipboard()
                .expect("copied actual input")
                .text()
                .as_deref(),
            Some(draft)
        );
        item.read_with(&visual, |item, _| {
            let document = item.document().expect("document");
            assert_eq!(
                (
                    serde_json::to_value(&document.doc).expect("draft preview"),
                    document.raw_assets.clone(),
                    document.doc.history.undo_depth(),
                    document.doc.history.redo_depth(),
                    item.is_dirty()
                ),
                before
            );
        });
    }
}

#[cfg(feature = "fanta-gpui-ui")]
#[gpui::test]
async fn pane_layout_preserves_valid_property_input_across_compaction(cx: &mut TestAppContext) {
    assert_pane_layout_preserves_property_draft("48", cx).await;
}

#[cfg(feature = "fanta-gpui-ui")]
#[gpui::test]
async fn pane_layout_preserves_invalid_property_input_across_compaction(
    cx: &mut TestAppContext,
) {
    assert_pane_layout_preserves_property_draft("-", cx).await;
}

async fn assert_mounted_sidebar_divider_drag(
    sidebar_kind: SidebarKind,
    first_move_finishes_drag: bool,
    cx: &mut TestAppContext,
) {
    init_visual_test(cx);
    let project = Project::test(FakeFs::new(cx.executor()), [], cx).await;
    let (item, view, mut visual) =
        mounted_canvas_interaction_fixture(&project, doc_with_one_page(), BTreeMap::new(), cx);
    view.update(&mut visual, |view, cx| {
        match sidebar_kind {
            SidebarKind::Layers => {
                view.layers_sidebar_visible = true;
                view.layers_sidebar_width = px(320.0);
            }
            SidebarKind::Inspector => {
                view.inspector_sidebar_visible = true;
                view.inspector_sidebar_width = px(320.0);
            }
        }
        cx.notify();
    });
    visual.run_until_parked();
    visual.update(|window, cx| {
        window.refresh();
        window.draw(cx).clear();
    });
    let (handle_selector, sidebar_selector) = match sidebar_kind {
        SidebarKind::Layers => ("fanta-layers-sidebar-resize-handle", "fanta-layers-sidebar"),
        SidebarKind::Inspector => (
            "fanta-inspector-sidebar-resize-handle",
            "fanta-inspector-sidebar",
        ),
    };
    let handle = visual
        .debug_bounds(handle_selector)
        .expect("actual divider hit region");
    let sidebar = visual.debug_bounds(sidebar_selector).expect("sidebar");
    assert_eq!(sidebar.size.width, px(320.0));
    assert!(handle.size.width >= px(4.0));
    let original = item.read_with(&visual, |item, _| {
        serde_json::to_value(&item.document().expect("document").doc).expect("snapshot")
    });
    let start = handle.center();
    let direction = match sidebar_kind {
        SidebarKind::Layers => 1.0,
        SidebarKind::Inspector => -1.0,
    };
    let finish = start + point(px(direction * 120.0), px(0.0));
    assert!(!sidebar.contains(&finish), "release is outside the sidebar");
    visual.simulate_mouse_down(start, MouseButton::Left, gpui::Modifiers::none());
    visual.run_until_parked();
    if !first_move_finishes_drag {
        visual.simulate_mouse_move(
            start + point(px(direction * 20.0), px(0.0)),
            MouseButton::Left,
            gpui::Modifiers::none(),
        );
        visual.run_until_parked();
    }
    visual.simulate_mouse_move(finish, MouseButton::Left, gpui::Modifiers::none());
    visual.run_until_parked();
    visual.update(|_, cx| {
        assert!(
            cx.has_active_drag(),
            "actual GPUI drag has captured the divider"
        );
    });
    visual.simulate_mouse_up(finish, MouseButton::Left, gpui::Modifiers::none());
    visual.run_until_parked();
    let expected = match sidebar_kind {
        SidebarKind::Layers => finish.x - sidebar.left(),
        SidebarKind::Inspector => sidebar.right() - finish.x,
    };
    view.read_with(&visual, |view, _| {
        let actual = match sidebar_kind {
            SidebarKind::Layers => view.layers_sidebar_width,
            SidebarKind::Inspector => view.inspector_sidebar_width,
        };
        assert_eq!(actual, expected);
    });
    visual.update(|window, cx| {
        assert!(
            !cx.has_active_drag(),
            "released divider must not remain captured"
        );
        window.refresh();
        window.draw(cx).clear();
    });
    assert_eq!(
        visual
            .debug_bounds(sidebar_selector)
            .expect("sidebar")
            .size
            .width,
        expected
    );
    item.read_with(&visual, |item, _| {
        assert_eq!(
            serde_json::to_value(&item.document().expect("document").doc).expect("snapshot"),
            original,
            "resizing a sidebar must not edit or select canvas content"
        );
    });
}

#[gpui::test]
async fn mounted_inspector_divider_single_move_and_release_resizes(cx: &mut TestAppContext) {
    assert_mounted_sidebar_divider_drag(SidebarKind::Inspector, true, cx).await;
}

#[gpui::test]
async fn mounted_inspector_divider_two_move_drag_resizes(cx: &mut TestAppContext) {
    assert_mounted_sidebar_divider_drag(SidebarKind::Inspector, false, cx).await;
}

#[gpui::test]
async fn mounted_layers_divider_single_move_and_release_resizes(cx: &mut TestAppContext) {
    assert_mounted_sidebar_divider_drag(SidebarKind::Layers, true, cx).await;
}

#[gpui::test]
async fn motion_timeline_spans_below_both_editor_sidebars(cx: &mut TestAppContext) {
    init_visual_test(cx);
    let project = Project::test(FakeFs::new(cx.executor()), [], cx).await;
    let item = crate::document::ready_item_for_test(
        &project,
        std::path::PathBuf::from("/tmp/Motion-layout.fig"),
        doc_with_one_page(),
        cx,
    );
    let window = cx.add_window(move |window, cx| FigView::new(item, project, window, cx));
    let view = window.entity(cx).expect("fig view");
    view.update(cx, |view, cx| {
        view.set_editor_mode(EditorMode::Motion, cx);
    });

    let mut visual_context = gpui::VisualTestContext::from_window(window.into(), cx);
    visual_context.simulate_resize(size(px(1_200.), px(800.)));
    visual_context.update(|window, cx| window.draw(cx).clear());

    let layers = visual_context
        .debug_bounds("fanta-layers-sidebar")
        .expect("layers sidebar");
    let inspector = visual_context
        .debug_bounds("fanta-inspector-sidebar")
        .expect("inspector sidebar");
    let timeline = visual_context
        .debug_bounds("timeline")
        .expect("motion timeline");
    let motion_panel = visual_context
        .debug_bounds("fanta-motion-panel")
        .expect("motion inspector panel");
    let close = |left: Pixels, right: Pixels| (left - right).abs() <= px(1.);

    assert!(close(timeline.left(), layers.left()));
    assert!(close(timeline.right(), inspector.right()));
    assert!(close(timeline.top(), layers.bottom()));
    assert!(close(timeline.top(), inspector.bottom()));
    assert_eq!(timeline.size.height, px(300.));
    assert!(close(motion_panel.left(), inspector.left()));
    assert!(close(motion_panel.right(), inspector.right()));
    assert!(close(motion_panel.bottom(), inspector.bottom()));
    assert!(motion_panel.top() > inspector.top());
}

/// Select All takes the active page's own children — never the page
/// roots, which is what an unscoped `children_of(None)` would return.
#[gpui::test]
async fn select_all_selects_the_active_pages_top_level_nodes(cx: &mut TestAppContext) {
    init_test(cx);
    let project = Project::test(FakeFs::new(cx.executor()), [], cx).await;
    let mut doc = doc_with_one_page();
    let page = doc.active_page().expect("active page");
    let mut first = CanvasNode::new(NodeData::Group(GroupNode::default()));
    first.parent = Some(page);
    let first_id = first.id;
    doc.apply(Operation::create_node(first))
        .expect("create first node");
    let mut second = CanvasNode::new(NodeData::Group(GroupNode::default()));
    second.parent = Some(page);
    let second_id = second.id;
    doc.apply(Operation::create_node(second))
        .expect("create second node");
    let item = crate::document::ready_item_for_test(
        &project,
        std::path::PathBuf::from("/tmp/Design.fig"),
        doc,
        cx,
    );
    let scratch = cx.add_window(|_, _| gpui::Empty);
    let view = scratch
        .update(cx, |_, window, cx| {
            cx.new(|cx| FigView::new(item.clone(), project.clone(), window, cx))
        })
        .expect("create fig view");

    scratch
        .update(cx, |_, window, cx| {
            view.update(cx, |view, cx| view.select_all(&SelectAll, window, cx));
        })
        .expect("select all");

    view.read_with(cx, |view, cx| {
        let mut selection = view
            .item
            .read(cx)
            .doc()
            .expect("document")
            .selection
            .as_slice()
            .to_vec();
        let mut expected = vec![first_id, second_id];
        // The selection does not promise insertion order, so compare as sets.
        selection.sort();
        expected.sort();
        assert_eq!(selection, expected);
    });
}

#[gpui::test]
async fn tool_activation_updates_before_canvas_viewport_exists(cx: &mut TestAppContext) {
    init_test(cx);
    let project = Project::test(FakeFs::new(cx.executor()), [], cx).await;
    let item = crate::document::ready_item_for_test(
        &project,
        std::path::PathBuf::from("/tmp/Design.fig"),
        doc_with_one_page(),
        cx,
    );
    let scratch = cx.add_window(|_, _| gpui::Empty);
    let view = scratch
        .update(cx, |_, window, cx| {
            cx.new(|cx| FigView::new(item.clone(), project.clone(), window, cx))
        })
        .unwrap();

    view.update(cx, |view, cx| {
        assert_eq!(view.active_tool(), ToolKind::Select);
        view.activate_tool(ToolKind::Text, cx);
        assert_eq!(view.active_tool(), ToolKind::Text);
    });
}

#[gpui::test]
async fn prototype_phone_preset_sizes_the_visible_presentation_surface(
    cx: &mut TestAppContext,
) {
    init_visual_test(cx);
    let project = Project::test(FakeFs::new(cx.executor()), [], cx).await;
    let mut document = doc_with_one_page();
    let mut frame = CanvasNode::new(NodeData::Group(GroupNode {
        clip_size: Some([390.0, 844.0]),
        ..GroupNode::default()
    }));
    frame.parent = document.active_page();
    document
        .apply(Operation::create_node(frame))
        .expect("create prototype frame");
    document.presentation = Some(fanta_doc::PresentationConfig {
        device_size: Some([390.0, 844.0]),
        preset: Some("phone".into()),
        ..Default::default()
    });
    let item = crate::document::ready_item_for_test(
        &project,
        std::path::PathBuf::from("/tmp/PhonePrototype.fig"),
        document,
        cx,
    );
    let window = cx.add_window(move |window, cx| FigView::new(item, project, window, cx));
    window
        .update(cx, |view, window, cx| {
            view.play_prototype(&PlayPrototype, window, cx)
        })
        .expect("start prototype presentation");
    cx.run_until_parked();
    window
        .read_with(cx, |view, _| assert!(view.is_presenting_prototype()))
        .expect("read prototype presentation state");

    let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
    visual.simulate_resize(size(px(1200.0), px(900.0)));
    visual.update(|window, cx| window.draw(cx).clear());
    assert!(
        visual
            .debug_bounds("fanta-prototype-presentation")
            .is_some()
    );
    let surface = visual
        .debug_bounds("fig-prototype-container")
        .expect("phone presentation surface");
    let width = f32::from(surface.size.width);
    let height = f32::from(surface.size.height);
    assert!((300.0..400.0).contains(&width), "phone width: {width}");
    assert!((700.0..800.0).contains(&height), "phone height: {height}");
    assert!(height > width * 2.0);
}

#[gpui::test]
async fn prototype_presentation_navigates_and_restores_the_editor_viewport(
    cx: &mut TestAppContext,
) {
    init_test(cx);
    let project = Project::test(FakeFs::new(cx.executor()), [], cx).await;
    let mut doc = doc_with_one_page();
    let page = doc.active_page().expect("active page");
    let mut first = CanvasNode::new(NodeData::Group(GroupNode {
        clip_size: Some([320.0, 180.0]),
        ..GroupNode::default()
    }));
    first.parent = Some(page);
    first.name = "First".into();
    doc.apply(Operation::create_node(first))
        .expect("create first prototype frame");
    let mut second = CanvasNode::new(NodeData::Group(GroupNode {
        clip_size: Some([320.0, 180.0]),
        ..GroupNode::default()
    }));
    second.parent = Some(page);
    second.name = "Second".into();
    second.transform = Transform2D::translation(400.0, 0.0);
    doc.apply(Operation::create_node(second))
        .expect("create second prototype frame");
    let ordered_frames = doc.scene.children_of(Some(page)).to_vec();
    let start_id = ordered_frames[0];
    let next_id = ordered_frames[1];
    doc.apply(Operation::SetFlowStart {
        old: doc.flow_start(),
        new: Some(start_id),
    })
    .expect("set prototype starting point");
    doc.history = Default::default();
    let item = crate::document::ready_item_for_test(
        &project,
        std::path::PathBuf::from("/tmp/Prototype.fig"),
        doc,
        cx,
    );
    let scratch = cx.add_window(|_, _| gpui::Empty);
    let view = scratch
        .update(cx, |_, window, cx| {
            cx.new(|cx| FigView::new(item.clone(), project.clone(), window, cx))
        })
        .expect("create fig view");
    let editor_viewport = Viewport {
        center: [42.0, 24.0],
        zoom: 1.5,
    };

    scratch
        .update(cx, |_, window, cx| {
            view.update(cx, |view, cx| {
                view.viewport = Some(editor_viewport);
                view.play_prototype(&PlayPrototype, window, cx);
            });
        })
        .expect("start prototype presentation");
    view.read_with(cx, |view, cx| {
        assert_eq!(view.editor_mode(cx), EditorMode::Prototype);
        assert_eq!(
            view.prototype_player
                .as_ref()
                .map(PrototypePlayerState::current_frame),
            Some(start_id)
        );
    });

    view.update(cx, |view, cx| view.show_next_prototype_frame(cx));
    view.read_with(cx, |view, _| {
        assert_eq!(
            view.prototype_player
                .as_ref()
                .map(PrototypePlayerState::current_frame),
            Some(next_id)
        );
    });

    view.update(cx, |view, cx| view.exit_prototype_session(cx));
    view.read_with(cx, |view, _| {
        assert!(view.prototype_player.is_none());
        let restored = view.viewport.expect("restored editor viewport");
        assert_eq!(restored.center, editor_viewport.center);
        assert_eq!(restored.zoom, editor_viewport.zoom);
    });
}

fn asset_clipboard_subtree() -> (Doc, NodeId, NodeId, BTreeMap<AssetId, Vec<u8>>) {
    let (mut doc, bitmap, images) = bitmap_canvas_doc();
    let page = doc.active_page().expect("page");
    let asset = *images.keys().next().expect("PNG asset");
    let mut frame = CanvasNode::new(NodeData::Group(GroupNode {
        clip_size: Some([240.0, 140.0]),
        ..Default::default()
    }));
    frame.name = "Asset subtree".into();
    frame.parent = Some(page);
    frame.index = doc.scene.next_child_index(Some(page));
    frame.transform = Transform2D::translation(-120.0, -70.0);
    let root = frame.id;
    doc.scene.insert(frame).expect("frame");
    let mut bitmap_node = doc.scene.get(bitmap).expect("bitmap").clone();
    doc.scene
        .remove(bitmap)
        .expect("remove old bitmap position");
    bitmap_node.parent = Some(root);
    bitmap_node.index = doc.scene.next_child_index(Some(root));
    bitmap_node.transform = Transform2D::translation(10.0, 10.0);
    doc.scene
        .insert(bitmap_node)
        .expect("reparent bitmap fixture");
    let mut image_vector = VectorNode::rect_solid(0.0, 0.0, 20.0, 30.0, Color::BLACK);
    image_vector.fills = [Fill::Image {
        asset,
        mode: fanta_doc::ImageFitMode::Fit,
        opacity: 0.7,
        crop: Some(Box::new([0.1, 0.2, 0.6, 0.5])),
        scale: Some(1.25),
        rotation: Some(30.0),
        blend: fanta_doc::BlendMode::Multiply,
        adjust: Default::default(),
    }]
    .into_iter()
    .collect();
    let mut vector = CanvasNode::new(NodeData::Vector(image_vector));
    vector.parent = Some(root);
    vector.index = doc.scene.next_child_index(Some(root));
    vector.transform = Transform2D::translation(210.0, 90.0);
    doc.scene.insert(vector).expect("image fill vector");
    let mut control =
        CanvasNode::new(NodeData::Text(TextNode::new("Do not modify", 100.0, 30.0)));
    control.parent = Some(page);
    control.index = doc.scene.next_child_index(Some(page));
    control.transform = Transform2D::translation(350.0, 150.0);
    doc.scene.insert(control).expect("unselected control");
    doc.selection.select_only(root);
    doc.history = Default::default();
    (doc, root, bitmap, images)
}

fn asset_clipboard_content(doc: &Doc) -> serde_json::Value {
    let mut snapshot = doc.clone_for_persist();
    snapshot.history = Default::default();
    snapshot.selection.clear();
    snapshot.metadata.modified_at = 0;
    serde_json::to_value(snapshot).expect("complete clipboard content")
}

fn bind_asset_clipboard_commands(cx: &mut TestAppContext) {
    cx.update(|cx| {
        cx.bind_keys([
            gpui::KeyBinding::new("cmd-x", CutSelection, Some("FigViewer")),
            gpui::KeyBinding::new("cmd-d", DuplicateSelection, Some("FigViewer")),
            gpui::KeyBinding::new("backspace", DeleteSelection, Some("FigViewer")),
        ]);
    });
}

async fn assert_asset_clipboard_lock_guards(command: &str, cx: &mut TestAppContext) {
    init_visual_test(cx);
    bind_asset_clipboard_commands(cx);
    let project = Project::test(FakeFs::new(cx.executor()), [], cx).await;
    for guard in ["node", "ancestor", "mixed parent", "source draft"] {
        let (mut doc, root, bitmap, images) = asset_clipboard_subtree();
        doc.selection.select_only(bitmap);
        if guard != "source draft" {
            let locked = if guard == "ancestor" { root } else { bitmap };
            doc.scene
                .get_mut(locked)
                .expect("locked node")
                .flags
                .insert(fanta_doc::NodeFlags::LOCKED);
        }
        if guard == "mixed parent" {
            doc.selection.replace_with([root, bitmap]);
        }
        let selection = doc.selection.as_slice().to_vec();
        let (item, view, mut visual) =
            mounted_canvas_interaction_fixture(&project, doc, images.clone(), cx);
        if guard == "source draft" {
            item.update(&mut visual, |item, cx| {
                item.set_source_edit_locked(true, cx)
            });
        }
        visual.update(|window, cx| {
            view.update(cx, |view, cx| view.focus_handle.focus(window, cx));
            window.draw(cx).clear();
            cx.write_to_clipboard(ClipboardItem::new_string("untouched clipboard".into()));
        });
        let before = item.read_with(&visual, |item, _| {
            asset_clipboard_content(item.doc().expect("document"))
        });
        visual.simulate_keystrokes(command);
        visual.run_until_parked();
        item.read_with(&visual, |item, _| {
            let document = item.document().expect("document");
            assert_eq!(
                asset_clipboard_content(&document.doc),
                before,
                "{command} must respect {guard} protection"
            );
            assert_eq!(document.doc.selection.as_slice(), selection);
            assert_eq!(document.raw_assets.as_ref(), &images);
            assert_eq!(document.doc.history.undo_depth(), 0);
            assert!(!item.is_dirty());
        });
        if command == "cmd-x" {
            let clipboard = visual.read_from_clipboard().expect("original clipboard");
            assert_eq!(
                clipboard.text().as_deref(),
                Some("untouched clipboard"),
                "a refused Cut must not replace the clipboard"
            );
        }
    }
}

#[gpui::test]
async fn mounted_asset_clipboard_cut_respects_node_ancestor_and_source_locks(
    cx: &mut TestAppContext,
) {
    assert_asset_clipboard_lock_guards("cmd-x", cx).await;
}

#[gpui::test]
async fn mounted_asset_clipboard_duplicate_respects_node_ancestor_and_source_locks(
    cx: &mut TestAppContext,
) {
    assert_asset_clipboard_lock_guards("cmd-d", cx).await;
}

#[gpui::test]
async fn mounted_asset_clipboard_delete_respects_node_ancestor_and_source_locks(
    cx: &mut TestAppContext,
) {
    assert_asset_clipboard_lock_guards("backspace", cx).await;
}

async fn assert_asset_clipboard_remove_master(
    set_reference: bool,
    command: &str,
    cx: &mut TestAppContext,
) {
    init_visual_test(cx);
    bind_asset_clipboard_commands(cx);
    let project = Project::test(FakeFs::new(cx.executor()), [], cx).await;
    let (mut doc, root, bitmap, images) = asset_clipboard_subtree();
    let component = fanta_doc::ComponentId::new();
    doc.components.defs.insert(
        component,
        fanta_doc::ComponentDef::new(component, root, "Asset master"),
    );
    if set_reference {
        let collection = VariableCollectionId::new();
        let mode = ModeId::new();
        let variable = VariableId::new();
        doc.variables.collections.insert(
            collection,
            VariableCollection {
                id: collection,
                name: "Opacity".into(),
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
                name: "Photo opacity".into(),
                ty: VariableType::Float,
                values_by_mode: BTreeMap::from([(mode, VarValue::Float { value: 0.4 })]),
                scopes: Vec::new(),
                options: Vec::new(),
            },
        );
        doc.components
            .defs
            .get_mut(&component)
            .expect("master definition")
            .props
            .push(fanta_doc::ComponentPropDef {
                id: fanta_doc::ComponentPropId::new(),
                name: "Photo opacity".into(),
                kind: fanta_doc::ComponentPropKind::Number,
                formatter: Default::default(),
                default: VarValue::Alias { variable },
                bindings: vec![fanta_doc::PropBindingTarget {
                    path: [bitmap].into_iter().collect(),
                    prop: fanta_doc::BoundProp::Opacity,
                }],
            });
    }
    let instance_component = if set_reference {
        let set = fanta_doc::ComponentId::new();
        doc.components
            .defs
            .get_mut(&component)
            .expect("master definition")
            .variant_of = Some(fanta_doc::ComponentSetMembership {
            set,
            axis_values: Default::default(),
        });
        doc.components.sets.insert(
            set,
            fanta_doc::ComponentSet {
                id: set,
                name: "Asset variants".into(),
                axes: Vec::new(),
                members: vec![component],
                default_variant: component,
                root: None,
            },
        );
        set
    } else {
        component
    };
    let mut instance = CanvasNode::new(NodeData::Instance(fanta_doc::InstanceNode {
        component: instance_component,
        overrides: Vec::new(),
        prop_values: Default::default(),
        derived: Vec::new(),
        local_size: [240.0, 140.0],
    }));
    instance.parent = doc.active_page();
    instance.index = doc.scene.next_child_index(instance.parent);
    instance.transform = Transform2D::translation(350.0, -70.0);
    let survivor = instance.id;
    doc.scene.insert(instance).expect("surviving instance");
    let (item, view, mut visual) =
        mounted_canvas_interaction_fixture(&project, doc, images.clone(), cx);
    visual.update(|window, cx| {
        view.update(cx, |view, cx| view.focus_handle.focus(window, cx));
        window.draw(cx).clear();
        cx.write_to_clipboard(ClipboardItem::new_string("original clipboard".into()));
    });
    let before = item.read_with(&visual, |item, _| {
        asset_clipboard_content(item.doc().expect("document"))
    });
    visual.simulate_keystrokes(command);
    visual.run_until_parked();
    if command == "backspace" {
        assert_eq!(
            visual
                .read_from_clipboard()
                .expect("clipboard")
                .text()
                .as_deref(),
            Some("original clipboard"),
            "Delete does not replace clipboard contents"
        );
    }

    let after = item.read_with(&visual, |item, _| {
        let document = item.document().expect("document");
        let doc = &document.doc;
        assert!(!doc.scene.contains(root), "Cut removes its selected master subtree");
        assert!(doc.scene.contains(survivor), "unselected placed content survives Cut");
        for definition in doc.components.defs.values() {
            assert!(doc.scene.contains(definition.root), "Cut must not leave a component definition pointing at deleted content");
        }
        let node = doc.scene.get(survivor).expect("survivor");
        let surviving_nodes = match &node.data {
            NodeData::Instance(instance) => fanta_doc::expand_instance(&doc.scene, &doc.components, instance).into_iter().map(|expanded| expanded.node).collect::<Vec<_>>(),
            _ => doc.scene.descendants_of(survivor).filter_map(|id| doc.scene.get(id).cloned()).collect(),
        };
        assert!(surviving_nodes.iter().any(|node| matches!(&node.data, NodeData::Bitmap(bitmap) if images.contains_key(&bitmap.asset))), "the unselected instance must still display its actual asset-bearing content");
        if set_reference {
            let photo = surviving_nodes.iter().find(|node| matches!(node.data, NodeData::Bitmap(_))).expect("surviving photo");
            assert!((photo.opacity.get() - 0.4).abs() < 1e-6, "detachment must preserve the variable-resolved instance appearance, got {}", photo.opacity.get());
        }
        assert_eq!(document.raw_assets.as_ref(), &images);
        assert_eq!(doc.history.undo_depth(), 1);
        asset_clipboard_content(doc)
    });
    visual.simulate_keystrokes("cmd-z");
    visual.run_until_parked();
    item.read_with(&visual, |item, _| {
        let document = item.document().expect("document");
        assert_eq!(
            asset_clipboard_content(&document.doc),
            before,
            "one Undo restores master and original instance references"
        );
        assert_eq!(document.raw_assets.as_ref(), &images);
        assert_eq!(document.doc.history.undo_depth(), 0);
    });
    visual.simulate_keystrokes("cmd-shift-z");
    visual.run_until_parked();
    item.read_with(&visual, |item, _| {
        let document = item.document().expect("document");
        assert_eq!(
            asset_clipboard_content(&document.doc),
            after,
            "Redo restores the exact safe Cut result"
        );
        assert_eq!(document.raw_assets.as_ref(), &images);
        assert_eq!(document.doc.history.undo_depth(), 1);
    });
}

#[gpui::test]
async fn mounted_asset_clipboard_cut_master_preserves_surviving_instance_and_undo(
    cx: &mut TestAppContext,
) {
    assert_asset_clipboard_remove_master(false, "cmd-x", cx).await;
}

#[gpui::test]
async fn mounted_asset_clipboard_cut_last_variant_preserves_set_instance_and_undo(
    cx: &mut TestAppContext,
) {
    assert_asset_clipboard_remove_master(true, "cmd-x", cx).await;
}

#[gpui::test]
async fn mounted_asset_clipboard_delete_last_variant_preserves_set_instance_and_undo(
    cx: &mut TestAppContext,
) {
    assert_asset_clipboard_remove_master(true, "backspace", cx).await;
}

async fn assert_asset_clipboard_cut_paste_restores_components(
    include_local_instance: bool,
    set_reference: bool,
    copy_then_delete: bool,
    cx: &mut TestAppContext,
) {
    init_visual_test(cx);
    bind_asset_clipboard_commands(cx);
    cx.update(|cx| {
        cx.bind_keys([
            gpui::KeyBinding::new("cmd-v", PasteSelection, Some("FigViewer")),
            gpui::KeyBinding::new("cmd-c", CopySelection, Some("FigViewer")),
        ])
    });
    let project = Project::test(FakeFs::new(cx.executor()), [], cx).await;
    let (mut doc, master, bitmap, images) = asset_clipboard_subtree();
    let component = fanta_doc::ComponentId::new();
    doc.components.defs.insert(
        component,
        fanta_doc::ComponentDef::new(component, master, "Movable asset master"),
    );
    let opacity_property = fanta_doc::ComponentPropId::new();
    doc.components
        .defs
        .get_mut(&component)
        .expect("master")
        .props
        .push(fanta_doc::ComponentPropDef {
            id: opacity_property,
            name: "Photo opacity".into(),
            kind: fanta_doc::ComponentPropKind::Number,
            formatter: Default::default(),
            default: VarValue::Float { value: 0.6 },
            bindings: vec![fanta_doc::PropBindingTarget {
                path: [bitmap].into_iter().collect(),
                prop: fanta_doc::BoundProp::Opacity,
            }],
        });
    let reference = if set_reference {
        let set = fanta_doc::ComponentId::new();
        doc.components
            .defs
            .get_mut(&component)
            .expect("master")
            .variant_of = Some(fanta_doc::ComponentSetMembership {
            set,
            axis_values: Default::default(),
        });
        doc.components.sets.insert(
            set,
            fanta_doc::ComponentSet {
                id: set,
                name: "Movable asset variants".into(),
                axes: Vec::new(),
                members: vec![component],
                default_variant: component,
                root: None,
            },
        );
        set
    } else {
        component
    };
    let root = if include_local_instance {
        let page = doc.active_page().expect("page");
        let mut section = CanvasNode::new(NodeData::Group(GroupNode {
            clip_size: Some([640.0, 300.0]),
            ..Default::default()
        }));
        section.parent = Some(page);
        section.index = doc.scene.next_child_index(Some(page));
        let section_id = section.id;
        doc.scene.insert(section).expect("selection container");
        let mut master_node = doc.scene.get(master).expect("master").clone();
        let children = doc
            .scene
            .descendants_of(master)
            .filter_map(|id| doc.scene.get(id).cloned())
            .collect::<Vec<_>>();
        doc.scene
            .remove(master)
            .expect("remove old master placement");
        master_node.parent = Some(section_id);
        master_node.index = doc.scene.next_child_index(Some(section_id));
        doc.scene
            .insert(master_node)
            .expect("move master into selection");
        for node in children.into_iter().filter(|node| node.id != master) {
            doc.scene.insert(node).expect("restore master child");
        }
        let mut instance = CanvasNode::new(NodeData::Instance(fanta_doc::InstanceNode {
            component: reference,
            overrides: Vec::new(),
            prop_values: BTreeMap::from([(opacity_property, VarValue::Float { value: 0.4 })]),
            derived: vec![fanta_doc::DerivedOverride {
                path: [bitmap].into_iter().collect(),
                transform: Some(Transform2D::translation(23.0, 17.0)),
                size: None,
                fills: None,
                path_data: None,
                stroke_path: None,
                stroke_weight: None,
                text: None,
            }],
            local_size: [240.0, 140.0],
        }));
        instance.parent = Some(section_id);
        instance.index = doc.scene.next_child_index(Some(section_id));
        instance.transform = Transform2D::translation(300.0, 0.0);
        doc.scene.insert(instance).expect("selected local instance");
        section_id
    } else {
        master
    };
    doc.selection.select_only(root);
    doc.history = Default::default();
    let (item, view, mut visual) =
        mounted_canvas_interaction_fixture(&project, doc, images.clone(), cx);
    visual.update(|window, cx| {
        view.update(cx, |view, cx| view.focus_handle.focus(window, cx));
        window.draw(cx).clear();
    });
    let before = item.read_with(&visual, |item, _| item.doc().expect("document").clone());
    visual.simulate_keystrokes(if copy_then_delete {
        "cmd-c backspace"
    } else {
        "cmd-x"
    });
    visual.run_until_parked();
    let after_cut = item.read_with(&visual, |item, _| {
        let document = item.document().expect("document");
        assert!(!document.doc.scene.contains(root));
        assert!(document.doc.components.def(component).is_none());
        assert_eq!(document.raw_assets.as_ref(), &images);
        assert_eq!(document.doc.history.undo_depth(), 1);
        asset_clipboard_content(&document.doc)
    });
    visual.simulate_keystrokes("cmd-v");
    visual.run_until_parked();
    let after_paste = item.read_with(&visual, |item, _| {
        let document = item.document().expect("document");
        let doc = &document.doc;
        let pasted = *doc.selection.as_slice().first().expect("selected pasted subtree");
        let nodes = doc.scene.descendants_of(pasted).collect::<Vec<_>>();
        let definitions = doc.components.defs.values().filter(|definition| nodes.contains(&definition.root)).collect::<Vec<_>>();
        assert_eq!(definitions.len(), 1, "Cut then Paste restores a functional component master, not an ordinary group");
        let definition = definitions.first().expect("restored definition");
        assert_eq!(definition.name, "Movable asset master");
        let instances = nodes.iter().filter_map(|id| match &doc.scene.get(*id).expect("pasted node").data { NodeData::Instance(instance) => Some(instance), _ => None }).collect::<Vec<_>>();
        assert_eq!(instances.len(), usize::from(include_local_instance), "placed instances retain component semantics");
        for instance in instances {
            let expected = if set_reference {
                let set = doc.components.sets.values().next().expect("restored set");
                assert_eq!(set.members, vec![definition.id]);
                assert_eq!(set.default_variant, definition.id);
                assert_eq!(definition.variant_of.as_ref().expect("membership").set, set.id);
                set.id
            } else { definition.id };
            assert_eq!(instance.component, expected, "local instance points at the restored master or set");
            let expanded = fanta_doc::expand_instance(&doc.scene, &doc.components, instance);
            let photo = expanded.iter().find(|entry| matches!(entry.node.data, NodeData::Bitmap(_))).expect("resolved photo");
            assert!((photo.node.opacity.get() - 0.4).abs() < 1e-6, "restored property binding targets the pasted bitmap");
            assert_eq!(photo.node.transform, Transform2D::translation(23.0, 17.0), "restored derived path targets the pasted bitmap");
            assert!(expanded.iter().any(|entry| matches!(&entry.node.data, NodeData::Bitmap(bitmap) if images.contains_key(&bitmap.asset))), "pasted instance still paints the actual bitmap");
        }
        for id in before.scene.roots().iter().flat_map(|id| before.scene.descendants_of(*id)).filter(|id| !before.scene.descendants_of(root).any(|removed| removed == *id)) {
            assert_eq!(doc.scene.get(id), before.scene.get(id), "unselected originals stay exact");
        }
        assert_eq!(document.raw_assets.as_ref(), &images);
        assert_eq!(doc.history.undo_depth(), 2);
        asset_clipboard_content(doc)
    });
    item.update(&mut visual, |item, cx| {
        item.with_document(cx, |document| {
            document.doc.selection.clear();
            ((), DocChange::Selection)
        })
    });
    visual.simulate_keystrokes("cmd-v");
    visual.run_until_parked();
    let after_second_paste = item.read_with(&visual, |item, _| {
        let doc = item.doc().expect("document");
        assert_eq!(
            doc.components.defs.len(),
            2,
            "each Paste restores an independent definition instead of rebinding the first copy"
        );
        assert_eq!(doc.components.sets.len(), if set_reference { 2 } else { 0 });
        for definition in doc.components.defs.values() {
            assert!(doc.scene.contains(definition.root));
        }
        for node in doc
            .scene
            .roots()
            .iter()
            .flat_map(|root| doc.scene.descendants_of(*root))
            .filter_map(|id| doc.scene.get(id))
        {
            if let NodeData::Instance(instance) = &node.data {
                assert!(
                    !fanta_doc::expand_instance(&doc.scene, &doc.components, instance)
                        .is_empty()
                );
            }
        }
        asset_clipboard_content(doc)
    });
    visual.simulate_keystrokes("cmd-z");
    visual.run_until_parked();
    item.read_with(&visual, |item, _| {
        assert_eq!(
            asset_clipboard_content(item.doc().expect("document")),
            after_paste,
            "Undo second Paste leaves first copy exact"
        )
    });
    visual.simulate_keystrokes("cmd-z");
    visual.run_until_parked();
    item.read_with(&visual, |item, _| {
        assert_eq!(
            asset_clipboard_content(item.doc().expect("document")),
            after_cut,
            "Undo Paste restores exact cut state"
        )
    });
    visual.simulate_keystrokes("cmd-z");
    visual.run_until_parked();
    item.read_with(&visual, |item, _| {
        let document = item.document().expect("document");
        assert_eq!(
            asset_clipboard_content(&document.doc),
            asset_clipboard_content(&before),
            "Undo Cut restores original component IDs and local instances"
        );
        assert_eq!(document.raw_assets.as_ref(), &images);
        assert!(!document.doc.history.can_undo());
    });
    visual.simulate_keystrokes("cmd-shift-z cmd-shift-z cmd-shift-z");
    visual.run_until_parked();
    item.read_with(&visual, |item, _| {
        let document = item.document().expect("document");
        assert_eq!(
            asset_clipboard_content(&document.doc),
            after_second_paste,
            "Redo restores the same pasted IDs, definitions and relationships"
        );
        assert_eq!(document.raw_assets.as_ref(), &images);
        assert!(!document.doc.history.can_redo());
    });
}

#[gpui::test]
async fn mounted_asset_clipboard_cut_paste_restores_direct_master(cx: &mut TestAppContext) {
    assert_asset_clipboard_cut_paste_restores_components(false, false, false, cx).await;
}

#[gpui::test]
async fn mounted_asset_clipboard_cut_paste_preserves_master_and_local_instance(
    cx: &mut TestAppContext,
) {
    assert_asset_clipboard_cut_paste_restores_components(true, false, false, cx).await;
}

#[gpui::test]
async fn mounted_asset_clipboard_cut_paste_restores_complete_variant_set(
    cx: &mut TestAppContext,
) {
    assert_asset_clipboard_cut_paste_restores_components(true, true, false, cx).await;
}

#[gpui::test]
async fn mounted_asset_clipboard_copy_delete_paste_restores_components(
    cx: &mut TestAppContext,
) {
    assert_asset_clipboard_cut_paste_restores_components(true, false, true, cx).await;
}

#[gpui::test]
async fn mounted_asset_clipboard_partial_variant_cut_preserves_content_and_clipboard_with_notice(
    cx: &mut TestAppContext,
) {
    init_visual_test(cx);
    bind_asset_clipboard_commands(cx);
    let file_system = FakeFs::new(cx.executor());
    let project = Project::test(file_system.clone(), [], cx).await;
    cx.update(|cx| {
        client::Client::set_global(project.read(cx).client(), cx);
        <dyn fs::Fs>::set_global(file_system, cx);
    });
    let (mut doc, master, _, images) = asset_clipboard_subtree();
    let mut other = CanvasNode::new(NodeData::Group(GroupNode {
        clip_size: Some([40.0, 20.0]),
        ..Default::default()
    }));
    other.parent = doc.active_page();
    other.index = doc.scene.next_child_index(other.parent);
    let other_root = other.id;
    doc.scene.insert(other).expect("other variant root");
    let set = fanta_doc::ComponentId::new();
    let member = fanta_doc::ComponentId::new();
    let other_member = fanta_doc::ComponentId::new();
    for (id, root) in [(member, master), (other_member, other_root)] {
        let mut definition = fanta_doc::ComponentDef::new(id, root, "Variant");
        definition.variant_of = Some(fanta_doc::ComponentSetMembership {
            set,
            axis_values: Default::default(),
        });
        doc.components.defs.insert(id, definition);
    }
    doc.components.sets.insert(
        set,
        fanta_doc::ComponentSet {
            id: set,
            name: "Partial selection".into(),
            axes: Vec::new(),
            members: vec![member, other_member],
            default_variant: member,
            root: None,
        },
    );
    doc.selection.select_only(master);
    doc.history = Default::default();
    let payload = CanvasClipboard::capture(&doc).expect("partial copy");
    assert!(
        payload
            .validate_cut()
            .expect_err("partial set rejected")
            .to_string()
            .contains("Select every variant")
    );
    let item = crate::document::ready_item_for_test(
        &project,
        "/tmp/PartialVariantCut.fig".into(),
        doc,
        cx,
    );
    item.update(cx, |item, cx| {
        item.with_document(cx, |document| {
            *Arc::make_mut(&mut document.raw_assets) = images.clone();
            ((), DocChange::None)
        })
    });
    let (multi_workspace, visual) = cx.add_window_view(|window, cx| {
        workspace::MultiWorkspace::test_new(project.clone(), window, cx)
    });
    let workspace = multi_workspace.read_with(visual, |multi_workspace, _| {
        multi_workspace.workspace().clone()
    });
    let view = visual.update(|window, cx| {
        let view = cx.new(|cx| FigView::new(item.clone(), project, window, cx));
        workspace.update(cx, |workspace, cx| {
            workspace.add_item_to_active_pane(Box::new(view.clone()), None, true, window, cx)
        });
        window.activate_window();
        view.update(cx, |view, cx| view.focus_handle.focus(window, cx));
        cx.write_to_clipboard(ClipboardItem::new_string("Kept clipboard".into()));
        view
    });
    visual.run_until_parked();
    visual.update(|window, cx| {
        view.update(cx, |view, cx| view.focus_handle.focus(window, cx));
        window.draw(cx).clear();
    });
    let before = item.read_with(visual, |item, _| {
        asset_clipboard_content(item.doc().expect("mounted document"))
    });
    visual.simulate_keystrokes("cmd-x");
    visual.run_until_parked();
    assert_eq!(
        visual
            .read_from_clipboard()
            .expect("clipboard")
            .text()
            .as_deref(),
        Some("Kept clipboard")
    );
    item.read_with(visual, |item, _| {
        let document = item.document().expect("document");
        assert_eq!(asset_clipboard_content(&document.doc), before);
        assert_eq!(document.doc.selection.as_slice(), &[master]);
        assert!(!document.doc.history.can_undo());
        assert!(!item.is_dirty());
        assert_eq!(document.raw_assets.as_ref(), &images);
    });
    workspace.read_with(visual, |workspace, _| {
        assert!(workspace.notification_ids().contains(&NotificationId::named(CANVAS_NOTICE_ID.into())), "rejected Cut must visibly explain the supported selection without deleting anything");
    });
}

#[gpui::test]
async fn mounted_asset_clipboard_menu_duplicate_and_cut_preserve_bytes_and_subtree(
    cx: &mut TestAppContext,
) {
    init_visual_test(cx);
    cx.update(|cx| {
        cx.bind_keys([
            gpui::KeyBinding::new("home", menu::SelectFirst, Some("menu")),
            gpui::KeyBinding::new("down", menu::SelectNext, Some("menu")),
            gpui::KeyBinding::new("enter", menu::Confirm, Some("menu")),
        ]);
    });
    let project = Project::test(FakeFs::new(cx.executor()), [], cx).await;
    let (doc, root, _, images) = asset_clipboard_subtree();
    let (item, view, mut visual) =
        mounted_canvas_interaction_fixture(&project, doc, images.clone(), cx);
    let before = item.read_with(&visual, |item, _| item.doc().expect("document").clone());
    let invoke_menu = |visual: &mut gpui::VisualTestContext, entry: usize| {
        let position = view.read_with(visual, |view, _| {
            view.container_bounds.expect("canvas bounds").center()
        });
        visual.simulate_mouse_down(position, MouseButton::Right, gpui::Modifiers::none());
        visual.simulate_mouse_up(position, MouseButton::Right, gpui::Modifiers::none());
        visual.run_until_parked();
        assert!(
            view.read_with(visual, |view, _| view.canvas_context_menu.is_some()),
            "actual canvas context menu opens"
        );
        visual.simulate_keystrokes("home");
        for _ in 0..entry {
            visual.simulate_keystrokes("down");
        }
        visual.simulate_keystrokes("enter");
        visual.run_until_parked();
    };
    invoke_menu(&mut visual, 3);
    let (copy, after_duplicate) = item.read_with(&visual, |item, _| {
        let document = item.document().expect("document");
        let doc = &document.doc;
        let copy = *doc
            .selection
            .as_slice()
            .first()
            .expect("selected duplicate");
        assert_ne!(copy, root);
        assert_eq!(doc.history.undo_depth(), 1);
        let originals = before.scene.descendants_of(root).collect::<Vec<_>>();
        let copies = doc.scene.descendants_of(copy).collect::<Vec<_>>();
        assert_eq!(copies.len(), originals.len());
        for (original, copied) in originals.iter().zip(&copies) {
            let original = before.scene.get(*original).expect("original node");
            let copied = doc.scene.get(*copied).expect("copied node");
            assert_ne!(original.id, copied.id);
            assert_eq!(
                original.data, copied.data,
                "Bitmap crop/tint and image-fill asset ID/transform stay exact"
            );
        }
        for id in before
            .scene
            .roots()
            .iter()
            .flat_map(|id| before.scene.descendants_of(*id))
        {
            assert_eq!(
                doc.scene.get(id),
                before.scene.get(id),
                "all original nodes remain exact"
            );
        }
        assert_eq!(document.raw_assets.as_ref(), &images);
        (copy, asset_clipboard_content(doc))
    });
    visual.simulate_keystrokes("cmd-z");
    visual.run_until_parked();
    item.read_with(&visual, |item, _| {
        assert_eq!(
            asset_clipboard_content(item.doc().expect("document")),
            asset_clipboard_content(&before)
        )
    });
    visual.simulate_keystrokes("cmd-shift-z");
    visual.run_until_parked();
    item.update(&mut visual, |item, cx| {
        item.with_document(cx, |document| {
            assert_eq!(asset_clipboard_content(&document.doc), after_duplicate);
            document.doc.selection.select_only(copy);
            ((), DocChange::Selection)
        });
    });
    invoke_menu(&mut visual, 1);
    let cut_payload = visual
        .read_from_clipboard()
        .and_then(|clipboard| {
            clipboard.entries.into_iter().find_map(|entry| match entry {
                ClipboardEntry::String(string) => string.metadata_json::<CanvasClipboard>(),
                ClipboardEntry::Image(_) | ClipboardEntry::ExternalPaths(_) => None,
            })
        })
        .expect("Cut places an actual canvas payload on the clipboard");
    assert_eq!(cut_payload.display_text(), "Asset subtree");
    item.read_with(&visual, |item, _| {
        let document = item.document().expect("document");
        let pasted = cut_payload
            .instantiate(&document.doc, 0.0, ClipboardPlacement::Paste)
            .expect("Cut payload remains usable in the same document");
        let expected = before
            .scene
            .descendants_of(root)
            .map(|id| before.scene.get(id).expect("original subtree").data.clone())
            .collect::<Vec<_>>();
        assert_eq!(
            pasted
                .nodes
                .iter()
                .map(|node| node.data.clone())
                .collect::<Vec<_>>(),
            expected,
            "Cut retains Bitmap/image-fill asset references and exact payloads"
        );
    });
    item.read_with(&visual, |item, _| {
        let document = item.document().expect("document");
        assert_eq!(
            asset_clipboard_content(&document.doc),
            asset_clipboard_content(&before),
            "Cut removes only the copied subtree"
        );
        assert_eq!(document.raw_assets.as_ref(), &images);
        assert_eq!(document.doc.history.undo_depth(), 2);
    });
    visual.simulate_keystrokes("cmd-z");
    visual.run_until_parked();
    item.read_with(&visual, |item, _| {
        assert_eq!(
            asset_clipboard_content(item.doc().expect("document")),
            after_duplicate
        )
    });
    visual.simulate_keystrokes("cmd-shift-z");
    visual.run_until_parked();
    item.read_with(&visual, |item, _| {
        let document = item.document().expect("document");
        assert_eq!(
            asset_clipboard_content(&document.doc),
            asset_clipboard_content(&before)
        );
        assert_eq!(document.raw_assets.as_ref(), &images);
    });
}

#[gpui::test]
async fn canvas_copy_cut_paste_and_duplicate_preserve_subtrees_and_history(
    cx: &mut TestAppContext,
) {
    init_test(cx);
    let project = Project::test(FakeFs::new(cx.executor()), [], cx).await;
    let mut doc = doc_with_one_page();
    let page = doc.active_page().expect("active page");
    let mut frame = CanvasNode::new(NodeData::Group(GroupNode {
        clip_size: Some([160.0, 90.0]),
        ..Default::default()
    }));
    frame.parent = Some(page);
    frame.transform = Transform2D::translation(30.0, 40.0);
    let frame_id = frame.id;
    doc.apply(Operation::create_node(frame)).unwrap();
    let mut text = CanvasNode::new(NodeData::Text(TextNode::new("Clipboard", 100.0, 30.0)));
    text.parent = Some(frame_id);
    text.transform = Transform2D::translation(12.0, 16.0);
    doc.apply(Operation::create_node(text)).unwrap();
    doc.selection.select_only(frame_id);
    doc.history = Default::default();
    let item = crate::document::ready_item_for_test(
        &project,
        std::path::PathBuf::from("/tmp/Clipboard.fig"),
        doc,
        cx,
    );
    let scratch = cx.add_window(|_, _| gpui::Empty);
    let view = scratch
        .update(cx, |_, window, cx| {
            cx.new(|cx| FigView::new(item.clone(), project.clone(), window, cx))
        })
        .expect("create fig view");

    view.update(cx, |view, cx| view.copy_selected_nodes(cx));
    let copied = cx
        .read_from_clipboard()
        .and_then(|clipboard| {
            clipboard.entries.into_iter().find_map(|entry| match entry {
                ClipboardEntry::String(string) => string.metadata_json::<CanvasClipboard>(),
                ClipboardEntry::Image(_) | ClipboardEntry::ExternalPaths(_) => None,
            })
        })
        .expect("canvas clipboard metadata");
    item.read_with(cx, |item, _| {
        assert_eq!(copied.display_text(), "Group");
        assert_eq!(item.doc().unwrap().scene.len(), 3);
    });

    view.update(cx, |view, cx| view.duplicate_selected_nodes(cx));
    let duplicate_id = item.read_with(cx, |item, _| {
        let doc = item.doc().unwrap();
        assert_eq!(doc.scene.len(), 5);
        assert_eq!(doc.history.undo_depth(), 1);
        doc.selection.as_slice()[0]
    });
    assert_ne!(duplicate_id, frame_id);
    item.update(cx, |item, cx| {
        assert!(item.undo(cx).expect("undo duplicate"));
        let doc = item.doc().unwrap();
        assert!(!doc.scene.contains(duplicate_id));
        assert!(
            doc.selection.is_empty(),
            "undo prunes deleted selection ids"
        );
        assert!(item.redo(cx).expect("redo duplicate"));
        assert!(item.doc().unwrap().scene.contains(duplicate_id));
    });

    item.update(cx, |item, cx| {
        item.with_document(cx, |document| {
            document.doc.selection.select_only(frame_id);
            ((), DocChange::Selection)
        });
    });
    view.update(cx, |view, cx| view.cut_selected_nodes(cx));
    item.read_with(cx, |item, _| {
        assert!(!item.doc().unwrap().scene.contains(frame_id));
    });
    view.update(cx, |view, cx| view.paste_selected_nodes(cx));
    item.read_with(cx, |item, _| {
        let doc = item.doc().unwrap();
        assert_eq!(doc.selection.len(), 1);
        let pasted = doc.selection.as_slice()[0];
        assert_ne!(pasted, frame_id);
        assert_eq!(doc.scene.children_of(Some(pasted)).len(), 1);
    });
}

#[gpui::test]
async fn mounted_duplicate_action_preserves_components_while_paste_retains_references(
    cx: &mut TestAppContext,
) {
    use fanta_doc::{
        Action, ComponentDef, ComponentId, ComponentSet, ComponentSetMembership, InstanceNode,
        Reaction, ReactionId, Trigger, VariantAxis,
    };

    init_visual_test(cx);
    cx.update(|cx| {
        cx.bind_keys([
            gpui::KeyBinding::new("cmd-d", DuplicateSelection, Some("FigViewer")),
            gpui::KeyBinding::new("cmd-c", CopySelection, Some("FigViewer")),
            gpui::KeyBinding::new("cmd-v", PasteSelection, Some("FigViewer")),
        ]);
    });
    let project = Project::test(FakeFs::new(cx.executor()), [], cx).await;
    let mut doc = doc_with_one_page();
    let page = doc.active_page().expect("active page");
    let mut section = CanvasNode::new(NodeData::Group(GroupNode {
        clip_size: Some([400.0, 200.0]),
        ..Default::default()
    }));
    section.parent = Some(page);
    section.transform = Transform2D::translation(-200.0, -100.0);
    let section_id = section.id;
    doc.apply(Operation::create_node(section)).expect("section");
    let mut frame = CanvasNode::new(NodeData::Group(GroupNode {
        clip_size: Some([140.0, 60.0]),
        ..Default::default()
    }));
    frame.parent = Some(section_id);
    frame.name = "Variant set frame".into();
    let frame_id = frame.id;
    doc.apply(Operation::create_node(frame)).expect("set frame");
    let set = ComponentId::new();
    let member = ComponentId::new();
    let hover = ComponentId::new();
    let external = ComponentId::new();
    for (component, state, parent, x) in [
        (member, "Default", frame_id, 0.0),
        (hover, "Hover", frame_id, 70.0),
        (external, "External", page, 300.0),
    ] {
        let mut master = CanvasNode::new(NodeData::Group(GroupNode {
            clip_size: Some([50.0, 30.0]),
            ..Default::default()
        }));
        master.parent = Some(parent);
        master.index = doc.scene.next_child_index(Some(parent));
        master.name = format!("State={state}");
        master.transform = Transform2D::translation(x, 0.0);
        let root = master.id;
        doc.apply(Operation::create_node(master)).expect("master");
        let mut definition = ComponentDef::new(component, root, state);
        if component != external {
            definition.variant_of = Some(ComponentSetMembership {
                set,
                axis_values: [("State".into(), state.into())].into_iter().collect(),
            });
        }
        doc.apply(Operation::DefineComponent {
            def: Box::new(definition),
        })
        .expect("component definition");
    }
    doc.apply(Operation::DefineComponentSet {
        set: Box::new(ComponentSet {
            id: set,
            name: "Button".into(),
            axes: vec![VariantAxis {
                name: "State".into(),
                values: vec!["Default".into(), "Hover".into()],
            }],
            members: vec![member, hover],
            default_variant: member,
            root: Some(frame_id),
        }),
    })
    .expect("component set");
    for (name, component, x) in [
        ("Set placement", set, 0.0),
        ("Member placement", member, 70.0),
        ("External placement", external, 140.0),
    ] {
        let mut instance = CanvasNode::new(NodeData::Instance(InstanceNode {
            component,
            overrides: Vec::new(),
            prop_values: Default::default(),
            derived: Vec::new(),
            local_size: [50.0, 30.0],
        }));
        instance.parent = Some(section_id);
        instance.index = doc.scene.next_child_index(Some(section_id));
        instance.name = name.into();
        instance.transform = Transform2D::translation(x, 100.0);
        instance.reactions.push(Reaction {
            id: ReactionId::new(),
            trigger: Trigger::Click,
            action: Action::UpdateVariant {
                component: set,
                variant: "Hover".into(),
            },
            extra_actions: vec![
                Action::UpdateVariant {
                    component: member,
                    variant: "Default".into(),
                },
                Action::UpdateVariant {
                    component: external,
                    variant: "External".into(),
                },
                Action::OpenLink {
                    url: "https://example.com/unchanged".into(),
                },
            ],
            transition: None,
            animation: None,
        });
        doc.apply(Operation::create_node(instance))
            .expect("instance");
    }
    doc.selection.select_only(section_id);
    doc.history = Default::default();
    let content = |doc: &Doc| {
        let mut snapshot = doc.clone();
        snapshot.history = Default::default();
        snapshot.selection.clear();
        // Undo/Redo record the current edit time instead of rewinding it.
        snapshot.metadata.modified_at = 0;
        serde_json::to_value(snapshot).expect("document content")
    };

    for duplicate in [true, false] {
        let (item, view, mut visual) =
            mounted_canvas_interaction_fixture(&project, doc.clone(), BTreeMap::new(), cx);
        visual.update(|window, cx| {
            view.update(cx, |view, cx| view.focus_handle.focus(window, cx));
            window.draw(cx).clear();
        });
        let before = item.read_with(&visual, |item, _| item.doc().expect("document").clone());
        if duplicate {
            visual.simulate_keystrokes("cmd-d");
        } else {
            visual.simulate_keystrokes("cmd-c cmd-v");
        }
        visual.run_until_parked();
        visual.update(|window, cx| window.draw(cx).clear());
        let after = item.read_with(&visual, |item, _| {
            let doc = item.doc().expect("document");
            assert_eq!(doc.history.undo_depth(), 1);
            assert_eq!(doc.selection.len(), 1);
            let copy = *doc.selection.as_slice().first().expect("selected copy");
            assert_ne!(copy, section_id);
            let copied_nodes = doc
                .scene
                .descendants_of(copy)
                .map(|id| doc.scene.get(id).expect("copied node"))
                .collect::<Vec<_>>();
            assert_eq!(
                copied_nodes.len(),
                before.scene.descendants_of(section_id).count()
            );
            assert_eq!(doc.components.defs.len(), if duplicate { 5 } else { 3 });
            assert_eq!(doc.components.sets.len(), if duplicate { 2 } else { 1 });
            let (expected_set, expected_member) = if duplicate {
                let copied_set = doc
                    .components
                    .sets
                    .values()
                    .find(|candidate| candidate.id != set)
                    .expect("Cmd-D keeps copied masters registered as a new set");
                let copied_frame = copied_set.root.expect("copied set frame");
                assert!(copied_nodes.iter().any(|node| node.id == copied_frame));
                for component in &copied_set.members {
                    let definition = doc.components.def(*component).expect("copied definition");
                    assert!(copied_nodes.iter().any(|node| node.id == definition.root));
                    assert_eq!(
                        definition.variant_of.as_ref().expect("membership").set,
                        copied_set.id
                    );
                }
                (copied_set.id, copied_set.default_variant)
            } else {
                assert_eq!(
                    doc.components, before.components,
                    "ordinary Paste retains definitions"
                );
                (set, member)
            };
            for original_id in before.scene.descendants_of(page) {
                assert_eq!(
                    doc.scene.get(original_id),
                    before.scene.get(original_id),
                    "source nodes stay unchanged"
                );
            }
            assert_eq!(
                doc.components.def(external),
                before.components.def(external)
            );
            for node in copied_nodes {
                let NodeData::Instance(instance) = &node.data else {
                    continue;
                };
                let expected_component = match node.name.as_str() {
                    "Set placement" => expected_set,
                    "Member placement" => expected_member,
                    "External placement" => external,
                    name => panic!("unexpected fixture instance: {name}"),
                };
                assert_eq!(instance.component, expected_component, "{}", node.name);
                let original = before
                    .scene
                    .descendants_of(section_id)
                    .filter_map(|id| before.scene.get(id))
                    .find(|original| original.name == node.name)
                    .expect("source instance");
                let mut expected = original.reactions.first().expect("source reaction").clone();
                expected.action = Action::UpdateVariant {
                    component: expected_set,
                    variant: "Hover".into(),
                };
                *expected
                    .extra_actions
                    .first_mut()
                    .expect("secondary variant action") = Action::UpdateVariant {
                    component: expected_member,
                    variant: "Default".into(),
                };
                assert_eq!(
                    node.reactions,
                    vec![expected],
                    "action order and external targets stay intact"
                );
            }
            content(doc)
        });
        visual.simulate_keystrokes("cmd-z");
        visual.run_until_parked();
        item.read_with(&visual, |item, _| {
            let doc = item.doc().expect("document");
            assert_eq!(
                content(doc),
                content(&before),
                "Undo restores exact document content"
            );
            assert_eq!(doc.history.undo_depth(), 0);
            assert!(
                doc.selection.is_empty(),
                "Undo prunes the removed copy from selection"
            );
        });
        visual.simulate_keystrokes("cmd-shift-z");
        visual.run_until_parked();
        item.read_with(&visual, |item, _| {
            let doc = item.doc().expect("document");
            assert_eq!(
                content(doc),
                after,
                "Redo restores the exact copies and definition IDs"
            );
            assert_eq!(doc.history.undo_depth(), 1);
            assert!(doc.selection.is_empty());
        });
    }
}

#[gpui::test]
async fn rejected_cross_document_canvas_paste_shows_notice_and_preserves_document(
    cx: &mut TestAppContext,
) {
    init_visual_test(cx);
    let file_system = FakeFs::new(cx.executor());
    let project = Project::test(file_system.clone(), [], cx).await;
    cx.update(|cx| {
        client::Client::set_global(project.read(cx).client(), cx);
        <dyn fs::Fs>::set_global(file_system, cx);
    });
    let (mut source, source_text, _) = text_selection_doc(false);
    source.selection.select_only(source_text);
    let payload = CanvasClipboard::capture(&source).expect("copy source text");
    cx.update(|cx| {
        cx.write_to_clipboard(ClipboardItem::new_string_with_json_metadata(
            payload.display_text(),
            payload,
        ));
    });

    let (mut destination, destination_text, _) = text_selection_doc(false);
    assert_ne!(source.id, destination.id);
    destination.selection.select_only(destination_text);
    let asset = AssetId::new();
    destination.asset_library.insert(
        asset,
        fanta_doc::ProjectAsset {
            name: "Kept.svg".into(),
            kind: ProjectAssetKind::Svg,
        },
    );
    let item = crate::document::ready_item_for_test(
        &project,
        std::path::PathBuf::from("/tmp/RejectedClipboard.fig"),
        destination,
        cx,
    );
    item.update(cx, |item, cx| {
        item.with_document(cx, |document| {
            Arc::make_mut(&mut document.raw_assets).insert(
                asset,
                br#"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"/>"#.to_vec(),
            );
            ((), DocChange::None)
        });
    });
    let (multi_workspace, cx) = cx.add_window_view(|window, cx| {
        workspace::MultiWorkspace::test_new(project.clone(), window, cx)
    });
    let workspace =
        multi_workspace.read_with(cx, |multi_workspace, _| multi_workspace.workspace().clone());
    let view = cx.update(|window, cx| {
        let view = cx.new(|cx| FigView::new(item.clone(), project, window, cx));
        workspace.update(cx, |workspace, cx| {
            workspace.add_item_to_active_pane(Box::new(view.clone()), None, true, window, cx);
        });
        window.activate_window();
        view
    });
    cx.run_until_parked();
    let snapshot = |item: &FigItem| {
        let document = item.document().expect("destination document");
        (
            serde_json::to_value(&document.doc).expect("serialize destination"),
            document.doc.selection.as_slice().to_vec(),
            document.doc.history.undo_depth(),
            document.doc.history.redo_depth(),
            document.raw_assets.clone(),
            item.is_dirty(),
        )
    };
    let before = item.read_with(cx, |item, _| snapshot(item));
    let notice = NotificationId::named(CANVAS_NOTICE_ID.into());
    workspace.read_with(cx, |workspace, _| {
        assert!(!workspace.notification_ids().contains(&notice));
    });

    view.update(cx, |view, cx| view.paste_selected_nodes(cx));
    cx.run_until_parked();

    workspace.read_with(cx, |workspace, _| {
        assert!(
            workspace.notification_ids().contains(&notice),
            "a rejected paste must reach the visible workspace notification surface"
        );
    });
    item.read_with(cx, |item, _| {
        assert_eq!(
            snapshot(item),
            before,
            "rejection preserves content, selection, history, asset bytes and dirty state"
        );
    });
}

#[gpui::test]
async fn right_click_menu_creates_an_instance_and_goes_back_to_its_main_component(
    cx: &mut TestAppContext,
) {
    init_test(cx);
    let project = Project::test(FakeFs::new(cx.executor()), [], cx).await;
    let mut doc = Doc::new();
    let page = CanvasNode::new(NodeData::Group(GroupNode::default()));
    let page_id = page.id;
    doc.apply(Operation::create_node(page)).expect("page");
    doc.add_page(page_id);
    doc.set_active_page(Some(page_id));
    let mut master = CanvasNode::new(NodeData::Group(GroupNode {
        clip_size: Some([100.0, 40.0]),
        ..Default::default()
    }));
    master.name = "Badge".into();
    master.parent = Some(page_id);
    master.transform = Transform2D::translation(-50.0, -20.0);
    let master_id = master.id;
    doc.apply(Operation::create_node(master)).expect("master");
    let component = fanta_doc::ComponentId::new();
    doc.apply(Operation::DefineComponent {
        def: Box::new(fanta_doc::ComponentDef::new(component, master_id, "Badge")),
    })
    .expect("component");
    let item = crate::document::ready_item_for_test(
        &project,
        std::path::PathBuf::from("/tmp/ContextMenu.fig"),
        doc,
        cx,
    );
    let scratch = cx.add_window(|_, _| gpui::Empty);
    let view = scratch
        .update(cx, |_, window, cx| {
            cx.new(|cx| FigView::new(item.clone(), project.clone(), window, cx))
        })
        .expect("create fig view");
    view.update(cx, |view, _| {
        view.set_container_bounds(Bounds {
            origin: point(px(0.0), px(0.0)),
            size: size(px(800.0), px(600.0)),
        });
        view.set_viewport_silent(Viewport::default());
    });
    let right_click = |position: Point<Pixels>, cx: &mut TestAppContext| {
        scratch
            .update(cx, |_, window, cx| {
                view.update(cx, |view, cx| {
                    view.deploy_canvas_context_menu(
                        &MouseDownEvent {
                            button: MouseButton::Right,
                            position,
                            modifiers: gpui::Modifiers::default(),
                            click_count: 1,
                            first_mouse: false,
                        },
                        window,
                        cx,
                    );
                });
            })
            .expect("right click");
    };

    // A right-click on the main component selects it and opens the menu.
    right_click(point(px(400.0), px(300.0)), cx);
    view.read_with(cx, |view, _| assert!(view.canvas_context_menu.is_some()));
    item.read_with(cx, |item, _| {
        assert_eq!(item.doc().unwrap().selection.as_slice(), &[master_id]);
    });

    // Create instance: placed beside the main component, and selected.
    scratch
        .update(cx, |_, window, cx| {
            view.update(cx, |view, cx| {
                view.create_instance_of(master_id, window, cx)
            });
        })
        .expect("create instance");
    let instance = item.read_with(cx, |item, _| {
        let doc = item.doc().unwrap();
        let instance = doc.selection.as_slice()[0];
        assert!(matches!(
            &doc.scene.get(instance).unwrap().data,
            NodeData::Instance(instance) if instance.component == component
        ));
        assert_eq!(doc.scene.world_bounds(instance).unwrap().min_x, 90.0);
        instance
    });

    // Right-click the instance, then Go to main component.
    right_click(point(px(520.0), px(300.0)), cx);
    item.read_with(cx, |item, _| {
        assert_eq!(item.doc().unwrap().selection.as_slice(), &[instance]);
    });
    scratch
        .update(cx, |_, window, cx| {
            view.update(cx, |view, cx| {
                view.go_to_main_component(instance, window, cx)
            });
        })
        .expect("go to main");
    item.read_with(cx, |item, _| {
        assert_eq!(item.doc().unwrap().selection.as_slice(), &[master_id]);
    });

    // On empty canvas the selection clears.
    right_click(point(px(20.0), px(20.0)), cx);
    item.read_with(cx, |item, _| {
        assert!(item.doc().unwrap().selection.is_empty())
    });
}

#[gpui::test]
async fn standalone_text_selects_on_single_click_and_edits_on_double_click(
    cx: &mut TestAppContext,
) {
    init_test(cx);
    let project = Project::test(FakeFs::new(cx.executor()), [], cx).await;
    let (doc, text_id, _) = text_selection_doc(false);
    let item = crate::document::ready_item_for_test(
        &project,
        std::path::PathBuf::from("/tmp/StandaloneText.fig"),
        doc,
        cx,
    );
    let scratch = cx.add_window(|_, _| gpui::Empty);
    let view = scratch
        .update(cx, |_, window, cx| {
            cx.new(|cx| FigView::new(item.clone(), project.clone(), window, cx))
        })
        .expect("create fig view");
    view.update(cx, |view, _| {
        view.set_container_bounds(Bounds {
            origin: point(px(0.0), px(0.0)),
            size: size(px(800.0), px(600.0)),
        });
        view.set_viewport_silent(Viewport::default());
    });
    let position = point(px(360.0), px(295.0));

    send_canvas_click(scratch, &view, position, 1, cx);
    item.read_with(cx, |item, _| {
        assert_eq!(
            item.doc().expect("document").selection.as_slice(),
            &[text_id]
        );
    });
    view.read_with(cx, |view, _| assert!(view.text_edit.is_none()));

    send_canvas_click(scratch, &view, position, 2, cx);
    view.read_with(cx, |view, _| {
        let edit = view.text_edit.as_ref().expect("text editor opened");
        let range = edit.session.selected_range();
        assert!(range.start < range.end, "double-click selects a word");
        assert!(range.end - range.start < "Hello world".len());
    });
}

#[gpui::test]
async fn wrapped_text_requires_drill_in_before_a_second_double_click_edits(
    cx: &mut TestAppContext,
) {
    init_test(cx);
    let project = Project::test(FakeFs::new(cx.executor()), [], cx).await;
    let (doc, text_id, frame) = text_selection_doc(true);
    let frame = frame.expect("text wrapper");
    let item = crate::document::ready_item_for_test(
        &project,
        std::path::PathBuf::from("/tmp/WrappedText.fig"),
        doc,
        cx,
    );
    let scratch = cx.add_window(|_, _| gpui::Empty);
    let view = scratch
        .update(cx, |_, window, cx| {
            cx.new(|cx| FigView::new(item.clone(), project.clone(), window, cx))
        })
        .expect("create fig view");
    view.update(cx, |view, _| {
        view.set_container_bounds(Bounds {
            origin: point(px(0.0), px(0.0)),
            size: size(px(800.0), px(600.0)),
        });
        view.set_viewport_silent(Viewport::default());
    });
    let position = point(px(340.0), px(285.0));

    send_canvas_click(scratch, &view, position, 1, cx);
    item.read_with(cx, |item, _| {
        assert_eq!(item.doc().expect("document").selection.as_slice(), &[frame]);
    });

    send_canvas_click(scratch, &view, position, 2, cx);
    item.read_with(cx, |item, _| {
        assert_eq!(
            item.doc().expect("document").selection.as_slice(),
            &[text_id]
        );
    });
    view.read_with(cx, |view, _| {
        assert!(
            view.text_edit.is_none(),
            "the first double-click only drills into the wrapper"
        );
    });

    send_canvas_click(scratch, &view, position, 3, cx);
    item.read_with(cx, |item, _| {
        assert_eq!(
            item.doc().expect("document").selection.as_slice(),
            &[text_id]
        );
    });
    view.read_with(cx, |view, _| {
        assert!(
            view.text_edit.is_none(),
            "the first click in the second pair keeps the text selected"
        );
    });

    send_canvas_click(scratch, &view, position, 4, cx);
    view.read_with(cx, |view, _| {
        let edit = view
            .text_edit
            .as_ref()
            .expect("second double-click opens text editing");
        let range = edit.session.selected_range();
        assert!(range.start < range.end, "word selection stays intact");
    });
}

#[gpui::test]
async fn canvas_double_click_routes_each_leaf_kind_without_changing_document(
    cx: &mut TestAppContext,
) {
    init_test(cx);
    let project = Project::test(FakeFs::new(cx.executor()), [], cx).await;
    let asset = AssetId::new();
    let cases = [
        (
            NodeData::Vector(VectorNode::rect_solid(0.0, 0.0, 120.0, 40.0, Color::WHITE)),
            ToolKind::NodeEdit,
        ),
        (
            NodeData::Bitmap(fanta_doc::BitmapNode {
                asset,
                natural_size: [120, 40],
                local_size: [120.0, 40.0],
                crop: None,
                fit: fanta_doc::ImageFitMode::Fill,
                tint: None,
            }),
            ToolKind::Crop,
        ),
        (
            NodeData::Video(fanta_doc::VideoNode {
                asset,
                natural_size: [120, 40],
                local_size: [120.0, 40.0],
                time_range_us: [0, 1_000_000],
                speed: 1.0,
                muted: false,
                volume: 1.0,
                poster_frame_us: None,
                poster: None,
                fit: fanta_doc::ImageFitMode::Fill,
            }),
            ToolKind::Select,
        ),
        (
            NodeData::Audio(fanta_doc::AudioNode {
                asset,
                local_size: [120.0, 40.0],
                time_range_us: [0, 1_000_000],
                volume: 1.0,
                muted: false,
                waveform_color: Color::WHITE,
            }),
            ToolKind::Select,
        ),
        (
            NodeData::NodeGraph(fanta_doc::NodeGraphNode {
                local_size: [120.0, 40.0],
                graph: Default::default(),
                preview: None,
            }),
            ToolKind::Select,
        ),
        (
            NodeData::Model3d(fanta_doc::Model3dNode {
                asset,
                local_size: [120.0, 40.0],
                camera: Default::default(),
                overrides: serde_json::Value::Null,
            }),
            ToolKind::Select,
        ),
        (
            NodeData::AiArtifact(fanta_doc::AiArtifactNode {
                local_size: [120.0, 40.0],
                prompt: "Keep this prompt".into(),
                model: "test".into(),
                params: serde_json::json!({"preserved": true}),
                inputs: Vec::new(),
                lineage_parent: None,
                output: Some(asset),
                status: fanta_doc::GenerationStatus::Done,
                seed: Some(42),
            }),
            ToolKind::Select,
        ),
        (
            NodeData::Embed(fanta_doc::EmbedNode {
                local_size: [120.0, 40.0],
                kind: "test".into(),
                payload: serde_json::json!({"preserved": true}),
            }),
            ToolKind::Select,
        ),
        (
            NodeData::Instance(fanta_doc::InstanceNode {
                component: fanta_doc::ComponentId::new(),
                local_size: [120.0, 40.0],
                overrides: Vec::new(),
                prop_values: BTreeMap::new(),
                derived: Vec::new(),
            }),
            ToolKind::Select,
        ),
    ];
    for (data, expected_tool) in cases {
        let kind = data.kind_tag();
        let mut doc = doc_with_one_page();
        let mut node = CanvasNode::new(data);
        node.parent = doc.active_page();
        node.transform = Transform2D::translation(-60.0, -20.0);
        let node_id = node.id;
        doc.apply(Operation::create_node(node))
            .expect("create test node");
        let history_depth = doc.history.undo_depth();
        let node_count = doc.scene.len();
        let (item, view, scratch) = canvas_interaction_fixture(&project, doc, cx);
        let original = item.read_with(cx, |item, _| {
            item.doc()
                .expect("loaded document")
                .scene
                .get(node_id)
                .expect("loaded node")
                .clone()
        });
        view.update(cx, |view, _| view.inspector_sidebar_visible = false);
        let position = point(px(390.0), px(300.0));
        send_canvas_click(scratch, &view, position, 1, cx);
        item.read_with(cx, |item, _| {
            assert_eq!(
                item.doc().expect("document").selection.as_slice(),
                &[node_id],
                "{kind}"
            );
        });
        send_canvas_click(scratch, &view, position, 2, cx);
        view.read_with(cx, |view, _| {
            assert_eq!(view.active_tool(), expected_tool, "{kind}");
            assert!(view.text_edit.is_none(), "{kind}");
            if expected_tool == ToolKind::Select {
                assert!(
                    view.inspector_sidebar_visible,
                    "{kind} reveals its properties"
                );
            }
        });
        item.read_with(cx, |item, _| {
            let doc = item.doc().expect("document");
            assert_eq!(doc.scene.get(node_id), Some(&original), "{kind}");
            assert_eq!(doc.scene.len(), node_count, "{kind}");
            assert_eq!(doc.history.undo_depth(), history_depth, "{kind}");
            assert!(!item.is_dirty(), "{kind}");
        });
    }
}

#[gpui::test]
async fn canvas_double_click_respects_wrappers_for_vector_and_bitmap(cx: &mut TestAppContext) {
    init_test(cx);
    let project = Project::test(FakeFs::new(cx.executor()), [], cx).await;
    for (data, expected_tool) in [
        (
            NodeData::Vector(VectorNode::rect_solid(0.0, 0.0, 120.0, 40.0, Color::WHITE)),
            ToolKind::NodeEdit,
        ),
        (
            NodeData::Bitmap(fanta_doc::BitmapNode {
                asset: AssetId::new(),
                natural_size: [120, 40],
                local_size: [120.0, 40.0],
                crop: None,
                fit: fanta_doc::ImageFitMode::Fill,
                tint: None,
            }),
            ToolKind::Crop,
        ),
    ] {
        let (mut doc, node_id, frame) = text_selection_doc(true);
        doc.scene.get_mut(node_id).expect("child").data = data;
        let (item, view, scratch) = canvas_interaction_fixture(&project, doc, cx);
        let position = point(px(350.0), px(290.0));
        for (count, selected) in [(1, frame.expect("wrapper")), (2, node_id), (3, node_id)] {
            send_canvas_click(scratch, &view, position, count, cx);
            item.read_with(cx, |item, _| {
                assert_eq!(
                    item.doc().expect("document").selection.as_slice(),
                    &[selected]
                );
            });
            view.read_with(cx, |view, _| {
                assert_eq!(view.active_tool(), ToolKind::Select)
            });
        }
        send_canvas_click(scratch, &view, position, 4, cx);
        view.read_with(cx, |view, _| assert_eq!(view.active_tool(), expected_tool));
    }
}

#[gpui::test]
async fn canvas_double_click_boolean_operand_edits_and_undo_preserves_container(
    cx: &mut TestAppContext,
) {
    init_test(cx);
    let project = Project::test(FakeFs::new(cx.executor()), [], cx).await;
    let (mut doc, operand, container) = text_selection_doc(true);
    let container = container.expect("container");
    doc.scene.get_mut(container).expect("container").data =
        NodeData::Boolean(fanta_doc::BooleanNode::default());
    doc.scene.get_mut(operand).expect("operand").data =
        NodeData::Vector(VectorNode::rect_solid(0.0, 0.0, 120.0, 40.0, Color::WHITE));
    let (item, view, scratch) = canvas_interaction_fixture(&project, doc, cx);
    let (original_operand, original_container, history_depth) =
        item.read_with(cx, |item, _| {
            let doc = item.doc().expect("document");
            (
                doc.scene.get(operand).expect("operand").clone(),
                doc.scene.get(container).expect("container").clone(),
                doc.history.undo_depth(),
            )
        });
    let position = point(px(350.0), px(290.0));
    for count in 1..=4 {
        send_canvas_click(scratch, &view, position, count, cx);
    }
    view.update(cx, |view, cx| {
        assert_eq!(view.active_tool(), ToolKind::NodeEdit);
        view.dispatch_tool_event(
            press_event(
                DVec2::new(320.0, 270.0),
                ToolButton::Primary,
                gpui::Modifiers::default(),
                1,
            ),
            cx,
        );
        view.dispatch_tool_event(
            move_event(DVec2::new(330.0, 280.0), gpui::Modifiers::default()),
            cx,
        );
        view.dispatch_tool_event(
            release_event(
                DVec2::new(330.0, 280.0),
                ToolButton::Primary,
                gpui::Modifiers::default(),
            ),
            cx,
        );
    });
    item.update(cx, |item, cx| {
        let doc = item.doc().expect("document");
        assert_ne!(
            doc.scene.get(operand).expect("operand").data,
            original_operand.data
        );
        assert_eq!(doc.scene.get(container), Some(&original_container));
        assert_eq!(doc.history.undo_depth(), history_depth + 1);
        assert!(item.undo(cx).expect("undo operand edit"));
        assert_eq!(
            item.doc().expect("document").scene.get(operand),
            Some(&original_operand)
        );
        assert_eq!(
            item.doc().expect("document").scene.get(container),
            Some(&original_container)
        );
    });
}

#[gpui::test]
async fn mounted_physical_leaf_entry_respects_rounded_ancestor_clips(cx: &mut TestAppContext) {
    init_visual_test(cx);
    let project = Project::test(FakeFs::new(cx.executor()), [], cx).await;
    for kind in ["text", "vector", "bitmap"] {
        for (clipping, local, enters) in [
            (true, DVec2::new(1., 1.), false),
            (true, DVec2::new(50., 50.), true),
            (false, DVec2::new(1., 1.), true),
        ] {
            let (mut doc, node_id, assets) = bitmap_canvas_doc();
            let data = match kind {
                "text" => NodeData::Text(TextNode::new("Visible text", 100., 100.)),
                "vector" => {
                    NodeData::Vector(VectorNode::rect_solid(0., 0., 100., 100., Color::WHITE))
                }
                _ => {
                    let mut bitmap = doc.scene.get(node_id).expect("bitmap").data.clone();
                    assert!(bitmap.set_local_size(100., 100.));
                    bitmap
                }
            };
            let transform =
                Transform2D::rotation(0.4).then(&Transform2D::translation(-50., -50.));
            let mut frame = CanvasNode::new(NodeData::Group(GroupNode {
                clip_size: Some([100., 100.]),
                corner_radius: Some(50.),
                ..Default::default()
            }));
            frame.parent = doc.active_page();
            frame.transform = transform;
            frame.meta = serde_json::json!({"clip_content": clipping});
            let frame_id = frame.id;
            doc.scene.insert(frame).expect("rounded frame");
            {
                let mut node = doc.scene.remove(node_id).expect("physical leaf");
                node.parent = Some(frame_id);
                node.transform = Transform2D::IDENTITY;
                node.data = data;
                doc.scene.insert(node).expect("reparented physical leaf");
            }
            doc.selection.select_only(node_id);
            doc.history = Default::default();
            let (item, view, mut visual) =
                mounted_canvas_interaction_fixture(&project, doc, assets, cx);
            let snapshot = |item: &FigItem| {
                let document = item.document().expect("ready document");
                let mut doc = document.doc.clone();
                doc.selection.clear();
                (
                    serde_json::to_value(doc).expect("authored document"),
                    document.raw_assets.clone(),
                )
            };
            let original = item.read_with(&visual, |item, _| snapshot(item));
            let position = view.read_with(&visual, |view, _| {
                let bounds = view.container_bounds.expect("mounted canvas");
                let viewport = view.viewport.expect("viewport");
                let (width, height) = bounds_size(bounds);
                let screen = fanta_canvas::world_to_screen(
                    transform.transform_point(local),
                    &viewport,
                    DVec2::new(width, height),
                );
                bounds.origin + point(px(screen.x as f32), px(screen.y as f32))
            });
            visual.simulate_event(MouseDownEvent {
                button: MouseButton::Right,
                position,
                modifiers: Default::default(),
                click_count: 1,
                first_mouse: false,
            });
            visual.simulate_event(MouseUpEvent {
                button: MouseButton::Right,
                position,
                modifiers: Default::default(),
                click_count: 1,
            });
            visual.run_until_parked();
            visual.update(|window, cx| window.draw(cx).clear());
            view.read_with(&visual, |view, _| {
                assert!(view.canvas_context_menu.is_some(), "{kind}: real menu");
                assert!(view.text_edit.is_none());
                assert_eq!(view.active_tool(), ToolKind::Select);
            });
            item.read_with(&visual, |item, _| {
                assert_eq!(
                    item.doc().expect("document").selection.contains(node_id),
                    enters,
                    "{kind}: menu at {local:?}, clipping {clipping}"
                );
                assert_eq!(snapshot(item), original);
                assert_eq!(item.doc().expect("document").history.undo_depth(), 0);
                assert!(!item.is_dirty());
            });
            visual.simulate_keystrokes("escape");
            visual.run_until_parked();
            item.update(&mut visual, |item, cx| {
                item.with_document(cx, |document| {
                    document.doc.selection.select_only(node_id);
                    ((), DocChange::Selection)
                });
            });
            visual.update(|window, cx| window.draw(cx).clear());
            mounted_canvas_click(&mut visual, position, 2);
            view.read_with(&visual, |view, _| {
                assert_eq!(
                    view.text_edit.is_some(),
                    enters && kind == "text",
                    "{kind}: double-click at {local:?}, clipping {clipping}"
                );
                let tool = match (enters, kind) {
                    (true, "vector") => ToolKind::NodeEdit,
                    (true, "bitmap") => ToolKind::Crop,
                    _ => ToolKind::Select,
                };
                assert_eq!(view.active_tool(), tool);
            });
            item.read_with(&visual, |item, _| {
                assert_eq!(snapshot(item), original);
                assert_eq!(item.doc().expect("document").history.undo_depth(), 0);
                assert!(!item.is_dirty());
            });
            visual.update(|window, _| window.remove_window());
        }
    }
}

#[gpui::test]
async fn canvas_double_click_does_not_edit_covered_or_locked_text(cx: &mut TestAppContext) {
    init_test(cx);
    let project = Project::test(FakeFs::new(cx.executor()), [], cx).await;
    for hidden in [false, true] {
        let (mut doc, text_id, frame) = text_selection_doc(true);
        let flag = if hidden {
            fanta_doc::NodeFlags::HIDDEN
        } else {
            fanta_doc::NodeFlags::LOCKED
        };
        doc.scene.get_mut(text_id).expect("text").flags.insert(flag);
        doc.selection.select_only(text_id);
        let (item, view, scratch) = canvas_interaction_fixture(&project, doc, cx);
        send_canvas_click(scratch, &view, point(px(340.0), px(285.0)), 2, cx);
        view.read_with(cx, |view, _| assert!(view.text_edit.is_none()));
        item.read_with(cx, |item, _| {
            assert_eq!(
                item.doc().expect("document").selection.as_slice(),
                &[frame.expect("wrapper")]
            );
            assert!(!item.is_dirty());
        });
    }
    let (mut doc, text_id, _) = text_selection_doc(false);
    let mut cover = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        -60.0,
        -20.0,
        120.0,
        40.0,
        Color::WHITE,
    )));
    cover.parent = doc.active_page();
    cover.index = IndexKey::after(doc.scene.get(text_id).expect("text").index);
    let cover_id = cover.id;
    doc.apply(Operation::create_node(cover))
        .expect("cover text");
    doc.selection.select_only(text_id);
    let (item, view, scratch) = canvas_interaction_fixture(&project, doc, cx);
    send_canvas_click(scratch, &view, point(px(390.0), px(300.0)), 2, cx);
    view.read_with(cx, |view, _| assert!(view.text_edit.is_none()));
    item.read_with(cx, |item, _| {
        assert_eq!(
            item.doc().expect("document").selection.as_slice(),
            &[cover_id]
        )
    });
}

#[gpui::test]
async fn canvas_double_click_with_shift_or_space_does_not_enter_editing(
    cx: &mut TestAppContext,
) {
    init_test(cx);
    let project = Project::test(FakeFs::new(cx.executor()), [], cx).await;
    for pan in [false, true] {
        let (mut doc, text_id, _) = text_selection_doc(false);
        doc.selection.select_only(text_id);
        let (item, view, scratch) = canvas_interaction_fixture(&project, doc, cx);
        view.update(cx, |view, _| view.space_pan = pan);
        let modifiers = gpui::Modifiers {
            shift: !pan,
            ..Default::default()
        };
        send_canvas_click_with_modifiers(
            scratch,
            &view,
            point(px(360.0), px(295.0)),
            2,
            modifiers,
            cx,
        );
        view.read_with(cx, |view, _| assert!(view.text_edit.is_none()));
        item.read_with(cx, |item, _| {
            let doc = item.doc().expect("document");
            if pan {
                assert_eq!(doc.selection.as_slice(), &[text_id]);
            } else {
                assert!(doc.selection.is_empty());
            }
            assert!(!item.is_dirty());
        });
    }
}

#[gpui::test]
async fn canvas_double_click_text_path_edit_preserves_baseline_and_undo(
    cx: &mut TestAppContext,
) {
    init_test(cx);
    let project = Project::test(FakeFs::new(cx.executor()), [], cx).await;
    let mut baseline = fanta_doc::PathData::new();
    baseline.move_to(-60.0, 20.0).line_to(180.0, 20.0);
    let text_path = fanta_doc::TextPathNode::new(baseline, "Hello world");
    let bounds = fanta_render::text_path_visual_bounds(&text_path).expect("visible text path");
    let world = (0..=bounds.height().ceil() as usize)
        .flat_map(|y| {
            (0..=bounds.width().ceil() as usize)
                .map(move |x| [bounds.min_x + x as f64, bounds.min_y + y as f64])
        })
        .find(|point| fanta_render::text_path_contains_point(&text_path, *point))
        .expect("point on a rendered glyph");
    let mut doc = doc_with_one_page();
    let mut node = CanvasNode::new(NodeData::TextPath(text_path));
    node.parent = doc.active_page();
    let original = node.clone();
    let node_id = node.id;
    doc.apply(Operation::create_node(node)).expect("text path");
    let history_depth = doc.history.undo_depth();
    let (item, view, scratch) = canvas_interaction_fixture(&project, doc, cx);
    let position = point(px((400.0 + world[0]) as f32), px((300.0 + world[1]) as f32));
    send_canvas_click(scratch, &view, position, 1, cx);
    send_canvas_click(scratch, &view, position, 2, cx);
    view.read_with(cx, |view, _| assert!(view.text_edit.is_some()));
    scratch
        .update(cx, |_, window, cx| {
            view.update(cx, |view, cx| {
                gpui::EntityInputHandler::replace_text_in_range(
                    view,
                    Some(0..11),
                    "Edited path",
                    window,
                    cx,
                );
                view.commit_text_edit(cx);
            });
        })
        .expect("commit path text input");
    item.update(cx, |item, cx| {
        let doc = item.doc().expect("document");
        let NodeData::TextPath(edited) = &doc.scene.get(node_id).expect("path").data else {
            panic!("text path preserved");
        };
        assert_eq!(edited.content, "Edited path");
        let NodeData::TextPath(original_path) = &original.data else {
            panic!("path fixture")
        };
        assert_eq!(edited.path, original_path.path);
        assert_eq!(edited.start, original_path.start);
        assert_eq!(edited.alignment, original_path.alignment);
        assert_eq!(edited.direction, original_path.direction);
        assert_eq!(edited.side, original_path.side);
        assert_eq!(edited.style, original_path.style);
        assert!(edited.style_runs.iter().all(|run| {
            run.style == original_path.style
                && run.start < run.end
                && run.end <= edited.content.len()
        }));
        assert_eq!(doc.history.undo_depth(), history_depth + 1);
        assert!(item.undo(cx).expect("undo text path edit"));
        assert_eq!(
            item.doc().expect("document").scene.get(node_id),
            Some(&original)
        );
        assert!(item.redo(cx).expect("redo text path edit"));
    });
}

#[cfg(feature = "fanta-gpui-ui")]
#[gpui::test]
async fn mounted_bound_instance_text_double_click_explains_refusal_without_editing(
    cx: &mut TestAppContext,
) {
    init_visual_test(cx);
    bind_canvas_default_macos_keys(cx);
    let project = Project::test(FakeFs::new(cx.executor()), [], cx).await;
    let (mut doc, master_text, master) = text_selection_doc(true);
    let master = master.expect("master frame");
    doc.scene.get_mut(master).expect("master").transform =
        Transform2D::translation(1000.0, 1000.0);
    let component = fanta_doc::ComponentId::new();
    doc.components.defs.insert(
        component,
        fanta_doc::ComponentDef::new(component, master, "Bound card"),
    );
    let collection = fanta_doc::VariableCollectionId::new();
    let mode = fanta_doc::ModeId::new();
    let variable = fanta_doc::VariableId::new();
    doc.variables.collections.insert(
        collection,
        fanta_doc::VariableCollection {
            id: collection,
            name: "Text variables".into(),
            modes: vec![fanta_doc::Mode {
                id: mode,
                name: "Default".into(),
            }],
            default_mode: mode,
            variable_order: vec![variable],
        },
    );
    doc.variables.variables.insert(
        variable,
        fanta_doc::Variable {
            id: variable,
            collection,
            name: "Bound caption".into(),
            ty: fanta_doc::VariableType::String,
            values_by_mode: BTreeMap::from([(
                mode,
                fanta_doc::VarValue::String {
                    value: "Bound label".into(),
                },
            )]),
            scopes: Vec::new(),
            options: Vec::new(),
        },
    );
    doc.scene
        .get_mut(master_text)
        .expect("master text")
        .bindings
        .insert(fanta_doc::BoundProp::TextContent, variable);
    let mut instance = CanvasNode::new(NodeData::Instance(fanta_doc::InstanceNode {
        component,
        overrides: Vec::new(),
        prop_values: BTreeMap::new(),
        derived: Vec::new(),
        local_size: [200.0, 100.0],
    }));
    instance.parent = doc.active_page();
    instance.transform = Transform2D::translation(-100.0, -50.0);
    let instance_id = instance.id;
    doc.scene.insert(instance).expect("instance");
    doc.history = Default::default();
    let item = crate::document::ready_item_for_test(
        &project,
        std::path::PathBuf::from("/tmp/Bound-text.fig"),
        doc,
        cx,
    );
    let (multi_workspace, visual) = cx.add_window_view(|window, cx| {
        workspace::MultiWorkspace::test_new(project.clone(), window, cx)
    });
    let workspace = multi_workspace.read_with(visual, |multi_workspace, _| {
        multi_workspace.workspace().clone()
    });
    let view = visual.update(|window, cx| {
        let view = cx.new(|cx| FigView::new(item.clone(), project, window, cx));
        workspace.update(cx, |workspace, cx| {
            workspace.add_item_to_active_pane(Box::new(view.clone()), None, true, window, cx)
        });
        window.activate_window();
        view
    });
    visual.simulate_resize(size(px(1400.0), px(900.0)));
    view.update(visual, |view, cx| {
        view.set_viewport_silent(Viewport::default());
        cx.notify();
    });
    visual.run_until_parked();
    visual.update(|window, cx| window.draw(cx).clear());
    let before = item.read_with(visual, |item, _| {
        let mut expected = item.doc().expect("document").clone();
        expected.selection.select_only(instance_id);
        serde_json::to_value(expected).expect("selected document before content changes")
    });
    let position = view.read_with(visual, |view, _| {
        view.container_bounds.expect("canvas").center() + point(px(-60.0), px(-15.0))
    });
    for click_count in [1, 2] {
        visual.simulate_event(MouseDownEvent {
            position,
            button: MouseButton::Left,
            modifiers: gpui::Modifiers::none(),
            click_count,
            first_mouse: false,
        });
        visual.simulate_event(MouseUpEvent {
            position,
            button: MouseButton::Left,
            modifiers: gpui::Modifiers::none(),
            click_count,
        });
    }
    visual.run_until_parked();
    view.read_with(visual, |view, _| {
        assert!(
            view.text_edit.is_none(),
            "a variable-controlled clone must not open an editor that cannot change its text"
        )
    });
    workspace.read_with(visual, |workspace, _| {
        assert!(
            workspace
                .notification_ids()
                .contains(&NotificationId::named(CANVAS_NOTICE_ID.into())),
            "double-click refusal must visibly explain the binding"
        )
    });
    item.read_with(visual, |item, _| {
        let doc = item.doc().expect("document");
        assert_eq!(serde_json::to_value(doc).expect("after clicks"), before);
        assert_eq!(doc.selection.as_slice(), &[instance_id]);
        assert!(!doc.history.can_undo());
        assert!(!item.content_preview_active());
    });

    item.update(visual, |item, cx| {
        item.with_document(cx, |document| {
            document
                .doc
                .scene
                .get_mut(master_text)
                .expect("master text")
                .bindings
                .clear();
            ((), DocChange::Content)
        });
    });
    visual.run_until_parked();
    visual.update(|window, cx| window.draw(cx).clear());
    for click_count in [1, 2] {
        visual.simulate_event(MouseDownEvent {
            position,
            button: MouseButton::Left,
            modifiers: gpui::Modifiers::none(),
            click_count,
            first_mouse: false,
        });
        visual.simulate_event(MouseUpEvent {
            position,
            button: MouseButton::Left,
            modifiers: gpui::Modifiers::none(),
            click_count,
        });
    }
    visual.run_until_parked();
    view.read_with(visual, |view, _| {
        assert!(view.text_edit.is_some(), "unbound clone remains editable")
    });
    let original_instance = item.read_with(visual, |item, _| {
        item.doc()
            .expect("document")
            .scene
            .get(instance_id)
            .expect("instance")
            .clone()
    });
    visual.update(|window, cx| {
        view.update(cx, |view, cx| {
            gpui::EntityInputHandler::replace_text_in_range(
                view,
                Some(0..11),
                "Preview first",
                window,
                cx,
            )
        })
    });
    visual.run_until_parked();
    item.update(visual, |item, cx| {
        assert!(item.content_preview_active());
        assert!(
            item.apply(
                Operation::BindProperty {
                    node: master_text,
                    prop: fanta_doc::BoundProp::TextContent,
                    old: None,
                    new: variable
                },
                cx
            )
            .is_err(),
            "ordinary sibling mutations cannot cross the active text preview barrier"
        );
    });
    // Exercise the defensive recheck with an owner-side context change; the
    // normal sibling operation above is correctly refused before this point.
    let owner = view.entity_id();
    let late_field = fanta_doc::Override {
        target_path: vec![master_text].into(),
        target_prop: fanta_doc::BoundProp::Opacity,
        value: fanta_doc::OverrideValue::Field {
            value: serde_json::json!({
                "bindings": [[{"prop": "text_content"}, variable]],
                "name": "Late binding payload"
            }),
        },
    };
    item.update(visual, |item, cx| {
        item.with_document_for_owner(owner, cx, |document| {
            let NodeData::Instance(instance) = &mut document
                .doc
                .scene
                .get_mut(instance_id)
                .expect("instance")
                .data
            else {
                panic!("instance")
            };
            instance.overrides.push(late_field.clone());
            ((), DocChange::None)
        })
        .expect("same preview owner")
    });
    let bound = item.read_with(visual, |item, _| {
        let mut expected = item.doc().expect("document").clone();
        expected.scene.get_mut(instance_id).expect("instance").data =
            original_instance.data.clone();
        let NodeData::Instance(instance) =
            &mut expected.scene.get_mut(instance_id).expect("instance").data
        else {
            panic!("instance")
        };
        instance.overrides.push(late_field.clone());
        (
            serde_json::to_value(expected).expect("binding plus original instance"),
            item.doc().expect("document").history.undo_depth(),
            true,
        )
    });
    visual.update(|window, cx| {
        view.update(cx, |view, cx| {
            gpui::EntityInputHandler::replace_text_in_range(
                view,
                Some(0..13),
                "Keep my draft",
                window,
                cx,
            )
        })
    });
    visual.run_until_parked();
    view.read_with(visual, |view, _| {
        assert_eq!(
            view.text_edit
                .as_ref()
                .expect("draft kept")
                .session
                .buffer(),
            "Keep my draft"
        )
    });
    visual.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.open_text_edit(master_text, TextEditSeed::SelectAll, window, cx);
            let edit = view
                .text_edit
                .as_ref()
                .expect("rejected draft remains open");
            assert_eq!(
                edit.session
                    .instance()
                    .expect("original virtual text")
                    .instance_id,
                instance_id
            );
            assert_eq!(
                edit.session.buffer(),
                "Keep my draft",
                "opening another text target must not replace the blocked draft"
            );
        })
    });
    let before_inspector_action = item.read_with(visual, |item, _| {
        serde_json::to_value(item.doc().expect("document")).expect("before inspector action")
    });
    visual.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.refresh_gpui_design(cx);
            let panel = view
                .gpui_design
                .as_ref()
                .expect("Design adapter")
                .panel
                .clone();
            view.handle_design_action(
                &panel,
                &fanta_gpui::design::DesignPanelAction::PropertyChangeRequested {
                    node_id: instance_id.to_string().into(),
                    property: fanta_gpui::design::DesignPanelProperty::Opacity,
                    value: fanta_gpui::design::DesignPanelValue::Number(40.0),
                },
                window,
                cx,
            );
        })
    });
    item.read_with(visual, |item, _| {
        assert_eq!(
            serde_json::to_value(item.doc().expect("document"))
                .expect("after inspector action"),
            before_inspector_action,
            "a rejected draft must block dependent inspector mutations"
        );
        assert!(
            item.content_preview_active(),
            "a refused inspector edit must not release the retained draft's preview barrier"
        );
    });
    let save = visual.update(|window, cx| {
        view.update(cx, |view, cx| {
            let project = view.project.clone();
            workspace::item::Item::save(
                view,
                workspace::item::SaveOptions::default(),
                project,
                window,
                cx,
            )
        })
    });
    let error = save
        .await
        .expect_err("Save must explain the blocked draft instead of silently ignoring it");
    assert!(error.to_string().contains("Escape"));
    visual.simulate_keystrokes("escape");
    visual.run_until_parked();
    view.read_with(visual, |view, _| {
        assert!(view.text_edit.is_none(), "Escape cancels the blocked draft")
    });
    item.read_with(visual, |item, _| {
        let doc = item.doc().expect("document");
        assert_eq!(
            serde_json::to_value(doc).expect("after cancellation"),
            bound.0
        );
        assert_eq!(doc.history.undo_depth(), bound.1);
        assert_eq!(item.is_dirty(), bound.2);
        assert!(
            !item.content_preview_active(),
            "cancel must release the item for editing and saving"
        );
    });
    item.update(visual, |item, cx| {
        let name = item
            .doc()
            .expect("document")
            .scene
            .get(instance_id)
            .expect("instance")
            .name
            .clone();
        item.apply(
            Operation::SetName {
                id: instance_id,
                old: name,
                new: "Editing resumed".into(),
            },
            cx,
        )
        .expect("ordinary edits resume after Escape");
    });
}

#[gpui::test]
async fn canvas_double_click_instance_text_commits_override_without_changing_master(
    cx: &mut TestAppContext,
) {
    init_test(cx);
    let project = Project::test(FakeFs::new(cx.executor()), [], cx).await;
    let (mut doc, master_text, master) = text_selection_doc(true);
    let master = master.expect("master frame");
    doc.scene.get_mut(master).expect("master").transform =
        Transform2D::translation(1000.0, 1000.0);
    let original_master_text = doc.scene.get(master_text).expect("master text").clone();
    let component = fanta_doc::ComponentId::new();
    doc.components.defs.insert(
        component,
        fanta_doc::ComponentDef::new(component, master, "Card"),
    );
    let mut instance = CanvasNode::new(NodeData::Instance(fanta_doc::InstanceNode {
        component,
        overrides: Vec::new(),
        prop_values: BTreeMap::new(),
        derived: Vec::new(),
        local_size: [200.0, 100.0],
    }));
    instance.parent = doc.active_page();
    instance.transform = Transform2D::translation(-100.0, -50.0);
    let instance_id = instance.id;
    let original_instance = instance.clone();
    doc.apply(Operation::create_node(instance))
        .expect("instance");
    let history_depth = doc.history.undo_depth();
    let (item, view, scratch) = canvas_interaction_fixture(&project, doc, cx);
    let position = point(px(340.0), px(285.0));
    send_canvas_click(scratch, &view, position, 1, cx);
    send_canvas_click(scratch, &view, position, 2, cx);
    view.read_with(cx, |view, _| {
        assert!(
            view.text_edit
                .as_ref()
                .expect("inline instance text")
                .session
                .instance()
                .is_some()
        );
    });
    scratch
        .update(cx, |_, window, cx| {
            view.update(cx, |view, cx| {
                gpui::EntityInputHandler::replace_text_in_range(
                    view,
                    Some(0..11),
                    "Instance only",
                    window,
                    cx,
                );
                view.commit_text_edit(cx);
            });
        })
        .expect("commit instance text input");
    item.update(cx, |item, cx| {
        let doc = item.doc().expect("document");
        assert_eq!(doc.scene.get(master_text), Some(&original_master_text));
        let NodeData::Instance(instance) = &doc.scene.get(instance_id).expect("instance").data else {
            panic!("instance preserved");
        };
        assert!(instance.overrides.iter().any(|entry| {
            matches!(&entry.value, fanta_doc::OverrideValue::Text { value } if value == "Instance only")
        }));
        assert_eq!(doc.history.undo_depth(), history_depth + 1);
        assert!(item.undo(cx).expect("undo instance override"));
        assert_eq!(item.doc().expect("document").scene.get(instance_id), Some(&original_instance));
        assert!(item.redo(cx).expect("redo instance override"));
        assert_eq!(item.doc().expect("document").scene.get(master_text), Some(&original_master_text));
    });
}

#[gpui::test]
async fn empty_canvas_pointer_click_clears_instance_before_or_after_repaint(
    cx: &mut TestAppContext,
) {
    use gpui::InputEvent as _;

    init_visual_test(cx);
    cx.update(|cx| {
        cx.bind_keys([gpui::KeyBinding::new("escape", Cancel, Some("FigViewer"))]);
    });
    let project = Project::test(FakeFs::new(cx.executor()), [], cx).await;
    let (mut doc, _, master) = text_selection_doc(true);
    let master = master.expect("master frame");
    doc.scene.get_mut(master).expect("master").transform =
        Transform2D::translation(1000.0, 1000.0);
    let component = fanta_doc::ComponentId::new();
    doc.components.defs.insert(
        component,
        fanta_doc::ComponentDef::new(component, master, "Card"),
    );
    let mut instance = CanvasNode::new(NodeData::Instance(fanta_doc::InstanceNode {
        component,
        overrides: Vec::new(),
        prop_values: BTreeMap::new(),
        derived: Vec::new(),
        local_size: [200.0, 100.0],
    }));
    instance.parent = doc.active_page();
    instance.transform = Transform2D::translation(-100.0, -50.0);
    let instance_id = instance.id;
    doc.apply(Operation::create_node(instance))
        .expect("instance");
    let mut vector = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        0.0,
        0.0,
        80.0,
        100.0,
        Color::WHITE,
    )));
    vector.parent = doc.active_page();
    vector.transform = Transform2D::translation(-250.0, -50.0);
    doc.apply(Operation::create_node(vector)).expect("vector");
    doc.history = Default::default();
    let item = crate::document::ready_item_for_test(
        &project,
        std::path::PathBuf::from("/tmp/Instance-deselection.fig"),
        doc,
        cx,
    );
    let (view, visual) = cx.add_window_view({
        let item = item.clone();
        move |window, cx| FigView::new(item, project, window, cx)
    });
    visual.simulate_resize(size(px(1400.0), px(900.0)));
    view.update(visual, |view, cx| {
        view.set_viewport_silent(Viewport::default());
        cx.notify();
    });
    visual.run_until_parked();
    visual.update(|window, cx| window.draw(cx).clear());

    for after_node_edit in [false, true] {
        if after_node_edit {
            let bounds = view.read_with(visual, |view, _| {
                view.container_bounds.expect("canvas bounds")
            });
            let position = bounds.center() + point(px(-210.0), px(0.0));
            for click_count in [1, 2] {
                visual.simulate_event(MouseDownEvent {
                    position,
                    button: MouseButton::Left,
                    modifiers: gpui::Modifiers::none(),
                    click_count,
                    first_mouse: false,
                });
                visual.simulate_event(MouseUpEvent {
                    position,
                    button: MouseButton::Left,
                    modifiers: gpui::Modifiers::none(),
                    click_count,
                });
            }
            assert_eq!(
                view.read_with(visual, |view, _| view.active_tool()),
                ToolKind::NodeEdit
            );
            visual.simulate_keystrokes("escape");
            assert_eq!(
                view.read_with(visual, |view, _| view.active_tool()),
                ToolKind::Select
            );
        }
        for paint_between_events in [false, true] {
            let bounds = view.read_with(visual, |view, _| {
                view.container_bounds.expect("canvas bounds")
            });
            visual.simulate_click(bounds.center(), gpui::Modifiers::none());
            item.read_with(visual, |item, _| {
                assert_eq!(
                    item.doc().expect("document").selection.as_slice(),
                    &[instance_id]
                );
            });
            let position = point(bounds.right() - px(40.0), bounds.top() + px(160.0));
            // Deliver the complete click before pumping the executor so
            // an event listener installed only after repaint cannot hide.
            visual.update(|window, cx| {
                window.dispatch_event(
                    MouseDownEvent {
                        position,
                        button: MouseButton::Left,
                        modifiers: gpui::Modifiers::none(),
                        click_count: 1,
                        first_mouse: false,
                    }
                    .to_platform_input(),
                    cx,
                );
                assert!(view.read(cx).primary_pressed());
                if paint_between_events {
                    window.draw(cx).clear();
                    assert!(view.read(cx).primary_pressed());
                }
                window.dispatch_event(
                    MouseUpEvent {
                        position,
                        button: MouseButton::Left,
                        modifiers: gpui::Modifiers::none(),
                        click_count: 1,
                    }
                    .to_platform_input(),
                    cx,
                );
            });
            visual.run_until_parked();
            view.read_with(visual, |view, _| {
                assert!(!view.primary_pressed);
                assert!(!view.canvas_pointer_down);
                assert_eq!(view.active_tool(), ToolKind::Select);
            });
            item.read_with(visual, |item, _| {
                let doc = item.doc().expect("document");
                assert!(doc.selection.is_empty(), "after_node_edit={after_node_edit}, paint_between_events={paint_between_events}");
                assert_eq!(doc.history.undo_depth(), 0);
                assert!(!item.is_dirty());
            });
        }
    }
}

#[gpui::test]
async fn mounted_curved_text_path_counters_and_whitespace_edit_after_crop(
    cx: &mut TestAppContext,
) {
    init_visual_test(cx);
    let project = Project::test(FakeFs::new(cx.executor()), [], cx).await;
    let (mut doc, _, images) = bitmap_canvas_doc();
    let (text_path, ink, counter, whitespace) = curved_text_path_hit_fixture();
    let mut node = CanvasNode::new(NodeData::TextPath(text_path));
    node.parent = doc.active_page();
    node.index = doc.scene.next_child_index(node.parent);
    node.transform = Transform2D::translation(-185.0, 100.0);
    let node_id = node.id;
    let original = node.clone();
    doc.apply(Operation::create_node(node))
        .expect("curved text path");
    doc.history = Default::default();
    let (item, view, mut visual) =
        mounted_canvas_interaction_fixture(&project, doc, images, cx);
    for zoom in [1.0, 0.65] {
        view.update(&mut visual, |view, cx| {
            view.set_viewport_silent(Viewport {
                center: [0.0, 0.0],
                zoom,
            });
            cx.notify();
        });
        visual.update(|window, cx| window.draw(cx).clear());
        let canvas = view.read_with(&visual, |view, _| view.container_bounds.expect("canvas"));
        mounted_canvas_click(&mut visual, canvas.center(), 1);
        mounted_canvas_click(&mut visual, canvas.center(), 2);
        assert_eq!(
            view.read_with(&visual, |view, _| view.active_tool()),
            ToolKind::Crop
        );
        visual.simulate_keystrokes("escape");
        assert_eq!(
            view.read_with(&visual, |view, _| view.active_tool()),
            ToolKind::Select
        );
        let screen_point = |local: [f64; 2]| {
            canvas.center()
                + point(
                    px(((local[0] - 185.0) * zoom) as f32),
                    px(((local[1] + 100.0) * zoom) as f32),
                )
        };
        let ink_position = screen_point(ink);
        mounted_canvas_click(&mut visual, ink_position, 1);
        item.read_with(&visual, |item, _| {
            assert_eq!(
                item.doc().expect("document").selection.as_slice(),
                &[node_id]
            );
        });
        mounted_canvas_click(&mut visual, ink_position, 2);
        view.read_with(&visual, |view, _| {
            let edit = view
                .text_edit
                .as_ref()
                .expect("visible curved glyph opens editor");
            assert_eq!(edit.session.node_id(), node_id);
            assert_eq!(edit.session.selected_range(), 5..9);
        });
        visual.simulate_keystrokes("escape");
        for local in [whitespace, counter] {
            let position = screen_point(local);
            view.read_with(&visual, |view, cx| {
                let doc = item.read(cx).doc().expect("document");
                let (width, height) = bounds_size(canvas);
                assert_eq!(
                    inspect_hit_test_screen(
                        &doc.scene,
                        &view.viewport.expect("viewport"),
                        DVec2::new(width, height),
                        screen_position_in_bounds(position, canvas),
                        doc.active_page(),
                    ),
                    None,
                    "inspection retains exact-ink geometry"
                );
            });
            mounted_canvas_click(&mut visual, position, 1);
            item.read_with(&visual, |item, _| {
                assert_eq!(
                    item.doc().expect("document").selection.as_slice(),
                    &[node_id]
                );
            });
            mounted_canvas_click(&mut visual, position, 2);
            view.read_with(&visual, |view, _| {
                let edit = view
                    .text_edit
                    .as_ref()
                    .expect("shaped text footprint opens editor");
                assert_eq!(edit.session.node_id(), node_id);
                if local == counter {
                    assert_eq!(edit.session.selected_range(), 5..9);
                }
            });
            if local == counter {
                visual.update(|window, cx| window.draw(cx).clear());
                visual.simulate_input("CHANGED");
            }
            visual.simulate_keystrokes("escape");
        }
        item.read_with(&visual, |item, _| {
            let doc = item.doc().expect("document");
            let NodeData::TextPath(edited) = &doc.scene.get(node_id).expect("text path").data
            else {
                panic!("text path kind preserved");
            };
            let NodeData::TextPath(original_path) = &original.data else {
                panic!("text path fixture");
            };
            assert_eq!(edited.content, "PATH CHANGED ME");
            assert_eq!(edited.path, original_path.path);
            assert_eq!(doc.history.undo_depth(), 1);
        });
        visual.simulate_keystrokes("cmd-z");
        item.read_with(&visual, |item, _| {
            let doc = item.doc().expect("document");
            assert_eq!(doc.scene.get(node_id), Some(&original));
            assert_eq!(doc.history.undo_depth(), 0);
        });
    }
}

#[gpui::test]
async fn mounted_curved_text_path_authoring_respects_occlusion_locks_and_clips(
    cx: &mut TestAppContext,
) {
    init_visual_test(cx);
    let project = Project::test(FakeFs::new(cx.executor()), [], cx).await;
    for guard in [
        "distant baseline gap",
        "locked",
        "covered",
        "clipped",
        "rounded clip",
    ] {
        let (text_path, _, counter, _) = curved_text_path_hit_fixture();
        let local = if guard == "distant baseline gap" {
            let bounds = NodeData::TextPath(text_path.clone())
                .local_bounds()
                .expect("broad bounds");
            [bounds.max_x - 1.0, bounds.max_y - 1.0]
        } else {
            counter
        };
        let mut doc = doc_with_one_page();
        let page = doc.active_page();
        let mut node = CanvasNode::new(NodeData::TextPath(text_path));
        node.parent = page;
        node.transform = Transform2D::translation(-local[0], -local[1]);
        if guard == "locked" {
            node.flags.insert(fanta_doc::NodeFlags::LOCKED);
        }
        if matches!(guard, "clipped" | "rounded clip") {
            let inside = if guard == "rounded clip" {
                [5.0, 5.0]
            } else {
                [210.0, 30.0]
            };
            let mut parent = CanvasNode::new(NodeData::Group(GroupNode {
                clip_size: Some([200.0, 120.0]),
                corner_radius: (guard == "rounded clip").then_some(60.0),
                ..Default::default()
            }));
            parent.parent = page;
            parent.transform = Transform2D::translation(-inside[0], -inside[1]);
            node.parent = Some(parent.id);
            node.transform =
                Transform2D::translation(inside[0] - local[0], inside[1] - local[1]);
            doc.apply(Operation::create_node(parent))
                .expect("clipping group");
        }
        let node_id = node.id;
        doc.apply(Operation::create_node(node))
            .expect("curved text");
        if guard == "covered" {
            let mut cover = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
                -12.0,
                -12.0,
                24.0,
                24.0,
                Color::BLACK,
            )));
            cover.parent = page;
            cover.index = doc.scene.next_child_index(page);
            doc.apply(Operation::create_node(cover))
                .expect("occluding vector");
        }
        doc.history = Default::default();
        let (item, view, mut visual) =
            mounted_canvas_interaction_fixture(&project, doc, BTreeMap::new(), cx);
        let position = view.read_with(&visual, |view, _| {
            view.container_bounds.expect("canvas").center()
        });
        for click_count in 1..=4 {
            mounted_canvas_click(&mut visual, position, click_count);
            view.read_with(&visual, |view, _| {
                assert!(view.text_edit.is_none(), "{guard}: click {click_count}");
            });
            item.read_with(&visual, |item, _| {
                let doc = item.doc().expect("document");
                assert!(
                    !doc.selection.contains(node_id),
                    "{guard}: click {click_count}"
                );
                assert_eq!(doc.history.undo_depth(), 0, "{guard}");
                assert!(!item.is_dirty(), "{guard}");
            });
        }
    }
}

#[test]
fn click_diagnostics_require_explicit_setting_and_bound_payload_work() {
    use std::ffi::OsStr;
    for value in [
        None,
        Some(OsStr::new("")),
        Some(OsStr::new("0")),
        Some(OsStr::new("true")),
    ] {
        assert!(ClickDiagnostics::from_setting(value).is_none());
    }
    assert!(ClickDiagnostics::capture::<()>(None, || panic!("disabled payload")).is_none());
    let mut diagnostics =
        ClickDiagnostics::from_setting(Some(OsStr::new("1"))).expect("enabled");
    for sequence in 1..=CLICK_DIAGNOSTIC_LIMIT {
        assert_eq!(
            ClickDiagnostics::capture(Some(&mut diagnostics), || sequence),
            Some((sequence, sequence))
        );
    }
    assert!(
        ClickDiagnostics::capture::<()>(Some(&mut diagnostics), || panic!("exhausted payload"))
            .is_none()
    );
    assert_eq!(diagnostics.reserved, CLICK_DIAGNOSTIC_LIMIT);
}

#[gpui::test]
async fn mounted_click_diagnostics_distinguish_cached_flags_from_press_and_release(
    cx: &mut TestAppContext,
) {
    use gpui::InputEvent as _;
    init_visual_test(cx);
    let project = Project::test(FakeFs::new(cx.executor()), [], cx).await;
    let (mut doc, first, images) = bitmap_canvas_doc();
    let mut second_node = CanvasNode::new(NodeData::Vector(fanta_doc::VectorNode::rect_solid(
        0.0,
        0.0,
        80.0,
        80.0,
        Color::WHITE,
    )));
    second_node.parent = doc.active_page();
    second_node.transform = Transform2D::translation(180.0, -40.0);
    let second = second_node.id;
    doc.scene.insert(second_node).expect("second target");
    doc.selection.select_only(first);
    let (item, view, mut visual) =
        mounted_canvas_interaction_fixture(&project, doc, images, cx);
    let (before, assets) = item.read_with(&visual, |item, _| {
        let document = item.document().expect("document");
        (
            serde_json::to_value(&document.doc.scene).expect("scene"),
            document.raw_assets.clone(),
        )
    });
    let none = gpui::Modifiers::none();
    let shift = gpui::Modifiers {
        shift: true,
        ..none
    };
    let command = gpui::Modifiers {
        platform: true,
        ..none
    };
    for (cached, down, up, extends) in [
        (shift, none, none, false),
        (command, none, none, false),
        (none, shift, none, true),
        (none, none, shift, true),
        (none, command, none, true),
        (none, none, command, true),
    ] {
        item.update(&mut visual, |item, cx| {
            item.with_document(cx, |document| {
                document.doc.selection.select_only(first);
                ((), DocChange::Selection)
            });
        });
        view.update(&mut visual, |view, _| {
            view.click_diagnostics =
                ClickDiagnostics::from_setting(Some(std::ffi::OsStr::new("1")));
        });
        visual.run_until_parked();
        let position = view.read_with(&visual, |view, _| {
            view.container_bounds.expect("canvas").center() + point(px(220.0), px(0.0))
        });
        visual.update(|window, cx| {
            window.dispatch_event(
                gpui::ModifiersChangedEvent {
                    modifiers: cached,
                    ..Default::default()
                }
                .to_platform_input(),
                cx,
            );
            assert_eq!(window.modifiers(), cached);
            window.dispatch_event(
                MouseDownEvent {
                    position,
                    button: MouseButton::Left,
                    modifiers: down,
                    click_count: 1,
                    first_mouse: false,
                }
                .to_platform_input(),
                cx,
            );
            assert_eq!(window.modifiers(), down);
            for _ in 0..2 {
                window.dispatch_event(
                    MouseUpEvent {
                        position,
                        button: MouseButton::Left,
                        modifiers: up,
                        click_count: 1,
                    }
                    .to_platform_input(),
                    cx,
                );
            }
        });
        visual.run_until_parked();
        item.read_with(&visual, |item, _| {
            let document = item.document().expect("document");
            assert!(document.doc.selection.contains(second));
            assert_eq!(document.doc.selection.contains(first), extends);
            assert_eq!(document.doc.selection.len(), if extends { 2 } else { 1 });
            assert_eq!(
                serde_json::to_value(&document.doc.scene).expect("scene"),
                before
            );
            assert_eq!(document.raw_assets, assets);
            assert_eq!(document.doc.history.undo_depth(), 0);
            assert!(!item.is_dirty());
        });
        view.read_with(&visual, |view, _| {
            let records = &view
                .click_diagnostics
                .as_ref()
                .expect("diagnostics")
                .records;
            let down_record = records
                .iter()
                .find(|record| record["stage"] == "canvas_down")
                .expect("down");
            assert_eq!(
                down_record["input"]["modifiers"],
                click_diagnostic_modifiers(down)
            );
            assert_eq!(
                down_record["input"]["window_modifiers"],
                click_diagnostic_modifiers(down)
            );
            assert_eq!(down_record["route"], "tool_press");
            assert_eq!(
                records
                    .iter()
                    .filter(|record| record["route"] == "tool_release")
                    .count(),
                1
            );
            assert!(
                records
                    .iter()
                    .any(|record| record["route"] == "primary_not_pressed")
            );
            let tool_records: Vec<_> = records
                .iter()
                .filter(|record| record["stage"] == "tool_dispatch")
                .collect();
            assert_eq!(tool_records.len(), 2, "one press and one accepted release");
            let release = tool_records
                .iter()
                .find(|record| record["input"]["pointer"]["kind"] == "release")
                .expect("tool release");
            assert_eq!(
                release["input"]["pointer"]["modifiers"],
                serde_json::to_value(crate::tools::modifier_keys(up)).expect("modifiers")
            );
            assert_eq!(release["before"]["selection"], serde_json::json!([first]));
            assert_eq!(
                release["after"]["selection_count"],
                if extends { 2 } else { 1 }
            );
            assert!(!view.primary_pressed && !view.canvas_pointer_down);
        });
        let records_before = view.read_with(&visual, |view, _| {
            view.click_diagnostics
                .as_ref()
                .expect("diagnostics")
                .records
                .len()
        });
        visual.simulate_event(MouseMoveEvent {
            position,
            pressed_button: None,
            modifiers: none,
        });
        view.update(&mut visual, |view, cx| {
            view.dispatch_tool_event(key_event(LogicalKey::Escape, none), cx)
        });
        assert_eq!(
            view.read_with(&visual, |view, _| view
                .click_diagnostics
                .as_ref()
                .expect("diagnostics")
                .records
                .len()),
            records_before,
            "moves and keys are not logged"
        );
    }
}

#[gpui::test]
async fn mounted_click_diagnostics_keep_pan_routes_and_selection_payload_bounded(
    cx: &mut TestAppContext,
) {
    init_visual_test(cx);
    let project = Project::test(FakeFs::new(cx.executor()), [], cx).await;
    let (mut doc, first, images) = bitmap_canvas_doc();
    doc.selection.select_only(first);
    for index in 0..20 {
        let mut node = CanvasNode::new(NodeData::Group(GroupNode::default()));
        node.parent = doc.active_page();
        node.transform = Transform2D::translation(2000.0 + f64::from(index), 0.0);
        let id = node.id;
        doc.scene.insert(node).expect("extra selected node");
        doc.selection.add(id);
    }
    let (item, view, mut visual) =
        mounted_canvas_interaction_fixture(&project, doc, images, cx);
    use gpui::InputEvent as _;
    let before = item.read_with(&visual, |item, _| {
        serde_json::to_value(&item.doc().expect("doc").scene).expect("scene")
    });
    let position = view.read_with(&visual, |view, _| {
        view.container_bounds.expect("canvas").center()
    });
    for paint_between_events in [false, true] {
        view.update(&mut visual, |view, _| {
            view.click_diagnostics =
                ClickDiagnostics::from_setting(Some(std::ffi::OsStr::new("1")));
            view.space_pan = true;
        });
        visual.update(|window, cx| {
            window.dispatch_event(
                MouseDownEvent {
                    position,
                    button: MouseButton::Left,
                    modifiers: gpui::Modifiers::none(),
                    click_count: 1,
                    first_mouse: false,
                }
                .to_platform_input(),
                cx,
            );
            assert!(view.read(cx).is_panning());
            if paint_between_events {
                window.draw(cx).clear();
            }
            window.dispatch_event(
                MouseUpEvent {
                    position,
                    button: MouseButton::Left,
                    modifiers: gpui::Modifiers::none(),
                    click_count: 1,
                }
                .to_platform_input(),
                cx,
            );
        });
        visual.run_until_parked();
        view.read_with(&visual, |view, _| {
            let records = &view
                .click_diagnostics
                .as_ref()
                .expect("diagnostics")
                .records;
            assert!(records.iter().any(|record| record["route"] == "pan"));
            let expected_route = if paint_between_events {
                "shared_release"
            } else {
                "space_pan"
            };
            assert!(
                records.iter().any(|record| record["stage"] == "canvas_up"
                    && record["route"] == expected_route)
            );
            assert!(
                !records
                    .iter()
                    .any(|record| record["stage"] == "tool_dispatch")
            );
            assert!(!view.is_panning());
            assert!(view.space_pan);
            assert!(!view.primary_pressed && !view.canvas_pointer_down);
            for record in records {
                assert_eq!(record["before"]["selection_count"], 21);
                assert_eq!(
                    record["before"]["selection"]
                        .as_array()
                        .expect("selection")
                        .len(),
                    CLICK_DIAGNOSTIC_SELECTION_LIMIT
                );
                assert_eq!(record["before"]["selection_truncated"], true);
            }
        });
    }
    item.read_with(&visual, |item, _| {
        let doc = item.doc().expect("doc");
        assert_eq!(serde_json::to_value(&doc.scene).expect("scene"), before);
        assert_eq!(doc.selection.len(), 21);
        assert_eq!(doc.history.undo_depth(), 0);
        assert!(!item.is_dirty());
    });
}

#[gpui::test]
async fn mounted_lost_primary_release_cancels_pending_press_and_drag(cx: &mut TestAppContext) {
    use gpui::InputEvent as _;

    init_visual_test(cx);
    let project = Project::test(FakeFs::new(cx.executor()), [], cx).await;
    let (mut doc, bitmap, images) = bitmap_canvas_doc();
    let mut control = doc.scene.get(bitmap).expect("bitmap").clone();
    control.id = NodeId::new();
    control.index = doc.scene.next_child_index(control.parent);
    control.transform = Transform2D::translation(300.0, 200.0);
    doc.scene.insert(control).expect("untouched bitmap");
    doc.selection.select_only(bitmap);
    let (item, view, mut visual) =
        mounted_canvas_interaction_fixture(&project, doc, images, cx);
    let (expected, assets, transform, world_bounds, order) =
        item.read_with(&visual, |item, _| {
            let document = item.document().expect("document");
            (
                serde_json::to_value(&document.doc).expect("full document"),
                document.raw_assets.clone(),
                document.doc.scene.get(bitmap).expect("bitmap").transform,
                document.doc.scene.world_bounds(bitmap),
                document
                    .doc
                    .scene
                    .children_of(document.doc.active_page())
                    .to_vec(),
            )
        });
    let bounds = view.read_with(&visual, |view, _| view.container_bounds.expect("canvas"));
    let start = bounds.center();
    for pressed_button in [None, Some(MouseButton::Right)] {
        for preview_before_loss in [false, true] {
            for repaint_after_press in [false, true] {
                visual.update(|window, cx| {
                    window.dispatch_event(
                        MouseDownEvent {
                            position: start,
                            button: MouseButton::Left,
                            modifiers: gpui::Modifiers::none(),
                            click_count: 1,
                            first_mouse: false,
                        }
                        .to_platform_input(),
                        cx,
                    );
                    assert!(view.read(cx).primary_pressed);
                    assert!(view.read(cx).canvas_pointer_down);
                    if repaint_after_press {
                        window.draw(cx).clear();
                    }
                    if preview_before_loss {
                        window.dispatch_event(
                            MouseMoveEvent {
                                position: start + point(px(24.0), px(12.0)),
                                pressed_button: Some(MouseButton::Left),
                                modifiers: gpui::Modifiers::none(),
                            }
                            .to_platform_input(),
                            cx,
                        );
                        let item = item.read(cx);
                        assert!(item.content_preview_active());
                        assert_ne!(
                            item.doc()
                                .expect("document")
                                .scene
                                .get(bitmap)
                                .expect("bitmap")
                                .transform,
                            transform
                        );
                        assert_eq!(item.doc().expect("document").history.undo_depth(), 0);
                    }
                    window.dispatch_event(
                        MouseMoveEvent {
                            position: bounds.origin - point(px(20.0), px(20.0)),
                            pressed_button,
                            modifiers: gpui::Modifiers::none(),
                        }
                        .to_platform_input(),
                        cx,
                    );
                });
                visual.run_until_parked();
                item.read_with(&visual, |item, _| {
                    let document = item.document().expect("document");
                    assert_eq!(
                        serde_json::to_value(&document.doc).expect("full document"),
                        expected,
                        "lost release must not promote or preserve a content preview"
                    );
                    assert_eq!(document.raw_assets, assets);
                    assert_eq!(document.doc.scene.world_bounds(bitmap), world_bounds);
                    assert_eq!(document.doc.history.undo_depth(), 0);
                    assert_eq!(document.doc.history.redo_depth(), 0);
                    assert_eq!(
                        document.doc.scene.children_of(document.doc.active_page()),
                        order
                    );
                    assert!(!item.content_preview_active());
                    assert!(!item.is_dirty());
                });
                view.read_with(&visual, |view, _| {
                    assert!(!view.primary_pressed && !view.canvas_pointer_down);
                    assert_eq!(view.active_tool(), ToolKind::Select);
                });
                for position in [start + point(px(40.0), px(30.0)), start] {
                    visual.simulate_event(MouseMoveEvent {
                        position,
                        pressed_button: None,
                        modifiers: gpui::Modifiers::none(),
                    });
                }
                visual.simulate_event(MouseUpEvent {
                    position: start,
                    button: MouseButton::Left,
                    modifiers: gpui::Modifiers::none(),
                    click_count: 1,
                });
                mounted_canvas_click(&mut visual, start, 1);
                item.read_with(&visual, |item, _| {
                    let document = item.document().expect("document");
                    assert_eq!(
                        serde_json::to_value(&document.doc).expect("full document"),
                        expected
                    );
                    assert_eq!(document.raw_assets, assets);
                    assert_eq!(
                        document.doc.scene.get(bitmap).expect("bitmap").transform,
                        transform
                    );
                    assert_eq!(document.doc.history.undo_depth(), 0);
                    assert_eq!(document.doc.history.redo_depth(), 0);
                    assert!(!item.content_preview_active());
                    assert!(!item.is_dirty());
                });
            }
        }
    }
}

#[gpui::test]
async fn mounted_deactivation_cancels_canvas_press_before_content_preview(
    cx: &mut TestAppContext,
) {
    init_visual_test(cx);
    let project = Project::test(FakeFs::new(cx.executor()), [], cx).await;
    let (mut doc, bitmap, images) = bitmap_canvas_doc();
    let mut control = doc.scene.get(bitmap).expect("bitmap").clone();
    control.id = NodeId::new();
    control.index = doc.scene.next_child_index(control.parent);
    control.transform = Transform2D::translation(300.0, 200.0);
    doc.scene.insert(control).expect("untouched bitmap");
    doc.selection.select_only(bitmap);
    let (item, view, mut visual) =
        mounted_canvas_interaction_fixture(&project, doc, images, cx);
    let (expected, assets, world_bounds, order) = item.read_with(&visual, |item, _| {
        let document = item.document().expect("document");
        (
            serde_json::to_value(&document.doc).expect("full document"),
            document.raw_assets.clone(),
            document.doc.scene.world_bounds(bitmap),
            document
                .doc
                .scene
                .children_of(document.doc.active_page())
                .to_vec(),
        )
    });
    let start = view.read_with(&visual, |view, _| {
        view.container_bounds.expect("canvas").center()
    });
    for workspace_deactivated in [false, true] {
        visual.simulate_event(MouseDownEvent {
            position: start,
            button: MouseButton::Left,
            modifiers: gpui::Modifiers::none(),
            click_count: 1,
            first_mouse: false,
        });
        view.read_with(&visual, |view, cx| {
            assert!(view.primary_pressed && view.canvas_pointer_down);
            assert!(!item.read(cx).content_preview_active());
        });
        view.update_in(&mut visual, |view, window, cx| {
            if workspace_deactivated {
                Item::workspace_deactivated(view, window, cx);
            } else {
                Item::deactivated(view, window, cx);
            }
        });
        item.read_with(&visual, |item, _| {
            let document = item.document().expect("document");
            assert_eq!(
                serde_json::to_value(&document.doc).expect("full document"),
                expected,
                "deactivation must preserve selection before a new click"
            );
            assert_eq!(document.raw_assets, assets);
            assert_eq!(document.doc.scene.world_bounds(bitmap), world_bounds);
            assert_eq!(document.doc.history.undo_depth(), 0);
            assert_eq!(document.doc.history.redo_depth(), 0);
            assert_eq!(
                document.doc.scene.children_of(document.doc.active_page()),
                order
            );
            assert!(!item.content_preview_active());
            assert!(!item.is_dirty());
        });
        view.read_with(&visual, |view, _| {
            assert!(!view.primary_pressed && !view.canvas_pointer_down);
        });
        visual.simulate_event(MouseMoveEvent {
            position: start + point(px(40.0), px(30.0)),
            pressed_button: None,
            modifiers: gpui::Modifiers::none(),
        });
        mounted_canvas_click(&mut visual, start, 1);
        item.read_with(&visual, |item, _| {
            let document = item.document().expect("document");
            assert_eq!(
                serde_json::to_value(&document.doc).expect("full document"),
                expected
            );
            assert_eq!(document.raw_assets, assets);
            assert_eq!(document.doc.history.undo_depth(), 0);
            assert_eq!(document.doc.history.redo_depth(), 0);
            assert!(!item.content_preview_active());
            assert!(!item.is_dirty());
        });
    }
}

#[gpui::test]
async fn mounted_pen_anchors_survive_pending_press_deactivation(cx: &mut TestAppContext) {
    init_visual_test(cx);
    let project = Project::test(FakeFs::new(cx.executor()), [], cx).await;
    let (doc, _, images) = bitmap_canvas_doc();
    let (item, view, mut visual) =
        mounted_canvas_interaction_fixture(&project, doc, images, cx);
    let center = view.read_with(&visual, |view, _| {
        view.container_bounds.expect("canvas").center()
    });
    mounted_canvas_click(&mut visual, center, 1);
    visual.update(|window, cx| {
        let focus_handle = view.read(cx).focus_handle.clone();
        focus_handle.focus(window, cx);
        window.dispatch_action(ActivatePenTool.boxed_clone(), cx);
    });
    visual.run_until_parked();
    view.read_with(&visual, |view, _| {
        assert_eq!(view.active_tool(), ToolKind::Pen)
    });
    for position in [
        center + point(px(-80.0), px(-152.0)),
        center + point(px(80.0), px(-152.0)),
    ] {
        mounted_canvas_click(&mut visual, position, 1);
    }
    let third_anchor = center + point(px(80.0), px(-80.0));
    visual.simulate_event(MouseDownEvent {
        position: third_anchor,
        button: MouseButton::Left,
        modifiers: gpui::Modifiers::none(),
        click_count: 1,
        first_mouse: false,
    });
    let expected_lines = vec![
        fanta_tools::ToolOverlay::PreviewLine {
            world_start: [-80.0, -152.0],
            world_end: [80.0, -152.0],
        },
        fanta_tools::ToolOverlay::PreviewLine {
            world_start: [80.0, -152.0],
            world_end: [80.0, -80.0],
        },
    ];
    view.read_with(&visual, |view, _| {
        assert_eq!(view.active_tool(), ToolKind::Pen);
        assert!(view.primary_pressed);
        assert_eq!(
            view.tools.overlays.get(..2),
            Some(expected_lines.as_slice())
        );
    });
    let (expected, assets) = item.read_with(&visual, |item, _| {
        let document = item.document().expect("document");
        assert!(!item.content_preview_active());
        (
            serde_json::to_value(&document.doc).expect("full document"),
            document.raw_assets.clone(),
        )
    });
    for workspace_deactivated in [false, true] {
        view.update_in(&mut visual, |view, window, cx| {
            if workspace_deactivated {
                Item::workspace_deactivated(view, window, cx);
            } else {
                Item::deactivated(view, window, cx);
            }
        });
        view.read_with(&visual, |view, _| {
            assert_eq!(view.active_tool(), ToolKind::Pen);
            assert_eq!(
                view.tools.overlays.get(..2),
                Some(expected_lines.as_slice())
            );
        });
        visual.simulate_event(MouseMoveEvent {
            position: third_anchor + point(px(12.0), px(14.0)),
            pressed_button: None,
            modifiers: gpui::Modifiers::none(),
        });
        view.read_with(&visual, |view, _| {
            assert_eq!(view.active_tool(), ToolKind::Pen);
            assert_eq!(
                view.tools.overlays.get(..2),
                Some(expected_lines.as_slice())
            );
        });
        item.read_with(&visual, |item, _| {
            let document = item.document().expect("document");
            assert_eq!(
                serde_json::to_value(&document.doc).expect("full document"),
                expected
            );
            assert_eq!(document.raw_assets, assets);
            assert_eq!(document.doc.history.undo_depth(), 0);
            assert_eq!(document.doc.history.redo_depth(), 0);
            assert!(!item.content_preview_active());
            assert!(!item.is_dirty());
        });
    }
}

#[gpui::test]
async fn mounted_gesture_perf_finishes_once_for_canvas_and_window_release(
    cx: &mut TestAppContext,
) {
    use gpui::InputEvent as _;

    init_visual_test(cx);
    let project = Project::test(FakeFs::new(cx.executor()), [], cx).await;
    let (doc, _, images) = bitmap_canvas_doc();
    let (item, view, mut visual) =
        mounted_canvas_interaction_fixture(&project, doc, images, cx);
    let before = item.read_with(&visual, |item, _| {
        serde_json::to_value(&item.doc().expect("document").scene).expect("scene")
    });
    let perf = crate::gesture_perf::GesturePerf::shared();
    view.update(&mut visual, |view, _| {
        view.gesture_perf = Some(perf.clone())
    });
    let mut expected_summaries = 0;
    for release_outside in [false, true] {
        for repaint in [false, true] {
            let bounds =
                view.read_with(&visual, |view, _| view.container_bounds.expect("canvas"));
            let start = bounds.origin + point(px(45.0), px(45.0));
            let end = if release_outside {
                bounds.origin - point(px(5.0), px(5.0))
            } else {
                start + point(px(5.0), px(5.0))
            };
            visual.update(|window, cx| {
                window.dispatch_event(
                    MouseDownEvent {
                        position: start,
                        button: MouseButton::Left,
                        modifiers: gpui::Modifiers::none(),
                        click_count: 1,
                        first_mouse: false,
                    }
                    .to_platform_input(),
                    cx,
                );
                assert!(perf.borrow().has_active_gesture());
                window.dispatch_event(
                    MouseMoveEvent {
                        position: end,
                        pressed_button: Some(MouseButton::Left),
                        modifiers: gpui::Modifiers::none(),
                    }
                    .to_platform_input(),
                    cx,
                );
                if repaint {
                    window.draw(cx).clear();
                }
                for _ in 0..2 {
                    window.dispatch_event(
                        MouseUpEvent {
                            position: end,
                            button: MouseButton::Left,
                            modifiers: gpui::Modifiers::none(),
                            click_count: 1,
                        }
                        .to_platform_input(),
                        cx,
                    );
                }
            });
            visual.run_until_parked();
            expected_summaries += 1;
            assert_eq!(perf.borrow().summary_count(), expected_summaries);
            assert_eq!(perf.borrow().last_ui_dispatch_calls(), Some(2));
            assert!(!perf.borrow().has_active_gesture());
            view.read_with(&visual, |view, _| assert!(!view.primary_pressed));
            item.read_with(&visual, |item, _| {
                let document = item.doc().expect("document");
                assert_eq!(
                    serde_json::to_value(&document.scene).expect("scene"),
                    before
                );
                assert_eq!(document.history.undo_depth(), 0);
            });
        }
    }
}

#[cfg(feature = "fanta-gpui-ui")]
#[gpui::test]
async fn mounted_canvas_toolbar_fits_narrow_panes_and_scrolls_without_panning(
    cx: &mut TestAppContext,
) {
    use fanta_gpui::toolbar::ToolbarMode;
    use gpui::Modifiers;

    init_visual_test(cx);
    let project = Project::test(FakeFs::new(cx.executor()), [], cx).await;
    let (doc, _, images) = bitmap_canvas_doc();
    let (item, view, host, mut visual) = pane_layout_fixture(&project, doc, images, cx);
    let toolbar = view.read_with(&visual, |view, _| {
        view.gpui_toolbar
            .as_ref()
            .expect("toolbar")
            .panel
            .entity_id()
    });
    let before = item.read_with(&visual, |item, _| {
        let document = item.document().expect("document");
        (
            serde_json::to_value(&document.doc).expect("document snapshot"),
            document.raw_assets.clone(),
            document.doc.history.undo_depth(),
            document.doc.history.redo_depth(),
            item.is_dirty(),
        )
    });
    let assert_workspace_tabs = |visual: &mut gpui::VisualTestContext,
                                 bounds: Bounds<Pixels>| {
        let tabs = [
            "fanta-collapsible-tab-fanta-editor-workspace-0",
            "fanta-collapsible-tab-fanta-editor-workspace-1",
            "fanta-collapsible-tab-fanta-editor-workspace-2",
        ]
        .map(|selector| visual.debug_bounds(selector).expect(selector));
        for tab in &tabs {
            assert!(tab.is_contained_within(&bounds), "{tab:?} / {bounds:?}");
        }
        let first = tabs.first().expect("Canvas tab");
        let last = tabs.last().expect("Code tab");
        let center = (first.left() + last.right()) / 2.0;
        assert!(
            (center - bounds.center().x).abs() <= px(1.0),
            "workspace tabs must center on their current body: {tabs:?} / {bounds:?}"
        );
    };
    for (mode_selector, mode, toolbar_mode, leading_selector, trailing_selector) in [
        (
            "toolbar-mode-design",
            EditorMode::Design,
            ToolbarMode::Design,
            "toolbar-group-move-tools-trigger",
            "toolbar-group-comment-tools-trigger",
        ),
        (
            "toolbar-mode-motion",
            EditorMode::Motion,
            ToolbarMode::Motion,
            "toolbar-tool-motion-select",
            "toolbar-tool-text",
        ),
        (
            "toolbar-mode-draw",
            EditorMode::Draw,
            ToolbarMode::Draw,
            "toolbar-tool-hand",
            "toolbar-tool-color-picker",
        ),
    ] {
        let mode_control = visual
            .debug_bounds("toolbar-mode-selector")
            .expect("mode control");
        visual.simulate_click(mode_control.center(), Modifiers::default());
        draw_pane_layout(&mut visual);
        let mode_choice = visual.debug_bounds(mode_selector).expect(mode_selector);
        visual.simulate_click(mode_choice.center(), Modifiers::default());
        draw_pane_layout(&mut visual);
        assert!(visual.debug_bounds("toolbar-mode-menu").is_none());
        let focus =
            visual.update(|window, cx| window.focused(cx).expect("mode selection focus"));
        for canvas_width in [720.0, 240.0, 256.0, 336.0, 720.0] {
            host.update(&mut visual, |host, cx| {
                host.pane_width = px(canvas_width + 680.0);
                cx.notify();
            });
            draw_pane_layout(&mut visual);
            let canvas = visual.debug_bounds("fig-container").expect("canvas");
            assert_eq!(canvas.size.width, px(canvas_width));
            assert_workspace_tabs(&mut visual, canvas);
            let surface = visual
                .debug_bounds("editor-toolbar-surface")
                .expect("toolbar surface");
            // The responsive surface may exactly fill the canvas width; GPUI's
            // point containment excludes its bottom/right edges.
            assert!(
                surface.left() >= canvas.left()
                    && surface.top() >= canvas.top()
                    && surface.right() <= canvas.right()
                    && surface.bottom() <= canvas.bottom(),
                "{canvas_width}: {surface:?} / {canvas:?}"
            );
            assert!((surface.center().x - canvas.center().x).abs() <= px(1.0));
            for selector in ["toolbar-mode-selector", "toolbar-tool-actions"] {
                let control = visual.debug_bounds(selector).expect(selector);
                assert!(
                    control.is_contained_within(&surface),
                    "{selector}: {control:?}"
                );
            }
            let viewport = visual
                .debug_bounds("toolbar-primary-viewport")
                .expect("scroll viewport");
            assert!(viewport.is_contained_within(&surface));
            if let Some(secondary) = visual.debug_bounds("toolbar-secondary-viewport") {
                assert!(secondary.is_contained_within(&surface));
            }
            let scroll_position = viewport.center();
            let canvas_viewport = view.read_with(&visual, |view, _| view.viewport());
            visual.simulate_event(ScrollWheelEvent {
                position: scroll_position,
                delta: ScrollDelta::Pixels(point(px(1000.0), px(0.0))),
                ..Default::default()
            });
            draw_pane_layout(&mut visual);
            assert_eq!(
                view.read_with(&visual, |view, _| view.viewport()),
                canvas_viewport
            );
            let first = visual
                .debug_bounds(leading_selector)
                .expect("leading mode control");
            assert!(
                first.is_contained_within(&viewport),
                "leading control: {first:?} / {viewport:?}"
            );
            let row = visual
                .debug_bounds("toolbar-primary-row")
                .expect("tool row");
            if row.size.width > viewport.size.width + px(1.0) {
                assert!(visual.debug_bounds("toolbar-primary-fade-end").is_some());
                visual.simulate_event(ScrollWheelEvent {
                    position: scroll_position,
                    delta: ScrollDelta::Pixels(point(px(-1000.0), px(0.0))),
                    ..Default::default()
                });
                draw_pane_layout(&mut visual);
                let moved = visual
                    .debug_bounds("toolbar-primary-row")
                    .expect("scrolled tool row");
                assert!(
                    moved.left() < row.left(),
                    "real wheel input must scroll the row"
                );
                let trailing = visual
                    .debug_bounds(trailing_selector)
                    .expect("trailing mode control");
                assert!(
                    trailing.is_contained_within(&viewport),
                    "trailing control: {trailing:?} / {viewport:?}"
                );
                assert!(visual.debug_bounds("toolbar-primary-fade-start").is_some());
                assert!(visual.debug_bounds("toolbar-primary-fade-end").is_none());
                assert_eq!(
                    view.read_with(&visual, |view, _| view.viewport()),
                    canvas_viewport
                );
                visual.simulate_event(ScrollWheelEvent {
                    position: scroll_position,
                    delta: ScrollDelta::Pixels(point(px(1000.0), px(0.0))),
                    ..Default::default()
                });
                draw_pane_layout(&mut visual);
                let restored = visual
                    .debug_bounds("toolbar-primary-row")
                    .expect("restored tool row");
                assert!((restored.left() - row.left()).abs() <= px(1.0));
            } else {
                let trailing = visual
                    .debug_bounds(trailing_selector)
                    .expect("accessible trailing mode control");
                assert!(trailing.is_contained_within(&viewport));
                // GPUI point containment excludes the bottom/right edges; a fitting
                // scroll row can share those edges with its viewport.
                assert!(
                    row.left() >= viewport.left()
                        && row.top() >= viewport.top()
                        && row.right() <= viewport.right()
                        && row.bottom() <= viewport.bottom(),
                    "{mode:?} at {canvas_width}: row {row:?} / viewport {viewport:?}"
                );
                assert!(visual.debug_bounds("toolbar-primary-fade-start").is_none());
                assert!(visual.debug_bounds("toolbar-primary-fade-end").is_none());
            }
            if canvas_width == 720.0 && mode == EditorMode::Design {
                assert!(
                    surface.size.width < canvas.size.width,
                    "wide Design pill retains its intrinsic size"
                );
                let exposed_canvas = point(canvas.left() + px(2.0), surface.center().y);
                assert!(!surface.contains(&exposed_canvas));
                visual.simulate_event(ScrollWheelEvent {
                    position: exposed_canvas,
                    delta: ScrollDelta::Pixels(point(px(0.0), px(24.0))),
                    ..Default::default()
                });
                draw_pane_layout(&mut visual);
                assert_ne!(
                    view.read_with(&visual, |view, _| view.viewport()),
                    canvas_viewport,
                    "the transparent width-bound wrapper must not block exposed canvas input"
                );
                visual.simulate_event(ScrollWheelEvent {
                    position: exposed_canvas,
                    delta: ScrollDelta::Pixels(point(px(0.0), px(-24.0))),
                    ..Default::default()
                });
                draw_pane_layout(&mut visual);
            }
            assert_eq!(
                view.read_with(&visual, |view, _| view.viewport()),
                canvas_viewport
            );
            visual.update(|window, _| assert!(focus.is_focused(window)));
            view.read_with(&visual, |view, cx| {
                assert_eq!(view.editor_mode(cx), mode);
                assert_eq!(
                    view.gpui_toolbar
                        .as_ref()
                        .expect("toolbar")
                        .panel
                        .read(cx)
                        .mode(),
                    toolbar_mode
                );
                assert_eq!(
                    view.gpui_toolbar
                        .as_ref()
                        .expect("same toolbar")
                        .panel
                        .entity_id(),
                    toolbar
                );
                assert!(view.layers_sidebar_visible && view.inspector_sidebar_visible);
                assert_eq!(
                    (view.layers_sidebar_width, view.inspector_sidebar_width),
                    (px(280.0), px(400.0))
                );
                assert_eq!(view.pane_width, Some(px(canvas_width + 680.0)));
            });
            item.read_with(&visual, |item, _| {
                let document = item.document().expect("document");
                assert_eq!(
                    (
                        serde_json::to_value(&document.doc).expect("document snapshot"),
                        document.raw_assets.clone(),
                        document.doc.history.undo_depth(),
                        document.doc.history.redo_depth(),
                        item.is_dirty(),
                    ),
                    before
                );
            });
        }
    }
    for (pane_width, canvas_width) in [
        (1016.0, 336.0),
        (588.0, 240.0),
        (450.0, 450.0),
        (1400.0, 720.0),
    ] {
        host.update(&mut visual, |host, cx| {
            host.pane_width = px(pane_width);
            cx.notify();
        });
        draw_pane_layout(&mut visual);
        let canvas = visual.debug_bounds("fig-container").expect("canvas");
        assert_eq!(canvas.size.width, px(canvas_width));
        assert_workspace_tabs(&mut visual, canvas);
        let layout = view.read_with(&visual, |view, _| view.pane_sidebar_layout());
        let canvas_tab = visual
            .debug_bounds("fanta-collapsible-tab-fanta-editor-workspace-0")
            .expect("Canvas tab above compact header");
        if layout.layers.is_none() {
            assert!(
                canvas_tab.top() >= canvas.top() + px(80.0),
                "compact workspace controls must clear the collapsed Layers header"
            );
        } else {
            assert!(canvas_tab.top() < canvas.top() + px(80.0));
        }
        let pane = Bounds::new(
            point(
                canvas.left() - layout.layers.unwrap_or_default(),
                canvas.top(),
            ),
            size(px(pane_width), canvas.size.height),
        );
        for (selector, workspace) in [
            (
                "fanta-collapsible-tab-fanta-editor-workspace-1",
                EditorWorkspace::Variables,
            ),
            (
                "fanta-collapsible-tab-fanta-editor-workspace-2",
                EditorWorkspace::Code,
            ),
        ] {
            let tab = visual.debug_bounds(selector).expect("workspace tab");
            visual.simulate_click(tab.center(), Modifiers::default());
            draw_pane_layout(&mut visual);
            assert_eq!(
                view.read_with(&visual, |view, cx| view.editor_workspace(cx)),
                workspace
            );
            assert_workspace_tabs(&mut visual, pane);
            let canvas_tab = visual
                .debug_bounds("fanta-collapsible-tab-fanta-editor-workspace-0")
                .expect("return Canvas tab");
            assert!(
                canvas_tab.top() < pane.top() + px(80.0),
                "non-Canvas workspace controls retain their original top position"
            );
            visual.simulate_click(canvas_tab.center(), Modifiers::default());
            draw_pane_layout(&mut visual);
            view.update_in(&mut visual, |view, window, cx| {
                assert_eq!(view.editor_workspace(cx), EditorWorkspace::Canvas);
                if workspace == EditorWorkspace::Code {
                    assert!(view.focus_handle(cx).is_focused(window));
                }
                assert_eq!(view.editor_mode(cx), EditorMode::Draw);
                assert_eq!(
                    view.gpui_toolbar
                        .as_ref()
                        .expect("same toolbar")
                        .panel
                        .entity_id(),
                    toolbar
                );
                assert!(view.layers_sidebar_visible && view.inspector_sidebar_visible);
                assert_eq!(
                    (view.layers_sidebar_width, view.inspector_sidebar_width),
                    (px(280.0), px(400.0))
                );
            });
            let returned_canvas = visual
                .debug_bounds("fig-container")
                .expect("returned canvas");
            assert_eq!(returned_canvas, canvas);
            assert_workspace_tabs(&mut visual, returned_canvas);
            item.read_with(&visual, |item, _| {
                let document = item.document().expect("document");
                assert_eq!(
                    (
                        serde_json::to_value(&document.doc).expect("document snapshot"),
                        document.raw_assets.clone(),
                        document.doc.history.undo_depth(),
                        document.doc.history.redo_depth(),
                        item.is_dirty(),
                    ),
                    before
                );
            });
        }
    }
}

#[cfg(feature = "fanta-gpui-ui")]
async fn assert_narrow_design_crop_controls(canvas_width: f32, cx: &mut TestAppContext) {
    init_visual_test(cx);
    cx.update(|cx| {
        cx.bind_keys([
            gpui::KeyBinding::new("home", menu::SelectFirst, Some("menu")),
            gpui::KeyBinding::new("down", menu::SelectNext, Some("menu")),
            gpui::KeyBinding::new("enter", menu::Confirm, Some("menu")),
            gpui::KeyBinding::new("cmd-z", Undo, Some("FigViewer")),
            gpui::KeyBinding::new("cmd-shift-z", Redo, Some("FigViewer")),
        ]);
    });
    let project = Project::test(FakeFs::new(cx.executor()), [], cx).await;
    let (doc, bitmap, images) = bitmap_canvas_doc();
    let (item, view, host, mut visual) = pane_layout_fixture(&project, doc, images, cx);
    let mouse_trace = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    host.update(&mut visual, |host, cx| {
        host.pane_width = px(canvas_width + 680.0);
        host.mouse_trace = Some(mouse_trace.clone());
        cx.notify();
    });
    view.update(&mut visual, |view, cx| {
        view.set_viewport_silent(Viewport::default());
        view.click_diagnostics =
            ClickDiagnostics::from_setting(Some(std::ffi::OsStr::new("1")));
        cx.notify();
    });
    draw_pane_layout(&mut visual);
    assert_eq!(
        visual
            .debug_bounds("fig-container")
            .expect("narrow canvas")
            .size
            .width,
        px(canvas_width)
    );
    assert_eq!(
        view.read_with(&visual, |view, _| view.viewport()),
        Some(Viewport::default())
    );
    let assert_controls = |visual: &mut gpui::VisualTestContext,
                           hint_visible: bool,
                           stage: &str| {
        let canvas = visual.debug_bounds("fig-container").expect("canvas");
        let controls = visual
            .debug_bounds("design-crop-controls")
            .expect("Crop controls");
        assert!(
            controls.is_contained_within(&canvas),
            "{controls:?} / {canvas:?}"
        );
        let mut occupied = Vec::new();
        for selector in [
            "design-crop-ratio",
            "design-crop-cancel",
            "design-crop-apply",
        ] {
            let bounds = visual.debug_bounds(selector).expect(selector);
            assert!(
                bounds.is_contained_within(&controls),
                "{selector}: {bounds:?} / {controls:?}"
            );
            assert!(bounds.size.width > px(0.0) && bounds.size.height > px(0.0));
            occupied.push(bounds);
        }
        let hint = visual.debug_bounds("design-crop-hint");
        view.read_with(visual, |view, _| {
            assert_eq!(
                hint.is_some(), hint_visible,
                "{stage} at {canvas_width}: canvas={canvas:?}, controls={controls:?}, hint={hint:?}, tool={:?}, pressed={}, canvas_pressed={}, ready={}, viewport={:?}, container={:?}, overlays={:?}",
                view.active_tool(), view.primary_pressed, view.canvas_pointer_down,
                view.tools.can_apply_crop(), view.viewport(), view.container_bounds,
                view.tools.overlays,
            );
        });
        if let Some(hint) = hint {
            assert!(hint.is_contained_within(&controls), "hint must not clip");
            occupied.push(hint);
        }
        for (index, bounds) in occupied.iter().enumerate() {
            for other in occupied.iter().skip(index + 1) {
                assert!(
                    !bounds.intersects(other),
                    "controls must not overlap: {bounds:?} / {other:?}"
                );
            }
        }
    };
    let assert_crop_press = |visual: &mut gpui::VisualTestContext,
                             position: Point<Pixels>,
                             stage: &str| {
        let diagnostic_regions: Vec<_> = [
            "fig-container",
            "design-crop-controls",
            "design-crop-hint",
            "design-crop-ratio",
            "design-crop-cancel",
            "design-crop-apply",
            "fanta-canvas-toolbar",
            "editor-toolbar",
            "editor-toolbar-surface",
            "fanta-inspector-overlay",
            "fanta-layers-sidebar-resize-handle",
            "fanta-inspector-sidebar-resize-handle",
        ]
        .into_iter()
        .map(|selector| (selector, visual.debug_bounds(selector)))
        .collect();
        view.read_with(visual, |view, _| {
            assert_eq!(view.active_tool(), ToolKind::Crop, "{stage}");
            assert!(
                view.primary_pressed && view.canvas_pointer_down,
                "{stage} at {canvas_width}: position={position:?}, pressed={}, canvas_pressed={}, ready={}, viewport={:?}, container={:?}, overlays={:?}, diagnostic_records={:?}, regions={diagnostic_regions:?}, window_mouse_records(limit=64)={:?}",
                view.primary_pressed, view.canvas_pointer_down,
                view.tools.can_apply_crop(), view.viewport(), view.container_bounds,
                view.tools.overlays,
                view.click_diagnostics.as_ref().map(|diagnostics| &diagnostics.records),
                mouse_trace.borrow(),
            );
        });
    };
    let assert_crop_preview = |visual: &gpui::VisualTestContext, stage: &str| {
        view.read_with(visual, |view, _| {
            assert!(
                view.tools.overlays.iter().any(|overlay| matches!(
                    overlay, fanta_tools::ToolOverlay::PreviewRect { .. }
                )),
                "{stage} at {canvas_width}: tool={:?}, pressed={}, canvas_pressed={}, ready={}, viewport={:?}, container={:?}, overlays={:?}",
                view.active_tool(), view.primary_pressed, view.canvas_pointer_down,
                view.tools.can_apply_crop(), view.viewport(), view.container_bounds,
                view.tools.overlays,
            );
        });
    };
    let position = view.read_with(&visual, |view, _| {
        view.container_bounds.expect("canvas").center()
    });
    mounted_canvas_click(&mut visual, position, 1);
    let (original_doc, original_assets, original_world) = item.read_with(&visual, |item, _| {
        let document = item.document().expect("document");
        (
            document.doc.clone(),
            document.raw_assets.clone(),
            document.doc.scene.world_transform(bitmap),
        )
    });
    let original = original_doc.scene.get(bitmap).expect("bitmap").clone();
    let page = original.parent.expect("page");
    let assert_unedited = |visual: &gpui::VisualTestContext| {
        item.read_with(visual, |item, _| {
            let document = item.document().expect("document");
            assert_eq!(
                serde_json::to_value(&document.doc).expect("actual document"),
                serde_json::to_value(&original_doc).expect("original document")
            );
            assert_eq!(document.raw_assets, original_assets);
            assert_eq!(
                (
                    document.doc.history.undo_depth(),
                    document.doc.history.redo_depth()
                ),
                (0, 0)
            );
            assert!(!item.is_dirty());
        });
    };
    mounted_canvas_click(&mut visual, position, 2);
    draw_pane_layout(&mut visual);
    assert_controls(&mut visual, true, "entered Crop");
    view.read_with(&visual, |view, _| {
        assert_eq!(view.active_tool(), ToolKind::Crop);
        assert!(!view.tools.can_apply_crop());
    });
    let apply = visual
        .debug_bounds("design-crop-apply")
        .expect("disabled Apply")
        .center();
    visual.simulate_click(apply, gpui::Modifiers::none());
    draw_pane_layout(&mut visual);
    assert_unedited(&visual);
    let cancel_start = position + point(px(-40.0), px(-20.0));
    let cancel_end = position + point(px(40.0), px(20.0));
    visual.simulate_mouse_down(cancel_start, MouseButton::Left, gpui::Modifiers::none());
    assert_crop_press(&mut visual, cancel_start, "cancel preview after press");
    visual.simulate_mouse_move(cancel_end, MouseButton::Left, gpui::Modifiers::none());
    assert_crop_preview(&visual, "cancel preview after move before redraw");
    visual.simulate_mouse_up(cancel_end, MouseButton::Left, gpui::Modifiers::none());
    assert_crop_preview(&visual, "cancel preview after release before redraw");
    view.read_with(&visual, |view, _| {
        assert!(
            view.tools.can_apply_crop(),
            "cancel preview after release at {canvas_width}: pressed={}, canvas_pressed={}, viewport={:?}, container={:?}, overlays={:?}",
            view.primary_pressed, view.canvas_pointer_down, view.viewport(),
            view.container_bounds, view.tools.overlays,
        );
    });
    draw_pane_layout(&mut visual);
    assert_controls(&mut visual, false, "released cancel preview after redraw");
    view.read_with(&visual, |view, _| assert!(view.tools.can_apply_crop()));
    assert_unedited(&visual);
    let cancel = visual
        .debug_bounds("design-crop-cancel")
        .expect("Cancel preview")
        .center();
    visual.simulate_click(cancel, gpui::Modifiers::none());
    draw_pane_layout(&mut visual);
    assert_controls(&mut visual, true, "cancelled preview");
    view.read_with(&visual, |view, _| {
        assert_eq!(view.active_tool(), ToolKind::Crop);
        assert!(!view.tools.can_apply_crop());
        assert!(
            !view
                .tools
                .overlays
                .iter()
                .any(|overlay| matches!(overlay, fanta_tools::ToolOverlay::PreviewRect { .. }))
        );
    });
    assert_unedited(&visual);
    let cancel = visual
        .debug_bounds("design-crop-cancel")
        .expect("Exit Crop")
        .center();
    visual.simulate_click(cancel, gpui::Modifiers::none());
    draw_pane_layout(&mut visual);
    assert!(visual.debug_bounds("design-crop-controls").is_none());
    view.read_with(&visual, |view, _| {
        assert_eq!(view.active_tool(), ToolKind::Select)
    });
    assert_unedited(&visual);
    mounted_canvas_click(&mut visual, position, 2);
    draw_pane_layout(&mut visual);
    assert_controls(&mut visual, true, "re-entered Crop");
    let ratio = visual.debug_bounds("design-crop-ratio").expect("Ratio");
    visual.simulate_click(ratio.center(), gpui::Modifiers::none());
    draw_pane_layout(&mut visual);
    let square_ratio = visual
        .debug_bounds("MENU_ITEM-1:1")
        .expect("square crop ratio menu item");
    visual.simulate_click(square_ratio.center(), gpui::Modifiers::none());
    draw_pane_layout(&mut visual);
    assert!(visual.debug_bounds("MENU_ITEM-1:1").is_none());
    let start = position + point(px(-40.0), px(-20.0));
    let end = position + point(px(40.0), px(20.0));
    visual.simulate_mouse_down(start, MouseButton::Left, gpui::Modifiers::none());
    assert_crop_press(&mut visual, start, "apply preview after press");
    visual.simulate_mouse_move(end, MouseButton::Left, gpui::Modifiers::none());
    assert_crop_preview(&visual, "apply preview after move before redraw");
    view.read_with(&visual, |view, _| assert!(!view.tools.can_apply_crop()));
    draw_pane_layout(&mut visual);
    assert_crop_preview(&visual, "apply preview after move redraw before release");
    assert!(
        visual.debug_bounds("design-crop-hint").is_none(),
        "apply preview after move redraw at {canvas_width}",
    );
    visual.simulate_mouse_up(end, MouseButton::Left, gpui::Modifiers::none());
    assert_crop_preview(&visual, "apply preview after release before redraw");
    view.read_with(&visual, |view, cx| {
        assert!(view.tools.can_apply_crop());
        assert_eq!(view.editor_mode(cx), EditorMode::Design);
        assert_eq!(view.active_tool(), ToolKind::Crop);
        assert_eq!(
            view.gpui_toolbar
                .as_ref()
                .expect("toolbar")
                .draw_options
                .crop_ratio
                .as_ref(),
            "1:1"
        );
        let preview = view
            .tools
            .overlays
            .iter()
            .find_map(|overlay| match overlay {
                fanta_tools::ToolOverlay::PreviewRect { world_rect } => Some(*world_rect),
                _ => None,
            })
            .expect("pending crop");
        assert_eq!(
            preview,
            fanta_doc::Bounds::from_xywh(-40.0, -20.0, 80.0, 80.0)
        );
    });
    item.read_with(&visual, |item, _| {
        assert_eq!(
            serde_json::to_value(item.doc().expect("document")).expect("preview document"),
            serde_json::to_value(&original_doc).expect("original document")
        );
        assert!(!item.is_dirty());
    });
    view.update(&mut visual, |view, cx| {
        view.dispatch_tool_event(
            key_event(LogicalKey::ArrowLeft, gpui::Modifiers::none()),
            cx,
        );
    });
    view.read_with(&visual, |view, _| {
        assert!(view.tools.overlays.is_empty());
        assert!(view.tools.can_apply_crop());
    });
    draw_pane_layout(&mut visual);
    assert!(visual.debug_bounds("design-crop-hint").is_none());
    let now = || {
        i64::try_from(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock after Unix epoch")
                .as_secs(),
        )
        .expect("timestamp fits document metadata")
    };
    let started = now();
    draw_pane_layout(&mut visual);
    assert_controls(&mut visual, false, "released apply preview after redraw");
    let apply = visual
        .debug_bounds("design-crop-apply")
        .expect("Apply crop");
    visual.simulate_click(apply.center(), gpui::Modifiers::none());
    let crop = item.read_with(&visual, |item, _| {
        let doc = item.doc().expect("document");
        let crop = single_selection(doc).expect("crop selected");
        assert_ne!(crop, bitmap);
        assert_eq!(doc.scene.len(), original_doc.scene.len() + 1);
        assert_eq!(doc.scene.children_of(Some(page)), &[crop]);
        assert_eq!(doc.scene.children_of(Some(crop)), &[bitmap]);
        assert_eq!(doc.scene.get(bitmap).expect("bitmap").data, original.data);
        assert_eq!(doc.scene.world_transform(bitmap), original_world);
        crop
    });
    let mut expected = original_doc.clone();
    let mut group = CanvasNode::new(NodeData::Group(GroupNode {
        local_size: Some([80.0, 80.0]),
        clip_size: Some([80.0, 80.0]),
        ..Default::default()
    }));
    group.id = crop;
    group.name = "Crop".into();
    group.parent = Some(page);
    group.index = original.index;
    group.transform = Transform2D::translation(-40.0, -20.0);
    let mut transaction = fanta_doc::Transaction::new("Crop");
    transaction.push(Operation::create_node(group));
    transaction.push(Operation::Reparent {
        id: bitmap,
        old_parent: Some(page),
        old_index: original.index,
        new_parent: Some(crop),
        new_index: IndexKey::FIRST,
    });
    transaction.push(Operation::SetTransform {
        id: bitmap,
        old: original.transform,
        new: Transform2D::translation(-60.0, -30.0),
    });
    expected
        .apply_transaction(transaction)
        .expect("expected crop transaction");
    expected.selection.select_only(crop);
    let assert_state = |visual: &gpui::VisualTestContext, expected: &Doc| {
        item.read_with(visual, |item, _| {
            let document = item.document().expect("document");
            let actual = &document.doc;
            assert!((started..=now()).contains(&actual.metadata.modified_at));
            let mut expected = expected.clone();
            // Only the wall-clock field varies between the independently constructed transaction and UI input.
            expected.metadata.modified_at = actual.metadata.modified_at;
            assert_eq!(
                serde_json::to_value(actual).expect("actual document"),
                serde_json::to_value(&expected).expect("expected document")
            );
            assert_eq!(document.raw_assets, original_assets);
            assert_eq!(
                (actual.history.undo_depth(), actual.history.redo_depth()),
                (expected.history.undo_depth(), expected.history.redo_depth())
            );
            assert_eq!(actual.scene.world_transform(bitmap), original_world);
            assert!(item.is_dirty());
        });
        view.read_with(visual, |view, cx| {
            assert_eq!(view.editor_mode(cx), EditorMode::Design);
            assert!(!view.tools.can_apply_crop());
            assert!(!view.tools.overlays.iter().any(|overlay| matches!(
                overlay,
                fanta_tools::ToolOverlay::PreviewRect { .. }
            )));
        });
    };
    assert_eq!(expected.history.undo_depth(), 1);
    assert_eq!(expected.history.redo_depth(), 0);
    assert_state(&visual, &expected);
    let applied = item.read_with(&visual, |item, _| {
        serde_json::to_value(item.doc().expect("document")).expect("applied document")
    });
    draw_pane_layout(&mut visual);
    let apply = visual
        .debug_bounds("design-crop-apply")
        .expect("Apply crop");
    visual.simulate_click(apply.center(), gpui::Modifiers::none());
    item.read_with(&visual, |item, _| {
        assert_eq!(
            serde_json::to_value(item.doc().expect("document")).expect("second apply"),
            applied
        );
    });
    assert_state(&visual, &expected);
    visual.simulate_keystrokes("cmd-z");
    assert!(expected.undo().expect("expected Undo"));
    expected.selection.clear();
    assert_eq!(expected.history.undo_depth(), 0);
    assert_eq!(expected.history.redo_depth(), 1);
    item.read_with(&visual, |item, _| {
        let doc = item.doc().expect("document");
        assert!(doc.selection.is_empty());
        assert_eq!(doc.scene.get(bitmap), Some(&original));
        assert_eq!(
            serde_json::to_value(&doc.scene).expect("undone scene"),
            serde_json::to_value(&original_doc.scene).expect("original scene")
        );
    });
    assert_state(&visual, &expected);
    visual.simulate_keystrokes("cmd-shift-z");
    assert!(expected.redo().expect("expected Redo"));
    assert_eq!(expected.history.undo_depth(), 1);
    assert_eq!(expected.history.redo_depth(), 0);
    item.read_with(&visual, |item, _| {
        let doc = item.doc().expect("document");
        assert!(doc.selection.is_empty());
        assert_eq!(doc.scene.children_of(Some(page)), &[crop]);
        assert_eq!(doc.scene.children_of(Some(crop)), &[bitmap]);
    });
    assert_state(&visual, &expected);
}

#[cfg(feature = "fanta-gpui-ui")]
#[gpui::test]
async fn mounted_canvas_crop_controls_fit_240_and_preserve_cancel_apply_history(
    cx: &mut TestAppContext,
) {
    assert_narrow_design_crop_controls(240.0, cx).await;
}

#[cfg(feature = "fanta-gpui-ui")]
#[gpui::test]
async fn mounted_canvas_crop_controls_fit_256_and_preserve_cancel_apply_history(
    cx: &mut TestAppContext,
) {
    assert_narrow_design_crop_controls(256.0, cx).await;
}

#[cfg(feature = "fanta-gpui-ui")]
#[gpui::test]
async fn mounted_canvas_crop_controls_fit_336_and_preserve_cancel_apply_history(
    cx: &mut TestAppContext,
) {
    assert_narrow_design_crop_controls(336.0, cx).await;
}

#[cfg(feature = "fanta-gpui-ui")]
async fn assert_mounted_design_crop_controls(cx: &mut TestAppContext, context_menu: bool) {
    init_visual_test(cx);
    cx.update(|cx| {
        cx.bind_keys([
            gpui::KeyBinding::new("home", menu::SelectFirst, Some("menu")),
            gpui::KeyBinding::new("down", menu::SelectNext, Some("menu")),
            gpui::KeyBinding::new("enter", menu::Confirm, Some("menu")),
        ]);
    });
    let project = Project::test(FakeFs::new(cx.executor()), [], cx).await;
    let (doc, bitmap, images) = bitmap_canvas_doc();
    let (item, view, mut visual) =
        mounted_canvas_interaction_fixture(&project, doc, images, cx);
    let (expected, original_assets) = item.read_with(&visual, |item, _| {
        let document = item.document().expect("document");
        let mut expected = document.doc.clone();
        expected.selection.select_only(bitmap);
        (
            serde_json::to_value(expected).expect("selected document"),
            document.raw_assets.clone(),
        )
    });
    let assert_unchanged = |visual: &gpui::VisualTestContext| {
        item.read_with(visual, |item, _| {
            let document = item.document().expect("document");
            assert_eq!(
                serde_json::to_value(&document.doc).expect("document"),
                expected
            );
            assert_eq!(document.doc.selection.as_slice(), &[bitmap]);
            assert_eq!(document.doc.history.undo_depth(), 0);
            assert_eq!(document.doc.history.redo_depth(), 0);
            assert_eq!(document.raw_assets, original_assets);
            assert!(!item.is_dirty());
        });
        view.read_with(visual, |view, cx| {
            assert_eq!(view.editor_mode(cx), EditorMode::Design);
        });
    };
    assert!(visual.debug_bounds("design-crop-controls").is_none());
    let position = view.read_with(&visual, |view, _| {
        view.container_bounds.expect("canvas").center()
    });
    if context_menu {
        visual.simulate_event(MouseDownEvent {
            position,
            button: MouseButton::Right,
            modifiers: Default::default(),
            click_count: 1,
            first_mouse: false,
        });
        visual.simulate_event(MouseUpEvent {
            position,
            button: MouseButton::Right,
            modifiers: Default::default(),
            click_count: 1,
        });
        visual.run_until_parked();
        view.read_with(&visual, |view, _| {
            assert!(view.canvas_context_menu.is_some());
        });
        visual.simulate_keystrokes("home down down down down down down enter");
    } else {
        mounted_canvas_click(&mut visual, position, 1);
        mounted_canvas_click(&mut visual, position, 2);
    }
    visual.update(|window, cx| window.draw(cx).clear());
    view.read_with(&visual, |view, _| {
        assert_eq!(view.active_tool(), ToolKind::Crop);
        assert!(!view.tools.can_apply_crop());
    });
    assert!(visual.debug_bounds("design-crop-hint").is_some());
    assert_unchanged(&visual);
    for selector in [
        "design-crop-ratio",
        "design-crop-cancel",
        "design-crop-apply",
    ] {
        assert!(visual.debug_bounds(selector).is_some(), "{selector}");
    }
    let apply = visual
        .debug_bounds("design-crop-apply")
        .expect("Apply crop");
    visual.simulate_click(apply.center(), gpui::Modifiers::none());
    view.update_in(&mut visual, |view, window, cx| {
        view.handle_design_crop_toolbar_action(
            &fanta_gpui::toolbar::ToolbarAction::DrawActionInvoked {
                action: fanta_gpui::toolbar::DrawToolbarAction::ApplyCrop,
            },
            window,
            cx,
        );
    });
    assert_unchanged(&visual);
    let ratio = visual.debug_bounds("design-crop-ratio").expect("Ratio");
    visual.simulate_click(ratio.center(), gpui::Modifiers::none());
    visual.update(|window, cx| window.draw(cx).clear());
    let square_ratio = visual
        .debug_bounds("MENU_ITEM-1:1")
        .expect("square crop ratio menu item");
    visual.simulate_click(square_ratio.center(), gpui::Modifiers::none());
    visual.update(|window, cx| window.draw(cx).clear());
    assert!(visual.debug_bounds("MENU_ITEM-1:1").is_none());
    view.read_with(&visual, |view, cx| {
        let adapter = view.gpui_toolbar.as_ref().expect("toolbar");
        assert_eq!(adapter.draw_options.crop_ratio.as_ref(), "1:1");
        let properties = view.gpui_properties.as_ref().expect("properties");
        assert_eq!(
            properties
                .draw
                .read(cx)
                .view_data()
                .options
                .crop_ratio
                .as_ref(),
            "1:1"
        );
    });
    assert_unchanged(&visual);
    let start = position + point(px(-40.0), px(-20.0));
    let end = position + point(px(40.0), px(20.0));
    visual.simulate_mouse_down(start, MouseButton::Left, gpui::Modifiers::none());
    visual.simulate_mouse_move(end, MouseButton::Left, gpui::Modifiers::none());
    visual.simulate_mouse_up(end, MouseButton::Left, gpui::Modifiers::none());
    view.read_with(&visual, |view, _| {
        let preview = view
            .tools
            .overlays
            .iter()
            .find_map(|overlay| match overlay {
                fanta_tools::ToolOverlay::PreviewRect { world_rect } => Some(*world_rect),
                _ => None,
            })
            .expect("ratio-constrained crop preview");
        assert_eq!(preview.width(), 80.0);
        assert_eq!(preview.height(), 80.0);
        assert!(view.tools.can_apply_crop());
    });
    assert_unchanged(&visual);
    visual.update(|window, cx| window.draw(cx).clear());
    assert!(visual.debug_bounds("design-crop-hint").is_none());
    let cancel = visual.debug_bounds("design-crop-cancel").expect("Cancel");
    visual.simulate_click(cancel.center(), gpui::Modifiers::none());
    view.read_with(&visual, |view, _| {
        assert_eq!(view.active_tool(), ToolKind::Crop);
        assert!(!view.tools.can_apply_crop());
        assert!(
            !view
                .tools
                .overlays
                .iter()
                .any(|overlay| matches!(overlay, fanta_tools::ToolOverlay::PreviewRect { .. }))
        );
    });
    assert_unchanged(&visual);
    visual.update(|window, cx| window.draw(cx).clear());
    assert!(visual.debug_bounds("design-crop-hint").is_some());
    let apply = visual
        .debug_bounds("design-crop-apply")
        .expect("Apply crop");
    visual.simulate_click(apply.center(), gpui::Modifiers::none());
    assert_unchanged(&visual);
    let cancel = visual.debug_bounds("design-crop-cancel").expect("Cancel");
    visual.simulate_click(cancel.center(), gpui::Modifiers::none());
    visual.update(|window, cx| window.draw(cx).clear());
    view.read_with(&visual, |view, _| {
        assert_eq!(view.active_tool(), ToolKind::Select);
    });
    assert!(visual.debug_bounds("design-crop-controls").is_none());
    assert_unchanged(&visual);
}

#[cfg(feature = "fanta-gpui-ui")]
#[gpui::test]
async fn mounted_design_crop_context_menu_controls_preserve_cancelled_document(
    cx: &mut TestAppContext,
) {
    assert_mounted_design_crop_controls(cx, true).await;
}

#[cfg(feature = "fanta-gpui-ui")]
#[gpui::test]
async fn mounted_design_crop_double_click_controls_preserve_cancelled_document(
    cx: &mut TestAppContext,
) {
    assert_mounted_design_crop_controls(cx, false).await;
}

#[cfg(feature = "fanta-gpui-ui")]
#[gpui::test]
async fn mounted_design_crop_apply_button_commits_once_and_roundtrips_history(
    cx: &mut TestAppContext,
) {
    init_visual_test(cx);
    cx.update(|cx| {
        cx.bind_keys([
            gpui::KeyBinding::new("home", menu::SelectFirst, Some("menu")),
            gpui::KeyBinding::new("down", menu::SelectNext, Some("menu")),
            gpui::KeyBinding::new("enter", menu::Confirm, Some("menu")),
        ]);
    });
    let project = Project::test(FakeFs::new(cx.executor()), [], cx).await;
    let (doc, bitmap, images) = bitmap_canvas_doc();
    let (item, view, mut visual) =
        mounted_canvas_interaction_fixture(&project, doc, images, cx);
    let position = view.read_with(&visual, |view, _| {
        view.container_bounds.expect("canvas").center()
    });
    mounted_canvas_click(&mut visual, position, 1);
    let (original_doc, original_assets, original_world) = item.read_with(&visual, |item, _| {
        let document = item.document().expect("document");
        (
            document.doc.clone(),
            document.raw_assets.clone(),
            document.doc.scene.world_transform(bitmap),
        )
    });
    let original = original_doc.scene.get(bitmap).expect("bitmap").clone();
    let page = original.parent.expect("page");
    mounted_canvas_click(&mut visual, position, 2);
    visual.update(|window, cx| window.draw(cx).clear());
    let ratio = visual.debug_bounds("design-crop-ratio").expect("Ratio");
    visual.simulate_click(ratio.center(), gpui::Modifiers::none());
    visual.update(|window, cx| window.draw(cx).clear());
    let square_ratio = visual
        .debug_bounds("MENU_ITEM-1:1")
        .expect("square crop ratio menu item");
    visual.simulate_click(square_ratio.center(), gpui::Modifiers::none());
    visual.update(|window, cx| window.draw(cx).clear());
    assert!(visual.debug_bounds("MENU_ITEM-1:1").is_none());
    let start = position + point(px(-40.0), px(-20.0));
    let end = position + point(px(40.0), px(20.0));
    visual.simulate_mouse_down(start, MouseButton::Left, gpui::Modifiers::none());
    visual.simulate_mouse_move(end, MouseButton::Left, gpui::Modifiers::none());
    view.read_with(&visual, |view, _| assert!(!view.tools.can_apply_crop()));
    visual.update(|window, cx| window.draw(cx).clear());
    assert!(visual.debug_bounds("design-crop-hint").is_none());
    visual.simulate_mouse_up(end, MouseButton::Left, gpui::Modifiers::none());
    view.read_with(&visual, |view, cx| {
        assert!(view.tools.can_apply_crop());
        assert_eq!(view.editor_mode(cx), EditorMode::Design);
        assert_eq!(view.active_tool(), ToolKind::Crop);
        assert_eq!(
            view.gpui_toolbar
                .as_ref()
                .expect("toolbar")
                .draw_options
                .crop_ratio
                .as_ref(),
            "1:1"
        );
        let preview = view
            .tools
            .overlays
            .iter()
            .find_map(|overlay| match overlay {
                fanta_tools::ToolOverlay::PreviewRect { world_rect } => Some(*world_rect),
                _ => None,
            })
            .expect("pending crop");
        assert_eq!(
            preview,
            fanta_doc::Bounds::from_xywh(-40.0, -20.0, 80.0, 80.0)
        );
    });
    item.read_with(&visual, |item, _| {
        assert_eq!(
            serde_json::to_value(item.doc().expect("document")).expect("preview document"),
            serde_json::to_value(&original_doc).expect("original document")
        );
        assert!(!item.is_dirty());
    });
    view.update(&mut visual, |view, cx| {
        view.dispatch_tool_event(
            key_event(LogicalKey::ArrowLeft, gpui::Modifiers::none()),
            cx,
        );
    });
    view.read_with(&visual, |view, _| {
        assert!(view.tools.overlays.is_empty());
        assert!(view.tools.can_apply_crop());
    });
    visual.update(|window, cx| window.draw(cx).clear());
    assert!(visual.debug_bounds("design-crop-hint").is_none());
    let now = || {
        i64::try_from(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock after Unix epoch")
                .as_secs(),
        )
        .expect("timestamp fits document metadata")
    };
    let started = now();
    visual.update(|window, cx| window.draw(cx).clear());
    let apply = visual
        .debug_bounds("design-crop-apply")
        .expect("Apply crop");
    visual.simulate_click(apply.center(), gpui::Modifiers::none());
    let crop = item.read_with(&visual, |item, _| {
        let doc = item.doc().expect("document");
        let crop = single_selection(doc).expect("crop selected");
        assert_ne!(crop, bitmap);
        assert_eq!(doc.scene.len(), original_doc.scene.len() + 1);
        assert_eq!(doc.scene.children_of(Some(page)), &[crop]);
        assert_eq!(doc.scene.children_of(Some(crop)), &[bitmap]);
        assert_eq!(doc.scene.get(bitmap).expect("bitmap").data, original.data);
        assert_eq!(doc.scene.world_transform(bitmap), original_world);
        crop
    });
    let mut expected = original_doc.clone();
    let mut group = CanvasNode::new(NodeData::Group(GroupNode {
        local_size: Some([80.0, 80.0]),
        clip_size: Some([80.0, 80.0]),
        ..Default::default()
    }));
    group.id = crop;
    group.name = "Crop".into();
    group.parent = Some(page);
    group.index = original.index;
    group.transform = Transform2D::translation(-40.0, -20.0);
    let mut transaction = fanta_doc::Transaction::new("Crop");
    transaction.push(Operation::create_node(group));
    transaction.push(Operation::Reparent {
        id: bitmap,
        old_parent: Some(page),
        old_index: original.index,
        new_parent: Some(crop),
        new_index: IndexKey::FIRST,
    });
    transaction.push(Operation::SetTransform {
        id: bitmap,
        old: original.transform,
        new: Transform2D::translation(-60.0, -30.0),
    });
    expected
        .apply_transaction(transaction)
        .expect("expected crop transaction");
    expected.selection.select_only(crop);
    let assert_state = |visual: &gpui::VisualTestContext, expected: &Doc| {
        item.read_with(visual, |item, _| {
            let document = item.document().expect("document");
            let actual = &document.doc;
            assert!((started..=now()).contains(&actual.metadata.modified_at));
            let mut expected = expected.clone();
            // Only the wall-clock field varies between the independently constructed transaction and UI input.
            expected.metadata.modified_at = actual.metadata.modified_at;
            assert_eq!(
                serde_json::to_value(actual).expect("actual document"),
                serde_json::to_value(&expected).expect("expected document")
            );
            assert_eq!(document.raw_assets, original_assets);
            assert_eq!(actual.scene.world_transform(bitmap), original_world);
            assert!(item.is_dirty());
        });
        view.read_with(visual, |view, cx| {
            assert_eq!(view.editor_mode(cx), EditorMode::Design);
            assert!(!view.tools.can_apply_crop());
            assert!(!view.tools.overlays.iter().any(|overlay| matches!(
                overlay,
                fanta_tools::ToolOverlay::PreviewRect { .. }
            )));
        });
    };
    assert_eq!(expected.history.undo_depth(), 1);
    assert_eq!(expected.history.redo_depth(), 0);
    assert_state(&visual, &expected);
    let applied = item.read_with(&visual, |item, _| {
        serde_json::to_value(item.doc().expect("document")).expect("applied document")
    });
    visual.update(|window, cx| window.draw(cx).clear());
    let apply = visual
        .debug_bounds("design-crop-apply")
        .expect("Apply crop");
    visual.simulate_click(apply.center(), gpui::Modifiers::none());
    item.read_with(&visual, |item, _| {
        assert_eq!(
            serde_json::to_value(item.doc().expect("document")).expect("second apply"),
            applied
        );
    });
    assert_state(&visual, &expected);
    visual.simulate_keystrokes("cmd-z");
    assert!(expected.undo().expect("expected Undo"));
    expected.selection.clear();
    assert_eq!(expected.history.undo_depth(), 0);
    assert_eq!(expected.history.redo_depth(), 1);
    item.read_with(&visual, |item, _| {
        let doc = item.doc().expect("document");
        assert!(doc.selection.is_empty());
        assert_eq!(doc.scene.get(bitmap), Some(&original));
        assert_eq!(
            serde_json::to_value(&doc.scene).expect("undone scene"),
            serde_json::to_value(&original_doc.scene).expect("original scene")
        );
    });
    assert_state(&visual, &expected);
    visual.simulate_keystrokes("cmd-shift-z");
    assert!(expected.redo().expect("expected Redo"));
    assert_eq!(expected.history.undo_depth(), 1);
    assert_eq!(expected.history.redo_depth(), 0);
    item.read_with(&visual, |item, _| {
        let doc = item.doc().expect("document");
        assert!(doc.selection.is_empty());
        assert_eq!(doc.scene.children_of(Some(page)), &[crop]);
        assert_eq!(doc.scene.children_of(Some(crop)), &[bitmap]);
    });
    assert_state(&visual, &expected);
}

#[gpui::test]
async fn mounted_bitmap_crop_applies_once_and_history_preserves_the_asset(
    cx: &mut TestAppContext,
) {
    init_visual_test(cx);
    let project = Project::test(FakeFs::new(cx.executor()), [], cx).await;
    let (doc, bitmap, images) = bitmap_canvas_doc();
    let (item, view, mut visual) =
        mounted_canvas_interaction_fixture(&project, doc, images, cx);
    let (original, original_world, original_assets) = item.read_with(&visual, |item, _| {
        let document = item.document().expect("document");
        (
            document.doc.scene.get(bitmap).expect("bitmap").clone(),
            document.doc.scene.world_transform(bitmap),
            document.raw_assets.clone(),
        )
    });
    let bounds = view.read_with(&visual, |view, _| view.container_bounds.expect("canvas"));
    mounted_canvas_click(&mut visual, bounds.center(), 1);
    assert_eq!(
        view.read_with(&visual, |view, _| view.active_tool()),
        ToolKind::Select
    );
    mounted_canvas_click(&mut visual, bounds.center(), 2);
    assert_eq!(
        view.read_with(&visual, |view, _| view.active_tool()),
        ToolKind::Crop
    );
    item.read_with(&visual, |item, _| {
        assert_eq!(
            item.doc().expect("document").selection.as_slice(),
            &[bitmap]
        );
        assert!(!item.is_dirty());
    });
    let start = bounds.center() + point(px(-40.0), px(-20.0));
    let end = bounds.center() + point(px(40.0), px(20.0));
    visual.simulate_mouse_down(start, MouseButton::Left, gpui::Modifiers::none());
    visual.simulate_mouse_move(end, MouseButton::Left, gpui::Modifiers::none());
    visual.simulate_mouse_up(end, MouseButton::Left, gpui::Modifiers::none());
    view.read_with(&visual, |view, _| {
        assert!(
            view.tools
                .overlays
                .iter()
                .any(|overlay| matches!(overlay, fanta_tools::ToolOverlay::PreviewRect { .. }))
        );
        assert!(!view.primary_pressed);
    });
    item.read_with(&visual, |item, _| {
        assert_eq!(
            item.doc().expect("document").scene.get(bitmap),
            Some(&original)
        );
        assert_eq!(item.doc().expect("document").history.undo_depth(), 0);
        assert!(!item.is_dirty());
    });
    visual.simulate_keystrokes("enter");
    let crop = item.read_with(&visual, |item, _| {
        let document = item.document().expect("document");
        let doc = &document.doc;
        let crop = single_selection(doc).expect("crop selected");
        assert_ne!(crop, bitmap);
        assert_eq!(doc.history.undo_depth(), 1);
        assert_eq!(doc.scene.len(), 3);
        assert_eq!(doc.scene.children_of(Some(crop)), &[bitmap]);
        assert!(matches!(&doc.scene.get(crop).expect("crop").data, NodeData::Group(group) if group.clip_size == Some([80.0, 40.0])));
        assert_eq!(doc.scene.world_bounds(crop), Some(fanta_doc::Bounds::from_xywh(-40.0, -20.0, 80.0, 40.0)));
        assert_eq!(doc.scene.get(bitmap).expect("bitmap").data, original.data);
        assert_eq!(doc.scene.world_transform(bitmap), original_world);
        assert_eq!(document.raw_assets, original_assets);
        assert!(item.is_dirty());
        crop
    });
    visual.simulate_keystrokes("enter");
    item.read_with(&visual, |item, _| {
        assert_eq!(item.doc().expect("document").history.undo_depth(), 1);
    });
    visual.simulate_keystrokes("cmd-z");
    item.read_with(&visual, |item, _| {
        let document = item.document().expect("document");
        assert_eq!(document.doc.scene.get(bitmap), Some(&original));
        assert!(document.doc.scene.get(crop).is_none());
        assert_eq!(document.doc.scene.len(), 2);
        assert_eq!(document.doc.history.undo_depth(), 0);
        assert_eq!(document.raw_assets, original_assets);
    });
    visual.simulate_keystrokes("cmd-shift-z");
    item.read_with(&visual, |item, _| {
        let document = item.document().expect("document");
        assert_eq!(
            document.doc.scene.get(bitmap).expect("bitmap").parent,
            Some(crop)
        );
        assert_eq!(
            document.doc.scene.get(bitmap).expect("bitmap").data,
            original.data
        );
        assert_eq!(document.doc.scene.world_transform(bitmap), original_world);
        assert_eq!(document.doc.history.undo_depth(), 1);
        assert_eq!(document.raw_assets, original_assets);
    });
}

#[gpui::test]
async fn mounted_bitmap_crop_escape_discards_released_and_active_previews(
    cx: &mut TestAppContext,
) {
    init_visual_test(cx);
    let project = Project::test(FakeFs::new(cx.executor()), [], cx).await;
    for release_before_cancel in [false, true] {
        let (doc, bitmap, images) = bitmap_canvas_doc();
        let (item, view, mut visual) =
            mounted_canvas_interaction_fixture(&project, doc, images, cx);
        let (original_scene, original_assets) = item.read_with(&visual, |item, _| {
            let document = item.document().expect("document");
            (
                serde_json::to_value(&document.doc.scene).expect("scene"),
                document.raw_assets.clone(),
            )
        });
        let bounds = view.read_with(&visual, |view, _| view.container_bounds.expect("canvas"));
        mounted_canvas_click(&mut visual, bounds.center(), 1);
        mounted_canvas_click(&mut visual, bounds.center(), 2);
        assert_eq!(
            view.read_with(&visual, |view, _| view.active_tool()),
            ToolKind::Crop
        );
        let start = bounds.center() + point(px(-40.0), px(-20.0));
        let end = bounds.center() + point(px(40.0), px(20.0));
        visual.simulate_mouse_down(start, MouseButton::Left, gpui::Modifiers::none());
        visual.simulate_mouse_move(end, MouseButton::Left, gpui::Modifiers::none());
        if release_before_cancel {
            visual.simulate_mouse_up(end, MouseButton::Left, gpui::Modifiers::none());
        }
        view.read_with(&visual, |view, _| {
            assert!(view.tools.overlays.iter().any(|overlay| matches!(
                overlay,
                fanta_tools::ToolOverlay::PreviewRect { .. }
            )));
        });
        visual.simulate_keystrokes("escape");
        if !release_before_cancel {
            visual.simulate_mouse_up(end, MouseButton::Left, gpui::Modifiers::none());
        }
        visual.simulate_keystrokes("enter");
        view.read_with(&visual, |view, _| {
            assert_eq!(view.active_tool(), ToolKind::Crop);
            assert!(!view.primary_pressed);
            assert!(!view.canvas_pointer_down);
            assert!(!view.tools.overlays.iter().any(|overlay| matches!(
                overlay,
                fanta_tools::ToolOverlay::PreviewRect { .. }
            )));
        });
        visual.simulate_keystrokes("escape");
        assert_eq!(
            view.read_with(&visual, |view, _| view.active_tool()),
            ToolKind::Select
        );
        item.read_with(&visual, |item, _| {
            let document = item.document().expect("document");
            assert!(document.doc.scene.get(bitmap).is_some());
            assert_eq!(
                serde_json::to_value(&document.doc.scene).expect("scene"),
                original_scene
            );
            assert_eq!(document.raw_assets, original_assets);
            assert_eq!(document.doc.history.undo_depth(), 0);
            assert!(!item.is_dirty());
        });
    }
}

async fn assert_mounted_node_edit_activation_shows_anchors(
    cx: &mut TestAppContext,
    context_menu: bool,
) {
    use gpui::InputEvent as _;
    init_visual_test(cx);
    cx.update(|cx| {
        cx.bind_keys([
            gpui::KeyBinding::new("home", menu::SelectFirst, Some("menu")),
            gpui::KeyBinding::new("down", menu::SelectNext, Some("menu")),
            gpui::KeyBinding::new("enter", menu::Confirm, Some("menu")),
        ]);
    });
    let project = Project::test(FakeFs::new(cx.executor()), [], cx).await;
    let mut doc = doc_with_one_page();
    let mut vector = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        0.0,
        0.0,
        120.0,
        80.0,
        Color::BLACK,
    )));
    vector.parent = doc.active_page();
    vector.transform = Transform2D::translation(-60.0, -40.0);
    let vector_id = vector.id;
    doc.apply(Operation::create_node(vector)).expect("vector");
    doc.history = Default::default();
    let (item, view, mut visual) =
        mounted_canvas_interaction_fixture(&project, doc, BTreeMap::new(), cx);
    let expected = item.read_with(&visual, |item, _| {
        let mut expected = item.doc().expect("document").clone();
        expected.selection.select_only(vector_id);
        serde_json::to_value(expected).expect("expected selected document")
    });
    let position = view.read_with(&visual, |view, _| {
        view.container_bounds.expect("canvas").center()
    });
    let assert_anchors = |view: &FigView| {
        assert_eq!(view.active_tool(), ToolKind::NodeEdit);
        let anchors: Vec<_> = view
            .tools
            .overlays
            .iter()
            .filter_map(|overlay| match overlay {
                fanta_tools::ToolOverlay::PathAnchor { world, selected } => {
                    assert!(!selected, "activation must not select arbitrary anchors");
                    Some(*world)
                }
                _ => None,
            })
            .collect();
        assert_eq!(
            anchors.len(),
            4,
            "context_menu={context_menu}: anchors must be visible before any hover or additional pointer event"
        );
        for expected in [[-60.0, -40.0], [60.0, -40.0], [60.0, 40.0], [-60.0, 40.0]] {
            assert!(
                anchors.contains(&expected),
                "missing world-space anchor {expected:?}"
            );
        }
    };
    if context_menu {
        visual.simulate_event(MouseDownEvent {
            position,
            button: MouseButton::Right,
            modifiers: Default::default(),
            click_count: 1,
            first_mouse: false,
        });
        visual.simulate_event(MouseUpEvent {
            position,
            button: MouseButton::Right,
            modifiers: Default::default(),
            click_count: 1,
        });
        visual.run_until_parked();
        view.read_with(&visual, |view, _| {
            assert!(view.canvas_context_menu.is_some())
        });
        visual.simulate_keystrokes("home down down down down down down enter");
        view.read_with(&visual, |view, _| assert_anchors(view));
    } else {
        mounted_canvas_click(&mut visual, position, 1);
        visual.run_until_parked();
        visual.update(|window, cx| window.draw(cx).clear());
        visual.update(|window, cx| {
            window.dispatch_event(
                MouseDownEvent {
                    position,
                    button: MouseButton::Left,
                    modifiers: Default::default(),
                    click_count: 2,
                    first_mouse: false,
                }
                .to_platform_input(),
                cx,
            );
            assert_anchors(view.read(cx));
        });
        visual.simulate_event(MouseUpEvent {
            position,
            button: MouseButton::Left,
            modifiers: Default::default(),
            click_count: 2,
        });
    }
    item.read_with(&visual, |item, _| {
        assert_eq!(
            serde_json::to_value(item.doc().expect("document")).expect("after activation"),
            expected
        );
        assert!(!item.content_preview_active());
        assert!(!item.is_dirty());
    });
    visual.simulate_keystrokes("escape");
    view.read_with(&visual, |view, _| {
        assert_eq!(view.active_tool(), ToolKind::Select);
        assert!(
            !view
                .tools
                .overlays
                .iter()
                .any(|overlay| matches!(overlay, fanta_tools::ToolOverlay::PathAnchor { .. }))
        );
    });
    item.read_with(&visual, |item, _| {
        let doc = item.doc().expect("document");
        assert_eq!(doc.history.undo_depth(), 0);
        assert_eq!(
            serde_json::to_value(&doc.scene).expect("scene after exit"),
            expected["scene"]
        );
        assert!(!item.is_dirty());
    });
}

#[gpui::test]
async fn mounted_node_edit_double_click_shows_anchors_immediately(cx: &mut TestAppContext) {
    assert_mounted_node_edit_activation_shows_anchors(cx, false).await;
}

#[gpui::test]
async fn mounted_node_edit_context_menu_shows_anchors_immediately(cx: &mut TestAppContext) {
    assert_mounted_node_edit_activation_shows_anchors(cx, true).await;
}

#[gpui::test]
async fn mounted_instance_entry_from_empty_selection_preserves_click_sequence_semantics(
    cx: &mut TestAppContext,
) {
    use gpui::InputEvent as _;

    init_visual_test(cx);
    let project = Project::test(FakeFs::new(cx.executor()), [], cx).await;
    for wrapped in [false, true] {
        for isolated_second_click in [false, true] {
            for redraw_between_down_and_up in [false, true] {
                let (mut doc, _, master) = text_selection_doc(true);
                let master = master.expect("master frame");
                doc.scene.get_mut(master).expect("master").transform =
                    Transform2D::translation(1000.0, 1000.0);
                let component = fanta_doc::ComponentId::new();
                doc.components.defs.insert(
                    component,
                    fanta_doc::ComponentDef::new(component, master, "Entry sequence card"),
                );
                let wrapper = wrapped.then(|| {
                    let mut node = CanvasNode::new(NodeData::Group(GroupNode {
                        clip_size: Some([300.0, 200.0]),
                        background: Some(Fill::solid(Color::WHITE)),
                        ..Default::default()
                    }));
                    node.parent = doc.active_page();
                    node.transform = Transform2D::translation(-150.0, -100.0);
                    let id = node.id;
                    doc.scene.insert(node).expect("wrapper");
                    id
                });
                let mut instance =
                    CanvasNode::new(NodeData::Instance(fanta_doc::InstanceNode {
                        component,
                        overrides: Vec::new(),
                        prop_values: BTreeMap::new(),
                        derived: Vec::new(),
                        local_size: [200.0, 100.0],
                    }));
                instance.parent = wrapper.or(doc.active_page());
                instance.transform = if wrapped {
                    Transform2D::translation(50.0, 50.0)
                } else {
                    Transform2D::translation(-100.0, -50.0)
                };
                let instance_id = instance.id;
                doc.scene.insert(instance).expect("instance");
                doc.selection.clear();
                doc.history = Default::default();
                let (item, view, mut visual) =
                    mounted_canvas_interaction_fixture(&project, doc, BTreeMap::new(), cx);
                view.update(&mut visual, |view, cx| {
                    view.set_viewport_silent(Viewport {
                        center: [0.0, 0.0],
                        zoom: 0.63,
                    });
                    cx.notify();
                });
                visual.run_until_parked();
                visual.update(|window, cx| window.draw(cx).clear());
                let expected = item.read_with(&visual, |item, _| {
                    let mut expected = item.doc().expect("loaded document").clone();
                    expected.selection.select_only(instance_id);
                    serde_json::to_value(expected).expect("expected selected document")
                });
                let position = view.read_with(&visual, |view, _| {
                    view.container_bounds.expect("canvas").center()
                        + point(px(-60.0 * 0.63), px(-15.0 * 0.63))
                });
                let counts: &[usize] = if isolated_second_click { &[2] } else { &[1, 2] };
                for &click_count in counts {
                    visual.update(|window, cx| {
                        window.dispatch_event(
                            MouseDownEvent {
                                position,
                                button: MouseButton::Left,
                                modifiers: gpui::Modifiers::none(),
                                click_count,
                                first_mouse: false,
                            }
                            .to_platform_input(),
                            cx,
                        );
                        if redraw_between_down_and_up {
                            window.draw(cx).clear();
                        }
                        window.dispatch_event(
                            MouseUpEvent {
                                position,
                                button: MouseButton::Left,
                                modifiers: gpui::Modifiers::none(),
                                click_count,
                            }
                            .to_platform_input(),
                            cx,
                        );
                    });
                    if click_count == 1 {
                        item.read_with(&visual, |item, _| {
                            assert_eq!(
                                item.doc().expect("document").selection.as_slice(),
                                &[wrapper.unwrap_or(instance_id)]
                            );
                        });
                    }
                    view.read_with(&visual, |view, _| {
                        assert!(!view.primary_pressed);
                        assert!(!view.canvas_pointer_down);
                    });
                }
                item.read_with(&visual, |item, _| {
                    assert_eq!(
                        serde_json::to_value(item.doc().expect("document")).expect("actual"),
                        expected,
                        "wrapped={wrapped}, isolated_second={isolated_second_click}, redraw={redraw_between_down_and_up}"
                    );
                    assert!(!item.is_dirty());
                });
                view.read_with(&visual, |view, _| {
                    assert_eq!(
                        view.text_edit.is_some(),
                        !wrapped && !isolated_second_click,
                        "the first pair selects/drills the wrapper; a selected leaf can enter editing"
                    );
                });
                mounted_canvas_click(&mut visual, position, 2);
                view.read_with(&visual, |view, _| {
                    assert!(
                        view.text_edit
                            .as_ref()
                            .expect("subsequent second click enters selected instance text")
                            .session
                            .instance()
                            .is_some()
                    );
                });
                visual.simulate_keystrokes("escape");
                item.read_with(&visual, |item, _| {
                    let doc = item.doc().expect("document");
                    assert_eq!(
                        serde_json::to_value(&doc.scene).expect("scene"),
                        expected["scene"]
                    );
                    assert_eq!(doc.history.undo_depth(), 0);
                    assert!(!item.is_dirty());
                });
            }
        }
    }
}

#[gpui::test]
async fn mounted_canvas_pairs_drill_group_then_boolean_before_vector_editing(
    cx: &mut TestAppContext,
) {
    init_visual_test(cx);
    let project = Project::test(FakeFs::new(cx.executor()), [], cx).await;
    let mut doc = doc_with_one_page();
    let mut group = CanvasNode::new(NodeData::Group(GroupNode {
        clip_size: Some([300.0, 200.0]),
        ..Default::default()
    }));
    group.parent = doc.active_page();
    group.transform = Transform2D::translation(-150.0, -100.0);
    let group_id = group.id;
    doc.apply(Operation::create_node(group)).expect("group");
    let mut boolean = CanvasNode::new(NodeData::Boolean(fanta_doc::BooleanNode {
        fills: [Fill::solid(Color::WHITE)].into_iter().collect(),
        ..Default::default()
    }));
    boolean.parent = Some(group_id);
    boolean.transform = Transform2D::translation(50.0, 50.0);
    let boolean_id = boolean.id;
    doc.apply(Operation::create_node(boolean)).expect("Boolean");
    let mut vector = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        0.0,
        0.0,
        80.0,
        40.0,
        Color::WHITE,
    )));
    vector.parent = Some(boolean_id);
    let vector_id = vector.id;
    doc.apply(Operation::create_node(vector)).expect("vector");
    doc.history = Default::default();
    let (item, view, mut visual) =
        mounted_canvas_interaction_fixture(&project, doc, BTreeMap::new(), cx);
    let original_scene = item.read_with(&visual, |item, _| {
        serde_json::to_value(&item.doc().expect("document").scene).expect("scene")
    });
    let bounds = view.read_with(&visual, |view, _| view.container_bounds.expect("canvas"));
    let position = bounds.center() + point(px(-60.0), px(-30.0));
    for (click_count, selected, tool) in [
        (1, group_id, ToolKind::Select),
        (2, boolean_id, ToolKind::Select),
        (3, boolean_id, ToolKind::Select),
        (4, vector_id, ToolKind::Select),
        (5, vector_id, ToolKind::Select),
        (6, vector_id, ToolKind::NodeEdit),
    ] {
        mounted_canvas_click(&mut visual, position, click_count);
        assert_eq!(
            view.read_with(&visual, |view, _| view.active_tool()),
            tool,
            "click {click_count}"
        );
        item.read_with(&visual, |item, _| {
            let doc = item.doc().expect("document");
            assert_eq!(doc.selection.as_slice(), &[selected], "click {click_count}");
            assert_eq!(
                serde_json::to_value(&doc.scene).expect("scene"),
                original_scene
            );
            assert_eq!(doc.history.undo_depth(), 0);
            assert!(!item.is_dirty());
        });
    }
    visual.simulate_keystrokes("escape");
    assert_eq!(
        view.read_with(&visual, |view, _| view.active_tool()),
        ToolKind::Select
    );
    assert!(item.read_with(&visual, |item, _| {
        item.doc().expect("document").selection.is_empty()
    }));
}

#[gpui::test]
async fn mounted_canvas_locked_and_read_only_leaves_never_enter_specialist_editors(
    cx: &mut TestAppContext,
) {
    init_visual_test(cx);
    let project = Project::test(FakeFs::new(cx.executor()), [], cx).await;
    for bitmap_kind in [false, true] {
        for read_only in [false, true] {
            let (mut doc, node_id, images) = bitmap_canvas_doc();
            let node = doc.scene.get_mut(node_id).expect("target");
            if !bitmap_kind {
                node.data = NodeData::Vector(VectorNode::rect_solid(
                    0.0,
                    0.0,
                    200.0,
                    100.0,
                    Color::WHITE,
                ));
            }
            if !read_only {
                node.flags.insert(fanta_doc::NodeFlags::LOCKED);
            }
            let (item, view, mut visual) =
                mounted_canvas_interaction_fixture(&project, doc, images, cx);
            if read_only {
                item.update(&mut visual, |item, cx| {
                    item.set_source_edit_locked(true, cx)
                });
                visual.run_until_parked();
            }
            let original_scene = item.read_with(&visual, |item, _| {
                serde_json::to_value(&item.doc().expect("document").scene).expect("scene")
            });
            let bounds =
                view.read_with(&visual, |view, _| view.container_bounds.expect("canvas"));
            for click_count in 1..=4 {
                mounted_canvas_click(&mut visual, bounds.center(), click_count);
                view.read_with(&visual, |view, _| {
                    assert_eq!(
                        view.active_tool(),
                        ToolKind::Select,
                        "bitmap={bitmap_kind}, read_only={read_only}, click={click_count}"
                    );
                    assert!(view.text_edit.is_none());
                    assert!(!view.primary_pressed);
                    assert!(!view.canvas_pointer_down);
                });
            }
            visual.simulate_keystrokes("enter");
            assert_eq!(
                view.read_with(&visual, |view, _| view.active_tool()),
                ToolKind::Select
            );
            item.read_with(&visual, |item, _| {
                let doc = item.doc().expect("document");
                if read_only {
                    assert_eq!(doc.selection.as_slice(), &[node_id]);
                } else {
                    assert!(doc.selection.is_empty());
                }
                assert_eq!(
                    serde_json::to_value(&doc.scene).expect("scene"),
                    original_scene
                );
                assert_eq!(doc.history.undo_depth(), 0);
                assert!(!item.is_dirty());
            });
        }
    }
}

#[gpui::test]
async fn canvas_double_click_read_only_document_does_not_enter_editing(
    cx: &mut TestAppContext,
) {
    init_test(cx);
    let project = Project::test(FakeFs::new(cx.executor()), [], cx).await;
    let (mut doc, text_id, _) = text_selection_doc(false);
    doc.selection.select_only(text_id);
    let original = doc.scene.get(text_id).expect("text").clone();
    let (item, view, scratch) = canvas_interaction_fixture(&project, doc, cx);
    item.update(cx, |item, cx| item.set_source_edit_locked(true, cx));
    send_canvas_click(scratch, &view, point(px(360.0), px(295.0)), 2, cx);
    view.read_with(cx, |view, _| assert!(view.text_edit.is_none()));
    item.read_with(cx, |item, _| {
        assert_eq!(
            item.doc().expect("document").scene.get(text_id),
            Some(&original)
        );
        assert!(!item.is_dirty());
    });
}

#[gpui::test]
async fn text_tool_click_creates_text_and_queues_inline_edit(cx: &mut TestAppContext) {
    init_test(cx);
    let project = Project::test(FakeFs::new(cx.executor()), [], cx).await;
    let item = crate::document::ready_item_for_test(
        &project,
        std::path::PathBuf::from("/tmp/Design.fig"),
        doc_with_one_page(),
        cx,
    );
    let item_for_assert = item.clone();
    let scratch = cx.add_window(|_, _| gpui::Empty);
    let view = scratch
        .update(cx, |_, window, cx| {
            cx.new(|cx| FigView::new(item.clone(), project.clone(), window, cx))
        })
        .unwrap();

    view.update(cx, |view, cx| {
        view.set_container_bounds(Bounds {
            origin: point(px(0.0), px(0.0)),
            size: size(px(800.0), px(600.0)),
        });
        view.set_viewport_silent(Viewport::default());
        view.activate_tool(ToolKind::Text, cx);
        view.dispatch_tool_event(
            press_event(
                DVec2::new(400.0, 300.0),
                ToolButton::Primary,
                gpui::Modifiers::default(),
                1,
            ),
            cx,
        );
        view.dispatch_tool_event(
            release_event(
                DVec2::new(400.0, 300.0),
                ToolButton::Primary,
                gpui::Modifiers::default(),
            ),
            cx,
        );

        assert_eq!(view.active_tool(), ToolKind::Select);
        assert!(
            view.pending_text_edit.is_some(),
            "new text should be queued for inline editing"
        );
    });

    item_for_assert.read_with(cx, |item, _cx| {
        let doc = &item.document().expect("ready document").doc;
        let selected = doc.selection.as_slice();
        assert_eq!(selected.len(), 1);
        let text = doc
            .scene
            .get(selected[0])
            .and_then(|node| node.data.as_text())
            .expect("selected node should be newly created text");
        assert_eq!(text.content, fanta_tools::text::PLACEHOLDER);
    });
}

#[gpui::test]
async fn canvas_create_rectangle_and_undo_restores_state(cx: &mut TestAppContext) {
    // Covers core BDD scenarios: create shapes, undo across visual ops.
    init_visual_test(cx);
    let project = Project::test(FakeFs::new(cx.executor()), [], cx).await;
    let initial_doc = doc_with_one_page();
    let page_id = initial_doc.active_page().expect("page");

    // Seed one page with nothing extra
    let item = crate::document::ready_item_for_test(
        &project,
        std::path::PathBuf::from("/tmp/CanvasUndo.fig"),
        initial_doc,
        cx,
    );

    let scratch = cx.add_window(|_, _| gpui::Empty);
    let _view = scratch
        .update(cx, |_, window, cx| {
            cx.new(|cx| FigView::new(item.clone(), project.clone(), window, cx))
        })
        .expect("create view");

    // Simulate a simple create via document (as higher level tools would)
    let created = cx.update(|cx| {
        let mut rect = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
            10.0,
            20.0,
            100.0,
            50.0,
            Color::rgb(100, 150, 200),
        )));
        rect.parent = Some(page_id);
        let id = rect.id;
        item.update(cx, |it, cx| {
            it.apply(Operation::create_node(rect), cx)
                .expect("apply create");
        });
        id
    });

    // Verify node exists
    item.read_with(cx, |it, _| {
        let doc = &it.document().expect("doc").doc;
        assert!(
            doc.scene.contains(created),
            "node should exist after create"
        );
    });

    // Undo
    let did_undo = cx.update(|cx| item.update(cx, |it, cx| it.undo(cx).expect("undo")));
    assert!(did_undo, "undo should succeed");

    item.read_with(cx, |it, _| {
        let doc = &it.document().expect("doc").doc;
        assert!(
            !doc.scene.contains(created),
            "node should be gone after undo"
        );
    });
}
