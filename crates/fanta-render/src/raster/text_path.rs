use super::text::{to_text_style, with_layout_engine};
use super::{Bounds, Canvas, Paint, TextBuffer, TextStyle, to_sk_color};
use fanta_doc::{MeasuredPath, TextPathAlignment, TextPathDirection, TextPathNode, TextPathSide};
use fanta_text::ShapedGlyphRun;
use skia_safe::{Font, Matrix, Path, Point, Rect, paint::Style as PaintStyle};
use std::cell::RefCell;
use std::collections::BTreeMap;

const GEOMETRY_EPSILON: f64 = 1e-9;
const TEXT_PATH_CACHE_CAP: usize = 2048;

thread_local! {
    static TEXT_PATH_LAYOUT_CACHE: RefCell<
        std::collections::HashMap<Vec<u8>, Option<TextPathLayout>>,
    > = RefCell::new(std::collections::HashMap::new());
}

#[derive(Clone)]
struct SourceGlyph {
    line: usize,
    source_x: f64,
    source_y: f64,
    fallback_advance: f64,
    glyph_id: u16,
    bounds: [f64; 4],
    font: Font,
    style: TextStyle,
}

struct PlacedGlyph {
    glyph_id: u16,
    font: Font,
    transform: Matrix,
    color: fanta_doc::Color,
}

struct ColoredPath {
    path: Path,
    color: fanta_doc::Color,
}

struct TextPathLayout {
    glyphs: Vec<PlacedGlyph>,
    decorations: Vec<ColoredPath>,
    silhouette: Path,
}

pub(crate) fn draw_text_path_node(canvas: &Canvas, node: &TextPathNode) {
    let Some(()) = with_text_path_layout(node, |layout| {
        for glyph in &layout.glyphs {
            let mut paint = glyph_paint(glyph.color);
            paint.set_style(PaintStyle::Fill);
            canvas.save();
            canvas.concat(&glyph.transform);
            canvas.draw_glyphs_at(
                &[glyph.glyph_id],
                &[Point::new(0.0, 0.0)][..],
                Point::new(0.0, 0.0),
                &glyph.font,
                &paint,
            );
            canvas.restore();
        }

        for decoration in &layout.decorations {
            canvas.draw_path(&decoration.path, &glyph_paint(decoration.color));
        }
    }) else {
        return;
    };
}

pub(crate) fn text_path_outline(node: &TextPathNode) -> Option<Path> {
    with_text_path_layout(node, |layout| {
        (!layout.silhouette.is_empty()).then(|| layout.silhouette.clone())
    })?
}

pub(crate) fn text_path_bounds(node: &TextPathNode) -> Option<Bounds> {
    let outline = text_path_outline(node)?;
    let bounds = outline.compute_tight_bounds();
    let values = [bounds.left, bounds.top, bounds.right, bounds.bottom];
    if values.iter().any(|value| !value.is_finite())
        || bounds.width() <= 0.0
        || bounds.height() <= 0.0
    {
        return None;
    }
    Some(Bounds::from_xywh(
        f64::from(bounds.left),
        f64::from(bounds.top),
        f64::from(bounds.width()),
        f64::from(bounds.height()),
    ))
}

fn with_text_path_layout<R>(
    node: &TextPathNode,
    visit: impl FnOnce(&TextPathLayout) -> R,
) -> Option<R> {
    let bytes = match serde_json::to_vec(node) {
        Ok(bytes) => bytes,
        Err(_) => {
            let layout = layout_text_path(node)?;
            return Some(visit(&layout));
        }
    };
    TEXT_PATH_LAYOUT_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if cache.len() >= TEXT_PATH_CACHE_CAP && !cache.contains_key(&bytes) {
            cache.clear();
        }
        cache
            .entry(bytes)
            .or_insert_with(|| layout_text_path(node))
            .as_ref()
            .map(visit)
    })
}

fn layout_text_path(node: &TextPathNode) -> Option<TextPathLayout> {
    if node.content.is_empty() || !text_style_is_shapeable(&node.style) {
        return None;
    }

    // A text path owns one geometric baseline. Treat authored hard breaks as
    // spacing on that baseline instead of asking Paragraph for later lines,
    // whose visitor clusters are line-clipped views into the same shaping run.
    // CR/LF are one-byte code points, so style-run byte offsets stay exact.
    let shaping_content: String = node
        .content
        .chars()
        .map(|character| match character {
            '\r' | '\n' => ' ',
            other => other,
        })
        .collect();
    let mut buffer = TextBuffer::from_str(shaping_content, to_text_style(&node.style));
    for run in &node.style_runs {
        if !text_style_is_shapeable(&run.style)
            || buffer
                .set_style(run.start..run.end, to_text_style(&run.style))
                .is_err()
        {
            return None;
        }
    }

    let mut shaped_layout = with_layout_engine(|engine| engine.layout(&buffer, f64::INFINITY));
    let shaped_runs = shaped_layout.shaped_glyph_runs();
    let (mut source_glyphs, line_extents) = source_glyphs(&shaped_runs, &buffer);
    if source_glyphs.is_empty() {
        return None;
    }

    let mut line_offsets = BTreeMap::new();
    let mut text_advance = 0.0;
    for (line, (start, end)) in line_extents {
        let width = (end - start).max(0.0);
        line_offsets.insert(line, (text_advance, start));
        text_advance += width;
    }
    if !text_advance.is_finite() {
        return None;
    }
    for glyph in &mut source_glyphs {
        let (line_offset, line_start) = line_offsets.get(&glyph.line).copied()?;
        glyph.source_x += line_offset - line_start;
    }

    let mut anchors: Vec<f64> = source_glyphs
        .iter()
        .map(|glyph| glyph.source_x)
        .filter(|value| value.is_finite())
        .collect();
    anchors.sort_by(f64::total_cmp);
    anchors.dedup_by(|left, right| (*left - *right).abs() <= GEOMETRY_EPSILON);
    if text_advance <= GEOMETRY_EPSILON {
        text_advance = source_glyphs
            .iter()
            .map(|glyph| glyph.source_x + glyph.fallback_advance)
            .filter(|value| value.is_finite())
            .fold(0.0, f64::max);
    }
    if text_advance <= GEOMETRY_EPSILON || !text_advance.is_finite() {
        return None;
    }

    let measured = MeasuredPath::new(&node.path);
    let segment_index = usize::try_from(node.start.segment()).ok()?;
    let start_distance =
        measured.distance_at_segment_position(segment_index, node.start.position())?;
    let segment = measured.segment(segment_index)?;
    let contour = measured
        .contours()
        .iter()
        .find(|contour| contour.contour_index() == segment.contour_index())?;
    let contour_length = contour.length();
    if contour_length <= GEOMETRY_EPSILON || !contour_length.is_finite() {
        return None;
    }
    let start_on_contour = start_distance - contour.start_distance();
    let alignment_offset = match node.alignment {
        TextPathAlignment::Start => 0.0,
        TextPathAlignment::Center => -text_advance * 0.5,
        TextPathAlignment::End => -text_advance,
    };
    let traversal = match node.direction {
        TextPathDirection::Forward => 1.0,
        TextPathDirection::Reverse => -1.0,
    };
    let side = match node.side {
        TextPathSide::Default => 1.0,
        TextPathSide::Flipped => -1.0,
    };

    let mut glyphs = Vec::new();
    let mut decorations = Vec::new();
    let mut silhouette = Path::new();
    for glyph in source_glyphs {
        let next_anchor_index =
            anchors.partition_point(|anchor| *anchor <= glyph.source_x + GEOMETRY_EPSILON);
        let next_anchor = anchors
            .get(next_anchor_index)
            .copied()
            .unwrap_or(text_advance);
        let shaped_advance = (next_anchor - glyph.source_x).max(0.0);
        let advance = if shaped_advance > GEOMETRY_EPSILON {
            shaped_advance
        } else {
            glyph.fallback_advance
        };
        let center = glyph.source_x + advance * 0.5 + alignment_offset;
        let unbounded_distance = start_on_contour + traversal * center;
        let contour_distance = if contour.is_closed() {
            unbounded_distance.rem_euclid(contour_length)
        } else if (-GEOMETRY_EPSILON..=contour_length + GEOMETRY_EPSILON)
            .contains(&unbounded_distance)
        {
            unbounded_distance.clamp(0.0, contour_length)
        } else {
            continue;
        };
        let sample =
            measured.point_tangent_on_contour(contour.contour_index(), contour_distance)?;
        let tangent = [sample.tangent[0] * traversal, sample.tangent[1] * traversal];
        let normal = [-tangent[1] * side, tangent[0] * side];
        let translation = [
            sample.point[0] - tangent[0] * advance * 0.5 + normal[0] * glyph.source_y,
            sample.point[1] - tangent[1] * advance * 0.5 + normal[1] * glyph.source_y,
        ];
        let transform = glyph_transform(tangent, normal, translation)?;

        if glyph.style.color.a == 0 {
            continue;
        }
        if let Some(mut outline) = glyph.font.get_path(glyph.glyph_id) {
            outline.transform(&transform);
            silhouette.add_path(&outline, (0.0, 0.0), None);
        } else if let Some(mut bounds_path) = glyph_bounds_path(glyph.bounds) {
            bounds_path.transform(&transform);
            silhouette.add_path(&bounds_path, (0.0, 0.0), None);
        }

        for decoration in decoration_paths(&glyph, advance, &transform) {
            silhouette.add_path(&decoration.path, (0.0, 0.0), None);
            decorations.push(decoration);
        }
        glyphs.push(PlacedGlyph {
            glyph_id: glyph.glyph_id,
            font: glyph.font,
            transform,
            color: glyph.style.color,
        });
    }

    (!glyphs.is_empty() || !decorations.is_empty()).then_some(TextPathLayout {
        glyphs,
        decorations,
        silhouette,
    })
}

fn source_glyphs(
    shaped_runs: &[ShapedGlyphRun],
    buffer: &TextBuffer,
) -> (Vec<SourceGlyph>, BTreeMap<usize, (f64, f64)>) {
    let mut glyphs = Vec::new();
    let mut line_extents = BTreeMap::new();
    for run in shaped_runs {
        let run_start = run.origin[0];
        let run_end = run.origin[0] + run.advance[0];
        include_line_extent(&mut line_extents, run.line, run_start);
        include_line_extent(&mut line_extents, run.line, run_end);
        for glyph in &run.glyphs {
            let source_x = run.origin[0] + glyph.position[0];
            let source_y = glyph.position[1];
            if !source_x.is_finite() || !source_y.is_finite() {
                continue;
            }
            let mut width = [0.0];
            run.font().get_widths(&[glyph.glyph_id], &mut width);
            let fallback_advance = f64::from(width[0]);
            let fallback_advance = if fallback_advance.is_finite() {
                fallback_advance.max(0.0)
            } else {
                0.0
            };
            include_line_extent(&mut line_extents, run.line, source_x);
            glyphs.push(SourceGlyph {
                line: run.line,
                source_x,
                source_y,
                fallback_advance,
                glyph_id: glyph.glyph_id,
                bounds: glyph.bounds,
                font: run.font().clone(),
                style: style_for_byte(buffer, glyph.utf8_range.start).clone(),
            });
        }
    }
    (glyphs, line_extents)
}

fn include_line_extent(line_extents: &mut BTreeMap<usize, (f64, f64)>, line: usize, value: f64) {
    if !value.is_finite() {
        return;
    }
    let extent = line_extents.entry(line).or_insert((0.0, 0.0));
    extent.0 = extent.0.min(value);
    extent.1 = extent.1.max(value);
}

fn style_for_byte(buffer: &TextBuffer, byte: usize) -> &TextStyle {
    let byte = byte.min(buffer.len().saturating_sub(1));
    buffer
        .runs()
        .iter()
        .find(|run| run.start <= byte && byte < run.end)
        .map(|run| &run.style)
        .unwrap_or_else(|| buffer.default_style())
}

fn text_style_is_shapeable(style: &fanta_doc::TextStyle) -> bool {
    let f32_max = f64::from(f32::MAX);
    style.size_px.is_finite()
        && style.size_px > 0.0
        && style.size_px <= f32_max
        && style.letter_spacing.is_finite()
        && style.letter_spacing.abs() <= f32_max
        && style.line_height.is_finite()
        && style.line_height >= 0.0
        && style.line_height <= f32_max
        && style
            .line_height_auto_percent
            .is_none_or(|value| value.is_finite() && value >= 0.0 && value <= f32_max)
        && style
            .font_variations
            .iter()
            .all(|variation| variation.value.is_finite())
}

fn glyph_transform(tangent: [f64; 2], normal: [f64; 2], translation: [f64; 2]) -> Option<Matrix> {
    let values = [
        tangent[0],
        normal[0],
        translation[0],
        tangent[1],
        normal[1],
        translation[1],
    ];
    if values
        .iter()
        .any(|value| !value.is_finite() || value.abs() > f64::from(f32::MAX))
    {
        return None;
    }
    Some(Matrix::new_all(
        values[0] as f32,
        values[1] as f32,
        values[2] as f32,
        values[3] as f32,
        values[4] as f32,
        values[5] as f32,
        0.0,
        0.0,
        1.0,
    ))
}

fn glyph_bounds_path(bounds: [f64; 4]) -> Option<Path> {
    if bounds
        .iter()
        .any(|value| !value.is_finite() || value.abs() > f64::from(f32::MAX))
        || bounds[2] <= bounds[0]
        || bounds[3] <= bounds[1]
    {
        return None;
    }
    let mut path = Path::new();
    path.add_rect(
        Rect::new(
            bounds[0] as f32,
            bounds[1] as f32,
            bounds[2] as f32,
            bounds[3] as f32,
        ),
        None,
    );
    Some(path)
}

fn decoration_paths(glyph: &SourceGlyph, advance: f64, transform: &Matrix) -> Vec<ColoredPath> {
    if advance <= GEOMETRY_EPSILON || !advance.is_finite() {
        return Vec::new();
    }
    let (_, metrics) = glyph.font.metrics();
    let fallback_thickness = (f64::from(glyph.font.size()) * 0.06).max(1.0);
    let mut decorations = Vec::with_capacity(2);
    if glyph.style.underline {
        let position = metrics
            .underline_position()
            .map(f64::from)
            .filter(|value| value.is_finite())
            .unwrap_or(f64::from(glyph.font.size()) * 0.1);
        let thickness = metrics
            .underline_thickness()
            .map(f64::from)
            .filter(|value| value.is_finite() && *value > 0.0)
            .unwrap_or(fallback_thickness);
        if let Some(path) = decoration_rect(advance, position, thickness, transform) {
            decorations.push(ColoredPath {
                path,
                color: glyph.style.color,
            });
        }
    }
    if glyph.style.strikethrough {
        let position = metrics
            .strikeout_position()
            .map(f64::from)
            .filter(|value| value.is_finite())
            .unwrap_or(-f64::from(glyph.font.size()) * 0.3);
        let thickness = metrics
            .strikeout_thickness()
            .map(f64::from)
            .filter(|value| value.is_finite() && *value > 0.0)
            .unwrap_or(fallback_thickness);
        if let Some(path) = decoration_rect(advance, position, thickness, transform) {
            decorations.push(ColoredPath {
                path,
                color: glyph.style.color,
            });
        }
    }
    decorations
}

fn decoration_rect(
    advance: f64,
    position: f64,
    thickness: f64,
    transform: &Matrix,
) -> Option<Path> {
    let bounds = [
        0.0,
        position - thickness * 0.5,
        advance,
        position + thickness * 0.5,
    ];
    let mut path = glyph_bounds_path(bounds)?;
    path.transform(transform);
    Some(path)
}

fn glyph_paint(color: fanta_doc::Color) -> Paint {
    let mut paint = Paint::default();
    paint.set_anti_alias(true);
    paint.set_color(to_sk_color(color));
    paint
}

#[cfg(test)]
mod tests {
    use super::*;
    use fanta_doc::{PathData, TextPathStart};

    fn line_node(content: &str) -> TextPathNode {
        let mut path = PathData::new();
        path.move_to(0.0, 100.0).line_to(500.0, 100.0);
        let mut node = TextPathNode::new(path, content);
        node.style.size_px = 32.0;
        node
    }

    fn glyph_origins(node: &TextPathNode) -> Vec<[f64; 2]> {
        layout_text_path(node)
            .expect("text path lays out")
            .glyphs
            .iter()
            .map(|glyph| {
                let point = glyph.transform.map_point(Point::new(0.0, 0.0));
                [f64::from(point.x), f64::from(point.y)]
            })
            .collect()
    }

    #[test]
    fn start_alignment_and_direction_place_the_shaped_sequence() {
        let mut node = line_node("Path");
        node.start = TextPathStart::new(0, 0.5).expect("valid midpoint");

        let start = glyph_origins(&node);
        assert!((start[0][0] - 250.0).abs() < 1.0);
        assert!(start[1][0] > start[0][0]);

        node.alignment = TextPathAlignment::Center;
        let centered = glyph_origins(&node);
        assert!(centered[0][0] < start[0][0]);

        node.alignment = TextPathAlignment::End;
        let ended = glyph_origins(&node);
        assert!(ended[0][0] < centered[0][0]);

        node.alignment = TextPathAlignment::Start;
        node.direction = TextPathDirection::Reverse;
        let reversed = glyph_origins(&node);
        assert!((reversed[0][0] - 250.0).abs() < 1.0);
        assert!(reversed[1][0] < reversed[0][0]);
    }

    #[test]
    fn side_reflects_glyphs_without_changing_traversal() {
        let mut node = line_node("Side");
        let default_origins = glyph_origins(&node);
        let default_bounds = text_path_bounds(&node).expect("default side bounds");

        node.side = TextPathSide::Flipped;
        let flipped_origins = glyph_origins(&node);
        let flipped_bounds = text_path_bounds(&node).expect("flipped side bounds");

        assert_eq!(default_origins.len(), flipped_origins.len());
        for (default, flipped) in default_origins.iter().zip(&flipped_origins) {
            assert!((default[0] - flipped[0]).abs() < 0.01);
        }
        assert!(default_origins[1][0] > default_origins[0][0]);
        assert!(flipped_origins[1][0] > flipped_origins[0][0]);
        assert!(default_bounds.min_y < 100.0);
        assert!(flipped_bounds.max_y > 100.0);
        assert!(default_bounds.center()[1] < 100.0);
        assert!(flipped_bounds.center()[1] > 100.0);
    }

    #[test]
    fn curved_path_rotates_glyphs_at_measured_tangents() {
        let mut path = PathData::new();
        path.move_to(0.0, 100.0).quad_to(100.0, -20.0, 220.0, 100.0);
        let mut node = TextPathNode::new(path, "Curve");
        node.style.size_px = 28.0;
        node.start = TextPathStart::new(0, 0.2).expect("valid curve position");

        let layout = layout_text_path(&node).expect("curved text lays out");
        let first = layout.glyphs.first().expect("first curved glyph");
        let origin = first.transform.map_point(Point::new(0.0, 0.0));
        let x_axis = first.transform.map_point(Point::new(1.0, 0.0));
        assert!((x_axis.y - origin.y).abs() > 0.05);
        let bounds = text_path_bounds(&node).expect("curved glyph bounds");
        assert!(bounds.is_finite() && bounds.width() > 0.0 && bounds.height() > 0.0);
    }

    #[test]
    fn open_paths_clip_centers_and_degenerate_paths_paint_nothing() {
        let mut path = PathData::new();
        path.move_to(0.0, 20.0).line_to(42.0, 20.0);
        let mut clipped = TextPathNode::new(path, "A deliberately long shaped run");
        clipped.style.size_px = 20.0;
        let clipped_layout = layout_text_path(&clipped).expect("prefix fits short path");
        assert!(clipped_layout.glyphs.len() < clipped.content.chars().count());
        assert!(text_path_bounds(&clipped).is_some());

        let mut zero = PathData::new();
        zero.move_to(1.0, 1.0).line_to(1.0, 1.0);
        let zero = TextPathNode::new(zero, "No length");
        assert!(layout_text_path(&zero).is_none());
        assert!(text_path_outline(&zero).is_none());
        assert!(text_path_bounds(&zero).is_none());

        let mut nonfinite = PathData::new();
        nonfinite.move_to(f64::NAN, 0.0).line_to(f64::INFINITY, 1.0);
        assert!(layout_text_path(&TextPathNode::new(nonfinite, "Finite")).is_none());
    }

    #[test]
    fn closed_contour_wraps_past_its_authored_start() {
        let mut node = TextPathNode::new(PathData::rect(0.0, 0.0, 80.0, 40.0), "ABCDEFG");
        node.style.size_px = 18.0;
        node.start = TextPathStart::new(2, 0.85).expect("valid late start");
        let layout = layout_text_path(&node).expect("closed path lays out");
        assert_eq!(layout.glyphs.len(), node.content.chars().count());
        assert!(text_path_bounds(&node).is_some());
    }
}
