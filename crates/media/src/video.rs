use anyhow::{Context as _, Result, anyhow, bail, ensure};
use block::ConcreteBlock;
use core_foundation::{base::TCFType, string::CFString};
use futures::channel::oneshot;
use objc::{
    Encode, Encoding, class, msg_send,
    rc::{StrongPtr, autoreleasepool},
    runtime::{Object, YES},
    sel, sel_impl,
};
use std::{
    ffi::c_void,
    future::Future,
    io::Write,
    panic::{AssertUnwindSafe, catch_unwind},
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    task::{Context, Poll},
};

const MAX_INPUT_BYTES: usize = 100 * 1024 * 1024;
const MAX_DIMENSION: u32 = 2048;

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
    let canceled = Arc::new(AtomicBool::new(false));
    let (sender, receiver) = oneshot::channel();
    let sender = Mutex::new(Some(sender));
    let generator = autoreleasepool(|| unsafe {
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
        let generator: *mut Object = msg_send![class!(AVAssetImageGenerator), alloc];
        let generator: *mut Object = msg_send![generator, initWithAsset:*asset];
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
