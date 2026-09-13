use anyhow::{Context as _, Result, bail, ensure};
use fanta_doc::{
    AssetId, BlendMode, CanvasNode, Color, Fill, FillRule, Gradient, GradientStop, GroupNode,
    ImageFitMode, IndexKey, NodeData, NodeId, Operation, PathData, Stroke, StrokeCap, StrokeJoin,
    Transaction, Transform2D, UnitInterval, VectorNode, VideoNode,
};
use serde_json::Value;
use std::{
    fs::File,
    io::{Read as _, Write as _},
    path::Path,
    sync::Arc,
};

use crate::document::{DocChange, FigDocument};

const MAX_LOCAL_VIDEO_BYTES: usize = 100 * 1024 * 1024;
const MAX_LOCAL_IMAGE_PIXELS: u64 = 32 * 1024 * 1024;

#[derive(Clone)]
pub(crate) struct PreparedLocalMedia {
    name: String,
    content: LocalMediaContent,
}

#[derive(Clone)]
enum LocalMediaContent {
    Image {
        bytes: Arc<[u8]>,
        natural_size: [u32; 2],
    },
    Video(PreparedVideo),
}

pub(crate) async fn prepare_local_media(path: &Path) -> Result<PreparedLocalMedia> {
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let video = extension == "mp4";
    ensure!(
        video
            || matches!(
                extension.as_str(),
                "png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp" | "tiff" | "tif"
            ),
        "Choose a PNG, JPEG, WebP, GIF, BMP, TIFF, or MP4 file."
    );
    let name = path
        .file_name()
        .context("The selected file has no name.")?
        .to_string_lossy()
        .into_owned();
    let limit = if video {
        MAX_LOCAL_VIDEO_BYTES
    } else {
        crate::document::MAX_IMAGE_SOURCE_BYTES
    };
    let bytes = read_local_media_file(path, limit)?;
    let content = if video {
        ensure!(
            cfg!(target_os = "macos"),
            "Local MP4 placement requires the macOS video decoder."
        );
        LocalMediaContent::Video(prepare_video(bytes.into()).await?)
    } else {
        let natural_size = validate_local_image(&bytes)?;
        LocalMediaContent::Image {
            bytes: bytes.into(),
            natural_size,
        }
    };
    Ok(PreparedLocalMedia { name, content })
}

fn read_local_media_file(path: &Path, limit: usize) -> Result<Vec<u8>> {
    ensure!(
        std::fs::metadata(path)
            .with_context(|| format!("Reading {}", path.display()))?
            .is_file(),
        "Choose a regular image or video file."
    );
    let file = File::open(path).with_context(|| format!("Opening {}", path.display()))?;
    let metadata = file.metadata()?;
    ensure!(metadata.is_file(), "Choose a regular image or video file.");
    ensure!(
        metadata.len() <= limit as u64,
        "{} exceeds the {} MiB file limit.",
        path.display(),
        limit / (1024 * 1024)
    );
    // The file can grow after metadata is read. Cap the open handle too, before
    // either decoder sees the bytes.
    let mut bytes = Vec::new();
    file.take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .with_context(|| format!("Reading {}", path.display()))?;
    ensure!(
        bytes.len() <= limit,
        "{} exceeds the {} MiB file limit.",
        path.display(),
        limit / (1024 * 1024)
    );
    Ok(bytes)
}

fn validate_local_image(bytes: &[u8]) -> Result<[u32; 2]> {
    let format = image::guess_format(bytes).context("The image format could not be read.")?;
    ensure!(
        matches!(
            format,
            image::ImageFormat::Png
                | image::ImageFormat::Jpeg
                | image::ImageFormat::WebP
                | image::ImageFormat::Gif
                | image::ImageFormat::Bmp
                | image::ImageFormat::Tiff
        ),
        "This image format is not supported for local placement."
    );
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(16_384);
    limits.max_image_height = Some(16_384);
    limits.max_alloc = Some(256 * 1024 * 1024);
    let mut reader = image::ImageReader::with_format(std::io::Cursor::new(bytes), format);
    reader.limits(limits.clone());
    let (width, height) = reader
        .into_dimensions()
        .context("The image dimensions could not be read.")?;
    ensure!(
        width > 0 && height > 0 && u64::from(width) * u64::from(height) <= MAX_LOCAL_IMAGE_PIXELS,
        "The image exceeds the 32 megapixel placement limit."
    );
    let mut reader = image::ImageReader::with_format(std::io::Cursor::new(bytes), format);
    reader.limits(limits);
    reader.decode().context("The image could not be decoded.")?;
    Ok([width, height])
}

pub(crate) fn place_local_media(
    document: &mut FigDocument,
    media: PreparedLocalMedia,
    center: [f64; 2],
    visible: [f64; 2],
) -> (Result<NodeId>, DocChange) {
    let result = place_local_media_inner(document, media, center, visible);
    let change = if result.is_ok() {
        DocChange::Content
    } else {
        DocChange::None
    };
    (result, change)
}

fn place_local_media_inner(
    document: &mut FigDocument,
    media: PreparedLocalMedia,
    center: [f64; 2],
    visible: [f64; 2],
) -> Result<NodeId> {
    let page = document
        .doc
        .active_page()
        .context("There is no active page.")?;
    ensure!(
        document.doc.pages.contains(&page)
            && document
                .pages
                .iter()
                .any(|entry| entry.root == Some(page) && !entry.hidden),
        "Choose a visible document page before placing media."
    );
    let root = document
        .doc
        .scene
        .get(page)
        .context("The page no longer exists.")?;
    ensure!(
        root.can_have_children(),
        "The active page cannot contain media."
    );
    for node in std::iter::once(root).chain(document.doc.scene.ancestors_of(page)) {
        ensure!(
            !node
                .flags
                .intersects(fanta_doc::NodeFlags::HIDDEN | fanta_doc::NodeFlags::LOCKED),
            "The active page is hidden or locked."
        );
    }
    ensure!(
        center.into_iter().all(f64::is_finite)
            && visible
                .into_iter()
                .all(|value| value.is_finite() && value > 0.),
        "The canvas placement bounds are invalid."
    );
    let natural_size = match &media.content {
        LocalMediaContent::Image { natural_size, .. } => *natural_size,
        LocalMediaContent::Video(video) => [video.metadata.width, video.metadata.height],
    };
    ensure!(
        natural_size.into_iter().all(|value| value > 0),
        "The media has no pixels."
    );
    let fit = (visible[0] * 0.8 / f64::from(natural_size[0]))
        .min(visible[1] * 0.8 / f64::from(natural_size[1]))
        .min(1.);
    let size = [
        f64::from(natural_size[0]) * fit,
        f64::from(natural_size[1]) * fit,
    ];
    let mut node = crate::structure::image_layer_node(
        &document.doc,
        AssetId::new(),
        natural_size,
        size,
        Some(page),
        center[0] - size[0] * 0.5,
        center[1] - size[1] * 0.5,
        Some(&media.name),
    )?;
    let mut added_images = Vec::new();
    let mut video_bytes = None;
    match media.content {
        LocalMediaContent::Image { bytes, .. } => {
            let (asset, _) = document.doc_and_assets().1.add_image(bytes.to_vec())?;
            added_images.push(asset);
            if let NodeData::Bitmap(bitmap) = &mut node.data {
                bitmap.asset = asset;
            }
        }
        LocalMediaContent::Video(video) => {
            let poster_asset = match &video.poster {
                Some(poster) => {
                    let (asset, _) = document.doc_and_assets().1.add_image(poster.png.to_vec())?;
                    added_images.push(asset);
                    Some(asset)
                }
                None => None,
            };
            let asset = AssetId::new();
            let mut data = video_node_data(&video, asset, poster_asset);
            data.local_size = size;
            node.data = NodeData::Video(data);
            video_bytes = Some((asset, video.bytes));
        }
    }
    let id = node.id;
    let mut transaction = Transaction::new("Place media");
    transaction.push(Operation::create_node(node));
    if let Err(error) = document.doc.apply_transaction(transaction) {
        for asset in added_images {
            document.doc_and_assets().1.remove(asset);
        }
        return Err(error.into());
    }
    if let Some((asset, bytes)) = video_bytes {
        Arc::make_mut(&mut document.raw_assets).insert(asset, bytes.to_vec());
    }
    document.doc.selection.select_only(id);
    Ok(id)
}

pub(crate) fn write_output(path: &Path, bytes: &[u8]) -> Result<()> {
    write_output_with(path, |file| file.write_all(bytes)).context("The result could not be saved")
}

fn write_output_with(
    path: &Path,
    write: impl FnOnce(&mut File) -> std::io::Result<()>,
) -> std::io::Result<()> {
    let permissions = match std::fs::metadata(path) {
        Ok(metadata) => Some(metadata.permissions()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error),
    };
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    let result = (|| {
        if let Some(permissions) = permissions {
            temporary.as_file().set_permissions(permissions)?;
        }
        write(temporary.as_file_mut())?;
        temporary.as_file().sync_all()
    })();
    if let Err(error) = result {
        return Err(discard_output_temporary(temporary, error));
    }
    match temporary.persist(path) {
        Ok(_) => Ok(()),
        Err(error) => Err(discard_output_temporary(error.file, error.error)),
    }
}

fn discard_output_temporary(
    temporary: tempfile::NamedTempFile,
    primary: std::io::Error,
) -> std::io::Error {
    match temporary.close() {
        Ok(()) => primary,
        Err(cleanup) => std::io::Error::new(
            primary.kind(),
            format!("{primary}; also failed to remove the temporary result: {cleanup}"),
        ),
    }
}

pub(crate) struct VectorArtwork {
    pub width: f64,
    pub height: f64,
    nodes: Vec<CanvasNode>,
    root: NodeId,
}

pub(crate) fn parse_svg(bytes: &[u8]) -> Result<VectorArtwork> {
    ensure!(
        bytes.len() <= 4 * 1024 * 1024,
        "The SVG is too large to import as editable vectors."
    );
    let text = std::str::from_utf8(bytes).context("The SVG is not valid text.")?;
    let xml = usvg::roxmltree::Document::parse(text).context("The SVG could not be read.")?;
    for node in xml.descendants().filter(|node| node.is_element()) {
        ensure!(
            !matches!(
                node.tag_name().name(),
                "image" | "text" | "foreignObject" | "script" | "animate" | "animateTransform"
            ),
            "This SVG contains {}. Save it as SVG, or generate artwork made from paths and shapes for editable placement.",
            node.tag_name().name()
        );
    }
    let options = usvg::Options {
        image_href_resolver: usvg::ImageHrefResolver {
            resolve_data: Box::new(|_, _, _| None),
            resolve_string: Box::new(|_, _| None),
        },
        ..usvg::Options::default()
    };
    let tree = usvg::Tree::from_xmltree(&xml, &options)
        .context("The SVG could not be converted to paths.")?;
    let width = tree.size().width() as f64;
    let height = tree.size().height() as f64;
    let mut root = CanvasNode::new(NodeData::Group(GroupNode {
        clip_size: Some([width, height]),
        ..GroupNode::default()
    }));
    root.name = "Generated vectors".into();
    let root_id = root.id;
    let mut nodes = vec![root];
    let mut remaining_segments = 100_000;
    append_svg_group(
        tree.root(),
        root_id,
        Transform2D::IDENTITY,
        &mut nodes,
        0,
        &mut remaining_segments,
    )?;
    ensure!(nodes.len() > 1, "The SVG contains no editable paths.");
    Ok(VectorArtwork {
        width,
        height,
        nodes,
        root: root_id,
    })
}

fn append_svg_group(
    group: &usvg::Group,
    parent: NodeId,
    parent_transform: Transform2D,
    nodes: &mut Vec<CanvasNode>,
    depth: usize,
    remaining_segments: &mut usize,
) -> Result<()> {
    ensure!(depth < 64, "The SVG has too many nested groups.");
    ensure!(
        group.clip_path().is_none() && group.mask().is_none() && group.filters().is_empty(),
        "This SVG uses clipping, masks, or filters. Save the SVG, or generate simpler paths for editable placement."
    );
    ensure!(
        group.blend_mode() == usvg::BlendMode::Normal,
        "This SVG uses a blend mode that cannot be imported faithfully yet. Save the SVG instead."
    );
    let mut index = IndexKey::default();
    for child in group.children() {
        ensure!(
            nodes.len() < 5000,
            "The SVG contains too many paths for editable placement."
        );
        let mut node = match child {
            usvg::Node::Group(group) => {
                let mut node = CanvasNode::new(NodeData::Group(GroupNode::default()));
                node.name = if group.id().is_empty() {
                    "Vector group".into()
                } else {
                    group.id().to_owned()
                };
                node.opacity = UnitInterval::new(group.opacity().get());
                node
            }
            usvg::Node::Path(path) => {
                if !path.is_visible() {
                    continue;
                }
                ensure!(
                    path.paint_order() == usvg::PaintOrder::FillAndStroke,
                    "This SVG draws a stroke behind its fill. Save the SVG to preserve its appearance."
                );
                let mut data = PathData::new();
                for segment in path.data().segments() {
                    ensure!(
                        *remaining_segments > 0,
                        "The SVG contains too many total control points for editable placement."
                    );
                    *remaining_segments -= 1;
                    use usvg::tiny_skia_path::PathSegment;
                    match segment {
                        PathSegment::MoveTo(point) => {
                            data.move_to(point.x as f64, point.y as f64);
                        }
                        PathSegment::LineTo(point) => {
                            data.line_to(point.x as f64, point.y as f64);
                        }
                        PathSegment::QuadTo(control, point) => {
                            data.quad_to(
                                control.x as f64,
                                control.y as f64,
                                point.x as f64,
                                point.y as f64,
                            );
                        }
                        PathSegment::CubicTo(first, second, point) => {
                            data.cubic_to(
                                first.x as f64,
                                first.y as f64,
                                second.x as f64,
                                second.y as f64,
                                point.x as f64,
                                point.y as f64,
                            );
                        }
                        PathSegment::Close => {
                            data.close();
                        }
                    }
                }
                let mut vector = VectorNode {
                    path: data,
                    ..VectorNode::default()
                };
                if let Some(fill) = path.fill() {
                    vector.path.fill_rule = if fill.rule() == usvg::FillRule::EvenOdd {
                        FillRule::EvenOdd
                    } else {
                        FillRule::NonZero
                    };
                    vector.fills.push(svg_paint(
                        fill.paint(),
                        fill.opacity().get(),
                        path.bounding_box(),
                    )?);
                }
                if let Some(stroke) = path.stroke() {
                    ensure!(
                        stroke.dashoffset() == 0.,
                        "This SVG offsets its stroke dashes. Save the SVG to preserve them."
                    );
                    ensure!(
                        stroke.linejoin() != usvg::LineJoin::MiterClip,
                        "This SVG uses clipped stroke joins. Save the SVG to preserve them."
                    );
                    let mut converted = Stroke::solid(Color::BLACK, stroke.width().get() as f64);
                    converted.paint =
                        svg_paint(stroke.paint(), stroke.opacity().get(), path.bounding_box())?;
                    converted.cap = match stroke.linecap() {
                        usvg::LineCap::Butt => StrokeCap::Butt,
                        usvg::LineCap::Round => StrokeCap::Round,
                        usvg::LineCap::Square => StrokeCap::Square,
                    };
                    converted.join = match stroke.linejoin() {
                        usvg::LineJoin::Round => StrokeJoin::Round,
                        usvg::LineJoin::Bevel => StrokeJoin::Bevel,
                        _ => StrokeJoin::Miter,
                    };
                    converted.miter_limit = stroke.miterlimit().get() as f64;
                    converted.dash = stroke
                        .dasharray()
                        .unwrap_or_default()
                        .iter()
                        .map(|value| *value as f64)
                        .collect();
                    vector.strokes.push(converted);
                }
                let mut node = CanvasNode::new(NodeData::Vector(vector));
                node.name = if path.id().is_empty() {
                    "Vector path".into()
                } else {
                    path.id().to_owned()
                };
                node
            }
            _ => bail!(
                "This SVG includes content that cannot be imported as editable paths. Save the SVG instead."
            ),
        };
        let transform = svg_transform(child.abs_transform());
        ensure!(
            transform.is_finite() && parent_transform.0.matrix2.determinant().abs() > f64::EPSILON,
            "The SVG contains an invalid transform."
        );
        node.transform = transform.then(&parent_transform.inverse());
        node.parent = Some(parent);
        node.index = index;
        index = IndexKey::after(index);
        let id = node.id;
        nodes.push(node);
        if let usvg::Node::Group(group) = child {
            append_svg_group(group, id, transform, nodes, depth + 1, remaining_segments)?;
        }
    }
    Ok(())
}

fn svg_transform(transform: usvg::Transform) -> Transform2D {
    Transform2D::from_components([
        transform.sx as f64,
        transform.ky as f64,
        transform.kx as f64,
        transform.sy as f64,
        transform.tx as f64,
        transform.ty as f64,
    ])
}

fn svg_paint(paint: &usvg::Paint, opacity: f32, bounds: usvg::Rect) -> Result<Fill> {
    let color = |color: usvg::Color, alpha: f32| {
        Color::rgba(
            color.red,
            color.green,
            color.blue,
            (alpha.clamp(0., 1.) * 255.).round() as u8,
        )
    };
    let normalize = |x: f32, y: f32, transform: usvg::Transform| {
        let point = svg_transform(transform).transform_point(glam::DVec2::new(x as f64, y as f64));
        [
            ((point.x - bounds.x() as f64) / bounds.width().max(f32::EPSILON) as f64) as f32,
            ((point.y - bounds.y() as f64) / bounds.height().max(f32::EPSILON) as f64) as f32,
        ]
    };
    let stops = |gradient: &usvg::BaseGradient| {
        gradient
            .stops()
            .iter()
            .map(|stop| GradientStop {
                position: stop.offset().get(),
                color: color(stop.color(), opacity * stop.opacity().get()),
            })
            .collect()
    };
    let gradient = match paint {
        usvg::Paint::Color(value) => return Ok(Fill::solid(color(*value, opacity))),
        usvg::Paint::LinearGradient(gradient) => {
            ensure!(
                gradient.spread_method() == usvg::SpreadMethod::Pad,
                "Repeating gradients cannot be imported faithfully yet. Save the SVG instead."
            );
            Gradient::Linear {
                start: normalize(gradient.x1(), gradient.y1(), gradient.transform()),
                end: normalize(gradient.x2(), gradient.y2(), gradient.transform()),
                stops: stops(gradient),
            }
        }
        usvg::Paint::RadialGradient(gradient) => {
            ensure!(
                gradient.spread_method() == usvg::SpreadMethod::Pad
                    && (gradient.fx() - gradient.cx()).abs() < 0.001
                    && (gradient.fy() - gradient.cy()).abs() < 0.001,
                "This SVG has a radial gradient that cannot be imported faithfully yet. Save the SVG instead."
            );
            let center = normalize(gradient.cx(), gradient.cy(), gradient.transform());
            let first = normalize(
                gradient.cx() + gradient.r().get(),
                gradient.cy(),
                gradient.transform(),
            );
            let second = normalize(
                gradient.cx(),
                gradient.cy() + gradient.r().get(),
                gradient.transform(),
            );
            Gradient::Radial {
                center,
                radius: ((first[0] - center[0]).powi(2) + (first[1] - center[1]).powi(2)).sqrt(),
                handles: Some([first, second]),
                stops: stops(gradient),
            }
        }
        usvg::Paint::Pattern(_) => {
            bail!("Pattern fills cannot be imported as editable vectors yet. Save the SVG instead.")
        }
    };
    Ok(Fill::Gradient {
        gradient,
        blend: BlendMode::Normal,
    })
}

pub(crate) fn place_svg(
    document: &mut FigDocument,
    mut artwork: VectorArtwork,
    x: f64,
    y: f64,
    provenance: Option<Value>,
) -> (Result<()>, DocChange) {
    let parent = document.doc.active_page();
    let mut transaction = Transaction::new("Place generated vectors");
    for mut node in artwork.nodes.drain(..) {
        if node.id == artwork.root {
            node.parent = parent;
            node.index = document.doc.scene.next_child_index(parent);
            node.transform = Transform2D::translation(x, y);
            if let Some(provenance) = provenance.clone() {
                node.meta = provenance;
            }
        }
        transaction.push(Operation::create_node(node));
    }
    match document.doc.apply_transaction(transaction) {
        Ok(()) => (Ok(()), DocChange::Content),
        Err(error) => (Err(error.into()), DocChange::None),
    }
}

#[derive(Clone, Debug)]
pub(crate) struct VideoMetadata {
    pub width: u32,
    pub height: u32,
    pub duration_us: i64,
}

#[derive(Clone)]
pub(crate) struct VideoPoster {
    pub png: Arc<[u8]>,
    pub time_us: i64,
}

#[derive(Clone)]
pub(crate) struct PreparedVideo {
    pub bytes: Arc<[u8]>,
    pub metadata: VideoMetadata,
    pub poster: Option<VideoPoster>,
}

async fn video_metadata_for_preparation(bytes: &Arc<[u8]>) -> Result<VideoMetadata> {
    let metadata = mp4_metadata(bytes)?;
    ensure!(
        u64::from(metadata.width) * u64::from(metadata.height) <= 32 * 1024 * 1024,
        "The video resolution is too large to preview. Save it to open it in a video player."
    );
    #[cfg(target_os = "macos")]
    let metadata = {
        // Track durations can include samples excluded by an MP4 edit list.
        // Placement and trimming must use the same timeline as the player.
        let playback = media::video::prepare_video_playback(bytes.clone(), 1200)?.await?;
        VideoMetadata {
            duration_us: i64::try_from(playback.info().duration_us)
                .context("The video playback duration is invalid.")?,
            ..metadata
        }
    };
    Ok(metadata)
}

pub(crate) async fn prepare_video(bytes: Arc<[u8]>) -> Result<PreparedVideo> {
    let metadata = video_metadata_for_preparation(&bytes).await?;
    #[cfg(target_os = "macos")]
    let poster = {
        let frame = media::video::video_frame(bytes.clone(), 1200)?.await?;
        let difference = (i64::from(frame.width) * i64::from(metadata.height)
            - i64::from(frame.height) * i64::from(metadata.width))
        .abs();
        ensure!(
            difference <= 2 * i64::from(metadata.width.max(metadata.height)),
            "The decoded video orientation does not match its track. Save it to open it in a video player."
        );
        let pixels = image::RgbaImage::from_raw(frame.width, frame.height, frame.rgba)
            .context("The video preview has invalid pixels.")?;
        let mut png = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(pixels).write_to(&mut png, image::ImageFormat::Png)?;
        let time_us = i64::try_from(frame.actual_time_us)
            .context("The video preview timestamp is invalid.")?;
        ensure!(
            time_us < metadata.duration_us,
            "The video preview is outside the clip."
        );
        Some(VideoPoster {
            png: png.into_inner().into(),
            time_us,
        })
    };
    #[cfg(not(target_os = "macos"))]
    let poster = None;
    Ok(PreparedVideo {
        bytes,
        metadata,
        poster,
    })
}

#[derive(Clone)]
pub(crate) struct PreparedVideoTrim {
    pub range_us: [i64; 2],
    pub source_duration_us: i64,
    pub poster: VideoPoster,
}

pub(crate) fn validate_video_trim(range: [i64; 2], duration_us: i64) -> Result<()> {
    ensure!(
        range[0] >= 0 && range[0] < range[1] && range[1] <= duration_us,
        "Choose a start before the end, within the original video duration."
    );
    Ok(())
}

#[cfg(target_os = "macos")]
pub(crate) async fn prepare_video_trim(
    bytes: Arc<[u8]>,
    range_us: [i64; 2],
) -> Result<PreparedVideoTrim> {
    let metadata = video_metadata_for_preparation(&bytes).await?;
    validate_video_trim(range_us, metadata.duration_us)?;
    let frame = media::video::video_frame_at(bytes, 1200, u64::try_from(range_us[0])?)?.await?;
    let difference = (i64::from(frame.width) * i64::from(metadata.height)
        - i64::from(frame.height) * i64::from(metadata.width))
    .abs();
    ensure!(
        difference <= 2 * i64::from(metadata.width.max(metadata.height)),
        "The decoded video orientation does not match its track."
    );
    ensure!(
        frame.actual_time_us < u64::try_from(range_us[1])?,
        "The video preview is outside the selected range."
    );
    let pixels = image::RgbaImage::from_raw(frame.width, frame.height, frame.rgba)
        .context("The video preview has invalid pixels.")?;
    let mut png = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(pixels).write_to(&mut png, image::ImageFormat::Png)?;
    Ok(PreparedVideoTrim {
        range_us,
        source_duration_us: metadata.duration_us,
        poster: VideoPoster {
            png: png.into_inner().into(),
            // This is the chosen source position. A containing VFR sample can
            // begin earlier; its sample timestamp must not move the trim start.
            time_us: range_us[0],
        },
    })
}

pub(crate) fn trim_video(
    document: &mut FigDocument,
    node: NodeId,
    expected: &VideoNode,
    trim: PreparedVideoTrim,
) -> (Result<()>, DocChange) {
    let result = (|| {
        validate_video_trim(trim.range_us, trim.source_duration_us)?;
        ensure!(
            expected.speed == 1.,
            "Trimming currently supports normal-speed video only."
        );
        ensure!(
            trim.poster.time_us == trim.range_us[0],
            "The poster must match the new trim start."
        );
        let current = document
            .doc
            .scene
            .get(node)
            .context("The video layer no longer exists.")?;
        ensure!(
            current.data == NodeData::Video(expected.clone()),
            "The video changed while its preview was loading. Apply the trim again."
        );
        ensure!(
            crate::clipboard::node_is_on_active_page(&document.doc, node),
            "Return to this video's page before trimming it."
        );
        ensure!(
            !current
                .flags
                .intersects(fanta_doc::NodeFlags::LOCKED | fanta_doc::NodeFlags::HIDDEN)
                && !document.doc.scene.ancestors_of(node).any(|parent| parent
                    .flags
                    .intersects(fanta_doc::NodeFlags::LOCKED | fanta_doc::NodeFlags::HIDDEN)),
            "Unlock and show this video layer before trimming it."
        );
        if expected.time_range_us == trim.range_us {
            return Ok(false);
        }
        let (poster, _) = document
            .doc_and_assets()
            .1
            .add_image(trim.poster.png.to_vec())
            .context("The trimmed video preview could not be added.")?;
        let mut updated = expected.clone();
        updated.time_range_us = trim.range_us;
        updated.poster_frame_us = Some(trim.poster.time_us);
        updated.poster = Some(poster);
        match document.doc.apply(Operation::ReplaceData {
            id: node,
            old: Box::new(NodeData::Video(expected.clone())),
            new: Box::new(NodeData::Video(updated)),
        }) {
            Ok(()) => Ok(true),
            Err(error) => {
                document.doc_and_assets().1.remove(poster);
                Err(error.into())
            }
        }
    })();
    match result {
        Ok(changed) => (
            Ok(()),
            if changed {
                DocChange::Content
            } else {
                DocChange::None
            },
        ),
        Err(error) => (Err(error), DocChange::None),
    }
}

pub(crate) fn mp4_metadata(bytes: &[u8]) -> Result<VideoMetadata> {
    let top = mp4_boxes(bytes)?;
    ensure!(
        top.iter().any(|(kind, _)| kind == b"ftyp"),
        "The generated file is not an MP4 video."
    );
    let movie = top
        .iter()
        .find(|(kind, _)| kind == b"moov")
        .context("The MP4 has no playable movie metadata.")?
        .1;
    for (_, track) in mp4_boxes(movie)?
        .into_iter()
        .filter(|(kind, _)| kind == b"trak")
    {
        let boxes = mp4_boxes(track)?;
        let Some((_, header)) = boxes.iter().find(|(kind, _)| kind == b"tkhd") else {
            continue;
        };
        let Some((_, media)) = boxes.iter().find(|(kind, _)| kind == b"mdia") else {
            continue;
        };
        let media = mp4_boxes(media)?;
        let Some((_, handler)) = media.iter().find(|(kind, _)| kind == b"hdlr") else {
            continue;
        };
        if handler.get(8..12) != Some(b"vide") {
            continue;
        }
        let (width, height) = mp4_display_dimensions(header)?;
        let duration_header = media
            .iter()
            .find(|(kind, _)| kind == b"mdhd")
            .context("The video duration is missing.")?
            .1;
        let version = *duration_header
            .first()
            .context("The video header is empty.")?;
        let (timescale, duration) = match version {
            0 => (
                be_u32(duration_header, 12)?,
                be_u32(duration_header, 16)? as u64,
            ),
            1 => (be_u32(duration_header, 20)?, be_u64(duration_header, 24)?),
            _ => bail!("The MP4 uses an unsupported timing format."),
        };
        ensure!(
            width > 0 && height > 0 && width <= 16384 && height <= 16384 && timescale > 0,
            "The video dimensions or timing are invalid."
        );
        let duration_us = duration
            .checked_mul(1_000_000)
            .context("The video is too long.")?
            / timescale as u64;
        ensure!(
            duration_us > 0 && duration_us <= 24 * 60 * 60 * 1_000_000,
            "The video duration is invalid."
        );
        return Ok(VideoMetadata {
            width,
            height,
            duration_us: duration_us as i64,
        });
    }
    bail!("The MP4 contains no supported video track.")
}

fn mp4_display_dimensions(header: &[u8]) -> Result<(u32, u32)> {
    let matrix_offset = match header.first() {
        Some(0) => 40,
        Some(1) => 52,
        _ => bail!("The MP4 uses an unsupported track header."),
    };
    let width = f64::from(be_u32(header, matrix_offset + 36)?) / 65536.;
    let height = f64::from(be_u32(header, matrix_offset + 40)?) / 65536.;
    ensure!(
        width > 0. && height > 0. && width <= 16384. && height <= 16384.,
        "The video dimensions are invalid."
    );
    let mut matrix = [0; 9];
    for (index, value) in matrix.iter_mut().enumerate() {
        *value = be_u32(header, matrix_offset + index * 4)? as i32;
    }
    let [a, b, u, c, d, v, _, _, w] = matrix;
    ensure!(
        u == 0 && v == 0 && w == 1 << 30,
        "The video uses an unsupported perspective transform."
    );
    let unit = |value: i32| matches!(value, -65536 | 65536);
    ensure!(
        (unit(a) && b == 0 && c == 0 && unit(d)) || (a == 0 && unit(b) && unit(c) && d == 0),
        "The video uses an unsupported scale or rotation. Save it to open it in a video player."
    );
    let a = f64::from(a) / 65536.;
    let b = f64::from(b) / 65536.;
    let c = f64::from(c) / 65536.;
    let d = f64::from(d) / 65536.;
    ensure!(
        (a * d - b * c).abs() > f64::EPSILON,
        "The video transform is invalid."
    );
    let display_width = (width * a.abs() + height * c.abs()).ceil();
    let display_height = (width * b.abs() + height * d.abs()).ceil();
    ensure!(
        display_width > 0.
            && display_height > 0.
            && display_width <= 16384.
            && display_height <= 16384.,
        "The transformed video dimensions are invalid."
    );
    Ok((display_width as u32, display_height as u32))
}

fn be_u32(bytes: &[u8], offset: usize) -> Result<u32> {
    let bytes = bytes
        .get(offset..offset + 4)
        .context("The MP4 header is truncated.")?;
    Ok(u32::from_be_bytes(bytes.try_into()?))
}
fn be_u64(bytes: &[u8], offset: usize) -> Result<u64> {
    let bytes = bytes
        .get(offset..offset + 8)
        .context("The MP4 header is truncated.")?;
    Ok(u64::from_be_bytes(bytes.try_into()?))
}
fn mp4_boxes(mut bytes: &[u8]) -> Result<Vec<([u8; 4], &[u8])>> {
    let mut result = Vec::new();
    while !bytes.is_empty() {
        ensure!(
            result.len() < 10000,
            "The MP4 contains too many metadata entries."
        );
        let size = be_u32(bytes, 0)?;
        let kind: [u8; 4] = bytes
            .get(4..8)
            .context("The MP4 header is truncated.")?
            .try_into()?;
        let (size, header) = match size {
            0 => (bytes.len(), 8),
            1 => (usize::try_from(be_u64(bytes, 8)?)?, 16),
            size => (size as usize, 8),
        };
        ensure!(
            size >= header && size <= bytes.len(),
            "The MP4 contains an invalid metadata size."
        );
        result.push((kind, &bytes[header..size]));
        bytes = &bytes[size..];
    }
    Ok(result)
}

pub(crate) fn place_video(
    document: &mut FigDocument,
    video: PreparedVideo,
    x: f64,
    y: f64,
    provenance: Option<Value>,
) -> (Result<()>, DocChange) {
    let poster_asset = match video.poster.as_ref() {
        Some(poster) => match document.doc_and_assets().1.add_image(poster.png.to_vec()) {
            Ok((asset, _)) => Some(asset),
            Err(error) => {
                return (
                    Err(error.context("The video preview could not be added.")),
                    DocChange::None,
                );
            }
        },
        None => None,
    };
    let asset = AssetId::new();
    let mut node = CanvasNode::new(NodeData::Video(video_node_data(
        &video,
        asset,
        poster_asset,
    )));
    node.name = "Generated video".into();
    node.parent = document.doc.active_page();
    node.index = document.doc.scene.next_child_index(node.parent);
    node.transform = Transform2D::translation(x, y);
    if let Some(provenance) = provenance {
        node.meta = provenance;
    }
    match document.doc.apply(Operation::create_node(node)) {
        Ok(()) => {
            Arc::make_mut(&mut document.raw_assets).insert(asset, video.bytes.to_vec());
            (Ok(()), DocChange::Content)
        }
        Err(error) => {
            if let Some(asset) = poster_asset {
                document.doc_and_assets().1.remove(asset);
            }
            (Err(error.into()), DocChange::None)
        }
    }
}

fn video_node_data(
    video: &PreparedVideo,
    asset: AssetId,
    poster_asset: Option<AssetId>,
) -> VideoNode {
    VideoNode {
        asset,
        natural_size: [video.metadata.width, video.metadata.height],
        local_size: [video.metadata.width as f64, video.metadata.height as f64],
        time_range_us: [0, video.metadata.duration_us],
        speed: 1.,
        muted: false,
        volume: 1.,
        poster_frame_us: video.poster.as_ref().map(|poster| poster.time_us),
        poster: poster_asset,
        fit: ImageFitMode::Fit,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn local_png(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            width,
            height,
            image::Rgba([32, 96, 192, 255]),
        ))
        .write_to(&mut bytes, image::ImageFormat::Png)
        .expect("PNG fixture");
        bytes.into_inner()
    }

    async fn local_media_item(
        cx: &mut gpui::TestAppContext,
    ) -> (gpui::Entity<crate::document::FigItem>, NodeId) {
        cx.update(|cx| {
            let settings = settings::SettingsStore::test(cx);
            cx.set_global(settings);
        });
        let project = project::Project::test(project::FakeFs::new(cx.executor()), [], cx).await;
        let mut doc = fanta_doc::Doc::new();
        let mut page = CanvasNode::new(NodeData::Group(GroupNode::default()));
        page.transform = Transform2D::scale(2.).then(&Transform2D::translation(100., 50.));
        let page_id = page.id;
        doc.apply(Operation::create_node(page)).expect("page");
        doc.add_page(page_id);
        doc.selection.select_only(page_id);
        let item =
            crate::document::ready_item_for_test(&project, "/tmp/Local-media.fig".into(), doc, cx);
        (item, page_id)
    }

    #[test]
    fn local_media_preparation_rejects_unsupported_corrupt_and_oversized_files() {
        let directory = tempfile::tempdir().expect("directory");
        for (name, bytes) in [
            ("unsupported.svg", b"<svg/>".to_vec()),
            ("corrupt.png", b"not an image".to_vec()),
            ("corrupt.mp4", b"not a video".to_vec()),
            ("too-wide.png", local_png(16_385, 1)),
        ] {
            let path = directory.path().join(name);
            std::fs::write(&path, bytes).expect("fixture");
            assert!(
                futures::executor::block_on(prepare_local_media(&path)).is_err(),
                "{name} must fail before placement"
            );
        }
        for (name, limit) in [
            ("huge.png", crate::document::MAX_IMAGE_SOURCE_BYTES),
            ("huge.mp4", MAX_LOCAL_VIDEO_BYTES),
        ] {
            let path = directory.path().join(name);
            File::create(&path)
                .expect("sparse fixture")
                .set_len(limit as u64 + 1)
                .expect("oversized length");
            let error = futures::executor::block_on(prepare_local_media(&path))
                .err()
                .expect("oversized file rejected");
            assert!(error.to_string().contains("file limit"), "{error:#}");
        }
        let bytes = local_png(2, 2);
        assert!(validate_local_image(bytes.get(..33).expect("PNG header")).is_err());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn local_media_preparation_decodes_real_mp4_and_rejects_corrupt_video() {
        let directory = tempfile::tempdir().expect("directory");
        let path = directory.path().join("Camera take.MP4");
        let bytes = include_bytes!("../../media/test_fixtures/quadrants.mp4");
        std::fs::write(&path, bytes).expect("video fixture");
        let prepared = futures::executor::block_on(prepare_local_media(&path))
            .expect("native MP4 preparation");
        assert_eq!(prepared.name, "Camera take.MP4");
        let LocalMediaContent::Video(video) = prepared.content else {
            panic!("prepared video");
        };
        assert_eq!(video.bytes.as_ref(), bytes);
        assert!(video.poster.is_some());
        let raw_metadata = mp4_metadata(bytes).expect("raw MP4 track metadata");
        let playback = futures::executor::block_on(
            media::video::prepare_video_playback(video.bytes.clone(), 1200)
                .expect("native playback request"),
        )
        .expect("native playback metadata");
        let playable_duration =
            i64::try_from(playback.info().duration_us).expect("playback duration fits");
        assert_eq!(raw_metadata.duration_us, 1_250_000);
        assert_eq!(playable_duration, 1_000_000);
        assert_eq!(video.metadata.duration_us, playable_duration);
        let placed_video = video_node_data(&video, AssetId::new(), None);
        assert_eq!(placed_video.time_range_us, [0, playable_duration]);
        validate_video_trim(placed_video.time_range_us, playable_duration)
            .expect("placed range fits the native playback timeline");
        assert!(validate_video_trim([0, raw_metadata.duration_us], playable_duration).is_err());
        let trim = futures::executor::block_on(prepare_video_trim(
            video.bytes.clone(),
            [250_000, playable_duration],
        ))
        .expect("trim within the playable timeline");
        assert_eq!(trim.source_duration_us, playable_duration);
        assert!(
            futures::executor::block_on(prepare_video_trim(
                video.bytes,
                [0, raw_metadata.duration_us],
            ))
            .is_err()
        );
        std::fs::write(
            &path,
            include_bytes!("../../media/test_fixtures/quadrants-corrupt.mp4"),
        )
        .expect("corrupt video fixture");
        assert!(futures::executor::block_on(prepare_local_media(&path)).is_err());
    }

    #[gpui::test]
    async fn local_media_placement_preserves_name_assets_and_world_bounds_with_one_undo(
        cx: &mut gpui::TestAppContext,
    ) {
        let (item, page_id) = local_media_item(cx).await;
        let directory = tempfile::tempdir().expect("directory");
        let path = directory.path().join("Reference artwork.PNG");
        let image_bytes = local_png(100, 50);
        std::fs::write(&path, &image_bytes).expect("image fixture");
        let image = prepare_local_media(&path).await.expect("image prepared");
        let video_bytes: Arc<[u8]> = Arc::from(&b"retained source video"[..]);
        let video = PreparedLocalMedia {
            name: "Camera take.mp4".to_owned(),
            content: LocalMediaContent::Video(PreparedVideo {
                bytes: video_bytes.clone(),
                metadata: VideoMetadata {
                    width: 100,
                    height: 50,
                    duration_us: 2_000_000,
                },
                poster: Some(VideoPoster {
                    png: local_png(2, 1).into(),
                    time_us: 0,
                }),
            }),
        };
        for (prepared, expected_name, expected_bytes) in [
            (image, "Reference artwork.PNG", image_bytes.as_slice()),
            (video, "Camera take.mp4", video_bytes.as_ref()),
        ] {
            item.update(cx, |item, cx| {
                item.with_document(cx, |document| {
                    let depth = document.doc.history.undo_depth();
                    let (result, change) =
                        place_local_media(document, prepared, [80., 60.], [50., 50.]);
                    let id = result.expect("place local media");
                    assert_eq!(change, DocChange::Content);
                    assert_eq!(document.doc.history.undo_depth(), depth + 1);
                    assert_eq!(document.doc.selection.as_slice(), &[id]);
                    let node = document.doc.scene.get(id).expect("placed layer").clone();
                    assert_eq!(node.name, expected_name);
                    assert_eq!(node.parent, Some(page_id));
                    assert_eq!(node.meta, Value::Null);
                    let asset = match &node.data {
                        NodeData::Bitmap(bitmap) => {
                            assert_eq!(bitmap.natural_size, [100, 50]);
                            assert_eq!(bitmap.local_size, [40., 20.]);
                            bitmap.asset
                        }
                        NodeData::Video(video) => {
                            assert_eq!(video.natural_size, [100, 50]);
                            assert_eq!(video.local_size, [40., 20.]);
                            assert_eq!(video.time_range_us, [0, 2_000_000]);
                            assert!(
                                document
                                    .raw_assets
                                    .contains_key(&video.poster.expect("poster"))
                            );
                            video.asset
                        }
                        _ => panic!("media node"),
                    };
                    assert_eq!(
                        document.raw_assets.get(&asset).expect("embedded asset"),
                        expected_bytes
                    );
                    let world = document.doc.scene.world_bounds(id).expect("world bounds");
                    assert_eq!(
                        (world.min_x, world.min_y, world.max_x, world.max_y),
                        (60., 50., 100., 70.)
                    );
                    assert!(document.doc.undo().expect("undo"));
                    assert!(document.doc.scene.get(id).is_none());
                    assert_eq!(document.doc.history.undo_depth(), depth);
                    assert!(document.doc.redo().expect("redo"));
                    assert_eq!(document.doc.scene.get(id), Some(&node));
                    assert_eq!(
                        document
                            .raw_assets
                            .get(&asset)
                            .expect("asset retained for redo"),
                        expected_bytes
                    );
                    ((), DocChange::Content)
                })
                .expect("document");
            });
        }
    }

    #[gpui::test]
    async fn local_media_placement_failures_preserve_existing_assets_selection_and_history(
        cx: &mut gpui::TestAppContext,
    ) {
        let (item, page_id) = local_media_item(cx).await;
        let image = PreparedLocalMedia {
            name: "Reference.png".to_owned(),
            content: LocalMediaContent::Image {
                bytes: local_png(2, 2).into(),
                natural_size: [2, 2],
            },
        };
        item.update(cx, |item, cx| {
            item.with_document(cx, |document| {
                document
                    .doc_and_assets()
                    .1
                    .add_image(local_png(1, 1))
                    .expect("existing image");
                let assets = document.raw_assets.clone();
                let depth = document.doc.history.undo_depth();
                let selection = document.doc.selection.as_slice().to_vec();
                for case in [
                    "locked",
                    "hidden",
                    "singular",
                    "non-page",
                    "missing",
                    "invalid-bounds",
                    "corrupt-poster",
                ] {
                    let original_root = document.doc.scene.get(page_id).expect("page").clone();
                    let mut prepared = image.clone();
                    let mut center = [0., 0.];
                    match case {
                        "locked" => document
                            .doc
                            .scene
                            .get_mut(page_id)
                            .expect("page")
                            .flags
                            .insert(fanta_doc::NodeFlags::LOCKED),
                        "hidden" => {
                            document
                                .pages
                                .iter_mut()
                                .find(|page| page.root == Some(page_id))
                                .expect("page")
                                .hidden = true
                        }
                        "singular" => {
                            document.doc.scene.get_mut(page_id).expect("page").transform =
                                Transform2D::scale(0.)
                        }
                        "non-page" => document.doc.pages.clear(),
                        "missing" => {
                            document.doc.set_active_page(None);
                        }
                        "invalid-bounds" => center[0] = f64::NAN,
                        "corrupt-poster" => {
                            prepared.content = LocalMediaContent::Video(PreparedVideo {
                                bytes: Arc::from(&b"source"[..]),
                                metadata: VideoMetadata {
                                    width: 2,
                                    height: 2,
                                    duration_us: 1000,
                                },
                                poster: Some(VideoPoster {
                                    png: Arc::from(&b"bad poster"[..]),
                                    time_us: 0,
                                }),
                            })
                        }
                        _ => unreachable!(),
                    }
                    let (result, change) =
                        place_local_media(document, prepared, center, [100., 100.]);
                    assert!(result.is_err(), "{case}");
                    assert_eq!(change, DocChange::None, "{case}");
                    assert_eq!(document.raw_assets, assets, "{case}");
                    assert_eq!(
                        document.doc.selection.as_slice(),
                        selection.as_slice(),
                        "{case}"
                    );
                    assert_eq!(document.doc.history.undo_depth(), depth, "{case}");
                    *document.doc.scene.get_mut(page_id).expect("page") = original_root;
                    document.doc.pages = vec![page_id];
                    document.doc.set_active_page(Some(page_id));
                    document
                        .pages
                        .iter_mut()
                        .find(|page| page.root == Some(page_id))
                        .expect("page")
                        .hidden = false;
                }
                ((), DocChange::None)
            })
            .expect("document");
        });
    }

    #[test]
    fn output_partial_write_failure_preserves_existing_file() {
        let directory = tempfile::tempdir().expect("output directory");
        let destination = directory.path().join("result.mp4");
        std::fs::write(&destination, b"previous complete video").expect("existing output");

        let error = write_output_with(&destination, |file| {
            file.write_all(b"incomplete new video")?;
            Err(std::io::Error::other("injected write failure"))
        })
        .expect_err("partial write must fail");

        assert!(error.to_string().contains("injected write failure"));
        assert_eq!(
            std::fs::read(&destination).expect("previous output remains readable"),
            b"previous complete video"
        );
        assert_eq!(
            std::fs::read_dir(directory.path())
                .expect("output directory")
                .count(),
            1,
            "failed writes must not leave temporary files"
        );
    }

    #[test]
    fn output_write_replaces_existing_file_with_exact_bytes() {
        let directory = tempfile::tempdir().expect("output directory");
        let destination = directory.path().join("result.png");
        std::fs::write(&destination, b"a longer previous image").expect("existing output");
        let expected = b"\x89PNG\r\n\x1a\n\0complete image";

        write_output(&destination, expected).expect("save replacement");

        assert_eq!(std::fs::read(&destination).expect("saved output"), expected);
        assert_eq!(
            std::fs::read_dir(directory.path())
                .expect("output directory")
                .count(),
            1
        );
    }

    #[test]
    fn svg_import_keeps_editable_paths_and_fill_rules() {
        let artwork = parse_svg(br##"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="100"><g opacity="0.5" transform="translate(10 5)"><path d="M0 0H20V20H0Z" fill="#ff0000" fill-rule="evenodd"/></g></svg>"##).expect("valid SVG");
        assert_eq!((artwork.width, artwork.height), (200., 100.));
        let vector = artwork
            .nodes
            .iter()
            .find_map(|node| match &node.data {
                NodeData::Vector(vector) => Some(vector),
                _ => None,
            })
            .expect("editable path");
        assert_eq!(vector.path.fill_rule, FillRule::EvenOdd);
        assert_eq!(
            vector.fills.first(),
            Some(&Fill::solid(Color::rgb(255, 0, 0)))
        );
        assert!(artwork.nodes.iter().any(|node| node.opacity.get() == 0.5));
    }

    #[test]
    fn svg_import_bounds_expanded_geometry_across_reused_paths() {
        let path = format!("M0 0 {} Z", "L1 1 L2 0 ".repeat(250));
        let instances = r##"<use href="#shape"/>"##.repeat(250);
        let svg = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="20"><defs><path id="shape" d="{path}"/></defs>{instances}</svg>"#
        );
        let error = parse_svg(svg.as_bytes())
            .err()
            .expect("expanded paths exceed budget");
        assert!(error.to_string().contains("total control points"));
    }

    #[test]
    fn svg_import_rejects_embedded_images_and_filters_without_partial_conversion() {
        assert!(parse_svg(br#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><image href="file:///private/image.png"/></svg>"#).is_err());
        assert!(parse_svg(br#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><text>hello</text></svg>"#).is_err());
    }

    #[test]
    fn mp4_metadata_reads_video_track_size_and_duration() {
        fn atom(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
            let mut bytes = ((body.len() + 8) as u32).to_be_bytes().to_vec();
            bytes.extend_from_slice(kind);
            bytes.extend_from_slice(body);
            bytes
        }
        let mut track = vec![0; 84];
        track[40..44].copy_from_slice(&65536u32.to_be_bytes());
        track[56..60].copy_from_slice(&65536u32.to_be_bytes());
        track[72..76].copy_from_slice(&(1u32 << 30).to_be_bytes());
        track[76..80].copy_from_slice(&(1280u32 << 16).to_be_bytes());
        track[80..84].copy_from_slice(&(720u32 << 16).to_be_bytes());
        let mut media_header = vec![0; 24];
        media_header[12..16].copy_from_slice(&16000u32.to_be_bytes());
        media_header[16..20].copy_from_slice(&80000u32.to_be_bytes());
        let mut handler = vec![0; 12];
        handler[8..12].copy_from_slice(b"vide");
        let mut media = atom(b"mdhd", &media_header);
        media.extend(atom(b"hdlr", &handler));
        let mut track_bytes = atom(b"tkhd", &track);
        track_bytes.extend(atom(b"mdia", &media));
        let mut file = atom(b"ftyp", b"isom");
        file.extend(atom(b"moov", &atom(b"trak", &track_bytes)));
        let metadata = mp4_metadata(&file).expect("valid video metadata");
        assert_eq!(
            (metadata.width, metadata.height, metadata.duration_us),
            (1280, 720, 5_000_000)
        );
    }

    #[test]
    fn mp4_display_dimensions_apply_rotation_reflection_and_version_one_headers() {
        let transforms: [([i32; 4], (u32, u32)); 6] = [
            ([1, 0, 0, 1], (320, 180)),
            ([0, 1, -1, 0], (180, 320)),
            ([-1, 0, 0, -1], (320, 180)),
            ([0, -1, 1, 0], (180, 320)),
            ([-1, 0, 0, 1], (320, 180)),
            ([0, 1, 1, 0], (180, 320)),
        ];
        for version in [0, 1] {
            for ([a, b, c, d], expected) in transforms {
                let offset = if version == 0 { 40 } else { 52 };
                let mut header = vec![0; offset + 44];
                header[0] = version;
                let matrix = [
                    a * 65536,
                    b * 65536,
                    0,
                    c * 65536,
                    d * 65536,
                    0,
                    180 * 65536,
                    320 * 65536,
                    1 << 30,
                ];
                for (index, value) in matrix.into_iter().enumerate() {
                    header[offset + index * 4..offset + index * 4 + 4]
                        .copy_from_slice(&value.to_be_bytes());
                }
                header[offset + 36..offset + 40].copy_from_slice(&(320u32 << 16).to_be_bytes());
                header[offset + 40..offset + 44].copy_from_slice(&(180u32 << 16).to_be_bytes());
                assert_eq!(
                    mp4_display_dimensions(&header).expect("affine track"),
                    expected
                );
                header[offset + 8..offset + 12].copy_from_slice(&1u32.to_be_bytes());
                assert!(
                    mp4_display_dimensions(&header).is_err(),
                    "perspective must not be silently flattened"
                );
            }
        }
    }

    #[test]
    fn mp4_display_dimensions_reject_truncated_singular_and_oversized_tracks() {
        let mut header = vec![0; 84];
        header[40..44].copy_from_slice(&65536u32.to_be_bytes());
        header[56..60].copy_from_slice(&65536u32.to_be_bytes());
        header[72..76].copy_from_slice(&(1u32 << 30).to_be_bytes());
        header[76..80].copy_from_slice(&(320u32 << 16).to_be_bytes());
        header[80..84].copy_from_slice(&(180u32 << 16).to_be_bytes());
        for end in 0..header.len() {
            assert!(mp4_display_dimensions(&header[..end]).is_err());
        }
        header[40..44].copy_from_slice(&0u32.to_be_bytes());
        assert!(mp4_display_dimensions(&header).is_err());
        header[40..44].copy_from_slice(&(100u32 << 16).to_be_bytes());
        assert!(mp4_display_dimensions(&header).is_err());
        header[40..44].copy_from_slice(&65536u32.to_be_bytes());
        header[44..48].copy_from_slice(&32768u32.to_be_bytes());
        assert!(
            mp4_display_dimensions(&header).is_err(),
            "shear is not supported"
        );
        header[44..48].copy_from_slice(&0u32.to_be_bytes());
        header[40..44].copy_from_slice(&32768u32.to_be_bytes());
        header[56..60].copy_from_slice(&32768u32.to_be_bytes());
        assert!(
            mp4_display_dimensions(&header).is_err(),
            "uniform scale is not supported"
        );
    }

    #[test]
    fn mp4_metadata_rejects_truncated_and_invalid_boxes() {
        assert!(mp4_metadata(b"not a video").is_err());
        assert!(mp4_boxes(&[0, 0, 0, 1, b'm', b'o', b'o', b'v']).is_err());
        assert!(mp4_boxes(&[0, 0, 0, 4, b'm', b'o', b'o', b'v']).is_err());
    }
}
