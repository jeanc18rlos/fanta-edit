//! Typed property reflection. Every property a node has is reachable under a
//! slash path (`opacity`, `fills/0/color`, `text/font_size`), with its kind,
//! its allowed enum values, and typed reads and writes. Writes join the
//! pending undo step like every other edit.

use std::sync::LazyLock;

use fanta_canvas::transform_angle;
use fanta_design_ops::DesignOp;
use fanta_doc::{
    BlendMode, Blur, BlurKind, BoundProp, CanvasNode, Color, Doc, Fill, ImageAdjust, ImageFitMode,
    LayoutMode, NodeData, NodeFlags, NodeId, Operation, Scope, ShadowKind, Stroke, StrokeAlign,
    StrokeCap, StrokeJoin, TextAlign, TextAutoResize, TextStyle, Transform2D, UnitInterval, VAlign,
};
use fanta_headless::edit_operations::{
    authored_local_bounds, blurs_operations, default_shadow, effects_operations,
    replace_data_operation, set_corner_radius,
};
use serde::{Serialize, de::DeserializeOwned};

use crate::{ApiError, DesignSession, Result};

/// A property's value.
#[derive(Debug, Clone, PartialEq)]
pub enum PropertyValue {
    Bool(bool),
    Int(i64),
    Float(f64),
    String(String),
    Color(Color),
    /// One of the names [`DesignSession::enum_values`] lists.
    Enum(String),
}

impl PropertyValue {
    pub fn kind(&self) -> PropertyKind {
        match self {
            Self::Bool(_) => PropertyKind::Bool,
            Self::Int(_) => PropertyKind::Int,
            Self::Float(_) => PropertyKind::Float,
            Self::String(_) => PropertyKind::String,
            Self::Color(_) => PropertyKind::Color,
            Self::Enum(_) => PropertyKind::Enum,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PropertyKind {
    Bool,
    Int,
    Float,
    String,
    Color,
    Enum,
}

/// One property a node has, as [`DesignSession::find_all_properties`] lists
/// it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PropertyInfo {
    pub path: String,
    pub kind: PropertyKind,
    pub writable: bool,
}

/// How a property write reaches the document.
enum Write {
    Operations(Vec<Operation>),
    DesignOp(DesignOp),
}

type Getter = fn(&Doc, &CanvasNode, usize) -> Option<PropertyValue>;
type Setter = fn(&Doc, &CanvasNode, usize, &PropertyValue) -> Result<Write>;

struct PropertyDef {
    /// The path, with `#` standing for a list index.
    pattern: &'static str,
    kind: PropertyKind,
    /// The scope a write needs; `None` for what no template locks, such as
    /// the name.
    scope: Option<Scope>,
    /// How many entries of the property the node has; 0 when it has none.
    count: fn(&CanvasNode) -> usize,
    get: Getter,
    set: Option<Setter>,
    /// The variable binding that drives the value. A literal write would
    /// fight it, so writes are refused while it is bound.
    bound: Option<fn(usize) -> BoundProp>,
    /// The names an enum property accepts.
    values: fn() -> Vec<String>,
}

const BLEND_MODES: [BlendMode; 18] = [
    BlendMode::Normal,
    BlendMode::Multiply,
    BlendMode::Screen,
    BlendMode::Overlay,
    BlendMode::Darken,
    BlendMode::Lighten,
    BlendMode::ColorDodge,
    BlendMode::ColorBurn,
    BlendMode::HardLight,
    BlendMode::SoftLight,
    BlendMode::Difference,
    BlendMode::Exclusion,
    BlendMode::Hue,
    BlendMode::Saturation,
    BlendMode::Color,
    BlendMode::Luminosity,
    BlendMode::LinearBurn,
    BlendMode::LinearDodge,
];
const FIT_MODES: [ImageFitMode; 4] = [
    ImageFitMode::Fill,
    ImageFitMode::Fit,
    ImageFitMode::Stretch,
    ImageFitMode::Tile,
];
const STROKE_ALIGNS: [StrokeAlign; 3] = [
    StrokeAlign::Center,
    StrokeAlign::Inside,
    StrokeAlign::Outside,
];
const STROKE_CAPS: [StrokeCap; 3] = [StrokeCap::Butt, StrokeCap::Round, StrokeCap::Square];
const STROKE_JOINS: [StrokeJoin; 3] = [StrokeJoin::Miter, StrokeJoin::Round, StrokeJoin::Bevel];
const SHADOW_KINDS: [ShadowKind; 2] = [ShadowKind::Drop, ShadowKind::Inner];
const BLUR_KINDS: [BlurKind; 2] = [BlurKind::Layer, BlurKind::Background];
const TEXT_ALIGNS: [TextAlign; 4] = [
    TextAlign::Left,
    TextAlign::Center,
    TextAlign::Right,
    TextAlign::Justify,
];
const VERTICAL_ALIGNS: [VAlign; 3] = [VAlign::Top, VAlign::Center, VAlign::Bottom];
const AUTO_RESIZES: [TextAutoResize; 3] = [
    TextAutoResize::None,
    TextAutoResize::WidthAndHeight,
    TextAutoResize::Height,
];
const LAYOUT_MODES: [LayoutMode; 3] = [
    LayoutMode::Horizontal,
    LayoutMode::Vertical,
    LayoutMode::Grid,
];
const FILL_TYPES: [&str; 6] = ["solid", "gradient", "image", "video", "pattern", "shader"];

static PROPERTIES: LazyLock<Vec<PropertyDef>> = LazyLock::new(property_table);

impl DesignSession {
    /// Every property `node` has, with its concrete list indices.
    pub fn find_all_properties(&self, node: NodeId) -> Result<Vec<PropertyInfo>> {
        let canvas_node = self.node(node)?;
        let doc = self.doc();
        let mut properties = Vec::new();
        for def in PROPERTIES.iter() {
            for index in 0..(def.count)(canvas_node) {
                if (def.get)(doc, canvas_node, index).is_some() {
                    properties.push(PropertyInfo {
                        path: def.pattern.replacen('#', &index.to_string(), 1),
                        kind: def.kind,
                        writable: def.set.is_some(),
                    });
                }
            }
        }
        Ok(properties)
    }

    pub fn property_kind(&self, node: NodeId, path: &str) -> Result<PropertyKind> {
        let (def, _) = self.resolve(node, path)?;
        Ok(def.kind)
    }

    /// The names an enum property accepts.
    pub fn enum_values(&self, node: NodeId, path: &str) -> Result<Vec<String>> {
        let (def, _) = self.resolve(node, path)?;
        if def.kind != PropertyKind::Enum {
            return Err(ApiError::PropertyKind {
                path: path.to_owned(),
                expected: PropertyKind::Enum,
                found: def.kind,
            });
        }
        Ok((def.values)())
    }

    pub fn property(&self, node: NodeId, path: &str) -> Result<PropertyValue> {
        let (def, index) = self.resolve(node, path)?;
        (def.get)(self.doc(), self.node(node)?, index).ok_or_else(|| ApiError::UnknownProperty {
            node,
            path: path.to_owned(),
        })
    }

    /// Write a property. The value's kind must match the property's.
    pub fn set_property(&mut self, node: NodeId, path: &str, value: PropertyValue) -> Result<()> {
        let (def, index) = self.resolve(node, path)?;
        if let Some(scope) = def.scope {
            self.permit(Some(node), scope)?;
        }
        if value.kind() != def.kind {
            return Err(ApiError::PropertyKind {
                path: path.to_owned(),
                expected: def.kind,
                found: value.kind(),
            });
        }
        let Some(set) = def.set else {
            return Err(ApiError::ReadOnly(path.to_owned()));
        };
        let canvas_node = self.node(node)?;
        if let Some(bound) = def.bound
            && canvas_node.bindings.contains_key(&bound(index))
        {
            return Err(ApiError::PropertyBound(path.to_owned()));
        }
        if let PropertyValue::Enum(name) = &value
            && !(def.values)().contains(name)
        {
            return Err(ApiError::InvalidEnumValue {
                path: path.to_owned(),
                value: name.clone(),
            });
        }
        match set(self.doc(), canvas_node, index, &value)? {
            Write::Operations(operations) => self.edit(operations),
            Write::DesignOp(operation) => self.edit_with_design_op(&operation).map(|_| ()),
        }
    }

    pub fn get_bool(&self, node: NodeId, path: &str) -> Result<bool> {
        match self.property(node, path)? {
            PropertyValue::Bool(value) => Ok(value),
            other => Err(kind_mismatch(path, PropertyKind::Bool, &other)),
        }
    }

    pub fn get_int(&self, node: NodeId, path: &str) -> Result<i64> {
        match self.property(node, path)? {
            PropertyValue::Int(value) => Ok(value),
            other => Err(kind_mismatch(path, PropertyKind::Int, &other)),
        }
    }

    pub fn get_float(&self, node: NodeId, path: &str) -> Result<f64> {
        match self.property(node, path)? {
            PropertyValue::Float(value) => Ok(value),
            other => Err(kind_mismatch(path, PropertyKind::Float, &other)),
        }
    }

    pub fn get_string(&self, node: NodeId, path: &str) -> Result<String> {
        match self.property(node, path)? {
            PropertyValue::String(value) => Ok(value),
            other => Err(kind_mismatch(path, PropertyKind::String, &other)),
        }
    }

    pub fn get_color(&self, node: NodeId, path: &str) -> Result<Color> {
        match self.property(node, path)? {
            PropertyValue::Color(value) => Ok(value),
            other => Err(kind_mismatch(path, PropertyKind::Color, &other)),
        }
    }

    pub fn get_enum(&self, node: NodeId, path: &str) -> Result<String> {
        match self.property(node, path)? {
            PropertyValue::Enum(value) => Ok(value),
            other => Err(kind_mismatch(path, PropertyKind::Enum, &other)),
        }
    }

    pub fn set_bool(&mut self, node: NodeId, path: &str, value: bool) -> Result<()> {
        self.set_property(node, path, PropertyValue::Bool(value))
    }

    pub fn set_int(&mut self, node: NodeId, path: &str, value: i64) -> Result<()> {
        self.set_property(node, path, PropertyValue::Int(value))
    }

    pub fn set_float(&mut self, node: NodeId, path: &str, value: f64) -> Result<()> {
        self.set_property(node, path, PropertyValue::Float(value))
    }

    pub fn set_string(&mut self, node: NodeId, path: &str, value: &str) -> Result<()> {
        self.set_property(node, path, PropertyValue::String(value.to_owned()))
    }

    pub fn set_color(&mut self, node: NodeId, path: &str, value: Color) -> Result<()> {
        self.set_property(node, path, PropertyValue::Color(value))
    }

    pub fn set_enum(&mut self, node: NodeId, path: &str, value: &str) -> Result<()> {
        self.set_property(node, path, PropertyValue::Enum(value.to_owned()))
    }

    /// Add a solid fill on top of `node`'s fills and return its index.
    pub fn add_fill(&mut self, node: NodeId, color: Color) -> Result<usize> {
        self.permit(Some(node), Scope::FillChange)?;
        let canvas_node = self.node(node)?;
        if !has_fills(canvas_node) {
            return Err(ApiError::InvalidArgument(format!("{node} has no fills")));
        }
        let index = fill_count(canvas_node);
        let paint = Fill::Solid {
            color,
            blend: BlendMode::Normal,
        };
        let operations = replace_data_operation(self.doc(), node, |data| match data {
            NodeData::Vector(vector) => vector.fills.push(paint),
            NodeData::Boolean(boolean) => boolean.fills.push(paint),
            NodeData::Group(group) if group.background.is_none() => group.background = Some(paint),
            NodeData::Group(group) => group.background_fills.push(paint),
            _ => {}
        });
        self.edit(operations)?;
        Ok(index)
    }

    pub fn remove_fill(&mut self, node: NodeId, index: usize) -> Result<()> {
        self.permit(Some(node), Scope::FillChange)?;
        let canvas_node = self.node(node)?;
        if index >= fill_count(canvas_node) {
            return Err(ApiError::InvalidArgument(format!(
                "{node} has no fill {index}"
            )));
        }
        let operations = replace_data_operation(self.doc(), node, |data| match data {
            NodeData::Vector(vector) => {
                vector.fills.remove(index);
            }
            NodeData::Boolean(boolean) => {
                boolean.fills.remove(index);
            }
            NodeData::Group(group) => match (group.background.is_some(), index) {
                (true, 0) => {
                    group.background = (!group.background_fills.is_empty())
                        .then(|| group.background_fills.remove(0));
                }
                (true, index) => {
                    group.background_fills.remove(index - 1);
                }
                (false, index) => {
                    group.background_fills.remove(index);
                }
            },
            _ => {}
        });
        self.edit(operations)
    }

    /// Add a solid stroke on top of `node`'s strokes and return its index.
    pub fn add_stroke(&mut self, node: NodeId, color: Color, width: f64) -> Result<usize> {
        self.permit(Some(node), Scope::StrokeChange)?;
        if !(width.is_finite() && width > 0.0) {
            return Err(ApiError::InvalidArgument(
                "a stroke width must be positive".into(),
            ));
        }
        let canvas_node = self.node(node)?;
        let Some(index) = canvas_node.data.strokes().map(<[Stroke]>::len) else {
            return Err(ApiError::InvalidArgument(format!("{node} has no strokes")));
        };
        let operations = replace_data_operation(self.doc(), node, |data| {
            if let Some(strokes) = data.strokes_mut() {
                strokes.push(Stroke::solid(color, width));
            }
        });
        self.edit(operations)?;
        Ok(index)
    }

    pub fn remove_stroke(&mut self, node: NodeId, index: usize) -> Result<()> {
        self.permit(Some(node), Scope::StrokeChange)?;
        let canvas_node = self.node(node)?;
        if index >= stroke_count(canvas_node) {
            return Err(ApiError::InvalidArgument(format!(
                "{node} has no stroke {index}"
            )));
        }
        let operations = replace_data_operation(self.doc(), node, |data| {
            if let Some(strokes) = data.strokes_mut() {
                strokes.remove(index);
            }
        });
        self.edit(operations)
    }

    /// Add a shadow with the editor's defaults and return its index.
    pub fn add_shadow(&mut self, node: NodeId, kind: ShadowKind) -> Result<usize> {
        self.permit(Some(node), Scope::AppearanceShadow)?;
        let index = self.node(node)?.effects.len();
        let operations = effects_operations(self.doc(), node, |effects| {
            effects.push(fanta_doc::Shadow {
                kind,
                ..default_shadow()
            });
        });
        self.edit(operations)?;
        Ok(index)
    }

    pub fn remove_shadow(&mut self, node: NodeId, index: usize) -> Result<()> {
        self.permit(Some(node), Scope::AppearanceShadow)?;
        if index >= self.node(node)?.effects.len() {
            return Err(ApiError::InvalidArgument(format!(
                "{node} has no shadow {index}"
            )));
        }
        let operations = effects_operations(self.doc(), node, |effects| {
            effects.remove(index);
        });
        self.edit(operations)
    }

    pub fn add_blur(&mut self, node: NodeId, kind: BlurKind, radius: f64) -> Result<usize> {
        self.permit(Some(node), Scope::AppearanceBlur)?;
        if !(radius.is_finite() && radius >= 0.0) {
            return Err(ApiError::InvalidArgument(
                "a blur radius must be zero or positive".into(),
            ));
        }
        let index = self.node(node)?.blurs.len();
        let operations =
            blurs_operations(self.doc(), node, |blurs| blurs.push(Blur { kind, radius }));
        self.edit(operations)?;
        Ok(index)
    }

    pub fn remove_blur(&mut self, node: NodeId, index: usize) -> Result<()> {
        self.permit(Some(node), Scope::AppearanceBlur)?;
        if index >= self.node(node)?.blurs.len() {
            return Err(ApiError::InvalidArgument(format!(
                "{node} has no blur {index}"
            )));
        }
        let operations = blurs_operations(self.doc(), node, |blurs| {
            blurs.remove(index);
        });
        self.edit(operations)
    }

    /// Mirror `node` across its centre, horizontally or vertically.
    pub fn flip(&mut self, node: NodeId, horizontally: bool) -> Result<()> {
        self.permit(Some(node), Scope::LayerFlip)?;
        let canvas_node = self.node(node)?;
        let doc = self.doc();
        let center = doc
            .scene
            .world_bounds(node)
            .ok_or_else(|| ApiError::InvalidArgument(format!("{node} has no bounds")))?
            .center();
        let world = doc
            .scene
            .world_transform(node)
            .ok_or(ApiError::NodeNotFound(node))?;
        let parent_world = canvas_node
            .parent
            .and_then(|parent| doc.scene.world_transform(parent))
            .unwrap_or(Transform2D::IDENTITY);
        let (scale_x, scale_y) = if horizontally {
            (-1.0, 1.0)
        } else {
            (1.0, -1.0)
        };
        let mirrored = world
            .then(&Transform2D::translation(-center.x, -center.y))
            .then(&Transform2D::scale_xy(scale_x, scale_y))
            .then(&Transform2D::translation(center.x, center.y));
        let new = mirrored.then(&parent_world.inverse());
        if !new.is_finite() {
            return Err(ApiError::InvalidArgument(format!(
                "{node}'s parent cannot be inverted"
            )));
        }
        let old = canvas_node.transform;
        self.edit(vec![Operation::SetTransform { id: node, old, new }])
    }

    /// The property definition `path` names on `node`, and its list index.
    fn resolve(&self, node: NodeId, path: &str) -> Result<(&'static PropertyDef, usize)> {
        let canvas_node = self.node(node)?;
        let unknown = || ApiError::UnknownProperty {
            node,
            path: path.to_owned(),
        };
        for def in PROPERTIES.iter() {
            if let Some(index) = match_pattern(def.pattern, path) {
                if index >= (def.count)(canvas_node) {
                    return Err(unknown());
                }
                return Ok((def, index));
            }
        }
        Err(unknown())
    }
}

/// The list index `path` takes in `pattern`, or `None` when it is another
/// property. Paths without a `#` match with index 0.
fn match_pattern(pattern: &str, path: &str) -> Option<usize> {
    let mut index = 0;
    let mut pattern_segments = pattern.split('/');
    let mut path_segments = path.split('/');
    loop {
        match (pattern_segments.next(), path_segments.next()) {
            (None, None) => return Some(index),
            (Some("#"), Some(segment)) => {
                if segment.is_empty() || !segment.bytes().all(|byte| byte.is_ascii_digit()) {
                    return None;
                }
                index = segment.parse().ok()?;
            }
            (Some(expected), Some(segment)) if expected == segment => {}
            _ => return None,
        }
    }
}

fn kind_mismatch(path: &str, expected: PropertyKind, found: &PropertyValue) -> ApiError {
    ApiError::PropertyKind {
        path: path.to_owned(),
        expected,
        found: found.kind(),
    }
}

/// The serde name a value has in project files, such as `"linear_burn"`.
fn enum_name<T: Serialize>(value: &T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_default()
}

fn enum_names<T: Serialize>(values: &[T]) -> Vec<String> {
    values.iter().map(enum_name).collect()
}

fn parse_enum<T: DeserializeOwned>(value: &PropertyValue) -> Result<T> {
    let PropertyValue::Enum(name) = value else {
        return Err(ApiError::InvalidArgument("expected an enum value".into()));
    };
    serde_json::from_value(serde_json::Value::String(name.clone())).map_err(|_| {
        ApiError::InvalidArgument(format!("`{name}` is not one of the property's values"))
    })
}

fn float(value: &PropertyValue) -> Result<f64> {
    match value {
        PropertyValue::Float(value) if value.is_finite() => Ok(*value),
        PropertyValue::Float(_) => {
            Err(ApiError::InvalidArgument("the value must be finite".into()))
        }
        _ => Err(ApiError::InvalidArgument("expected a float value".into())),
    }
}

fn color(value: &PropertyValue) -> Result<Color> {
    match value {
        PropertyValue::Color(color) => Ok(*color),
        _ => Err(ApiError::InvalidArgument("expected a color value".into())),
    }
}

fn string(value: &PropertyValue) -> Result<&str> {
    match value {
        PropertyValue::String(value) => Ok(value),
        _ => Err(ApiError::InvalidArgument("expected a string value".into())),
    }
}

fn boolean(value: &PropertyValue) -> Result<bool> {
    match value {
        PropertyValue::Bool(value) => Ok(*value),
        _ => Err(ApiError::InvalidArgument("expected a bool value".into())),
    }
}

fn one(_: &CanvasNode) -> usize {
    1
}

fn no_values() -> Vec<String> {
    Vec::new()
}

fn has_fills(node: &CanvasNode) -> bool {
    matches!(
        node.data,
        NodeData::Vector(_) | NodeData::Boolean(_) | NodeData::Group(_)
    )
}

/// A node's fills in stacking order. A frame's background is fill 0, as
/// variable bindings count it.
pub(crate) fn fills(data: &NodeData) -> Vec<&Fill> {
    match data {
        NodeData::Vector(vector) => vector.fills.iter().collect(),
        NodeData::Boolean(boolean) => boolean.fills.iter().collect(),
        NodeData::Group(group) => group
            .background
            .iter()
            .chain(group.background_fills.iter())
            .collect(),
        _ => Vec::new(),
    }
}

pub(crate) fn fill_mut(data: &mut NodeData, index: usize) -> Option<&mut Fill> {
    match data {
        NodeData::Vector(vector) => vector.fills.get_mut(index),
        NodeData::Boolean(boolean) => boolean.fills.get_mut(index),
        NodeData::Group(group) => match (group.background.is_some(), index) {
            (true, 0) => group.background.as_mut(),
            (true, index) => group.background_fills.get_mut(index - 1),
            (false, index) => group.background_fills.get_mut(index),
        },
        _ => None,
    }
}

fn fill_count(node: &CanvasNode) -> usize {
    fills(&node.data).len()
}

fn fill_at(node: &CanvasNode, index: usize) -> Option<&Fill> {
    fills(&node.data).get(index).copied()
}

fn image_adjust(node: &CanvasNode, index: usize) -> Option<&ImageAdjust> {
    match fill_at(node, index)? {
        Fill::Image { adjust, .. } => Some(adjust),
        _ => None,
    }
}

fn stroke_count(node: &CanvasNode) -> usize {
    node.data.strokes().map_or(0, <[Stroke]>::len)
}

fn stroke_at(node: &CanvasNode, index: usize) -> Option<&Stroke> {
    node.data.strokes()?.get(index)
}

fn text_style(node: &CanvasNode) -> Option<&TextStyle> {
    match &node.data {
        NodeData::Text(text) => Some(&text.style),
        _ => None,
    }
}

fn text_count(node: &CanvasNode) -> usize {
    usize::from(matches!(node.data, NodeData::Text(_)))
}

/// Write `change` into a text node's base style and every rich-text run, as
/// the agent's `set_text_style` does.
fn text_style_write(doc: &Doc, node: &CanvasNode, change: impl Fn(&mut TextStyle)) -> Write {
    Write::Operations(replace_data_operation(doc, node.id, |data| {
        if let NodeData::Text(text) = data {
            change(&mut text.style);
            for run in &mut text.style_runs {
                change(&mut run.style);
            }
        }
    }))
}

fn auto_layout_count(node: &CanvasNode) -> usize {
    usize::from(matches!(&node.data, NodeData::Group(group) if group.auto_layout.is_some()))
}

/// The node's box in world units with its rotation taken out: the width and
/// height its resize handles change.
fn oriented_size(doc: &Doc, node: &CanvasNode) -> Option<[f64; 2]> {
    let local = authored_local_bounds(&doc.scene, node.id)?;
    let world = doc.scene.world_transform(node.id)?;
    let unrotated = world.then(&Transform2D::rotation(-transform_angle(&world)));
    let bounds = local.transformed(&unrotated);
    Some([bounds.width(), bounds.height()])
}

fn set_props(id: NodeId) -> DesignOp {
    DesignOp::SetProps {
        id: id.to_string(),
        name: None,
        x: None,
        y: None,
        width: None,
        height: None,
        opacity: None,
        fill: None,
        corner_radius: None,
        text: None,
        hidden: None,
        locked: None,
    }
}

fn flag_write(node: &CanvasNode, flag: NodeFlags, on: bool) -> Write {
    let old = node.flags;
    let mut new = old;
    new.set(flag, on);
    Write::Operations(if new == old {
        Vec::new()
    } else {
        vec![Operation::SetFlags {
            id: node.id,
            old,
            new,
        }]
    })
}

fn property_table() -> Vec<PropertyDef> {
    let mut table = vec![
        PropertyDef {
            pattern: "name",
            kind: PropertyKind::String,
            scope: None,
            count: one,
            get: |_, node, _| Some(PropertyValue::String(node.name.clone())),
            set: Some(|_, node, _, value| {
                let new = string(value)?.to_owned();
                Ok(Write::Operations(if new == node.name {
                    Vec::new()
                } else {
                    vec![Operation::SetName {
                        id: node.id,
                        old: node.name.clone(),
                        new,
                    }]
                }))
            }),
            bound: None,
            values: no_values,
        },
        PropertyDef {
            pattern: "visible",
            kind: PropertyKind::Bool,
            scope: Some(Scope::LayerVisibility),
            count: one,
            get: |_, node, _| Some(PropertyValue::Bool(!node.flags.contains(NodeFlags::HIDDEN))),
            set: Some(|_, node, _, value| {
                Ok(flag_write(node, NodeFlags::HIDDEN, !boolean(value)?))
            }),
            bound: Some(|_| BoundProp::Visible),
            values: no_values,
        },
        PropertyDef {
            pattern: "locked",
            kind: PropertyKind::Bool,
            scope: None,
            count: one,
            get: |_, node, _| Some(PropertyValue::Bool(node.flags.contains(NodeFlags::LOCKED))),
            set: Some(|_, node, _, value| Ok(flag_write(node, NodeFlags::LOCKED, boolean(value)?))),
            bound: None,
            values: no_values,
        },
        PropertyDef {
            pattern: "opacity",
            kind: PropertyKind::Float,
            scope: Some(Scope::LayerOpacity),
            count: one,
            get: |_, node, _| Some(PropertyValue::Float(f64::from(node.opacity.get()))),
            set: Some(|_, node, _, value| {
                let new = UnitInterval::new(float(value)? as f32);
                Ok(Write::Operations(if new == node.opacity {
                    Vec::new()
                } else {
                    vec![Operation::SetOpacity {
                        id: node.id,
                        old: node.opacity,
                        new,
                    }]
                }))
            }),
            bound: Some(|_| BoundProp::Opacity),
            values: no_values,
        },
        PropertyDef {
            pattern: "blend_mode",
            kind: PropertyKind::Enum,
            scope: Some(Scope::LayerBlendMode),
            count: one,
            get: |_, node, _| Some(PropertyValue::Enum(enum_name(&node.blend_mode))),
            set: Some(|_, node, _, value| {
                let new: BlendMode = parse_enum(value)?;
                Ok(Write::Operations(if new == node.blend_mode {
                    Vec::new()
                } else {
                    vec![Operation::SetBlendMode {
                        id: node.id,
                        old: node.blend_mode,
                        new,
                    }]
                }))
            }),
            bound: None,
            values: || enum_names(&BLEND_MODES),
        },
        PropertyDef {
            pattern: "position/x",
            kind: PropertyKind::Float,
            scope: Some(Scope::LayerMove),
            count: one,
            get: |doc, node, _| {
                doc.scene
                    .world_bounds(node.id)
                    .map(|bounds| PropertyValue::Float(bounds.min_x))
            },
            set: Some(|_, node, _, value| {
                let mut operation = set_props(node.id);
                if let DesignOp::SetProps { x, .. } = &mut operation {
                    *x = Some(float(value)?);
                }
                Ok(Write::DesignOp(operation))
            }),
            bound: None,
            values: no_values,
        },
        PropertyDef {
            pattern: "position/y",
            kind: PropertyKind::Float,
            scope: Some(Scope::LayerMove),
            count: one,
            get: |doc, node, _| {
                doc.scene
                    .world_bounds(node.id)
                    .map(|bounds| PropertyValue::Float(bounds.min_y))
            },
            set: Some(|_, node, _, value| {
                let mut operation = set_props(node.id);
                if let DesignOp::SetProps { y, .. } = &mut operation {
                    *y = Some(float(value)?);
                }
                Ok(Write::DesignOp(operation))
            }),
            bound: None,
            values: no_values,
        },
        PropertyDef {
            pattern: "size/width",
            kind: PropertyKind::Float,
            scope: Some(Scope::LayerResize),
            count: one,
            get: |doc, node, _| {
                oriented_size(doc, node).map(|[width, _]| PropertyValue::Float(width))
            },
            set: Some(|_, node, _, value| {
                let mut operation = set_props(node.id);
                if let DesignOp::SetProps { width, .. } = &mut operation {
                    *width = Some(float(value)?);
                }
                Ok(Write::DesignOp(operation))
            }),
            bound: Some(|_| BoundProp::ClipWidth),
            values: no_values,
        },
        PropertyDef {
            pattern: "size/height",
            kind: PropertyKind::Float,
            scope: Some(Scope::LayerResize),
            count: one,
            get: |doc, node, _| {
                oriented_size(doc, node).map(|[_, height]| PropertyValue::Float(height))
            },
            set: Some(|_, node, _, value| {
                let mut operation = set_props(node.id);
                if let DesignOp::SetProps { height, .. } = &mut operation {
                    *height = Some(float(value)?);
                }
                Ok(Write::DesignOp(operation))
            }),
            bound: Some(|_| BoundProp::ClipHeight),
            values: no_values,
        },
        PropertyDef {
            pattern: "rotation",
            kind: PropertyKind::Float,
            scope: Some(Scope::LayerRotate),
            count: one,
            get: |doc, node, _| {
                doc.scene
                    .world_transform(node.id)
                    .map(|world| PropertyValue::Float(transform_angle(&world).to_degrees()))
            },
            set: Some(|_, node, _, value| {
                Ok(Write::DesignOp(DesignOp::Rotate {
                    id: node.id.to_string(),
                    degrees: float(value)?,
                }))
            }),
            bound: None,
            values: no_values,
        },
        PropertyDef {
            pattern: "corner_radius",
            kind: PropertyKind::Float,
            scope: Some(Scope::ShapeChange),
            count: |node| {
                usize::from(matches!(
                    node.data,
                    NodeData::Vector(_) | NodeData::Group(_)
                ))
            },
            get: |_, node, _| match &node.data {
                NodeData::Vector(vector) => {
                    Some(PropertyValue::Float(vector.corner_radius.unwrap_or(0.0)))
                }
                NodeData::Group(group) => {
                    Some(PropertyValue::Float(group.corner_radius.unwrap_or(0.0)))
                }
                _ => None,
            },
            set: Some(|doc, node, _, value| {
                let radius = float(value)?;
                if radius < 0.0 {
                    return Err(ApiError::InvalidArgument(
                        "a corner radius cannot be negative".into(),
                    ));
                }
                Ok(Write::Operations(replace_data_operation(
                    doc,
                    node.id,
                    |data| set_corner_radius(data, radius),
                )))
            }),
            bound: Some(|_| BoundProp::CornerRadius),
            values: no_values,
        },
        PropertyDef {
            pattern: "fills/#/type",
            kind: PropertyKind::Enum,
            scope: Some(Scope::FillChangeType),
            count: fill_count,
            get: |_, node, index| {
                let name = match fill_at(node, index)? {
                    Fill::Solid { .. } => "solid",
                    Fill::Gradient { .. } => "gradient",
                    Fill::Image { .. } => "image",
                    Fill::Video { .. } => "video",
                    Fill::Pattern { .. } => "pattern",
                    Fill::Shader { .. } => "shader",
                };
                Some(PropertyValue::Enum(name.to_owned()))
            },
            set: None,
            bound: None,
            values: || FILL_TYPES.iter().map(|name| (*name).to_owned()).collect(),
        },
        PropertyDef {
            pattern: "fills/#/color",
            kind: PropertyKind::Color,
            scope: Some(Scope::FillChange),
            count: fill_count,
            get: |_, node, index| match fill_at(node, index)? {
                Fill::Solid { color, .. } => Some(PropertyValue::Color(*color)),
                _ => None,
            },
            set: Some(|doc, node, index, value| {
                let new = color(value)?;
                if !matches!(fill_at(node, index), Some(Fill::Solid { .. })) {
                    return Err(ApiError::InvalidArgument(format!(
                        "fill {index} is not a solid color"
                    )));
                }
                Ok(Write::Operations(replace_data_operation(
                    doc,
                    node.id,
                    |data| {
                        if let Some(Fill::Solid { color, .. }) = fill_mut(data, index) {
                            *color = new;
                        }
                    },
                )))
            }),
            bound: Some(|index| BoundProp::FillColor {
                index: u16::try_from(index).unwrap_or(u16::MAX),
            }),
            values: no_values,
        },
        PropertyDef {
            pattern: "fills/#/blend_mode",
            kind: PropertyKind::Enum,
            scope: Some(Scope::FillChange),
            count: fill_count,
            get: |_, node, index| {
                let blend = match fill_at(node, index)? {
                    Fill::Solid { blend, .. }
                    | Fill::Gradient { blend, .. }
                    | Fill::Image { blend, .. }
                    | Fill::Video { blend, .. }
                    | Fill::Pattern { blend, .. }
                    | Fill::Shader { blend, .. } => blend,
                };
                Some(PropertyValue::Enum(enum_name(blend)))
            },
            set: Some(|doc, node, index, value| {
                let new: BlendMode = parse_enum(value)?;
                Ok(Write::Operations(replace_data_operation(
                    doc,
                    node.id,
                    |data| {
                        if let Some(
                            Fill::Solid { blend, .. }
                            | Fill::Gradient { blend, .. }
                            | Fill::Image { blend, .. }
                            | Fill::Video { blend, .. }
                            | Fill::Pattern { blend, .. }
                            | Fill::Shader { blend, .. },
                        ) = fill_mut(data, index)
                        {
                            *blend = new;
                        }
                    },
                )))
            }),
            bound: None,
            values: || enum_names(&BLEND_MODES),
        },
        PropertyDef {
            pattern: "fills/#/image/fit",
            kind: PropertyKind::Enum,
            scope: Some(Scope::LayerCrop),
            count: fill_count,
            get: |_, node, index| match fill_at(node, index)? {
                Fill::Image { mode, .. } => Some(PropertyValue::Enum(enum_name(mode))),
                _ => None,
            },
            set: Some(|doc, node, index, value| {
                let new: ImageFitMode = parse_enum(value)?;
                Ok(Write::Operations(replace_data_operation(
                    doc,
                    node.id,
                    |data| {
                        if let Some(Fill::Image { mode, .. }) = fill_mut(data, index) {
                            *mode = new;
                        }
                    },
                )))
            }),
            bound: None,
            values: || enum_names(&FIT_MODES),
        },
        PropertyDef {
            pattern: "image/fit",
            kind: PropertyKind::Enum,
            scope: Some(Scope::LayerCrop),
            count: |node| usize::from(matches!(node.data, NodeData::Bitmap(_))),
            get: |_, node, _| match &node.data {
                NodeData::Bitmap(bitmap) => Some(PropertyValue::Enum(enum_name(&bitmap.fit))),
                _ => None,
            },
            set: Some(|doc, node, _, value| {
                let new: ImageFitMode = parse_enum(value)?;
                Ok(Write::Operations(replace_data_operation(
                    doc,
                    node.id,
                    |data| {
                        if let NodeData::Bitmap(bitmap) = data {
                            bitmap.fit = new;
                        }
                    },
                )))
            }),
            bound: None,
            values: || enum_names(&FIT_MODES),
        },
        PropertyDef {
            pattern: "strokes/#/color",
            kind: PropertyKind::Color,
            scope: Some(Scope::StrokeChange),
            count: stroke_count,
            get: |_, node, index| match &stroke_at(node, index)?.paint {
                Fill::Solid { color, .. } => Some(PropertyValue::Color(*color)),
                _ => None,
            },
            set: Some(|doc, node, index, value| {
                let new = color(value)?;
                if !matches!(
                    stroke_at(node, index).map(|stroke| &stroke.paint),
                    Some(Fill::Solid { .. })
                ) {
                    return Err(ApiError::InvalidArgument(format!(
                        "stroke {index} is not a solid color"
                    )));
                }
                Ok(Write::Operations(replace_data_operation(
                    doc,
                    node.id,
                    |data| {
                        if let Some(Fill::Solid { color, .. }) = data
                            .strokes_mut()
                            .and_then(|strokes| strokes.get_mut(index))
                            .map(|stroke| &mut stroke.paint)
                        {
                            *color = new;
                        }
                    },
                )))
            }),
            bound: Some(|index| BoundProp::StrokeColor {
                index: u16::try_from(index).unwrap_or(u16::MAX),
            }),
            values: no_values,
        },
        PropertyDef {
            pattern: "strokes/#/width",
            kind: PropertyKind::Float,
            scope: Some(Scope::StrokeChange),
            count: stroke_count,
            get: |_, node, index| Some(PropertyValue::Float(stroke_at(node, index)?.width)),
            set: Some(|doc, node, index, value| {
                let width = float(value)?;
                if width < 0.0 {
                    return Err(ApiError::InvalidArgument(
                        "a stroke width cannot be negative".into(),
                    ));
                }
                Ok(Write::Operations(replace_data_operation(
                    doc,
                    node.id,
                    |data| {
                        if let Some(stroke) = data
                            .strokes_mut()
                            .and_then(|strokes| strokes.get_mut(index))
                        {
                            stroke.width = width;
                        }
                    },
                )))
            }),
            bound: Some(|index| BoundProp::StrokeWidth {
                index: u16::try_from(index).unwrap_or(u16::MAX),
            }),
            values: no_values,
        },
        PropertyDef {
            pattern: "strokes/#/align",
            kind: PropertyKind::Enum,
            scope: Some(Scope::StrokeChange),
            count: stroke_count,
            get: |_, node, index| {
                Some(PropertyValue::Enum(enum_name(
                    &stroke_at(node, index)?.align,
                )))
            },
            set: Some(|doc, node, index, value| {
                let new: StrokeAlign = parse_enum(value)?;
                Ok(Write::Operations(replace_data_operation(
                    doc,
                    node.id,
                    |data| {
                        if let Some(stroke) = data
                            .strokes_mut()
                            .and_then(|strokes| strokes.get_mut(index))
                        {
                            stroke.align = new;
                        }
                    },
                )))
            }),
            bound: None,
            values: || enum_names(&STROKE_ALIGNS),
        },
        PropertyDef {
            pattern: "strokes/#/cap",
            kind: PropertyKind::Enum,
            scope: Some(Scope::StrokeChange),
            count: stroke_count,
            get: |_, node, index| {
                Some(PropertyValue::Enum(enum_name(&stroke_at(node, index)?.cap)))
            },
            set: Some(|doc, node, index, value| {
                let new: StrokeCap = parse_enum(value)?;
                Ok(Write::Operations(replace_data_operation(
                    doc,
                    node.id,
                    |data| {
                        if let Some(stroke) = data
                            .strokes_mut()
                            .and_then(|strokes| strokes.get_mut(index))
                        {
                            stroke.cap = new;
                        }
                    },
                )))
            }),
            bound: None,
            values: || enum_names(&STROKE_CAPS),
        },
        PropertyDef {
            pattern: "strokes/#/join",
            kind: PropertyKind::Enum,
            scope: Some(Scope::StrokeChange),
            count: stroke_count,
            get: |_, node, index| {
                Some(PropertyValue::Enum(enum_name(
                    &stroke_at(node, index)?.join,
                )))
            },
            set: Some(|doc, node, index, value| {
                let new: StrokeJoin = parse_enum(value)?;
                Ok(Write::Operations(replace_data_operation(
                    doc,
                    node.id,
                    |data| {
                        if let Some(stroke) = data
                            .strokes_mut()
                            .and_then(|strokes| strokes.get_mut(index))
                        {
                            stroke.join = new;
                        }
                    },
                )))
            }),
            bound: None,
            values: || enum_names(&STROKE_JOINS),
        },
        PropertyDef {
            pattern: "effects/#/type",
            kind: PropertyKind::Enum,
            scope: Some(Scope::AppearanceShadow),
            count: |node| node.effects.len(),
            get: |_, node, index| {
                Some(PropertyValue::Enum(enum_name(
                    &node.effects.get(index)?.kind,
                )))
            },
            set: None,
            bound: None,
            values: || enum_names(&SHADOW_KINDS),
        },
        PropertyDef {
            pattern: "effects/#/color",
            kind: PropertyKind::Color,
            scope: Some(Scope::AppearanceShadow),
            count: |node| node.effects.len(),
            get: |_, node, index| Some(PropertyValue::Color(node.effects.get(index)?.color)),
            set: Some(|doc, node, index, value| {
                let new = color(value)?;
                Ok(Write::Operations(effects_operations(
                    doc,
                    node.id,
                    |effects| {
                        if let Some(shadow) = effects.get_mut(index) {
                            shadow.color = new;
                        }
                    },
                )))
            }),
            bound: None,
            values: no_values,
        },
        PropertyDef {
            pattern: "effects/#/blur",
            kind: PropertyKind::Float,
            scope: Some(Scope::AppearanceShadow),
            count: |node| node.effects.len(),
            get: |_, node, index| Some(PropertyValue::Float(node.effects.get(index)?.blur)),
            set: Some(|doc, node, index, value| {
                let new = float(value)?.max(0.0);
                Ok(Write::Operations(effects_operations(
                    doc,
                    node.id,
                    |effects| {
                        if let Some(shadow) = effects.get_mut(index) {
                            shadow.blur = new;
                        }
                    },
                )))
            }),
            bound: None,
            values: no_values,
        },
        PropertyDef {
            pattern: "effects/#/spread",
            kind: PropertyKind::Float,
            scope: Some(Scope::AppearanceShadow),
            count: |node| node.effects.len(),
            get: |_, node, index| Some(PropertyValue::Float(node.effects.get(index)?.spread)),
            set: Some(|doc, node, index, value| {
                let new = float(value)?;
                Ok(Write::Operations(effects_operations(
                    doc,
                    node.id,
                    |effects| {
                        if let Some(shadow) = effects.get_mut(index) {
                            shadow.spread = new;
                        }
                    },
                )))
            }),
            bound: None,
            values: no_values,
        },
        PropertyDef {
            pattern: "blurs/#/type",
            kind: PropertyKind::Enum,
            scope: Some(Scope::AppearanceBlur),
            count: |node| node.blurs.len(),
            get: |_, node, index| {
                Some(PropertyValue::Enum(enum_name(&node.blurs.get(index)?.kind)))
            },
            set: None,
            bound: None,
            values: || enum_names(&BLUR_KINDS),
        },
        PropertyDef {
            pattern: "blurs/#/radius",
            kind: PropertyKind::Float,
            scope: Some(Scope::AppearanceBlur),
            count: |node| node.blurs.len(),
            get: |_, node, index| Some(PropertyValue::Float(node.blurs.get(index)?.radius)),
            set: Some(|doc, node, index, value| {
                let new = float(value)?.max(0.0);
                Ok(Write::Operations(blurs_operations(doc, node.id, |blurs| {
                    if let Some(blur) = blurs.get_mut(index) {
                        blur.radius = new;
                    }
                })))
            }),
            bound: None,
            values: no_values,
        },
        PropertyDef {
            pattern: "text/content",
            kind: PropertyKind::String,
            scope: Some(Scope::TextEdit),
            count: text_count,
            get: |_, node, _| match &node.data {
                NodeData::Text(text) => Some(PropertyValue::String(text.content.clone())),
                _ => None,
            },
            set: Some(|_, node, _, value| {
                let mut operation = set_props(node.id);
                if let DesignOp::SetProps { text, .. } = &mut operation {
                    *text = Some(string(value)?.to_owned());
                }
                Ok(Write::DesignOp(operation))
            }),
            bound: Some(|_| BoundProp::TextContent),
            values: no_values,
        },
        PropertyDef {
            pattern: "text/font_family",
            kind: PropertyKind::String,
            scope: Some(Scope::TextCharacter),
            count: text_count,
            get: |_, node, _| Some(PropertyValue::String(text_style(node)?.font_family.clone())),
            set: Some(|doc, node, _, value| {
                let family = string(value)?.trim().to_owned();
                if family.is_empty() {
                    return Err(ApiError::InvalidArgument(
                        "a font family cannot be empty".into(),
                    ));
                }
                Ok(text_style_write(doc, node, move |style| {
                    style.font_family = family.clone()
                }))
            }),
            bound: Some(|_| BoundProp::TextStyle),
            values: no_values,
        },
        PropertyDef {
            pattern: "text/font_size",
            kind: PropertyKind::Float,
            scope: Some(Scope::TextCharacter),
            count: text_count,
            get: |_, node, _| Some(PropertyValue::Float(text_style(node)?.size_px)),
            set: Some(|doc, node, _, value| {
                let size = float(value)?;
                if size <= 0.0 {
                    return Err(ApiError::InvalidArgument(
                        "a font size must be positive".into(),
                    ));
                }
                Ok(text_style_write(doc, node, |style| style.size_px = size))
            }),
            bound: Some(|_| BoundProp::TextStyle),
            values: no_values,
        },
        PropertyDef {
            pattern: "text/font_weight",
            kind: PropertyKind::Int,
            scope: Some(Scope::TextCharacter),
            count: text_count,
            get: |_, node, _| Some(PropertyValue::Int(i64::from(text_style(node)?.weight))),
            set: Some(|doc, node, _, value| {
                let PropertyValue::Int(weight) = value else {
                    return Err(ApiError::InvalidArgument("expected an int value".into()));
                };
                let weight = u16::try_from(*weight)
                    .ok()
                    .filter(|weight| (1..=1000).contains(weight))
                    .ok_or_else(|| {
                        ApiError::InvalidArgument("a font weight must be between 1 and 1000".into())
                    })?;
                Ok(text_style_write(doc, node, |style| style.weight = weight))
            }),
            bound: Some(|_| BoundProp::TextStyle),
            values: no_values,
        },
        PropertyDef {
            pattern: "text/italic",
            kind: PropertyKind::Bool,
            scope: Some(Scope::TextCharacter),
            count: text_count,
            get: |_, node, _| Some(PropertyValue::Bool(text_style(node)?.italic)),
            set: Some(|doc, node, _, value| {
                let italic = boolean(value)?;
                Ok(text_style_write(doc, node, |style| style.italic = italic))
            }),
            bound: Some(|_| BoundProp::TextStyle),
            values: no_values,
        },
        PropertyDef {
            pattern: "text/underline",
            kind: PropertyKind::Bool,
            scope: Some(Scope::TextCharacter),
            count: text_count,
            get: |_, node, _| Some(PropertyValue::Bool(text_style(node)?.underline)),
            set: Some(|doc, node, _, value| {
                let underline = boolean(value)?;
                Ok(text_style_write(doc, node, |style| {
                    style.underline = underline
                }))
            }),
            bound: Some(|_| BoundProp::TextStyle),
            values: no_values,
        },
        PropertyDef {
            pattern: "text/strikethrough",
            kind: PropertyKind::Bool,
            scope: Some(Scope::TextCharacter),
            count: text_count,
            get: |_, node, _| Some(PropertyValue::Bool(text_style(node)?.strikethrough)),
            set: Some(|doc, node, _, value| {
                let strikethrough = boolean(value)?;
                Ok(text_style_write(doc, node, |style| {
                    style.strikethrough = strikethrough
                }))
            }),
            bound: Some(|_| BoundProp::TextStyle),
            values: no_values,
        },
        PropertyDef {
            pattern: "text/letter_spacing",
            kind: PropertyKind::Float,
            scope: Some(Scope::TextCharacter),
            count: text_count,
            get: |_, node, _| Some(PropertyValue::Float(text_style(node)?.letter_spacing)),
            set: Some(|doc, node, _, value| {
                let spacing = float(value)?;
                Ok(text_style_write(doc, node, |style| {
                    style.letter_spacing = spacing
                }))
            }),
            bound: Some(|_| BoundProp::TextStyle),
            values: no_values,
        },
        PropertyDef {
            pattern: "text/line_height",
            kind: PropertyKind::Float,
            scope: Some(Scope::TextCharacter),
            count: text_count,
            get: |_, node, _| Some(PropertyValue::Float(text_style(node)?.line_height)),
            set: Some(|doc, node, _, value| {
                let line_height = float(value)?;
                if line_height <= 0.0 {
                    return Err(ApiError::InvalidArgument(
                        "a line height must be positive".into(),
                    ));
                }
                Ok(text_style_write(doc, node, |style| {
                    style.line_height = line_height;
                    style.line_height_auto_percent = None;
                }))
            }),
            bound: Some(|_| BoundProp::TextStyle),
            values: no_values,
        },
        PropertyDef {
            pattern: "text/color",
            kind: PropertyKind::Color,
            scope: Some(Scope::TextCharacter),
            count: text_count,
            get: |_, node, _| Some(PropertyValue::Color(text_style(node)?.color)),
            set: Some(|doc, node, _, value| {
                let new = color(value)?;
                Ok(text_style_write(doc, node, |style| style.color = new))
            }),
            bound: Some(|_| BoundProp::TextStyle),
            values: no_values,
        },
        PropertyDef {
            pattern: "text/align",
            kind: PropertyKind::Enum,
            scope: Some(Scope::TextCharacter),
            count: text_count,
            get: |_, node, _| match &node.data {
                NodeData::Text(text) => Some(PropertyValue::Enum(enum_name(&text.align))),
                _ => None,
            },
            set: Some(|doc, node, _, value| {
                let new: TextAlign = parse_enum(value)?;
                Ok(Write::Operations(replace_data_operation(
                    doc,
                    node.id,
                    |data| {
                        if let NodeData::Text(text) = data {
                            text.align = new;
                        }
                    },
                )))
            }),
            bound: None,
            values: || enum_names(&TEXT_ALIGNS),
        },
        PropertyDef {
            pattern: "text/vertical_align",
            kind: PropertyKind::Enum,
            scope: Some(Scope::TextCharacter),
            count: text_count,
            get: |_, node, _| match &node.data {
                NodeData::Text(text) => Some(PropertyValue::Enum(enum_name(&text.vertical_align))),
                _ => None,
            },
            set: Some(|doc, node, _, value| {
                let new: VAlign = parse_enum(value)?;
                Ok(Write::Operations(replace_data_operation(
                    doc,
                    node.id,
                    |data| {
                        if let NodeData::Text(text) = data {
                            text.vertical_align = new;
                        }
                    },
                )))
            }),
            bound: None,
            values: || enum_names(&VERTICAL_ALIGNS),
        },
        PropertyDef {
            pattern: "text/auto_resize",
            kind: PropertyKind::Enum,
            scope: Some(Scope::TextCharacter),
            count: text_count,
            get: |_, node, _| match &node.data {
                NodeData::Text(text) => Some(PropertyValue::Enum(enum_name(&text.auto_resize))),
                _ => None,
            },
            set: Some(|doc, node, _, value| {
                let new: TextAutoResize = parse_enum(value)?;
                Ok(Write::Operations(replace_data_operation(
                    doc,
                    node.id,
                    |data| {
                        if let NodeData::Text(text) = data {
                            text.auto_resize = new;
                        }
                    },
                )))
            }),
            bound: None,
            values: || enum_names(&AUTO_RESIZES),
        },
        PropertyDef {
            pattern: "layout/mode",
            kind: PropertyKind::Enum,
            scope: Some(Scope::LayerResize),
            count: auto_layout_count,
            get: |_, node, _| match &node.data {
                NodeData::Group(group) => group
                    .auto_layout
                    .as_ref()
                    .map(|layout| PropertyValue::Enum(enum_name(&layout.mode))),
                _ => None,
            },
            set: None,
            bound: None,
            values: || enum_names(&LAYOUT_MODES),
        },
        PropertyDef {
            pattern: "layout/gap",
            kind: PropertyKind::Float,
            scope: Some(Scope::LayerResize),
            count: auto_layout_count,
            get: |_, node, _| match &node.data {
                NodeData::Group(group) => group
                    .auto_layout
                    .as_ref()
                    .map(|layout| PropertyValue::Float(layout.spacing)),
                _ => None,
            },
            set: Some(|doc, node, _, value| {
                let gap = float(value)?;
                Ok(Write::Operations(replace_data_operation(
                    doc,
                    node.id,
                    |data| {
                        if let NodeData::Group(group) = data
                            && let Some(layout) = group.auto_layout.as_mut()
                        {
                            layout.spacing = gap;
                        }
                    },
                )))
            }),
            bound: None,
            values: no_values,
        },
    ];
    table.extend([
        padding_def(
            "layout/padding/top",
            |_, node, _| padding(node, 0),
            |doc, node, _, value| set_padding(doc, node, 0, value),
        ),
        padding_def(
            "layout/padding/right",
            |_, node, _| padding(node, 1),
            |doc, node, _, value| set_padding(doc, node, 1, value),
        ),
        padding_def(
            "layout/padding/bottom",
            |_, node, _| padding(node, 2),
            |doc, node, _, value| set_padding(doc, node, 2, value),
        ),
        padding_def(
            "layout/padding/left",
            |_, node, _| padding(node, 3),
            |doc, node, _, value| set_padding(doc, node, 3, value),
        ),
        adjust_def(
            "fills/#/image/adjust/exposure",
            |_, node, index| adjust_value(node, index, |adjust| adjust.exposure),
            |doc, node, index, value| {
                set_adjust(doc, node, index, value, |adjust, value| {
                    adjust.exposure = value
                })
            },
        ),
        adjust_def(
            "fills/#/image/adjust/contrast",
            |_, node, index| adjust_value(node, index, |adjust| adjust.contrast),
            |doc, node, index, value| {
                set_adjust(doc, node, index, value, |adjust, value| {
                    adjust.contrast = value
                })
            },
        ),
        adjust_def(
            "fills/#/image/adjust/saturation",
            |_, node, index| adjust_value(node, index, |adjust| adjust.saturation),
            |doc, node, index, value| {
                set_adjust(doc, node, index, value, |adjust, value| {
                    adjust.saturation = value
                })
            },
        ),
        adjust_def(
            "fills/#/image/adjust/temperature",
            |_, node, index| adjust_value(node, index, |adjust| adjust.temperature),
            |doc, node, index, value| {
                set_adjust(doc, node, index, value, |adjust, value| {
                    adjust.temperature = value
                })
            },
        ),
        adjust_def(
            "fills/#/image/adjust/tint",
            |_, node, index| adjust_value(node, index, |adjust| adjust.tint),
            |doc, node, index, value| {
                set_adjust(doc, node, index, value, |adjust, value| adjust.tint = value)
            },
        ),
        adjust_def(
            "fills/#/image/adjust/highlights",
            |_, node, index| adjust_value(node, index, |adjust| adjust.highlights),
            |doc, node, index, value| {
                set_adjust(doc, node, index, value, |adjust, value| {
                    adjust.highlights = value
                })
            },
        ),
        adjust_def(
            "fills/#/image/adjust/shadows",
            |_, node, index| adjust_value(node, index, |adjust| adjust.shadows),
            |doc, node, index, value| {
                set_adjust(doc, node, index, value, |adjust, value| {
                    adjust.shadows = value
                })
            },
        ),
    ]);
    table.push(PropertyDef {
        pattern: "effects/#/x",
        kind: PropertyKind::Float,
        scope: Some(Scope::AppearanceShadow),
        count: |node| node.effects.len(),
        get: |_, node, index| Some(PropertyValue::Float(node.effects.get(index)?.offset[0])),
        set: Some(|doc, node, index, value| set_shadow_offset(doc, node, index, 0, value)),
        bound: None,
        values: no_values,
    });
    table.push(PropertyDef {
        pattern: "effects/#/y",
        kind: PropertyKind::Float,
        scope: Some(Scope::AppearanceShadow),
        count: |node| node.effects.len(),
        get: |_, node, index| Some(PropertyValue::Float(node.effects.get(index)?.offset[1])),
        set: Some(|doc, node, index, value| set_shadow_offset(doc, node, index, 1, value)),
        bound: None,
        values: no_values,
    });
    table
}

fn padding_def(pattern: &'static str, get: Getter, set: Setter) -> PropertyDef {
    PropertyDef {
        pattern,
        kind: PropertyKind::Float,
        scope: Some(Scope::LayerResize),
        count: auto_layout_count,
        get,
        set: Some(set),
        bound: None,
        values: no_values,
    }
}

fn adjust_def(pattern: &'static str, get: Getter, set: Setter) -> PropertyDef {
    PropertyDef {
        pattern,
        kind: PropertyKind::Float,
        scope: Some(Scope::AppearanceAdjustments),
        count: fill_count,
        get,
        set: Some(set),
        bound: None,
        values: no_values,
    }
}

fn padding(node: &CanvasNode, side: usize) -> Option<PropertyValue> {
    match &node.data {
        NodeData::Group(group) => group
            .auto_layout
            .as_ref()
            .and_then(|layout| layout.padding.get(side))
            .map(|padding| PropertyValue::Float(*padding)),
        _ => None,
    }
}

fn set_padding(doc: &Doc, node: &CanvasNode, side: usize, value: &PropertyValue) -> Result<Write> {
    let padding = float(value)?;
    if padding < 0.0 {
        return Err(ApiError::InvalidArgument(
            "padding cannot be negative".into(),
        ));
    }
    Ok(Write::Operations(replace_data_operation(
        doc,
        node.id,
        |data| {
            if let NodeData::Group(group) = data
                && let Some(slot) = group
                    .auto_layout
                    .as_mut()
                    .and_then(|layout| layout.padding.get_mut(side))
            {
                *slot = padding;
            }
        },
    )))
}

fn adjust_value(
    node: &CanvasNode,
    index: usize,
    read: fn(&ImageAdjust) -> f32,
) -> Option<PropertyValue> {
    image_adjust(node, index).map(|adjust| PropertyValue::Float(f64::from(read(adjust))))
}

fn set_adjust(
    doc: &Doc,
    node: &CanvasNode,
    index: usize,
    value: &PropertyValue,
    write: fn(&mut ImageAdjust, f32),
) -> Result<Write> {
    let amount = float(value)?.clamp(-1.0, 1.0) as f32;
    if image_adjust(node, index).is_none() {
        return Err(ApiError::InvalidArgument(format!(
            "fill {index} is not an image"
        )));
    }
    Ok(Write::Operations(replace_data_operation(
        doc,
        node.id,
        |data| {
            if let Some(Fill::Image { adjust, .. }) = fill_mut(data, index) {
                write(adjust, amount);
            }
        },
    )))
}

fn set_shadow_offset(
    doc: &Doc,
    node: &CanvasNode,
    index: usize,
    axis: usize,
    value: &PropertyValue,
) -> Result<Write> {
    let offset = float(value)?;
    Ok(Write::Operations(effects_operations(
        doc,
        node.id,
        |effects| {
            if let Some(slot) = effects
                .get_mut(index)
                .and_then(|shadow| shadow.offset.get_mut(axis))
            {
                *slot = offset;
            }
        },
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NodeKind;

    fn session_with_rectangle() -> (DesignSession, NodeId) {
        let mut session = DesignSession::new().expect("session");
        let page = session.active_page().expect("page");
        let rectangle = session
            .create_node(page, NodeKind::Rectangle)
            .expect("rectangle");
        (session, rectangle)
    }

    #[test]
    fn every_listed_property_reads_back_with_its_kind() {
        let mut session = DesignSession::new().expect("session");
        let page = session.active_page().expect("page");
        let frame = session.create_node(page, NodeKind::Frame).expect("frame");
        let text = session
            .create_node(frame, NodeKind::Text("Hi".into()))
            .expect("text");
        session
            .add_stroke(frame, Color::BLACK, 2.0)
            .expect("stroke");
        session.add_shadow(frame, ShadowKind::Drop).expect("shadow");
        session.add_blur(frame, BlurKind::Layer, 3.0).expect("blur");
        for node in [page, frame, text] {
            let properties = session.find_all_properties(node).expect("properties");
            assert!(!properties.is_empty());
            for property in properties {
                let value = session
                    .property(node, &property.path)
                    .expect(&property.path);
                assert_eq!(value.kind(), property.kind, "{}", property.path);
                if property.kind == PropertyKind::Enum {
                    let values = session.enum_values(node, &property.path).expect("values");
                    let PropertyValue::Enum(name) = value else {
                        unreachable!("kinds match");
                    };
                    assert!(values.contains(&name), "{} = {name}", property.path);
                }
            }
        }
        let paths: Vec<String> = session
            .find_all_properties(frame)
            .expect("properties")
            .into_iter()
            .map(|property| property.path)
            .collect();
        for expected in [
            "strokes/0/width",
            "effects/0/blur",
            "blurs/0/radius",
            "fills/0/color",
        ] {
            assert!(
                paths.iter().any(|path| path == expected),
                "{expected} in {paths:?}"
            );
        }
    }

    #[test]
    fn typed_setters_refuse_mismatched_kinds_and_unknown_enum_values() {
        let (mut session, rectangle) = session_with_rectangle();
        assert!(matches!(
            session.set_string(rectangle, "opacity", "half"),
            Err(ApiError::PropertyKind { .. })
        ));
        assert!(matches!(
            session.set_enum(rectangle, "blend_mode", "glow"),
            Err(ApiError::InvalidEnumValue { .. })
        ));
        assert!(matches!(
            session.set_enum(rectangle, "fills/0/type", "image"),
            Err(ApiError::ReadOnly(_))
        ));
        assert!(matches!(
            session.get_float(rectangle, "no/such/path"),
            Err(ApiError::UnknownProperty { .. })
        ));
        assert!(matches!(
            session.get_color(rectangle, "fills/7/color"),
            Err(ApiError::UnknownProperty { .. })
        ));
        session
            .set_enum(rectangle, "blend_mode", "linear_burn")
            .expect("blend");
        assert_eq!(
            session.get_enum(rectangle, "blend_mode").expect("blend"),
            "linear_burn"
        );
        session
            .set_float(rectangle, "opacity", 0.5)
            .expect("opacity");
        assert!((session.get_float(rectangle, "opacity").expect("opacity") - 0.5).abs() < 1e-6);
        session.set_bool(rectangle, "visible", false).expect("hide");
        assert!(!session.get_bool(rectangle, "visible").expect("visible"));
    }

    #[test]
    fn indexed_paths_address_fills_strokes_effects_and_blurs() {
        let (mut session, rectangle) = session_with_rectangle();
        let red = Color::rgb(255, 0, 0);
        let second = session.add_fill(rectangle, red).expect("fill");
        assert_eq!(second, 1);
        assert_eq!(
            session
                .get_color(rectangle, "fills/1/color")
                .expect("color"),
            red
        );
        session
            .remove_fill(rectangle, 0)
            .expect("remove the first fill");
        assert_eq!(
            session
                .get_color(rectangle, "fills/0/color")
                .expect("color"),
            red
        );

        let stroke = session
            .add_stroke(rectangle, Color::BLACK, 1.0)
            .expect("stroke");
        session
            .set_float(rectangle, &format!("strokes/{stroke}/width"), 4.0)
            .expect("width");
        session
            .set_enum(rectangle, "strokes/0/align", "inside")
            .expect("align");
        assert_eq!(
            session
                .get_float(rectangle, "strokes/0/width")
                .expect("width"),
            4.0
        );

        let shadow = session
            .add_shadow(rectangle, ShadowKind::Inner)
            .expect("shadow");
        assert_eq!(
            session.get_enum(rectangle, "effects/0/type").expect("kind"),
            "inner"
        );
        session
            .set_float(rectangle, &format!("effects/{shadow}/y"), 6.0)
            .expect("offset");
        assert_eq!(
            session.get_float(rectangle, "effects/0/y").expect("offset"),
            6.0
        );

        session
            .add_blur(rectangle, BlurKind::Background, 2.0)
            .expect("blur");
        session
            .set_float(rectangle, "blurs/0/radius", 8.0)
            .expect("radius");
        assert_eq!(
            session
                .get_float(rectangle, "blurs/0/radius")
                .expect("radius"),
            8.0
        );
        session.remove_blur(rectangle, 0).expect("remove blur");
        assert!(session.get_float(rectangle, "blurs/0/radius").is_err());
    }

    #[test]
    fn frame_fill_zero_is_the_background() {
        let mut session = DesignSession::new().expect("session");
        let page = session.active_page().expect("page");
        let frame = session.create_node(page, NodeKind::Frame).expect("frame");
        let blue = Color::rgb(0, 0, 255);
        session
            .set_color(frame, "fills/0/color", blue)
            .expect("background");
        let NodeData::Group(group) = &session.node(frame).expect("frame").data else {
            panic!("a frame is a group");
        };
        assert_eq!(
            group.background,
            Some(Fill::Solid {
                color: blue,
                blend: BlendMode::Normal
            })
        );
    }

    #[test]
    fn text_style_properties_reach_every_style_run() {
        let mut session = DesignSession::new().expect("session");
        let page = session.active_page().expect("page");
        let text = session
            .create_node(page, NodeKind::Text("Hello world".into()))
            .expect("text");
        let run_style = text_style(session.node(text).expect("text"))
            .cloned()
            .expect("a text style");
        let operations = replace_data_operation(session.doc(), text, |data| {
            if let NodeData::Text(text) = data {
                text.style_runs.push(fanta_doc::TextStyleRun {
                    start: 0,
                    end: 5,
                    style: run_style,
                });
            }
        });
        session.edit(operations).expect("add a run");
        session
            .set_float(text, "text/font_size", 32.0)
            .expect("size");
        session
            .set_int(text, "text/font_weight", 700)
            .expect("weight");
        let NodeData::Text(node) = &session.node(text).expect("text").data else {
            panic!("a text node");
        };
        assert_eq!(node.style.size_px, 32.0);
        assert!(
            node.style_runs
                .iter()
                .all(|run| run.style.size_px == 32.0 && run.style.weight == 700)
        );
        assert!(session.set_int(text, "text/font_weight", 0).is_err());
    }

    #[test]
    fn geometry_properties_preserve_rotation() {
        let (mut session, rectangle) = session_with_rectangle();
        session
            .set_float(rectangle, "rotation", 30.0)
            .expect("rotate");
        session
            .set_float(rectangle, "size/width", 240.0)
            .expect("resize");
        assert!((session.get_float(rectangle, "rotation").expect("rotation") - 30.0).abs() < 1e-6);
        assert!((session.get_float(rectangle, "size/width").expect("width") - 240.0).abs() < 1e-6);
        session
            .set_float(rectangle, "rotation", 0.0)
            .expect("unrotate");
        session
            .set_float(rectangle, "position/x", 50.0)
            .expect("move");
        assert!((session.get_float(rectangle, "position/x").expect("x") - 50.0).abs() < 1e-6);
        session.flip(rectangle, true).expect("flip");
        assert!((session.get_float(rectangle, "position/x").expect("x") - 50.0).abs() < 1e-6);
    }

    #[test]
    fn bound_properties_refuse_literal_writes() {
        let (mut session, rectangle) = session_with_rectangle();
        let ops: Vec<DesignOp> = serde_json::from_value(serde_json::json!([
            {"op": "create_variable_collection", "name": "Tokens", "modes": ["Light"]},
        ]))
        .expect("ops");
        session.edit_with_design_op(&ops[0]).expect("collection");
        let collection = session
            .doc()
            .variables
            .collections
            .values()
            .next()
            .map(|collection| collection.id)
            .expect("collection id");
        let ops: Vec<DesignOp> = serde_json::from_value(serde_json::json!([
            {"op": "create_variable", "collection": collection.to_string(), "name": "Fade",
             "kind": "float", "value": 0.25},
        ]))
        .expect("ops");
        session.edit_with_design_op(&ops[0]).expect("variable");
        let variable = session
            .doc()
            .variables
            .variables
            .values()
            .next()
            .map(|variable| variable.id)
            .expect("variable id");
        let ops: Vec<DesignOp> = serde_json::from_value(serde_json::json!([
            {"op": "bind_variable", "id": rectangle.to_string(),
             "property": {"prop": "opacity"}, "variable": variable.to_string()},
        ]))
        .expect("ops");
        session.edit_with_design_op(&ops[0]).expect("bind");
        assert!(matches!(
            session.set_float(rectangle, "opacity", 1.0),
            Err(ApiError::PropertyBound(_))
        ));
    }

    #[test]
    fn property_patterns_are_unique_and_parse() {
        let mut seen = std::collections::HashSet::new();
        for def in PROPERTIES.iter() {
            assert!(seen.insert(def.pattern), "{} is listed twice", def.pattern);
            let concrete = def.pattern.replace('#', "3");
            assert_eq!(
                match_pattern(def.pattern, &concrete),
                Some(if def.pattern.contains('#') { 3 } else { 0 })
            );
            if def.kind == PropertyKind::Enum {
                assert!(
                    !(def.values)().is_empty(),
                    "{} lists its values",
                    def.pattern
                );
            }
        }
        assert_eq!(match_pattern("fills/#/color", "fills/x/color"), None);
        assert_eq!(match_pattern("fills/#/color", "fills/0/color/extra"), None);
    }
}
