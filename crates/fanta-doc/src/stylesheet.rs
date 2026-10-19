use crate::{BoundProp, CanvasNode, StyleId, VariableId};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StyleProperty {
    Fill,
    Fills,
    Background,
    BackgroundFills,
    Strokes,
    Opacity,
    BlendMode,
    Effects,
    Blurs,
    CornerRadius,
    CornerRadii,
    CornerSmoothing,
    Typography,
    FontFamily,
    FontSize,
    FontWeight,
    FontItalic,
    LineHeight,
    LetterSpacing,
    Color,
    Align,
    VerticalAlign,
    AutoResize,
    MaxLines,
    Truncate,
    ParagraphSpacing,
    ParagraphIndent,
    AutoLayout,
    Layout,
    Padding,
    Gap,
    Grid,
    LayoutChild,
    Constraints,
}

impl StyleProperty {
    pub fn parse(name: &str) -> Result<Self, StyleError> {
        let name = name.replace('-', "_");
        let name = match name.as_str() {
            "style" => "typography",
            "size_px" => "font_size",
            other => other,
        };
        serde_json::from_value(Value::String(name.to_owned()))
            .map_err(|_| StyleError::UnknownProperty(name.to_owned()))
    }

    fn path(self, node_type: &str) -> Result<Vec<&'static str>, StyleError> {
        let text = matches!(node_type, "text" | "text_path");
        let group = node_type == "group";
        let painted = matches!(node_type, "vector" | "boolean");
        let box_text = node_type == "text";
        let path = match self {
            Self::Fill if text => vec!["style", "color"],
            Self::Fill if group => vec!["background"],
            Self::Fill if painted => vec!["fills"],
            Self::Fills if painted => vec!["fills"],
            Self::Background if group => vec!["background"],
            Self::BackgroundFills if group => vec!["background_fills"],
            Self::Strokes if group || painted => vec!["strokes"],
            Self::Opacity => vec!["opacity"],
            Self::BlendMode => vec!["blend_mode"],
            Self::Effects => vec!["effects"],
            Self::Blurs => vec!["blurs"],
            Self::CornerRadius if group || node_type == "vector" => vec!["corner_radius"],
            Self::CornerRadii if group || node_type == "vector" => vec!["corner_radii"],
            Self::CornerSmoothing if group || node_type == "vector" => vec!["corner_smoothing"],
            Self::Typography if text => vec!["style"],
            Self::FontFamily if text => vec!["style", "font_family"],
            Self::FontSize if text => vec!["style", "size_px"],
            Self::FontWeight if text => vec!["style", "weight"],
            Self::FontItalic if text => vec!["style", "italic"],
            Self::LineHeight if text => vec!["style", "line_height"],
            Self::LetterSpacing if text => vec!["style", "letter_spacing"],
            Self::Color if text => vec!["style", "color"],
            Self::Align if box_text => vec!["align"],
            Self::VerticalAlign if box_text => vec!["vertical_align"],
            Self::AutoResize if box_text => vec!["auto_resize"],
            Self::MaxLines if box_text => vec!["max_lines"],
            Self::Truncate if box_text => vec!["truncate"],
            Self::ParagraphSpacing if box_text => vec!["paragraph_spacing"],
            Self::ParagraphIndent if box_text => vec!["paragraph_indent"],
            Self::AutoLayout if group => vec!["auto_layout"],
            Self::Layout if group => vec!["auto_layout", "mode"],
            Self::Padding if group => vec!["auto_layout", "padding"],
            Self::Gap if group => vec!["auto_layout", "spacing"],
            Self::Grid if group => vec!["grid"],
            Self::LayoutChild => vec!["layout_child"],
            Self::Constraints => vec!["constraints"],
            _ => {
                return Err(StyleError::IncompatibleProperty {
                    property: self,
                    node_type: node_type.to_owned(),
                });
            }
        };
        Ok(path)
    }

    pub fn binding(self) -> Option<BoundProp> {
        match self {
            Self::Fill | Self::Color | Self::Background => Some(BoundProp::FillColor { index: 0 }),
            Self::Opacity => Some(BoundProp::Opacity),
            Self::CornerRadius => Some(BoundProp::CornerRadius),
            Self::Typography => Some(BoundProp::TextStyle),
            _ => None,
        }
    }

    pub fn validate_literal(self, value: &Value) -> Result<(), StyleError> {
        fn validate<T: serde::de::DeserializeOwned>(value: &Value) -> Result<(), StyleError> {
            serde_json::from_value::<T>(value.clone())
                .map(|_| ())
                .map_err(|error| StyleError::InvalidValue(error.to_string()))
        }
        match self {
            Self::Fill | Self::Background => {
                if value.is_null() {
                    Ok(())
                } else if value.is_array() {
                    validate::<Vec<crate::Fill>>(value)
                } else if value.get("kind").is_some() {
                    validate::<crate::Fill>(value)
                } else {
                    validate::<crate::Color>(value)
                }
            }
            Self::Fills | Self::BackgroundFills => validate::<Vec<crate::Fill>>(value),
            Self::Strokes => validate::<Vec<crate::Stroke>>(value),
            Self::Opacity => {
                if value
                    .as_f64()
                    .is_some_and(|value| (0.0..=1.0).contains(&value))
                {
                    Ok(())
                } else {
                    Err(StyleError::InvalidValue("opacity must be in 0..=1".into()))
                }
            }
            Self::BlendMode => validate::<crate::BlendMode>(value),
            Self::Effects => validate::<Vec<crate::Shadow>>(value),
            Self::Blurs => validate::<Vec<crate::Blur>>(value),
            Self::CornerRadius => validate::<Option<f64>>(value),
            Self::CornerRadii => validate::<Option<[f64; 4]>>(value),
            Self::CornerSmoothing
            | Self::FontSize
            | Self::LineHeight
            | Self::LetterSpacing
            | Self::Gap
            | Self::ParagraphSpacing
            | Self::ParagraphIndent => validate::<f64>(value),
            Self::Typography => validate::<crate::TextStyle>(value),
            Self::FontFamily => validate::<String>(value),
            Self::FontWeight => validate::<u16>(value),
            Self::FontItalic | Self::Truncate => validate::<bool>(value),
            Self::Color => validate::<crate::Color>(value),
            Self::Align => validate::<crate::TextAlign>(value),
            Self::VerticalAlign => validate::<crate::VAlign>(value),
            Self::AutoResize => validate::<crate::TextAutoResize>(value),
            Self::MaxLines => validate::<Option<u32>>(value),
            Self::AutoLayout => validate::<Option<crate::AutoLayout>>(value),
            Self::Layout => validate::<crate::LayoutMode>(value),
            Self::Padding if value.is_number() => validate::<f64>(value),
            Self::Padding => validate::<[f64; 4]>(value),
            Self::Grid => validate::<Option<crate::GridLayout>>(value),
            Self::LayoutChild => validate::<Option<crate::LayoutChild>>(value),
            Self::Constraints => validate::<Option<crate::Constraints>>(value),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StyleDeclaration {
    pub value: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub binding: Option<VariableId>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StyleRule {
    pub id: StyleId,
    pub declarations: BTreeMap<StyleProperty, StyleDeclaration>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct StyleRegistry {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub rules: BTreeMap<StyleId, StyleRule>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct StyleAssignment {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub style_ids: Vec<StyleId>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub local_patch: BTreeMap<StyleProperty, StyleDeclaration>,
    #[serde(
        default,
        skip_serializing_if = "BTreeMap::is_empty",
        with = "crate::binding::map_as_seq"
    )]
    pub binding_overrides: BTreeMap<BoundProp, VariableId>,
}

impl StyleAssignment {
    pub fn is_empty(&self) -> bool {
        self.style_ids.is_empty()
            && self.local_patch.is_empty()
            && self.binding_overrides.is_empty()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum StyleError {
    #[error("unknown style property {0}")]
    UnknownProperty(String),
    #[error("style {0} is missing")]
    MissingRule(StyleId),
    #[error("style property {property:?} is incompatible with {node_type}")]
    IncompatibleProperty {
        property: StyleProperty,
        node_type: String,
    },
    #[error("style property {0:?} cannot be variable-bound")]
    UnsupportedBinding(StyleProperty),
    #[error("invalid style value: {0}")]
    InvalidValue(String),
    #[error("style declarations overlap at {0}")]
    OverlappingProperty(String),
}

impl StyleRegistry {
    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    pub fn authored_node_value(&self, node: &CanvasNode) -> Result<Value, StyleError> {
        let mut value = serde_json::to_value(node)
            .map_err(|error| StyleError::InvalidValue(error.to_string()))?;
        let node_type = value
            .get("type")
            .and_then(Value::as_str)
            .ok_or_else(|| StyleError::InvalidValue("missing node type".into()))?
            .to_owned();
        for id in &node.style_assignment.style_ids {
            let rule = self.rules.get(id).ok_or(StyleError::MissingRule(*id))?;
            for property in rule.declarations.keys() {
                remove_path(&mut value, &property.path(&node_type)?);
            }
        }
        for property in node.style_assignment.local_patch.keys() {
            remove_path(&mut value, &property.path(&node_type)?);
        }
        if let Some(bindings) = value.get_mut("bindings").and_then(Value::as_array_mut) {
            let authored = node
                .style_assignment
                .local_patch
                .iter()
                .filter_map(|(property, declaration)| {
                    declaration
                        .binding
                        .map(|binding| (property.binding(), binding))
                })
                .filter_map(|(property, binding)| property.map(|property| (property, binding)))
                .collect::<BTreeMap<_, _>>();
            for id in &node.style_assignment.style_ids {
                let rule = self.rules.get(id).ok_or(StyleError::MissingRule(*id))?;
                for property in rule
                    .declarations
                    .keys()
                    .filter_map(|property| property.binding())
                {
                    if !authored.contains_key(&property)
                        && !node
                            .style_assignment
                            .binding_overrides
                            .contains_key(&property)
                    {
                        let address = serde_json::to_value(property)
                            .map_err(|error| StyleError::InvalidValue(error.to_string()))?;
                        bindings.retain(|pair| {
                            pair.as_array().and_then(|pair| pair.first()) != Some(&address)
                        });
                    }
                }
            }
        }
        Ok(value)
    }

    pub fn compile_node_value(
        &self,
        value: &mut Value,
        assignment: &StyleAssignment,
    ) -> Result<(), StyleError> {
        let node_type = value
            .get("type")
            .and_then(Value::as_str)
            .ok_or_else(|| StyleError::InvalidValue("node type is missing".into()))?
            .to_owned();
        for id in &assignment.style_ids {
            let rule = self.rules.get(id).ok_or(StyleError::MissingRule(*id))?;
            Self::apply_declarations(value, &node_type, &rule.declarations, true)?;
        }
        Self::apply_declarations(value, &node_type, &assignment.local_patch, false)?;
        if !assignment.binding_overrides.is_empty() {
            let object = value
                .as_object_mut()
                .ok_or_else(|| StyleError::InvalidValue("node must be an object".into()))?;
            let bindings = object
                .entry("bindings")
                .or_insert_with(|| json!([]))
                .as_array_mut()
                .ok_or_else(|| StyleError::InvalidValue("bindings must be an array".into()))?;
            for (property, variable) in &assignment.binding_overrides {
                let address = serde_json::to_value(property)
                    .map_err(|error| StyleError::InvalidValue(error.to_string()))?;
                bindings
                    .retain(|pair| pair.as_array().and_then(|pair| pair.first()) != Some(&address));
                bindings.push(json!([address, variable]));
            }
        }
        value
            .as_object_mut()
            .ok_or_else(|| StyleError::InvalidValue("node must be an object".into()))?
            .insert(
                "style_assignment".into(),
                serde_json::to_value(assignment)
                    .map_err(|error| StyleError::InvalidValue(error.to_string()))?,
            );
        serde_json::from_value::<CanvasNode>(value.clone())
            .map_err(|error| StyleError::InvalidValue(error.to_string()))?;
        Ok(())
    }

    fn apply_declarations(
        value: &mut Value,
        node_type: &str,
        declarations: &BTreeMap<StyleProperty, StyleDeclaration>,
        reject_overlap: bool,
    ) -> Result<(), StyleError> {
        let mut paths: Vec<Vec<&str>> = Vec::new();
        for (property, declaration) in declarations {
            property.validate_literal(&declaration.value)?;
            let path = property.path(node_type)?;
            for previous in &paths {
                if reject_overlap && (path.starts_with(previous) || previous.starts_with(&path)) {
                    return Err(StyleError::OverlappingProperty(path.join(".")));
                }
            }
            paths.push(path.clone());
            let mut literal = declaration.value.clone();
            if matches!(property, StyleProperty::Fill | StyleProperty::Background)
                && node_type == "group"
                && !literal.is_null()
                && literal.get("kind").is_none()
            {
                literal = json!({"kind":"solid", "color":literal});
            }
            if *property == StyleProperty::Fill
                && matches!(node_type, "vector" | "boolean" | "bitmap" | "video")
            {
                literal = if literal.is_array() {
                    literal
                } else if literal.get("kind").is_some() {
                    json!([literal])
                } else {
                    json!([{"kind":"solid","color":literal}])
                };
            }
            if *property == StyleProperty::Padding && literal.is_number() {
                literal = json!([literal.clone(), literal.clone(), literal.clone(), literal]);
            }
            set_path(value, &path, literal)?;
            if let Some(bound) = property.binding() {
                let bindings = value
                    .as_object_mut()
                    .ok_or_else(|| StyleError::InvalidValue("node is not an object".into()))?
                    .entry("bindings")
                    .or_insert_with(|| json!([]));
                let bindings = bindings.as_array_mut().ok_or_else(|| {
                    StyleError::InvalidValue("bindings must be a pair array".into())
                })?;
                let address = serde_json::to_value(bound)
                    .map_err(|error| StyleError::InvalidValue(error.to_string()))?;
                bindings
                    .retain(|pair| pair.as_array().and_then(|pair| pair.first()) != Some(&address));
                if let Some(variable) = declaration.binding {
                    bindings.push(json!([address, variable]));
                }
            } else if declaration.binding.is_some() {
                return Err(StyleError::UnsupportedBinding(*property));
            }
        }
        Ok(())
    }

    /// Reapply authored leaf claims after a composite typography token is resolved.
    pub fn apply_local_patch_after_bindings(
        &self,
        node: &CanvasNode,
    ) -> Result<CanvasNode, StyleError> {
        if node.style_assignment.is_empty() {
            return Ok(node.clone());
        }
        let node_type = match &node.data {
            crate::NodeData::Text(_) => "text",
            crate::NodeData::TextPath(_) => "text_path",
            _ => return Ok(node.clone()),
        };
        let mut declarations: BTreeMap<StyleProperty, StyleDeclaration> = BTreeMap::new();
        for id in &node.style_assignment.style_ids {
            let rule = self.rules.get(id).ok_or(StyleError::MissingRule(*id))?;
            merge_claims(&mut declarations, &rule.declarations, node_type)?;
        }
        merge_claims(
            &mut declarations,
            &node.style_assignment.local_patch,
            node_type,
        )?;
        declarations.retain(|property, declaration| {
            declaration.binding.is_none()
                && property.binding().is_none_or(|bound| {
                    !node.style_assignment.binding_overrides.contains_key(&bound)
                })
        });
        let mut resolved = node.clone();
        let style = match &mut resolved.data {
            crate::NodeData::Text(text) => &mut text.style,
            crate::NodeData::TextPath(text) => &mut text.style,
            _ => return Ok(resolved),
        };
        for (property, declaration) in declarations {
            let value = declaration.value;
            match property {
                StyleProperty::FontFamily => style.font_family = typed_literal(value)?,
                StyleProperty::FontSize => style.size_px = typed_literal(value)?,
                StyleProperty::FontWeight => style.weight = typed_literal(value)?,
                StyleProperty::FontItalic => style.italic = typed_literal(value)?,
                StyleProperty::LineHeight => style.line_height = typed_literal(value)?,
                StyleProperty::LetterSpacing => style.letter_spacing = typed_literal(value)?,
                StyleProperty::Color | StyleProperty::Fill => style.color = typed_literal(value)?,
                _ => {}
            }
        }
        Ok(resolved)
    }
}

fn typed_literal<T: serde::de::DeserializeOwned>(value: Value) -> Result<T, StyleError> {
    serde_json::from_value(value).map_err(|error| StyleError::InvalidValue(error.to_string()))
}

fn merge_claims(
    target: &mut BTreeMap<StyleProperty, StyleDeclaration>,
    incoming: &BTreeMap<StyleProperty, StyleDeclaration>,
    node_type: &str,
) -> Result<(), StyleError> {
    for (property, declaration) in incoming {
        let path = property.path(node_type)?;
        let replaced = target
            .keys()
            .copied()
            .filter(|previous| {
                previous
                    .path(node_type)
                    .is_ok_and(|previous| previous.starts_with(&path))
            })
            .collect::<Vec<_>>();
        for previous in replaced {
            target.remove(&previous);
        }
        target.insert(*property, declaration.clone());
    }
    Ok(())
}

fn set_path(value: &mut Value, path: &[&str], literal: Value) -> Result<(), StyleError> {
    let Some((first, rest)) = path.split_first() else {
        return Err(StyleError::InvalidValue("empty property path".into()));
    };
    let object = value
        .as_object_mut()
        .ok_or_else(|| StyleError::InvalidValue("property parent is not an object".into()))?;
    if rest.is_empty() {
        object.insert((*first).to_owned(), literal);
        return Ok(());
    }
    let child = object
        .entry((*first).to_owned())
        .or_insert_with(|| Value::Object(Map::new()));
    if child.is_null() {
        *child = Value::Object(Map::new());
    }
    set_path(child, rest, literal)
}

fn remove_path(value: &mut Value, path: &[&str]) {
    let Some((first, rest)) = path.split_first() else {
        return;
    };
    let Some(object) = value.as_object_mut() else {
        return;
    };
    if rest.is_empty() {
        object.remove(*first);
    } else if let Some(child) = object.get_mut(*first) {
        remove_path(child, rest);
    }
}
