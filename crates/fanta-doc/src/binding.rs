//! [`BoundProp`] — the single property address shared by variable bindings and
//! instance overrides.
//!
//! ## Why one address type
//!
//! Two features need to name "a specific writable property of a node": variable
//! bindings (`CanvasNode.bindings: BTreeMap<BoundProp, VariableId>`) and
//! instance overrides (`Override.target_prop: BoundProp`). They are the same
//! question — *which* property — so they use the same key. That also means the
//! read/write logic lives in exactly one place: [`BoundProp::read_literal`]
//! (snapshot the current value, for `UnbindProperty`'s "bake the literal back"
//! and for diffing) and [`BoundProp::apply_resolved`] (write a resolved value
//! in). Render-time variable substitution and override application both go
//! through `apply_resolved`, so a node can never be written two slightly
//! different ways.
//!
//! `Eq + Hash + Ord` is required so `BoundProp` can key a `BTreeMap` and so two
//! overrides targeting the same property compare equal.

use crate::color::Color;
use crate::node::{CanvasNode, NodeData, NodeFlags};
use crate::style::{Fill, UnitInterval};
use crate::value::{ResolvedVarValue, VariableType};
use serde::{Deserialize, Serialize};

/// Addresses one variable-bindable / overridable property of a node.
///
/// Indexed variants (`FillColor`, `StrokeColor`, `StrokeWidth`) name a slot in
/// the node's stacked `fills` / `strokes` `SmallVec` — binding the *second*
/// fill's color is `FillColor { index: 1 }`. Out-of-range indices are tolerated
/// (the accessors no-op) so a stale binding after a fill is deleted never panics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "prop", rename_all = "snake_case")]
pub enum BoundProp {
    /// Color of `fills[index]`.
    FillColor { index: u16 },
    /// Color of `strokes[index]`'s paint.
    StrokeColor { index: u16 },
    /// Width of `strokes[index]`.
    StrokeWidth { index: u16 },
    /// `VectorNode.corner_radius` (vectors only).
    CornerRadius,
    /// Node-level `opacity`.
    Opacity,
    /// Visibility — backed by the [`NodeFlags::HIDDEN`] bit, not a scalar bool.
    Visible,
    /// `TextNode.content` (text only).
    TextContent,
    /// The whole `TextNode.style` — a composite typography token (font family,
    /// size, weight, spacing, …), bound to a [`VariableType::Typography`]
    /// variable ([`VarValue::TextStyle`]). Text only.
    ///
    /// [`VariableType::Typography`]: crate::value::VariableType::Typography
    /// [`VarValue::TextStyle`]: crate::value::VarValue::TextStyle
    TextStyle,
    /// Clip width — `GroupNode.clip_size[0]` (frames only).
    ClipWidth,
    /// Clip height — `GroupNode.clip_size[1]` (frames only).
    ClipHeight,
}

impl BoundProp {
    /// The variable type this property accepts. Inspector and component-schema
    /// UIs use this to filter variables before authoring a binding, while
    /// render-time application remains tolerant of stale or mismatched data.
    pub const fn variable_type(self) -> VariableType {
        match self {
            Self::FillColor { .. } | Self::StrokeColor { .. } => VariableType::Color,
            Self::StrokeWidth { .. }
            | Self::CornerRadius
            | Self::Opacity
            | Self::ClipWidth
            | Self::ClipHeight => VariableType::Float,
            Self::Visible => VariableType::Boolean,
            Self::TextContent => VariableType::String,
            Self::TextStyle => VariableType::Typography,
        }
    }

    pub fn accepts_variable_type(self, variable_type: VariableType) -> bool {
        self.variable_type() == variable_type
    }

    /// Whether this property is addressable on `node` — i.e. the slot it names
    /// exists on the node's variant.
    ///
    /// This is **slot existence**, deliberately not "does it currently hold a
    /// readable value": a gradient fill has no solid color, yet `FillColor`
    /// still applies to it (binding a color replaces the gradient with a
    /// solid). Callers that filter bindings against a built node — e.g. the
    /// `.fig` importer dropping a fill binding on a node that landed no such
    /// fill — use this, and must not conflate it with [`read_literal`] (which
    /// is `Null` for a gradient).
    ///
    /// [`read_literal`]: BoundProp::read_literal
    pub fn applies_to(&self, node: &CanvasNode) -> bool {
        match self {
            Self::FillColor { index } => match &node.data {
                NodeData::Vector(v) => v.fills.get(*index as usize).is_some(),
                NodeData::Group(g) => group_fill(g, *index).is_some(),
                NodeData::Text(_) => *index == 0,
                _ => false,
            },
            Self::StrokeColor { index } | Self::StrokeWidth { index } => node
                .data
                .strokes()
                .is_some_and(|s| s.get(*index as usize).is_some()),
            Self::CornerRadius => matches!(node.data, NodeData::Vector(_)),
            Self::Opacity | Self::Visible => true,
            Self::TextContent | Self::TextStyle => matches!(node.data, NodeData::Text(_)),
            Self::ClipWidth | Self::ClipHeight => {
                matches!(&node.data, NodeData::Group(g) if g.clip_size.is_some())
            }
        }
    }

    /// Snapshot the node's current literal value at this property, as a
    /// `serde_json::Value`. Used by `UnbindProperty` to bake the
    /// last-resolved literal back onto the node, and as a generic diff probe.
    ///
    /// Returns `Null` when the property doesn't apply to this node's variant or
    /// the indexed slot is missing — callers treat that as "nothing to read".
    pub fn read_literal(&self, node: &CanvasNode) -> serde_json::Value {
        use serde_json::json;
        match self {
            Self::FillColor { index } => node_fill_color(node, *index)
                .map(|c| json!(c.to_hex()))
                .unwrap_or(serde_json::Value::Null),
            Self::StrokeColor { index } => stroke_color(node, *index)
                .map(|c| json!(c.to_hex()))
                .unwrap_or(serde_json::Value::Null),
            Self::StrokeWidth { index } => node
                .data
                .strokes()
                .and_then(|s| s.get(*index as usize))
                .map(|s| json!(s.width))
                .unwrap_or(serde_json::Value::Null),
            Self::CornerRadius => match &node.data {
                NodeData::Vector(v) => v
                    .corner_radius
                    .map(|r| json!(r))
                    .unwrap_or(serde_json::Value::Null),
                _ => serde_json::Value::Null,
            },
            Self::Opacity => json!(node.opacity.get()),
            Self::Visible => json!(!node.flags.contains(NodeFlags::HIDDEN)),
            Self::TextContent => match &node.data {
                NodeData::Text(t) => json!(t.content),
                _ => serde_json::Value::Null,
            },
            Self::TextStyle => match &node.data {
                NodeData::Text(t) => {
                    serde_json::to_value(&t.style).unwrap_or(serde_json::Value::Null)
                }
                _ => serde_json::Value::Null,
            },
            Self::ClipWidth => clip_size(node)
                .map(|s| json!(s[0]))
                .unwrap_or(serde_json::Value::Null),
            Self::ClipHeight => clip_size(node)
                .map(|s| json!(s[1]))
                .unwrap_or(serde_json::Value::Null),
        }
    }

    /// Read the node's current literal through the same typed vocabulary used
    /// by variables and motion keyframes. Returns `None` when this property
    /// does not apply or its indexed paint slot has no resolvable value.
    pub fn read_resolved(&self, node: &CanvasNode) -> Option<ResolvedVarValue> {
        match self {
            Self::FillColor { index } => {
                node_fill_color(node, *index).map(|value| ResolvedVarValue::Color { value })
            }
            Self::StrokeColor { index } => {
                stroke_color(node, *index).map(|value| ResolvedVarValue::Color { value })
            }
            Self::StrokeWidth { index } => node
                .data
                .strokes()
                .and_then(|strokes| strokes.get(*index as usize))
                .map(|stroke| ResolvedVarValue::Float {
                    value: stroke.width,
                }),
            Self::CornerRadius => match &node.data {
                NodeData::Vector(vector) => vector
                    .corner_radius
                    .map(|value| ResolvedVarValue::Float { value }),
                _ => None,
            },
            Self::Opacity => Some(ResolvedVarValue::Float {
                value: f64::from(node.opacity.get()),
            }),
            Self::Visible => Some(ResolvedVarValue::Boolean {
                value: !node.flags.contains(NodeFlags::HIDDEN),
            }),
            Self::TextContent => match &node.data {
                NodeData::Text(text) => Some(ResolvedVarValue::String {
                    value: text.content.clone(),
                }),
                _ => None,
            },
            Self::TextStyle => match &node.data {
                NodeData::Text(text) => Some(ResolvedVarValue::TextStyle {
                    value: text.style.clone(),
                }),
                _ => None,
            },
            Self::ClipWidth => {
                clip_size(node).map(|size| ResolvedVarValue::Float { value: size[0] })
            }
            Self::ClipHeight => {
                clip_size(node).map(|size| ResolvedVarValue::Float { value: size[1] })
            }
        }
    }

    /// Write a resolved value into the node at this property. Type-mismatched
    /// values (e.g. a `Float` aimed at `FillColor`) and out-of-range indices
    /// are ignored rather than panicking, mirroring the dangling-ref tolerance
    /// the rest of the model takes (see `Doc::validate`). Returns whether
    /// anything was written.
    pub fn apply_resolved(&self, node: &mut CanvasNode, value: ResolvedVarValue) -> bool {
        match (self, value) {
            (Self::FillColor { index }, ResolvedVarValue::Color { value }) => {
                set_node_fill_color(node, *index, value)
            }
            (Self::StrokeColor { index }, ResolvedVarValue::Color { value }) => {
                if let Some(stroke) = node
                    .data
                    .strokes_mut()
                    .and_then(|s| s.get_mut(*index as usize))
                {
                    stroke.paint.set_solid_color(value);
                    return true;
                }
                false
            }
            (Self::StrokeWidth { index }, ResolvedVarValue::Float { value }) => {
                if let Some(stroke) = node
                    .data
                    .strokes_mut()
                    .and_then(|s| s.get_mut(*index as usize))
                {
                    stroke.width = value;
                    return true;
                }
                false
            }
            (Self::CornerRadius, ResolvedVarValue::Float { value }) => {
                if let NodeData::Vector(v) = &mut node.data {
                    v.corner_radius = Some(value);
                    return true;
                }
                false
            }
            (Self::Opacity, ResolvedVarValue::Float { value }) => {
                node.opacity = UnitInterval::new(value as f32);
                true
            }
            (Self::Visible, ResolvedVarValue::Boolean { value }) => {
                // No scalar bool — visibility is the HIDDEN bitflag, inverted.
                node.flags.set(NodeFlags::HIDDEN, !value);
                true
            }
            (Self::TextContent, ResolvedVarValue::String { value }) => {
                if let NodeData::Text(t) = &mut node.data {
                    t.content = value;
                    return true;
                }
                false
            }
            (Self::TextStyle, ResolvedVarValue::TextStyle { value }) => {
                // A typography token replaces the node's whole base style; the
                // per-run overrides in `style_runs` still layer on top.
                if let NodeData::Text(t) = &mut node.data {
                    t.style = value;
                    return true;
                }
                false
            }
            (Self::ClipWidth, ResolvedVarValue::Float { value }) => {
                if let Some(size) = clip_size_mut(node) {
                    size[0] = value;
                    return true;
                }
                false
            }
            (Self::ClipHeight, ResolvedVarValue::Float { value }) => {
                if let Some(size) = clip_size_mut(node) {
                    size[1] = value;
                    return true;
                }
                false
            }
            // Type mismatch (e.g. a string aimed at Opacity) — ignore.
            _ => false,
        }
    }
}

// ---- shared field accessors -------------------------------------------------
//
// These keep `read_literal` / `apply_resolved` from open-coding the same
// variant checks. A Figma fill can land as vector `fills[index]`, a frame
// `background`, or a text glyph color; strokes come from
// `NodeData::strokes` (vectors or frames).

fn node_fill_color(node: &CanvasNode, index: u16) -> Option<Color> {
    match &node.data {
        NodeData::Vector(v) => fill_color(v.fills.get(index as usize)),
        NodeData::Group(g) => fill_color(group_fill(g, index)),
        NodeData::Text(t) if index == 0 => Some(t.style.color),
        _ => None,
    }
}

fn set_node_fill_color(node: &mut CanvasNode, index: u16, color: Color) -> bool {
    match &mut node.data {
        NodeData::Vector(v) => {
            let Some(slot) = v.fills.get_mut(index as usize) else {
                return false;
            };
            slot.set_solid_color(color);
            true
        }
        NodeData::Group(g) => {
            let Some(slot) = group_fill_mut(g, index) else {
                return false;
            };
            slot.set_solid_color(color);
            true
        }
        NodeData::Text(t) if index == 0 => {
            t.set_glyph_color(color);
            true
        }
        _ => false,
    }
}

fn fill_color(fill: Option<&Fill>) -> Option<Color> {
    fill?.solid_color()
}

fn group_fill(group: &crate::node::GroupNode, index: u16) -> Option<&Fill> {
    if let Some(background) = group.background.as_ref() {
        if index == 0 {
            return Some(background);
        }
        group.background_fills.get(index as usize - 1)
    } else {
        group.background_fills.get(index as usize)
    }
}

fn group_fill_mut(group: &mut crate::node::GroupNode, index: u16) -> Option<&mut Fill> {
    if group.background.is_some() {
        if index == 0 {
            return group.background.as_mut();
        }
        group.background_fills.get_mut(index as usize - 1)
    } else {
        group.background_fills.get_mut(index as usize)
    }
}

fn stroke_color(node: &CanvasNode, index: u16) -> Option<Color> {
    node.data
        .strokes()?
        .get(index as usize)?
        .paint
        .solid_color()
}

fn clip_size(node: &CanvasNode) -> Option<[f64; 2]> {
    match &node.data {
        NodeData::Group(g) => g.clip_size,
        _ => None,
    }
}

fn clip_size_mut(node: &mut CanvasNode) -> Option<&mut [f64; 2]> {
    match &mut node.data {
        NodeData::Group(g) => g.clip_size.as_mut(),
        _ => None,
    }
}

/// serde adapter for `CanvasNode.bindings`.
///
/// A `BTreeMap<BoundProp, VariableId>` cannot serialize as a JSON object: its
/// key is a tagged enum (`{"prop":"fill_color","index":0}`), and JSON object
/// keys must be strings. So we project the map to a sequence of `[prop, var]`
/// pairs on the wire and rebuild the map on load. The in-memory type stays a
/// `BTreeMap` (deterministic order, O(log n) lookup) — only the projection
/// changes.
pub(crate) mod map_as_seq {
    use super::BoundProp;
    use crate::id::VariableId;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::collections::BTreeMap;

    pub fn serialize<S: Serializer>(
        map: &BTreeMap<BoundProp, VariableId>,
        s: S,
    ) -> Result<S::Ok, S::Error> {
        map.iter().collect::<Vec<_>>().serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        d: D,
    ) -> Result<BTreeMap<BoundProp, VariableId>, D::Error> {
        Ok(Vec::<(BoundProp, VariableId)>::deserialize(d)?
            .into_iter()
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::Color;
    use crate::node::{GroupNode, VectorNode};

    fn rect() -> CanvasNode {
        CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
            0.0,
            0.0,
            10.0,
            10.0,
            Color::WHITE,
        )))
    }

    #[test]
    fn applies_to_is_slot_existence_not_color_readability() {
        use crate::color::{Gradient, GradientStop};

        // A GRADIENT fill occupies fill slot 0. `FillColor` still applies to it
        // (binding a color replaces the gradient) even though `read_literal`
        // would report `Null` — this is the exact case a naive
        // read_literal-based predicate gets wrong.
        let mut grad_rect = rect();
        if let NodeData::Vector(v) = &mut grad_rect.data {
            v.fills[0] = Fill::Gradient {
                gradient: Gradient::Linear {
                    start: [0.0, 0.0],
                    end: [1.0, 1.0],
                    stops: vec![GradientStop {
                        position: 0.0,
                        color: Color::WHITE,
                    }],
                },
                blend: Default::default(),
            };
        }
        assert!(BoundProp::FillColor { index: 0 }.applies_to(&grad_rect));
        assert_eq!(
            BoundProp::FillColor { index: 0 }.read_literal(&grad_rect),
            serde_json::Value::Null,
            "a gradient reads Null yet the binding still applies"
        );

        // Out-of-range and wrong-variant slots do not apply.
        assert!(!BoundProp::FillColor { index: 5 }.applies_to(&grad_rect));
        assert!(!BoundProp::StrokeColor { index: 0 }.applies_to(&grad_rect));
        assert!(!BoundProp::TextContent.applies_to(&grad_rect));

        // Wrapper-level props apply to any node; text/clip props are variant-gated.
        assert!(BoundProp::Opacity.applies_to(&grad_rect));
        assert!(BoundProp::Visible.applies_to(&grad_rect));
        let frame = CanvasNode::new(NodeData::Group(GroupNode {
            clip_size: Some([10.0, 10.0]),
            ..GroupNode::default()
        }));
        assert!(BoundProp::ClipWidth.applies_to(&frame));
        assert!(!BoundProp::ClipWidth.applies_to(&grad_rect));
    }

    #[test]
    fn every_bound_property_declares_its_variable_type() {
        assert_eq!(
            BoundProp::FillColor { index: 0 }.variable_type(),
            VariableType::Color
        );
        assert_eq!(BoundProp::Opacity.variable_type(), VariableType::Float);
        assert_eq!(BoundProp::Visible.variable_type(), VariableType::Boolean);
        assert_eq!(BoundProp::TextContent.variable_type(), VariableType::String);
        assert_eq!(
            BoundProp::TextStyle.variable_type(),
            VariableType::Typography
        );
        assert!(BoundProp::ClipWidth.accepts_variable_type(VariableType::Float));
        assert!(!BoundProp::ClipWidth.accepts_variable_type(VariableType::Color));
    }

    #[test]
    fn text_style_binding_replaces_the_whole_node_style() {
        use crate::node::{TextNode, TextStyle};
        use crate::value::{ResolvedVarValue, VariableType};

        let mut node = CanvasNode::new(NodeData::Text(TextNode::new("hi", 100.0, 40.0)));
        // A typography token addresses text nodes only.
        assert!(BoundProp::TextStyle.applies_to(&node));
        assert!(!BoundProp::TextStyle.applies_to(&rect()));

        // Resolving a Typography variable replaces the node's whole base style.
        let big = TextStyle {
            size_px: 48.0,
            weight: 700,
            ..TextStyle::default()
        };
        assert_eq!(
            ResolvedVarValue::TextStyle { value: big.clone() }.variable_type(),
            VariableType::Typography
        );
        let wrote = BoundProp::TextStyle.apply_resolved(
            &mut node,
            ResolvedVarValue::TextStyle { value: big.clone() },
        );
        assert!(wrote);
        match &node.data {
            NodeData::Text(t) => assert_eq!(t.style, big),
            _ => panic!("expected text node"),
        }

        // A non-text node ignores the write.
        let mut r = rect();
        assert!(
            !BoundProp::TextStyle
                .apply_resolved(&mut r, ResolvedVarValue::TextStyle { value: big })
        );
    }

    #[test]
    fn visible_flips_hidden_flag() {
        let mut n = rect();
        assert!(!n.flags.contains(NodeFlags::HIDDEN));
        let wrote =
            BoundProp::Visible.apply_resolved(&mut n, ResolvedVarValue::Boolean { value: false });
        assert!(wrote);
        assert!(
            n.flags.contains(NodeFlags::HIDDEN),
            "Visible=false sets HIDDEN"
        );
        // read_literal reports the inverse of HIDDEN.
        assert_eq!(
            BoundProp::Visible.read_literal(&n),
            serde_json::json!(false)
        );
        BoundProp::Visible.apply_resolved(&mut n, ResolvedVarValue::Boolean { value: true });
        assert!(!n.flags.contains(NodeFlags::HIDDEN));
    }

    #[test]
    fn fill_color_index_zero_changes_only_that_slot() {
        let mut n = rect();
        // Add a second fill so we can prove only index 0 changes.
        if let NodeData::Vector(v) = &mut n.data {
            v.fills.push(Fill::solid(Color::BLACK));
        }
        let red = Color::rgb(255, 0, 0);
        let wrote = BoundProp::FillColor { index: 0 }
            .apply_resolved(&mut n, ResolvedVarValue::Color { value: red });
        assert!(wrote);
        assert_eq!(node_fill_color(&n, 0), Some(red));
        assert_eq!(
            node_fill_color(&n, 1),
            Some(Color::BLACK),
            "fills[1] untouched"
        );
    }

    #[test]
    fn fill_color_can_target_frame_background_and_text_color() {
        let red = Color::rgb(255, 0, 0);

        let mut frame = CanvasNode::new(NodeData::Group(GroupNode {
            background: Some(Fill::solid(Color::WHITE)),
            ..Default::default()
        }));
        assert!(
            BoundProp::FillColor { index: 0 }
                .apply_resolved(&mut frame, ResolvedVarValue::Color { value: red })
        );
        assert_eq!(node_fill_color(&frame, 0), Some(red));

        let mut text =
            CanvasNode::new(NodeData::Text(crate::node::TextNode::new("hi", 10.0, 10.0)));
        if let NodeData::Text(text_node) = &mut text.data {
            let mut run_style = text_node.style.clone();
            run_style.color = Color::BLACK;
            text_node.style_runs.push(crate::node::TextStyleRun {
                start: 0,
                end: text_node.content.len(),
                style: run_style,
            });
        }
        assert!(
            BoundProp::FillColor { index: 0 }
                .apply_resolved(&mut text, ResolvedVarValue::Color { value: red })
        );
        assert_eq!(node_fill_color(&text, 0), Some(red));
        match &text.data {
            NodeData::Text(text_node) => assert!(
                text_node
                    .style_runs
                    .iter()
                    .all(|run| run.style.color == red),
                "text color binding must recolor rich text runs"
            ),
            _ => panic!("expected text"),
        }
    }

    #[test]
    fn fill_color_can_target_every_frame_background_slot() {
        let red = Color::rgb(255, 0, 0);
        let green = Color::rgb(0, 255, 0);
        let mut frame = CanvasNode::new(NodeData::Group(GroupNode {
            background: Some(Fill::solid(Color::WHITE)),
            background_fills: smallvec::smallvec![
                Fill::solid(Color::BLACK),
                Fill::solid(Color::rgb(0, 0, 255)),
            ],
            ..Default::default()
        }));

        assert!(BoundProp::FillColor { index: 1 }.applies_to(&frame));
        assert!(BoundProp::FillColor { index: 2 }.applies_to(&frame));
        assert!(!BoundProp::FillColor { index: 3 }.applies_to(&frame));
        assert!(
            BoundProp::FillColor { index: 1 }
                .apply_resolved(&mut frame, ResolvedVarValue::Color { value: red })
        );
        assert!(
            BoundProp::FillColor { index: 2 }
                .apply_resolved(&mut frame, ResolvedVarValue::Color { value: green })
        );
        assert_eq!(node_fill_color(&frame, 0), Some(Color::WHITE));
        assert_eq!(node_fill_color(&frame, 1), Some(red));
        assert_eq!(node_fill_color(&frame, 2), Some(green));
    }

    #[test]
    fn out_of_range_index_is_a_noop_not_a_panic() {
        let mut n = rect();
        let wrote = BoundProp::FillColor { index: 99 }.apply_resolved(
            &mut n,
            ResolvedVarValue::Color {
                value: Color::BLACK,
            },
        );
        assert!(!wrote);
    }

    #[test]
    fn type_mismatch_is_ignored() {
        let mut n = rect();
        // A string aimed at Opacity: must no-op.
        let wrote = BoundProp::Opacity.apply_resolved(
            &mut n,
            ResolvedVarValue::String {
                value: "nope".into(),
            },
        );
        assert!(!wrote);
        assert_eq!(n.opacity, UnitInterval::ONE);
    }

    #[test]
    fn read_literal_corner_radius_and_text() {
        let mut v = rect();
        if let NodeData::Vector(vn) = &mut v.data {
            vn.corner_radius = Some(8.0);
        }
        assert_eq!(
            BoundProp::CornerRadius.read_literal(&v),
            serde_json::json!(8.0)
        );

        let t = CanvasNode::new(NodeData::Text(crate::node::TextNode::new("hi", 10.0, 10.0)));
        assert_eq!(
            BoundProp::TextContent.read_literal(&t),
            serde_json::json!("hi")
        );
    }

    #[test]
    fn clip_size_write_targets_frame() {
        let mut frame = CanvasNode::new(NodeData::Group(GroupNode {
            clip_size: Some([100.0, 50.0]),
            background: None,
            explicit_modes: Default::default(),
            ..Default::default()
        }));
        BoundProp::ClipWidth.apply_resolved(&mut frame, ResolvedVarValue::Float { value: 200.0 });
        assert_eq!(clip_size(&frame), Some([200.0, 50.0]));
        assert_eq!(
            BoundProp::ClipHeight.read_literal(&frame),
            serde_json::json!(50.0)
        );
    }

    #[test]
    fn bound_prop_is_orderable_for_btreemap_keys() {
        use std::collections::BTreeMap;
        let mut m: BTreeMap<BoundProp, u8> = BTreeMap::new();
        m.insert(BoundProp::Opacity, 1);
        m.insert(BoundProp::FillColor { index: 0 }, 2);
        m.insert(BoundProp::FillColor { index: 1 }, 3);
        assert_eq!(m.len(), 3);
    }
}
