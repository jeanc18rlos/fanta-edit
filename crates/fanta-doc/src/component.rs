//! Components, component sets (variants), and the library that holds them.
//!
//! ## How masters live in the scene
//!
//! A component *master* is not a special node type — it is an ordinary `Group`
//! subtree living under a hidden "Components" page. [`ComponentDef::root`] points
//! at that subtree's root. The payoff: every existing op (move, restyle,
//! reparent, replace-data) edits a master *for free*, with no component-specific
//! op surface. The library here adds only the metadata that a plain subtree
//! can't carry — the prop schema, variant membership, and a revision counter.
//!
//! ## Why a `rev` counter
//!
//! Instances are expanded lazily ([`crate::resolve::expand_instance`]) and the
//! render layer memoizes the expansion. The memo key includes `ComponentDef.rev`
//! so that editing a master *invalidates every instance of it* without the
//! render layer diffing subtrees. [`ComponentLibrary::bump_rev_for_node`] is the
//! one place that bumps it: after any scene-mutating op, the doc calls it with
//! the touched node, and it bumps `rev` on every def whose `root` is an
//! ancestor-or-self of that node (index-free, O(depth) via `scene.ancestors_of`).

use crate::binding::BoundProp;
use crate::id::{ComponentId, ComponentPropId, ModeId, NodeId, VariableCollectionId};
use crate::node::OverridePath;
use crate::scene::Scene;
use crate::value::VarValue;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Every component master and component set in the document.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ComponentLibrary {
    /// Single-component masters by id.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub defs: BTreeMap<ComponentId, ComponentDef>,
    /// Component sets (variant groups) by id. A set's members reference `defs`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub sets: BTreeMap<ComponentId, ComponentSet>,
}

impl ComponentLibrary {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.defs.is_empty() && self.sets.is_empty()
    }

    pub fn def(&self, id: ComponentId) -> Option<&ComponentDef> {
        self.defs.get(&id)
    }

    /// Bump `rev` on every [`ComponentDef`] whose `root` is an ancestor-or-self
    /// of `node_id`. Called after a scene-mutating op so instances of any master
    /// containing the touched node get re-expanded. O(depth) — walks the touched
    /// node's ancestor chain (plus itself) and matches roots against it.
    ///
    /// Returns the number of defs whose `rev` changed (mostly for tests).
    pub fn bump_rev_for_node(&mut self, scene: &Scene, node_id: NodeId) -> usize {
        if self.defs.is_empty() {
            return 0;
        }
        // Build the ancestor-or-self set for `node_id`: itself plus every
        // ancestor. `ancestors_of` does not yield `node_id` itself, so we add it.
        let mut chain: Vec<NodeId> = Vec::with_capacity(8);
        chain.push(node_id);
        for anc in scene.ancestors_of(node_id) {
            chain.push(anc.id);
        }
        let mut bumped = 0;
        for def in self.defs.values_mut() {
            if chain.contains(&def.root) {
                def.rev = def.rev.wrapping_add(1);
                bumped += 1;
            }
        }
        bumped
    }
}

/// One component master: a named subtree plus its prop schema and (optionally)
/// its membership in a variant set.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ComponentDef {
    pub id: ComponentId,
    /// Root of the master subtree in the scene (under the hidden Components page).
    pub root: NodeId,
    pub name: String,
    /// `Some` when this def is one variant of a [`ComponentSet`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variant_of: Option<ComponentSetMembership>,
    /// Exposed properties (instance-swappable text, booleans, nested-instance
    /// swaps, variant selectors).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub props: Vec<ComponentPropDef>,
    /// Monotonic revision. Bumped whenever the master subtree is edited, so
    /// instance expansions can be memoized and invalidated cheaply. Skipped from
    /// JSON when zero so a freshly-defined, never-edited master stays compact.
    #[serde(default, skip_serializing_if = "is_zero_u64")]
    pub rev: u64,
}

fn is_zero_u64(v: &u64) -> bool {
    *v == 0
}

impl ComponentDef {
    /// A new def over `root` with no props and rev 0.
    pub fn new(id: ComponentId, root: NodeId, name: impl Into<String>) -> Self {
        Self {
            id,
            root,
            name: name.into(),
            variant_of: None,
            props: Vec::new(),
            rev: 0,
        }
    }
}

/// One property a component exposes to its instances.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ComponentPropDef {
    pub id: ComponentPropId,
    pub name: String,
    pub kind: ComponentPropKind,
    /// The input widget that renders this prop. `Auto` (the default, and what
    /// pre-formatter files deserialize to) uses the kind's natural editor.
    #[serde(default, skip_serializing_if = "ComponentPropFormatter::is_auto")]
    pub formatter: ComponentPropFormatter,
    /// Default value used when an instance doesn't override the prop. A
    /// [`VarValue`] so it shares the typed-value machinery (and can itself be a
    /// variable alias).
    pub default: VarValue,
    /// Descendants this prop drives. A Bool prop bound to a child's `Visible`
    /// shows/hides it; a Text prop bound to `TextContent` swaps its text. Empty
    /// for an unbound (display-only) prop. Applied during instance expansion.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bindings: Vec<PropBindingTarget>,
}

/// Binds a [`ComponentPropDef`] to a writable property of a master descendant,
/// so an instance's prop value drives that descendant when the instance expands.
/// `path` is the def-local path (master ids, root-first, root excluded) — the
/// same address space as [`Override::target_path`](crate::node::Override).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PropBindingTarget {
    /// Def-local path to the descendant being driven.
    pub path: OverridePath,
    /// Which writable property of that descendant the prop value is written to.
    pub prop: BoundProp,
}

/// What a [`ComponentPropDef`] controls — its primitive type. Pairs with a
/// [`ComponentPropFormatter`] (the input widget) to form a "type + formatter"
/// dynamic-form schema.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ComponentPropKind {
    /// A boolean toggle (typically driving a descendant's visibility).
    Bool,
    /// A text string (typically driving a descendant text node's content).
    Text,
    /// A number (drives a descendant's corner radius, stroke width, opacity, …).
    Number,
    /// A color (drives a descendant's fill or stroke color).
    Color,
    /// Swap a nested instance for a different component.
    InstanceSwap,
    /// A variant selector along one axis of the owning component set.
    Variant { axis: String },
}

impl ComponentPropKind {
    /// The natural literal default for a freshly-added prop of this kind.
    pub fn default_value(&self) -> VarValue {
        match self {
            Self::Bool => VarValue::Boolean { value: false },
            Self::Number => VarValue::Float { value: 0.0 },
            Self::Color => VarValue::Color {
                value: crate::Color::rgb(153, 153, 153),
            },
            // Text, Variant, InstanceSwap all carry a string payload.
            Self::Text | Self::Variant { .. } | Self::InstanceSwap => VarValue::String {
                value: String::new(),
            },
        }
    }

    /// The input widgets ("formatters") valid for this kind, first = default.
    pub fn formatters(&self) -> &'static [ComponentPropFormatter] {
        use ComponentPropFormatter as F;
        match self {
            Self::Number => &[F::Field, F::Slider],
            // Single-widget kinds: `Auto` resolves to the natural editor.
            _ => &[F::Auto],
        }
    }
}

/// Which input widget renders a [`ComponentPropDef`] in the inspector — the
/// "formatter" half of the type + formatter schema. `Auto` (also what old files
/// deserialize to) picks the kind's natural editor: color → picker, text →
/// single-line, bool → toggle, number → field. `Number` may opt into a slider.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComponentPropFormatter {
    /// The kind's natural editor.
    #[default]
    Auto,
    /// Numeric input field.
    Field,
    /// Numeric slider (0..1).
    Slider,
}

impl ComponentPropFormatter {
    /// True for the default `Auto` formatter (skipped from JSON when so).
    pub fn is_auto(&self) -> bool {
        matches!(self, Self::Auto)
    }

    /// Human label for the formatter picker.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Auto => "Auto",
            Self::Field => "Field",
            Self::Slider => "Slider",
        }
    }
}

/// A variant group: several [`ComponentDef`]s differing along named axes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ComponentSet {
    pub id: ComponentId,
    pub name: String,
    /// The variant axes (e.g. "Size" with values [S, M, L], "State" with
    /// [Default, Hover]). A member picks one value per axis.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub axes: Vec<VariantAxis>,
    /// Member component ids (each is a key in [`ComponentLibrary::defs`]).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub members: Vec<ComponentId>,
    /// The variant shown when an instance of the *set* is created without an
    /// explicit variant selection.
    pub default_variant: ComponentId,
}

/// One axis of a [`ComponentSet`] and its allowed values.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VariantAxis {
    pub name: String,
    pub values: Vec<String>,
}

/// Where a [`ComponentDef`] sits within its [`ComponentSet`]: which set, and
/// which value it takes on each axis (axis name → value).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ComponentSetMembership {
    pub set: ComponentId,
    /// Axis-name → chosen value. `BTreeMap` for deterministic JSON ordering.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub axis_values: BTreeMap<String, String>,
}

/// A per-frame theme pin: which mode is active for a given collection, when a
/// frame's [`crate::node::GroupNode::explicit_modes`] (this exact map type)
/// names one. Re-exported here only as documentation of the map's value shape;
/// the field itself lives on `GroupNode`.
pub type ExplicitModes = BTreeMap<VariableCollectionId, ModeId>;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::Color;
    use crate::node::{CanvasNode, GroupNode, NodeData, VectorNode};

    #[test]
    fn library_round_trips() {
        let comp_id = ComponentId::from_u128(1);
        let root = NodeId::from_u128(2);
        let mut lib = ComponentLibrary::new();
        let mut def = ComponentDef::new(comp_id, root, "Button");
        def.props.push(ComponentPropDef {
            id: ComponentPropId::from_u128(3),
            name: "Label".into(),
            kind: ComponentPropKind::Text,
            formatter: Default::default(),
            default: VarValue::String {
                value: "Click".into(),
            },
            bindings: Vec::new(),
        });
        lib.defs.insert(comp_id, def);
        let j = serde_json::to_string(&lib).unwrap();
        let back: ComponentLibrary = serde_json::from_str(&j).unwrap();
        assert_eq!(lib, back);
        // rev 0 is omitted.
        assert!(!j.contains("\"rev\""));
    }

    #[test]
    fn bump_rev_for_node_hits_ancestor_master_only() {
        // Master subtree: group `root` → child rect. A second, unrelated group.
        let mut scene = Scene::new();
        let root = CanvasNode::new(NodeData::Group(GroupNode::default()));
        let root_id = root.id;
        scene.insert(root).unwrap();
        let mut child = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
            0.0,
            0.0,
            10.0,
            10.0,
            Color::WHITE,
        )));
        child.parent = Some(root_id);
        let child_id = child.id;
        scene.insert(child).unwrap();

        let other = CanvasNode::new(NodeData::Group(GroupNode::default()));
        let other_id = other.id;
        scene.insert(other).unwrap();

        let mut lib = ComponentLibrary::new();
        let def_id = ComponentId::from_u128(1);
        lib.defs
            .insert(def_id, ComponentDef::new(def_id, root_id, "M"));
        let other_def_id = ComponentId::from_u128(2);
        lib.defs.insert(
            other_def_id,
            ComponentDef::new(other_def_id, other_id, "Other"),
        );

        // Editing the child must bump the master containing it, not the other.
        let bumped = lib.bump_rev_for_node(&scene, child_id);
        assert_eq!(bumped, 1);
        assert_eq!(lib.defs[&def_id].rev, 1);
        assert_eq!(lib.defs[&other_def_id].rev, 0);

        // Editing the root itself (ancestor-or-SELF) also bumps it.
        lib.bump_rev_for_node(&scene, root_id);
        assert_eq!(lib.defs[&def_id].rev, 2);
    }

    #[test]
    fn empty_library_serializes_compact() {
        let lib = ComponentLibrary::new();
        assert_eq!(serde_json::to_string(&lib).unwrap(), "{}");
    }
}

#[cfg(test)]
mod formatter_tests {
    use super::*;

    #[test]
    fn component_prop_formatter_round_trips_and_old_docs_default_to_auto() {
        use crate::color::Color;
        use crate::value::VarValue;
        let mut def = ComponentDef::new(ComponentId::from_u128(1), NodeId::from_u128(2), "Button");
        def.props.push(ComponentPropDef {
            id: ComponentPropId::from_u128(3),
            name: "Radius".to_owned(),
            kind: ComponentPropKind::Number,
            formatter: ComponentPropFormatter::Slider,
            default: VarValue::Float { value: 4.0 },
            bindings: Vec::new(),
        });
        def.props.push(ComponentPropDef {
            id: ComponentPropId::from_u128(4),
            name: "Tint".to_owned(),
            kind: ComponentPropKind::Color,
            formatter: ComponentPropFormatter::Auto,
            default: VarValue::Color {
                value: Color::rgb(0x99, 0x99, 0x99),
            },
            bindings: Vec::new(),
        });

        let json = serde_json::to_string(&def).unwrap();
        // The default `Auto` formatter is omitted; an explicit one is kept.
        assert!(!json.contains("\"formatter\":\"auto\""));
        assert!(json.contains("\"formatter\":\"slider\""));

        // Full round-trip preserves the new Number/Color kinds + formatter.
        let back: ComponentDef = serde_json::from_str(&json).unwrap();
        assert_eq!(back, def);

        // A pre-formatter doc (no "formatter" field) deserializes to Auto.
        let dropped = json.replace(",\"formatter\":\"slider\"", "");
        let back2: ComponentDef = serde_json::from_str(&dropped).unwrap();
        assert!(back2.props[0].formatter.is_auto());
    }
}
