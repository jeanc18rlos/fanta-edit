use anyhow::{Context as _, Result, bail, ensure};
use fanta_doc::{
    AssetId, BlendMode, CanvasNode, Color, Fill, FillRule, Gradient, GradientStop, GroupNode,
    ImageFitMode, IndexKey, NodeData, NodeId, Operation, PathData, Stroke, StrokeCap, StrokeJoin,
    Transaction, Transform2D, UnitInterval, VectorNode, VideoNode,
};
use serde_json::Value;
use std::{fs::File, io::Write as _, path::Path, sync::Arc};

use crate::document::{DocChange, FigDocument};

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
        node.index = index.clone();
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

#[derive(Debug)]
pub(crate) struct VideoMetadata {
    pub width: u32,
    pub height: u32,
    pub duration_us: i64,
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
        ensure!(header.len() >= 8, "The MP4 video track is incomplete.");
        let width = be_u32(header, header.len() - 8)? >> 16;
        let height = be_u32(header, header.len() - 4)? >> 16;
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
    bytes: Arc<[u8]>,
    metadata: VideoMetadata,
    x: f64,
    y: f64,
    provenance: Option<Value>,
) -> (Result<()>, DocChange) {
    let asset = AssetId::new();
    let mut node = CanvasNode::new(NodeData::Video(VideoNode {
        asset,
        natural_size: [metadata.width, metadata.height],
        local_size: [metadata.width as f64, metadata.height as f64],
        time_range_us: [0, metadata.duration_us],
        speed: 1.,
        muted: false,
        volume: 1.,
        poster_frame_us: None,
        poster: None,
        fit: ImageFitMode::Fit,
    }));
    node.name = "Generated video".into();
    node.parent = document.doc.active_page();
    node.index = document.doc.scene.next_child_index(node.parent);
    node.transform = Transform2D::translation(x, y);
    if let Some(provenance) = provenance {
        node.meta = provenance;
    }
    match document.doc.apply(Operation::create_node(node)) {
        Ok(()) => {
            Arc::make_mut(&mut document.raw_assets).insert(asset, bytes.to_vec());
            (Ok(()), DocChange::Content)
        }
        Err(error) => (Err(error.into()), DocChange::None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn mp4_metadata_rejects_truncated_and_invalid_boxes() {
        assert!(mp4_metadata(b"not a video").is_err());
        assert!(mp4_boxes(&[0, 0, 0, 1, b'm', b'o', b'o', b'v']).is_err());
        assert!(mp4_boxes(&[0, 0, 0, 4, b'm', b'o', b'o', b'v']).is_err());
    }
}
