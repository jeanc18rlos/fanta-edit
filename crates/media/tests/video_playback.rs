#[cfg(target_os = "macos")]
mod native {
    use anyhow::{Context, Result, bail, ensure};
    use core_foundation::runloop::{CFRunLoopRunInMode, kCFRunLoopDefaultMode};
    use media::video::{
        PreparedVideoPlayback, VideoFrameUpdate, VideoPlayback, VideoPlaybackFrame,
        VideoPlaybackState, prepare_video_playback, video_host_time_seconds,
    };
    use std::{
        collections::BTreeSet,
        io::Write as _,
        path::PathBuf,
        sync::Arc,
        time::{Duration, Instant},
    };

    const CLIP: &[u8] = include_bytes!("../test_fixtures/playback-three-scenes.mp4");
    const QUADRANTS: &[u8] = include_bytes!("../test_fixtures/quadrants.mp4");
    const ROTATE_90: &[u8] = include_bytes!("../test_fixtures/quadrants-rotate-90.mp4");
    const ROTATE_180: &[u8] = include_bytes!("../test_fixtures/quadrants-rotate-180.mp4");
    const ROTATE_270: &[u8] = include_bytes!("../test_fixtures/quadrants-rotate-270.mp4");
    const MIRROR: &[u8] = include_bytes!("../test_fixtures/quadrants-mirrored.mp4");
    const CORRUPT: &[u8] = include_bytes!("../test_fixtures/quadrants-corrupt.mp4");

    fn tick() {
        unsafe {
            CFRunLoopRunInMode(kCFRunLoopDefaultMode, 0.01, 0);
        }
    }

    fn wait(mut predicate: impl FnMut() -> Result<bool>, label: &str) -> Result<()> {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if predicate()? {
                return Ok(());
            }
            ensure!(Instant::now() < deadline, "Timed out: {label}");
            tick();
        }
    }

    fn prepare(bytes: &[u8], maximum: u32) -> Result<PreparedVideoPlayback> {
        use std::future::Future;
        let mut request = Box::pin(prepare_video_playback(Arc::from(bytes), maximum)?);
        let waker = futures::task::noop_waker();
        let mut context = std::task::Context::from_waker(&waker);
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let std::task::Poll::Ready(result) = request.as_mut().poll(&mut context) {
                return result;
            }
            ensure!(
                Instant::now() < deadline,
                "Timed out preparing local playback"
            );
            tick();
        }
    }

    fn player(bytes: &[u8], maximum: u32) -> Result<VideoPlayback> {
        let mut player = VideoPlayback::new(prepare(bytes, maximum)?)?;
        player.set_audio(true, 0.)?;
        wait(
            || Ok(player.status()?.state == VideoPlaybackState::Paused),
            "player ready",
        )?;
        Ok(player)
    }

    fn frame(player: &mut VideoPlayback) -> Result<VideoPlaybackFrame> {
        let mut result = None;
        wait(
            || {
                if let VideoFrameUpdate::Frame(frame) =
                    player.frame_for_host_time(video_host_time_seconds())?
                {
                    result = Some(frame);
                }
                Ok(result.is_some())
            },
            "decoded output frame",
        )?;
        result.context("Missing decoded output frame")
    }

    fn colors(frame: &VideoPlaybackFrame) -> Result<[[u8; 3]; 4]> {
        let buffer = &frame.buffer;
        ensure!(
            buffer.lock_base_address(1) == 0,
            "Could not lock video frame"
        );
        let result = (|| {
            let width = buffer.get_width();
            let height = buffer.get_height();
            let stride = buffer.get_bytes_per_row();
            ensure!(
                width > 0 && height > 0 && stride >= width * 4,
                "Invalid video frame layout"
            );
            let pointer = unsafe { buffer.get_base_address() };
            ensure!(!pointer.is_null(), "Frame has no CPU-readable storage");
            let bytes =
                unsafe { std::slice::from_raw_parts(pointer.cast::<u8>(), stride * height) };
            let mut result = [[0; 3]; 4];
            for (destination, (x, y)) in result.iter_mut().zip([
                (width / 4, height / 4),
                (width * 3 / 4, height / 4),
                (width / 4, height * 3 / 4),
                (width * 3 / 4, height * 3 / 4),
            ]) {
                let pixel = &bytes[y * stride + x * 4..y * stride + x * 4 + 4];
                *destination = [pixel[2], pixel[1], pixel[0]];
                ensure!(pixel[3] == 255, "Expected opaque decoded frame");
            }
            Ok(result)
        })();
        ensure!(
            buffer.unlock_base_address(1) == 0,
            "Could not unlock video frame"
        );
        result
    }

    fn near(actual: [u8; 3], expected: [u8; 3]) -> Result<()> {
        ensure!(
            actual
                .iter()
                .zip(expected)
                .all(|(a, b)| (i16::from(*a) - i16::from(b)).abs() < 35),
            "Pixel {actual:?} differs from {expected:?}"
        );
        Ok(())
    }

    fn seek_frame(player: &mut VideoPlayback, position: u64) -> Result<VideoPlaybackFrame> {
        player.seek(position)?;
        wait(
            || Ok(player.status()?.state == VideoPlaybackState::Paused),
            "paused seek completion",
        )?;
        let frame = frame(player)?;
        ensure!(
            frame.presentation_time_us.abs_diff(position) <= 40_000,
            "Seek returned an old frame: {} vs {position}",
            frame.presentation_time_us
        );
        Ok(frame)
    }

    fn playback_advances_pauses_and_seeks() -> Result<()> {
        let mut player = player(CLIP, 128)?;
        ensure!(
            player.info().width == 128
                && player.info().height == 96
                && player.info().duration_us == 3_000_000,
            "Unexpected playback metadata"
        );
        near(colors(&seek_frame(&mut player, 0)?)?[0], [255, 0, 0])?;
        player.play()?;
        let mut timestamps = BTreeSet::new();
        wait(
            || {
                if let VideoFrameUpdate::Frame(frame) =
                    player.frame_for_host_time(video_host_time_seconds())?
                {
                    timestamps.insert(frame.presentation_time_us);
                }
                Ok(player.status()?.current_time_us >= 450_000)
            },
            "video clock advance",
        )?;
        ensure!(
            timestamps.len() >= 3,
            "The player did not supply multiple advancing frames: {timestamps:?}"
        );
        player.pause()?;
        let paused = player.status()?.current_time_us;
        let until = Instant::now() + Duration::from_millis(200);
        while Instant::now() < until {
            tick();
        }
        ensure!(
            player.status()?.current_time_us.abs_diff(paused) < 35_000,
            "Paused video kept advancing"
        );
        near(
            colors(&seek_frame(&mut player, 1_500_000)?)?[0],
            [0, 255, 255],
        )?;
        near(
            colors(&seek_frame(&mut player, 2_500_000)?)?[0],
            [255, 0, 255],
        )?;
        player.play()?;
        wait(
            || Ok(player.status()?.state == VideoPlaybackState::Ended),
            "natural end",
        )?;
        player.play()?;
        wait(
            || {
                let status = player.status()?;
                Ok(status.state == VideoPlaybackState::Playing && status.current_time_us < 500_000)
            },
            "restart after end",
        )?;
        player.pause()?;
        Ok(())
    }

    fn repeated_same_position_seek_can_resume_advancing_frames() -> Result<()> {
        let mut player = player(CLIP, 128)?;
        for _ in 0..3 {
            near(
                colors(&seek_frame(&mut player, 1_200_000)?)?[0],
                [0, 255, 255],
            )?;
        }
        player.play()?;
        let mut timestamps = BTreeSet::new();
        wait(
            || {
                if let VideoFrameUpdate::Frame(frame) =
                    player.frame_for_host_time(video_host_time_seconds())?
                {
                    near(colors(&frame)?[0], [0, 255, 255])?;
                    timestamps.insert(frame.presentation_time_us);
                }
                Ok(player.status()?.current_time_us >= 1_650_000)
            },
            "playback after repeated same-position seeks",
        )?;
        ensure!(
            timestamps.len() >= 3 && timestamps.last().is_some_and(|time| *time > 1_400_000),
            "Repeated seek pinned the frame query: {timestamps:?}"
        );
        player.pause()?;
        near(
            colors(&seek_frame(&mut player, 2_500_000)?)?[0],
            [255, 0, 255],
        )?;
        Ok(())
    }

    fn rapid_seek_keeps_only_the_latest_target() -> Result<()> {
        let mut player = player(CLIP, 128)?;
        player.seek(2_700_000)?;
        player.seek(1_300_000)?;
        player.seek(200_000)?;
        wait(
            || Ok(player.status()?.state == VideoPlaybackState::Paused),
            "latest seek",
        )?;
        let settled = player.status()?;
        ensure!(
            settled.current_time_us.abs_diff(200_000) < 40_000,
            "Latest seek did not settle at its target: {settled:?}"
        );
        let frame = frame(&mut player)?;
        let frame_colors = colors(&frame)?;
        ensure!(
            frame.presentation_time_us.abs_diff(200_000) < 40_000,
            "Obsolete seek won: {} after player settled at {}; quadrants={frame_colors:?}, expected_first=[255, 0, 0]",
            frame.presentation_time_us,
            settled.current_time_us
        );
        near(frame_colors[0], [255, 0, 0])?;
        player.play()?;
        player.seek(1_500_000)?;
        player.pause()?;
        wait(
            || Ok(player.status()?.state == VideoPlaybackState::Paused),
            "pause during seek",
        )?;
        let time = player.status()?.current_time_us;
        ensure!(
            time.abs_diff(1_500_000) < 40_000,
            "Pause lost seek position"
        );
        Ok(())
    }

    fn queued_obsolete_frames_never_escape_a_completed_seek() -> Result<()> {
        let mut player = player(CLIP, 128)?;
        for (obsolete, intermediate, latest, expected) in [
            (2_700_000, 1_300_000, 200_000, [255, 0, 0]),
            (200_000, 2_700_000, 1_500_000, [0, 255, 255]),
            (1_500_000, 200_000, 2_500_000, [255, 0, 255]),
            (2_500_000, 1_500_000, 0, [255, 0, 0]),
        ] {
            player.seek(obsolete)?;
            wait(
                || Ok(player.status()?.state == VideoPlaybackState::Paused),
                "obsolete seek completed without consuming its output",
            )?;
            let old_time = player.status()?.current_time_us;
            ensure!(
                old_time.abs_diff(obsolete) < 40_000,
                "Obsolete-position precondition failed: {old_time} vs {obsolete}"
            );
            // Allow native output delivery while leaving its old frame unread.
            // The next seek must not expose it as its first completed frame.
            tick();
            player.seek(intermediate)?;
            player.seek(latest)?;
            wait(
                || Ok(player.status()?.state == VideoPlaybackState::Paused),
                "coalesced latest seek after unread output",
            )?;
            let settled = player.status()?;
            ensure!(
                settled.current_time_us.abs_diff(latest) < 40_000,
                "Coalesced seek clock lost latest target {latest}: {settled:?}"
            );
            let first = frame(&mut player)?;
            let first_colors = colors(&first)?;
            ensure!(
                first.presentation_time_us.abs_diff(latest) < 40_000,
                "First published frame {} is obsolete after {obsolete} -> {intermediate} -> {latest}; player settled at {}; quadrants={first_colors:?}, expected_first={expected:?}",
                first.presentation_time_us,
                settled.current_time_us
            );
            near(first_colors[0], expected)?;
        }
        let mut player = self::player(QUADRANTS, 128)?;
        for (target, sample_start, sample_end, expected) in [
            (125_000, 0, 250_000, [255, 0, 0]),
            (150_000, 0, 250_000, [255, 0, 0]),
            (175_000, 0, 250_000, [255, 0, 0]),
            (125_000, 0, 250_000, [255, 0, 0]),
            (875_000, 750_000, 1_000_000, [0, 255, 255]),
        ] {
            player.seek(target)?;
            wait(
                || Ok(player.status()?.state == VideoPlaybackState::Paused),
                "seek inside a low-frame-rate sample",
            )?;
            let first = frame(&mut player)?;
            // The compositor may retime a source sample to the requested display
            // position. Its pixels must still belong to the containing sample.
            ensure!(
                (sample_start..sample_end).contains(&first.presentation_time_us)
                    && first.presentation_time_us <= target + 1,
                "Seek returned display PTS {} outside sample [{sample_start}, {sample_end}) or after target {target}",
                first.presentation_time_us
            );
            near(colors(&first)?[0], expected)?;
        }
        Ok(())
    }

    fn playback_preserves_oriented_bounded_pixels() -> Result<()> {
        let palette = [[255, 0, 0], [0, 255, 0], [0, 0, 255], [255, 255, 0]];
        for (bytes, dimensions, expected) in [
            (QUADRANTS, (64, 48), [0, 1, 2, 3]),
            (ROTATE_90, (48, 64), [1, 3, 0, 2]),
            (ROTATE_180, (64, 48), [3, 2, 1, 0]),
            (ROTATE_270, (48, 64), [2, 0, 3, 1]),
            (MIRROR, (64, 48), [1, 0, 3, 2]),
        ] {
            let mut player = player(bytes, 64)?;
            let frame = seek_frame(&mut player, 0)?;
            ensure!(
                (frame.buffer.get_width(), frame.buffer.get_height()) == dimensions,
                "Incorrect oriented dimensions"
            );
            for (actual, index) in colors(&frame)?.into_iter().zip(expected) {
                near(actual, palette[index])?;
            }
        }
        Ok(())
    }

    fn trim_clamps_seeks_pauses_at_end_and_replays_from_start() -> Result<()> {
        let mut player = player(CLIP, 128)?;
        let original = player.info();
        for (start, end) in [(0, 0), (2_000_000, 1_000_000), (0, 3_000_001)] {
            ensure!(
                player.set_time_range(start, end).is_err(),
                "Invalid trim was accepted"
            );
        }
        let start = 1_200_000;
        let end = 2_000_000;
        player.set_time_range(start, end)?;
        wait(
            || Ok(player.status()?.state == VideoPlaybackState::Paused),
            "trim start",
        )?;
        ensure!(player.info() == original, "Trim changed source metadata");
        ensure!(
            player.status()?.duration_us == original.duration_us,
            "Trim lost source duration"
        );
        let first = frame(&mut player)?;
        near(colors(&first)?[0], [0, 255, 255])?;
        ensure!(
            first.presentation_time_us.abs_diff(start) < 40_000,
            "Trim did not begin at its source time"
        );

        for (requested, target) in [(0, start), (original.duration_us, end - 1)] {
            player.seek(requested)?;
            wait(
                || Ok(player.status()?.state == VideoPlaybackState::Paused),
                "clamped trim seek",
            )?;
            let position = player.status()?.current_time_us;
            ensure!(
                position.abs_diff(target) < 40_000,
                "Seek escaped trim: {position} vs {target}"
            );
            let current = frame(&mut player)?;
            ensure!(
                current.presentation_time_us < end,
                "Seek exposed the excluded next scene"
            );
            near(colors(&current)?[0], [0, 255, 255])?;
        }
        near(
            colors(&seek_frame(&mut player, 1_400_000)?)?[0],
            [0, 255, 255],
        )?;
        player.play()?;
        wait(
            || Ok(player.status()?.current_time_us >= 1_500_000),
            "trim playback advance",
        )?;
        player.pause()?;
        let paused = player.status()?.current_time_us;
        let until = Instant::now() + Duration::from_millis(150);
        while Instant::now() < until {
            tick();
        }
        ensure!(
            player.status()?.current_time_us.abs_diff(paused) < 35_000,
            "Trim advanced while paused"
        );
        player.play()?;
        let mut frame_count = 0;
        wait(
            || {
                if let VideoFrameUpdate::Frame(current) =
                    player.frame_for_host_time(video_host_time_seconds())?
                {
                    ensure!(
                        current.presentation_time_us < end,
                        "Playback exposed the excluded next scene"
                    );
                    near(colors(&current)?[0], [0, 255, 255])?;
                    frame_count += 1;
                }
                Ok(player.status()?.state == VideoPlaybackState::Ended)
            },
            "native trim end",
        )?;
        ensure!(frame_count > 0, "Trim supplied no advancing frames");
        ensure!(
            player.status()?.current_time_us.abs_diff(end) <= 1,
            "Native trim did not stop at its end"
        );
        player.play()?;
        wait(
            || {
                let status = player.status()?;
                Ok(status.state == VideoPlaybackState::Playing
                    && status.current_time_us < 1_400_000)
            },
            "trim replay",
        )?;
        let replay = frame(&mut player)?;
        near(colors(&replay)?[0], [0, 255, 255])?;
        ensure!(
            replay.presentation_time_us < 1_400_000,
            "Replay ignored trim start"
        );
        player.pause()?;
        player.set_time_range(0, original.duration_us)?;
        near(
            colors(&seek_frame(&mut player, 2_500_000)?)?[0],
            [255, 0, 255],
        )?;
        ensure!(
            player.info() == original,
            "Restoring the range changed the original video"
        );
        Ok(())
    }

    fn trim_change_replaces_pending_seek_without_leaking_old_frames() -> Result<()> {
        let mut player = player(CLIP, 128)?;
        player.play()?;
        player.seek(2_700_000)?;
        player.seek(200_000)?;
        player.set_time_range(1_100_000, 1_800_000)?;
        wait(
            || Ok(player.status()?.state == VideoPlaybackState::Paused),
            "trim replaced queued seek",
        )?;
        let first = frame(&mut player)?;
        ensure!(
            first.presentation_time_us.abs_diff(1_100_000) < 40_000,
            "Old queued seek escaped new trim"
        );
        near(colors(&first)?[0], [0, 255, 255])?;
        ensure!(
            player.set_time_range(1_800_000, 1_100_000).is_err(),
            "Reversed trim accepted"
        );
        player.seek(0)?;
        wait(
            || Ok(player.status()?.state == VideoPlaybackState::Paused),
            "valid trim survives rejection",
        )?;
        ensure!(
            player.status()?.current_time_us.abs_diff(1_100_000) < 40_000,
            "Rejected trim changed prior bounds"
        );
        near(colors(&frame(&mut player)?)?[0], [0, 255, 255])?;
        Ok(())
    }

    fn trim_inside_long_sample_keeps_containing_pixels() -> Result<()> {
        let mut player = player(QUADRANTS, 128)?;
        player.set_time_range(125_000, 200_000)?;
        wait(
            || Ok(player.status()?.state == VideoPlaybackState::Paused),
            "trim inside low-FPS sample",
        )?;
        let first = frame(&mut player)?;
        ensure!(
            first.presentation_time_us <= 125_001,
            "Trim began after its requested sample"
        );
        near(colors(&first)?[0], [255, 0, 0])?;
        player.play()?;
        wait(
            || {
                if let VideoFrameUpdate::Frame(current) =
                    player.frame_for_host_time(video_host_time_seconds())?
                {
                    ensure!(
                        current.presentation_time_us < 200_000,
                        "Low-FPS trim emitted an excluded frame"
                    );
                    near(colors(&current)?[0], [255, 0, 0])?;
                }
                Ok(player.status()?.state == VideoPlaybackState::Ended)
            },
            "low-FPS native trim end",
        )?;
        ensure!(
            player.status()?.current_time_us.abs_diff(200_000) <= 1,
            "Low-FPS trim overran its end"
        );
        Ok(())
    }

    fn native_failure_and_thread_restriction_surface_errors() -> Result<()> {
        ensure!(
            prepare_video_playback(Arc::from([]), 64).is_err(),
            "Empty input accepted"
        );
        ensure!(
            prepare_video_playback(Arc::from(CLIP), 0).is_err(),
            "Invalid output limit accepted"
        );
        let prepared = prepare(CLIP, 64)?;
        let rejected = std::thread::spawn(move || VideoPlayback::new(prepared).is_err())
            .join()
            .map_err(|_| anyhow::anyhow!("Thread-restriction test panicked"))?;
        ensure!(rejected, "Player accepted a non-main-thread session");
        let error = match prepare(CORRUPT, 64) {
            Err(error) => error,
            Ok(_) => bail!("Corrupt samples were accepted as usable video"),
        };
        ensure!(
            error.to_string().contains("decodable first frame"),
            "Corruption must fail actual source decoding, not a readiness timeout: {error:#}"
        );
        Ok(())
    }

    fn owned_inputs() -> Result<BTreeSet<PathBuf>> {
        let prefix = format!("fanta-video-playback-{}-", std::process::id());
        std::fs::read_dir(std::env::temp_dir())?
            .filter_map(|entry| match entry {
                Ok(entry) if entry.file_name().to_string_lossy().starts_with(&prefix) => {
                    Some(Ok(entry.path()))
                }
                Ok(_) => None,
                Err(error) => Some(Err(error.into())),
            })
            .collect()
    }

    fn close_and_cancel_release_sessions_and_files() -> Result<()> {
        let baseline = owned_inputs()?;
        for _ in 0..3 {
            let request = prepare_video_playback(Arc::from(CLIP), 128)?;
            ensure!(
                owned_inputs()?.len() > baseline.len(),
                "No owned loading input found"
            );
            drop(request);
            wait(
                || Ok(owned_inputs()? == baseline),
                "canceled loading cleanup",
            )?;
            let mut player = player(CLIP, 128)?;
            let frame = seek_frame(&mut player, 0)?;
            player.play()?;
            player.seek(2_000_000)?;
            drop(player);
            // A retained output frame must remain usable after the player is gone.
            near(colors(&frame)?[0], [255, 0, 0])?;
            drop(frame);
            wait(
                || Ok(owned_inputs()? == baseline),
                "closed playback cleanup",
            )?;
        }
        Ok(())
    }

    pub fn run() -> Result<()> {
        if std::env::var_os("FANTA_VIDEO_SEEK_TRACE").as_deref() == Some(std::ffi::OsStr::new("1"))
        {
            writeln!(
                std::io::stderr().lock(),
                "FANTA_VIDEO_SEEK_TRACE=1: native seek/frame diagnostics enabled, at most 128 records per player"
            )?;
        }
        let cases: &[(&str, fn() -> Result<()>)] = &[
            (
                "repeated_same_position_seek_can_resume_advancing_frames",
                repeated_same_position_seek_can_resume_advancing_frames,
            ),
            (
                "playback_advances_pauses_and_seeks",
                playback_advances_pauses_and_seeks,
            ),
            (
                "rapid_seek_keeps_only_the_latest_target",
                rapid_seek_keeps_only_the_latest_target,
            ),
            (
                "queued_obsolete_frames_never_escape_a_completed_seek",
                queued_obsolete_frames_never_escape_a_completed_seek,
            ),
            (
                "playback_preserves_oriented_bounded_pixels",
                playback_preserves_oriented_bounded_pixels,
            ),
            (
                "trim_clamps_seeks_pauses_at_end_and_replays_from_start",
                trim_clamps_seeks_pauses_at_end_and_replays_from_start,
            ),
            (
                "trim_change_replaces_pending_seek_without_leaking_old_frames",
                trim_change_replaces_pending_seek_without_leaking_old_frames,
            ),
            (
                "trim_inside_long_sample_keeps_containing_pixels",
                trim_inside_long_sample_keeps_containing_pixels,
            ),
            (
                "native_failure_and_thread_restriction_surface_errors",
                native_failure_and_thread_restriction_surface_errors,
            ),
            (
                "close_and_cancel_release_sessions_and_files",
                close_and_cancel_release_sessions_and_files,
            ),
        ];
        let mut failures = Vec::new();
        for (name, test) in cases {
            let result = std::panic::catch_unwind(*test)
                .unwrap_or_else(|_| Err(anyhow::anyhow!("Native case panicked")));
            match result {
                Ok(()) => println!("PASS {name}"),
                Err(error) => {
                    eprintln!("FAIL {name}: {error:#}");
                    failures.push(*name);
                }
            }
        }
        if !failures.is_empty() {
            bail!(
                "{} native playback cases failed: {failures:?}",
                failures.len()
            );
        }
        wait(
            || Ok(owned_inputs()?.is_empty()),
            "final native input cleanup",
        )?;
        println!("All {} native playback cases passed", cases.len());
        Ok(())
    }
}

#[cfg(target_os = "macos")]
fn main() -> anyhow::Result<()> {
    native::run()
}

#[cfg(not(target_os = "macos"))]
fn main() {
    println!("Native AVFoundation playback tests require macOS.");
}
