//! Design-system variables — tokens with per-mode values.
//!
//! ## Why this shape
//!
//! Figma's variable system is: a *collection* owns a set of *modes* (e.g.
//! "Light" / "Dark", or "Mobile" / "Desktop"); a *variable* belongs to one
//! collection and stores one value *per mode*. Resolving a variable means
//! picking the value for the collection's effective mode (which a frame can pin
//! via [`GroupNode::explicit_modes`], or the doc via `Doc::active_modes`, falling
//! back to the collection's `default_mode`). We mirror that exactly so a Figma
//! import is lossless and so our own variable UI has the same mental model.
//!
//! `BTreeMap` everywhere keyed by id keeps the JSON projection deterministic
//! (stable ordering ⇒ clean diffs ⇒ AI-friendly), matching the rest of the doc.
//!
//! [`GroupNode::explicit_modes`]: crate::node::GroupNode::explicit_modes

use crate::id::{ModeId, VariableCollectionId, VariableId};
use crate::value::{VarValue, VariableType};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// The whole variable system for a document: every collection and every
/// variable, addressable by id. Lives on [`crate::doc::Doc`].
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct VariableRegistry {
    /// Collections by id.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub collections: BTreeMap<VariableCollectionId, VariableCollection>,
    /// Variables by id. A variable names its owning collection, so this is a
    /// flat map (not nested under the collection) — that makes "resolve this
    /// `VariableId`" an O(log n) lookup regardless of which collection owns it.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub variables: BTreeMap<VariableId, Variable>,
}

impl VariableRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.collections.is_empty() && self.variables.is_empty()
    }

    /// Look up a variable by id.
    pub fn variable(&self, id: VariableId) -> Option<&Variable> {
        self.variables.get(&id)
    }

    /// Look up the collection owning `var_id`, if both exist.
    pub fn collection_of(&self, var_id: VariableId) -> Option<&VariableCollection> {
        let v = self.variables.get(&var_id)?;
        self.collections.get(&v.collection)
    }
}

/// A named group of variables sharing one mode axis (the modes are the columns
/// in Figma's variable table; the variables are the rows).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VariableCollection {
    pub id: VariableCollectionId,
    pub name: String,
    /// The modes (columns). Always non-empty in a well-formed collection;
    /// `default_mode` must be one of these.
    pub modes: Vec<Mode>,
    /// The mode used when nothing pins one (no frame `explicit_modes`, no
    /// `Doc.active_modes` entry). The ultimate fallback in mode resolution.
    pub default_mode: ModeId,
    /// Display order of the collection's variables in the panel. UI-only;
    /// resolution never consults it. Variables not listed sort after, by id.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub variable_order: Vec<VariableId>,
}

impl VariableCollection {
    /// Whether `mode` is one of this collection's declared modes.
    pub fn has_mode(&self, mode: ModeId) -> bool {
        self.modes.iter().any(|m| m.id == mode)
    }
}

/// One mode (column) within a [`VariableCollection`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Mode {
    pub id: ModeId,
    pub name: String,
}

/// A single design-system variable (token): one typed value per mode.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Variable {
    pub id: VariableId,
    /// The collection this variable belongs to (whose modes index `values_by_mode`).
    pub collection: VariableCollectionId,
    pub name: String,
    /// Static type — every entry in `values_by_mode` must carry this type (an
    /// [`VarValue::Alias`] resolves to it). Stored explicitly so the UI can
    /// filter bindable properties and importers preserve `variableResolvedType`.
    pub ty: VariableType,
    /// The per-mode values. A lookup misses (no entry for the effective mode)
    /// degrade gracefully in resolution — they yield `None`, not a panic.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub values_by_mode: BTreeMap<ModeId, VarValue>,
    /// Where this variable is offered in the binding UI. Pure UI filtering —
    /// growable without a migration (unknown scopes from a newer file simply
    /// round-trip). Empty = "all scopes".
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scopes: Vec<VariableScope>,
}

impl Variable {
    /// The value for `mode`, if this variable defines one.
    pub fn value_for_mode(&self, mode: ModeId) -> Option<&VarValue> {
        self.values_by_mode.get(&mode)
    }
}

/// UI-only filtering hint for where a variable may be bound. Figma's
/// `variableScopes`. Kept as a closed-but-tolerant enum: unknown strings from a
/// newer file decode to [`VariableScope::Unknown`] so we never reject a doc we
/// merely don't fully understand.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VariableScope {
    AllFills,
    StrokeColor,
    CornerRadius,
    WidthHeight,
    Opacity,
    TextContent,
    /// Any scope this build doesn't recognize. Carries the raw tag so it
    /// round-trips unchanged. Lets the scope vocabulary grow without a bump.
    #[serde(other)]
    Unknown,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::Color;

    fn fixture() -> (VariableRegistry, VariableCollectionId, ModeId, VariableId) {
        let coll_id = VariableCollectionId::from_u128(1);
        let light = ModeId::from_u128(10);
        let var_id = VariableId::from_u128(100);
        let mut reg = VariableRegistry::new();
        reg.collections.insert(
            coll_id,
            VariableCollection {
                id: coll_id,
                name: "Theme".into(),
                modes: vec![Mode {
                    id: light,
                    name: "Light".into(),
                }],
                default_mode: light,
                variable_order: vec![var_id],
            },
        );
        reg.variables.insert(
            var_id,
            Variable {
                id: var_id,
                collection: coll_id,
                name: "bg".into(),
                ty: VariableType::Color,
                values_by_mode: BTreeMap::from([(
                    light,
                    VarValue::Color {
                        value: Color::WHITE,
                    },
                )]),
                scopes: vec![VariableScope::AllFills],
            },
        );
        (reg, coll_id, light, var_id)
    }

    #[test]
    fn registry_round_trips_and_resolves_collection() {
        let (reg, coll_id, light, var_id) = fixture();
        let j = serde_json::to_string(&reg).unwrap();
        let back: VariableRegistry = serde_json::from_str(&j).unwrap();
        assert_eq!(reg, back);
        assert_eq!(back.collection_of(var_id).unwrap().id, coll_id);
        assert!(back.collections[&coll_id].has_mode(light));
        assert_eq!(
            back.variable(var_id).unwrap().value_for_mode(light),
            Some(&VarValue::Color {
                value: Color::WHITE
            })
        );
    }

    #[test]
    fn empty_registry_serializes_compact() {
        let reg = VariableRegistry::new();
        assert!(reg.is_empty());
        let j = serde_json::to_string(&reg).unwrap();
        assert_eq!(j, "{}");
    }

    #[test]
    fn unknown_scope_decodes_to_unknown_variant() {
        let v: VariableScope = serde_json::from_str("\"some_future_scope\"").unwrap();
        assert_eq!(v, VariableScope::Unknown);
    }
}
