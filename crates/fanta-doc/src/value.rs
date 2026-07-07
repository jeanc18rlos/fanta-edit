//! The shared typed value — [`VarValue`] — and its mode-resolved sibling.
//!
//! ## Why one value type for everything
//!
//! Design-system variables, component property defaults, instance property
//! values, instance text/bool overrides, and the `SetVariable` prototype action
//! all need to carry "a color, or a number, or a string, or a boolean, or a
//! reference to another variable". Rather than invent a parallel value enum per
//! feature (each with its own serde shape and its own resolution rules) we reuse
//! a single [`VarValue`]. That keeps the cross-feature wiring honest: a
//! component's `default` and a variable's per-mode value are *the same type*, so
//! binding one to the other never needs a lossy conversion.
//!
//! [`ResolvedVarValue`] is the same set minus [`VarValue::Alias`]: once
//! `resolve.rs` has chased every alias to a concrete literal, the result can no
//! longer be a reference, and encoding that in the type means render/export
//! never have to handle an unresolved alias.

use crate::color::Color;
use crate::id::VariableId;
use crate::node::TextStyle;
use serde::{Deserialize, Serialize};

/// A typed value, reused across variables, component props, instance overrides,
/// and the `SetVariable` action.
///
/// `#[serde(tag = "kind")]` keeps the JSON projection self-describing and
/// AI-readable (`{"kind":"color","value":"#FF0000"}`), matching the tagged
/// shape [`crate::style::Fill`] and [`crate::color::Gradient`] already use.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum VarValue {
    /// A color literal (same sRGB-with-alpha space as every other doc color).
    Color { value: Color },
    /// A floating-point number — radii, widths, opacities, spacing, …
    Float { value: f64 },
    /// A string literal — text content, font family, arbitrary token strings.
    String { value: String },
    /// A boolean — visibility, instance-swap toggles, feature flags.
    Boolean { value: bool },
    /// A composite **text style** token — font family, size, weight, italic,
    /// decorations, color, letter spacing, and line height in one value. This is
    /// how Fanta models Figma "text styles": a named, reusable type style that a
    /// text node binds its whole `TextStyle` to (see `BoundProp::TextStyle`).
    TextStyle { value: TextStyle },
    /// A reference to another variable. Resolved (possibly across collections)
    /// by [`crate::resolve::resolve_bound_value`]. Never appears in a
    /// [`ResolvedVarValue`].
    Alias { variable: VariableId },
}

impl VarValue {
    /// The [`VariableType`] this value carries, or `None` for an [`Alias`]
    /// (whose type is only known after resolution).
    ///
    /// [`Alias`]: VarValue::Alias
    pub fn variable_type(&self) -> Option<VariableType> {
        match self {
            Self::Color { .. } => Some(VariableType::Color),
            Self::Float { .. } => Some(VariableType::Float),
            Self::String { .. } => Some(VariableType::String),
            Self::Boolean { .. } => Some(VariableType::Boolean),
            Self::TextStyle { .. } => Some(VariableType::Typography),
            Self::Alias { .. } => None,
        }
    }

    /// If this value is already concrete (not an [`Alias`]), return the
    /// equivalent [`ResolvedVarValue`]. Used as the base case in alias chasing.
    ///
    /// [`Alias`]: VarValue::Alias
    pub fn as_resolved(&self) -> Option<ResolvedVarValue> {
        match self {
            Self::Color { value } => Some(ResolvedVarValue::Color { value: *value }),
            Self::Float { value } => Some(ResolvedVarValue::Float { value: *value }),
            Self::String { value } => Some(ResolvedVarValue::String {
                value: value.clone(),
            }),
            Self::Boolean { value } => Some(ResolvedVarValue::Boolean { value: *value }),
            Self::TextStyle { value } => Some(ResolvedVarValue::TextStyle {
                value: value.clone(),
            }),
            Self::Alias { .. } => None,
        }
    }
}

/// A fully-resolved value — the same set as [`VarValue`] without [`Alias`].
///
/// Producing this is the whole job of alias resolution: render and export only
/// ever see concrete literals, so they cannot accidentally try to paint an
/// unresolved reference.
///
/// [`Alias`]: VarValue::Alias
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ResolvedVarValue {
    Color { value: Color },
    Float { value: f64 },
    String { value: String },
    Boolean { value: bool },
    TextStyle { value: TextStyle },
}

impl ResolvedVarValue {
    pub fn variable_type(&self) -> VariableType {
        match self {
            Self::Color { .. } => VariableType::Color,
            Self::Float { .. } => VariableType::Float,
            Self::String { .. } => VariableType::String,
            Self::Boolean { .. } => VariableType::Boolean,
            Self::TextStyle { .. } => VariableType::Typography,
        }
    }

    /// Widen back into a [`VarValue`] (never an alias).
    pub fn to_var_value(&self) -> VarValue {
        match self {
            Self::Color { value } => VarValue::Color { value: *value },
            Self::Float { value } => VarValue::Float { value: *value },
            Self::String { value } => VarValue::String {
                value: value.clone(),
            },
            Self::Boolean { value } => VarValue::Boolean { value: *value },
            Self::TextStyle { value } => VarValue::TextStyle {
                value: value.clone(),
            },
        }
    }
}

/// The static type of a variable. Mirrors Figma's `variableResolvedType`.
///
/// A variable's per-mode values must all share this type; binding only makes
/// sense when the bound property's expected type matches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VariableType {
    Color,
    Float,
    String,
    Boolean,
    /// A composite text-style token (font + size + weight + metrics). Figma's
    /// text styles, modeled as a variable type so they live in the Variables
    /// editor alongside color/number/string tokens.
    Typography,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn var_value_tagged_json_round_trips() {
        let v = VarValue::Color {
            value: Color::rgb(0x12, 0x34, 0x56),
        };
        let j = serde_json::to_value(&v).unwrap();
        assert_eq!(j["kind"], "color");
        let back: VarValue = serde_json::from_value(j).unwrap();
        assert_eq!(v, back);
    }

    #[test]
    fn text_style_var_value_round_trips_and_resolves() {
        let v = VarValue::TextStyle {
            value: TextStyle {
                size_px: 28.0,
                weight: 600,
                ..TextStyle::default()
            },
        };
        let j = serde_json::to_value(&v).unwrap();
        assert_eq!(j["kind"], "text_style");
        let back: VarValue = serde_json::from_value(j).unwrap();
        assert_eq!(v, back);
        assert_eq!(v.variable_type(), Some(VariableType::Typography));
        // A concrete text style resolves to itself and widens back losslessly.
        let r = v.as_resolved().unwrap();
        assert_eq!(r.variable_type(), VariableType::Typography);
        assert_eq!(r.to_var_value(), v);
    }

    #[test]
    fn alias_has_no_static_type_and_no_resolved_form() {
        let a = VarValue::Alias {
            variable: VariableId::from_u128(1),
        };
        assert_eq!(a.variable_type(), None);
        assert!(a.as_resolved().is_none());
    }

    #[test]
    fn resolved_widens_and_narrows_consistently() {
        let r = ResolvedVarValue::Float { value: 8.0 };
        assert_eq!(r.variable_type(), VariableType::Float);
        let widened = r.to_var_value();
        assert_eq!(widened.as_resolved(), Some(r));
    }
}
