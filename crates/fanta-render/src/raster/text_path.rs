use super::text::{to_text_style, with_layout_engine};
use super::{Bounds, Canvas, Paint, TextBuffer, TextStyle, to_sk_color};
use fanta_doc::{MeasuredPath, TextPathAlignment, TextPathDirection, TextPathNode, TextPathSide};
use fanta_text::{ShapedGlyphRun, ShapedTextCluster};
use skia_safe::{Font, Matrix, Path, Point, Rect, paint::Style as PaintStyle};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::ops::Range;
use unicode_segmentation::UnicodeSegmentation;

const GEOMETRY_EPSILON: f64 = 1e-9;
const TEXT_PATH_CACHE_CAP: usize = 2048;

thread_local! {
    static TEXT_PATH_LAYOUT_CACHE: RefCell<
        std::collections::HashMap<Vec<u8>, Option<TextPathLayout>>,
    > = RefCell::new(std::collections::HashMap::new());
}

#[derive(Clone)]
struct SourceGlyph {
    paint_x: f64,
    paint_y: f64,
    glyph_id: u16,
    bounds: [f64; 4],
    font: Font,
    style: TextStyle,
}

struct SourceCluster {
    line: usize,
    utf8_range: Range<usize>,
    source_x: f64,
    advance: f64,
    right_to_left: bool,
    top: f64,
    bottom: f64,
    font_size: f64,
    glyphs: Vec<SourceGlyph>,
}

struct PlacedGlyph {
    glyph_id: u16,
    font: Font,
    transform: Matrix,
    color: fanta_doc::Color,
}

#[derive(Clone)]
struct ColoredPath {
    path: Path,
    color: fanta_doc::Color,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextPathCaretSegment {
    pub start: [f64; 2],
    pub end: [f64; 2],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextPathAffinity {
    Upstream,
    Downstream,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextPathPosition {
    pub byte: usize,
    pub affinity: TextPathAffinity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextPathVisualDirection {
    /// Toward earlier positions in the visual order induced by path placement.
    Previous,
    /// Toward later positions in the visual order induced by path placement.
    Next,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TextPathSelectionQuad {
    pub utf8_range: Range<usize>,
    pub points: [[f64; 2]; 4],
}

#[derive(Clone)]
struct PlacedCaret {
    position: TextPathPosition,
    segment: TextPathCaretSegment,
}

#[derive(Clone)]
struct VisualCaret {
    position: TextPathPosition,
    visual_offset: f64,
}

struct PlacedCluster {
    utf8_range: Range<usize>,
    carets: Vec<PlacedCaret>,
}

#[derive(Clone)]
pub(crate) enum TextPathOutline {
    Empty,
    Exact(Path),
    UnsupportedGlyph,
}

impl TextPathOutline {
    pub(crate) fn into_exact(self) -> Option<Path> {
        match self {
            Self::Exact(path) => Some(path),
            Self::Empty | Self::UnsupportedGlyph => None,
        }
    }
}

struct TextPathLayout {
    glyphs: Vec<PlacedGlyph>,
    decorations: Vec<ColoredPath>,
    clusters: Vec<PlacedCluster>,
    // Open paths intentionally omit off-path geometry, but keyboard navigation
    // must still retain every source boundary so clipped text remains editable.
    visual_carets: Vec<VisualCaret>,
    outline: TextPathOutline,
    containment_path: Path,
    visual_bounds: Option<Bounds>,
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

pub(crate) fn text_path_outline(node: &TextPathNode) -> TextPathOutline {
    with_text_path_layout(node, |layout| layout.outline.clone()).unwrap_or(TextPathOutline::Empty)
}

pub(crate) fn text_path_bounds(node: &TextPathNode) -> Option<Bounds> {
    text_path_visual_bounds(node)
}

/// Tight local bounds for outline glyphs and conservative font bounds for a
/// color or bitmap glyph whose alpha silhouette Skia cannot expose as a path.
pub fn text_path_visual_bounds(node: &TextPathNode) -> Option<Bounds> {
    with_text_path_layout(node, |layout| layout.visual_bounds).flatten()
}

/// Local caret geometry at the nearest shaped grapheme boundary.
pub fn text_path_caret_segment(node: &TextPathNode, byte: usize) -> Option<TextPathCaretSegment> {
    text_path_caret_segment_at(
        node,
        TextPathPosition {
            byte,
            affinity: TextPathAffinity::Downstream,
        },
    )
}

/// Affinity-aware caret lookup for a byte shared by two visual BiDi runs.
pub fn text_path_caret_segment_at(
    node: &TextPathNode,
    position: TextPathPosition,
) -> Option<TextPathCaretSegment> {
    with_text_path_layout(node, |layout| {
        let carets = || layout.clusters.iter().flat_map(|cluster| &cluster.carets);
        carets()
            .find(|caret| caret.position == position)
            .or_else(|| carets().find(|caret| caret.position.byte == position.byte))
            .or_else(|| {
                carets().min_by_key(|caret| {
                    caret
                        .position
                        .byte
                        .abs_diff(position.byte.min(node.content.len()))
                })
            })
            .map(|caret| caret.segment)
    })?
}

/// Nearest affinity-aware text position for a local point.
pub fn text_path_hit_test_position(
    node: &TextPathNode,
    point: [f64; 2],
) -> Option<TextPathPosition> {
    if point.iter().any(|value| !value.is_finite()) {
        return None;
    }
    with_text_path_layout(node, |layout| {
        layout
            .clusters
            .iter()
            .flat_map(|cluster| &cluster.carets)
            .min_by(|left, right| {
                distance_squared_to_segment(point, left.segment)
                    .total_cmp(&distance_squared_to_segment(point, right.segment))
            })
            .map(|caret| caret.position)
    })?
}

/// Nearest UTF-8 grapheme-boundary byte for a local point.
pub fn text_path_hit_test(node: &TextPathNode, point: [f64; 2]) -> Option<usize> {
    text_path_hit_test_position(node, point).map(|position| position.byte)
}

/// The adjacent caret stop in the placed text path's visual order.
///
/// Skia's cluster geometry supplies the BiDi order and reverse path traversal
/// orients it on the path. Affinity-distinct positions at a BiDi boundary
/// therefore remain independently addressable even when they share a source
/// byte.
pub fn text_path_visual_neighbor(
    node: &TextPathNode,
    position: TextPathPosition,
    direction: TextPathVisualDirection,
) -> Option<TextPathPosition> {
    with_text_path_layout(node, |layout| {
        let stops = visual_caret_stops(layout);
        let index = visual_stop_index(&stops, position)?;
        let target = match direction {
            TextPathVisualDirection::Previous => index.checked_sub(1)?,
            TextPathVisualDirection::Next => {
                let target = index.checked_add(1)?;
                (target < stops.len()).then_some(target)?
            }
        };
        visual_stop_position(stops.get(target)?, direction)
    })?
}

/// The first or last caret stop on the shaped visual line.
pub fn text_path_visual_line_edge(
    node: &TextPathNode,
    direction: TextPathVisualDirection,
) -> Option<TextPathPosition> {
    with_text_path_layout(node, |layout| {
        let stops = visual_caret_stops(layout);
        let stop = match direction {
            TextPathVisualDirection::Previous => stops.first()?,
            TextPathVisualDirection::Next => stops.last()?,
        };
        visual_stop_position(stop, direction)
    })?
}

/// The visually earlier or later of two selection endpoints.
pub fn text_path_visual_selection_edge(
    node: &TextPathNode,
    anchor: TextPathPosition,
    head: TextPathPosition,
    direction: TextPathVisualDirection,
) -> Option<TextPathPosition> {
    with_text_path_layout(node, |layout| {
        let stops = visual_caret_stops(layout);
        let anchor_index = visual_stop_index(&stops, anchor)?;
        let head_index = visual_stop_index(&stops, head)?;
        let (index, position) = match direction {
            TextPathVisualDirection::Previous if anchor_index < head_index => {
                (anchor_index, anchor)
            }
            TextPathVisualDirection::Previous if head_index < anchor_index => (head_index, head),
            TextPathVisualDirection::Next if anchor_index > head_index => (anchor_index, anchor),
            TextPathVisualDirection::Next if head_index > anchor_index => (head_index, head),
            _ => return visual_stop_position(stops.get(anchor_index)?, direction),
        };
        let stop = stops.get(index)?;
        stop.carets
            .iter()
            .any(|caret| caret.position == position)
            .then_some(position)
            .or_else(|| visual_stop_position(stop, direction))
    })?
}

/// Local quadrilaterals covering the shaped clusters selected by a byte range.
pub fn text_path_selection_quads(
    node: &TextPathNode,
    range: Range<usize>,
) -> Vec<TextPathSelectionQuad> {
    let start = range.start.min(range.end).min(node.content.len());
    let end = range.start.max(range.end).min(node.content.len());
    if start == end {
        return Vec::new();
    }
    with_text_path_layout(node, |layout| {
        layout
            .clusters
            .iter()
            .filter_map(|cluster| selection_quad(cluster, start..end))
            .collect()
    })
    .unwrap_or_default()
}

/// Whether a local point intersects visible TextPath content.
///
/// Vector glyphs use their exact outline. For a color or bitmap glyph whose
/// alpha mask is unavailable as a path, this conservatively uses its visual
/// bounds so editing remains possible without inventing an effect silhouette.
pub fn text_path_contains_point(node: &TextPathNode, point: [f64; 2]) -> bool {
    if point
        .iter()
        .any(|value| !value.is_finite() || value.abs() > f64::from(f32::MAX))
    {
        return false;
    }
    with_text_path_layout(node, |layout| match &layout.outline {
        TextPathOutline::Exact(path) => path.contains(Point::new(point[0] as f32, point[1] as f32)),
        TextPathOutline::UnsupportedGlyph => layout
            .containment_path
            .contains(Point::new(point[0] as f32, point[1] as f32)),
        TextPathOutline::Empty => false,
    })
    .unwrap_or(false)
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
    if !text_style_is_shapeable(&node.style) {
        return None;
    }
    let path_context = PathContext::new(node)?;
    if node.content.is_empty() {
        let frame = path_context.frame(0.0, 0.0)?;
        let caret = placed_caret(
            0,
            TextPathAffinity::Downstream,
            frame.baseline_origin,
            frame.normal,
            -node.style.size_px * 0.8,
            node.style.size_px * 0.2,
        );
        return Some(TextPathLayout {
            glyphs: Vec::new(),
            decorations: Vec::new(),
            clusters: vec![PlacedCluster {
                utf8_range: 0..0,
                carets: vec![caret],
            }],
            visual_carets: vec![VisualCaret {
                position: TextPathPosition {
                    byte: 0,
                    affinity: TextPathAffinity::Downstream,
                },
                visual_offset: 0.0,
            }],
            outline: TextPathOutline::Empty,
            containment_path: Path::new(),
            visual_bounds: None,
        });
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
        if !text_style_is_shapeable(&run.style) {
            return None;
        }
        if let Err(error) = buffer.set_style(run.start..run.end, to_text_style(&run.style)) {
            tracing::warn!(?error, "invalid TextPath style run");
            return None;
        }
    }

    let mut shaped_layout = with_layout_engine(|engine| engine.layout(&buffer, f64::INFINITY));
    let shaped_runs = match shaped_layout.try_shaped_glyph_runs() {
        Ok(runs) => runs,
        Err(error) => {
            tracing::warn!(?error, "could not build TextPath shaped glyph snapshot");
            return None;
        }
    };
    let shaped_clusters = match shaped_layout.try_shaped_text_clusters() {
        Ok(clusters) => clusters,
        Err(error) => {
            tracing::warn!(?error, "could not build TextPath shaped cluster snapshot");
            return None;
        }
    };
    let (mut source_clusters, line_extents) =
        source_clusters(&shaped_runs, &shaped_clusters, &buffer)?;
    if source_clusters.is_empty() {
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
    for cluster in &mut source_clusters {
        let (line_offset, line_start) = line_offsets.get(&cluster.line).copied()?;
        let offset = line_offset - line_start;
        cluster.source_x += offset;
        for glyph in &mut cluster.glyphs {
            glyph.paint_x += offset;
        }
    }
    if text_advance <= GEOMETRY_EPSILON {
        text_advance = source_clusters
            .iter()
            .map(|cluster| cluster.source_x + cluster.advance)
            .filter(|value| value.is_finite())
            .fold(0.0, f64::max);
    }
    if !text_advance.is_finite() {
        return None;
    }

    let alignment_offset = match node.alignment {
        TextPathAlignment::Start => 0.0,
        TextPathAlignment::Center => -text_advance * 0.5,
        TextPathAlignment::End => -text_advance,
    };

    let mut glyphs = Vec::new();
    let mut decorations = Vec::new();
    let mut clusters = Vec::new();
    let mut visual_carets = Vec::new();
    let mut silhouette = Path::new();
    let mut containment_path = Path::new();
    let mut visual_bounds = None;
    let mut unsupported_outline = false;
    for cluster in source_clusters {
        let visual_start = cluster.source_x + alignment_offset;
        let caret_positions = cluster_caret_positions(
            &node.content,
            cluster.utf8_range.clone(),
            cluster.right_to_left,
        );
        visual_carets.extend(caret_positions.iter().map(|(position, visual_fraction)| {
            VisualCaret {
                position: *position,
                visual_offset: path_context.traversal
                    * (visual_start + cluster.advance * *visual_fraction),
            }
        }));

        let center = visual_start + cluster.advance * 0.5;
        let Some(frame) = path_context.frame(center, cluster.advance) else {
            continue;
        };
        let cluster_transform =
            glyph_transform(frame.tangent, frame.normal, frame.baseline_origin)?;
        let font_size = cluster.font_size;
        let top = if cluster.top.is_finite() {
            cluster.top
        } else {
            -font_size * 0.8
        };
        let bottom = if cluster.bottom.is_finite() && cluster.bottom > top {
            cluster.bottom
        } else {
            font_size * 0.2
        };
        let placed_cluster = placed_cluster(
            cluster.utf8_range.clone(),
            &caret_positions,
            cluster.advance,
            frame,
            top,
            bottom,
        );

        let cluster_is_whitespace = node
            .content
            .get(cluster.utf8_range.clone())
            .is_some_and(|text| text.chars().all(char::is_whitespace));
        let mut cluster_has_unsupported_outline = false;
        for glyph in &cluster.glyphs {
            if glyph.style.color.a == 0 {
                continue;
            }
            let relative_x = glyph.paint_x - cluster.source_x;
            let translation = [
                frame.baseline_origin[0]
                    + frame.tangent[0] * relative_x
                    + frame.normal[0] * glyph.paint_y,
                frame.baseline_origin[1]
                    + frame.tangent[1] * relative_x
                    + frame.normal[1] * glyph.paint_y,
            ];
            let transform = glyph_transform(frame.tangent, frame.normal, translation)?;
            if let Some(mut outline) = glyph.font.get_path(glyph.glyph_id) {
                outline.transform(&transform);
                include_path_bounds(&mut visual_bounds, &outline);
                silhouette.add_path(&outline, (0.0, 0.0), None);
                containment_path.add_path(&outline, (0.0, 0.0), None);
            } else if !cluster_is_whitespace {
                cluster_has_unsupported_outline = true;
                if let Some(mut bounds_path) = glyph_bounds_path(glyph.bounds) {
                    bounds_path.transform(&transform);
                    include_path_bounds(&mut visual_bounds, &bounds_path);
                    containment_path.add_path(&bounds_path, (0.0, 0.0), None);
                }
            }
            glyphs.push(PlacedGlyph {
                glyph_id: glyph.glyph_id,
                font: glyph.font.clone(),
                transform,
                color: glyph.style.color,
            });
        }

        if cluster_has_unsupported_outline {
            unsupported_outline = true;
            if let Some(quad) = full_cluster_quad(&placed_cluster) {
                if let Some(bounds) = quad_bounds(quad) {
                    include_bounds(&mut visual_bounds, bounds);
                }
                if let Some(path) = quad_path(quad) {
                    containment_path.add_path(&path, (0.0, 0.0), None);
                }
            }
        }
        if let Some(glyph) = cluster.glyphs.first() {
            if glyph.style.color.a != 0 {
                for decoration in decoration_paths(glyph, cluster.advance, &cluster_transform) {
                    include_path_bounds(&mut visual_bounds, &decoration.path);
                    silhouette.add_path(&decoration.path, (0.0, 0.0), None);
                    containment_path.add_path(&decoration.path, (0.0, 0.0), None);
                    decorations.push(decoration);
                }
            }
        }
        clusters.push(placed_cluster);
    }

    if path_context.traversal < 0.0 {
        // Sorting carets by the negated offset reverses distinct stops, but the
        // stable sort preserves tie order. Reverse insertion order as well so
        // entering a shared BiDi stop chooses the affinity for reverse travel.
        visual_carets.reverse();
    }

    let outline = classify_outline(silhouette, unsupported_outline);
    (!visual_carets.is_empty()).then_some(TextPathLayout {
        glyphs,
        decorations,
        clusters,
        visual_carets,
        outline,
        containment_path,
        visual_bounds,
    })
}

fn classify_outline(silhouette: Path, unsupported_outline: bool) -> TextPathOutline {
    if unsupported_outline {
        TextPathOutline::UnsupportedGlyph
    } else if silhouette.is_empty() {
        TextPathOutline::Empty
    } else {
        TextPathOutline::Exact(silhouette)
    }
}

struct PathContext {
    measured: MeasuredPath,
    contour_index: usize,
    contour_length: f64,
    start_on_contour: f64,
    closed: bool,
    traversal: f64,
    side: f64,
}

#[derive(Clone, Copy)]
struct PlacementFrame {
    baseline_origin: [f64; 2],
    tangent: [f64; 2],
    normal: [f64; 2],
}

impl PathContext {
    fn new(node: &TextPathNode) -> Option<Self> {
        let measured = MeasuredPath::new(&node.path);
        let segment_index = match usize::try_from(node.start.segment()) {
            Ok(segment_index) => segment_index,
            Err(error) => {
                tracing::warn!(?error, "TextPath segment index does not fit this platform");
                return None;
            }
        };
        let start_distance =
            measured.distance_at_segment_position(segment_index, node.start.position())?;
        let segment = measured.segment(segment_index)?;
        let contour = measured
            .contours()
            .iter()
            .find(|contour| contour.contour_index() == segment.contour_index())?;
        let contour_index = contour.contour_index();
        let contour_length = contour.length();
        let start_on_contour = start_distance - contour.start_distance();
        let closed = contour.is_closed();
        if contour_length <= GEOMETRY_EPSILON
            || !contour_length.is_finite()
            || !start_on_contour.is_finite()
        {
            return None;
        }
        Some(Self {
            measured,
            contour_index,
            contour_length,
            start_on_contour,
            closed,
            traversal: match node.direction {
                TextPathDirection::Forward => 1.0,
                TextPathDirection::Reverse => -1.0,
            },
            side: match node.side {
                TextPathSide::Default => 1.0,
                TextPathSide::Flipped => -1.0,
            },
        })
    }

    fn frame(&self, center: f64, advance: f64) -> Option<PlacementFrame> {
        if !center.is_finite() || !advance.is_finite() || advance < 0.0 {
            return None;
        }
        let unbounded_distance = self.start_on_contour + self.traversal * center;
        let contour_distance = if self.closed {
            unbounded_distance.rem_euclid(self.contour_length)
        } else if (-GEOMETRY_EPSILON..=self.contour_length + GEOMETRY_EPSILON)
            .contains(&unbounded_distance)
        {
            unbounded_distance.clamp(0.0, self.contour_length)
        } else {
            return None;
        };
        let sample = self
            .measured
            .point_tangent_on_contour(self.contour_index, contour_distance)?;
        let tangent = [
            sample.tangent[0] * self.traversal,
            sample.tangent[1] * self.traversal,
        ];
        let normal = [-tangent[1] * self.side, tangent[0] * self.side];
        Some(PlacementFrame {
            baseline_origin: [
                sample.point[0] - tangent[0] * advance * 0.5,
                sample.point[1] - tangent[1] * advance * 0.5,
            ],
            tangent,
            normal,
        })
    }
}

fn source_clusters(
    shaped_runs: &[ShapedGlyphRun],
    shaped_clusters: &[ShapedTextCluster],
    buffer: &TextBuffer,
) -> Option<(Vec<SourceCluster>, BTreeMap<usize, (f64, f64)>)> {
    let mut clusters = BTreeMap::<(usize, usize, usize), SourceCluster>::new();
    let mut line_extents = BTreeMap::new();
    for cluster in shaped_clusters {
        let source_x = cluster.bounds[0];
        let advance = (cluster.bounds[2] - cluster.bounds[0]).max(0.0);
        let top = cluster.bounds[1] - cluster.baseline;
        let bottom = cluster.bounds[3] - cluster.baseline;
        if !source_x.is_finite() || !advance.is_finite() || !top.is_finite() || !bottom.is_finite()
        {
            return None;
        }
        include_line_extent(&mut line_extents, cluster.line, source_x);
        include_line_extent(&mut line_extents, cluster.line, source_x + advance);
        clusters.insert(
            (
                cluster.line,
                cluster.utf8_range.start,
                cluster.utf8_range.end,
            ),
            SourceCluster {
                line: cluster.line,
                utf8_range: cluster.utf8_range.clone(),
                source_x,
                advance,
                right_to_left: cluster.right_to_left,
                top,
                bottom,
                font_size: style_for_byte(buffer, cluster.utf8_range.start).size_px,
                glyphs: Vec::new(),
            },
        );
    }
    for run in shaped_runs {
        for glyph in &run.glyphs {
            let paint_position = glyph.paint_position();
            let paint_x = run.origin[0] + paint_position[0];
            let paint_y = paint_position[1];
            if !paint_x.is_finite()
                || !paint_y.is_finite()
                || glyph.bounds.iter().any(|value| !value.is_finite())
                || glyph.cluster_bounds.iter().any(|value| !value.is_finite())
                || !glyph.cluster_advance.is_finite()
            {
                return None;
            }
            let source_x = glyph.cluster_bounds[0];
            let advance = glyph.cluster_advance.max(0.0);
            include_line_extent(&mut line_extents, run.line, source_x);
            include_line_extent(&mut line_extents, run.line, source_x + advance);
            let (_, metrics) = run.font().metrics();
            let top = (glyph.cluster_bounds[1] - run.origin[1])
                .min(f64::from(metrics.ascent))
                .min(paint_y + glyph.bounds[1]);
            let bottom = (glyph.cluster_bounds[3] - run.origin[1])
                .max(f64::from(metrics.descent))
                .max(paint_y + glyph.bounds[3]);
            let source_glyph = SourceGlyph {
                paint_x,
                paint_y,
                glyph_id: glyph.glyph_id,
                bounds: glyph.bounds,
                font: run.font().clone(),
                style: style_for_byte(buffer, glyph.utf8_range.start).clone(),
            };
            let key = (run.line, glyph.utf8_range.start, glyph.utf8_range.end);
            if let Some(cluster) = clusters.get_mut(&key) {
                if (cluster.source_x - source_x).abs() > GEOMETRY_EPSILON
                    || (cluster.advance - advance).abs() > GEOMETRY_EPSILON
                    || cluster.right_to_left != glyph.right_to_left
                {
                    return None;
                }
                cluster.top = cluster.top.min(top);
                cluster.bottom = cluster.bottom.max(bottom);
                cluster.glyphs.push(source_glyph);
            } else {
                clusters.insert(
                    key,
                    SourceCluster {
                        line: run.line,
                        utf8_range: glyph.utf8_range.clone(),
                        source_x,
                        advance,
                        right_to_left: glyph.right_to_left,
                        top,
                        bottom,
                        font_size: f64::from(run.font().size()),
                        glyphs: vec![source_glyph],
                    },
                );
            }
        }
    }
    let mut clusters: Vec<_> = clusters.into_values().collect();
    clusters.sort_by(|left, right| {
        left.line
            .cmp(&right.line)
            .then_with(|| left.source_x.total_cmp(&right.source_x))
    });
    Some((clusters, line_extents))
}

fn placed_cluster(
    utf8_range: Range<usize>,
    caret_positions: &[(TextPathPosition, f64)],
    advance: f64,
    frame: PlacementFrame,
    top: f64,
    bottom: f64,
) -> PlacedCluster {
    let carets = caret_positions
        .iter()
        .map(|(position, visual_fraction)| {
            let point = [
                frame.baseline_origin[0] + frame.tangent[0] * advance * *visual_fraction,
                frame.baseline_origin[1] + frame.tangent[1] * advance * *visual_fraction,
            ];
            placed_caret(
                position.byte,
                position.affinity,
                point,
                frame.normal,
                top,
                bottom,
            )
        })
        .collect();
    PlacedCluster { utf8_range, carets }
}

fn cluster_caret_positions(
    content: &str,
    utf8_range: Range<usize>,
    right_to_left: bool,
) -> Vec<(TextPathPosition, f64)> {
    let mut boundaries = content
        .get(utf8_range.clone())
        .map(|text| {
            text.grapheme_indices(true)
                .map(|(byte, _)| utf8_range.start + byte)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if boundaries.first().copied() != Some(utf8_range.start) {
        boundaries.insert(0, utf8_range.start);
    }
    if boundaries.last().copied() != Some(utf8_range.end) {
        boundaries.push(utf8_range.end);
    }
    let last_boundary_index = boundaries.len().saturating_sub(1);
    let interval_count = last_boundary_index.max(1) as f64;
    boundaries
        .into_iter()
        .enumerate()
        .map(|(index, byte)| {
            let logical_fraction = index as f64 / interval_count;
            let visual_fraction = if right_to_left {
                1.0 - logical_fraction
            } else {
                logical_fraction
            };
            let affinity = if index == last_boundary_index {
                TextPathAffinity::Upstream
            } else {
                TextPathAffinity::Downstream
            };
            (TextPathPosition { byte, affinity }, visual_fraction)
        })
        .collect()
}

fn placed_caret(
    byte: usize,
    affinity: TextPathAffinity,
    point: [f64; 2],
    normal: [f64; 2],
    top: f64,
    bottom: f64,
) -> PlacedCaret {
    PlacedCaret {
        position: TextPathPosition { byte, affinity },
        segment: TextPathCaretSegment {
            start: [point[0] + normal[0] * top, point[1] + normal[1] * top],
            end: [point[0] + normal[0] * bottom, point[1] + normal[1] * bottom],
        },
    }
}

struct VisualCaretStop<'a> {
    visual_offset: f64,
    carets: Vec<&'a VisualCaret>,
}

fn visual_caret_stops(layout: &TextPathLayout) -> Vec<VisualCaretStop<'_>> {
    let mut carets = layout.visual_carets.iter().collect::<Vec<_>>();
    carets.sort_by(|left, right| left.visual_offset.total_cmp(&right.visual_offset));

    let mut stops: Vec<VisualCaretStop<'_>> = Vec::new();
    for caret in carets {
        if let Some(stop) = stops.last_mut()
            && (stop.visual_offset - caret.visual_offset).abs() <= GEOMETRY_EPSILON
        {
            stop.carets.push(caret);
        } else {
            stops.push(VisualCaretStop {
                visual_offset: caret.visual_offset,
                carets: vec![caret],
            });
        }
    }
    stops
}

fn visual_stop_index(stops: &[VisualCaretStop<'_>], position: TextPathPosition) -> Option<usize> {
    stops
        .iter()
        .position(|stop| stop.carets.iter().any(|caret| caret.position == position))
        .or_else(|| {
            stops.iter().position(|stop| {
                stop.carets
                    .iter()
                    .any(|caret| caret.position.byte == position.byte)
            })
        })
        .or_else(|| {
            stops
                .iter()
                .enumerate()
                .min_by_key(|(_, stop)| {
                    stop.carets
                        .iter()
                        .map(|caret| caret.position.byte.abs_diff(position.byte))
                        .min()
                        .unwrap_or(usize::MAX)
                })
                .map(|(index, _)| index)
        })
}

fn visual_stop_position(
    stop: &VisualCaretStop<'_>,
    direction: TextPathVisualDirection,
) -> Option<TextPathPosition> {
    match direction {
        TextPathVisualDirection::Previous => stop.carets.first(),
        TextPathVisualDirection::Next => stop.carets.last(),
    }
    .map(|caret| caret.position)
}

fn selection_quad(
    cluster: &PlacedCluster,
    selection: Range<usize>,
) -> Option<TextPathSelectionQuad> {
    let start = selection.start.max(cluster.utf8_range.start);
    let end = selection.end.min(cluster.utf8_range.end);
    if start >= end {
        return None;
    }
    let start_caret = cluster
        .carets
        .iter()
        .filter(|caret| caret.position.byte <= start)
        .max_by_key(|caret| caret.position.byte)
        .or_else(|| cluster.carets.first())?;
    let end_caret = cluster
        .carets
        .iter()
        .filter(|caret| caret.position.byte >= end)
        .min_by_key(|caret| caret.position.byte)
        .or_else(|| cluster.carets.last())?;
    if start_caret.position.byte == end_caret.position.byte {
        return None;
    }
    Some(TextPathSelectionQuad {
        utf8_range: start_caret.position.byte.min(end_caret.position.byte)
            ..start_caret.position.byte.max(end_caret.position.byte),
        points: [
            start_caret.segment.start,
            end_caret.segment.start,
            end_caret.segment.end,
            start_caret.segment.end,
        ],
    })
}

fn full_cluster_quad(cluster: &PlacedCluster) -> Option<[[f64; 2]; 4]> {
    let first = cluster.carets.first()?;
    let last = cluster.carets.last()?;
    Some([
        first.segment.start,
        last.segment.start,
        last.segment.end,
        first.segment.end,
    ])
}

fn distance_squared_to_segment(point: [f64; 2], segment: TextPathCaretSegment) -> f64 {
    let delta = [
        segment.end[0] - segment.start[0],
        segment.end[1] - segment.start[1],
    ];
    let length_squared = delta[0] * delta[0] + delta[1] * delta[1];
    let projection = if length_squared > GEOMETRY_EPSILON {
        ((point[0] - segment.start[0]) * delta[0] + (point[1] - segment.start[1]) * delta[1])
            / length_squared
    } else {
        0.0
    }
    .clamp(0.0, 1.0);
    let nearest = [
        segment.start[0] + delta[0] * projection,
        segment.start[1] + delta[1] * projection,
    ];
    (point[0] - nearest[0]).powi(2) + (point[1] - nearest[1]).powi(2)
}

fn include_path_bounds(bounds: &mut Option<Bounds>, path: &Path) {
    let path_bounds = path.compute_tight_bounds();
    let values = [
        path_bounds.left,
        path_bounds.top,
        path_bounds.right,
        path_bounds.bottom,
    ];
    if values.iter().any(|value| !value.is_finite())
        || path_bounds.width() <= 0.0
        || path_bounds.height() <= 0.0
    {
        return;
    }
    include_bounds(
        bounds,
        Bounds::from_xywh(
            f64::from(path_bounds.left),
            f64::from(path_bounds.top),
            f64::from(path_bounds.width()),
            f64::from(path_bounds.height()),
        ),
    );
}

fn quad_bounds(points: [[f64; 2]; 4]) -> Option<Bounds> {
    if points
        .iter()
        .flatten()
        .any(|coordinate| !coordinate.is_finite())
    {
        return None;
    }
    let mut min_x = f64::INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    for point in points {
        min_x = min_x.min(point[0]);
        min_y = min_y.min(point[1]);
        max_x = max_x.max(point[0]);
        max_y = max_y.max(point[1]);
    }
    (max_x > min_x && max_y > min_y)
        .then(|| Bounds::from_xywh(min_x, min_y, max_x - min_x, max_y - min_y))
}

fn quad_path(points: [[f64; 2]; 4]) -> Option<Path> {
    if points
        .iter()
        .flatten()
        .any(|coordinate| !coordinate.is_finite() || coordinate.abs() > f64::from(f32::MAX))
    {
        return None;
    }
    let mut path = Path::new();
    path.move_to((points[0][0] as f32, points[0][1] as f32));
    for point in &points[1..] {
        path.line_to((point[0] as f32, point[1] as f32));
    }
    path.close();
    Some(path)
}

fn include_bounds(bounds: &mut Option<Bounds>, addition: Bounds) {
    if !addition.is_finite() || addition.width() <= 0.0 || addition.height() <= 0.0 {
        return;
    }
    *bounds = Some(match bounds {
        Some(bounds) => bounds.union(&addition),
        None => addition,
    });
}

fn include_line_extent(line_extents: &mut BTreeMap<usize, (f64, f64)>, line: usize, value: f64) {
    if !value.is_finite() {
        return;
    }
    line_extents
        .entry(line)
        .and_modify(|extent| {
            extent.0 = extent.0.min(value);
            extent.1 = extent.1.max(value);
        })
        .or_insert((value, value));
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
        assert!(matches!(text_path_outline(&zero), TextPathOutline::Empty));
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

    #[test]
    fn complex_clusters_keep_one_curve_frame_and_logical_caret_order() {
        let node = line_node("a\u{301} שָׁלוֹם");
        let layout = layout_text_path(&node).expect("complex text lays out");
        let accent = node.content.find('\u{301}').expect("combining mark byte");
        let combining_cluster = layout
            .clusters
            .iter()
            .find(|cluster| cluster.utf8_range.start == 0 && cluster.utf8_range.end > accent)
            .expect("base and combining mark share a shaped cluster");
        assert_eq!(combining_cluster.carets.len(), 2);

        let hebrew_start = node.content.find('ש').expect("Hebrew byte");
        let rtl_cluster = layout
            .clusters
            .iter()
            .find(|cluster| cluster.utf8_range.start == hebrew_start)
            .expect("Hebrew cluster remains source-addressable");
        let logical_start = rtl_cluster.carets.first().expect("start caret");
        let logical_end = rtl_cluster.carets.last().expect("end caret");
        assert!(logical_start.segment.start[0] > logical_end.segment.start[0]);
    }

    #[test]
    fn visual_navigation_follows_shaped_bidi_order() {
        let node = line_node("abc אבג xyz");
        let layout = layout_text_path(&node).expect("mixed-direction text lays out");
        let stop_count = visual_caret_stops(&layout).len();
        let mut positions = vec![
            text_path_visual_line_edge(&node, TextPathVisualDirection::Previous)
                .expect("visual line start"),
        ];
        for _ in 1..stop_count {
            let next = text_path_visual_neighbor(
                &node,
                *positions.last().expect("at least the visual start"),
                TextPathVisualDirection::Next,
            )
            .expect("one next position per remaining visual stop");
            positions.push(next);
        }

        assert_eq!(positions.len(), stop_count);
        assert_eq!(
            positions.last().copied(),
            text_path_visual_line_edge(&node, TextPathVisualDirection::Next)
        );
        assert!(
            positions.windows(2).any(|pair| pair[1].byte < pair[0].byte),
            "moving visually forward must traverse the RTL run in descending byte order"
        );
        assert!(positions.iter().enumerate().any(|(index, position)| {
            positions
                .iter()
                .skip(index + 1)
                .any(|other| position.byte == other.byte && position.affinity != other.affinity)
        }));
        assert!(positions.iter().all(|position| {
            position.byte <= node.content.len() && node.content.is_char_boundary(position.byte)
        }));
        assert!(
            text_path_visual_neighbor(
                &node,
                *positions.last().expect("visual end"),
                TextPathVisualDirection::Next,
            )
            .is_none()
        );

        let pair = positions
            .windows(2)
            .find(|pair| pair[1].byte < pair[0].byte)
            .expect("a visually ordered RTL pair");
        assert_eq!(
            text_path_visual_selection_edge(
                &node,
                pair[1],
                pair[0],
                TextPathVisualDirection::Previous,
            ),
            Some(pair[0])
        );
        assert_eq!(
            text_path_visual_selection_edge(&node, pair[1], pair[0], TextPathVisualDirection::Next),
            Some(pair[1])
        );

        let rtl_node = line_node("אבג");
        let rtl_start = text_path_visual_line_edge(&rtl_node, TextPathVisualDirection::Previous)
            .expect("RTL visual line start");
        let rtl_end = text_path_visual_line_edge(&rtl_node, TextPathVisualDirection::Next)
            .expect("RTL visual line end");
        assert!(rtl_start.byte > rtl_end.byte);
    }

    #[test]
    fn visual_navigation_follows_reverse_path_traversal() {
        let mut node = line_node("abc");
        node.start = TextPathStart::new(0, 0.8).expect("valid path position");
        node.direction = TextPathDirection::Reverse;

        let visual_start = text_path_visual_line_edge(&node, TextPathVisualDirection::Previous)
            .expect("reverse visual start");
        let visual_end = text_path_visual_line_edge(&node, TextPathVisualDirection::Next)
            .expect("reverse visual end");
        assert_eq!(visual_start.byte, node.content.len());
        assert_eq!(visual_end.byte, 0);

        let middle = TextPathPosition {
            byte: 2,
            affinity: TextPathAffinity::Downstream,
        };
        assert_eq!(
            text_path_visual_neighbor(&node, middle, TextPathVisualDirection::Previous)
                .expect("Left follows reverse traversal")
                .byte,
            3
        );
        assert_eq!(
            text_path_visual_neighbor(&node, middle, TextPathVisualDirection::Next)
                .expect("Right follows reverse traversal")
                .byte,
            1
        );
    }

    #[test]
    fn reverse_visual_navigation_reverses_bidi_tie_affinities_without_looping() {
        let forward = line_node("abc אבג xyz");
        let forward_layout = layout_text_path(&forward).expect("forward text path lays out");
        let stop_count = visual_caret_stops(&forward_layout).len();
        let mut expected = vec![
            text_path_visual_line_edge(&forward, TextPathVisualDirection::Next)
                .expect("forward visual end"),
        ];
        for _ in 1..stop_count {
            let previous = text_path_visual_neighbor(
                &forward,
                *expected.last().expect("forward visual end is retained"),
                TextPathVisualDirection::Previous,
            )
            .expect("one previous position per remaining forward stop");
            expected.push(previous);
        }
        assert!(
            text_path_visual_neighbor(
                &forward,
                *expected.last().expect("forward visual start"),
                TextPathVisualDirection::Previous,
            )
            .is_none()
        );

        let mut reverse = forward;
        reverse.start = TextPathStart::new(0, 1.0).expect("valid path end");
        reverse.direction = TextPathDirection::Reverse;
        let reverse_layout = layout_text_path(&reverse).expect("reverse text path lays out");
        assert_eq!(visual_caret_stops(&reverse_layout).len(), stop_count);
        let mut actual = vec![
            text_path_visual_line_edge(&reverse, TextPathVisualDirection::Previous)
                .expect("reverse visual start"),
        ];
        for _ in 1..stop_count {
            let next = text_path_visual_neighbor(
                &reverse,
                *actual.last().expect("reverse visual start is retained"),
                TextPathVisualDirection::Next,
            )
            .expect("one next position per remaining reverse stop");
            actual.push(next);
        }
        assert!(
            text_path_visual_neighbor(
                &reverse,
                *actual.last().expect("reverse visual end"),
                TextPathVisualDirection::Next,
            )
            .is_none()
        );
        assert_eq!(actual, expected);
    }

    #[test]
    fn visual_navigation_retains_source_boundaries_clipped_by_an_open_path() {
        let mut path = PathData::new();
        path.move_to(0.0, 20.0).line_to(42.0, 20.0);
        let mut node = TextPathNode::new(path, "ABCDEFGHIJ🦀");
        node.style.size_px = 20.0;

        let layout = layout_text_path(&node).expect("source navigation survives path clipping");
        let placed_end = layout
            .clusters
            .iter()
            .map(|cluster| cluster.utf8_range.end)
            .max()
            .expect("a visible prefix fits the short path");
        assert!(
            placed_end < node.content.len(),
            "the suffix must be clipped"
        );

        let expected_bytes = node
            .content
            .grapheme_indices(true)
            .map(|(byte, _)| byte)
            .chain(std::iter::once(node.content.len()))
            .collect::<Vec<_>>();
        let visual_start = text_path_visual_line_edge(&node, TextPathVisualDirection::Previous)
            .expect("full-source visual start");
        let visual_end = text_path_visual_line_edge(&node, TextPathVisualDirection::Next)
            .expect("full-source visual end");
        assert_eq!(visual_start.byte, 0);
        assert_eq!(visual_end.byte, node.content.len());

        let mut positions = vec![visual_start];
        while let Some(next) = text_path_visual_neighbor(
            &node,
            *positions.last().expect("visual start is retained"),
            TextPathVisualDirection::Next,
        ) {
            assert!(
                positions.len() < expected_bytes.len(),
                "visual traversal must not cycle"
            );
            positions.push(next);
        }
        assert_eq!(
            positions
                .iter()
                .map(|position| position.byte)
                .collect::<Vec<_>>(),
            expected_bytes
        );

        let previous =
            text_path_visual_neighbor(&node, visual_end, TextPathVisualDirection::Previous)
                .expect("Left enters the clipped suffix by one grapheme");
        assert_eq!(previous.byte, node.content.find('🦀').expect("crab byte"));
        assert_eq!(
            text_path_visual_neighbor(&node, previous, TextPathVisualDirection::Next),
            Some(visual_end),
            "Right returns through the clipped suffix"
        );
    }

    #[test]
    fn caret_hit_test_and_selection_round_trip_utf8_boundaries() {
        let mut path = PathData::new();
        path.move_to(20.0, 160.0)
            .cubic_to(180.0, 10.0, 320.0, 260.0, 520.0, 100.0);
        let mut node = TextPathNode::new(path, "office 🦀");
        node.style.size_px = 30.0;

        let mut boundaries: Vec<_> = node
            .content
            .grapheme_indices(true)
            .map(|(byte, _)| byte)
            .collect();
        boundaries.push(node.content.len());
        for byte in boundaries {
            let segment = text_path_caret_segment(&node, byte).expect("caret on the path");
            let midpoint = [
                (segment.start[0] + segment.end[0]) * 0.5,
                (segment.start[1] + segment.end[1]) * 0.5,
            ];
            assert_eq!(text_path_hit_test(&node, midpoint), Some(byte));
        }

        let selection = text_path_selection_quads(&node, 1..node.content.len());
        assert!(!selection.is_empty());
        assert!(selection.iter().all(|quad| {
            quad.utf8_range.start < quad.utf8_range.end
                && quad
                    .points
                    .iter()
                    .flatten()
                    .all(|coordinate| coordinate.is_finite())
        }));
    }

    #[test]
    fn trailing_space_keeps_its_source_caret_and_alignment_advance() {
        let mut node = line_node("A ");
        node.start = TextPathStart::new(0, 0.5).expect("valid midpoint");
        let layout = layout_text_path(&node).expect("trailing space lays out");
        assert!(
            layout
                .clusters
                .iter()
                .any(|cluster| cluster.utf8_range == (1..2)),
            "the glyphless trailing-space cluster must survive shaping"
        );

        let before_space = text_path_caret_segment(&node, 1).expect("caret before space");
        let after_space = text_path_caret_segment(&node, 2).expect("caret after space");
        assert!(after_space.start[0] > before_space.start[0]);

        let mut end_aligned = node.clone();
        end_aligned.alignment = TextPathAlignment::End;
        let end_aligned_origin = glyph_origins(&end_aligned)[0][0];
        let mut without_space = line_node("A");
        without_space.start = node.start;
        without_space.alignment = TextPathAlignment::End;
        let without_space_origin = glyph_origins(&without_space)[0][0];
        assert!(
            end_aligned_origin < without_space_origin,
            "end alignment must include the trailing-space advance"
        );
    }

    #[test]
    fn whitespace_only_content_keeps_all_carets_and_alignment_advance() {
        let mut node = line_node("   ");
        node.start = TextPathStart::new(0, 0.5).expect("valid midpoint");
        let layout = layout_text_path(&node).expect("whitespace-only text lays out");
        assert!(!layout.clusters.is_empty());
        assert!(text_path_visual_bounds(&node).is_none());

        let carets = (0..=node.content.len())
            .map(|byte| text_path_caret_segment(&node, byte).expect("whitespace caret"))
            .collect::<Vec<_>>();
        assert!(
            carets
                .windows(2)
                .all(|pair| pair[1].start[0] > pair[0].start[0])
        );
        for (byte, caret) in carets.iter().enumerate() {
            let midpoint = [
                (caret.start[0] + caret.end[0]) * 0.5,
                (caret.start[1] + caret.end[1]) * 0.5,
            ];
            assert_eq!(text_path_hit_test(&node, midpoint), Some(byte));
        }
        let start = carets[0];

        let mut end_aligned = node;
        end_aligned.alignment = TextPathAlignment::End;
        let aligned_end = text_path_caret_segment(&end_aligned, end_aligned.content.len())
            .expect("end-aligned trailing caret");
        assert!((aligned_end.start[0] - start.start[0]).abs() < 0.01);
    }

    #[test]
    fn empty_content_has_a_rotated_fallback_caret_but_no_visual_bounds() {
        let mut node = line_node("");
        node.start = TextPathStart::new(0, 0.4).expect("valid path position");
        let caret = text_path_caret_segment(&node, 0).expect("empty fallback caret");
        assert!((caret.start[0] - 200.0).abs() < 0.01);
        assert!((caret.end[0] - 200.0).abs() < 0.01);
        assert!(caret.start[1] < 100.0 && caret.end[1] > 100.0);
        assert_eq!(text_path_hit_test(&node, [350.0, 20.0]), Some(0));
        let position = TextPathPosition {
            byte: 0,
            affinity: TextPathAffinity::Downstream,
        };
        assert_eq!(
            text_path_visual_line_edge(&node, TextPathVisualDirection::Previous),
            Some(position)
        );
        assert_eq!(
            text_path_visual_line_edge(&node, TextPathVisualDirection::Next),
            Some(position)
        );
        assert!(
            text_path_visual_neighbor(&node, position, TextPathVisualDirection::Next).is_none()
        );
        assert!(text_path_visual_bounds(&node).is_none());
        assert!(!text_path_contains_point(&node, [200.0, 100.0]));
    }

    #[test]
    fn unsupported_glyph_outline_never_becomes_a_rectangular_effect_silhouette() {
        let mut partial_vector_outline = Path::new();
        partial_vector_outline.add_rect(Rect::from_xywh(1.0, 2.0, 3.0, 4.0), None);
        let unsupported = classify_outline(partial_vector_outline, true);
        assert!(matches!(unsupported, TextPathOutline::UnsupportedGlyph));
        assert!(unsupported.into_exact().is_none());
        assert!(matches!(
            classify_outline(Path::new(), false),
            TextPathOutline::Empty
        ));
    }
}
