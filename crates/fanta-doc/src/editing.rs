//! Editing rules: which changes a template's users may make to a node, and
//! whether the node is a placeholder they fill in. The rules are data on the
//! node; whatever edits the document on a user's behalf enforces them.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

/// One kind of change a node can forbid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Scope {
    #[serde(rename = "layer/move")]
    LayerMove,
    #[serde(rename = "layer/resize")]
    LayerResize,
    #[serde(rename = "layer/rotate")]
    LayerRotate,
    #[serde(rename = "layer/flip")]
    LayerFlip,
    #[serde(rename = "layer/crop")]
    LayerCrop,
    #[serde(rename = "layer/opacity")]
    LayerOpacity,
    #[serde(rename = "layer/blend_mode")]
    LayerBlendMode,
    #[serde(rename = "layer/visibility")]
    LayerVisibility,
    #[serde(rename = "layer/clipping")]
    LayerClipping,
    #[serde(rename = "appearance/adjustments")]
    AppearanceAdjustments,
    #[serde(rename = "appearance/filter")]
    AppearanceFilter,
    #[serde(rename = "appearance/effect")]
    AppearanceEffect,
    #[serde(rename = "appearance/blur")]
    AppearanceBlur,
    #[serde(rename = "appearance/shadow")]
    AppearanceShadow,
    #[serde(rename = "appearance/animation")]
    AppearanceAnimation,
    #[serde(rename = "lifecycle/destroy")]
    LifecycleDestroy,
    #[serde(rename = "lifecycle/duplicate")]
    LifecycleDuplicate,
    #[serde(rename = "editor/add")]
    EditorAdd,
    #[serde(rename = "editor/select")]
    EditorSelect,
    #[serde(rename = "fill/change")]
    FillChange,
    #[serde(rename = "fill/change_type")]
    FillChangeType,
    #[serde(rename = "stroke/change")]
    StrokeChange,
    #[serde(rename = "shape/change")]
    ShapeChange,
    #[serde(rename = "text/edit")]
    TextEdit,
    #[serde(rename = "text/character")]
    TextCharacter,
    /// A scope from a newer file that this build does not know. It loads
    /// instead of failing the page, and saves back as `unknown`.
    #[serde(other, rename = "unknown")]
    Unknown,
}

impl Scope {
    /// Every scope this build knows.
    pub const ALL: [Scope; 25] = [
        Scope::LayerMove,
        Scope::LayerResize,
        Scope::LayerRotate,
        Scope::LayerFlip,
        Scope::LayerCrop,
        Scope::LayerOpacity,
        Scope::LayerBlendMode,
        Scope::LayerVisibility,
        Scope::LayerClipping,
        Scope::AppearanceAdjustments,
        Scope::AppearanceFilter,
        Scope::AppearanceEffect,
        Scope::AppearanceBlur,
        Scope::AppearanceShadow,
        Scope::AppearanceAnimation,
        Scope::LifecycleDestroy,
        Scope::LifecycleDuplicate,
        Scope::EditorAdd,
        Scope::EditorSelect,
        Scope::FillChange,
        Scope::FillChangeType,
        Scope::StrokeChange,
        Scope::ShapeChange,
        Scope::TextEdit,
        Scope::TextCharacter,
    ];

    /// The scope's key as files spell it, such as `layer/move`.
    pub fn key(self) -> &'static str {
        match self {
            Scope::LayerMove => "layer/move",
            Scope::LayerResize => "layer/resize",
            Scope::LayerRotate => "layer/rotate",
            Scope::LayerFlip => "layer/flip",
            Scope::LayerCrop => "layer/crop",
            Scope::LayerOpacity => "layer/opacity",
            Scope::LayerBlendMode => "layer/blend_mode",
            Scope::LayerVisibility => "layer/visibility",
            Scope::LayerClipping => "layer/clipping",
            Scope::AppearanceAdjustments => "appearance/adjustments",
            Scope::AppearanceFilter => "appearance/filter",
            Scope::AppearanceEffect => "appearance/effect",
            Scope::AppearanceBlur => "appearance/blur",
            Scope::AppearanceShadow => "appearance/shadow",
            Scope::AppearanceAnimation => "appearance/animation",
            Scope::LifecycleDestroy => "lifecycle/destroy",
            Scope::LifecycleDuplicate => "lifecycle/duplicate",
            Scope::EditorAdd => "editor/add",
            Scope::EditorSelect => "editor/select",
            Scope::FillChange => "fill/change",
            Scope::FillChangeType => "fill/change_type",
            Scope::StrokeChange => "stroke/change",
            Scope::ShapeChange => "shape/change",
            Scope::TextEdit => "text/edit",
            Scope::TextCharacter => "text/character",
            Scope::Unknown => "unknown",
        }
    }

    /// The known scope `key` names.
    pub fn from_key(key: &str) -> Option<Scope> {
        Scope::ALL.into_iter().find(|scope| scope.key() == key)
    }
}

impl std::fmt::Display for Scope {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.key())
    }
}

/// What a template's users may change on a node, and whether it is a
/// placeholder. A node without rules forbids nothing.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EditingRules {
    /// Scopes the node forbids wherever the session defers to the node.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub disabled_scopes: BTreeSet<Scope>,
    /// Present when the node is a placeholder a template's user fills in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<Placeholder>,
}

impl EditingRules {
    pub fn is_default(&self) -> bool {
        self.disabled_scopes.is_empty() && self.placeholder.is_none()
    }

    pub fn allows(&self, scope: Scope) -> bool {
        !self.disabled_scopes.contains(&scope)
    }
}

/// How an editor presents a placeholder until it is filled.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Placeholder {
    /// Highlight the node as unfilled.
    #[serde(default, skip_serializing_if = "crate::serde_util::is_false")]
    pub show_overlay: bool,
    /// Show a button that starts replacing its content.
    #[serde(default, skip_serializing_if = "crate::serde_util::is_false")]
    pub show_button: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CanvasNode, Doc, NodeData, Operation, TextNode};

    fn rules() -> EditingRules {
        EditingRules {
            disabled_scopes: BTreeSet::from([Scope::LayerMove, Scope::LifecycleDestroy]),
            placeholder: Some(Placeholder {
                show_overlay: true,
                show_button: false,
            }),
        }
    }

    #[test]
    fn scope_keys_match_their_serialized_names() {
        for scope in Scope::ALL {
            assert_eq!(
                serde_json::to_value(scope).expect("serialize"),
                serde_json::Value::String(scope.key().to_owned())
            );
            assert_eq!(Scope::from_key(scope.key()), Some(scope));
        }
        assert_eq!(
            Scope::ALL.iter().collect::<BTreeSet<_>>().len(),
            Scope::ALL.len()
        );
        assert_eq!(Scope::from_key("unknown"), None);
    }

    #[test]
    fn nodes_carry_editing_rules_only_when_they_have_them() {
        let mut node = CanvasNode::new(NodeData::Text(TextNode::new("Name", 80.0, 20.0)));
        let plain = serde_json::to_value(&node).expect("serialize");
        assert!(
            plain.get("editing").is_none(),
            "old files stay byte-identical"
        );

        node.editing = Some(Box::new(rules()));
        let value = serde_json::to_value(&node).expect("serialize");
        assert_eq!(
            value.get("editing"),
            Some(&serde_json::json!({
                "disabled_scopes": ["layer/move", "lifecycle/destroy"],
                "placeholder": { "show_overlay": true }
            }))
        );
        let back: CanvasNode = serde_json::from_value(value).expect("deserialize");
        assert_eq!(back, node);
    }

    #[test]
    fn scopes_from_a_newer_file_still_load() {
        let rules: EditingRules = serde_json::from_value(serde_json::json!({
            "disabled_scopes": ["layer/move", "layer/skew"]
        }))
        .expect("deserialize");
        assert_eq!(
            rules.disabled_scopes,
            BTreeSet::from([Scope::LayerMove, Scope::Unknown])
        );
        assert!(!rules.allows(Scope::LayerMove));
        assert!(rules.allows(Scope::LayerResize));
    }

    #[test]
    fn set_editing_rules_applies_and_undoes() {
        let mut doc = Doc::new();
        let node = CanvasNode::new(NodeData::Text(TextNode::new("Name", 80.0, 20.0)));
        let id = node.id;
        doc.apply(Operation::create_node(node)).expect("create");
        doc.apply(Operation::SetEditingRules {
            id,
            old: None,
            new: Some(Box::new(rules())),
        })
        .expect("set rules");
        let editing = |doc: &Doc| doc.scene.get(id).and_then(|node| node.editing.clone());
        assert_eq!(editing(&doc), Some(Box::new(rules())));
        assert!(doc.undo().expect("undo"));
        assert_eq!(editing(&doc), None);
        assert!(doc.redo().expect("redo"));
        assert_eq!(editing(&doc), Some(Box::new(rules())));
    }
}
