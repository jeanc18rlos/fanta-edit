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

/// Generate the three parallel value types and their conversions from one
/// table of `Variant(PayloadType) => VariableTypeVariant` rows.
///
/// The three types deliberately share a variant set but not a spelling: the
/// value variant `TextStyle` addresses the type variant `Typography` (matching
/// Figma), so each row carries both names. Everything downstream — the
/// `#[serde(tag = "kind")]` wire shape, the `Alias`-only-on-`VarValue`
/// asymmetry, and the four conversions — follows mechanically, so adding a
/// value kind is a single new row rather than an eight-site edit. The macro is
/// invoked exactly once; it exists to make the parallelism a compiler-checked
/// fact instead of a hand-maintained coincidence.
macro_rules! var_values {
    ( $( $(#[$vmeta:meta])* $variant:ident($ty:ty) => $vartype:ident ),+ $(,)? ) => {
        /// A typed value, reused across variables, component props, instance
        /// overrides, and the `SetVariable` action.
        ///
        /// `#[serde(tag = "kind")]` keeps the JSON projection self-describing
        /// and AI-readable (`{"kind":"color","value":"#FF0000"}`), matching the
        /// tagged shape [`crate::style::Fill`] and [`crate::color::Gradient`]
        /// already use.
        #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
        #[serde(tag = "kind", rename_all = "snake_case")]
        pub enum VarValue {
            $( $(#[$vmeta])* $variant { value: $ty }, )+
            /// A reference to another variable. Resolved (possibly across
            /// collections) by [`crate::resolve::resolve_bound_value`]. Never
            /// appears in a [`ResolvedVarValue`].
            Alias { variable: VariableId },
        }

        /// A fully-resolved value — the same set as [`VarValue`] without
        /// [`Alias`](VarValue::Alias).
        ///
        /// Producing this is the whole job of alias resolution: render and
        /// export only ever see concrete literals, so they cannot accidentally
        /// try to paint an unresolved reference.
        #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
        #[serde(tag = "kind", rename_all = "snake_case")]
        pub enum ResolvedVarValue {
            $( $variant { value: $ty }, )+
        }

        /// The static type of a variable. Mirrors Figma's
        /// `variableResolvedType`.
        ///
        /// A variable's per-mode values must all share this type; binding only
        /// makes sense when the bound property's expected type matches.
        /// `Typography` is the composite text-style token (font + size + weight
        /// + metrics) carried by [`VarValue::TextStyle`].
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(rename_all = "snake_case")]
        pub enum VariableType {
            $( $vartype, )+
        }

        impl VarValue {
            /// The [`VariableType`] this value carries, or `None` for an
            /// [`Alias`](VarValue::Alias) (whose type is only known after
            /// resolution).
            pub fn variable_type(&self) -> Option<VariableType> {
                match self {
                    $( Self::$variant { .. } => Some(VariableType::$vartype), )+
                    Self::Alias { .. } => None,
                }
            }

            /// If this value is already concrete (not an
            /// [`Alias`](VarValue::Alias)), return the equivalent
            /// [`ResolvedVarValue`]. The base case in alias chasing.
            #[allow(clippy::clone_on_copy)] // uniform over Copy + non-Copy payloads
            pub fn as_resolved(&self) -> Option<ResolvedVarValue> {
                match self {
                    $( Self::$variant { value } => {
                        Some(ResolvedVarValue::$variant { value: value.clone() })
                    } )+
                    Self::Alias { .. } => None,
                }
            }
        }

        impl ResolvedVarValue {
            pub fn variable_type(&self) -> VariableType {
                match self {
                    $( Self::$variant { .. } => VariableType::$vartype, )+
                }
            }

            /// Widen back into a [`VarValue`] (never an alias).
            #[allow(clippy::clone_on_copy)] // uniform over Copy + non-Copy payloads
            pub fn to_var_value(&self) -> VarValue {
                match self {
                    $( Self::$variant { value } => {
                        VarValue::$variant { value: value.clone() }
                    } )+
                }
            }
        }
    };
}

var_values! {
    /// A color literal (same sRGB-with-alpha space as every other doc color).
    Color(Color) => Color,
    /// A floating-point number — radii, widths, opacities, spacing, …
    Float(f64) => Float,
    /// A string literal — text content, font family, arbitrary token strings.
    String(String) => String,
    /// A boolean — visibility, instance-swap toggles, feature flags.
    Boolean(bool) => Boolean,
    /// A composite **text style** token — font family, size, weight, italic,
    /// decorations, color, letter spacing, and line height in one value. This is
    /// how Fanta models Figma "text styles": a named, reusable type style that a
    /// text node binds its whole `TextStyle` to (see `BoundProp::TextStyle`).
    TextStyle(TextStyle) => Typography,
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

    /// The macro must not perturb the wire. Pin the `kind` tag of every value
    /// and resolved-value variant, plus the `VariableType` tags and the full
    /// conversion round-trip, so a bad table row is caught at the byte level.
    #[test]
    fn every_value_kind_has_a_pinned_tag_and_round_trips() {
        let cases: &[(VarValue, &str, &str)] = &[
            (
                VarValue::Color {
                    value: Color::WHITE,
                },
                "color",
                "color",
            ),
            (VarValue::Float { value: 8.0 }, "float", "float"),
            (VarValue::String { value: "x".into() }, "string", "string"),
            (VarValue::Boolean { value: true }, "boolean", "boolean"),
            (
                VarValue::TextStyle {
                    value: TextStyle::default(),
                },
                "text_style",
                "typography",
            ),
        ];
        for (v, kind, type_tag) in cases {
            // VarValue wire tag.
            let vj = serde_json::to_value(v).unwrap();
            assert_eq!(vj["kind"], *kind, "VarValue {kind} tag");
            assert_eq!(serde_json::from_value::<VarValue>(vj).unwrap(), *v);
            // Concrete → resolved → widened is lossless, same tag.
            let r = v.as_resolved().unwrap();
            let rj = serde_json::to_value(&r).unwrap();
            assert_eq!(rj["kind"], *kind, "ResolvedVarValue {kind} tag");
            assert_eq!(serde_json::from_value::<ResolvedVarValue>(rj).unwrap(), r);
            assert_eq!(r.to_var_value(), *v);
            // VariableType agreement on both sides, with its own (renamed) tag.
            assert_eq!(v.variable_type(), Some(r.variable_type()));
            let tj = serde_json::to_value(r.variable_type()).unwrap();
            assert_eq!(tj, serde_json::Value::String((*type_tag).into()));
        }
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
