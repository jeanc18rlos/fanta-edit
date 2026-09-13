use std::error::Error;
use std::fmt;

use fanta_doc::{
    BlendMode, Doc, Fill, MeasuredPath, NodeData, NodeFlags, NodeId, Operation, PathSegment,
    TextPathNode, TextPathStart,
};

use crate::text::PLACEHOLDER;

const MINIMUM_PATH_LENGTH: f64 = 1.0e-6;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextPathConversionError {
    SelectOneVector,
    MissingNode,
    LockedOrHidden,
    DegeneratePath,
    RoundedGeometry,
    ClippedGeometry,
    UnsupportedPaint,
}

impl fmt::Display for TextPathConversionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::SelectOneVector => "Select exactly one vector path first.",
            Self::MissingNode => "The selected vector no longer exists.",
            Self::LockedOrHidden => "Unlock and show the selected vector before converting it.",
            Self::DegeneratePath => "The selected vector needs a finite, non-zero path.",
            Self::RoundedGeometry => {
                "Flatten the vector's rounded corners before converting it to text on a path."
            }
            Self::ClippedGeometry => {
                "Remove or flatten the vector's clipping viewport before converting it."
            }
            Self::UnsupportedPaint => {
                "Text on Path supports an unpainted vector or one normal solid fill or stroke. Remove gradients, image paints, blends, or extra paints and try again."
            }
        })
    }
}

impl Error for TextPathConversionError {}

pub fn text_path_conversion(doc: &Doc) -> Result<(NodeId, Operation), TextPathConversionError> {
    let &[node_id] = doc.selection.as_slice() else {
        return Err(TextPathConversionError::SelectOneVector);
    };
    let node = doc
        .scene
        .get(node_id)
        .ok_or(TextPathConversionError::MissingNode)?;
    if node.flags.intersects(NodeFlags::LOCKED | NodeFlags::HIDDEN)
        || doc.scene.ancestors_of(node_id).any(|ancestor| {
            ancestor
                .flags
                .intersects(NodeFlags::LOCKED | NodeFlags::HIDDEN)
        })
    {
        return Err(TextPathConversionError::LockedOrHidden);
    }
    let NodeData::Vector(vector) = &node.data else {
        return Err(TextPathConversionError::SelectOneVector);
    };
    if vector.corner_radius.is_some()
        || vector.corner_radii.is_some()
        || vector.corner_smoothing != 0.0
    {
        return Err(TextPathConversionError::RoundedGeometry);
    }
    if vector.local_size.is_some() {
        return Err(TextPathConversionError::ClippedGeometry);
    }
    if !vector.path.segments.iter().all(path_segment_is_finite) {
        return Err(TextPathConversionError::DegeneratePath);
    }
    let measured = MeasuredPath::new(&vector.path);
    if !measured.total_length().is_finite() || measured.total_length() <= MINIMUM_PATH_LENGTH {
        return Err(TextPathConversionError::DegeneratePath);
    }
    let start_segment = measured
        .segments()
        .iter()
        .find(|segment| segment.length().is_finite() && segment.length() > MINIMUM_PATH_LENGTH)
        .and_then(|segment| u32::try_from(segment.drawable_segment_index()).ok())
        .ok_or(TextPathConversionError::DegeneratePath)?;

    let color = match (vector.fills.as_slice(), vector.strokes.as_slice()) {
        ([], []) => fanta_doc::Color::BLACK,
        ([fill], []) => normal_solid_color(fill)?,
        ([], [stroke]) => normal_solid_color(&stroke.paint)?,
        _ => return Err(TextPathConversionError::UnsupportedPaint),
    };
    let mut text_path = TextPathNode::new(vector.path.clone(), PLACEHOLDER);
    text_path.style.color = color;
    text_path.start = TextPathStart::DEFAULT.with_segment(start_segment);
    Ok((
        node_id,
        Operation::ReplaceData {
            id: node_id,
            old: Box::new(node.data.clone()),
            new: Box::new(NodeData::TextPath(text_path)),
        },
    ))
}

fn path_segment_is_finite(segment: &PathSegment) -> bool {
    let point_is_finite = |point: &[f64; 2]| point.iter().all(|coordinate| coordinate.is_finite());
    match segment {
        PathSegment::Move { to } | PathSegment::Line { to } => point_is_finite(to),
        PathSegment::Quad { ctrl, to } => point_is_finite(ctrl) && point_is_finite(to),
        PathSegment::Cubic { ctrl1, ctrl2, to } => {
            point_is_finite(ctrl1) && point_is_finite(ctrl2) && point_is_finite(to)
        }
        PathSegment::Close => true,
    }
}

fn normal_solid_color(fill: &Fill) -> Result<fanta_doc::Color, TextPathConversionError> {
    match fill {
        Fill::Solid {
            color,
            blend: BlendMode::Normal,
        } => Ok(*color),
        Fill::Solid { .. } | Fill::Gradient { .. } | Fill::Image { .. } => {
            Err(TextPathConversionError::UnsupportedPaint)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fanta_doc::{CanvasNode, GroupNode, PathData, Transform2D, VectorNode};

    fn doc_with_vector(vector: VectorNode) -> (Doc, NodeId) {
        let mut doc = Doc::new();
        let node = CanvasNode::new(NodeData::Vector(vector));
        let node_id = node.id;
        doc.apply(Operation::create_node(node))
            .expect("create vector");
        doc.selection.select_only(node_id);
        doc.history = Default::default();
        (doc, node_id)
    }

    fn plain_path() -> PathData {
        let mut path = PathData::new();
        path.move_to(0.0, 0.0).line_to(100.0, 20.0);
        path
    }

    #[test]
    fn conversion_replaces_only_data_and_is_undoable() {
        let color = fanta_doc::Color::rgb(12, 34, 56);
        let vector = VectorNode {
            path: plain_path(),
            strokes: smallvec::smallvec![fanta_doc::Stroke::solid(color, 3.0)],
            ..VectorNode::default()
        };
        let (mut doc, node_id) = doc_with_vector(vector.clone());
        {
            let node = doc.scene.get_mut(node_id).expect("node");
            node.name = "Orbit".to_owned();
            node.transform = Transform2D::translation(40.0, 80.0);
            node.meta = serde_json::json!({"source": "test"});
        }
        let wrapper_before = doc.scene.get(node_id).expect("node").clone();

        let (converted_id, operation) = text_path_conversion(&doc).expect("convertible");
        assert_eq!(converted_id, node_id);
        doc.apply(operation).expect("apply conversion");

        let converted = doc.scene.get(node_id).expect("same node");
        assert_eq!(converted.id, wrapper_before.id);
        assert_eq!(converted.name, wrapper_before.name);
        assert_eq!(converted.transform, wrapper_before.transform);
        assert_eq!(converted.meta, wrapper_before.meta);
        let NodeData::TextPath(text_path) = &converted.data else {
            panic!("expected text path");
        };
        assert_eq!(text_path.path, vector.path);
        assert_eq!(text_path.content, PLACEHOLDER);
        assert_eq!(text_path.style.color, color);
        assert_eq!(doc.selection.as_slice(), [node_id]);

        assert!(doc.undo().expect("undo conversion"));
        assert!(matches!(
            doc.scene.get(node_id).map(|node| &node.data),
            Some(NodeData::Vector(_))
        ));
        assert!(doc.redo().expect("redo conversion"));
        assert!(matches!(
            doc.scene.get(node_id).map(|node| &node.data),
            Some(NodeData::TextPath(_))
        ));
    }

    #[test]
    fn conversion_rejects_ambiguous_paints_without_mutating_history() {
        let vector = VectorNode {
            path: plain_path(),
            fills: smallvec::smallvec![Fill::solid(fanta_doc::Color::BLACK)],
            strokes: smallvec::smallvec![fanta_doc::Stroke::solid(fanta_doc::Color::WHITE, 1.0,)],
            ..VectorNode::default()
        };
        let (doc, node_id) = doc_with_vector(vector);
        let undo_depth = doc.history.undo_depth();

        assert_eq!(
            text_path_conversion(&doc).expect_err("ambiguous paint must fail"),
            TextPathConversionError::UnsupportedPaint
        );
        assert_eq!(doc.history.undo_depth(), undo_depth);
        assert!(matches!(
            doc.scene.get(node_id).map(|node| &node.data),
            Some(NodeData::Vector(_))
        ));
    }

    #[test]
    fn conversion_rejects_geometry_that_cannot_round_trip_visually() {
        let mut rounded = VectorNode {
            path: PathData::rect(0.0, 0.0, 100.0, 40.0),
            ..VectorNode::default()
        };
        rounded.corner_radius = Some(8.0);
        let (doc, _) = doc_with_vector(rounded);
        assert_eq!(
            text_path_conversion(&doc).expect_err("rounded geometry must fail"),
            TextPathConversionError::RoundedGeometry
        );

        let mut clipped = VectorNode {
            path: plain_path(),
            ..VectorNode::default()
        };
        clipped.local_size = Some([50.0, 50.0]);
        let (doc, _) = doc_with_vector(clipped);
        assert_eq!(
            text_path_conversion(&doc).expect_err("clipped geometry must fail"),
            TextPathConversionError::ClippedGeometry
        );
    }

    #[test]
    fn conversion_rejects_a_degenerate_path() {
        let vector = VectorNode {
            path: PathData::new(),
            ..VectorNode::default()
        };
        let (doc, _) = doc_with_vector(vector);
        assert_eq!(
            text_path_conversion(&doc).expect_err("empty path must fail"),
            TextPathConversionError::DegeneratePath
        );

        let mut path = plain_path();
        path.move_to(f64::NAN, 0.0).line_to(20.0, 20.0);
        let vector = VectorNode {
            path,
            ..VectorNode::default()
        };
        let (doc, _) = doc_with_vector(vector);
        assert_eq!(
            text_path_conversion(&doc).expect_err("non-finite geometry must fail"),
            TextPathConversionError::DegeneratePath
        );
    }

    #[test]
    fn conversion_starts_on_the_first_non_degenerate_segment() {
        let mut path = PathData::new();
        path.move_to(5.0, 5.0)
            .line_to(5.0, 5.0)
            .move_to(20.0, 30.0)
            .line_to(120.0, 30.0);
        let vector = VectorNode {
            path,
            ..VectorNode::default()
        };
        let (doc, _) = doc_with_vector(vector);

        let (_, operation) = text_path_conversion(&doc).expect("second contour is drawable");
        let Operation::ReplaceData { new, .. } = operation else {
            panic!("conversion must replace node data");
        };
        let NodeData::TextPath(text_path) = *new else {
            panic!("conversion must create a text path");
        };
        assert_eq!(text_path.start.segment(), 1);
    }

    #[test]
    fn conversion_rejects_a_vector_under_a_locked_ancestor() {
        let mut doc = Doc::new();
        let mut group = CanvasNode::new(NodeData::Group(GroupNode::default()));
        group.flags.insert(NodeFlags::LOCKED);
        let group_id = group.id;
        doc.apply(Operation::create_node(group))
            .expect("create locked group");
        let mut vector = CanvasNode::new(NodeData::Vector(VectorNode {
            path: plain_path(),
            ..VectorNode::default()
        }));
        vector.parent = Some(group_id);
        let vector_id = vector.id;
        doc.apply(Operation::create_node(vector))
            .expect("create child vector");
        doc.selection.select_only(vector_id);

        assert_eq!(
            text_path_conversion(&doc).expect_err("locked ancestor must fail"),
            TextPathConversionError::LockedOrHidden
        );
    }
}
