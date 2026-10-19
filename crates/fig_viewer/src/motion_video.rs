//! Rendered motion exports: PNG frames, GIF and MP4 sampled from one animation
//! clip on its stage frame, and the stage setup the Claude Motion Stage package
//! (`motion_stage`) shares. Frames come from Fanta's own renderer, so every
//! animated property looks exactly as it does in the Motion timeline.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context as _, Result, anyhow, bail, ensure};
use fanta_doc::{
    AnimationClip, AnimationClipId, Bounds, CanvasNode, Color, Doc, Fill, LoopMode,
    MotionEvaluation, MotionProperty, NodeData, NodeId, Transform2D, Viewport,
    resolve_effective_mode,
};
use fanta_render::{AssetResolver, RasterRenderer, visual_world_bounds};
use futures::{AsyncReadExt as _, AsyncWriteExt as _};
use image::codecs::gif::{GifEncoder, Repeat};
use image::codecs::png::{CompressionType, FilterType, PngEncoder};
use image::{Delay, ExtendedColorType, Frame, ImageEncoder as _, RgbaImage};

use crate::export::{
    ExportArtifact, ExportEntry, ExportFormat, ExportPayload, ExportSizing, raster_dimensions,
    render_inputs, resolve_export_bindings,
};

pub(crate) const DEFAULT_FPS: u16 = 30;
const MAX_FPS: u16 = 120;
/// Browsers stretch GIF delays below two centiseconds, so faster clips are
/// resampled.
const GIF_MAX_FPS: u16 = 30;
const MAX_FRAMES: usize = 7_200;
const FFMPEG_ENV: &str = "FANTA_FFMPEG";

/// One clip on the frame it plays in. The stage is detached from its page with
/// its box at the origin, so this document's world space is the export's
/// coordinate space.
pub(crate) struct MotionStage {
    pub(crate) clip: AnimationClipId,
    pub(crate) clip_name: String,
    pub(crate) duration_ms: u32,
    pub(crate) fps: u16,
    pub(crate) loop_mode: LoopMode,
    /// Painted behind transparent stage pixels in formats without alpha.
    pub(crate) backdrop: Color,
    pub(crate) explicit_backdrop: Option<Color>,
    pub(crate) stage: NodeId,
    pub(crate) doc: Doc,
    pub(crate) size: [f64; 2],
    pub(crate) notes: Vec<String>,
}

impl MotionStage {
    pub(crate) fn prepare(doc: &Doc, root: Option<NodeId>) -> Result<Self> {
        let clip = crate::motion_export::clip_for_target(doc, root)?;
        let stage = stage_node(doc, clip, root)?;
        let mut notes = Vec::new();

        let mut detached = resolve_export_bindings(doc).into_owned();
        // Modes pinned by the stage's ancestors stop applying once it is
        // detached, so they become document-wide for the export.
        detached.active_modes = doc
            .variables
            .collections
            .values()
            .map(|collection| {
                (
                    collection.id,
                    resolve_effective_mode(&doc.scene, stage, collection, &doc.active_modes),
                )
            })
            .collect();
        if detached
            .scene
            .get(stage)
            .and_then(|node| node.parent)
            .is_some()
        {
            let index = detached.scene.next_child_index(None);
            detached
                .scene
                .set_parent(stage, None, index)
                .context("detaching the stage frame")?;
        }
        detached
            .scene
            .get_mut(stage)
            .context("the stage frame disappeared")?
            .transform = Transform2D::IDENTITY;

        let stage_name = node_label(&detached, stage);
        let framed = match detached.scene.get(stage).map(|node| &node.data) {
            Some(NodeData::Group(group)) => group
                .clip_size
                .or(group.local_size)
                .map(|[width, height]| Bounds::from_xywh(0.0, 0.0, width, height)),
            _ => None,
        };
        let stage_box = match framed {
            Some(bounds) => bounds,
            None => {
                notes.push(format!(
                    "{stage_name} has no frame size, so the export frames its contents at rest."
                ));
                visual_world_bounds(&detached.scene, stage, 0.0)
                    .context("the stage has no visible content")?
            }
        };
        ensure!(
            stage_box.is_finite() && stage_box.width() > 0.0 && stage_box.height() > 0.0,
            "{stage_name} has an empty size"
        );
        if stage_box.min_x != 0.0 || stage_box.min_y != 0.0 {
            detached
                .scene
                .get_mut(stage)
                .context("the stage frame disappeared")?
                .transform = Transform2D::translation(-stage_box.min_x, -stage_box.min_y);
        }

        if clip.tracks.values().any(|track| {
            track.target.node == stage
                && !matches!(track.target.property, MotionProperty::Bound { .. })
        }) {
            notes.push(format!(
                "{stage_name} is the stage, so its own position, rotation and scale tracks are not animated in exports."
            ));
        }
        if detached
            .scene
            .descendants_of(stage)
            .filter_map(|id| detached.scene.get(id))
            .any(paints_video)
        {
            notes.push("Video layers export their poster frame.".to_string());
        }

        let explicit_backdrop = clip.settings.background;
        Ok(Self {
            clip: clip.id,
            clip_name: if clip.name.trim().is_empty() {
                "Animation".to_string()
            } else {
                clip.name.clone()
            },
            duration_ms: clip.duration_ms,
            fps: clip.settings.fps.unwrap_or(DEFAULT_FPS).clamp(1, MAX_FPS),
            loop_mode: clip.settings.loop_mode,
            backdrop: explicit_backdrop.unwrap_or(Color::WHITE),
            explicit_backdrop,
            stage,
            doc: detached,
            size: [stage_box.width(), stage_box.height()],
            notes,
        })
    }

    pub(crate) fn clip(&self) -> Result<&AnimationClip> {
        self.doc
            .motion
            .clip(self.clip)
            .context("the animation clip disappeared")
    }

    pub(crate) fn bounds(&self) -> Bounds {
        Bounds::from_xywh(0.0, 0.0, self.size[0], self.size[1])
    }

    /// The clip sampled at `playhead_ms`, without the stage's own transform
    /// channels: the stage is the camera, so it stays put.
    pub(crate) fn evaluate(&self, playhead_ms: u32) -> Result<MotionEvaluation> {
        let mut evaluation = self
            .doc
            .motion
            .evaluate(self.clip, playhead_ms)
            .context("the animation clip disappeared")?;
        let stage = self.stage;
        evaluation.overrides.retain(|target, _| {
            target.node != stage || matches!(target.property, MotionProperty::Bound { .. })
        });
        Ok(evaluation)
    }
}

/// The frame a clip plays in: its declared stage, the exported frame when it
/// holds every animated layer, or the top-level frame that does.
fn stage_node(doc: &Doc, clip: &AnimationClip, root: Option<NodeId>) -> Result<NodeId> {
    if let Some(scope) = clip.settings.scope
        && doc.scene.get(scope).is_some()
    {
        return Ok(scope);
    }
    let targets = clip
        .tracks
        .values()
        .map(|track| track.target.node)
        .filter(|node| doc.scene.get(*node).is_some())
        .collect::<Vec<_>>();
    ensure!(
        !targets.is_empty(),
        "the animation has no tracks on layers that still exist"
    );
    if let Some(root) = root
        && is_group(doc, root)
        && targets
            .iter()
            .all(|target| *target == root || is_descendant(doc, *target, root))
    {
        return Ok(root);
    }
    let pages = doc.pages().iter().copied().collect::<HashSet<_>>();
    let mut stage = None;
    for target in targets {
        let top =
            top_level_node(doc, &pages, target).context("an animated layer is not on a page")?;
        match stage {
            None => stage = Some(top),
            Some(existing) if existing == top => {}
            Some(_) => bail!(
                "the animation spans several top-level frames; select the frame to export or set the clip's stage"
            ),
        }
    }
    let stage = stage.context("the animation has no stage frame")?;
    ensure!(
        is_group(doc, stage),
        "put the animated layers inside a frame to export the animation"
    );
    Ok(stage)
}

fn top_level_node(doc: &Doc, pages: &HashSet<NodeId>, node: NodeId) -> Option<NodeId> {
    let mut current = node;
    loop {
        match doc.scene.get(current)?.parent {
            None => return Some(current),
            Some(parent) if pages.contains(&parent) => return Some(current),
            Some(parent) => current = parent,
        }
    }
}

fn is_group(doc: &Doc, node: NodeId) -> bool {
    matches!(
        doc.scene.get(node).map(|node| &node.data),
        Some(NodeData::Group(_))
    )
}

pub(crate) fn is_descendant(doc: &Doc, node: NodeId, ancestor: NodeId) -> bool {
    doc.scene
        .ancestors_of(node)
        .any(|candidate| candidate.id == ancestor)
}

pub(crate) fn node_label(doc: &Doc, node: NodeId) -> String {
    doc.scene
        .get(node)
        .map(|node| node.name.trim())
        .filter(|name| !name.is_empty())
        .map_or_else(
            || "An unnamed layer".to_string(),
            |name| format!("“{name}”"),
        )
}

fn paints_video(node: &CanvasNode) -> bool {
    let video_fill = |fills: &[Fill]| fills.iter().any(|fill| matches!(fill, Fill::Video { .. }));
    match &node.data {
        NodeData::Video(_) => true,
        NodeData::Vector(vector) => video_fill(&vector.fills),
        NodeData::Group(group) => {
            group
                .background
                .as_ref()
                .is_some_and(|fill| matches!(fill, Fill::Video { .. }))
                || video_fill(&group.background_fills)
        }
        _ => false,
    }
}

/// Playheads to sample, in milliseconds. One pass covers `[0, duration)` at
/// `fps`; a clip that plays once also gets its final pose, and a ping-pong clip
/// plays back down to its second frame so the sequence loops seamlessly.
pub(crate) fn sample_times(duration_ms: u32, fps: u16, loop_mode: LoopMode) -> Result<Vec<u32>> {
    ensure!(fps > 0, "the frame rate must be positive");
    let fps = f64::from(fps);
    let steps = (f64::from(duration_ms) / 1000.0 * fps).round() as usize;
    if duration_ms == 0 || steps == 0 {
        return Ok(vec![0]);
    }
    let at = |index: usize| -> u32 {
        (index as f64 * 1000.0 / fps)
            .round()
            .min(f64::from(duration_ms)) as u32
    };
    let times = match loop_mode {
        LoopMode::Once => (0..steps).map(at).chain([duration_ms]).collect::<Vec<_>>(),
        LoopMode::Repeat => (0..steps).map(at).collect(),
        LoopMode::PingPong => (0..steps)
            .map(at)
            .chain([duration_ms])
            .chain((1..steps).rev().map(at))
            .collect(),
    };
    ensure!(
        times.len() <= MAX_FRAMES,
        "the animation needs {} frames; the limit is {MAX_FRAMES}. Lower its frame rate or duration",
        times.len()
    );
    Ok(times)
}

/// Renders the stage at successive playheads into one reused surface whose
/// top-left pixel sits on the stage origin.
pub(crate) struct FrameRenderer<'a> {
    stage: &'a MotionStage,
    renderer: RasterRenderer,
    viewport: Viewport,
    pub(crate) width: u32,
    pub(crate) height: u32,
}

impl<'a> FrameRenderer<'a> {
    pub(crate) fn new(
        stage: &'a MotionStage,
        resolver: Option<Arc<dyn AssetResolver>>,
        zoom: f64,
        even: bool,
    ) -> Result<Self> {
        let (mut width, mut height) =
            raster_dimensions(stage.bounds(), ExportSizing::Scale(zoom), &stage.clip_name)?;
        if even {
            width += width % 2;
            height += height % 2;
        }
        let mut renderer = RasterRenderer::new(width, height)
            .map_err(|error| anyhow!("creating a {width}x{height} frame surface: {error}"))?;
        if let Some(resolver) = resolver {
            renderer.set_asset_resolver(resolver);
        }
        let viewport = Viewport {
            center: [
                f64::from(width) / (2.0 * zoom),
                f64::from(height) / (2.0 * zoom),
            ],
            zoom,
        };
        Ok(Self {
            stage,
            renderer,
            viewport,
            width,
            height,
        })
    }

    /// Straight-alpha RGBA of the stage at `playhead_ms`.
    pub(crate) fn render(&mut self, playhead_ms: u32) -> Result<Vec<u8>> {
        let evaluation = self.stage.evaluate(playhead_ms)?;
        let mut inputs = render_inputs(&self.stage.doc);
        inputs.motion = Some(&evaluation);
        self.renderer.render_page_with(
            &self.stage.doc.scene,
            &self.viewport,
            Some(self.stage.stage),
            &inputs,
        );
        Ok(self.renderer.copy_rgba())
    }
}

/// Export one clip as rendered media or as a Claude Motion Stage package.
pub(crate) fn export_motion_media(
    doc: &Doc,
    resolver: Option<Arc<dyn AssetResolver>>,
    root: Option<NodeId>,
    format: ExportFormat,
    sizing: ExportSizing,
) -> Result<ExportArtifact> {
    let stage = MotionStage::prepare(doc, root)?;
    let zoom = sizing.zoom(stage.bounds())?;
    let mut artifact = match format {
        ExportFormat::MotionFrames => png_frames(&stage, resolver, zoom, false)?,
        ExportFormat::MotionGif => ExportArtifact::file(gif(&stage, resolver, zoom)?),
        ExportFormat::MotionVideo => match find_ffmpeg() {
            Some(ffmpeg) => ExportArtifact::file(mp4(&ffmpeg, &stage, resolver, zoom)?),
            None => {
                let mut frames = png_frames(&stage, resolver, zoom, true)?;
                frames.extension = Some(ExportFormat::MotionFrames.extension());
                frames.notes.push(format!(
                    "ffmpeg was not found, so Fanta wrote the video's frames as PNGs instead. Install it (macOS: brew install ffmpeg; Windows: winget install Gyan.FFmpeg) or set {FFMPEG_ENV} to its path and export the MP4 again, or encode these frames with: ffmpeg -framerate {} -i frame_%06d.png -c:v libx264 -pix_fmt yuv420p video.mp4",
                    stage.fps
                ));
                frames
            }
        },
        ExportFormat::MotionStage => crate::motion_stage::stage_package(&stage, resolver, zoom)?,
        other => bail!("{} is not a motion media format", other.label()),
    };
    // The export's own notes (a fallback) lead, because a one-line summary
    // shows only the first note. The Stage package lists the stage's notes in
    // its README instead.
    if format != ExportFormat::MotionStage {
        artifact.notes.extend(stage.notes.iter().cloned());
    }
    Ok(artifact)
}

/// With `video`, the frames are the ones an MP4 export encodes: opaque over the
/// clip's backdrop, with even dimensions.
fn png_frames(
    stage: &MotionStage,
    resolver: Option<Arc<dyn AssetResolver>>,
    zoom: f64,
    video: bool,
) -> Result<ExportArtifact> {
    let times = sample_times(stage.duration_ms, stage.fps, stage.loop_mode)?;
    let mut frames = FrameRenderer::new(stage, resolver, zoom, video)?;
    let mut entries = Vec::with_capacity(times.len() + 1);
    for (index, time) in times.iter().enumerate() {
        let mut rgba = frames.render(*time)?;
        if video {
            rgba = flatten_rgba(&rgba, stage.backdrop);
        }
        entries.push(ExportEntry {
            path: format!("frame_{index:06}.png"),
            bytes: encode_png(frames.width, frames.height, &rgba)?,
        });
    }
    let info = serde_json::json!({
        "clip": stage.clip_name,
        "fps": stage.fps,
        "duration": f64::from(stage.duration_ms) / 1000.0,
        "loop": loop_name(stage.loop_mode),
        "width": frames.width,
        "height": frames.height,
        "scale": zoom,
        "frames": times.len(),
        "pattern": "frame_%06d.png",
    });
    entries.push(ExportEntry {
        path: "frames.json".to_string(),
        bytes: serde_json::to_vec_pretty(&info)?,
    });
    Ok(ExportArtifact {
        payload: ExportPayload::Directory(entries),
        extension: None,
        notes: Vec::new(),
    })
}

pub(crate) fn loop_name(mode: LoopMode) -> &'static str {
    match mode {
        LoopMode::Once => "once",
        LoopMode::Repeat => "loop",
        LoopMode::PingPong => "boomerang",
    }
}

pub(crate) fn encode_png(width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    PngEncoder::new_with_quality(&mut bytes, CompressionType::Fast, FilterType::Adaptive)
        .write_image(rgba, width, height, ExtendedColorType::Rgba8)
        .context("encoding a PNG frame")?;
    Ok(bytes)
}

fn composite(channel: u8, alpha: u8, backdrop: u8) -> u8 {
    let alpha = u16::from(alpha);
    ((u16::from(channel) * alpha + u16::from(backdrop) * (255 - alpha) + 127) / 255) as u8
}

fn flatten_rgba(rgba: &[u8], backdrop: Color) -> Vec<u8> {
    let mut flat = Vec::with_capacity(rgba.len());
    for pixel in rgba.chunks_exact(4) {
        flat.extend_from_slice(&[
            composite(pixel[0], pixel[3], backdrop.r),
            composite(pixel[1], pixel[3], backdrop.g),
            composite(pixel[2], pixel[3], backdrop.b),
            255,
        ]);
    }
    flat
}

fn flatten_rgb(rgba: &[u8], backdrop: Color) -> Vec<u8> {
    let mut flat = Vec::with_capacity(rgba.len() / 4 * 3);
    for pixel in rgba.chunks_exact(4) {
        flat.extend_from_slice(&[
            composite(pixel[0], pixel[3], backdrop.r),
            composite(pixel[1], pixel[3], backdrop.g),
            composite(pixel[2], pixel[3], backdrop.b),
        ]);
    }
    flat
}

/// GIF delays are whole centiseconds; accumulating the rounding keeps the
/// average frame time exact.
pub(crate) fn gif_delay_centiseconds(index: usize, fps: u16) -> u32 {
    let at = |index: usize| (index as f64 * 100.0 / f64::from(fps)).round() as u32;
    at(index + 1).saturating_sub(at(index)).max(2)
}

fn gif(
    stage: &MotionStage,
    resolver: Option<Arc<dyn AssetResolver>>,
    zoom: f64,
) -> Result<Vec<u8>> {
    let fps = stage.fps.min(GIF_MAX_FPS);
    let times = sample_times(stage.duration_ms, fps, stage.loop_mode)?;
    let mut frames = FrameRenderer::new(stage, resolver, zoom, false)?;
    let mut bytes = Vec::new();
    {
        let mut encoder = GifEncoder::new_with_speed(&mut bytes, 10);
        encoder
            .set_repeat(match stage.loop_mode {
                LoopMode::Once => Repeat::Finite(0),
                LoopMode::Repeat | LoopMode::PingPong => Repeat::Infinite,
            })
            .context("configuring GIF looping")?;
        for (index, time) in times.iter().enumerate() {
            let flat = flatten_rgba(&frames.render(*time)?, stage.backdrop);
            let image = RgbaImage::from_raw(frames.width, frames.height, flat)
                .context("a GIF frame has the wrong size")?;
            let delay = gif_delay_centiseconds(index, fps) * 10;
            encoder
                .encode_frame(Frame::from_parts(
                    image,
                    0,
                    0,
                    Delay::from_numer_denom_ms(delay, 1),
                ))
                .context("encoding a GIF frame")?;
        }
    }
    Ok(bytes)
}

/// `FANTA_FFMPEG`, then `PATH`. Apps opened from the macOS Finder do not
/// inherit the shell's `PATH`, so Homebrew's prefixes are checked as well.
pub(crate) fn find_ffmpeg() -> Option<PathBuf> {
    if let Some(explicit) = std::env::var_os(FFMPEG_ENV).map(PathBuf::from)
        && explicit.is_file()
    {
        return Some(explicit);
    }
    let name = if cfg!(windows) {
        "ffmpeg.exe"
    } else {
        "ffmpeg"
    };
    let mut directories = std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).collect::<Vec<_>>())
        .unwrap_or_default();
    if cfg!(target_os = "macos") {
        directories.push(PathBuf::from("/opt/homebrew/bin"));
        directories.push(PathBuf::from("/usr/local/bin"));
    }
    directories
        .into_iter()
        .map(|directory| directory.join(name))
        .find(|candidate| candidate.is_file())
}

fn mp4(
    ffmpeg: &Path,
    stage: &MotionStage,
    resolver: Option<Arc<dyn AssetResolver>>,
    zoom: f64,
) -> Result<Vec<u8>> {
    let times = sample_times(stage.duration_ms, stage.fps, stage.loop_mode)?;
    let mut frames = FrameRenderer::new(stage, resolver, zoom, true)?;
    let workspace = tempfile::Builder::new()
        .prefix("fanta-video-")
        .tempdir()
        .context("creating a temporary video folder")?;
    let output = workspace.path().join("video.mp4");
    let mut child = util::command::new_command(ffmpeg)
        .args(["-hide_banner", "-loglevel", "error", "-y"])
        .args(["-f", "rawvideo", "-pix_fmt", "rgb24", "-s"])
        .arg(format!("{}x{}", frames.width, frames.height))
        .arg("-framerate")
        .arg(stage.fps.to_string())
        .args(["-i", "-", "-c:v", "libx264", "-pix_fmt", "yuv420p"])
        .args(["-crf", "18", "-preset", "medium", "-movflags", "+faststart"])
        .arg(&output)
        .stdin(util::command::Stdio::piped())
        .stdout(util::command::Stdio::null())
        .stderr(util::command::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .with_context(|| format!("starting {}", ffmpeg.display()))?;
    let mut stdin = child.stdin.take().context("ffmpeg has no input stream")?;
    let mut stderr = child.stderr.take().context("ffmpeg has no error stream")?;
    // Exports already run off the main thread, so this one blocks until the
    // encoder is done. Errors are read while frames are written, so a chatty
    // encoder cannot stall on a full pipe.
    let (written, errors) = futures::executor::block_on(futures::future::join(
        async move {
            for time in &times {
                let rgb = flatten_rgb(&frames.render(*time)?, stage.backdrop);
                stdin
                    .write_all(&rgb)
                    .await
                    .context("sending a frame to ffmpeg")?;
            }
            // Windows pipes buffer writes, so the input is flushed before EOF.
            stdin.close().await.context("closing ffmpeg's input")
        },
        async move {
            let mut text = String::new();
            match stderr.read_to_string(&mut text).await {
                Ok(_) => text,
                Err(error) => format!("(could not read ffmpeg's errors: {error})"),
            }
        },
    ));
    let status = futures::executor::block_on(child.status()).context("waiting for ffmpeg")?;
    if let Err(error) = written {
        return Err(error.context(format!("ffmpeg stopped early: {}", errors.trim())));
    }
    ensure!(
        status.success(),
        "ffmpeg failed ({status}): {}",
        errors.trim()
    );
    std::fs::read(&output).context("reading the encoded video")
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use fanta_doc::{GroupNode, MotionSpec, UnitInterval, VectorNode};
    use image::AnimationDecoder as _;
    use image::codecs::gif::GifDecoder;
    use serde_json::json;

    pub(crate) struct AnimatedStage {
        pub(crate) doc: Doc,
        pub(crate) frame: NodeId,
        pub(crate) card: NodeId,
        pub(crate) holder: NodeId,
        pub(crate) dot: NodeId,
    }

    pub(crate) fn insert(doc: &mut Doc, mut node: CanvasNode) -> NodeId {
        node.index = doc.scene.next_child_index(node.parent);
        let id = node.id;
        doc.scene.insert(node).expect("test layer is valid");
        id
    }

    pub(crate) fn layer(name: &str, parent: NodeId, data: NodeData, x: f64, y: f64) -> CanvasNode {
        let mut node = CanvasNode::new(data);
        node.name = name.to_string();
        node.parent = Some(parent);
        node.transform = Transform2D::translation(x, y);
        node
    }

    pub(crate) fn square(size: f64, color: Color) -> NodeData {
        NodeData::Vector(VectorNode::rect_solid(0.0, 0.0, size, size, color))
    }

    pub(crate) fn stage_frame(doc: &mut Doc) -> NodeId {
        let mut frame = CanvasNode::new(NodeData::Group(GroupNode {
            clip_size: Some([200.0, 100.0]),
            background: Some(Fill::solid(Color::rgb(240, 240, 240))),
            corner_radius: Some(12.0),
            ..GroupNode::default()
        }));
        frame.name = "Stage".to_string();
        frame.transform = Transform2D::translation(40.0, 30.0);
        insert(doc, frame)
    }

    pub(crate) fn add_clip(doc: &mut Doc, spec: serde_json::Value) {
        let spec: MotionSpec = serde_json::from_value(spec).expect("motion spec");
        let clip = spec
            .to_clip(AnimationClipId::new(), None)
            .expect("spec becomes a clip");
        doc.motion.clips.insert(clip.id, clip);
    }

    /// A 200 × 100 frame placed at (40, 30) on the canvas. A red card moves,
    /// turns and fades inside a static 80% wrapper; a group that paints nothing
    /// slides down and carries a blue dot that springs wider and then hides.
    pub(crate) fn animated_stage() -> AnimatedStage {
        let mut doc = Doc::new();
        let frame = stage_frame(&mut doc);
        let mut wrapper = layer(
            "Wrapper",
            frame,
            NodeData::Group(GroupNode::default()),
            5.0,
            0.0,
        );
        wrapper.opacity = UnitInterval::new(0.8);
        let wrapper = insert(&mut doc, wrapper);
        let card = insert(
            &mut doc,
            layer(
                "Card",
                wrapper,
                square(40.0, Color::rgb(220, 30, 30)),
                20.0,
                30.0,
            ),
        );
        let holder = insert(
            &mut doc,
            layer(
                "Holder",
                frame,
                NodeData::Group(GroupNode::default()),
                120.0,
                10.0,
            ),
        );
        let dot = insert(
            &mut doc,
            layer(
                "Dot",
                holder,
                square(20.0, Color::rgb(30, 60, 220)),
                10.0,
                10.0,
            ),
        );
        add_clip(
            &mut doc,
            json!({
                "version": 1, "name": "Hero", "duration": 1.0, "fps": 10,
                "layers": [
                    {"node": card.to_string(), "origin": [0.5, 0.5], "tracks": {
                        "translation_x": [{"t": 0.0, "v": 20.0, "easing": "ease_out"}, {"t": 1.0, "v": 140.0}],
                        "rotation": [{"t": 0.0, "v": 0.0, "easing": {"cubic_bezier": [0.2, 0.0, 0.0, 1.0]}}, {"t": 1.0, "v": 45.0}],
                        "opacity": [{"t": 0.5, "v": 1.0}, {"t": 1.0, "v": 0.5}]
                    }},
                    {"node": holder.to_string(), "tracks": {
                        "translation_y": [{"t": 0.0, "v": 10.0}, {"t": 1.0, "v": 50.0}]
                    }},
                    {"node": dot.to_string(), "origin": [0.5, 0.5], "tracks": {
                        "scale_x": [{"t": 0.0, "v": 1.0, "easing": {"spring": {"mass": 1.0, "stiffness": 120.0, "damping": 14.0}}}, {"t": 1.0, "v": 2.0}],
                        "visible": [{"t": 0.0, "v": true}, {"t": 0.75, "v": false}]
                    }}
                ]
            }),
        );
        AnimatedStage {
            doc,
            frame,
            card,
            holder,
            dot,
        }
    }

    pub(crate) fn directory_entries(artifact: &ExportArtifact) -> &[ExportEntry] {
        match &artifact.payload {
            ExportPayload::Directory(entries) => entries,
            ExportPayload::File(_) => panic!("expected a folder export"),
        }
    }

    pub(crate) fn entry<'a>(entries: &'a [ExportEntry], path: &str) -> &'a [u8] {
        &entries
            .iter()
            .find(|entry| entry.path == path)
            .unwrap_or_else(|| panic!("the export has no {path}"))
            .bytes
    }

    fn pixel(png: &[u8], x: u32, y: u32) -> [u8; 4] {
        image::load_from_memory(png)
            .expect("frame decodes")
            .to_rgba8()
            .get_pixel(x, y)
            .0
    }

    fn assert_color(actual: [u8; 4], expected: [u8; 3], what: &str) {
        assert!(
            actual[..3]
                .iter()
                .zip(expected)
                .all(|(actual, expected)| actual.abs_diff(expected) <= 4)
                && actual[3] == 255,
            "{what}: expected {expected:?}, got {actual:?}"
        );
    }

    #[test]
    fn png_frames_render_the_clip_on_its_detached_stage() {
        let fixture = animated_stage();
        let artifact = export_motion_media(
            &fixture.doc,
            None,
            Some(fixture.frame),
            ExportFormat::MotionFrames,
            ExportSizing::Scale(1.0),
        )
        .expect("frames export");
        let entries = directory_entries(&artifact);
        assert_eq!(
            entries.len(),
            12,
            "eleven frames at 10 fps plus frames.json"
        );
        let info: serde_json::Value =
            serde_json::from_slice(entry(entries, "frames.json")).expect("frames.json");
        assert_eq!(info["frames"], 11);
        assert_eq!(info["fps"], 10);
        assert_eq!(
            (info["width"].as_u64(), info["height"].as_u64()),
            (Some(200), Some(100))
        );
        assert_eq!(info["loop"], "once");

        // The frame's canvas position is gone: the card's centre starts at
        // (5 + 20 + 20, 30 + 20) and shows through the 80% wrapper.
        let first = entry(entries, "frame_000000.png");
        assert_color(pixel(first, 45, 50), [224, 72, 72], "card at rest");
        assert_color(pixel(first, 140, 30), [30, 60, 220], "dot at rest");
        // At the end the card has moved 120 px and faded to 50%, the holder
        // has slid 40 px down and the dot is hidden.
        let last = entry(entries, "frame_000010.png");
        assert_color(pixel(last, 45, 50), [240, 240, 240], "where the card was");
        assert_color(pixel(last, 165, 50), [232, 156, 156], "card at the end");
        assert_color(pixel(last, 140, 70), [240, 240, 240], "hidden dot");
    }

    #[test]
    fn gif_frames_and_looping_follow_the_clip() {
        let mut fixture = animated_stage();
        let gif = |doc: &Doc| {
            let artifact = export_motion_media(
                doc,
                None,
                None,
                ExportFormat::MotionGif,
                ExportSizing::Scale(1.0),
            )
            .expect("GIF export");
            match artifact.payload {
                ExportPayload::File(bytes) => bytes,
                ExportPayload::Directory(_) => panic!("a GIF is one file"),
            }
        };
        let decode = |bytes: &[u8]| {
            GifDecoder::new(std::io::Cursor::new(bytes))
                .expect("GIF header")
                .into_frames()
                .collect_frames()
                .expect("GIF frames")
        };
        let has_loop_extension =
            |bytes: &[u8]| bytes.windows(11).any(|window| window == b"NETSCAPE2.0");

        let once = gif(&fixture.doc);
        let frames = decode(&once);
        assert_eq!(frames.len(), 11);
        assert!(
            frames.iter().all(|frame| {
                std::time::Duration::from(frame.delay()) == std::time::Duration::from_millis(100)
            }),
            "every frame lasts a tenth of a second"
        );
        assert_eq!(frames[0].buffer().dimensions(), (200, 100));
        assert!(!has_loop_extension(&once), "a clip that plays once stops");

        for clip in fixture.doc.motion.clips.values_mut() {
            clip.settings.loop_mode = LoopMode::Repeat;
        }
        let looping = gif(&fixture.doc);
        assert_eq!(decode(&looping).len(), 10);
        assert!(
            has_loop_extension(&looping),
            "a looping clip repeats forever"
        );
    }

    #[test]
    fn mp4_export_encodes_with_ffmpeg_or_writes_its_frames() {
        let fixture = animated_stage();
        let artifact = export_motion_media(
            &fixture.doc,
            None,
            Some(fixture.frame),
            ExportFormat::MotionVideo,
            ExportSizing::Scale(1.0),
        )
        .expect("video export");
        if find_ffmpeg().is_some() {
            let ExportPayload::File(bytes) = &artifact.payload else {
                panic!("an MP4 is one file");
            };
            assert_eq!(
                bytes.get(4..8),
                Some(&b"ftyp"[..]),
                "an MP4 starts with ftyp"
            );
            assert!(artifact.extension.is_none());
        } else {
            assert_eq!(artifact.extension, Some("frames"));
            assert!(
                artifact
                    .notes
                    .first()
                    .is_some_and(|note| note.contains("ffmpeg")),
                "the first note explains the fallback: {:?}",
                artifact.notes
            );
            assert_eq!(directory_entries(&artifact).len(), 12);
        }
    }

    #[test]
    fn sampling_covers_each_loop_mode_with_seamless_ends() {
        let once = sample_times(1_000, 30, LoopMode::Once).expect("once");
        assert_eq!(once.len(), 31, "a clip played once ends on its final pose");
        assert_eq!(once.first(), Some(&0));
        assert_eq!(once.last(), Some(&1_000));
        assert_eq!(once.get(1), Some(&33));

        let repeat = sample_times(1_000, 30, LoopMode::Repeat).expect("repeat");
        assert_eq!(
            repeat.len(),
            30,
            "a loop omits the frame that equals its first"
        );
        assert_eq!(repeat.last(), Some(&967));

        let ping_pong = sample_times(1_000, 10, LoopMode::PingPong).expect("ping-pong");
        assert_eq!(
            ping_pong,
            vec![
                0, 100, 200, 300, 400, 500, 600, 700, 800, 900, 1_000, 900, 800, 700, 600, 500,
                400, 300, 200, 100
            ]
        );

        assert_eq!(sample_times(0, 30, LoopMode::Once).expect("still"), vec![0]);
        assert!(sample_times(600_000, 120, LoopMode::Repeat).is_err());
    }

    #[test]
    fn gif_delays_average_the_exact_frame_time() {
        let total = (0..30)
            .map(|index| gif_delay_centiseconds(index, 30))
            .sum::<u32>();
        assert_eq!(total, 100, "thirty frames at 30 fps last one second");
        assert!((0..30).all(|index| (3..=4).contains(&gif_delay_centiseconds(index, 30))));
        assert_eq!(
            gif_delay_centiseconds(0, 100),
            2,
            "delays never drop below two centiseconds"
        );
    }

    #[test]
    fn flattening_composites_straight_alpha_over_the_backdrop() {
        let backdrop = Color::rgb(0, 0, 255);
        let pixels = [255, 0, 0, 255, 255, 0, 0, 0, 255, 255, 255, 128];
        assert_eq!(
            flatten_rgb(&pixels, backdrop),
            vec![255, 0, 0, 0, 0, 255, 128, 128, 255]
        );
        assert_eq!(&flatten_rgba(&pixels, backdrop)[..4], &[255, 0, 0, 255]);
    }
}
