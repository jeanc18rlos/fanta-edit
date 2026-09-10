use anyhow::{Context as _, Result, anyhow, bail, ensure};
use block::ConcreteBlock;
use core_foundation::{base::TCFType, string::CFString};
use core_video::pixel_buffer::{CVPixelBuffer, CVPixelBufferRef, kCVPixelFormatType_32BGRA};
use futures::channel::oneshot;
use objc::{
    Encode, Encoding, class, msg_send,
    rc::{StrongPtr, autoreleasepool},
    runtime::{BOOL, NO, Object, YES},
    sel, sel_impl,
};
use std::{
    ffi::c_void,
    future::Future,
    io::Write,
    marker::PhantomData,
    panic::{AssertUnwindSafe, catch_unwind},
    pin::Pin,
    rc::Rc,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    task::{Context, Poll},
};

const MAX_INPUT_BYTES: usize = 100 * 1024 * 1024;
const MAX_DIMENSION: u32 = 2048;

const ASSET_KEYS: &[&str] = &["tracks", "duration", "playable", "hasProtectedContent"];
const TRACK_KEYS: &[&str] = &[
    "naturalSize",
    "preferredTransform",
    "nominalFrameRate",
    "formatDescriptions",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VideoPlaybackInfo {
    pub width: u32,
    pub height: u32,
    pub duration_us: u64,
}

pub struct PreparedVideoPlayback {
    asset: Option<StrongPtr>,
    track: Option<StrongPtr>,
    input: Arc<tempfile::TempPath>,
    info: VideoPlaybackInfo,
    transform: NativeTransform,
    frame_duration: NativeTime,
    duration: NativeTime,
}

// Preparation contains only immutable, asynchronously loaded AVAsset/AVAssetTrack
// objects. AVPlayer itself is created later on the consuming UI thread.
unsafe impl Send for PreparedVideoPlayback {}

impl PreparedVideoPlayback {
    pub fn info(&self) -> VideoPlaybackInfo {
        self.info
    }
}

impl Drop for PreparedVideoPlayback {
    fn drop(&mut self) {
        autoreleasepool(|| {
            drop(self.track.take());
            drop(self.asset.take());
        });
    }
}

pub struct VideoPlaybackRequest {
    asset: Option<StrongPtr>,
    track: Option<StrongPtr>,
    input: Arc<tempfile::TempPath>,
    loaded: oneshot::Receiver<()>,
    maximum_dimension: u32,
    finished: bool,
    sample_check: Option<VideoFrameRequest>,
    prepared: Option<PreparedVideoPlayback>,
}

// This moves immutable asset loading state and the same cancellable image
// generator used for posters. No player or native UI object crosses threads.
unsafe impl Send for VideoPlaybackRequest {}

impl Drop for VideoPlaybackRequest {
    fn drop(&mut self) {
        drop(self.sample_check.take());
        drop(self.prepared.take());
        autoreleasepool(|| unsafe {
            if !self.finished
                && let Some(asset) = &self.asset
            {
                let _: () = msg_send![**asset, cancelLoading];
            }
            drop(self.track.take());
            drop(self.asset.take());
        });
    }
}

impl Future for VideoPlaybackRequest {
    type Output = Result<PreparedVideoPlayback>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        if self.finished {
            return Poll::Ready(Err(anyhow!("Video preparation was already consumed.")));
        }
        loop {
            if let Some(check) = self.sample_check.as_mut() {
                match Pin::new(check).poll(cx) {
                    Poll::Pending => return Poll::Pending,
                    Poll::Ready(Err(error)) => {
                        return Poll::Ready(Err(
                            error.context("The video has no decodable first frame.")
                        ));
                    }
                    Poll::Ready(Ok(frame)) => {
                        drop(frame);
                        self.finished = true;
                        drop(self.sample_check.take());
                        return Poll::Ready(
                            self.prepared
                                .take()
                                .context("The prepared video was released."),
                        );
                    }
                }
            }
            match Pin::new(&mut self.loaded).poll(cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Err(_)) => {
                    return Poll::Ready(Err(anyhow!(
                        "Video preparation stopped before completion."
                    )));
                }
                Poll::Ready(Ok(())) => {}
            }
            let step = autoreleasepool(|| unsafe { self.finish_loading_step() });
            match step {
                Ok(Some(prepared)) => {
                    // The player compositor can replace corrupt samples with black and
                    // report normal completion. Verify one source sample before composing.
                    let check = prepared
                        .asset
                        .as_ref()
                        .context("The video asset was released.")
                        .and_then(|asset| asset_frame(asset, prepared.input.clone(), 64));
                    match check {
                        Ok(check) => {
                            self.sample_check = Some(check);
                            self.prepared = Some(prepared);
                        }
                        Err(error) => return Poll::Ready(Err(error)),
                    }
                }
                Ok(None) => {}
                Err(error) => return Poll::Ready(Err(error)),
            }
        }
    }
}

impl VideoPlaybackRequest {
    unsafe fn finish_loading_step(&mut self) -> Result<Option<PreparedVideoPlayback>> {
        let asset = self.asset.as_ref().context("Video asset was released.")?;
        if self.track.is_none() {
            check_loaded(**asset, ASSET_KEYS)?;
            let playable: BOOL = unsafe { msg_send![**asset, isPlayable] };
            let protected: BOOL = unsafe { msg_send![**asset, hasProtectedContent] };
            ensure!(
                playable == YES && protected == NO,
                "This video cannot be played locally."
            );
            let tracks: *mut Object =
                unsafe { msg_send![**asset, tracksWithMediaType:AVMediaTypeVideo] };
            ensure!(!tracks.is_null(), "The video has no playable track.");
            let count: usize = unsafe { msg_send![tracks, count] };
            ensure!(
                count == 1,
                "Video preview requires exactly one video track."
            );
            let track: *mut Object = unsafe { msg_send![tracks, objectAtIndex:0usize] };
            ensure!(!track.is_null(), "The video track could not be loaded.");
            self.track = Some(unsafe { StrongPtr::retain(track) });
            self.loaded = load_native_keys(track, TRACK_KEYS, self.input.clone());
            return Ok(None);
        }
        let track = self.track.as_ref().context("Video track was released.")?;
        check_loaded(**track, TRACK_KEYS)?;
        let duration: NativeTime = unsafe { msg_send![**asset, duration] };
        let duration_us = time_us(duration)?;
        ensure!(
            duration_us > 0 && duration_us <= 86_400_000_000,
            "Video duration is invalid or exceeds 24 hours."
        );
        let descriptions: *mut Object = unsafe { msg_send![**track, formatDescriptions] };
        ensure!(
            !descriptions.is_null(),
            "The video has no format description."
        );
        let count: usize = unsafe { msg_send![descriptions, count] };
        ensure!(
            count == 1,
            "Videos that change format during playback are unsupported."
        );
        let description: *const c_void = unsafe { msg_send![descriptions, objectAtIndex:0usize] };
        ensure!(
            !description.is_null(),
            "The video format could not be read."
        );
        let size = unsafe { CMVideoFormatDescriptionGetPresentationDimensions(description, 1, 0) };
        let transform: NativeTransform = unsafe { msg_send![**track, preferredTransform] };
        let (info, transform) =
            playback_geometry(size, transform, self.maximum_dimension, duration_us)?;
        let frame_rate: f32 = unsafe { msg_send![**track, nominalFrameRate] };
        ensure!(
            frame_rate.is_finite() && frame_rate >= 0. && frame_rate <= 120.,
            "Video frame rate exceeds the preview limit."
        );
        let frame_duration = NativeTime {
            value: 1,
            timescale: frame_rate.round().clamp(1., 60.) as i32,
            flags: 1,
            epoch: 0,
        };
        Ok(Some(PreparedVideoPlayback {
            asset: self.asset.take(),
            track: self.track.take(),
            input: self.input.clone(),
            info,
            transform,
            frame_duration,
            duration,
        }))
    }
}

/// Writes the local input on the caller's thread, then loads metadata and checks one source frame asynchronously.
/// Call this constructor on a background executor and impose a loading deadline.
pub fn prepare_video_playback(
    bytes: Arc<[u8]>,
    maximum_dimension: u32,
) -> Result<VideoPlaybackRequest> {
    ensure!(
        !bytes.is_empty() && bytes.len() <= MAX_INPUT_BYTES,
        "Choose a video smaller than 100 MiB."
    );
    ensure!(
        (1..=MAX_DIMENSION).contains(&maximum_dimension),
        "Video preview dimensions must be between 1 and 2048 pixels."
    );
    let mut input = tempfile::Builder::new()
        .prefix(&format!("fanta-video-playback-{}-", std::process::id()))
        .suffix(".mp4")
        .tempfile()?;
    input
        .write_all(&bytes)
        .context("Could not prepare the video for playback.")?;
    input.flush()?;
    let input = Arc::new(input.into_temp_path());
    autoreleasepool(|| unsafe {
        let path = CFString::new(
            input
                .to_str()
                .context("The temporary video path is not valid text.")?,
        );
        let url: *mut Object =
            msg_send![class!(NSURL), fileURLWithPath:path.as_concrete_TypeRef() as *mut Object];
        ensure!(!url.is_null(), "Could not create the local video URL.");
        let restrictions: *mut Object =
            msg_send![class!(NSNumber), numberWithUnsignedInteger:0xffffusize];
        ensure!(
            !restrictions.is_null(),
            "Could not restrict video references."
        );
        let options: *mut Object = msg_send![class!(NSDictionary), dictionaryWithObject:restrictions forKey:AVURLAssetReferenceRestrictionsKey];
        ensure!(!options.is_null(), "Could not configure video playback.");
        let asset: *mut Object = msg_send![class!(AVURLAsset), alloc];
        let asset: *mut Object = msg_send![asset, initWithURL:url options:options];
        ensure!(!asset.is_null(), "Could not open the local video asset.");
        let asset = StrongPtr::new(asset);
        let loaded = load_native_keys(*asset, ASSET_KEYS, input.clone());
        Ok(VideoPlaybackRequest {
            asset: Some(asset),
            track: None,
            input,
            loaded,
            maximum_dimension,
            finished: false,
            sample_check: None,
            prepared: None,
        })
    })
}

fn load_native_keys(
    object: *mut Object,
    keys: &[&str],
    input: Arc<tempfile::TempPath>,
) -> oneshot::Receiver<()> {
    let keys = core_foundation::array::CFArray::from_CFTypes(
        &keys
            .iter()
            .map(|key| CFString::new(key))
            .collect::<Vec<_>>(),
    );
    let (sender, receiver) = oneshot::channel();
    let sender = Mutex::new(Some(sender));
    let callback = ConcreteBlock::new(move || {
        let _input = &input;
        let mut sender = sender
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(sender) = sender.take()
            && sender.send(()).is_err()
        {
            // Dropping the loading future deliberately abandons this completion.
        }
    })
    .copy();
    unsafe {
        let _: () = msg_send![object, loadValuesAsynchronouslyForKeys:keys.as_concrete_TypeRef() as *mut Object completionHandler:&*callback];
    }
    receiver
}

fn check_loaded(object: *mut Object, keys: &[&str]) -> Result<()> {
    for key in keys {
        let name = CFString::new(key);
        let mut error: *mut Object = std::ptr::null_mut();
        let status: isize = unsafe {
            msg_send![object, statusOfValueForKey:name.as_concrete_TypeRef() as *mut Object error:&mut error]
        };
        ensure!(
            status == 2,
            "Could not load video {key}: {}",
            error_description(error)
        );
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VideoPlaybackState {
    Loading,
    Paused,
    Playing,
    Seeking,
    Ended,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VideoPlaybackStatus {
    pub state: VideoPlaybackState,
    pub current_time_us: u64,
    pub duration_us: u64,
}

pub struct VideoPlaybackFrame {
    pub buffer: CVPixelBuffer,
    pub presentation_time_us: u64,
}

pub enum VideoFrameUpdate {
    Unchanged,
    Empty,
    Frame(VideoPlaybackFrame),
}

pub struct VideoPlayback {
    player: Option<StrongPtr>,
    item: Option<StrongPtr>,
    output: Option<StrongPtr>,
    prepared: PreparedVideoPlayback,
    seek_completion: Arc<Mutex<Option<bool>>>,
    pending_seek: Option<u64>,
    seeking: bool,
    requested_play: bool,
    _thread_confined: PhantomData<Rc<()>>,
}

impl VideoPlayback {
    /// Create and use the session on a single UI thread. It owns no frame queue;
    /// callers retain only the currently displayed frame and discard replaced ones.
    pub fn new(prepared: PreparedVideoPlayback) -> Result<Self> {
        autoreleasepool(|| unsafe {
            let main_thread: BOOL = msg_send![class!(NSThread), isMainThread];
            ensure!(
                main_thread == YES,
                "Create video playback on the main UI thread."
            );
            let asset = prepared
                .asset
                .as_ref()
                .context("The prepared video was released.")?;
            let track = prepared
                .track
                .as_ref()
                .context("The prepared track was released.")?;
            let item: *mut Object = msg_send![class!(AVPlayerItem), alloc];
            let item: *mut Object = msg_send![item, initWithAsset:**asset];
            ensure!(!item.is_null(), "Could not create video playback.");
            let item = StrongPtr::new(item);
            let composition: *mut Object =
                msg_send![class!(AVMutableVideoComposition), videoComposition];
            let instruction: *mut Object = msg_send![
                class!(AVMutableVideoCompositionInstruction),
                videoCompositionInstruction
            ];
            let layer: *mut Object = msg_send![class!(AVMutableVideoCompositionLayerInstruction), videoCompositionLayerInstructionWithAssetTrack:**track];
            ensure!(
                !composition.is_null() && !instruction.is_null() && !layer.is_null(),
                "Could not prepare oriented video playback."
            );
            let _: () = msg_send![layer, setTransform:prepared.transform atTime:NativeTime::ZERO];
            let layers: *mut Object = msg_send![class!(NSArray), arrayWithObject:layer];
            ensure!(!layers.is_null(), "Could not prepare the video layer.");
            let _: () = msg_send![instruction, setLayerInstructions:layers];
            let _: () = msg_send![instruction, setTimeRange:NativeTimeRange { start: NativeTime::ZERO, duration: prepared.duration }];
            let instructions: *mut Object = msg_send![class!(NSArray), arrayWithObject:instruction];
            ensure!(
                !instructions.is_null(),
                "Could not prepare video composition instructions."
            );
            let _: () = msg_send![composition, setInstructions:instructions];
            let _: () = msg_send![composition, setRenderSize:NativeSize { width: prepared.info.width as f64, height: prepared.info.height as f64 }];
            let _: () = msg_send![composition, setFrameDuration:prepared.frame_duration];
            let track_id: i32 = msg_send![**track, trackID];
            let _: () = msg_send![composition, setSourceTrackIDForFrameTiming:track_id];
            let _: () = msg_send![*item, setVideoComposition:composition];
            let attributes: *mut Object = msg_send![class!(NSMutableDictionary), dictionary];
            ensure!(!attributes.is_null(), "Could not configure video frames.");
            let format: *mut Object =
                msg_send![class!(NSNumber), numberWithUnsignedInt:kCVPixelFormatType_32BGRA];
            let compatible: *mut Object = msg_send![class!(NSNumber), numberWithBool:YES];
            let surface: *mut Object = msg_send![class!(NSDictionary), dictionary];
            ensure!(
                !format.is_null() && !compatible.is_null() && !surface.is_null(),
                "Could not configure video frame storage."
            );
            let _: () =
                msg_send![attributes, setObject:format forKey:kCVPixelBufferPixelFormatTypeKey];
            let _: () = msg_send![attributes, setObject:compatible forKey:kCVPixelBufferMetalCompatibilityKey];
            let _: () = msg_send![attributes, setObject:surface forKey:kCVPixelBufferIOSurfacePropertiesKey];
            let output: *mut Object = msg_send![class!(AVPlayerItemVideoOutput), alloc];
            let output: *mut Object = msg_send![output, initWithPixelBufferAttributes:attributes];
            ensure!(
                !output.is_null(),
                "Could not initialize video frame output."
            );
            let output = StrongPtr::new(output);
            let _: () = msg_send![*output, setSuppressesPlayerRendering:YES];
            let _: () = msg_send![*item, addOutput:*output];
            let player: *mut Object = msg_send![class!(AVPlayer), alloc];
            let player: *mut Object = msg_send![player, initWithPlayerItem:*item];
            ensure!(!player.is_null(), "Could not initialize the video player.");
            let player = StrongPtr::new(player);
            Ok(Self {
                player: Some(player),
                item: Some(item),
                output: Some(output),
                prepared,
                seek_completion: Arc::new(Mutex::new(None)),
                pending_seek: None,
                seeking: false,
                requested_play: false,
                _thread_confined: PhantomData,
            })
        })
    }

    pub fn info(&self) -> VideoPlaybackInfo {
        self.prepared.info
    }

    pub fn play(&mut self) -> Result<()> {
        let status = self.status()?;
        self.requested_play = true;
        if status.state == VideoPlaybackState::Ended {
            self.pending_seek = Some(0);
        }
        self.advance_seek()?;
        if !self.seeking {
            autoreleasepool(|| unsafe {
                let _: () = msg_send![self.player()?, play];
                Ok::<_, anyhow::Error>(())
            })?;
        }
        Ok(())
    }

    pub fn pause(&mut self) -> Result<()> {
        self.requested_play = false;
        autoreleasepool(|| unsafe {
            let _: () = msg_send![self.player()?, pause];
            Ok(())
        })
    }

    pub fn set_audio(&mut self, muted: bool, volume: f32) -> Result<()> {
        ensure!(
            volume.is_finite() && (0. ..=1.).contains(&volume),
            "Video volume must be between zero and one."
        );
        autoreleasepool(|| unsafe {
            let player = self.player()?;
            let _: () = msg_send![player, setMuted:if muted { YES } else { NO }];
            let _: () = msg_send![player, setVolume:volume];
            Ok(())
        })
    }

    /// Only one native seek runs at a time. Repeated requests replace its queued
    /// successor; status/frame polling applies the latest completion on this thread.
    pub fn seek(&mut self, time_us: u64) -> Result<()> {
        ensure!(
            time_us <= self.prepared.info.duration_us,
            "The requested position is outside this video."
        );
        self.pending_seek = Some(time_us);
        self.advance_seek()
    }

    pub fn status(&mut self) -> Result<VideoPlaybackStatus> {
        self.advance_seek()?;
        autoreleasepool(|| unsafe {
            let player = self.player()?;
            let item = self.item.as_ref().context("The video player was closed.")?;
            let item_status: isize = msg_send![**item, status];
            let player_status: isize = msg_send![player, status];
            if item_status == 2 || player_status == 2 {
                let error: *mut Object = if item_status == 2 {
                    msg_send![**item, error]
                } else {
                    msg_send![player, error]
                };
                bail!("Video playback failed: {}", error_description(error));
            }
            let current: NativeTime = msg_send![player, currentTime];
            let current_time_us = if item_status == 0 {
                0
            } else {
                time_us(current)?.min(self.prepared.info.duration_us)
            };
            let control: isize = msg_send![player, timeControlStatus];
            let state = if self.seeking {
                VideoPlaybackState::Seeking
            } else if current_time_us >= self.prepared.info.duration_us {
                self.requested_play = false;
                VideoPlaybackState::Ended
            } else if item_status == 0 || (self.requested_play && control != 2) {
                VideoPlaybackState::Loading
            } else if control == 2 {
                VideoPlaybackState::Playing
            } else {
                VideoPlaybackState::Paused
            };
            Ok(VideoPlaybackStatus {
                state,
                current_time_us,
                duration_us: self.prepared.info.duration_us,
            })
        })
    }

    pub fn frame_for_host_time(&mut self, host_time: f64) -> Result<VideoFrameUpdate> {
        ensure!(
            host_time.is_finite() && host_time >= 0.,
            "The video display timestamp is invalid."
        );
        self.status()?;
        if self.seeking {
            return Ok(VideoFrameUpdate::Unchanged);
        }
        autoreleasepool(|| unsafe {
            let output = self
                .output
                .as_ref()
                .context("The video player was closed.")?;
            let requested: NativeTime = msg_send![**output, itemTimeForHostTime:host_time];
            if requested.flags & 1 == 0 {
                return Ok(VideoFrameUpdate::Unchanged);
            }
            let available: BOOL = msg_send![**output, hasNewPixelBufferForItemTime:requested];
            if available != YES {
                return Ok(VideoFrameUpdate::Unchanged);
            }
            let mut presentation = NativeTime::ZERO;
            let buffer: CVPixelBufferRef = msg_send![**output, copyPixelBufferForItemTime:requested itemTimeForDisplay:&mut presentation];
            if buffer.is_null() {
                return Ok(VideoFrameUpdate::Empty);
            }
            let buffer = CVPixelBuffer::wrap_under_create_rule(buffer);
            ensure!(
                buffer.get_pixel_format() == kCVPixelFormatType_32BGRA
                    && buffer.get_width() == self.prepared.info.width as usize
                    && buffer.get_height() == self.prepared.info.height as usize,
                "The video player returned an unexpected frame format or size."
            );
            let presentation_time_us = time_us(presentation)?;
            ensure!(
                presentation_time_us <= self.prepared.info.duration_us,
                "The video player returned an invalid frame time."
            );
            Ok(VideoFrameUpdate::Frame(VideoPlaybackFrame {
                buffer,
                presentation_time_us,
            }))
        })
    }

    fn player(&self) -> Result<*mut Object> {
        self.player
            .as_ref()
            .map(|player| **player)
            .context("The video player was closed.")
    }

    fn advance_seek(&mut self) -> Result<()> {
        let mut completed = false;
        if self.seeking {
            let completion = self
                .seek_completion
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .take();
            let Some(finished) = completion else {
                return Ok(());
            };
            self.seeking = false;
            ensure!(
                finished || self.pending_seek.is_some(),
                "The video seek was interrupted. Try seeking again."
            );
            completed = true;
        }
        if let Some(target) = self.pending_seek.take() {
            let completion = Arc::downgrade(&self.seek_completion);
            let input = self.prepared.input.clone();
            let callback = ConcreteBlock::new(move |finished: BOOL| {
                let _input = &input;
                if let Some(completion) = completion.upgrade() {
                    *completion
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(finished == YES);
                }
            })
            .copy();
            autoreleasepool(|| unsafe {
                let player = self.player()?;
                let _: () = msg_send![player, pause];
                let time = NativeTime {
                    value: target as i64,
                    timescale: 1_000_000,
                    flags: 1,
                    epoch: 0,
                };
                let _: () = msg_send![player, seekToTime:time toleranceBefore:NativeTime::ZERO toleranceAfter:NativeTime::ZERO completionHandler:&*callback];
                Ok::<_, anyhow::Error>(())
            })?;
            self.seeking = true;
        } else if completed && self.requested_play {
            autoreleasepool(|| unsafe {
                let _: () = msg_send![self.player()?, play];
                Ok::<_, anyhow::Error>(())
            })?;
        }
        Ok(())
    }
}

impl Drop for VideoPlayback {
    fn drop(&mut self) {
        autoreleasepool(|| unsafe {
            if let Some(player) = &self.player {
                let _: () = msg_send![**player, pause];
            }
            if let Some(item) = &self.item {
                let _: () = msg_send![**item, cancelPendingSeeks];
                if let Some(output) = &self.output {
                    let _: () = msg_send![**item, removeOutput:**output];
                }
            }
            if let Some(player) = &self.player {
                let _: () = msg_send![**player, replaceCurrentItemWithPlayerItem:std::ptr::null_mut::<Object>()];
            }
            drop(self.player.take());
            drop(self.output.take());
            drop(self.item.take());
        });
    }
}

pub fn video_host_time_seconds() -> f64 {
    unsafe { CACurrentMediaTime() }
}

fn time_us(time: NativeTime) -> Result<u64> {
    ensure!(
        time.flags & 1 != 0
            && time.flags & (4 | 8 | 16) == 0
            && time.timescale > 0
            && time.epoch == 0
            && time.value >= 0,
        "The video returned an invalid timestamp."
    );
    Ok(u64::try_from(
        i128::from(time.value) * 1_000_000 / i128::from(time.timescale),
    )?)
}

fn playback_geometry(
    size: NativeSize,
    transform: NativeTransform,
    maximum: u32,
    duration_us: u64,
) -> Result<(VideoPlaybackInfo, NativeTransform)> {
    ensure!(
        size.width.is_finite()
            && size.height.is_finite()
            && size.width > 0.
            && size.height > 0.
            && size.width <= 8192.
            && size.height <= 8192.
            && size.width * size.height <= 33_554_432.,
        "Video source dimensions exceed the preview limit."
    );
    let NativeTransform { a, b, c, d, tx, ty } = transform;
    ensure!(
        [a, b, c, d, tx, ty].iter().all(|value| value.is_finite())
            && [a, b, c, d]
                .iter()
                .all(|value| [0., 1., -1.].contains(value))
            && (a * d - b * c).abs() == 1.
            && a * b == 0.
            && c * d == 0.
            && tx.abs() <= 1e9
            && ty.abs() <= 1e9,
        "Video preview supports only quarter-turn rotations and reflections."
    );
    let points = [
        (0., 0.),
        (size.width, 0.),
        (0., size.height),
        (size.width, size.height),
    ]
    .map(|(x, y)| (a * x + c * y + tx, b * x + d * y + ty));
    let min_x = points
        .iter()
        .map(|point| point.0)
        .fold(f64::INFINITY, f64::min);
    let min_y = points
        .iter()
        .map(|point| point.1)
        .fold(f64::INFINITY, f64::min);
    let width = points
        .iter()
        .map(|point| point.0)
        .fold(f64::NEG_INFINITY, f64::max)
        - min_x;
    let height = points
        .iter()
        .map(|point| point.1)
        .fold(f64::NEG_INFINITY, f64::max)
        - min_y;
    let scale = (f64::from(maximum) / width.max(height)).min(1.);
    Ok((
        VideoPlaybackInfo {
            width: (width * scale).ceil().min(f64::from(maximum)) as u32,
            height: (height * scale).ceil().min(f64::from(maximum)) as u32,
            duration_us,
        },
        NativeTransform {
            a: a * scale,
            b: b * scale,
            c: c * scale,
            d: d * scale,
            tx: (tx - min_x) * scale,
            ty: (ty - min_y) * scale,
        },
    ))
}

#[derive(Debug)]
pub struct VideoFrame {
    /// Straight-alpha RGBA pixels, with the first row at the top of the image.
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub actual_time_us: u64,
}

pub struct VideoFrameRequest {
    receiver: oneshot::Receiver<Result<VideoFrame>>,
    generator: Option<StrongPtr>,
    canceled: Arc<AtomicBool>,
    _input: Arc<tempfile::TempPath>,
}

// AVAssetImageGenerator does its decoding on its own queue. The request's sole
// native operation after construction is cancellation; its callback never accesses this pointer.
unsafe impl Send for VideoFrameRequest {}

impl Future for VideoFrameRequest {
    type Output = Result<VideoFrame>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match Pin::new(&mut self.receiver).poll(cx) {
            Poll::Ready(result) => {
                if let Some(generator) = self.generator.take() {
                    autoreleasepool(|| drop(generator));
                }
                Poll::Ready(result.unwrap_or_else(|_| {
                    Err(anyhow!(
                        "The video decoder stopped without returning a frame."
                    ))
                }))
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

impl Drop for VideoFrameRequest {
    fn drop(&mut self) {
        self.canceled.store(true, Ordering::Release);
        if let Some(generator) = self.generator.take() {
            autoreleasepool(|| unsafe {
                let _: () = msg_send![*generator, cancelAllCGImageGeneration];
                drop(generator);
            });
        }
    }
}

/// Starts decoding the first frame of a local video. Call on a background executor:
/// creating the temporary input writes up to 100 MiB. Dropping the returned future
/// cancels decoding. Callers should impose their own deadline.
pub fn video_frame(bytes: Arc<[u8]>, maximum_dimension: u32) -> Result<VideoFrameRequest> {
    ensure!(
        !bytes.is_empty() && bytes.len() <= MAX_INPUT_BYTES,
        "Choose a video smaller than 100 MiB."
    );
    ensure!(
        (1..=MAX_DIMENSION).contains(&maximum_dimension),
        "Video preview dimensions must be between 1 and 2048 pixels."
    );
    let mut input = tempfile::Builder::new()
        .prefix("fanta-video-poster-")
        .suffix(".mp4")
        .tempfile()?;
    input
        .write_all(&bytes)
        .context("Could not prepare the video for decoding.")?;
    input.flush()?;
    let input = Arc::new(input.into_temp_path());
    autoreleasepool(|| unsafe {
        let path = input
            .to_str()
            .context("The temporary video path is not valid text.")?;
        let path = CFString::new(path);
        let path_pointer = path.as_concrete_TypeRef() as *mut Object;
        let url: *mut Object = msg_send![class!(NSURL), fileURLWithPath:path_pointer];
        ensure!(!url.is_null(), "Could not create the local video URL.");
        // Downloaded containers must never resolve additional files or network resources.
        let restrictions: *mut Object =
            msg_send![class!(NSNumber), numberWithUnsignedInteger:0xffffusize];
        ensure!(
            !restrictions.is_null(),
            "Could not configure local video reference restrictions."
        );
        let options: *mut Object = msg_send![class!(NSDictionary), dictionaryWithObject:restrictions forKey:AVURLAssetReferenceRestrictionsKey];
        ensure!(
            !options.is_null(),
            "Could not configure local video decoding."
        );
        let asset: *mut Object = msg_send![class!(AVURLAsset), alloc];
        let asset: *mut Object = msg_send![asset, initWithURL:url options:options];
        ensure!(!asset.is_null(), "Could not open the local video asset.");
        let asset = StrongPtr::new(asset);
        asset_frame(&asset, input.clone(), maximum_dimension)
    })
}

fn asset_frame(
    asset: &StrongPtr,
    input: Arc<tempfile::TempPath>,
    maximum_dimension: u32,
) -> Result<VideoFrameRequest> {
    let canceled = Arc::new(AtomicBool::new(false));
    let (sender, receiver) = oneshot::channel();
    let sender = Mutex::new(Some(sender));
    let generator = autoreleasepool(|| unsafe {
        let generator: *mut Object = msg_send![class!(AVAssetImageGenerator), alloc];
        let generator: *mut Object = msg_send![generator, initWithAsset:**asset];
        ensure!(
            !generator.is_null(),
            "Could not initialize the video decoder."
        );
        let generator = StrongPtr::new(generator);
        let _: () = msg_send![*generator, setAppliesPreferredTrackTransform:YES];
        let size = NativeSize {
            width: maximum_dimension as f64,
            height: maximum_dimension as f64,
        };
        let _: () = msg_send![*generator, setMaximumSize:size];
        let _: () = msg_send![*generator, setRequestedTimeToleranceBefore:NativeTime::ZERO];
        let _: () = msg_send![*generator, setRequestedTimeToleranceAfter:NativeTime::ZERO];
        let time: *mut Object = msg_send![class!(NSValue), valueWithCMTime:NativeTime::ZERO];
        ensure!(!time.is_null(), "Could not request a video frame time.");
        let times: *mut Object = msg_send![class!(NSArray), arrayWithObject:time];
        ensure!(!times.is_null(), "Could not request the video frame.");
        let callback = ConcreteBlock::new({
            let input = input.clone();
            let canceled = canceled.clone();
            move |_requested: NativeTime,
                  image: *const c_void,
                  actual: NativeTime,
                  status: isize,
                  error: *mut Object| {
                // The framework's copied callback keeps the file alive through completion/cancellation,
                // without retaining its generator and introducing a callback ownership cycle.
                let _input = &input;
                let result = catch_unwind(AssertUnwindSafe(|| {
                    autoreleasepool(|| {
                        if canceled.load(Ordering::Acquire) || status == 2 {
                            bail!("Video preview decoding was canceled.");
                        }
                        if status != 0 || image.is_null() {
                            bail!(
                                "The video could not be decoded: {}",
                                error_description(error)
                            );
                        }
                        copy_frame(image, actual, maximum_dimension)
                    })
                }))
                .unwrap_or_else(|_| Err(anyhow!("The video decoder could not copy its frame.")));
                let mut sender = match sender.lock() {
                    Ok(sender) => sender,
                    Err(poisoned) => poisoned.into_inner(),
                };
                if let Some(sender) = sender.take() {
                    if let Err(undelivered) = sender.send(result) {
                        // A dropped request deliberately abandons its result, including any allocated pixels.
                        drop(undelivered);
                    }
                }
            }
        })
        .copy();
        // msg_send passes arguments by value; borrow the block so Rust still releases its copied owner.
        let _: () = msg_send![*generator, generateCGImagesAsynchronouslyForTimes:times completionHandler:&*callback];
        Ok::<_, anyhow::Error>(generator)
    })?;
    Ok(VideoFrameRequest {
        receiver,
        generator: Some(generator),
        canceled,
        _input: input,
    })
}

fn error_description(error: *mut Object) -> String {
    if error.is_null() {
        return "No usable video frame was returned.".into();
    }
    unsafe {
        let description: *mut Object = msg_send![error, localizedDescription];
        if description.is_null() {
            return "The media framework rejected this video.".into();
        }
        CFString::wrap_under_get_rule(description.cast())
            .to_string()
            .chars()
            .take(600)
            .collect()
    }
}

fn copy_frame(
    image: *const c_void,
    time: NativeTime,
    maximum_dimension: u32,
) -> Result<VideoFrame> {
    ensure!(
        time.flags & 1 != 0
            && time.flags & (4 | 8 | 16) == 0
            && time.timescale > 0
            && time.epoch == 0
            && time.value >= 0,
        "The decoder returned an invalid frame timestamp."
    );
    let actual_time_us =
        u64::try_from(i128::from(time.value) * 1_000_000 / i128::from(time.timescale))?;
    unsafe {
        let width = CGImageGetWidth(image);
        let height = CGImageGetHeight(image);
        ensure!(
            width > 0
                && height > 0
                && width <= maximum_dimension as usize
                && height <= maximum_dimension as usize,
            "The decoder returned a frame outside the preview size limit."
        );
        let stride = width
            .checked_mul(4)
            .context("Video frame dimensions overflowed.")?;
        let length = stride
            .checked_mul(height)
            .context("Video frame dimensions overflowed.")?;
        let mut rgba = Vec::new();
        rgba.try_reserve_exact(length)
            .context("Could not allocate the video preview.")?;
        rgba.resize(length, 0);
        let color_space = CGColorSpaceCreateWithName(kCGColorSpaceSRGB);
        ensure!(
            !color_space.is_null(),
            "Could not create the video preview color space."
        );
        let color_space = ColorSpace(color_space);
        // Big-endian 32-bit RGBA with premultiplied alpha gives a fixed byte layout on both Mac architectures.
        let context = CGBitmapContextCreate(
            rgba.as_mut_ptr().cast(),
            width,
            height,
            8,
            stride,
            color_space.0,
            (4 << 12) | 1,
        );
        ensure!(
            !context.is_null(),
            "Could not create the video preview bitmap."
        );
        let context = BitmapContext(context);
        CGContextDrawImage(
            context.0,
            NativeRect {
                origin: NativePoint { x: 0., y: 0. },
                size: NativeSize {
                    width: width as f64,
                    height: height as f64,
                },
            },
            image,
        );
        drop(context);
        for pixel in rgba.chunks_exact_mut(4) {
            let alpha = pixel[3];
            if alpha > 0 && alpha < 255 {
                for channel in &mut pixel[..3] {
                    *channel = ((u16::from(*channel) * 255 + u16::from(alpha) / 2)
                        / u16::from(alpha))
                    .min(255) as u8;
                }
            }
        }
        Ok(VideoFrame {
            rgba,
            width: width as u32,
            height: height as u32,
            actual_time_us,
        })
    }
}

struct ColorSpace(*mut c_void);
impl Drop for ColorSpace {
    fn drop(&mut self) {
        unsafe {
            CGColorSpaceRelease(self.0);
        }
    }
}
struct BitmapContext(*mut c_void);
impl Drop for BitmapContext {
    fn drop(&mut self) {
        unsafe {
            CGContextRelease(self.0);
        }
    }
}

#[derive(Clone, Copy)]
#[repr(C)]
struct NativeTime {
    value: i64,
    timescale: i32,
    flags: u32,
    epoch: i64,
}
impl NativeTime {
    const ZERO: Self = Self {
        value: 0,
        timescale: 1,
        flags: 1,
        epoch: 0,
    };
}
unsafe impl Encode for NativeTime {
    fn encode() -> Encoding {
        unsafe { Encoding::from_str("{CMTime=qiIq}") }
    }
}
#[derive(Clone, Copy)]
#[repr(C)]
struct NativeSize {
    width: f64,
    height: f64,
}

#[derive(Clone, Copy)]
#[repr(C)]
struct NativeTransform {
    a: f64,
    b: f64,
    c: f64,
    d: f64,
    tx: f64,
    ty: f64,
}
unsafe impl Encode for NativeTransform {
    fn encode() -> Encoding {
        unsafe { Encoding::from_str("{CGAffineTransform=dddddd}") }
    }
}

#[repr(C)]
struct NativeTimeRange {
    start: NativeTime,
    duration: NativeTime,
}
unsafe impl Encode for NativeTimeRange {
    fn encode() -> Encoding {
        unsafe { Encoding::from_str("{CMTimeRange={CMTime=qiIq}{CMTime=qiIq}}") }
    }
}
unsafe impl Encode for NativeSize {
    fn encode() -> Encoding {
        unsafe { Encoding::from_str("{CGSize=dd}") }
    }
}
#[repr(C)]
struct NativePoint {
    x: f64,
    y: f64,
}
#[repr(C)]
struct NativeRect {
    origin: NativePoint,
    size: NativeSize,
}

#[link(name = "AVFoundation", kind = "framework")]
#[link(name = "Foundation", kind = "framework")]
unsafe extern "C" {
    static AVURLAssetReferenceRestrictionsKey: *mut Object;
    static AVMediaTypeVideo: *mut Object;
}

#[link(name = "CoreVideo", kind = "framework")]
unsafe extern "C" {
    static kCVPixelBufferPixelFormatTypeKey: *mut Object;
    static kCVPixelBufferMetalCompatibilityKey: *mut Object;
    static kCVPixelBufferIOSurfacePropertiesKey: *mut Object;
}

#[link(name = "CoreMedia", kind = "framework")]
unsafe extern "C" {
    fn CMVideoFormatDescriptionGetPresentationDimensions(
        description: *const c_void,
        use_pixel_aspect_ratio: u8,
        use_clean_aperture: u8,
    ) -> NativeSize;
}

#[link(name = "QuartzCore", kind = "framework")]
unsafe extern "C" {
    fn CACurrentMediaTime() -> f64;
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    static kCGColorSpaceSRGB: *const c_void;
    fn CGImageGetWidth(image: *const c_void) -> usize;
    fn CGImageGetHeight(image: *const c_void) -> usize;
    fn CGColorSpaceCreateWithName(name: *const c_void) -> *mut c_void;
    fn CGColorSpaceRelease(space: *mut c_void);
    fn CGBitmapContextCreate(
        data: *mut c_void,
        width: usize,
        height: usize,
        bits_per_component: usize,
        bytes_per_row: usize,
        space: *mut c_void,
        bitmap_info: u32,
    ) -> *mut c_void;
    fn CGContextDrawImage(context: *mut c_void, rect: NativeRect, image: *const c_void);
    fn CGContextRelease(context: *mut c_void);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        path::Path,
        sync::Weak,
        time::{Duration, Instant},
    };

    const QUADRANTS: &[u8] = include_bytes!("../test_fixtures/quadrants.mp4");
    const TINY_QUADRANTS: &[u8] = include_bytes!("../test_fixtures/quadrants-tiny.mp4");
    const ROTATE_90: &[u8] = include_bytes!("../test_fixtures/quadrants-rotate-90.mp4");
    const ROTATE_180: &[u8] = include_bytes!("../test_fixtures/quadrants-rotate-180.mp4");
    const ROTATE_270: &[u8] = include_bytes!("../test_fixtures/quadrants-rotate-270.mp4");
    const MIRRORED: &[u8] = include_bytes!("../test_fixtures/quadrants-mirrored.mp4");
    const CORRUPT: &[u8] = include_bytes!("../test_fixtures/quadrants-corrupt.mp4");

    #[test]
    fn video_playback_geometry_rejects_unbounded_or_nonorthogonal_sources() -> Result<()> {
        let identity = NativeTransform {
            a: 1.,
            b: 0.,
            c: 0.,
            d: 1.,
            tx: 0.,
            ty: 0.,
        };
        let (info, transform) = playback_geometry(
            NativeSize {
                width: 1920.,
                height: 1080.,
            },
            identity,
            640,
            1_000_000,
        )?;
        assert_eq!((info.width, info.height), (640, 360));
        assert!((transform.a - 1. / 3.).abs() < 1e-12);
        assert!(
            playback_geometry(
                NativeSize {
                    width: 10_000.,
                    height: 1080.
                },
                identity,
                640,
                1_000_000
            )
            .is_err()
        );
        assert!(
            playback_geometry(
                NativeSize {
                    width: f64::NAN,
                    height: 1080.
                },
                identity,
                640,
                1_000_000
            )
            .is_err()
        );
        assert!(
            playback_geometry(
                NativeSize {
                    width: 1920.,
                    height: 1080.
                },
                NativeTransform { c: 0.2, ..identity },
                640,
                1_000_000
            )
            .is_err()
        );
        assert!(
            playback_geometry(
                NativeSize {
                    width: 1920.,
                    height: 1080.
                },
                NativeTransform {
                    a: 2.,
                    d: 2.,
                    ..identity
                },
                640,
                1_000_000
            )
            .is_err()
        );
        let (info, transform) = playback_geometry(
            NativeSize {
                width: 1920.,
                height: 1080.,
            },
            NativeTransform {
                a: 0.,
                b: 1.,
                c: -1.,
                d: 0.,
                tx: 35.,
                ty: -90.,
            },
            640,
            1_000_000,
        )?;
        assert_eq!((info.width, info.height), (360, 640));
        assert_eq!((transform.tx, transform.ty), (360., 0.));
        Ok(())
    }

    fn decode(bytes: &[u8], maximum_dimension: u32) -> Result<VideoFrame> {
        let request = video_frame(Arc::from(bytes), maximum_dimension)?;
        smol::block_on(async {
            match futures::future::select(request, smol::Timer::after(Duration::from_secs(10)))
                .await
            {
                futures::future::Either::Left((result, _)) => result,
                futures::future::Either::Right((_, request)) => {
                    drop(request);
                    bail!("Native video test timed out after ten seconds")
                }
            }
        })
    }

    fn assert_quadrants(frame: &VideoFrame, expected: [usize; 4]) {
        let colors = [[255i16, 0, 0], [0, 255, 0], [0, 0, 255], [255, 255, 0]];
        let width = frame.width as usize;
        let height = frame.height as usize;
        assert_eq!(frame.rgba.len(), width * height * 4);
        for ((x, y), expected) in [
            (width / 4, height / 4),
            (width * 3 / 4, height / 4),
            (width / 4, height * 3 / 4),
            (width * 3 / 4, height * 3 / 4),
        ]
        .into_iter()
        .zip(expected)
        {
            let start = (y * width + x) * 4;
            let actual = &frame.rgba[start..start + 4];
            for (channel, expected) in actual[..3].iter().zip(colors[expected]) {
                assert!(
                    (i16::from(*channel) - expected).abs() < 35,
                    "decoded quadrant at {x},{y}: {actual:?}, expected {}",
                    expected
                );
            }
            assert_eq!(actual[3], 255);
        }
    }

    fn wait_for_cleanup(input: Weak<tempfile::TempPath>, path: &Path) -> Result<()> {
        let deadline = Instant::now() + Duration::from_secs(5);
        while input.strong_count() != 0 || path.exists() {
            ensure!(
                Instant::now() < deadline,
                "Native callback retained its temporary video after completion/cancellation."
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        Ok(())
    }

    #[test]
    fn video_poster_decodes_real_h264_first_frame_and_bounds_output() -> Result<()> {
        let frame = decode(QUADRANTS, 1024)?;
        assert_eq!(
            (frame.width, frame.height),
            (256, 192),
            "never upscale small videos"
        );
        assert_eq!(frame.actual_time_us, 0);
        assert_quadrants(&frame, [0, 1, 2, 3]);
        let tiny = decode(TINY_QUADRANTS, 256)?;
        assert_eq!((tiny.width, tiny.height), (64, 48));
        assert_quadrants(&tiny, [0, 1, 2, 3]);
        let small = decode(QUADRANTS, 16)?;
        assert_eq!((small.width, small.height), (16, 12));
        assert_quadrants(&small, [0, 1, 2, 3]);
        Ok(())
    }

    #[test]
    fn video_poster_applies_rotated_track_matrices_to_pixels() -> Result<()> {
        for (bytes, dimensions, colors) in [
            (ROTATE_90, (48, 64), [1, 3, 0, 2]),
            (ROTATE_180, (64, 48), [3, 2, 1, 0]),
            (ROTATE_270, (48, 64), [2, 0, 3, 1]),
        ] {
            let frame = decode(bytes, 64)?;
            assert_eq!((frame.width, frame.height), dimensions);
            assert_eq!(frame.actual_time_us, 0);
            assert_quadrants(&frame, colors);
        }
        Ok(())
    }

    #[test]
    fn video_poster_applies_mirrored_track_matrix_to_pixels() -> Result<()> {
        let frame = decode(MIRRORED, 64)?;
        assert_eq!((frame.width, frame.height), (64, 48));
        assert_quadrants(&frame, [1, 0, 3, 2]);
        Ok(())
    }

    #[test]
    fn video_poster_rejects_corrupt_encoded_samples_and_invalid_limits() -> Result<()> {
        assert!(
            decode(CORRUPT, 64).is_err(),
            "valid MP4 metadata must not substitute for decoding actual samples"
        );
        assert!(video_frame(Arc::from([]), 64).is_err());
        assert!(video_frame(Arc::from(QUADRANTS), 0).is_err());
        assert!(video_frame(Arc::from(QUADRANTS), 2049).is_err());
        Ok(())
    }

    #[test]
    fn video_poster_request_is_send_and_cancellation_releases_native_input() -> Result<()> {
        fn assert_send<T: Send>() {}
        assert_send::<VideoFrameRequest>();
        let request = video_frame(Arc::from(QUADRANTS), 64)?;
        let input = Arc::downgrade(&request._input);
        let path = request._input.to_path_buf();
        let canceled = request.canceled.clone();
        assert!(path.exists());
        let generator = request.generator.as_ref().context("native request owner")?;
        let restrictions: usize = autoreleasepool(|| unsafe {
            let asset: *mut Object = msg_send![**generator, asset];
            msg_send![asset, referenceRestrictions]
        });
        assert_eq!(
            restrictions, 0xffff,
            "the decoder must not follow local or remote external media references"
        );
        drop(request);
        assert!(canceled.load(Ordering::Acquire));
        wait_for_cleanup(input, &path)
    }

    #[test]
    fn video_poster_completion_releases_callback_and_temporary_input() -> Result<()> {
        let request = video_frame(Arc::from(QUADRANTS), 64)?;
        let input = Arc::downgrade(&request._input);
        let path = request._input.to_path_buf();
        let frame = smol::block_on(async {
            match futures::future::select(request, smol::Timer::after(Duration::from_secs(10)))
                .await
            {
                futures::future::Either::Left((result, _)) => result,
                futures::future::Either::Right((_, request)) => {
                    drop(request);
                    bail!("Native completion test timed out")
                }
            }
        })?;
        assert_quadrants(&frame, [0, 1, 2, 3]);
        wait_for_cleanup(input, &path)
    }
}
